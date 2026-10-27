//! 内容ハッシュ (XXH3-64、docs/05 §5.5): 外部変更の検知 (FR-117) で、mtime と長さが同じ書き換えを見つけ、
//! 内容が同じで mtime だけが変わったもの (同期ソフト・ウイルス対策) を変更とみなさないための比較キー。
//! 16 MiB 以下のファイルだけ、読込ジョブ (ストリーミング) と保存ジョブ (書いた内容) の中で計算する。

use std::hash::Hasher;

use twox_hash::XxHash3_64;

use crate::fs::{ContentHash, FileStamp, FileSystem, FsError, FsErrorKind};
use crate::fspath::FsPath;

/// ハッシュを持つファイルの大きさの上限 (バイト)。これを超えるファイルは `(mtime, len, FileId)` だけで比べる。
pub(crate) const HASH_MAX_LEN: u64 = 16 * 1024 * 1024;

/// ストリーミングのハッシュ計算。
pub(crate) struct ContentHasher(XxHash3_64);

impl ContentHasher {
    pub(crate) fn new() -> ContentHasher {
        ContentHasher(XxHash3_64::new())
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }

    pub(crate) fn finish(&self) -> ContentHash {
        ContentHash(self.0.finish().to_be_bytes())
    }
}

/// `path` の内容のハッシュ。`len` が上限を超えるとき・読んでいる間に大きさが変わったときは `Ok(None)`。
pub(crate) fn hash_file(
    fs: &dyn FileSystem,
    path: &FsPath,
    len: u64,
) -> Result<Option<ContentHash>, FsError> {
    if len > HASH_MAX_LEN {
        return Ok(None);
    }
    let mut reader = fs.open_read(path)?;
    let mut hasher = ContentHasher::new();
    let mut total = 0u64;
    loop {
        match reader.next_chunk() {
            Ok(Some(c)) => {
                total += c.len() as u64;
                if total > len {
                    return Ok(None);
                }
                hasher.update(c);
            }
            Ok(None) => break,
            Err(crate::fs::ChunkError::Io(e)) => return Err(e),
            Err(_) => return Err(FsError::new(FsErrorKind::Other)),
        }
    }
    Ok((total == len).then(|| hasher.finish()))
}

/// 保存の前の照合 (05 §5.5。書き始める前の 1 回だけ): 内容ハッシュで `want` (開いたときの Stamp) と今のファイルを比べる。
///
/// - `mtime` も `len` も同じ: 内容が違えば `Conflict` (mtime の分解能の内の書き換えを見逃さない)。
/// - `len` と `FileId` が同じで `mtime` だけが違う: 内容が同じなら変更なしとみなし、新しい mtime の Stamp を返す
///   (以後の照合に使う。同期ソフトが mtime だけを更新したときに確認を出さない)。
/// - それ以外 (長さや FileId が違う・ハッシュが無い・16 MiB 超・存在しない): そのまま返す (Port の照合に任せる)。
pub(crate) fn reconcile(
    fs: &dyn FileSystem,
    path: &FsPath,
    want: FileStamp,
) -> Result<FileStamp, FsError> {
    let Some(want_hash) = want.hash else {
        return Ok(want);
    };
    let Ok(meta) = fs.metadata(path) else {
        return Ok(want);
    };
    let have = FileStamp::from_meta(&meta, None);
    if have.len != want.len || have.file_id != want.file_id || have.len > HASH_MAX_LEN {
        return Ok(want);
    }
    let Some(have_hash) = hash_file(fs, path, have.len)? else {
        return Ok(want);
    };
    let have = FileStamp {
        hash: Some(have_hash),
        ..have
    };
    if have.mtime == want.mtime {
        if have_hash == want_hash {
            Ok(want)
        } else {
            Err(FsError::new(FsErrorKind::Conflict {
                current: Some(have),
            }))
        }
    } else if have_hash == want_hash {
        Ok(have)
    } else {
        Ok(want)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 同じ入力は同じハッシュで_分割しても変わらない() {
        let mut a = ContentHasher::new();
        a.update(b"hello world");
        let mut b = ContentHasher::new();
        b.update(b"hello ");
        b.update(b"world");
        assert_eq!(a.finish(), b.finish());
        let mut c = ContentHasher::new();
        c.update(b"hello worle");
        assert_ne!(a.finish(), c.finish());
    }

    #[test]
    fn 既知の値() {
        // XXH3-64 (seed 0) の空入力は 2D06800538D394C2
        let h = ContentHasher::new().finish();
        assert_eq!(h.0, 0x2D06_8005_38D3_94C2u64.to_be_bytes());
    }
}
