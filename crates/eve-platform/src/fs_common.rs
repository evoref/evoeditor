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

/// `open_read` の本体。開いた後のハンドルで通常のファイルと確かめ、ディレクトリ・FIFO・デバイスは読まない (`InvalidPath` +
/// `NotAFile`、05 §2.1 の `open_read()` の行・§4.5)。Unix は `O_NONBLOCK` で開き (FIFO を開くときに書き手を待って止まらない)、
/// `fstat` で通常のファイルと確かめてから非ブロックを外す。非ブロックの `open` が `EAGAIN` (書込のリースが掛かった通常の
/// ファイル) なら、非ブロックなしで開き直す (リースが破られるまで待つ)。Windows は `GetFileType` が `FILE_TYPE_CHAR`
/// (`NUL`・コンソール) か `FILE_TYPE_PIPE` なら拒む。どちらも、属性 (`fstat` / `GetFileInformationByHandle`) を読めなければ
/// 確かめずに進まず `Err` にする。
pub(crate) fn open_read(path: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
    let not_a_file = || FsError::with_detail(FsErrorKind::InvalidPath, FsErrorDetail::NotAFile);
    let f = match open_file(path) {
        Ok(f) => f,
        // Windows はディレクトリを開くと `ERROR_ACCESS_DENIED`、Unix はソケットを開くと `ENXIO` になる。在るが通常のファイルで
        // ないなら、権限などの問題ではなく `InvalidPath` (05 §2.1 の写像の表、§4.5)
        Err(_) if fs::metadata(path.as_path()).is_ok_and(|m| !m.is_file()) => {
            return Err(not_a_file());
        }
        Err(e) => return Err(map_io_error(e, path)),
    };
    // Windows の `NUL`・コンソール・パイプ。種類を問い合わせられない (`FILE_TYPE_UNKNOWN`) ときは下の属性で決める
    #[cfg(windows)]
    if crate::sys::is_char_or_pipe(&f) {
        return Err(not_a_file());
    }
    let meta = f.metadata().map_err(|e| map_io_error(e, path))?;
    if !meta.is_file() {
        return Err(not_a_file());
    }
    // ローカルの FS では通常のファイルの読込に `O_NONBLOCK` は効かないので、外せなくても読める (失敗は無視する)
    #[cfg(unix)]
    let _ = crate::sys::clear_nonblocking(&f);
    Ok(Box::new(FileChunks {
        file: f,
        buf: vec![0u8; first_buf_len(meta.len())],
        path: path.clone(),
    }))
}

/// 最初の読込のバッファの大きさ: `min(READ_CHUNK_BYTES, len + 1)`。小さなファイルを開くたびに 1 MiB を確保して 0 で埋めない
/// (横断検索は 1 ファイルずつ開いて読む、03 §12.7.5)。`len + 1` にするのは、変わっていないファイルを 1 回の読込で読み終え、
/// 次の読込で終わりと分かるようにするため (チャンクの区切りは 1 MiB のバッファのときと同じ: 1 MiB 以下は 1 チャンク、超えれば
/// 1 MiB ずつ)。開いた後にファイルが伸びて 1 回の読込でバッファが埋まったら、その場で `READ_CHUNK_BYTES` に広げて読み足して
/// から返す (`FileChunks::next_chunk`)。伸びたファイルの最初のチャンクも 1 MiB のバッファのときと同じになり、読込の文字コードの
/// 判定 (03 §8.2) が小さな最初のチャンクだけで決まらない。
fn first_buf_len(len: u64) -> usize {
    usize::try_from(len)
        .map_or(READ_CHUNK_BYTES, |n| n.saturating_add(1))
        .min(READ_CHUNK_BYTES)
}

/// 読むために開く。Unix は `O_NONBLOCK | O_NOCTTY` (FIFO の書き手を待たない。端末を制御端末にしない)。`EAGAIN` /
/// `EWOULDBLOCK` は書込のリースが掛かった通常のファイル (読み取りで開く FIFO は `EAGAIN` にならない) なので、`O_NONBLOCK`
/// なしで開き直す (05 §2.1 の `open_read()` の行)。
fn open_file(path: &FsPath) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let open = |flags| {
            fs::OpenOptions::new()
                .read(true)
                .custom_flags(flags)
                .open(path.as_path())
        };
        match open(libc::O_NONBLOCK | libc::O_NOCTTY) {
            Err(e) if matches!(e.raw_os_error(), Some(c) if c == libc::EAGAIN || c == libc::EWOULDBLOCK) => {
                open(libc::O_NOCTTY)
            }
            r => r,
        }
    }
    #[cfg(not(unix))]
    {
        File::open(path.as_path())
    }
}

/// ファイルをチャンクで読む。
struct FileChunks {
    file: File,
    /// 読込のバッファ (最初は `first_buf_len`、伸びたファイルでは `READ_CHUNK_BYTES`)。
    buf: Vec<u8>,
    path: FsPath,
}

impl FileChunks {
    /// `buf[from..]` に読む (`EINTR` は再試行する)。読んだバイト数。
    fn read_into(&mut self, from: usize) -> Result<usize, ChunkError> {
        loop {
            match self.file.read(&mut self.buf[from..]) {
                Ok(n) => return Ok(n),
                // EINTR は本層の中で再試行して外に出さない
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(ChunkError::Io(map_io_error(e, &self.path))),
            }
        }
    }
}

impl ReadChunks for FileChunks {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        let mut n = self.read_into(0)?;
        if n == 0 {
            return Ok(None);
        }
        // 1 MiB より小さいバッファが埋まった (開いた後にファイルが伸びた): その場で 1 MiB に広げ、読み足してから返す (1 回だけ)
        if n == self.buf.len() && n < READ_CHUNK_BYTES {
            self.buf.resize(READ_CHUNK_BYTES, 0);
            n += self.read_into(n)?;
        }
        Ok(Some(&self.buf[..n]))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    /// 一時ディレクトリ (終わりに消す)。
    struct Tmp(std::path::PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Tmp {
            static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "eve-platform-chunks-{tag}-{}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&p).expect("一時ディレクトリ");
            Tmp(p)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 長さ `len` のファイルを読んだチャンクの長さの列。
    fn chunk_lens(len: usize) -> Vec<usize> {
        let t = Tmp::new(&len.to_string());
        let p = t.0.join("f.bin");
        fs::write(&p, vec![b'x'; len]).expect("書く");
        let mut r = open_read(&FsPath::new(p)).expect("開く");
        let mut out = Vec::new();
        while let Some(c) = r.next_chunk().expect("読む") {
            out.push(c.len());
        }
        out
    }

    #[test]
    fn 最初のバッファは長さに_1_を足して_1_mib_まで() {
        let max = u64::try_from(READ_CHUNK_BYTES).expect("1 MiB");
        assert_eq!(first_buf_len(0), 1);
        assert_eq!(first_buf_len(10), 11);
        assert_eq!(first_buf_len(max - 1), READ_CHUNK_BYTES);
        assert_eq!(first_buf_len(max), READ_CHUNK_BYTES);
        assert_eq!(first_buf_len(u64::MAX), READ_CHUNK_BYTES);
    }

    #[test]
    fn 変わっていないファイルのチャンクの区切りは_1_mib_のバッファと同じ() {
        assert_eq!(chunk_lens(0), Vec::<usize>::new(), "空のファイル");
        assert_eq!(chunk_lens(5), [5], "1 MiB 以下は 1 チャンク");
        assert_eq!(
            chunk_lens(READ_CHUNK_BYTES),
            [READ_CHUNK_BYTES],
            "1 MiB ちょうど"
        );
        assert_eq!(
            chunk_lens(READ_CHUNK_BYTES + 1),
            [READ_CHUNK_BYTES, 1],
            "1 MiB + 1 は 1 MiB と 1"
        );
    }

    #[test]
    fn 開いた後に伸びたファイルは広げたバッファで最後まで読む() {
        let t = Tmp::new("grow");
        let p = t.0.join("f.bin");
        fs::write(&p, b"0123456789").expect("書く");
        let mut r = open_read(&FsPath::new(p.clone())).expect("開く");
        // 開いた後に 2 MiB 足す (最初のバッファは 11 バイト)
        let tail = vec![b'y'; 2 * READ_CHUNK_BYTES];
        fs::OpenOptions::new()
            .append(true)
            .open(&p)
            .expect("足す")
            .write_all(&tail)
            .expect("足す");
        let mut lens = Vec::new();
        let mut all = Vec::new();
        while let Some(c) = r.next_chunk().expect("読む") {
            lens.push(c.len());
            all.extend_from_slice(c);
        }
        assert_eq!(all.len(), 10 + tail.len(), "伸びた分も読む");
        assert_eq!(all.get(..10), Some(b"0123456789".as_slice()));
        // 最初のバッファ (11 バイト) が埋まったら、その場で 1 MiB に広げて読み足す: 最初のチャンクに伸びた分が入る (1 MiB の
        // バッファのときと同じ区切り。読込の文字コードの判定が 11 バイトだけで決まらない)
        assert_eq!(
            lens.first(),
            Some(&READ_CHUNK_BYTES),
            "最初のチャンクに伸びた分も入る: {lens:?}"
        );
        assert_eq!(all.get(10..11), Some(b"y".as_slice()));
        assert!(lens.iter().all(|n| *n <= READ_CHUNK_BYTES));
    }
}
