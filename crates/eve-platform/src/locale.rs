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

/// Unix の環境変数の値 (`LC_ALL` → `LC_MESSAGES` → `LANG` の順で最初の空でない値) から言語タグの候補を作る
/// (05 §6.1)。`ja_JP.UTF-8` → `ja-JP`。`.` 以降 (符号化) と `@` 以降 (修飾子) を落とし、`C` / `POSIX` は値なし。
/// 最初の空でない値が `C` のときは、後ろの変数へは進まない (POSIX の優先順位)。
pub fn tag_from_posix_env(
    lc_all: Option<&str>,
    lc_messages: Option<&str>,
    lang: Option<&str>,
) -> Option<String> {
    let value = [lc_all, lc_messages, lang]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|v| !v.is_empty())?;
    let base = value.split(['.', '@']).next().unwrap_or("");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    Some(base.replace('_', "-"))
}

/// UI の言語の OS ロケール検出 (05 §6.1)。言語タグの候補 (例 `ja-JP`) を返し、取れなければ `None`。
/// Windows は UI 言語 (`GetUserDefaultUILanguage`) を見て、`LANG` / `LC_*` は見ない (Git Bash などが無関係に設定するため)。
#[cfg(windows)]
pub fn ui_language_tag() -> Option<String> {
    #[allow(unsafe_code)]
    // SAFETY: `GetUserDefaultUILanguage` は引数も前提条件もなく LANGID を返すだけ。
    let langid = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    #[allow(unsafe_code)]
    // SAFETY: `buf` は `LOCALE_NAME_MAX_LENGTH` 要素で、長さを `cchName` に渡している。
    let n = unsafe {
        windows_sys::Win32::Globalization::LCIDToLocaleName(
            u32::from(langid),
            buf.as_mut_ptr(),
            buf.len() as i32,
            0,
        )
    };
    // 戻り値は終端の NUL を含む長さ。0 は失敗
    let len = usize::try_from(n).ok()?.checked_sub(1).filter(|l| *l > 0)?;
    String::from_utf16(&buf[..len]).ok()
}

#[cfg(not(windows))]
pub fn ui_language_tag() -> Option<String> {
    let var = |k: &str| std::env::var(k).ok();
    tag_from_posix_env(
        var("LC_ALL").as_deref(),
        var("LC_MESSAGES").as_deref(),
        var("LANG").as_deref(),
    )
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
    fn posix_の環境変数から言語タグを作る() {
        let t = tag_from_posix_env;
        assert_eq!(t(None, None, Some("ja_JP.UTF-8")).as_deref(), Some("ja-JP"));
        assert_eq!(
            t(None, None, Some("en_US.UTF-8@euro")).as_deref(),
            Some("en-US")
        );
        assert_eq!(
            t(Some("en_GB"), Some("ja_JP"), Some("ja_JP")).as_deref(),
            Some("en-GB")
        );
        assert_eq!(
            t(Some(""), Some("ja_JP"), None).as_deref(),
            Some("ja-JP"),
            "空は飛ばす"
        );
        assert_eq!(t(None, None, Some("C")), None);
        assert_eq!(t(None, None, Some("POSIX")), None);
        assert_eq!(
            t(Some("C"), None, Some("ja_JP")),
            None,
            "最初の値が C なら後ろへ進まない"
        );
        assert_eq!(t(None, None, None), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_は_ui_言語のタグを返す() {
        let tag = ui_language_tag().expect("UI 言語を取得できる");
        assert!(tag.len() >= 2 && tag.is_ascii(), "{tag}");
    }

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
