//! 緊急退避の書き手 (docs/02 §11.2.1、05 §7.5 / §5.6)。`eve_core::RescueWriter` の Windows 実装。
//!
//! 公開済みの `DocSnapshot` だけを読み、`std::fs` を直接使って (Port を通さない) 退避先に書く。**小さい文書から**、1 文書書くごとに
//! `manifest.toml` を原子的に (一時ファイル + fsync + rename) 更新し、時間予算内に書けた分が確実に残るようにする。
//! 退避先は `state_dir/recovery/<UTC timestamp>-<pid>[-<n>]/`。状態ディレクトリに書けなければ、代替の退避先
//! `%TEMP%\eve-recovery-<SID>\` (所有者のみの DACL、05 §7.5) に書く。作るディレクトリは所有者のみ (中のファイルは継承)。
//!
//! 本文は内部の UTF-8 のまま・改行を正規化せず・不正バイトの PUA エスケープを保つ (元の文字コードへの再エンコードはしない)。
//! 範囲外 (後続): 書く前の空き容量の確認、代替の退避先の既存ディレクトリの所有者検査 (05 §7.5)、30 日を超えた退避の案内。

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eve_core::{DocSnapshot, RescueOutcome, RescueReason, RescueWriter};
use eve_text::{ByteRange, LineEnding, TextRead};

use crate::inflight::{create_dir_all_private, toml_str};
use crate::save::current_user_sid;

/// manifest の版 (意味の変更・必須キーの追加で上げる。キーの追加は版を上げない)。
const FORMAT_VERSION: u32 = 1;

/// 緊急退避の書き手。
#[derive(Clone, Debug, Default)]
pub struct WinRescue {
    state_dir: Option<PathBuf>,
}

impl WinRescue {
    /// `state_dir` は 05 §7.5 の状態ディレクトリ (無ければ代替の退避先だけに書く)。
    pub fn new(state_dir: Option<PathBuf>) -> WinRescue {
        WinRescue { state_dir }
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

fn now_secs() -> i64 {
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
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::from(io::ErrorKind::AlreadyExists))
}

/// 退避先を決めて作る。通常の場所 (`state_dir/recovery`) に作れなければ代替の退避先。戻り値は (ディレクトリ, 代替か)。
/// `rescue` は「退避先に保存」(`reason = "user_save"`) で、`recovery/rescue/` の下に作る (05 §7.5)。
fn open_dir(state_dir: Option<&Path>, secs: i64, rescue: bool) -> io::Result<(PathBuf, bool)> {
    let sub = |base: PathBuf| if rescue { base.join("rescue") } else { base };
    if let Some(sd) = state_dir
        && let Ok(d) = create_unique_dir(&sub(sd.join("recovery")), secs)
    {
        return Ok((d, false));
    }
    let sid = current_user_sid()?;
    let base = std::env::temp_dir().join(format!("eve-recovery-{sid}"));
    create_unique_dir(&sub(base), secs).map(|d| (d, true))
}

/// 本文のファイル名の `<basename>` に使える形にする (予約文字・制御文字を `_`、長さを制限)。
fn safe_basename(name: &str) -> String {
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
    if let Ok(sid) = current_user_sid() {
        let base = std::env::temp_dir().join(format!("eve-recovery-{sid}"));
        scan_base(&base, false, &mut out);
        scan_base(&base.join("rescue"), true, &mut out);
    }
    out
}

// ---------- manifest ----------

struct ManifestDoc {
    file: String,
    path: Option<String>,
    untitled_name: Option<String>,
    encoding: &'static str,
    bom: bool,
    default_eol: &'static str,
    eol_mixed: bool,
    final_newline: bool,
    pua_escaped: bool,
    revision: u64,
    len: usize,
    written: bool,
    sensitive: bool,
}

fn eol_name(e: LineEnding) -> &'static str {
    match e {
        LineEnding::Lf => "lf",
        LineEnding::Crlf => "crlf",
        LineEnding::Cr => "cr",
    }
}

fn manifest_text(
    reason: RescueReason,
    created_at: &str,
    complete: bool,
    docs: &[ManifestDoc],
) -> String {
    let mut t = String::new();
    let kv = |t: &mut String, k: &str, v: String| {
        t.push_str(k);
        t.push_str(" = ");
        t.push_str(&v);
        t.push('\n');
    };
    kv(&mut t, "format_version", FORMAT_VERSION.to_string());
    kv(
        &mut t,
        "created_by",
        toml_str(&format!("evoeditor {}", env!("CARGO_PKG_VERSION"))),
    );
    kv(&mut t, "created_at", toml_str(created_at));
    kv(&mut t, "reason", toml_str(reason.as_str()));
    kv(&mut t, "complete", complete.to_string());
    for d in docs {
        t.push_str("\n[[document]]\n");
        kv(&mut t, "file", toml_str(&d.file));
        if let Some(p) = &d.path {
            kv(&mut t, "path", toml_str(p));
        }
        if let Some(u) = &d.untitled_name {
            kv(&mut t, "untitled_name", toml_str(u));
        }
        kv(&mut t, "encoding", toml_str(d.encoding));
        kv(&mut t, "bom", d.bom.to_string());
        kv(&mut t, "default_eol", toml_str(d.default_eol));
        kv(&mut t, "eol_mixed", d.eol_mixed.to_string());
        kv(&mut t, "final_newline", d.final_newline.to_string());
        kv(&mut t, "pua_escaped", d.pua_escaped.to_string());
        kv(&mut t, "revision", d.revision.to_string());
        kv(&mut t, "len", d.len.to_string());
        kv(&mut t, "written", d.written.to_string());
        if d.sensitive {
            kv(&mut t, "sensitive", "true".to_owned());
        }
    }
    t
}

/// manifest を原子的に更新する (同じディレクトリの一時ファイルに書いて fsync し、rename。途中で強制終了されても
/// 直前の版か新しい版のどちらかが残る。recovery ディレクトリ内の自前のファイルなので最初から rename、05 §11.2.1)。
fn write_manifest(dir: &Path, text: &str) -> io::Result<()> {
    let tmp = dir.join("manifest.toml.tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, dir.join("manifest.toml"))
}

// ---------- 書き手 ----------

impl RescueWriter for WinRescue {
    fn write(
        &self,
        docs: &[Arc<DocSnapshot>],
        reason: RescueReason,
        budget: Duration,
    ) -> RescueOutcome {
        let start = Instant::now();
        let expired = || start.elapsed() >= budget;
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
        let (dir, fallback) = match open_dir(
            self.state_dir.as_deref(),
            secs,
            reason == RescueReason::UserSave,
        ) {
            Ok(x) => x,
            Err(e) => {
                out.error = Some(e.to_string());
                return out;
            }
        };
        out.dir = Some(dir.clone());
        out.fallback = fallback;
        let created_at = timestamp_rfc3339(secs);

        // 小さい文書から
        let mut order: Vec<&Arc<DocSnapshot>> = docs.iter().collect();
        order.sort_by_key(|d| (d.snapshot.len_bytes(), d.doc));
        let mut entries: Vec<ManifestDoc> = Vec::with_capacity(order.len());
        let publish = |entries: &[ManifestDoc], complete: bool| {
            write_manifest(&dir, &manifest_text(reason, &created_at, complete, entries))
        };
        for (i, d) in order.iter().enumerate() {
            if expired() {
                break;
            }
            let base = d
                .path
                .as_ref()
                .and_then(|p| p.as_path().file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .or_else(|| d.untitled_name.clone())
                .unwrap_or_default();
            let file = format!("{}-{}.txt", i + 1, safe_basename(&base));
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
                entries.pop();
                continue;
            }
            match write_body(&dir.join(&file), d, &expired) {
                Ok(Some(last)) => {
                    entry.final_newline = matches!(last, b'\n' | b'\r');
                    entry.written = true;
                    *entries.last_mut().expect("直前に積んだ") = entry;
                    out.written += 1;
                    let _ = publish(&entries, false);
                }
                // 時間切れ・書込の失敗: 書けた分の manifest を残して終わる
                Ok(None) | Err(_) => break,
            }
        }
        out.complete = out.written == out.total;
        let _ = publish(&entries, out.complete);
        if out.written == 0 && out.total > 0 && out.error.is_none() {
            out.error = Some("書き込めませんでした".to_owned());
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
fn write_body(path: &Path, d: &DocSnapshot, expired: &dyn Fn() -> bool) -> io::Result<Option<u8>> {
    let mut f = File::create(path)?;
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
        let t = manifest_text(RescueReason::Panic, "2026-10-21T00:00:00Z", true, &[d]);
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
}
