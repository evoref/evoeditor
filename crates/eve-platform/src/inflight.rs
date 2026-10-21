//! 保存中フラグ (docs/05 §4.6 / §7.5、02 §4.5)。保存ジョブの開始時 (一時ファイルを作る前) に
//! `state_dir/recovery/inflight/<hash(path)>/<pid>-<seq>.toml` を書き、成功時に消す。開くジョブが `inflight_for` で読む。
//!
//! フラグ自身は `write_atomic` を通さず、ここの下位関数で直接書く (フラグを書くためにフラグを書かない)。TOML・版付き。
//! `hash(path)` は 05 §7.5 の正規形 (親を `canonicalize` + ファイル名、verbatim 接頭辞を外し、大文字小文字を畳み、`\` に揃え、
//! 末尾の区切りを除く) を UTF-16 単位列にして XXH3-64。シンボリックリンクはリンク自身とリンク先の 2 つのキーで照会する。

#![allow(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use eve_core::{FileTime, FsPath, InflightRecord, SaveMethod};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

use crate::path::simplify_verbatim;
use crate::save::OwnerOnly;

/// フラグの版 (キーの追加は版を上げずに行ってよい。意味の変更・必須キーの追加で上げる)。
const FORMAT_VERSION: u32 = 1;

/// プロセス内の保存ジョブごとの通し番号 (同じプロセスの前の保存が残したフラグを上書きしない)。
static SEQ: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

// ---------- キー ----------

/// フラグのキーの入力となる正規形 (親を `canonicalize` し、ファイル名を付け直す。親が無ければ `None`)。
fn normalized(path: &Path) -> Option<String> {
    let name = path.file_name()?;
    let parent = path.parent()?;
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let parent = fs::canonicalize(parent).ok()?;
    let joined = parent.join(name);
    let s = simplify_verbatim(&joined.to_string_lossy());
    // Windows は大文字小文字を区別しない形に畳み、区切りは `\` に揃え、末尾の区切りを除く
    let s = s.replace('/', "\\").to_uppercase();
    Some(s.trim_end_matches('\\').to_owned())
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

/// TOML の基本文字列にする。
fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7F => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// プロセスの作成時刻 (UNIX 紀元からの経過)。取れなければ `None`。
fn process_start() -> Option<FileTime> {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
    // SAFETY: 自プロセスのハンドルと、書込先の有効な `FILETIME` 4 つ。
    let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    if ok == 0 {
        return None;
    }
    let ticks = (u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime);
    // 100 ns 単位、1601-01-01 起点 → UNIX 紀元
    const EPOCH_DIFF_SECS: i64 = 11_644_473_600;
    let secs = (ticks / 10_000_000) as i64 - EPOCH_DIFF_SECS;
    let nanos = ((ticks % 10_000_000) * 100) as u32;
    Some(FileTime { secs, nanos })
}

/// `state_dir/recovery/inflight/<key>/` を所有者のみの DACL で作る (中のファイルは継承で所有者のみ、05 §5.6)。
fn create_flag_dir(state_dir: &Path, key: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(state_dir)?;
    let dir = state_dir.join("recovery").join("inflight").join(key);
    create_dir_all_private(&dir)?;
    Ok(dir)
}

/// 保護付きの DACL (所有者と SYSTEM のみ、継承を遮断・中へ継承) でディレクトリを作る。既にあれば何もしない。
pub(crate) fn create_dir_all_private(dir: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
    if dir.is_dir() {
        return Ok(());
    }
    if let Some(parent) = dir.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        create_dir_all_private(parent)?;
    }
    let sa = OwnerOnly::new_dir()?;
    let w: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: NUL 終端のパス、有効な `SECURITY_ATTRIBUTES`。
    let ok = unsafe { CreateDirectoryW(w.as_ptr(), sa.as_ptr()) };
    if ok == 0 {
        let e = io::Error::last_os_error();
        // 並行して作られた
        if dir.is_dir() {
            return Ok(());
        }
        return Err(e);
    }
    Ok(())
}

/// フラグを書く (一時名に書いて fsync し、名前を変える。照合も保存中フラグも使わない下位関数)。書いたフラグのパスを返す。
pub(crate) fn write_flag(state_dir: &Path, key: &str, body: &FlagBody<'_>) -> io::Result<PathBuf> {
    let dir = create_flag_dir(state_dir, key)?;
    let name = format!("{}-{}.toml", std::process::id(), next_seq());
    let path = dir.join(&name);
    let mut text = String::new();
    let mut line = |k: &str, v: String| {
        text.push_str(k);
        text.push_str(" = ");
        text.push_str(&v);
        text.push('\n');
    };
    line("format_version", FORMAT_VERSION.to_string());
    line("target", toml_str(&body.target.to_string_lossy()));
    line("pid", std::process::id().to_string());
    if let Some(t) = process_start() {
        line("started_at_secs", t.secs.to_string());
        line("started_at_nanos", t.nanos.to_string());
    }
    line("method", toml_str(method_name(body.method)));
    if let Some(t) = body.tmp {
        line("tmp", toml_str(&t.to_string_lossy()));
    }
    if let Some(b) = body.backup {
        line("backup", toml_str(&b.to_string_lossy()));
    }
    line("backup_recorded", body.backup_recorded.to_string());
    line("backup_complete", body.backup_complete.to_string());
    write_replace(&path, text.as_bytes())?;
    Ok(path)
}

/// 同じディレクトリの一時名に書いて fsync し、名前を変える。
fn write_replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

// ---------- 読み手 ----------

/// TOML の基本文字列 (引用符の中) をほどく。`None` は不正。
fn parse_string(v: &str) -> Option<String> {
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next()? {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let hex: String = it.by_ref().take(4).collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

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
        let Some((k, v)) = l.split_once(" = ") else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "format_version" => version = v.parse::<u32>().ok(),
            "target" => target = parse_string(v),
            "pid" => pid = v.parse::<u32>().ok(),
            "method" => method = parse_string(v),
            "tmp" => tmp = parse_string(v),
            "backup" => backup = parse_string(v),
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
        let traces = [rec.tmp.as_ref(), rec.backup.as_ref()]
            .into_iter()
            .flatten()
            .any(|t| t.as_path().exists());
        if !traces {
            let _ = fs::remove_file(&p);
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

    #[test]
    fn 文字列の往復() {
        for s in ["plain", "a\"b\\c", "日本語 \t\n", "\u{1}x"] {
            assert_eq!(parse_string(&toml_str(s)).as_deref(), Some(s), "{s:?}");
        }
        assert_eq!(parse_string("no quotes"), None);
    }

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
