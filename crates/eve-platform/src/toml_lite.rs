//! 状態ディレクトリの小さな TOML (保存中フラグ・緊急退避とスワップの `manifest.toml`、05 §7.5) の読み書き。
//! 使うのは `key = value` の行・基本文字列・`[[表の配列]]` の見出しだけ。TOML の crate は使わない (依存を増やさない)。
//! ディスク上の形式はここで決まる。変えるときは読み手の互換 (未知のキーは無視、版は `format_version`) を保つ。

/// `key = value` の行を積む書き手。値は書いたまま (文字列は `basic_string` で引用する)。
#[derive(Default)]
pub(crate) struct Writer {
    text: String,
}

impl Writer {
    /// `key = value` の 1 行。
    pub(crate) fn kv(&mut self, key: &str, value: &str) {
        self.text.push_str(key);
        self.text.push_str(" = ");
        self.text.push_str(value);
        self.text.push('\n');
    }

    /// 値を基本文字列にした `key = "value"` の 1 行。
    pub(crate) fn str(&mut self, key: &str, value: &str) {
        self.kv(key, &basic_string(value));
    }

    /// 空行と `[[name]]` の見出し (表の配列の 1 要素の始まり)。
    pub(crate) fn table_array(&mut self, name: &str) {
        self.text.push_str("\n[[");
        self.text.push_str(name);
        self.text.push_str("]]\n");
    }

    pub(crate) fn finish(self) -> String {
        self.text
    }
}

/// TOML の基本文字列にする。
pub(crate) fn basic_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7F => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// TOML の基本文字列 (引用符の中) をほどく。`None` は不正。
pub(crate) fn parse_basic_string(v: &str) -> Option<String> {
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next()? {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let hex: String = it.by_ref().take(4).collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// 1 行を `key = value` に分ける (最初の ` = ` で分け、両側の空白を除く)。` = ` が無い行は `None`。
pub(crate) fn key_value(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once(" = ")?;
    Some((k.trim(), v.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 文字列の往復() {
        for s in ["plain", "a\"b\\c", "日本語 \t\n\r", "\u{1}x\u{7f}", ""] {
            assert_eq!(
                parse_basic_string(&basic_string(s)).as_deref(),
                Some(s),
                "{s:?}"
            );
        }
        assert_eq!(basic_string("\u{1}\u{7f}é"), "\"\\u0001\\u007Fé\"");
        assert_eq!(parse_basic_string("no quotes"), None);
        assert_eq!(parse_basic_string("\"\\x\""), None);
    }

    #[test]
    fn 書き手の形() {
        let mut w = Writer::default();
        w.kv("a", "1");
        w.str("b", "x\"y");
        w.table_array("document");
        w.kv("c", "true");
        assert_eq!(
            w.finish(),
            "a = 1\nb = \"x\\\"y\"\n\n[[document]]\nc = true\n"
        );
    }

    #[test]
    fn 行の分け方() {
        assert_eq!(key_value("  k = v  "), Some(("k", "v")));
        assert_eq!(key_value("k = "), Some(("k", "")));
        assert_eq!(key_value("k = a = b"), Some(("k", "a = b")));
        assert_eq!(key_value("k=v"), None);
        assert_eq!(key_value("k ="), None);
    }
}
