//! OS ごとの小さな部品の窓口。状態ディレクトリの部品 (保存中フラグ・直接上書き・状態ファイル・スワップ・緊急退避・ログ) は
//! OS に依らずここの関数だけを呼ぶ (docs/05 §5.6 / §7.5)。
//!
//! OS の API を呼ぶ unsafe は `sys::windows` / `sys::unix` に閉じ、安全な関数として出す (R5)。保存 (`save` / `unix_fs`)・直接上書き
//! (`inplace`)・保存中フラグ (`inflight`)・スワップの走査 (`swap_scan`)・テストのフック (`hooks`) はそれを呼ぶだけで unsafe を持たない。
//! エラーの写像と保存先の解決は OS ごとのアルゴリズムの側 (`fs` / `save` / `unix_fs`) にあり、ここから出し直す。

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use eve_core::{FileMeta, FileStamp};

#[cfg(unix)]
pub(crate) mod unix;
#[cfg(windows)]
pub(crate) mod windows;

#[cfg(windows)]
pub(crate) use crate::fs::map_io_error;
#[cfg(windows)]
pub(crate) use crate::save::resolve_target;
#[cfg(unix)]
pub(crate) use crate::unix_fs::{map_io_error, resolve_target};
#[cfg(unix)]
pub(crate) use unix::*;
#[cfg(windows)]
pub(crate) use windows::*;

/// 照合の比較 (05 §5.5。ハッシュは持たないので (mtime, len, FileId))。
pub(crate) fn matches(want: &FileStamp, have: &FileStamp) -> bool {
    want.mtime == have.mtime && want.len == have.len && want.file_id == have.file_id
}

pub(crate) fn same_file_state(a: &FileMeta, b: &FileMeta) -> bool {
    a.mtime == b.mtime && a.len == b.len && a.file_id == b.file_id
}

/// 状態ディレクトリの中のファイルを書く `OpenOptions` (05 §5.6 の「所有者のみ」。Unix は `0600` で作る。umask に依らない。
/// Windows はディレクトリの保護付き DACL を継承する)。
pub(crate) fn private_options() -> OpenOptions {
    #[cfg_attr(windows, allow(unused_mut))]
    let mut o = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
}

/// `File::create` の所有者のみ版 (在れば切り詰める)。
pub(crate) fn create_private(path: &Path) -> io::Result<File> {
    private_options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

/// 緊急退避の代替の退避先 (05 §7.5)。Windows は `%TEMP%\eve-recovery-<SID>\`、Unix は `$TMPDIR/eve-recovery-<uid>/`
/// (`TMPDIR` が無ければ `/tmp`。`std::env::temp_dir` と同じ規則)。
pub(crate) fn recovery_fallback_dir() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let who = current_user_sid()?;
    #[cfg(unix)]
    let who = euid().to_string();
    Ok(std::env::temp_dir().join(format!("eve-recovery-{who}")))
}
