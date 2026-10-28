//! スワップの復旧の探索 (docs/03 §10.3、05 §5.3 / §7.5)。`swap/*.swp` を列挙して manifest を要約し、本文を読み、他のプロセスの残骸を消す。
//! 書く側は `swap.rs`。pid の生存確認は `OpenProcess` + 起動時刻の一致 (05 §5.3)。

#![allow(unsafe_code)]

use std::fs;
use std::io;
use std::path::Path;

use eve_core::{FileTime, FsError, FsErrorKind, FsPath, SwapBody, SwapEntry, SwapOwner};

use crate::fs::map_io_error;
use crate::inflight::{parse_string, process_start};

/// manifest から読んだ、復旧に要る項目。
#[derive(Default)]
struct Parsed {
    version: Option<u32>,
    pid: Option<u32>,
    started_at: Option<FileTime>,
    file: Option<String>,
    path: Option<String>,
    untitled_name: Option<String>,
    encoding: Option<String>,
    bom: bool,
    default_eol: Option<String>,
    revision: Option<u64>,
    len: Option<u64>,
    written: Option<bool>,
    sensitive: bool,
}

/// manifest の版 (これより新しい版は読まない)。
const MANIFEST_VERSION: u32 = 1;

/// manifest を読む。行頭の `key = value` だけを見て、最初の `[[document]]` までをヘッダ、以後を文書とする (スワップは 1 つだけ)。
/// 未知のキーは無視する (キーの追加は版を上げない、02 §11.2.1)。
fn parse_manifest(text: &str) -> Parsed {
    let mut p = Parsed::default();
    let (mut secs, mut nanos) = (None::<i64>, None::<u32>);
    let mut in_doc = false;
    for l in text.lines() {
        let l = l.trim();
        if l == "[[document]]" {
            if in_doc {
                break; // 2 つ目以降は読まない
            }
            in_doc = true;
            continue;
        }
        let Some((k, v)) = l.split_once(" = ") else {
            continue;
        };
        let v = v.trim();
        match (in_doc, k.trim()) {
            (false, "format_version") => p.version = v.parse().ok(),
            (false, "pid") => p.pid = v.parse().ok(),
            (false, "started_at_secs") => secs = v.parse().ok(),
            (false, "started_at_nanos") => nanos = v.parse().ok(),
            (true, "file") => p.file = parse_string(v),
            (true, "path") => p.path = parse_string(v),
            (true, "untitled_name") => p.untitled_name = parse_string(v),
            (true, "encoding") => p.encoding = parse_string(v),
            (true, "bom") => p.bom = v == "true",
            (true, "default_eol") => p.default_eol = parse_string(v),
            (true, "revision") => p.revision = v.parse().ok(),
            (true, "len") => p.len = v.parse().ok(),
            (true, "written") => p.written = v.parse().ok(),
            (true, "sensitive") => p.sensitive = v == "true",
            _ => {}
        }
    }
    p.started_at = secs.map(|secs| FileTime {
        secs,
        nanos: nanos.unwrap_or(0),
    });
    p
}

/// 起動時刻 (UNIX 秒) が近い。pid の再利用は起動時刻が違う。
fn same_start(a: FileTime, b: FileTime) -> bool {
    (a.secs - b.secs).abs() <= 2
}

/// pid のプロセスが、記録された起動時刻のまま動いているか (05 §5.3)。起動時刻の記録が無ければ pid だけで判定する。
fn process_alive(pid: u32, started_at: Option<FileTime>) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, GetLastError};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    const ERROR_ACCESS_DENIED: u32 = 5;
    // SAFETY: 有効な引数。返ったハンドルは下で閉じる。
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        // アクセス拒否は「いるが開けない」。それ以外 (無効な pid) はいない
        // SAFETY: 直前の呼出しのエラーを読むだけ。
        return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
    }
    let mut code = 0u32;
    // SAFETY: 有効なハンドルと書込先。
    let running = unsafe { GetExitCodeProcess(h, &mut code) } != 0 && code == STILL_ACTIVE;
    let mut matches = true;
    if running && let Some(want) = started_at {
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
        // SAFETY: 有効なハンドルと書込先の `FILETIME` 4 つ。
        if unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) } != 0 {
            let ticks = (u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime);
            const EPOCH_DIFF_SECS: i64 = 11_644_473_600;
            let secs = (ticks / 10_000_000) as i64 - EPOCH_DIFF_SECS;
            matches = same_start(FileTime { secs, nanos: 0 }, want);
        }
    }
    // SAFETY: `OpenProcess` が返したハンドル。
    unsafe { CloseHandle(h) };
    running && matches
}

/// 自分のプロセスが書いたものか (pid と起動時刻が一致する)。
fn is_mine(pid: u32, started_at: Option<FileTime>) -> bool {
    if pid != std::process::id() {
        return false;
    }
    match (started_at, process_start()) {
        (Some(a), Some(b)) => same_start(a, b),
        _ => true,
    }
}

/// `swap/*.swp` を列挙して要約する。自分のものは含めない。名前の形に合わないものは無視する。
pub(crate) fn list_swaps(state_dir: &Path) -> Vec<SwapEntry> {
    let Ok(rd) = fs::read_dir(state_dir.join("swap")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let dir = e.path();
        if !dir.is_dir() || dir.extension().is_none_or(|x| x != "swp") {
            continue;
        }
        let name_pid = dir
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.rsplit('-').next())
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(0);
        let parsed = fs::read_to_string(dir.join("manifest.toml"))
            .map(|t| parse_manifest(&t))
            .ok();
        let readable = parsed.as_ref().is_some_and(|p| {
            p.version == Some(MANIFEST_VERSION)
                && p.written == Some(true)
                && p.file.is_some()
                && p.encoding.is_some()
        });
        let p = parsed.unwrap_or_default();
        let pid = p.pid.unwrap_or(name_pid);
        if is_mine(pid, p.started_at) {
            continue;
        }
        let owner = if process_alive(pid, p.started_at) {
            SwapOwner::Alive
        } else {
            SwapOwner::Stale
        };
        out.push(SwapEntry {
            dir: FsPath::new(dir),
            path: p.path.as_deref().map(FsPath::from),
            untitled_name: p.untitled_name,
            revision: p.revision.unwrap_or(0),
            len: p.len.unwrap_or(0),
            pid,
            owner,
            sensitive: p.sensitive,
            readable,
        });
    }
    out.sort_by(|a, b| a.dir.as_path().cmp(b.dir.as_path()));
    out
}

/// スワップの本文と形式を読む。
pub(crate) fn read_swap(entry: &SwapEntry) -> Result<SwapBody, FsError> {
    let dir = entry.dir.as_path();
    let err = |e: io::Error| map_io_error(e, &entry.dir);
    let text = fs::read_to_string(dir.join("manifest.toml")).map_err(err)?;
    let p = parse_manifest(&text);
    let bad = || FsError::new(FsErrorKind::Other);
    let file = p.file.ok_or_else(bad)?;
    // manifest の `file` はスワップのディレクトリの直下の名前だけ (パスの区切りを含むものは使わない)
    if file.contains(['/', '\\']) || file == ".." {
        return Err(bad());
    }
    let body = fs::read(dir.join(&file)).map_err(err)?;
    let text = String::from_utf8(body).map_err(|_| bad())?;
    let encoding = p
        .encoding
        .as_deref()
        .and_then(eve_core::Encoding::from_label)
        .ok_or_else(bad)?;
    let default_eol = match p.default_eol.as_deref() {
        Some("crlf") => eve_text::LineEnding::Crlf,
        Some("cr") => eve_text::LineEnding::Cr,
        _ => eve_text::LineEnding::Lf,
    };
    Ok(SwapBody {
        text,
        encoding,
        has_bom: p.bom,
        default_eol,
    })
}

/// 他のプロセスのスワップを消す。`state_dir/swap/` の直下の `.swp` ディレクトリ以外は消さない。無ければ成功。
pub(crate) fn remove_swap(state_dir: &Path, entry: &SwapEntry) -> Result<(), FsError> {
    let dir = entry.dir.as_path();
    let base = state_dir.join("swap");
    if dir.parent() != Some(base.as_path()) || dir.extension().is_none_or(|x| x != "swp") {
        return Err(FsError::new(FsErrorKind::InvalidPath));
    }
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(map_io_error(e, &entry.dir)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_の要約() {
        let t = "format_version = 1\ncreated_by = \"x\"\nreason = \"swap\"\ncomplete = true\npid = 42\nstarted_at_secs = 100\nstarted_at_nanos = 5\n\n[[document]]\nfile = \"1-a.r3.txt\"\npath = \"C:\\\\w\\\\a.txt\"\nencoding = \"Shift_JIS\"\nbom = false\ndefault_eol = \"crlf\"\nrevision = 3\nlen = 10\nwritten = true\nsensitive = true\nunknown_key = 1\n";
        let p = parse_manifest(t);
        assert_eq!(p.version, Some(1));
        assert_eq!(p.pid, Some(42));
        assert_eq!(
            p.started_at,
            Some(FileTime {
                secs: 100,
                nanos: 5
            })
        );
        assert_eq!(p.file.as_deref(), Some("1-a.r3.txt"));
        assert_eq!(p.path.as_deref(), Some("C:\\w\\a.txt"));
        assert_eq!(p.encoding.as_deref(), Some("Shift_JIS"));
        assert_eq!(
            (p.revision, p.len, p.written),
            (Some(3), Some(10), Some(true))
        );
        assert!(p.sensitive);
    }

    #[test]
    fn 起動時刻の近さ() {
        let a = FileTime {
            secs: 100,
            nanos: 0,
        };
        assert!(same_start(
            a,
            FileTime {
                secs: 102,
                nanos: 0
            }
        ));
        assert!(!same_start(
            a,
            FileTime {
                secs: 103,
                nanos: 0
            }
        ));
    }

    #[test]
    fn 自分の_pid_は動いていて_存在しない_pid_は動いていない() {
        assert!(process_alive(std::process::id(), process_start()));
        // 起動時刻が違えば pid が同じでも別のプロセス
        let other = FileTime { secs: 1, nanos: 0 };
        assert!(!process_alive(std::process::id(), Some(other)));
        // 0xFFFF_FFF0 は実在しない pid
        assert!(!process_alive(0xFFFF_FFF0, None));
    }
}
