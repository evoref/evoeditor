//! ファインダの候補 `Candidate` (docs/03 §12.7.2 の「候補の持ち方と解放」)。
//!
//! 候補は走査の側 (`bg`) で作り、4,096 件前後の不変の塊 (`Arc<[Candidate]>`) にして主スレッドへ送る。主スレッドは塊の
//! `Arc` を列に足すだけで、候補を 1 件ずつ作らない・複製しない。

use std::path::{Path, PathBuf};

use crate::io::fspath::FsPath;

/// ワークスペースの中のファイル 1 つ (ファインダの候補)。
///
/// 相対パスは区切りを全 OS で `/` にした UTF-8 の文字列で、照合と表示に使う。相対パスの要素のどれか 1 つでも UTF-8 で
/// 表せないときは、置換文字 (U+FFFD) にした表示用の文字列と、開くための相対パス全体の元の形 ([`Candidate::path`] が使う)
/// の両方を持つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    rel: Box<str>,
    lower: Box<[u8]>,
    name_start: u32,
    chars: u64,
    raw: Option<Box<Path>>,
}

impl Candidate {
    /// UTF-8 の相対パス (区切りは `/`) から作る。
    #[must_use]
    pub fn new(rel: &str) -> Candidate {
        Candidate::build(rel.into(), None)
    }

    /// 走査が作る: `rel` は区切りが `/` の相対パスのバイト列 (Unix は元のバイト列、Windows は WTF-8)、`os_rel` は同じ相対パスの
    /// OS の形 (UTF-8 で表せないときだけ持つ)。
    pub(crate) fn from_walk(rel: &[u8], os_rel: impl FnOnce() -> PathBuf) -> Candidate {
        match std::str::from_utf8(rel) {
            Ok(s) => Candidate::build(s.into(), None),
            Err(_) => Candidate::build(
                String::from_utf8_lossy(rel).into(),
                Some(os_rel().into_boxed_path()),
            ),
        }
    }

    fn build(rel: Box<str>, raw: Option<Box<Path>>) -> Candidate {
        let lower: Box<[u8]> = rel.bytes().map(|b| b.to_ascii_lowercase()).collect();
        let chars = lower.iter().fold(0u64, |acc, &b| acc | char_bit(b));
        let name_start = rel.rfind('/').map_or(0, |i| i + 1);
        Candidate {
            name_start: u32::try_from(name_start).unwrap_or(u32::MAX),
            rel,
            lower,
            chars,
            raw,
        }
    }

    /// ルートからの相対パス (区切りは `/`。UTF-8 で表せない部分は U+FFFD)。照合と表示に使う。
    #[must_use]
    pub fn rel(&self) -> &str {
        &self.rel
    }

    /// 相対パスの ASCII の英字だけを小文字にしたバイト列 (照合は ASCII の英字だけを畳む、03 §12.7.4)。
    #[must_use]
    pub fn lower(&self) -> &[u8] {
        &self.lower
    }

    /// ファイル名 (最後の `/` の後ろ) の開始位置 (相対パスの中のバイト位置)。
    #[must_use]
    pub fn name_start(&self) -> usize {
        usize::try_from(self.name_start).unwrap_or(usize::MAX)
    }

    /// ファイル名 (相対パスの最後の要素)。
    #[must_use]
    pub fn name(&self) -> &str {
        self.rel.get(self.name_start()..).unwrap_or_default()
    }

    /// 相対パスに含む文字の集合 ([`char_bit`] の和)。照合の速い棄却に使う。
    #[must_use]
    pub fn chars(&self) -> u64 {
        self.chars
    }

    /// 相対パスのどれかの要素が UTF-8 で表せず、表示用の文字列が置換文字を含む。
    #[must_use]
    pub fn is_lossy(&self) -> bool {
        self.raw.is_some()
    }

    /// ルート `root` の下の絶対パス (名前が UTF-8 で表せないときは元の形から作る)。区切りは OS の形で足す (Windows の
    /// verbatim の接頭辞 `\\?\` の付いたルートでも、`/` を区切りとして読まれない形にする)。
    #[must_use]
    pub fn path(&self, root: &FsPath) -> FsPath {
        if let Some(raw) = &self.raw {
            return root.join(raw);
        }
        let mut p = root.as_path().to_path_buf();
        for part in self.rel.split('/') {
            p.push(part);
        }
        FsPath::new(p)
    }
}

/// 文字の集合の 1 ビット (照合の速い棄却に使う、03 §12.7.2)。照らす検索語と候補で同じ関数を使う。ASCII の英字は大文字小文字を
/// 畳んで `a`〜`z` を 0〜25、数字を 26〜35、ほかの ASCII を 36〜62 に (重なってよい)、ASCII の外のバイトを 63 に写す。
/// 写し方は関数なので、検索語の文字のビットが候補の集合に無ければ、その候補は一致しない。
#[must_use]
pub fn char_bit(b: u8) -> u64 {
    let b = b.to_ascii_lowercase();
    let bit = match b {
        b'a'..=b'z' => b - b'a',
        b'0'..=b'9' => 26 + (b - b'0'),
        0x80..=0xFF => 63,
        _ => 36 + b % 27,
    };
    1u64 << bit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 相対パスから名前と小文字と文字の集合を作る() {
        let c = Candidate::new("src/Editor/Mod.rs");
        assert_eq!(c.rel(), "src/Editor/Mod.rs");
        assert_eq!(c.name(), "Mod.rs");
        assert_eq!(c.name_start(), 11);
        assert_eq!(c.lower(), b"src/editor/mod.rs");
        for b in b"srceditomd./" {
            assert_ne!(c.chars() & char_bit(*b), 0, "{}", *b as char);
        }
        assert_eq!(c.chars() & char_bit(b'z'), 0);
        assert!(!c.is_lossy());
    }

    #[test]
    fn 区切りの無い名前と_ascii_の外の文字() {
        let c = Candidate::new("資料.txt");
        assert_eq!(c.name(), "資料.txt");
        assert_eq!(c.name_start(), 0);
        assert_eq!(c.lower(), "資料.txt".as_bytes(), "ASCII の外は畳まない");
        assert_ne!(c.chars() & (1 << 63), 0);
    }

    #[test]
    fn 文字のビットは大文字小文字を畳み_範囲に収まる() {
        for b in 0..=255u8 {
            assert_eq!(char_bit(b).count_ones(), 1);
            assert_eq!(char_bit(b), char_bit(b.to_ascii_lowercase()));
        }
        assert_eq!(char_bit(b'a'), 1);
        assert_eq!(char_bit(b'0'), 1 << 26);
    }

    #[test]
    fn 絶対パスは要素ごとに足す() {
        let c = Candidate::new("a/b/c.txt");
        let p = c.path(&FsPath::from("/w"));
        let want: PathBuf = ["/w", "a", "b", "c.txt"].iter().collect();
        assert_eq!(p.as_path(), want.as_path());
    }

    #[test]
    fn utf8_で表せない名前は元の形も持つ() {
        let c = Candidate::from_walk(b"d/\xFFx", || PathBuf::from("d").join("raw"));
        assert!(c.is_lossy());
        assert_eq!(c.rel(), "d/\u{FFFD}x");
        assert_eq!(c.name(), "\u{FFFD}x");
        let p = c.path(&FsPath::from("/w"));
        assert_eq!(p.as_path(), Path::new("/w").join("d").join("raw").as_path());
    }
}
