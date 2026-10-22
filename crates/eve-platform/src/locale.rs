//! OS のロケール由来のヒント (docs/05 §6 の「`"system"` の意味」)。`file.default_encoding = "system"` が使う。
//!
//! Windows は `GetACP()` のコードページ (日本語環境の 932 → `shift_jis`)。写像は OS を呼ばない純粋な関数で、どの OS でも試せる。

/// ANSI コードページ → 文字コードのラベル (07 §5.2 の許容値)。対応しないコードページは `None`。
pub fn encoding_for_codepage(codepage: u32) -> Option<&'static str> {
    Some(match codepage {
        932 => "shift_jis",
        20932 | 51932 => "euc-jp",
        50220..=50222 => "iso-2022-jp",
        65001 => "utf-8",
        _ => return None,
    })
}

/// このマシンの既定の文字コード (ヒント)。Windows だけ。
#[cfg(windows)]
pub fn system_encoding() -> Option<&'static str> {
    #[allow(unsafe_code)]
    // SAFETY: `GetACP` は引数も前提条件もなく、現在の ANSI コードページを返すだけ。
    let cp = unsafe { windows_sys::Win32::Globalization::GetACP() };
    encoding_for_codepage(cp)
}

#[cfg(not(windows))]
pub fn system_encoding() -> Option<&'static str> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn コードページを文字コードに写す() {
        assert_eq!(encoding_for_codepage(932), Some("shift_jis"));
        assert_eq!(encoding_for_codepage(51932), Some("euc-jp"));
        assert_eq!(encoding_for_codepage(50221), Some("iso-2022-jp"));
        assert_eq!(encoding_for_codepage(65001), Some("utf-8"));
        assert_eq!(
            encoding_for_codepage(1252),
            None,
            "西欧のコードページは対応しない"
        );
    }
}
