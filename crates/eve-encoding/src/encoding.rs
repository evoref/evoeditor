//! 文字コードの種類 (docs/03-core-design.md §8.1)。v0.1 で読み書きするのは UTF-8・UTF-16LE/BE・Shift_JIS (CP932)・EUC-JP・ISO-2022-JP。
//! encoding_rs の静的な `Encoding` を包み、UTF-16 は自前の書き手を持つ (encoding_rs は UTF-16 のエンコードを提供しない、§8.6)。

/// 読み書きできる文字コード。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    /// Shift_JIS (Windows の CP932 と同じ写像、encoding_rs の `SHIFT_JIS`)。
    ShiftJis,
    EucJp,
    /// 状態付き符号化 (`is_stateful`)。
    Iso2022Jp,
}

impl Encoding {
    /// 全種類 (設定の `file.detection_candidates` の既定など)。
    pub const ALL: [Encoding; 6] = [
        Encoding::Utf8,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
        Encoding::ShiftJis,
        Encoding::EucJp,
        Encoding::Iso2022Jp,
    ];

    /// 画面・設定の表記 (ステータスバーの値は翻訳しない、04 §8.7)。
    pub fn name(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf16Le => "UTF-16LE",
            Encoding::Utf16Be => "UTF-16BE",
            Encoding::ShiftJis => "Shift_JIS",
            Encoding::EucJp => "EUC-JP",
            Encoding::Iso2022Jp => "ISO-2022-JP",
        }
    }

    /// ステータスバーの短い形 (`SJIS` など。07 §11.6 / 04 §8.7)。短くできないものは `name` と同じ。
    pub fn short_name(self) -> &'static str {
        match self {
            Encoding::ShiftJis => "SJIS",
            Encoding::Iso2022Jp => "JIS",
            e => e.name(),
        }
    }

    /// 設定・コマンド行の指定 (`--encoding`、`file.default_encoding`) から読む。大小・`-` `_` の違いを許す。
    pub fn from_label(label: &str) -> Option<Encoding> {
        let norm: String = label
            .trim()
            .chars()
            .filter(|c| !matches!(c, '-' | '_' | ' '))
            .flat_map(char::to_lowercase)
            .collect();
        Some(match norm.as_str() {
            "utf8" => Encoding::Utf8,
            "utf16le" | "utf16" => Encoding::Utf16Le,
            "utf16be" => Encoding::Utf16Be,
            "shiftjis" | "sjis" | "cp932" | "windows31j" | "mskanji" | "xsjis" => {
                Encoding::ShiftJis
            }
            "eucjp" | "ujis" | "xeucjp" => Encoding::EucJp,
            "iso2022jp" | "jis" | "csiso2022jp" => Encoding::Iso2022Jp,
            _ => return None,
        })
    }

    /// 状態付き符号化 (ISO-2022-JP のみ)。不正バイトを元のバイトに戻して書くことができない (§8.3)。
    pub fn is_stateful(self) -> bool {
        self == Encoding::Iso2022Jp
    }

    pub fn is_utf16(self) -> bool {
        matches!(self, Encoding::Utf16Le | Encoding::Utf16Be)
    }

    /// 7 bit だけで書かれる文字コード。
    pub fn is_7bit(self) -> bool {
        self == Encoding::Iso2022Jp
    }

    /// BOM のバイト列 (BOM を持てる文字コードだけ)。
    pub fn bom(self) -> Option<&'static [u8]> {
        match self {
            Encoding::Utf8 => Some(b"\xEF\xBB\xBF"),
            Encoding::Utf16Le => Some(b"\xFF\xFE"),
            Encoding::Utf16Be => Some(b"\xFE\xFF"),
            _ => None,
        }
    }

    /// デコード後の本文の大きさの倍率の見積り (§8.3 の `memory_factor`。Shift_JIS・EUC-JP・UTF-16 は 1.5、UTF-8 は 1)。
    pub fn memory_factor(self) -> f64 {
        match self {
            Encoding::Utf8 | Encoding::Iso2022Jp => 1.0,
            _ => 1.5,
        }
    }

    pub(crate) fn rs(self) -> &'static encoding_rs::Encoding {
        match self {
            Encoding::Utf8 => encoding_rs::UTF_8,
            Encoding::Utf16Le => encoding_rs::UTF_16LE,
            Encoding::Utf16Be => encoding_rs::UTF_16BE,
            Encoding::ShiftJis => encoding_rs::SHIFT_JIS,
            Encoding::EucJp => encoding_rs::EUC_JP,
            Encoding::Iso2022Jp => encoding_rs::ISO_2022_JP,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 表記から読む() {
        assert_eq!(Encoding::from_label("UTF-8"), Some(Encoding::Utf8));
        assert_eq!(Encoding::from_label("shift_jis"), Some(Encoding::ShiftJis));
        assert_eq!(Encoding::from_label("CP932"), Some(Encoding::ShiftJis));
        assert_eq!(Encoding::from_label("euc-jp"), Some(Encoding::EucJp));
        assert_eq!(
            Encoding::from_label("ISO-2022-JP"),
            Some(Encoding::Iso2022Jp)
        );
        assert_eq!(Encoding::from_label("utf-16be"), Some(Encoding::Utf16Be));
        assert_eq!(Encoding::from_label("latin1"), None);
        for e in Encoding::ALL {
            assert_eq!(Encoding::from_label(e.name()), Some(e), "{}", e.name());
        }
    }

    #[test]
    fn 状態付きは_iso_2022_jp_だけ() {
        assert!(Encoding::Iso2022Jp.is_stateful());
        assert!(Encoding::ALL.iter().filter(|e| e.is_stateful()).count() == 1);
    }
}
