//! `FsPath`: パスの型 (docs/02-architecture.md §4.4)。`PathBuf` の newtype で、非 UTF-8 のパスや Windows の不正な UTF-16 を
//! そのまま保持する (05 §3.5)。Port・ジョブ・エラー・文書のパスはすべてこの型を使う。
//!
//! 境界型 (eve-view) はパスを `String` で運ぶ。変換は eve-core だけが行い、`FsPath` → `String` は表示用の一方向
//! (`display`)。`String` から `FsPath` へは戻さない (境界から入るパスは `Paths::normalize` を通す)。

use std::fmt;
use std::path::{Path, PathBuf};

/// ファイルシステム上のパス。
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FsPath(PathBuf);

impl FsPath {
    pub fn new(path: impl Into<PathBuf>) -> FsPath {
        FsPath(path.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// 親のパス。ルートや空のパスは `None`。
    pub fn parent(&self) -> Option<FsPath> {
        self.0
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| FsPath(p.to_path_buf()))
    }

    pub fn join(&self, child: impl AsRef<Path>) -> FsPath {
        FsPath(self.0.join(child))
    }

    /// 最後の要素 (ファイル名)。表示用 (UTF-8 に変換できない部分は U+FFFD)。
    pub fn file_name(&self) -> Option<String> {
        self.0.file_name().map(|n| n.to_string_lossy().into_owned())
    }

    /// 表示用のパス。UTF-8 に変換できない部分は U+FFFD にし、Windows の `\\?\` 接頭辞は外す (05 §3.3 の簡略化)。
    /// ホームの短縮 (`~/…`) は `Paths` の値を使う変換関数で行う (後のタスク)。
    pub fn display(&self) -> String {
        let s = self.0.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else if let Some(rest) = s.strip_prefix(r"\\?\") {
            rest.to_owned()
        } else {
            s.into_owned()
        }
    }
}

// パス全体をログやパニックに出さない (docs/09 §8.4「秘匿」、NFR-504)。`Debug` はファイル名だけを出す。
impl fmt::Debug for FsPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.file_name() {
            Some(name) => write!(f, "FsPath(…/{name})"),
            None => write!(f, "FsPath(…)"),
        }
    }
}

impl From<&str> for FsPath {
    fn from(s: &str) -> FsPath {
        FsPath(PathBuf::from(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 表示用のパスは接頭辞を外す() {
        assert_eq!(FsPath::from(r"\\?\C:\dir\a.txt").display(), r"C:\dir\a.txt");
        assert_eq!(
            FsPath::from(r"\\?\UNC\server\share\a.txt").display(),
            r"\\server\share\a.txt"
        );
        assert_eq!(FsPath::from("/tmp/a.txt").display(), "/tmp/a.txt");
    }

    #[test]
    fn デバッグ出力にディレクトリを含めない() {
        let p = FsPath::from("/home/someone/secret/doc.txt");
        let s = format!("{p:?}");
        assert!(s.contains("doc.txt"));
        assert!(!s.contains("someone") && !s.contains("secret"), "{s}");
    }

    #[test]
    fn 親と結合() {
        let p = FsPath::from("/a/b/c.txt");
        assert_eq!(p.parent(), Some(FsPath::from("/a/b")));
        assert_eq!(p.file_name().as_deref(), Some("c.txt"));
        assert_eq!(FsPath::from("/a").join("b"), FsPath::from("/a/b"));
        assert_eq!(FsPath::from("x").parent(), None, "親が空なら None");
    }
}
