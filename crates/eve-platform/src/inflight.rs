//! 保存中フラグ (docs/05 §4.6 / §7.5、02 §4.5)。保存ジョブの開始時 (一時ファイルを作る前) に
//! `state_dir/recovery/inflight/<hash(path)>/<pid>-<seq>.toml` を書き、成功時に消す。開くジョブが `inflight_for` で読む。
//!
//! フラグ自身は `write_atomic` を通さず、ここの下位関数で直接書く (フラグを書くためにフラグを書かない)。TOML・版付き。
//! `hash(path)` は 05 §7.5 の正規形 (親を `canonicalize` + ファイル名、verbatim 接頭辞を外し、大文字小文字を畳み、`\` に揃え、
//! 末尾の区切りを除く) を UTF-16 単位列にして XXH3-64。シンボリックリンクはリンク自身とリンク先の 2 つのキーで照会する。

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use eve_core::{FileTime, FsPath, InflightRecord, SaveMethod, SaveWarning};

#[cfg(windows)]
use crate::path::simplify_verbatim;
use crate::sys::create_dir_private;
use crate::toml_lite::{self, parse_basic_string};

/// フラグの版 (キーの追加は版を上げずに行ってよい。意味の変更・必須キーの追加で上げる)。
const FORMAT_VERSION: u32 = 1;

/// プロセス内の保存ジョブごとの通し番号 (同じプロセスの前の保存が残したフラグを上書きしない)。
static SEQ: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

// ---------- キー ----------

/// フラグのキーの入力となる正規形 (親を `canonicalize` し、ファイル名を付け直す。親が無ければ `None`)。
pub(crate) fn normalized(path: &Path) -> Option<String> {
    let name = path.file_name()?;
    let parent = path.parent()?;
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let parent = fs::canonicalize(parent).ok()?;
    let joined = parent.join(name);
    Some(normal_form(&joined.to_string_lossy()))
}

/// Windows は大文字小文字を区別しない形に畳み、区切りは `\` に揃え、末尾の区切りを除く。
#[cfg(windows)]
fn normal_form(s: &str) -> String {
    let s = simplify_verbatim(s).replace('/', "\\").to_uppercase();
    s.trim_end_matches('\\').to_owned()
}

/// Unix は大文字小文字を区別する FS が既定なので畳まない (05 §2 の表)。末尾の区切りだけを除く。
#[cfg(unix)]
fn normal_form(s: &str) -> String {
    let t = s.trim_end_matches('/');
    if t.is_empty() {
        "/".to_owned()
    } else {
        t.to_owned()
    }
}

/// `hash(path)` (XXH3-64 の 16 桁の 16 進)。正規形の UTF-16 単位列 (LE) を入力にする。
pub(crate) fn flag_key(path: &Path) -> Option<String> {
    let n = normalized(path)?;
    let bytes: Vec<u8> = n.encode_utf16().flat_map(u16::to_le_bytes).collect();
    Some(format!("{:016x}", xxhash_rust::xxh3::xxh3_64(&bytes)))
}

/// 照会するキー: 開くパス自身と、シンボリックリンクならリンク先 (ダングリングは親を正規化し直したパス + 名前)。
pub(crate) fn query_keys(path: &Path) -> Vec<String> {
    let mut keys = Vec::new();
    let mut push = |k: Option<String>| {
        if let Some(k) = k
            && !keys.contains(&k)
        {
            keys.push(k);
        }
    };
    push(flag_key(path));
    if let Ok(sm) = fs::symlink_metadata(path)
        && sm.file_type().is_symlink()
    {
        // 存在するリンク先は最終リンク先、無ければ readlink のリンク先 (相対ならリンクの親から)
        let resolved = fs::canonicalize(path).ok().or_else(|| {
            let t = fs::read_link(path).ok()?;
            Some(if t.is_absolute() {
                t
            } else {
                path.parent()?.join(t)
            })
        });
        push(resolved.and_then(|t| flag_key(&t)));
    }
    keys
}

// ---------- 書き手 ----------

/// 書くフラグの内容。
pub(crate) struct FlagBody<'a> {
    pub target: &'a Path,
    pub method: SaveMethod,
    pub tmp: Option<&'a Path>,
    pub backup: Option<&'a Path>,
    pub backup_recorded: bool,
    pub backup_complete: bool,
}

fn method_name(m: SaveMethod) -> &'static str {
    match m {
        SaveMethod::Atomic => "atomic",
        SaveMethod::InPlace => "in_place",
        SaveMethod::NewFile => "new_file",
    }
}

/// プロセスの作成時刻 (UNIX 紀元からの経過、Windows: `GetProcessTimes`)。取れなければ `None`。
#[cfg(windows)]
pub(crate) fn process_start() -> Option<FileTime> {
    crate::sys::process_start()
}

/// プロセスの作成時刻 (Unix: `/proc/self/stat`)。取れなければ `None`。
#[cfg(unix)]
pub(crate) fn process_start() -> Option<FileTime> {
    crate::sys::process_start_of(std::process::id())
}

/// `state_dir/recovery/inflight/<key>/` を所有者のみの DACL で作る (中のファイルは継承で所有者のみ、05 §5.6)。
pub(crate) fn create_flag_dir(state_dir: &Path, key: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(state_dir)?;
    let dir = state_dir.join("recovery").join("inflight").join(key);
    create_dir_all_private(&dir)?;
    Ok(dir)
}

/// 所有者のみのディレクトリを親から順に作る (Windows: 所有者と SYSTEM のみの保護付き DACL で中へ継承、Unix: 0700。05 §5.6)。
/// 既にあれば何もしない。
pub(crate) fn create_dir_all_private(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    if let Some(parent) = dir.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        create_dir_all_private(parent)?;
    }
    create_dir_private(dir)
}

/// 書いたフラグを同じパスのまま書き直す (一時名 + 名前の変更。05 §4.4 手順 4 の `backup_complete = true`)。
pub(crate) fn rewrite_flag(path: &Path, body: &FlagBody<'_>) -> io::Result<()> {
    write_replace(path, flag_text(body).as_bytes())
}

/// フラグの内容 (TOML)。
fn flag_text(body: &FlagBody<'_>) -> String {
    let mut w = toml_lite::Writer::default();
    w.kv("format_version", &FORMAT_VERSION.to_string());
    w.str("target", &body.target.to_string_lossy());
    w.kv("pid", &std::process::id().to_string());
    if let Some(t) = process_start() {
        w.kv("started_at_secs", &t.secs.to_string());
        w.kv("started_at_nanos", &t.nanos.to_string());
    }
    w.str("method", method_name(body.method));
    if let Some(t) = body.tmp {
        w.str("tmp", &t.to_string_lossy());
    }
    if let Some(b) = body.backup {
        w.str("backup", &b.to_string_lossy());
    }
    w.kv("backup_recorded", &body.backup_recorded.to_string());
    w.kv("backup_complete", &body.backup_complete.to_string());
    w.finish()
}

/// フラグを書く (一時名に書いて fsync し、名前を変える。照合も保存中フラグも使わない下位関数)。書いたフラグのパスを返す。
pub(crate) fn write_flag(state_dir: &Path, key: &str, body: &FlagBody<'_>) -> io::Result<PathBuf> {
    let dir = create_flag_dir(state_dir, key)?;
    let name = format!("{}-{}.toml", std::process::id(), next_seq());
    let path = dir.join(&name);
    let text = flag_text(body);
    write_replace(&path, text.as_bytes())?;
    Ok(path)
}

/// 同じディレクトリの一時名に書いて fsync し、名前を変える。
fn write_replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = crate::sys::create_private(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

// ---------- 読み手 ----------

/// フラグ 1 つを読む。解析できなければ `malformed = true` (`target` は照会したパス、`pid` はファイル名から)。
fn parse_flag(flag: &Path, queried: &Path) -> InflightRecord {
    let pid_from_name = flag
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.split('-').next())
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    let malformed_rec = |tmp: Option<FsPath>| InflightRecord {
        target: FsPath::new(queried),
        pid: pid_from_name,
        started_at: None,
        method: SaveMethod::Atomic,
        tmp,
        backup: None,
        backup_recorded: false,
        backup_complete: false,
        flag: FsPath::new(flag),
        malformed: true,
    };
    let Ok(bytes) = fs::read(flag) else {
        return malformed_rec(None);
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return malformed_rec(None);
    };
    let (mut version, mut target, mut pid, mut method) = (None, None, None, None);
    let (mut tmp, mut backup) = (None::<String>, None::<String>);
    let (mut recorded, mut complete) = (None, None);
    let (mut s_secs, mut s_nanos) = (None::<i64>, None::<u32>);
    for l in text.lines() {
        let Some((k, v)) = toml_lite::key_value(l) else {
            continue;
        };
        match k {
            "format_version" => version = v.parse::<u32>().ok(),
            "target" => target = parse_basic_string(v),
            "pid" => pid = v.parse::<u32>().ok(),
            "method" => method = parse_basic_string(v),
            "tmp" => tmp = parse_basic_string(v),
            "backup" => backup = parse_basic_string(v),
            "backup_recorded" => recorded = v.parse::<bool>().ok(),
            "backup_complete" => complete = v.parse::<bool>().ok(),
            "started_at_secs" => s_secs = v.parse().ok(),
            "started_at_nanos" => s_nanos = v.parse().ok(),
            _ => {} // 未知のキーは無視する
        }
    }
    let tmp_path = tmp.as_deref().map(FsPath::from);
    let method = match method.as_deref() {
        Some("atomic") => Some(SaveMethod::Atomic),
        Some("in_place") => Some(SaveMethod::InPlace),
        Some("new_file") => Some(SaveMethod::NewFile),
        _ => None,
    };
    // 未知の版・必須キーの欠落・値の不正
    let (
        Some(FORMAT_VERSION),
        Some(target),
        Some(pid),
        Some(method),
        Some(recorded),
        Some(complete),
    ) = (version, target, pid, method, recorded, complete)
    else {
        return malformed_rec(tmp_path);
    };
    InflightRecord {
        target: FsPath::from(target.as_str()),
        pid,
        started_at: s_secs.map(|secs| FileTime {
            secs,
            nanos: s_nanos.unwrap_or(0),
        }),
        method,
        tmp: tmp_path,
        backup: backup.as_deref().map(FsPath::from),
        backup_recorded: recorded,
        backup_complete: complete,
        flag: FsPath::new(flag),
        malformed: false,
    }
}

/// `inflight_for`: 開くパス (とリンク先) のキーのディレクトリだけを読む。ディレクトリが無ければ空。
pub(crate) fn read_flags(state_dir: &Path, path: &Path) -> Vec<InflightRecord> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for key in query_keys(path) {
        let dir = state_dir.join("recovery").join("inflight").join(&key);
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "toml") && seen.insert(p.clone()) {
                out.push(parse_flag(&p, path));
            }
        }
    }
    out.sort_by(|a, b| a.flag.cmp(&b.flag));
    out
}

/// `list_inflight`: `recovery/inflight/*/*.toml` をすべて読む。書いたプロセスが動いているもの (自分を含む) は除く。
pub(crate) fn list_all(state_dir: &Path) -> Vec<InflightRecord> {
    let Ok(keys) = fs::read_dir(state_dir.join("recovery").join("inflight")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for k in keys.flatten() {
        let Ok(rd) = fs::read_dir(k.path()) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "toml") {
                continue;
            }
            let rec = parse_flag(&p, Path::new(""));
            // 解析できないフラグは起動時刻を読めない。pid だけで生存を見る (動いていれば保存中かもしれないので除く)
            if crate::swap_scan::process_alive(rec.pid, rec.started_at) {
                continue;
            }
            out.push(rec);
        }
    }
    out.sort_by(|a, b| a.flag.cmp(&b.flag));
    out
}

/// 同じキーのディレクトリに残る、自分の今回の記録以外の記録を消す (05 §4.1 手順 7)。
/// 記録された一時ファイル・退避名が実在する記録は残す。
pub(crate) fn clear_others(state_dir: &Path, key: &str, mine: &Path) {
    let dir = state_dir.join("recovery").join("inflight").join(key);
    let Ok(rd) = fs::read_dir(&dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p == mine || p.extension().is_none_or(|x| x != "toml") {
            continue;
        }
        let rec = parse_flag(&p, Path::new(""));
        // 痕跡は一時ファイルと、置換の退避名 (`.eve-bak-*`)。直接上書きの事前バックアップ (`state_dir/backup/`) は含めない
        // (02 §4.5「報告した記録を消す時機」。バックアップは利用者が手で扱う)
        let backup = if rec.method == SaveMethod::InPlace {
            None
        } else {
            rec.backup.as_ref()
        };
        let traces = [rec.tmp.as_ref(), backup]
            .into_iter()
            .flatten()
            .any(|t| t.as_path().exists());
        if !traces {
            let _ = fs::remove_file(&p);
        }
    }
}

// ---------- 保存の側のフラグ ----------

/// 保存中フラグ (05 §4.1 手順 0d / 7 / 8)。状態ディレクトリが無い・状態ファイルの書込 (`state_file`) ではフラグを書かない。
pub(crate) struct FlagGuard {
    pub(crate) state_dir: Option<PathBuf>,
    pub(crate) key: Option<String>,
    pub(crate) path: Option<PathBuf>,
}

impl FlagGuard {
    /// 中断の後片付けで一時ファイルを消せなかった: その名前を状態ディレクトリのログに残す (05 §4.1 手順 8。本文は含まれるので
    /// 「どこにあるか」だけ。docs/review/v0.3-findings.md の SAF-41)。保存中フラグは残っているので、次の起動の走査も見つける。
    pub(crate) fn log_leftover_tmp(&self, tmp: &Path, err: &std::io::Error) {
        crate::log::Logger::with_level(self.state_dir.as_deref(), crate::log::Level::Warn).log(
            crate::log::Level::Warn,
            &format!(
                "中断した保存の一時ファイルを消せませんでした: {} ({err})",
                tmp.display()
            ),
        );
    }

    /// 一時ファイルを作る前に書く。書けなければ警告して続ける (原子的置換では T が無傷のまま残るため)。
    pub(crate) fn begin(
        state_dir: Option<&Path>,
        state_file: bool,
        target: &Path,
        method: SaveMethod,
        tmp: &Path,
        bak: Option<&Path>,
        warnings: &mut Vec<SaveWarning>,
    ) -> FlagGuard {
        let mut g = FlagGuard {
            state_dir: None,
            key: None,
            path: None,
        };
        let Some(sd) = state_dir.filter(|_| !state_file) else {
            return g;
        };
        g.state_dir = Some(sd.to_path_buf());
        g.key = flag_key(target);
        let written = g.key.as_deref().and_then(|key| {
            write_flag(
                sd,
                key,
                &FlagBody {
                    target,
                    method,
                    tmp: Some(tmp),
                    backup: bak,
                    backup_recorded: false,
                    backup_complete: false,
                },
            )
            .ok()
        });
        match written {
            Some(p) => g.path = Some(p),
            None => warnings.push(SaveWarning::InflightFlagNotWritten),
        }
        g
    }

    /// 直接上書きのフラグ (05 §4.4 手順 2)。書けなければ `None` (呼び側が `BackupFailed` にするか、警告で続ける)。
    pub(crate) fn begin_in_place(
        state_dir: &Path,
        target: &Path,
        backup: Option<&Path>,
    ) -> Option<FlagGuard> {
        let key = flag_key(target)?;
        let path = write_flag(
            state_dir,
            &key,
            &FlagBody {
                target,
                method: SaveMethod::InPlace,
                tmp: None,
                backup,
                backup_recorded: backup.is_some(),
                backup_complete: false,
            },
        )
        .ok()?;
        Some(FlagGuard {
            state_dir: Some(state_dir.to_path_buf()),
            key: Some(key),
            path: Some(path),
        })
    }

    /// 事前バックアップを最終名にした後、フラグを `backup_complete = true` で書き直す。
    pub(crate) fn mark_backup_complete(&self, target: &Path, backup: &Path) -> io::Result<()> {
        let Some(p) = &self.path else {
            return Ok(());
        };
        rewrite_flag(
            p,
            &FlagBody {
                target,
                method: SaveMethod::InPlace,
                tmp: None,
                backup: Some(backup),
                backup_recorded: true,
                backup_complete: true,
            },
        )
    }

    /// 自分のフラグを消す。
    pub(crate) fn remove(&mut self) {
        if let Some(p) = self.path.take() {
            let _ = fs::remove_file(p);
        }
    }

    /// 成功: 自分のフラグと、同じキーに残る他の記録 (痕跡ファイルが実在しないもの) を消す。
    pub(crate) fn finish_success(&mut self) {
        let mine = self.path.clone();
        self.remove();
        if let (Some(sd), Some(key), Some(mine)) = (&self.state_dir, &self.key, mine) {
            clear_others(sd, key, &mine);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!("eve-inflight-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    // STR-D5: TOML の読み書きを toml_lite にまとめても、フラグの形式はバイト単位で変わらず、同じに読める
    #[test]
    fn フラグの全文はバイト単位で変わらない() {
        let body = FlagBody {
            target: Path::new("C:\\w\\\"日本\"\n.txt"),
            method: SaveMethod::InPlace,
            tmp: Some(Path::new("C:\\w\\t.tmp")),
            backup: Some(Path::new("C:\\b\\x")),
            backup_recorded: true,
            backup_complete: false,
        };
        let started = process_start()
            .map(|t| {
                format!(
                    "started_at_secs = {}\nstarted_at_nanos = {}\n",
                    t.secs, t.nanos
                )
            })
            .unwrap_or_default();
        let want = format!(
            "format_version = 1\ntarget = \"C:\\\\w\\\\\\\"日本\\\"\\n.txt\"\npid = {}\n{started}\
             method = \"in_place\"\ntmp = \"C:\\\\w\\\\t.tmp\"\nbackup = \"C:\\\\b\\\\x\"\n\
             backup_recorded = true\nbackup_complete = false\n",
            std::process::id()
        );
        assert_eq!(flag_text(&body), want);
        let body = FlagBody {
            target: Path::new("/a"),
            method: SaveMethod::NewFile,
            tmp: None,
            backup: None,
            backup_recorded: false,
            backup_complete: true,
        };
        let want = format!(
            "format_version = 1\ntarget = \"/a\"\npid = {}\n{started}method = \"new_file\"\n\
             backup_recorded = false\nbackup_complete = true\n",
            std::process::id()
        );
        assert_eq!(flag_text(&body), want);
    }

    #[test]
    fn フラグの読みは空白と未知のキーと空の値を同じに扱う() {
        let d = tmp_dir("parse-lite");
        let f = d.join("9-1.toml");
        // 行頭の空白・値の後の空白は許し、空の値は前の値を消す (後勝ち)、`=` の前後に空白が無い行は無視する
        fs::write(
            &f,
            "  format_version = 1  \ntarget = \"/x\"\ntarget=\"/y\"\npid = 9\npid = \nmethod = \"atomic\"\n\
             backup_recorded = false\nbackup_complete = false\nzzz = 1\n",
        )
        .unwrap();
        let r = parse_flag(&f, Path::new("/q"));
        assert!(r.malformed, "pid が空の値で消えたので不正");
        fs::write(
            &f,
            "  format_version = 1  \ntarget = \"/x\"\ntarget=\"/y\"\npid = 9\nmethod = \"atomic\"\n\
             tmp = \"/t\"\nbackup_recorded = true\nbackup_complete = false\nzzz = 1\n",
        )
        .unwrap();
        let r = parse_flag(&f, Path::new("/q"));
        assert!(!r.malformed);
        assert_eq!(r.target, FsPath::new("/x"));
        assert_eq!(r.pid, 9);
        assert_eq!(r.tmp, Some(FsPath::new("/t")));
        assert!(r.backup_recorded && !r.backup_complete);
        let _ = fs::remove_dir_all(d);
    }

    #[cfg(unix)]
    #[test]
    fn unix_のキーは大文字小文字を区別する() {
        let d = tmp_dir("key-unix");
        let k1 = flag_key(&d.join("File.TXT")).unwrap();
        let k2 = flag_key(&d.join("file.txt")).unwrap();
        assert_ne!(k1, k2, "区別する FS が既定なので畳まない");
        assert_eq!(normal_form("/a/b/"), "/a/b");
        assert_eq!(normal_form("/"), "/");
        let _ = fs::remove_dir_all(d);
    }

    #[cfg(unix)]
    #[test]
    fn unix_のフラグのディレクトリは_0700() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp_dir("mode");
        let key = flag_key(&d.join("a.txt")).unwrap();
        let dir = create_flag_dir(&d.join("state"), &key).unwrap();
        for p in [
            dir.as_path(),
            dir.parent().unwrap(),
            dir.parent().unwrap().parent().unwrap(),
        ] {
            assert_eq!(
                fs::metadata(p).unwrap().permissions().mode() & 0o777,
                0o700,
                "{p:?}"
            );
        }
        let _ = fs::remove_dir_all(d);
    }

    #[cfg(windows)]
    #[test]
    fn キーは綴りの違いを畳む() {
        let d = tmp_dir("key");
        let f = d.join("File.TXT");
        fs::write(&f, b"x").unwrap();
        let k1 = flag_key(&f).unwrap();
        let k2 = flag_key(&d.join("file.txt")).unwrap();
        assert_eq!(k1, k2, "大文字小文字を畳む");
        assert_eq!(k1.len(), 16);
        // 親が無ければキーを作らない
        assert!(flag_key(&d.join("nodir").join("a.txt")).is_none());
        // 存在しないファイル名でもキーは作れる
        assert!(flag_key(&d.join("new.txt")).is_some());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn フラグを書いて読む() {
        let d = tmp_dir("rw");
        let target = d.join("a.txt");
        let tmp = d.join(".a.txt.eve-tmp-1-aaaaaaaa");
        let key = flag_key(&target).unwrap();
        let body = FlagBody {
            target: &target,
            method: SaveMethod::InPlace,
            tmp: Some(&tmp),
            backup: Some(&d.join("x.bak")),
            backup_recorded: true,
            backup_complete: false,
        };
        let state = d.join("state");
        let flag = write_flag(&state, &key, &body).unwrap();
        assert!(flag.starts_with(state.join("recovery").join("inflight")));
        let recs = read_flags(&state, &target);
        assert_eq!(recs.len(), 1);
        let r = &recs[0];
        assert!(!r.malformed);
        assert_eq!(r.method, SaveMethod::InPlace);
        assert_eq!(r.pid, std::process::id());
        assert_eq!(r.tmp.as_ref().unwrap().as_path(), tmp);
        assert!(r.backup_recorded && !r.backup_complete);
        assert!(r.started_at.is_some(), "プロセスの起動時刻を書く");
        assert_eq!(r.flag.as_path(), flag);
        // 同じプロセスの 2 つ目は別のフラグ (上書きしない)
        let flag2 = write_flag(&state, &key, &body).unwrap();
        assert_ne!(flag, flag2);
        assert_eq!(read_flags(&state, &target).len(), 2);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn 壊れたフラグは_malformed_で返す() {
        let d = tmp_dir("bad");
        let target = d.join("a.txt");
        let key = flag_key(&target).unwrap();
        let dir = d.join("state").join("recovery").join("inflight").join(&key);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("77-1.toml"),
            "format_version = 99\ntarget = \"x\"\n",
        )
        .unwrap();
        fs::write(dir.join("78-1.toml"), "garbage").unwrap();
        let recs = read_flags(&d.join("state"), &target);
        assert_eq!(recs.len(), 2);
        assert!(recs.iter().all(|r| r.malformed));
        assert_eq!(recs[0].pid, 77, "pid はファイル名から");
        assert_eq!(recs[0].target.as_path(), target, "target は照会したパス");
        let _ = fs::remove_dir_all(d);
    }

    #[cfg(windows)]
    #[test]
    fn フラグのディレクトリは所有者のみの_dacl() {
        let d = tmp_dir("dacl");
        // 実際のキー (パスの XXH3-64) を使う。固定の 16 進の文字列は秘密情報の検出に当たる
        let key = flag_key(&d.join("a.txt")).unwrap();
        let dir = create_flag_dir(&d.join("state"), &key).unwrap();
        let out = std::process::Command::new("icacls")
            .arg(&dir)
            .output()
            .unwrap();
        let acl = String::from_utf8_lossy(&out.stdout);
        assert!(acl.contains("SYSTEM"), "{acl}");
        assert!(
            !acl.contains("BUILTIN\\Users") && !acl.contains("Everyone"),
            "{acl}"
        );
        assert!(!acl.contains("Administrators"), "{acl}");
        let _ = fs::remove_dir_all(d);
    }
}
