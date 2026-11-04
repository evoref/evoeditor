//! `state_dir` の直下の小さな状態ファイル (`last_version.toml` など、05 §7.1 / §7.5) の読み書き。
//! 書込は一時ファイル + 置換 (最終書込勝ち、05 §7.5 (b))。`state_dir` が無ければ所有者のみで作る (起動経路では呼ばれない)。
//! 置いたファイルの ACL は `state_dir` から継承する (05 §5.6 の「それ以外の `state_dir` 配下」)。

use std::fs;
use std::io::{self, Write as _};
use std::path::Path;

use eve_core::{FsError, FsPath};

use crate::inflight::create_dir_all_private;
use crate::rescue::timestamp_basic;
use crate::sys::map_io_error;

fn checked(name: &str) -> io::Result<&str> {
    // 直下のファイル名だけ (区切り・親への移動を含めない)
    if name.is_empty() || name.contains(['/', '\\', ':']) || name == "." || name == ".." {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    Ok(name)
}

pub(crate) fn read(state_dir: &Path, name: &str) -> Result<Vec<u8>, FsError> {
    let path = state_dir.join(name);
    checked(name)
        .and_then(|_| fs::read(&path))
        .map_err(|e| map_io_error(e, &FsPath::new(path)))
}

pub(crate) fn write(state_dir: &Path, name: &str, data: &[u8]) -> Result<(), FsError> {
    let path = state_dir.join(name);
    write_io(state_dir, name, data).map_err(|e| map_io_error(e, &FsPath::new(path)))
}

fn write_io(state_dir: &Path, name: &str, data: &[u8]) -> io::Result<()> {
    let name = checked(name)?;
    create_dir_all_private(state_dir)?;
    let tmp = state_dir.join(format!("{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut f = crate::sys::create_private(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        // 置換 (同じボリューム内の `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`)
        fs::rename(&tmp, state_dir.join(name))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub(crate) fn set_aside(state_dir: &Path, name: &str) -> Result<(), FsError> {
    let path = state_dir.join(name);
    (|| {
        let name = checked(name)?;
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
        let to = state_dir.join(format!("{name}.unsupported-{}", timestamp_basic(secs)));
        // 既にあれば上書きしない (`rename` は置き換えるので、先に確かめる)
        if to.exists() {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        fs::rename(&path, to)
    })()
    .map_err(|e| map_io_error(e, &FsPath::new(path.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 状態ファイルを書いて読み_退避できる() {
        let dir = std::env::temp_dir().join(format!("eve-state-file-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let sd = dir.join("state");
        // 無いときは NotFound、書けば state_dir ごと作る
        assert_eq!(
            read(&sd, "a.toml").unwrap_err().kind,
            eve_core::FsErrorKind::NotFound
        );
        write(&sd, "a.toml", b"x = 1\n").unwrap();
        write(&sd, "a.toml", b"x = 2\n").unwrap();
        assert_eq!(read(&sd, "a.toml").unwrap(), b"x = 2\n");
        // 一時ファイルは残らない
        let names: Vec<_> = fs::read_dir(&sd)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.toml"]);
        set_aside(&sd, "a.toml").unwrap();
        assert!(!sd.join("a.toml").exists());
        let names: Vec<_> = fs::read_dir(&sd)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().all(|n| n.starts_with("a.toml.unsupported-")),
            "{names:?}"
        );
        assert!(write(&sd, "../x", b"").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
