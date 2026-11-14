//! `FileSystem` の読み取り側のうち OS に依らない部分 (docs/05 §2.1 / §3.3 / §4.5)。`WinFs` (`fs`) と `UnixFs` (`unix_fs`) が共有する。
//! エラーの写像は OS ごと (`sys::map_io_error`)。
//!
//! 読込側の規則 (05 §4.5): チャンクは 1 MiB、`EINTR` (`Interrupted`) は内側で再試行する、ハンドルは読込の完了後すぐ閉じる
//! (呼び側が `ReadChunks` を捨てる)、共有モードは std の既定 (読み・書き・削除を許す)、mmap は使わない。

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use eve_core::{
    ChunkError, FileKind, FsError, FsErrorDetail, FsErrorKind, FsPath, LinkInfo, LinkKind,
    ReadChunks,
};

use crate::sys::map_io_error;

/// 読込のチャンクの大きさ (05 §4.5。64 KiB〜1 MiB でベンチで決める。既定は 1 MiB)。
pub const READ_CHUNK_BYTES: usize = 1024 * 1024;

pub(crate) fn file_kind(meta: &fs::Metadata) -> FileKind {
    if meta.is_file() {
        FileKind::File
    } else if meta.is_dir() {
        FileKind::Dir
    } else {
        FileKind::Other
    }
}

/// `p` がシンボリックリンクならリンク自体の情報 (05 §3.3。辿った先が在るので `dangling = false`)。
pub(crate) fn link_info(p: &Path) -> Option<LinkInfo> {
    fs::symlink_metadata(p)
        .ok()
        .filter(|sm| sm.file_type().is_symlink())
        .map(|_| LinkInfo {
            kind: LinkKind::Symlink,
            target: fs::read_link(p).ok().map(FsPath::new),
            dangling: false,
        })
}

/// `metadata` の失敗の写像。リンクは在るがリンク先が無い (ダングリング) なら `NotFound` + `DanglingLink`、それ以外は OS の写像。
/// ディレクトリの項目の名前 (UTF-8 で表せるものだけ。`FileSystem::list_dir_names`)。
pub(crate) fn list_dir_names(dir: &FsPath) -> Result<Vec<String>, FsError> {
    let entries = fs::read_dir(dir.as_path()).map_err(|e| metadata_error(e, dir))?;
    Ok(entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect())
}

pub(crate) fn metadata_error(e: io::Error, path: &FsPath) -> FsError {
    let p = path.as_path();
    if e.kind() == io::ErrorKind::NotFound
        && let Ok(sm) = fs::symlink_metadata(p)
        && sm.file_type().is_symlink()
    {
        let target = fs::read_link(p).ok().map(FsPath::new);
        return FsError {
            kind: FsErrorKind::NotFound,
            detail: target.map(|target| FsErrorDetail::DanglingLink { target }),
            source: Some(e),
        };
    }
    map_io_error(e, path)
}

/// `open_read` の本体。ディレクトリ・デバイスは開かない (`InvalidPath` + `NotAFile`、05 §4.5。Windows の `File::open` は
/// ディレクトリで成功しないので、ここに来るのは Unix)。
pub(crate) fn open_read(path: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
    let f = match File::open(path.as_path()) {
        Ok(f) => f,
        // Windows はディレクトリを開くと `ERROR_ACCESS_DENIED` になる。ディレクトリなら権限の問題ではなく
        // `InvalidPath` (05 §2.1 の写像の表、§4.5)
        Err(_) if std::fs::metadata(path.as_path()).is_ok_and(|m| m.is_dir()) => {
            return Err(FsError::with_detail(
                FsErrorKind::InvalidPath,
                FsErrorDetail::NotAFile,
            ));
        }
        Err(e) => return Err(map_io_error(e, path)),
    };
    if let Ok(m) = f.metadata()
        && !m.is_file()
    {
        return Err(FsError::with_detail(
            FsErrorKind::InvalidPath,
            FsErrorDetail::NotAFile,
        ));
    }
    Ok(Box::new(FileChunks {
        file: f,
        buf: vec![0u8; READ_CHUNK_BYTES],
        path: path.clone(),
    }))
}

/// ファイルをチャンクで読む。
struct FileChunks {
    file: File,
    buf: Vec<u8>,
    path: FsPath,
}

impl ReadChunks for FileChunks {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        loop {
            match self.file.read(&mut self.buf) {
                Ok(0) => return Ok(None),
                Ok(n) => return Ok(Some(&self.buf[..n])),
                // EINTR は本層の中で再試行して外に出さない
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(ChunkError::Io(map_io_error(e, &self.path))),
            }
        }
    }
}
