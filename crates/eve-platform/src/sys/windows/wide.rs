//! `…W` 関数に渡す NUL 終端の UTF-16 (長いパスは拡張長形式)。

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

/// Win32 の `…W` 関数に渡す NUL 終端の UTF-16。長いパスは拡張長形式 (`\\?\`) にする (`extend_long`)。
pub(crate) fn wide(p: &Path) -> Vec<u16> {
    let mut w = extend_long(p.as_os_str().encode_wide().collect());
    w.push(0);
    w
}

/// NUL 終端の UTF-16 (拡張長形式にしない)。
pub(crate) fn wide_plain(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// `MAX_PATH` に近い長さのパスを、`\\?\C:\…` / `\\?\UNC\server\share\…` の形にする (NFR-404)。
/// Rust の `std::fs` は自動でこの形にするが、`ReplaceFileW` などの直接の呼出しは自動では変換されない。
/// 短いパス・相対パス・すでに `\\?\` / `\\.\` のものは変えない (拡張長形式は `.` `..` `/` を解釈しないため、必要なときだけ使う)。
pub(crate) fn extend_long(w: Vec<u16>) -> Vec<u16> {
    // MAX_PATH (260) から、ファイル名と NUL の余裕を引いた値 (ディレクトリの作成の上限 248 より手前)
    const LIMIT: usize = 240;
    let bs = u16::from(b'\\');
    let is_alpha = |c: u16| u8::try_from(c).is_ok_and(|b| b.is_ascii_alphabetic());
    if w.len() < LIMIT {
        return w;
    }
    let unc = w.len() >= 3 && w[0] == bs && w[1] == bs;
    let mut out: Vec<u16> = if unc {
        if w[2] == u16::from(b'?') || w[2] == u16::from(b'.') {
            return w;
        }
        r"\\?\UNC\"
            .encode_utf16()
            .chain(w[2..].iter().copied())
            .collect()
    } else if w.len() >= 3
        && is_alpha(w[0])
        && w[1] == u16::from(b':')
        && (w[2] == bs || w[2] == u16::from(b'/'))
    {
        r"\\?\".encode_utf16().chain(w.iter().copied()).collect()
    } else {
        return w;
    };
    for c in &mut out {
        if *c == u16::from(b'/') {
            *c = bs;
        }
    }
    out
}

#[cfg(test)]
mod long_path_tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn 短いパスは変えない() {
        assert_eq!(extend_long(w(r"C:\a\b.txt")), w(r"C:\a\b.txt"));
        assert_eq!(
            extend_long(w(r"\\srv\share\a.txt")),
            w(r"\\srv\share\a.txt")
        );
    }

    #[test]
    fn 長いパスは拡張長形式にする() {
        let tail = r"d\".repeat(130);
        assert_eq!(
            extend_long(w(&format!(r"C:\{tail}f.txt"))),
            w(&format!(r"\\?\C:\{tail}f.txt"))
        );
        assert_eq!(
            extend_long(w(&format!(r"\\srv\share\{tail}f.txt"))),
            w(&format!(r"\\?\UNC\srv\share\{tail}f.txt"))
        );
        // `/` は `\` にする
        assert_eq!(
            extend_long(w(&format!("C:/{}f.txt", "d/".repeat(130)))),
            w(&format!(r"\\?\C:\{tail}f.txt"))
        );
    }

    #[test]
    fn すでに拡張長形式や相対パスは変えない() {
        let tail = r"d\".repeat(130);
        let v = w(&format!(r"\\?\C:\{tail}f.txt"));
        assert_eq!(extend_long(v.clone()), v);
        let r = w(&format!("{tail}f.txt"));
        assert_eq!(extend_long(r.clone()), r);
    }
}
