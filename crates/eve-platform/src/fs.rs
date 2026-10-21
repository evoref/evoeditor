//! `FileSystem` Port の実装 (docs/05 §2.1 / §4.5、Windows)。読み取り側 (`metadata` / `canonicalize` / `open_read`)。
//! 保存 (`write_atomic`) は `save` (05 §4.1 / §4.2)。
//!
//! 読込側の規則 (05 §4.5): チャンクは 1 MiB、`EINTR` (`Interrupted`) は内側で再試行する、ハンドルは読込の完了後すぐ閉じる
//! (呼び側が `ReadChunks` を捨てる)、共有モードは std の既定 (読み・書き・削除を許す)、mmap は使わない。

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::UNIX_EPOCH;

use eve_core::{
    ChunkError, FileKind, FileMeta, FileSystem, FileTime, FsError, FsErrorDetail, FsErrorKind,
    FsKind, FsPath, LinkInfo, LinkKind, ReadChunks, SaveOptions, SaveReport,
};

use crate::path::simplify_verbatim;

/// 読込のチャンクの大きさ (05 §4.5。64 KiB〜1 MiB でベンチで決める。既定は 1 MiB)。
pub const READ_CHUNK_BYTES: usize = 1024 * 1024;

/// Windows のファイルシステム。`state_dir` (05 §7.5) を持つと、保存で保存中フラグを書き、開くときに `inflight_for` で読む。
/// 持たない (`default()`) と、フラグは書かず空を返す (テスト・状態ディレクトリを使えないとき)。
#[derive(Clone, Debug, Default)]
pub struct WinFs {
    state_dir: Option<std::path::PathBuf>,
}

impl WinFs {
    pub fn new(state_dir: impl Into<std::path::PathBuf>) -> WinFs {
        WinFs {
            state_dir: Some(state_dir.into()),
        }
    }

    pub(crate) fn state_dir(&self) -> Option<&Path> {
        self.state_dir.as_deref()
    }
}

/// OS のエラーを `FsError` に写す (05 §2.1 の写像。Windows の `ERROR_*` コード)。
pub fn map_io_error(e: io::Error, path: &FsPath) -> FsError {
    let (kind, detail) = match e.raw_os_error() {
        // ERROR_FILE_NOT_FOUND
        Some(2) => (FsErrorKind::NotFound, None),
        // ERROR_PATH_NOT_FOUND: 親フォルダが無い (フォルダは作らない)
        Some(3) => (
            FsErrorKind::NotFound,
            path.parent()
                .map(|dir| FsErrorDetail::ParentMissing { dir }),
        ),
        // ERROR_ACCESS_DENIED
        Some(5) => (FsErrorKind::PermissionDenied, None),
        // ERROR_WRITE_PROTECT
        Some(19) => (FsErrorKind::ReadOnlyFs, None),
        // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
        Some(32 | 33) => (FsErrorKind::Busy, None),
        // ERROR_BAD_NETPATH / ERROR_NETNAME_DELETED / ERROR_BAD_NET_NAME / ERROR_UNEXP_NET_ERR
        Some(53 | 64 | 67 | 59 | 1231) => (FsErrorKind::Other, Some(FsErrorDetail::Network)),
        // ERROR_HANDLE_DISK_FULL / ERROR_DISK_FULL
        Some(39 | 112) => (FsErrorKind::DiskFull, None),
        // ERROR_INVALID_NAME / ERROR_BAD_PATHNAME / ERROR_FILENAME_EXCED_RANGE / ERROR_DIRECTORY
        Some(123 | 161 | 206 | 267) => (FsErrorKind::InvalidPath, None),
        _ => match e.kind() {
            io::ErrorKind::NotFound => (FsErrorKind::NotFound, None),
            io::ErrorKind::PermissionDenied => (FsErrorKind::PermissionDenied, None),
            io::ErrorKind::InvalidInput | io::ErrorKind::InvalidFilename => {
                (FsErrorKind::InvalidPath, None)
            }
            _ => (FsErrorKind::Other, None),
        },
    };
    FsError {
        kind,
        detail,
        source: Some(e),
    }
}

fn file_time(t: io::Result<std::time::SystemTime>) -> FileTime {
    match t {
        Ok(t) => match t.duration_since(UNIX_EPOCH) {
            Ok(d) => FileTime {
                secs: d.as_secs() as i64,
                nanos: d.subsec_nanos(),
            },
            // 紀元より前 (まれ): 負の秒で表す
            Err(e) => {
                let d = e.duration();
                FileTime {
                    secs: -(d.as_secs() as i64) - i64::from(d.subsec_nanos() > 0),
                    nanos: if d.subsec_nanos() > 0 {
                        1_000_000_000 - d.subsec_nanos()
                    } else {
                        0
                    },
                }
            }
        },
        Err(_) => FileTime::default(),
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) mod winapi {
    use std::fs::File;
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetDriveTypeW, GetFileInformationByHandle,
    };

    /// `GetDriveTypeW` の戻り値 `DRIVE_REMOTE`。
    const DRIVE_REMOTE: u32 = 4;

    /// ボリューム番号・ファイルインデックス・ハードリンク数。
    pub struct HandleInfo {
        pub volume: u64,
        pub index: u128,
        pub nlink: u32,
    }

    pub fn file_info(f: &File) -> io::Result<HandleInfo> {
        // SAFETY: 全ビット 0 の `BY_HANDLE_FILE_INFORMATION` は有効な値 (すべて整数)。
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `f` は開いているファイルのハンドル、`info` は書込先の有効な構造体。
        if unsafe { GetFileInformationByHandle(f.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(HandleInfo {
            volume: u64::from(info.dwVolumeSerialNumber),
            index: (u128::from(info.nFileIndexHigh) << 32) | u128::from(info.nFileIndexLow),
            nlink: info.nNumberOfLinks,
        })
    }

    /// ネットワーク上か (UNC、またはドライブがリモート)。判定できなければ `false`。
    pub fn is_remote(path: &Path) -> bool {
        let s = path.to_string_lossy();
        let s = s.strip_prefix("\\\\?\\").unwrap_or(&s);
        if s.starts_with("UNC\\") || (s.starts_with("\\\\") && !s.starts_with("\\\\.\\")) {
            return true;
        }
        let mut it = s.chars();
        let (Some(d), Some(':')) = (it.next(), it.next()) else {
            return false;
        };
        if !d.is_ascii_alphabetic() {
            return false;
        }
        let root: Vec<u16> = format!("{}:\\", d.to_ascii_uppercase())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: `root` は NUL 終端の UTF-16。
        unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
    }
}

#[cfg(not(windows))]
mod winapi {
    use std::fs::File;
    use std::io;
    use std::path::Path;

    pub struct HandleInfo {
        pub volume: u64,
        pub index: u128,
        pub nlink: u32,
    }

    pub fn file_info(_: &File) -> io::Result<HandleInfo> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    pub fn is_remote(_: &Path) -> bool {
        false
    }
}

fn fs_kind(path: &Path) -> FsKind {
    if cfg!(windows) {
        if winapi::is_remote(path) {
            FsKind::Network
        } else {
            FsKind::Local
        }
    } else {
        FsKind::Unknown
    }
}

impl FileSystem for WinFs {
    fn inflight_for(&self, path: &FsPath) -> Result<Vec<eve_core::InflightRecord>, FsError> {
        Ok(self
            .state_dir()
            .map(|sd| crate::inflight::read_flags(sd, path.as_path()))
            .unwrap_or_default())
    }

    fn list_recovery_dirs(&self) -> Result<Vec<eve_core::RecoveryDirEntry>, FsError> {
        Ok(crate::rescue::list_recovery_dirs(self.state_dir()))
    }

    fn write_in_place(
        &self,
        path: &FsPath,
        data: &mut dyn ReadChunks,
        len: u64,
        opts: &SaveOptions,
        cancel: &eve_core::CancelToken,
    ) -> Result<SaveReport, FsError> {
        self.write_in_place_impl(path, data, len, opts, cancel)
    }

    fn remove_file(&self, path: &FsPath) -> Result<(), FsError> {
        fs::remove_file(path.as_path()).map_err(|e| map_io_error(e, path))
    }

    fn metadata(&self, path: &FsPath) -> Result<FileMeta, FsError> {
        let p = path.as_path();
        let meta = match fs::metadata(p) {
            Ok(m) => m,
            Err(e) => {
                // リンクは在るがリンク先が無い (ダングリング)
                if e.kind() == io::ErrorKind::NotFound
                    && let Ok(sm) = fs::symlink_metadata(p)
                    && sm.file_type().is_symlink()
                {
                    let target = fs::read_link(p).ok().map(FsPath::new);
                    let detail = target.map(|target| FsErrorDetail::DanglingLink { target });
                    return Err(FsError {
                        kind: FsErrorKind::NotFound,
                        detail,
                        source: Some(e),
                    });
                }
                return Err(map_io_error(e, path));
            }
        };
        let kind = if meta.is_file() {
            FileKind::File
        } else if meta.is_dir() {
            FileKind::Dir
        } else {
            FileKind::Other
        };
        let link = fs::symlink_metadata(p)
            .ok()
            .filter(|sm| sm.file_type().is_symlink())
            .map(|_| LinkInfo {
                kind: LinkKind::Symlink,
                target: fs::read_link(p).ok().map(FsPath::new),
                dangling: false,
            });
        // ファイルの識別とハードリンク数 (開けなければ分からないものとして続ける)
        let (file_id, nlink) = if kind == FileKind::File {
            match File::open(p).and_then(|f| winapi::file_info(&f)) {
                Ok(i) => (
                    Some(eve_core::FileId {
                        volume: i.volume,
                        index: i.index,
                    }),
                    i.nlink,
                ),
                Err(_) => (None, 1),
            }
        } else {
            (None, 1)
        };
        Ok(FileMeta {
            kind,
            len: meta.len(),
            mtime: file_time(meta.modified()),
            readonly: meta.permissions().readonly(),
            file_id,
            nlink,
            link,
            fs_kind: fs_kind(p),
            owner_is_me: None,
        })
    }

    fn canonicalize(&self, path: &FsPath) -> Result<FsPath, FsError> {
        match fs::canonicalize(path.as_path()) {
            // `\\?\C:\…` をそのまま表示・保存に使わない (05 §3.3)
            Ok(c) => Ok(FsPath::new(simplify_verbatim(&c.to_string_lossy()))),
            Err(e) => Err(map_io_error(e, path)),
        }
    }

    fn open_read(&self, path: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
        let f = File::open(path.as_path()).map_err(|e| map_io_error(e, path))?;
        // ディレクトリは開けない (Windows の `File::open` はディレクトリで成功しないが、他 OS のために確かめる)
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

    fn write_atomic(
        &self,
        path: &FsPath,
        data: &mut dyn ReadChunks,
        opts: &SaveOptions,
    ) -> Result<SaveReport, FsError> {
        // 一時ファイル + `ReplaceFileW` (05 §4.1 / §4.2)。実装は `save`
        self.write_atomic_impl(path, data, opts)
    }
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// 一時ディレクトリ (終わりに消す)。
    struct Tmp(std::path::PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let n = std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let p =
                std::env::temp_dir().join(format!("eve-platform-{tag}-{}-{n}", std::process::id()));
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn path(&self, name: &str) -> FsPath {
            FsPath::new(self.0.join(name))
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn read_all(fsys: &WinFs, p: &FsPath) -> Vec<u8> {
        let mut r = fsys.open_read(p).unwrap();
        let mut out = Vec::new();
        while let Some(c) = r.next_chunk().unwrap() {
            out.extend_from_slice(c);
        }
        out
    }

    #[test]
    fn メタ情報と読込() {
        let t = Tmp::new("meta");
        let p = t.path("a.txt");
        File::create(p.as_path())
            .unwrap()
            .write_all("こんにちは\n".as_bytes())
            .unwrap();
        let fsys = WinFs::default();
        let m = fsys.metadata(&p).unwrap();
        assert_eq!(m.kind, FileKind::File);
        assert_eq!(m.len, "こんにちは\n".len() as u64);
        assert!(!m.readonly);
        assert!(m.mtime.secs > 1_600_000_000);
        assert_eq!(m.nlink, 1);
        assert_eq!(read_all(&fsys, &p), "こんにちは\n".as_bytes());
        if cfg!(windows) {
            let id = m.file_id.expect("Windows ではファイルの識別が取れる");
            assert_eq!(
                fsys.metadata(&p).unwrap().file_id,
                Some(id),
                "同じファイルは同じ識別"
            );
            assert_eq!(m.fs_kind, FsKind::Local);
        }
    }

    #[test]
    fn 大きなファイルは複数のチャンクで読める() {
        let t = Tmp::new("big");
        let p = t.path("big.bin");
        let data: Vec<u8> = (0..(READ_CHUNK_BYTES * 2 + 123))
            .map(|i| (i % 251) as u8)
            .collect();
        File::create(p.as_path()).unwrap().write_all(&data).unwrap();
        let fsys = WinFs::default();
        let mut r = fsys.open_read(&p).unwrap();
        let mut chunks = 0;
        let mut out = Vec::new();
        while let Some(c) = r.next_chunk().unwrap() {
            assert!(c.len() <= READ_CHUNK_BYTES);
            chunks += 1;
            out.extend_from_slice(c);
        }
        assert!(chunks >= 3);
        assert_eq!(out, data);
    }

    #[test]
    fn 存在しないパスと親の無いパス() {
        let t = Tmp::new("nf");
        let fsys = WinFs::default();
        let e = fsys.metadata(&t.path("none.txt")).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::NotFound);
        assert!(e.detail.is_none(), "親が在るときは detail なし");
        let e = fsys.metadata(&t.path("nodir\\x.txt")).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::NotFound);
        if cfg!(windows) {
            assert!(
                matches!(e.detail, Some(FsErrorDetail::ParentMissing { .. })),
                "{:?}",
                e.detail
            );
        }
        let e = fsys.open_read(&t.path("none.txt")).map(|_| ()).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::NotFound);
    }

    #[test]
    fn ディレクトリは開けない() {
        let t = Tmp::new("dir");
        let fsys = WinFs::default();
        let d = FsPath::new(t.0.clone());
        assert_eq!(fsys.metadata(&d).unwrap().kind, FileKind::Dir);
        let e = fsys.open_read(&d).map(|_| ()).unwrap_err();
        assert!(
            matches!(
                e.kind,
                FsErrorKind::InvalidPath | FsErrorKind::PermissionDenied
            ),
            "{:?}",
            e.kind
        );
    }

    // Windows の読み取り専用属性を戻すだけ (Unix の書込権限の話ではない)
    #[allow(clippy::permissions_set_readonly_false)]
    #[test]
    fn 読み取り専用属性() {
        let t = Tmp::new("ro");
        let p = t.path("r.txt");
        File::create(p.as_path()).unwrap().write_all(b"x").unwrap();
        let mut perm = fs::metadata(p.as_path()).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(p.as_path(), perm.clone()).unwrap();
        assert!(WinFs::default().metadata(&p).unwrap().readonly);
        perm.set_readonly(false);
        fs::set_permissions(p.as_path(), perm).unwrap();
    }

    #[test]
    fn canonicalize_は_verbatim_を外す() {
        let t = Tmp::new("canon");
        let p = t.path("c.txt");
        File::create(p.as_path()).unwrap().write_all(b"x").unwrap();
        let c = WinFs::default().canonicalize(&p).unwrap();
        let s = c.as_path().to_string_lossy().into_owned();
        assert!(!s.starts_with("\\\\?\\"), "{s}");
        assert!(s.ends_with("c.txt"));
    }

    #[test]
    fn エラーの写像() {
        let p = FsPath::new("C:\\a\\b.txt");
        let k = |code: i32| map_io_error(io::Error::from_raw_os_error(code), &p);
        assert_eq!(k(2).kind, FsErrorKind::NotFound);
        assert!(matches!(
            k(3).detail,
            Some(FsErrorDetail::ParentMissing { .. })
        ));
        assert_eq!(k(5).kind, FsErrorKind::PermissionDenied);
        assert_eq!(k(19).kind, FsErrorKind::ReadOnlyFs);
        assert_eq!(k(32).kind, FsErrorKind::Busy);
        assert_eq!(k(33).kind, FsErrorKind::Busy);
        assert_eq!(k(112).kind, FsErrorKind::DiskFull);
        assert_eq!(k(123).kind, FsErrorKind::InvalidPath);
        let n = k(64);
        assert_eq!(
            (n.kind, n.detail),
            (FsErrorKind::Other, Some(FsErrorDetail::Network))
        );
        assert!(k(5).source.is_some(), "元の OS エラーを保持する");
    }
}
