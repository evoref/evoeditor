//! `FileSystem::read_dir` の本体 (docs/02 §4.5、05 §2.1 の `read_dir()` の行。v0.5 の M31)。`WinFs` と `UnixFs` が共有する。
//!
//! std の `read_dir` で列挙する (Windows は `FindFirstFileExW` / `FindNextFileW`、Unix は `readdir`)。種類・大きさ・属性は
//! 列挙が返すものから取り、項目ごとの `stat` をしない。
//! - Windows: `DirEntry::file_type` / `DirEntry::metadata` は列挙のデータ (属性と再解析のタグ) から作られ、追加の呼出しが無い。
//!   std の `is_symlink` は「再解析点で、タグが名前の代理」なので、シンボリックリンクとジャンクションが `Symlink` になり、
//!   それ以外の再解析点 (`OneDrive` のプレースホルダ・重複除去のファイルなど) は属性のディレクトリの別で `Dir` / `File` になる。
//! - Unix: `DirEntry::file_type` は `d_type` を使い、`DT_UNKNOWN` のときだけ `lstat` で補う。`len` は `None`、`offline` は `false`。

use std::fs;
use std::io;

use eve_core::{DirEntry, DirListing, EntryKind, FsError, FsErrorDetail, FsErrorKind, FsPath};

use crate::sys::map_io_error;

/// `dir` の直下を `max_entries` 個まで列挙する。`max_entries` 個を返した後にまだ項目があれば `truncated`。
/// 項目 1 つの読み取りの失敗はその項目を飛ばし、列挙そのものの途中の失敗はそこまでの項目で `incomplete`。
pub(crate) fn read_dir(dir: &FsPath, max_entries: usize) -> Result<DirListing, FsError> {
    let iter = open(dir)?;
    let mut listing = DirListing::default();
    for item in iter {
        // std の反復子が `Err` を返すのは列挙そのものの失敗 (Unix の `readdir` の errno、Windows の `FindNextFileW` の
        // `ERROR_NO_MORE_FILES` 以外)。欠けた一覧を完全に見せないよう印を立ててやめる (Windows の反復子は失敗の後も
        // 呼べば同じ失敗を返し得るので、続けない)
        let e = match item {
            Ok(e) => e,
            Err(_) => {
                listing.incomplete = true;
                break;
            }
        };
        // 項目 1 つの失敗 (Unix の `DT_UNKNOWN` の `lstat` の失敗。項目が消えたなど) は飛ばす
        let Some(entry) = entry_of(&e) else {
            continue;
        };
        if listing.entries.len() == max_entries {
            listing.truncated = true;
            break;
        }
        listing.entries.push(entry);
    }
    Ok(listing)
}

/// 列挙を始める。`EINTR` は再試行する。失敗の写像は 05 §2.1 (ディレクトリでなければ `InvalidPath`、detail なし)。
fn open(dir: &FsPath) -> Result<fs::ReadDir, FsError> {
    loop {
        match fs::read_dir(dir.as_path()) {
            Ok(rd) => return Ok(rd),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(open_error(e, dir)),
        }
    }
}

fn open_error(e: io::Error, dir: &FsPath) -> FsError {
    // 在るがディレクトリでない (ファイル・デバイス・ファイルを指すリンク)。Windows はファイルに列挙を掛けると
    // `ERROR_DIRECTORY` や `ERROR_PATH_NOT_FOUND` を返し得るので、失敗のときだけ `metadata` で確かめて揃える
    if fs::metadata(dir.as_path()).is_ok_and(|m| !m.is_dir()) {
        return FsError {
            kind: FsErrorKind::InvalidPath,
            detail: None,
            source: Some(e),
        };
    }
    let mut err = map_io_error(e, dir);
    if err.kind == FsErrorKind::NotFound {
        // `NotFound` の detail は、ダングリングのリンクだけに付ける。Windows の `ERROR_PATH_NOT_FOUND` は親が在っても
        // `ParentMissing` に写るので落とす (02 §4.5 の read_dir の注)
        err.detail = fs::symlink_metadata(dir.as_path())
            .is_ok_and(|m| m.file_type().is_symlink())
            .then(|| {
                fs::read_link(dir.as_path()).map(|target| FsErrorDetail::DanglingLink {
                    target: FsPath::new(target),
                })
            })
            .and_then(Result::ok);
    }
    err
}

fn entry_of(e: &fs::DirEntry) -> Option<DirEntry> {
    let ft = e.file_type().ok()?;
    let kind = if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let (len, offline) = extra(e, kind);
    Some(DirEntry {
        name: e.file_name(),
        kind,
        len,
        offline,
    })
}

/// 呼び戻しが起きる属性 (`FILE_ATTRIBUTE_OFFLINE` / `RECALL_ON_OPEN` / `RECALL_ON_DATA_ACCESS`)。
#[cfg(windows)]
const RECALL_ATTRS: u32 = windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_OFFLINE
    | windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_RECALL_ON_OPEN
    | windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;

/// 大きさとオフラインの属性 (Windows)。`DirEntry::metadata` は列挙のデータから作られ、追加の呼出しをしない。
/// 大きさはファイルだけに返す (ディレクトリ・リンクの大きさは中身・リンク先の大きさではない)。オフラインは種類を問わない。
#[cfg(windows)]
fn extra(e: &fs::DirEntry, kind: EntryKind) -> (Option<u64>, bool) {
    use std::os::windows::fs::MetadataExt;
    match e.metadata() {
        Ok(m) => (
            (kind == EntryKind::File).then_some(m.len()),
            m.file_attributes() & RECALL_ATTRS != 0,
        ),
        Err(_) => (None, false),
    }
}

/// Unix は大きさを返さない (`stat` をしないため) し、呼び戻しの属性も無い。
#[cfg(not(windows))]
fn extra(_e: &fs::DirEntry, _kind: EntryKind) -> (Option<u64>, bool) {
    (None, false)
}
