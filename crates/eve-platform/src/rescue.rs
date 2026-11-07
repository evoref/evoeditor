//! 緊急退避の書き手 (docs/02 §11.2.1、05 §7.5 / §5.6)。`eve_core::RescueWriter` の実装 (Windows と Unix。OS ごとの部品は `sys`)。
//!
//! 公開済みの `DocSnapshot` だけを読み、`std::fs` を直接使って (Port を通さない) 退避先に書く。**小さい文書から**、1 文書書くごとに
//! `manifest.toml` を原子的に (一時ファイル + fsync + rename) 更新し、時間予算内に書けた分が確実に残るようにする。
//! 退避先は `state_dir/recovery/<UTC timestamp>-<pid>[-<n>]/`。状態ディレクトリに書けなければ、代替の退避先
//! `%TEMP%\eve-recovery-<SID>\` (所有者のみの DACL) / `$TMPDIR/eve-recovery-<uid>/` (0700) (05 §7.5) に書く。作るディレクトリは所有者のみ (中のファイルは継承)。
//!
//! 本文は内部の UTF-8 のまま・改行を正規化せず・不正バイトの PUA エスケープを保つ (元の文字コードへの再エンコードはしない)。
//! 書く前に空き容量を確かめ (足りなければ代替の退避先へ)、代替の退避先の既存ディレクトリは所有者が自分であることを確かめる (05 §7.5)。
//! 範囲外: 30 日を超えた退避の案内。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eve_core::{DocSnapshot, RescueOutcome, RescueReason, RescueWriter};
use eve_text::{ByteRange, LineEnding, TextRead};

use crate::inflight::create_dir_all_private;
use crate::toml_lite;

/// manifest の版 (意味の変更・必須キーの追加で上げる。キーの追加は版を上げない)。
const FORMAT_VERSION: u32 = 1;

/// 緊急退避の書き手。
#[derive(Clone, Debug, Default)]
pub struct StateRescue {
    state_dir: Option<PathBuf>,
    /// これより遅い退避の段をログに書く (既定は `SLOW_STAGE`)。
    slow_stage: Duration,
    /// 最初の文書の manifest を書いた直後に、期限 (`received + budget`) が過ぎるまで眠り、さらに眠る余りの時間 (単体試験だけが
    /// 変える。別プロセスの試験は test-hooks の `rescue_stall_after_manifest`)。`None` は眠らない。
    stall_after_manifest: Option<Duration>,
}

impl StateRescue {
    /// `state_dir` は 05 §7.5 の状態ディレクトリ (無ければ代替の退避先だけに書く)。
    pub fn new(state_dir: Option<PathBuf>) -> StateRescue {
        StateRescue {
            state_dir,
            slow_stage: SLOW_STAGE,
            stall_after_manifest: None,
        }
    }

    /// 段の時間をログに書くしきい値を変える (試験用。時間に依らずに「速い」「遅い」を確かめるため)。
    pub fn with_slow_stage(mut self, slow_stage: Duration) -> StateRescue {
        self.slow_stage = slow_stage;
        self
    }

    /// 最初の文書の manifest を書いた直後に、期限が過ぎるまで眠り、さらに `extra` だけ眠らせる (単体試験用。test-hooks の
    /// `rescue_stall_after_manifest` と同じ)。
    #[cfg(test)]
    fn with_stall_after_manifest(mut self, extra: Duration) -> StateRescue {
        self.stall_after_manifest = Some(extra);
        self
    }
}

// ---------- 時刻 (UTC) ----------

/// UNIX 秒 → (年, 月, 日, 時, 分, 秒)。Howard Hinnant の civil_from_days。
fn civil(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        (rem % 3600 / 60) as u32,
        (rem % 60) as u32,
    )
}

pub(crate) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// `YYYYMMDDTHHMMSSZ` (ISO 8601 の基本形式、05 §7.5 の退避ディレクトリ名)。
pub(crate) fn timestamp_basic(secs: i64) -> String {
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}Z")
}

/// RFC 3339 の UTC (`2026-10-15T12:34:56Z`)。
pub(crate) fn timestamp_rfc3339(secs: i64) -> String {
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

// ---------- 退避先 ----------

/// `<base>/<UTC timestamp>-<pid>[-<n>]/` を新しく作る (同じ名前があれば空いている最小の `-<n>`、n は 2 以上)。
fn create_unique_dir(base: &Path, secs: i64) -> io::Result<PathBuf> {
    create_dir_all_private(base)?;
    let stem = format!("{}-{}", timestamp_basic(secs), std::process::id());
    for n in 1..1000 {
        let name = if n == 1 {
            stem.clone()
        } else {
            format!("{stem}-{n}")
        };
        let dir = base.join(name);
        // 在れば失敗する (AlreadyExists) 作り方。Unix は 0700 (Windows は親の保護付き DACL を継承する)
        #[cfg_attr(windows, allow(unused_mut))]
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::from(io::ErrorKind::AlreadyExists))
}

/// 空き容量の問い合わせ (調べられなければ `None`)。
type FreeFn<'a> = &'a dyn Fn(&Path) -> Option<u64>;
/// ディレクトリの所有者が自分か。
type OwnedFn<'a> = &'a dyn Fn(&Path) -> io::Result<bool>;

/// 退避先を決めて作る。通常の場所 (`state_dir/recovery`) に作れなければ代替の退避先。戻り値は (ディレクトリ, 代替か)。
/// `rescue` は「退避先に保存」(`reason = "user_save"`) で、`recovery/rescue/` の下に作る (05 §7.5)。
/// `needed` は書く本文の合計 (バイト): 通常の場所の空きが足りなければ代替の退避先へ、どちらも足りなければ
/// (調べられないときも) 通常の場所に書けるだけ書く (小さい文書から書くので、書けた分が残る)。
fn open_dir(
    state_dir: Option<&Path>,
    secs: i64,
    rescue: bool,
    needed: u64,
) -> io::Result<(PathBuf, bool)> {
    open_dir_with(
        state_dir,
        secs,
        rescue,
        needed,
        &crate::sys::free_bytes,
        &crate::sys::owned_by_me,
    )
}

fn open_dir_with(
    state_dir: Option<&Path>,
    secs: i64,
    rescue: bool,
    needed: u64,
    free: FreeFn<'_>,
    owned: OwnedFn<'_>,
) -> io::Result<(PathBuf, bool)> {
    let sub = |base: PathBuf| if rescue { base.join("rescue") } else { base };
    let enough = |dir: &Path| free(dir).is_none_or(|f| f >= needed);
    // 空きが足りなかった候補 (ほかに使える場所が無いときに使う)
    let mut short: Option<(PathBuf, bool)> = None;
    if let Some(sd) = state_dir {
        let base = sub(sd.join("recovery"));
        if let Ok(d) = create_unique_dir(&base, secs) {
            if enough(&d) {
                return Ok((d, false));
            }
            let _ = fs::remove_dir(&d);
            short = Some((base, false));
        }
    }
    let fallback = crate::sys::recovery_fallback_dir()?;
    create_dir_all_private(&fallback)?;
    // 既にある場所は所有者が自分でなければ使わない (他のユーザーが先に作った場所に書かない、05 §7.5)
    if !owned(&fallback)? {
        return short
            .map(|(base, alt)| create_unique_dir(&base, secs).map(|d| (d, alt)))
            .unwrap_or_else(|| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "代替の退避先の所有者が現在のユーザーではありません",
                ))
            });
    }
    let base = sub(fallback);
    let d = create_unique_dir(&base, secs)?;
    if enough(&d) || short.is_none() {
        return Ok((d, true));
    }
    // どちらも足りない: 通常の場所に書けるだけ書く
    let _ = fs::remove_dir(&d);
    let (base, alt) = short.expect("上で確かめた");
    create_unique_dir(&base, secs).map(|d| (d, alt))
}

/// 本文のファイル名の `<basename>` に使える形にする (予約文字・制御文字を `_`、長さを制限)。
pub(crate) fn safe_basename(name: &str) -> String {
    // 100 文字まで。Unix はファイル名の上限が 255 バイトなので、UTF-8 で 200 バイトまでにも抑える (前後の `<n>-` と
    // `.r<rev>.txt` の分を残す。05 §4.1 の「一時ファイル・退避名の長さ」)
    let byte_budget = if cfg!(unix) { 200 } else { usize::MAX };
    let mut bytes = 0;
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .take_while(|c| {
            bytes += c.len_utf8();
            bytes <= byte_budget
        })
        .collect();
    let trimmed = cleaned.trim_matches(['.', ' ']);
    if trimmed.is_empty() {
        "document".to_owned()
    } else {
        trimmed.to_owned()
    }
}

// ---------- 起動時の走査 ----------

/// 退避ディレクトリの名前の形か: `<YYYYMMDDTHHMMSSZ>-<pid>[-<n>]` (05 §7.5。n は 2 以上)。
pub(crate) fn is_recovery_dir_name(name: &str) -> bool {
    let b = name.as_bytes();
    // YYYYMMDDTHHMMSSZ
    if b.len() < 18
        || !b[..8].iter().all(u8::is_ascii_digit)
        || b[8] != b'T'
        || !b[9..15].iter().all(u8::is_ascii_digit)
        || b[15] != b'Z'
        || b[16] != b'-'
    {
        return false;
    }
    let mut parts = name[17..].split('-');
    let digits =
        |s: Option<&str>| s.is_some_and(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()));
    if !digits(parts.next()) {
        return false;
    }
    match parts.next() {
        None => true,
        Some(n) => {
            digits(Some(n)) && n.parse::<u32>().is_ok_and(|n| n >= 2) && parts.next().is_none()
        }
    }
}

/// `base` の直下にある、名前の形に合うディレクトリ (中のファイルは読まない)。
fn scan_base(base: &Path, rescue: bool, out: &mut Vec<eve_core::RecoveryDirEntry>) {
    let Ok(rd) = fs::read_dir(base) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() && is_recovery_dir_name(&e.file_name().to_string_lossy()) {
            out.push(eve_core::RecoveryDirEntry {
                path: eve_core::FsPath::new(e.path()),
                rescue,
            });
        }
    }
}

/// 起動時の退避の案内用の列挙 (05 §7.5 の「`recovery/` の走査と削除の規則」)。`state_dir/recovery/` の直下と `recovery/rescue/` の
/// 直下、代替の退避先 (`%TEMP%\eve-recovery-<SID>\` とその `rescue\`) の同じ位置だけを見る。`inflight/` は見ない。
pub(crate) fn list_recovery_dirs(state_dir: Option<&Path>) -> Vec<eve_core::RecoveryDirEntry> {
    let mut out = Vec::new();
    if let Some(sd) = state_dir {
        let base = sd.join("recovery");
        scan_base(&base, false, &mut out);
        scan_base(&base.join("rescue"), true, &mut out);
    }
    if let Ok(base) = crate::sys::recovery_fallback_dir() {
        scan_base(&base, false, &mut out);
        scan_base(&base.join("rescue"), true, &mut out);
    }
    out
}

// ---------- manifest ----------

pub(crate) struct ManifestDoc {
    pub(crate) file: String,
    pub(crate) path: Option<String>,
    pub(crate) untitled_name: Option<String>,
    pub(crate) encoding: &'static str,
    pub(crate) bom: bool,
    pub(crate) default_eol: &'static str,
    pub(crate) eol_mixed: bool,
    pub(crate) final_newline: bool,
    pub(crate) pua_escaped: bool,
    pub(crate) revision: u64,
    pub(crate) len: usize,
    pub(crate) written: bool,
    pub(crate) sensitive: bool,
}

pub(crate) fn eol_name(e: LineEnding) -> &'static str {
    match e {
        LineEnding::Lf => "lf",
        LineEnding::Crlf => "crlf",
        LineEnding::Cr => "cr",
    }
}

/// 書いたプロセスの識別 (スワップの manifest の `pid` / `started_at_*`。復旧が「他のインスタンスが開いている」と残骸を分ける、03 §10.3)。
pub(crate) struct Owner {
    pub pid: u32,
    pub started_at: Option<eve_core::FileTime>,
}

pub(crate) fn manifest_text(
    reason: RescueReason,
    created_at: &str,
    complete: bool,
    docs: &[ManifestDoc],
    owner: Option<&Owner>,
) -> String {
    let mut w = toml_lite::Writer::default();
    w.kv("format_version", &FORMAT_VERSION.to_string());
    w.str(
        "created_by",
        &format!("evoeditor {}", env!("CARGO_PKG_VERSION")),
    );
    w.str("created_at", created_at);
    w.str("reason", reason.as_str());
    w.kv("complete", &complete.to_string());
    if let Some(o) = owner {
        w.kv("pid", &o.pid.to_string());
        if let Some(st) = o.started_at {
            w.kv("started_at_secs", &st.secs.to_string());
            w.kv("started_at_nanos", &st.nanos.to_string());
        }
    }
    for d in docs {
        w.table_array("document");
        w.str("file", &d.file);
        if let Some(p) = &d.path {
            w.str("path", p);
        }
        if let Some(u) = &d.untitled_name {
            w.str("untitled_name", u);
        }
        w.str("encoding", d.encoding);
        w.kv("bom", &d.bom.to_string());
        w.str("default_eol", d.default_eol);
        w.kv("eol_mixed", &d.eol_mixed.to_string());
        w.kv("final_newline", &d.final_newline.to_string());
        w.kv("pua_escaped", &d.pua_escaped.to_string());
        w.kv("revision", &d.revision.to_string());
        w.kv("len", &d.len.to_string());
        w.kv("written", &d.written.to_string());
        if d.sensitive {
            w.kv("sensitive", "true");
        }
    }
    w.finish()
}

/// manifest を原子的に更新する (同じディレクトリの一時ファイルに書いて fsync し、rename。途中で強制終了されても
/// 直前の版か新しい版のどちらかが残る。recovery ディレクトリ内の自前のファイルなので最初から rename、05 §11.2.1)。
pub(crate) fn write_manifest(dir: &Path, text: &str) -> io::Result<()> {
    let tmp = dir.join("manifest.toml.tmp");
    {
        let mut f = crate::sys::create_private(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, dir.join("manifest.toml"))
}

// ---------- 書き手 ----------

/// 本文を 1 つ書く関数 (`write_body` と同じ型)。試験で書込の失敗を差し込むために `write_with` が受ける。
type BodyWriter<'a> = &'a dyn Fn(&Path, &DocSnapshot, &dyn Fn() -> bool) -> io::Result<Option<u8>>;

impl RescueWriter for StateRescue {
    fn write(
        &self,
        docs: &[Arc<DocSnapshot>],
        reason: RescueReason,
        received: Instant,
        budget: Duration,
    ) -> RescueOutcome {
        self.write_with(docs, reason, received, budget, &write_body)
    }
}

/// 退避の各段の経過時間 (Issue #342、docs/review/v0.3-findings.md の SAF-35)。時刻は受信 (`received`) から数える。
/// 状態ディレクトリのログ (`log/evoeditor.log`) に 1 行ずつ書く。ファイルは最初に書くときに 1 回だけ開き、以後は追記だけ
/// (fsync しない。退避の時間を延ばさない)。開けなければログを諦めて退避を続ける。`EVE_LOG` に依らず warn で書く (05 §7.3)。
///
/// - 受信から `SLOW_STAGE` を過ぎた後の各段と、書き終えられなかったときに、それまでの段の時刻をまとめて書く (「遅い」)。
/// - 外から終わらされ得る経路 (CTRL_CLOSE・セッション終了・シグナル。OS や送り手が猶予の後に強制終了する) では、速くても、
///   始めた時点の 1 行 (`begin`) と、各段を終えるたびの 1 行 (「途中」。`PROGRESS_LINES` 行まで) を書く。早くに終わらされても、
///   始めたか・最後にどの段まで進んだかが残る。経路は CAS に勝った経路の `reason` で決まる。
/// - 時間切れの印 (`note`) は積むだけで、書くのは最後の manifest の後 (`finish`)。打切りから最後の manifest までの間に
///   ログの書込を挟まない (書けた本文が `written = false` のまま残る窓を広げない)。
///
/// それ以外の経路 (パニック・Fatal・中止して終了・退避先に保存) の速く終わる退避では何も書かない (ファイルも開かない)。
struct Stages<'a> {
    received: Instant,
    slow_stage: Duration,
    reason: RescueReason,
    state_dir: Option<&'a Path>,
    /// ログのファイル。`None` はまだ開いていない、`Some(None)` は開けなかった。
    log: Option<Option<crate::log::AppendLog>>,
    done: String,
    /// 外から終わらされ得る経路か (始めと各段を、速くても書く)。
    progress: bool,
    /// 書いた「途中」の行の数。
    progress_lines: usize,
}

/// これより遅い退避の段をログに書く。
const SLOW_STAGE: Duration = Duration::from_millis(500);
/// 外から終わらされ得る経路で、速くても書く「途中」の行の上限 (文書が多くてもログの書込で退避を延ばさない。1 文書の退避は
/// ディレクトリ・manifest・本文・完了の manifest の 4 行)。
const PROGRESS_LINES: usize = 8;

/// 外から終わらされ得る経路か (猶予の後に OS や送り手が強制終了する: CTRL_CLOSE は約 5 s、セッション終了、シグナルの送り手が
/// 続けて送る SIGKILL)。
fn killable(reason: RescueReason) -> bool {
    matches!(
        reason,
        RescueReason::ConsoleClose | RescueReason::SessionEnd | RescueReason::Signal
    )
}

impl<'a> Stages<'a> {
    fn new(
        reason: RescueReason,
        state_dir: Option<&'a Path>,
        received: Instant,
        slow_stage: Duration,
    ) -> Stages<'a> {
        Stages {
            received,
            slow_stage,
            reason,
            state_dir,
            log: None,
            done: String::new(),
            progress: killable(reason),
            progress_lines: 0,
        }
    }

    /// 1 行を書く (最初の 1 回だけファイルを開く)。
    fn log_line(&mut self, message: &str) {
        let state_dir = self.state_dir;
        if let Some(log) = self
            .log
            .get_or_insert_with(|| crate::log::AppendLog::open(state_dir))
        {
            log.line(crate::log::Level::Warn, message);
        }
    }

    /// 書き始める。外から終わらされ得る経路では、受信から何 ms で始めたかを 1 行書く。
    fn begin(&mut self, docs: usize, bytes: u64, budget: Duration) {
        if self.progress {
            let line = format!(
                "退避を始めた ({}、受信から {} ms、{docs} 件 {bytes} B、予算 {} ms)",
                self.reason.as_str(),
                self.received.elapsed().as_millis(),
                budget.as_millis()
            );
            self.log_line(&line);
        }
    }

    /// 段 `name` を積む (書かない)。時間切れの印に使う (`finish` の行に載る)。
    fn note(&mut self, name: &str) -> u128 {
        let ms = self.received.elapsed().as_millis();
        if !self.done.is_empty() {
            self.done.push_str(", ");
        }
        self.done.push_str(&format!("{name} {ms} ms"));
        ms
    }

    /// 段 `name` を終えた。遅ければ、ここまでの段をログに書く。外から終わらされ得る経路では、速くてもこの段を書く。
    fn mark(&mut self, name: &str) {
        let ms = self.note(name);
        if self.received.elapsed() >= self.slow_stage {
            self.write("遅い");
        } else if self.progress && self.progress_lines < PROGRESS_LINES {
            self.progress_lines += 1;
            let line = format!(
                "退避の段の時間 ({}、途中): {name} {ms} ms",
                self.reason.as_str()
            );
            self.log_line(&line);
        }
    }

    /// 終わった (最後の manifest の後に呼ぶ)。書き終えられなかったら、ここまでの段 (時間切れの印を含む) と理由
    /// (時間切れ・書込の失敗) をログに書く。
    fn finish(&mut self, out: &RescueOutcome, timed_out: bool) {
        if !out.complete {
            let cause = if timed_out {
                "時間切れ"
            } else {
                "書込の失敗"
            };
            self.write(&format!(
                "書き終えられなかった ({}/{} 件、{cause})",
                out.written, out.total
            ));
        }
    }

    fn write(&mut self, what: &str) {
        let line = format!(
            "退避の段の時間 ({}、{what}): {}",
            self.reason.as_str(),
            self.done
        );
        self.log_line(&line);
    }
}

impl StateRescue {
    /// `write` の本体。時間切れだけが中断で、1 文書の書込の失敗 (権限・ロック) はその文書を `written = false` のまま
    /// 次の文書へ進む (複数文書で、1 件の失敗が他の文書を落とさないため。03 §9.8 の不変条件 5、§10.1)。
    ///
    /// 予算は `received` (受信の時刻) から数え、各文書の頭・本文を作る前・本文のチャンクの間・manifest を書き直す前に確かめる。
    /// 過ぎていたら、書けた分を載せた最後の manifest (`complete = false`) を書いて打ち切る。fsync などの同期の I/O の
    /// 途中では打ち切れない (Issue #342)。
    fn write_with(
        &self,
        docs: &[Arc<DocSnapshot>],
        reason: RescueReason,
        received: Instant,
        budget: Duration,
        body: BodyWriter<'_>,
    ) -> RescueOutcome {
        // test-hooks: `recovery_timeout=<ms>` で退避の段の予算を縮め、時間切れ (`complete = false`) を起こす (09 §3.6)
        let budget = crate::hooks::value("recovery_timeout")
            .map_or(budget, |ms| budget.min(Duration::from_millis(ms)));
        // test-hooks: `rescue_stall_after_manifest=<ms>` で、最初の文書の manifest を書いた直後に、期限が過ぎるまで眠り、
        // さらに `<ms>` だけ眠る (本文を書く前の時間切れ・外からの終了を、時間の大小に頼らずに再現する、09 §3.6)
        let stall = crate::hooks::value("rescue_stall_after_manifest")
            .map(Duration::from_millis)
            .or(self.stall_after_manifest);
        let expired = || received.elapsed() >= budget;
        let mut stages = Stages::new(reason, self.state_dir.as_deref(), received, self.slow_stage);
        let mut out = RescueOutcome {
            total: docs.len(),
            sensitive: docs.iter().any(|d| d.sensitive),
            ..RescueOutcome::default()
        };
        if docs.is_empty() {
            out.complete = true;
            return out;
        }
        let secs = now_secs();
        let total_bytes = docs
            .iter()
            .map(|d| d.snapshot.len_bytes() as u64)
            .sum::<u64>();
        stages.begin(docs.len(), total_bytes, budget);
        // 本文の合計に manifest などの余裕を足した、必要な空き容量
        let needed = total_bytes.saturating_add(1024 * 1024);
        let (dir, fallback) = match open_dir(
            self.state_dir.as_deref(),
            secs,
            reason == RescueReason::UserSave,
            needed,
        ) {
            Ok(x) => x,
            Err(e) => {
                out.error = Some(eve_core::RescueError::Io(e.to_string()));
                stages.mark("ディレクトリ (失敗)");
                stages.finish(&out, false);
                return out;
            }
        };
        stages.mark("ディレクトリ");
        out.dir = Some(dir.clone());
        out.fallback = fallback;
        let created_at = timestamp_rfc3339(secs);

        // 小さい文書から
        let mut order: Vec<&Arc<DocSnapshot>> = docs.iter().collect();
        order.sort_by_key(|d| (d.snapshot.len_bytes(), d.doc));
        let mut entries: Vec<ManifestDoc> = Vec::with_capacity(order.len());
        let publish = |entries: &[ManifestDoc], complete: bool| {
            write_manifest(
                &dir,
                &manifest_text(reason, &created_at, complete, entries, None),
            )
        };
        let mut timed_out = false;
        for (i, d) in order.iter().enumerate() {
            let n = i + 1;
            if expired() {
                timed_out = true;
                stages.note(&format!("時間切れ (文書 {n} の前)"));
                break;
            }
            let base = d
                .path
                .as_ref()
                .and_then(|p| p.as_path().file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .or_else(|| d.untitled_name.clone())
                .unwrap_or_default();
            let file = format!("{n}-{}.txt", safe_basename(&base));
            let mut entry = ManifestDoc {
                file: file.clone(),
                path: d.path.as_ref().map(eve_core::FsPath::display),
                untitled_name: d.untitled_name.clone(),
                encoding: d.format.encoding.name(),
                bom: d.format.has_bom,
                default_eol: eol_name(d.format.default_eol),
                eol_mixed: d.format.eol_stats.is_mixed(),
                final_newline: false,
                pua_escaped: d.format.invalid_bytes > 0,
                revision: d.revision.0,
                len: d.snapshot.len_bytes(),
                written: false,
                sensitive: d.sensitive,
            };
            // 書く前に written = false の項目を載せる (途中で止まっても、書きかけの本文があることが分かる)
            entries.push(entry_clone(&entry));
            if publish(&entries, false).is_err() {
                stages.mark(&format!("manifest {n} (失敗)"));
                entries.pop();
                continue;
            }
            stages.mark(&format!("manifest {n}"));
            if i == 0
                && let Some(extra) = stall
            {
                std::thread::sleep(budget.saturating_sub(received.elapsed()) + extra);
            }
            // 本文を作る前に予算を確かめる (本文のファイルを作らずに打ち切る。manifest には written = false で残る)
            if expired() {
                timed_out = true;
                stages.note(&format!("本文 {n} (時間切れ、書く前)"));
                break;
            }
            let bytes = d.snapshot.len_bytes();
            match body(&dir.join(&file), d, &expired) {
                Ok(Some(last)) => {
                    stages.mark(&format!("本文 {n} ({bytes} B)"));
                    entry.final_newline = matches!(last, b'\n' | b'\r');
                    entry.written = true;
                    *entries.last_mut().expect("直前に積んだ") = entry;
                    out.written += 1;
                    // 最後の文書の書き直しは、下の完了の manifest が兼ねる
                    if n < order.len() {
                        // 書き直す前に予算を確かめる。過ぎていたら書き直さずに、書けた分を完了の manifest に載せて打ち切る
                        if expired() {
                            timed_out = true;
                            stages.note(&format!("時間切れ (manifest {n} の書き直しの前)"));
                            break;
                        }
                        let _ = publish(&entries, false);
                    }
                }
                // 時間切れ: 書けた分の manifest を残して終わる
                Ok(None) => {
                    timed_out = true;
                    stages.note(&format!("本文 {n} (時間切れ、途中)"));
                    break;
                }
                // 書込の失敗: この文書は `written = false` のまま (manifest は書く前に載せたものと同じなので書き直さない)、
                // 次の文書へ (時間切れでなければ続ける)
                Err(e) => {
                    stages.mark(&format!("本文 {n} (失敗)"));
                    out.error = Some(eve_core::RescueError::Io(e.to_string()));
                }
            }
        }
        out.complete = out.written == out.total;
        // 最後の manifest は予算を過ぎても書く。書けなければ、最後の文書は manifest で `written = false` のまま残る
        // (本文は残る) ので、完了にしない
        match publish(&entries, out.complete) {
            Ok(()) => stages.mark("完了の manifest"),
            Err(e) => {
                out.complete = false;
                out.error = Some(eve_core::RescueError::Io(e.to_string()));
                stages.mark("完了の manifest (失敗)");
            }
        }
        stages.finish(&out, timed_out);
        if out.written == 0 && out.total > 0 && out.error.is_none() {
            out.error = Some(eve_core::RescueError::NotWritten);
        }
        out
    }
}

fn entry_clone(e: &ManifestDoc) -> ManifestDoc {
    ManifestDoc {
        file: e.file.clone(),
        path: e.path.clone(),
        untitled_name: e.untitled_name.clone(),
        encoding: e.encoding,
        bom: e.bom,
        default_eol: e.default_eol,
        eol_mixed: e.eol_mixed,
        final_newline: e.final_newline,
        pua_escaped: e.pua_escaped,
        revision: e.revision,
        len: e.len,
        written: e.written,
        sensitive: e.sensitive,
    }
}

/// 本文を書いて fsync する。時間切れなら `Ok(None)`、書き終えたら最後のバイト (空なら 0)。
pub(crate) fn write_body(
    path: &Path,
    d: &DocSnapshot,
    expired: &dyn Fn() -> bool,
) -> io::Result<Option<u8>> {
    let mut f = crate::sys::create_private(path)?;
    let mut last = 0u8;
    for chunk in d.snapshot.chunks(ByteRange::all(d.snapshot.len_bytes())) {
        if expired() {
            return Ok(None);
        }
        f.write_all(chunk.as_bytes())?;
        last = chunk.as_bytes().last().copied().unwrap_or(last);
    }
    f.sync_all()?;
    Ok(Some(last))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 時刻の形() {
        // 2026-10-21 12:34:56 UTC
        let secs = 1_792_586_096;
        assert_eq!(timestamp_basic(secs), "20261021T123456Z");
        assert_eq!(timestamp_rfc3339(secs), "2026-10-21T12:34:56Z");
        assert_eq!(timestamp_basic(0), "19700101T000000Z");
        // うるう年の 2 月 29 日
        assert_eq!(timestamp_basic(1_709_164_800), "20240229T000000Z");
        assert_eq!(timestamp_basic(-1), "19691231T235959Z");
    }

    #[test]
    fn 退避ディレクトリの名前の形() {
        for ok in [
            "20261021T123456Z-1234",
            "20261021T123456Z-1234-2",
            "20261021T123456Z-1-17",
        ] {
            assert!(is_recovery_dir_name(ok), "{ok}");
        }
        for ng in [
            "inflight",
            "rescue",
            "20261021T123456Z",
            "20261021T123456Z-",
            "20261021T123456-1234",
            "20261021T123456Z-12a",
            "20261021T123456Z-1234-1",
            "20261021T123456Z-1234-2-3",
            "2026-10-21T123456Z-1234",
        ] {
            assert!(!is_recovery_dir_name(ng), "{ng}");
        }
    }

    #[test]
    fn ファイル名は予約文字を置き換える() {
        assert_eq!(safe_basename("a:b*c?.txt"), "a_b_c_.txt");
        assert_eq!(safe_basename("..."), "document");
        assert_eq!(safe_basename("日本語.txt"), "日本語.txt");
        assert!(safe_basename(&"あ".repeat(500)).chars().count() <= 100);
        if cfg!(unix) {
            assert!(
                safe_basename(&"あ".repeat(500)).len() <= 200,
                "Unix は 255 バイトの上限に収める"
            );
        }
    }

    #[test]
    fn manifest_の形() {
        let d = ManifestDoc {
            file: "1-a.txt".into(),
            path: Some("C:\\w\\a.txt".into()),
            untitled_name: None,
            encoding: "Shift_JIS",
            bom: false,
            default_eol: "crlf",
            eol_mixed: false,
            final_newline: true,
            pua_escaped: false,
            revision: 42,
            len: 7,
            written: true,
            sensitive: false,
        };
        let t = manifest_text(
            RescueReason::Panic,
            "2026-10-21T00:00:00Z",
            true,
            &[d],
            None,
        );
        assert!(t.starts_with("format_version = 1\n"), "{t}");
        for want in [
            "reason = \"panic\"",
            "complete = true",
            "[[document]]",
            "file = \"1-a.txt\"",
            "path = \"C:\\\\w\\\\a.txt\"",
            "encoding = \"Shift_JIS\"",
            "default_eol = \"crlf\"",
            "revision = 42",
            "written = true",
        ] {
            assert!(t.contains(want), "{want}\n{t}");
        }
        assert!(!t.contains("sensitive"), "false は書かない");
    }

    // STR-D5: TOML の書き手を toml_lite にまとめても、ディスク上の形式はバイト単位で変わらない
    #[test]
    fn manifest_の全文はバイト単位で変わらない() {
        let d = |file: &str, path: Option<&str>, untitled: Option<&str>, sensitive| ManifestDoc {
            file: file.into(),
            path: path.map(Into::into),
            untitled_name: untitled.map(Into::into),
            encoding: "UTF-8",
            bom: true,
            default_eol: "lf",
            eol_mixed: true,
            final_newline: false,
            pua_escaped: true,
            revision: 7,
            len: 3,
            written: false,
            sensitive,
        };
        let owner = Owner {
            pid: 42,
            started_at: Some(eve_core::FileTime {
                secs: 100,
                nanos: 5,
            }),
        };
        let t = manifest_text(
            RescueReason::Signal,
            "2026-10-21T00:00:00Z",
            false,
            &[
                d(
                    "1-a.txt",
                    Some("C:\\w\\\"a\"\t\u{1}\u{7f}.txt"),
                    None,
                    false,
                ),
                d("2-無題.txt", None, Some("無題-2"), true),
            ],
            Some(&owner),
        );
        let want = format!(
            "format_version = 1\ncreated_by = \"evoeditor {}\"\ncreated_at = \"2026-10-21T00:00:00Z\"\n\
             reason = \"signal\"\ncomplete = false\npid = 42\nstarted_at_secs = 100\nstarted_at_nanos = 5\n\
             \n[[document]]\nfile = \"1-a.txt\"\npath = \"C:\\\\w\\\\\\\"a\\\"\\t\\u0001\\u007F.txt\"\n\
             encoding = \"UTF-8\"\nbom = true\ndefault_eol = \"lf\"\neol_mixed = true\nfinal_newline = false\n\
             pua_escaped = true\nrevision = 7\nlen = 3\nwritten = false\n\
             \n[[document]]\nfile = \"2-無題.txt\"\nuntitled_name = \"無題-2\"\n\
             encoding = \"UTF-8\"\nbom = true\ndefault_eol = \"lf\"\neol_mixed = true\nfinal_newline = false\n\
             pua_escaped = true\nrevision = 7\nlen = 3\nwritten = false\nsensitive = true\n",
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(t, want);
        // 所有者なし・文書なし
        let t = manifest_text(RescueReason::Panic, "x", true, &[], None);
        let want = format!(
            "format_version = 1\ncreated_by = \"evoeditor {}\"\ncreated_at = \"x\"\nreason = \"panic\"\ncomplete = true\n",
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(t, want);
    }

    fn scratch(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!("eve-opendir-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// 通常の場所の空きは 100、代替の場所 (名前に `eve-recovery-` を含む) の空きは `alt_free`。
    fn free_of(alt_free: Option<u64>) -> impl Fn(&Path) -> Option<u64> {
        move |p: &Path| {
            if p.to_string_lossy().contains("eve-recovery-") {
                alt_free
            } else {
                Some(100)
            }
        }
    }

    // verifies: AC-NFR-302-1
    #[test]
    fn 通常の場所の空きが足りなければ代替の退避先に書く() {
        let st = scratch("short");
        let (d, fallback) = open_dir_with(
            Some(&st),
            1_792_586_096,
            false,
            1000,
            &free_of(Some(10_000)),
            &|_| Ok(true),
        )
        .unwrap();
        assert!(fallback, "代替の退避先: {}", d.display());
        assert!(d.to_string_lossy().contains("eve-recovery-"));
        // 通常の場所に空のディレクトリを残さない
        let left: Vec<_> = fs::read_dir(st.join("recovery")).unwrap().collect();
        assert!(left.is_empty());
        let _ = fs::remove_dir_all(&st);
        let _ = fs::remove_dir_all(d.parent().unwrap());
    }

    #[test]
    fn 空きが足りていれば通常の場所に書く() {
        let st = scratch("enough");
        let (d, fallback) = open_dir_with(
            Some(&st),
            1_792_586_096,
            false,
            50,
            &free_of(Some(0)),
            &|_| Ok(true),
        )
        .unwrap();
        assert!(!fallback && d.starts_with(&st));
        let _ = fs::remove_dir_all(&st);
    }

    #[test]
    fn どちらも足りないときは通常の場所に書けるだけ書く() {
        let st = scratch("both");
        let (d, fallback) = open_dir_with(
            Some(&st),
            1_792_586_096,
            false,
            1_000_000,
            &free_of(Some(5)),
            &|_| Ok(true),
        )
        .unwrap();
        assert!(!fallback && d.starts_with(&st), "{}", d.display());
        let _ = fs::remove_dir_all(&st);
    }

    #[test]
    fn 空きを調べられなければ通常の場所に書く() {
        let st = scratch("unknown");
        let (_, fallback) = open_dir_with(
            Some(&st),
            1_792_586_096,
            false,
            u64::MAX,
            &|_| None,
            &|_| Ok(true),
        )
        .unwrap();
        assert!(!fallback);
        let _ = fs::remove_dir_all(&st);
    }

    // verifies: AC-NFR-302-1
    #[test]
    fn 代替の退避先の所有者が違えば使わない() {
        // 通常の場所が使えない (state_dir なし) ので代替しか無い
        let e = open_dir_with(None, 1_792_586_096, false, 1, &|_| Some(u64::MAX), &|_| {
            Ok(false)
        })
        .unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn 代替の所有者が違っても通常の場所があれば通常の場所に書く() {
        let st = scratch("foreign");
        let (d, fallback) = open_dir_with(
            Some(&st),
            1_792_586_096,
            false,
            1000,
            &free_of(Some(10_000)),
            &|_| Ok(false),
        )
        .unwrap();
        assert!(!fallback && d.starts_with(&st));
        let _ = fs::remove_dir_all(&st);
    }
}

/// 書込の失敗・遅れを差し込む試験 (本文の書き手 `write_with` は crate の外に出さない、STR-D5)。
#[cfg(all(test, any(windows, target_os = "linux")))]
mod body_writer_tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use eve_core::{DocSnapshot, DocumentId, FsPath, RescueReason, RescueWriter, TextFormat};
    use eve_text::Text;
    use eve_view::Revision;

    use super::{StateRescue, write_body};

    fn tmp(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!("eve-rescue-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// 文書のパス (OS の形の絶対パス)。
    fn abs(rel: &str) -> String {
        if cfg!(windows) {
            format!("C:\\{rel}")
        } else {
            format!("/{rel}")
        }
    }

    fn doc(
        id: u64,
        text: &str,
        path: Option<&str>,
        f: impl FnOnce(&mut TextFormat),
    ) -> Arc<DocSnapshot> {
        let mut format = TextFormat::new_document();
        f(&mut format);
        Arc::new(DocSnapshot {
            doc: DocumentId(id),
            snapshot: Text::from(text).snapshot(),
            format,
            revision: Revision(id * 10),
            saved_node_exists: false,
            sensitive: false,
            path: path.map(FsPath::from),
            untitled_name: path.is_none().then(|| format!("無題-{id}")),
        })
    }

    fn manifest(dir: &Path) -> String {
        fs::read_to_string(dir.join("manifest.toml")).unwrap()
    }

    // 03 §9.8 の不変条件 5: 1 文書の書込の失敗 (権限・ロック) は、その文書だけ `written = false` にして次の文書へ進む。
    // 時間切れだけが中断。書けた文書は退避に残る。
    #[test]
    fn 一つの文書の書込が失敗しても_他の文書は退避する() {
        let state = tmp("one-fails");
        let w = StateRescue::new(Some(state.join("s")));
        // 小さい文書から書くので、"a" (1 バイト) が最初、"bbb" が 2 番目、"cccccc" が最後
        let docs = [
            doc(1, "a", Some(&abs("a")), |_| {}),
            doc(2, "bbb", Some(&abs("b")), |_| {}),
            doc(3, "cccccc", Some(&abs("c")), |_| {}),
        ];
        let fail_second = |path: &Path, d: &DocSnapshot, expired: &dyn Fn() -> bool| {
            if d.doc == DocumentId(2) {
                return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            }
            write_body(path, d, expired)
        };
        let out = w.write_with(
            &docs,
            RescueReason::Signal,
            Instant::now(),
            Duration::from_secs(10),
            &fail_second,
        );
        assert_eq!(out.total, 3);
        assert_eq!(out.written, 2, "失敗した 1 件を除く 2 件は書けた");
        assert!(!out.complete, "全件ではないので complete は false");
        let dir = out.dir.expect("退避先");
        assert_eq!(fs::read_to_string(dir.join("1-a.txt")).unwrap(), "a");
        assert_eq!(fs::read_to_string(dir.join("3-c.txt")).unwrap(), "cccccc");
        let m = manifest(&dir);
        assert!(m.contains("complete = false"), "{m}");
        assert_eq!(m.matches("written = true").count(), 2, "{m}");
        assert_eq!(m.matches("written = false").count(), 1, "{m}");
        // 時間切れではなく書込の失敗であることがログで分かる (Issue #342)
        let text = log_text(&state.join("s"));
        assert!(
            text.contains("本文 2 (失敗)")
                && text.contains("書き終えられなかった (2/3 件、書込の失敗)"),
            "{text}"
        );
        let _ = fs::remove_dir_all(state);
    }

    fn log_text(sd: &Path) -> String {
        fs::read_to_string(sd.join("log").join("evoeditor.log")).unwrap_or_default()
    }

    // Issue #342 (SAF-35): 遅い退避は、各段の経過時間を状態ディレクトリのログに残す (途中で終わっても、どこまで進んだかが分かる)。
    // 外から終わらされない経路 (パニックなど) の速い退避では何も書かない。
    #[test]
    fn 遅い退避は各段の時間をログに残し_速い退避は書かない() {
        let state = tmp("slow-stages");
        let sd = state.join("s");
        let docs = [doc(1, "unsaved text", None, |_| {})];
        // 時間に依らないよう、しきい値を差し替える: 速い退避は 60 秒より速く、遅い退避 (本文の書込で 600 ms 待つ) は 100 ms より遅い
        // (既定の 500 ms のままだと、負荷の高い機械で速い退避がしきい値を超えて落ちた)
        let fast_w = StateRescue::new(Some(sd.clone())).with_slow_stage(Duration::from_secs(60));
        let fast = fast_w.write(
            &docs,
            RescueReason::Panic,
            Instant::now(),
            Duration::from_secs(10),
        );
        assert!(fast.complete);
        assert!(
            !log_text(&sd).contains("退避"),
            "速い退避ではログに書かない"
        );
        let slow_body = |path: &Path, d: &DocSnapshot, expired: &dyn Fn() -> bool| {
            std::thread::sleep(Duration::from_millis(600));
            write_body(path, d, expired)
        };
        let w = StateRescue::new(Some(sd.clone())).with_slow_stage(Duration::from_millis(100));
        let out = w.write_with(
            &docs,
            RescueReason::Panic,
            Instant::now(),
            Duration::from_secs(10),
            &slow_body,
        );
        assert!(out.complete);
        let text = log_text(&sd);
        let line = text
            .lines()
            .find(|l| l.contains("退避の段の時間 (panic、遅い)"))
            .unwrap_or_else(|| panic!("遅い段のログがある: {text}"));
        assert!(
            line.contains("ディレクトリ")
                && line.contains("manifest 1")
                && line.contains("本文 1 (12 B)"),
            "{line}"
        );
        let _ = fs::remove_dir_all(state);
    }

    // Issue #342: 外から終わらされ得る経路 (CTRL_CLOSE など) は、速くても、始めた時点 (受信から何 ms か) と各段をログに残す
    // (OS の猶予切れで早くに終わらされても、始めたか・最後にどの段まで進んだかが分かる)
    #[test]
    fn 外から終わらされ得る経路は速くても始めと各段をログに残す() {
        let state = tmp("progress");
        let sd = state.join("s");
        let w = StateRescue::new(Some(sd.clone())).with_slow_stage(Duration::from_secs(60));
        let out = w.write(
            &[doc(1, "unsaved text", None, |_| {})],
            RescueReason::ConsoleClose,
            Instant::now(),
            Duration::from_secs(10),
        );
        assert!(out.complete);
        let text = log_text(&sd);
        let lines: Vec<&str> = text.lines().collect();
        let begin = lines
            .iter()
            .position(|l| l.contains("退避を始めた (console_close、受信から "))
            .unwrap_or_else(|| panic!("始めの行がある: {text}"));
        assert!(
            lines[begin].contains("1 件 12 B") && lines[begin].contains("予算 10000 ms"),
            "{text}"
        );
        // 始めの行の後に、段ごとの行が順に並ぶ
        let mut at = begin;
        for stage in [
            ": ディレクトリ ",
            ": manifest 1 ",
            ": 本文 1 (12 B) ",
            ": 完了の manifest ",
        ] {
            let found = lines[at + 1..]
                .iter()
                .position(|l| {
                    l.contains("退避の段の時間 (console_close、途中)") && l.contains(stage)
                })
                .unwrap_or_else(|| panic!("{stage} の行が順に並ぶ: {text}"));
            at += 1 + found;
        }
        assert!(
            !text.contains("遅い") && !text.contains("書き終えられなかった"),
            "{text}"
        );
        let _ = fs::remove_dir_all(state);
    }

    // Issue #342: manifest 1 の後、本文を作る前に予算が尽きたら、本文のファイルを作らずに打ち切り、最後の manifest
    // (`complete = false`、`written = false`) を書く。ログは「時間切れ」で、書込の失敗と見分けられる
    #[test]
    fn 本文を作る前に予算が尽きたら本文を作らずに_complete_false_の_manifest_を残す() {
        let state = tmp("before-body");
        let sd = state.join("s");
        let w = StateRescue::new(Some(sd.clone()))
            .with_slow_stage(Duration::from_secs(60))
            // 期限が過ぎるまで眠り、さらに 10 ms 眠る (時間の大小に頼らない。予算は負荷の下でも manifest 1 まで届く長さ)
            .with_stall_after_manifest(Duration::from_millis(10));
        let out = w.write(
            &[doc(1, "unsaved text", None, |_| {})],
            RescueReason::ConsoleClose,
            Instant::now(),
            Duration::from_secs(3),
        );
        assert!(!out.complete && out.written == 0, "{out:?}");
        assert_eq!(out.error, Some(eve_core::RescueError::NotWritten));
        let dir = out.dir.expect("退避先");
        let m = manifest(&dir);
        assert!(
            m.contains("complete = false") && m.contains("written = false"),
            "{m}"
        );
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["manifest.toml"], "本文のファイルを作らない");
        let text = log_text(&sd);
        assert!(text.contains("本文 1 (時間切れ、書く前)"), "{text}");
        assert!(
            text.contains("書き終えられなかった (0/1 件、時間切れ)"),
            "{text}"
        );
        let _ = fs::remove_dir_all(state);
    }

    // 予算は受信の時刻から数える (05 §11.2 の時系列表の「受信から t = 3.5 s」)。受信から予算を過ぎて呼ばれたら、
    // 1 件も書かずに `complete = false` の manifest を残す
    #[test]
    fn 予算は受信の時刻から数える() {
        let state = tmp("received");
        let sd = state.join("s");
        let received = Instant::now()
            .checked_sub(Duration::from_millis(500))
            .expect("500 ms 前の時刻");
        let w = StateRescue::new(Some(sd.clone())).with_slow_stage(Duration::from_secs(60));
        let out = w.write(
            &[doc(1, "x", None, |_| {})],
            RescueReason::Signal,
            received,
            Duration::from_millis(200),
        );
        assert!(!out.complete && out.written == 0, "{out:?}");
        let m = manifest(&out.dir.expect("退避先"));
        assert!(m.contains("complete = false"), "{m}");
        let text = log_text(&sd);
        let begin = text
            .lines()
            .find(|l| l.contains("退避を始めた (signal、受信から "))
            .unwrap_or_else(|| panic!("{text}"));
        let ms: u64 = begin
            .split("受信から ")
            .nth(1)
            .and_then(|r| r.split(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("{begin}"));
        assert!(ms >= 500, "受信からの時間を書く: {begin}");
        assert!(text.contains("時間切れ (文書 1 の前)"), "{text}");
        let _ = fs::remove_dir_all(state);
    }

    // manifest を書き直す前にも予算を確かめる: 過ぎていたら書き直さずに、書けた文書を最後の manifest (`complete = false`) に
    // 載せて打ち切る (残りの文書には進まない)
    #[test]
    fn manifest_の書き直しの前に予算が尽きたら書けた分を最後の_manifest_に載せて打ち切る() {
        let state = tmp("before-rewrite");
        let sd = state.join("s");
        let docs = [
            doc(1, "a", Some(&abs("a")), |_| {}),
            doc(2, "bbb", Some(&abs("b")), |_| {}),
        ];
        // 1 件目の本文は書き終えるが、期限が過ぎるまで待ってから書き終える (時間の大小に頼らない)
        let slow_first = |path: &Path, _d: &DocSnapshot, expired: &dyn Fn() -> bool| {
            while !expired() {
                std::thread::sleep(Duration::from_millis(5));
            }
            fs::write(path, b"a")?;
            Ok(Some(b'a'))
        };
        let w = StateRescue::new(Some(sd.clone())).with_slow_stage(Duration::from_secs(60));
        let out = w.write_with(
            &docs,
            RescueReason::ConsoleClose,
            Instant::now(),
            Duration::from_secs(3),
            &slow_first,
        );
        assert_eq!((out.written, out.total, out.complete), (1, 2, false));
        let dir = out.dir.expect("退避先");
        let m = manifest(&dir);
        assert!(m.contains("complete = false"), "{m}");
        assert_eq!(
            m.matches("[[document]]").count(),
            1,
            "2 件目には進まない: {m}"
        );
        assert!(m.contains("written = true"), "{m}");
        assert!(!dir.join("2-b.txt").exists());
        let text = log_text(&sd);
        assert!(
            text.contains("時間切れ (manifest 1 の書き直しの前)"),
            "{text}"
        );
        assert!(
            text.contains("書き終えられなかった (1/2 件、時間切れ)"),
            "{text}"
        );
        let _ = fs::remove_dir_all(state);
    }

    // 最後の manifest が書けなければ完了にしない (最後の文書の書き直しは最後の manifest が兼ねるので、manifest では
    // `written = false` のまま残る。本文は残る)
    #[test]
    fn 最後の_manifest_が書けなければ完了にせず誤りを返す() {
        let state = tmp("final-manifest");
        let sd = state.join("s");
        // 本文を書いた後に、manifest の一時ファイルの名前をディレクトリで塞ぐ (最後の manifest の作成が失敗する)
        let block_manifest = |path: &Path, d: &DocSnapshot, expired: &dyn Fn() -> bool| {
            let r = write_body(path, d, expired);
            fs::create_dir(path.parent().unwrap().join("manifest.toml.tmp"))?;
            r
        };
        let w = StateRescue::new(Some(sd.clone())).with_slow_stage(Duration::from_secs(60));
        let out = w.write_with(
            &[doc(1, "unsaved text", None, |_| {})],
            RescueReason::ConsoleClose,
            Instant::now(),
            Duration::from_secs(10),
            &block_manifest,
        );
        assert_eq!((out.written, out.total), (1, 1));
        assert!(!out.complete, "{out:?}");
        assert!(
            matches!(out.error, Some(eve_core::RescueError::Io(_))),
            "{out:?}"
        );
        let dir = out.dir.expect("退避先");
        let m = manifest(&dir);
        assert!(
            m.contains("complete = false") && m.contains("written = false"),
            "manifest 1 のまま: {m}"
        );
        assert_eq!(
            fs::read_to_string(dir.join("1-無題-1.txt")).unwrap(),
            "unsaved text",
            "本文は残る"
        );
        let text = log_text(&sd);
        assert!(
            text.contains("完了の manifest (失敗)")
                && text.contains("書き終えられなかった (1/1 件、書込の失敗)"),
            "{text}"
        );
        let _ = fs::remove_dir_all(state);
    }
}
