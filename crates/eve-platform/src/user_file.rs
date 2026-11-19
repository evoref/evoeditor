//! 利用者が設定ディレクトリに置いた小さなファイルを、上限まで読む (ユーザーの言語ファイル `locales/<lang>.toml`、
//! docs/07 §17.4 の「開き方」)。
//!
//! 開いた後のハンドルで通常のファイルであることを確かめ (ディレクトリ・デバイス・FIFO は読まない)、読むのは上限 + 1 バイト
//! まで (上限を超えたかを、開く前の metadata ではなく読んだ量で決める)。Unix では FIFO を開くときに書き手を待って止まらない
//! ように `O_NONBLOCK` で開く (通常のファイルの読込には効かない)。Windows ではディレクトリも開けるように
//! `FILE_FLAG_BACKUP_SEMANTICS` で開き、開いた後に通常のファイルかを確かめる。

use std::io::Read as _;
use std::path::Path;

/// 読めなかった理由。
#[derive(Debug)]
pub enum UserFileError {
    /// 通常のファイルでない (ディレクトリ・デバイス・FIFO など)。読んでいない。
    NotRegular,
    /// 開けない・読めない (権限・I/O の誤り。ファイルが無いことを含む)。
    Io(std::io::Error),
}

/// `path` を開き、通常のファイルなら先頭から `limit + 1` バイトまで読む。読んだ量が `limit` を超えていれば、呼び出し側が
/// 大きすぎると決める (このバイト列は `limit + 1` バイト)。
///
/// # Errors
///
/// 開けない・読めないとき `UserFileError::Io`、通常のファイルでないとき `UserFileError::NotRegular`。
pub fn read_regular_limited(path: &Path, limit: usize) -> Result<Vec<u8>, UserFileError> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // FIFO の書き手を待たない。端末を制御端末にしない
        opts.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        // ディレクトリも開けるようにして、開いた後のハンドルで「通常のファイルでない」と決める
        opts.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS);
    }
    let file = opts.open(path).map_err(UserFileError::Io)?;
    let meta = file.metadata().map_err(UserFileError::Io)?;
    if !meta.is_file() {
        return Err(UserFileError::NotRegular);
    }
    let max = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut out = Vec::new();
    file.take(max)
        .read_to_end(&mut out)
        .map_err(UserFileError::Io)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("eve-user-file-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn 上限_1_バイト多くまで読む() {
        let d = temp_dir("limit");
        let p = d.join("fr.toml");
        std::fs::write(&p, b"0123456789").unwrap();
        assert_eq!(read_regular_limited(&p, 100).unwrap(), b"0123456789");
        assert_eq!(read_regular_limited(&p, 10).unwrap(), b"0123456789");
        assert_eq!(
            read_regular_limited(&p, 4).unwrap(),
            b"01234",
            "上限 + 1 バイトまで"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ディレクトリは通常のファイルでない() {
        let d = temp_dir("dir");
        let p = d.join("fr.toml");
        std::fs::create_dir_all(&p).unwrap();
        assert!(matches!(
            read_regular_limited(&p, 10),
            Err(UserFileError::NotRegular)
        ));
        assert!(matches!(
            read_regular_limited(&d.join("none.toml"), 10),
            Err(UserFileError::Io(_))
        ));
        let _ = std::fs::remove_dir_all(&d);
    }
}
