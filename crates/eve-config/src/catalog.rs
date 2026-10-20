//! 言語ファイル (文言カタログ) の解析と解決 (docs/07 §17、docs/02 §4.1)。
//!
//! 形式は TOML のキー → 文字列 + `{name}` プレースホルダ。ネストしたテーブルは `.` でつないだ平らなキーにする。
//! `[meta]` は文言ではない。`{key:<command_id>}` は呼び出し側が渡す関数で解決済みキーマップのキー表記に置き換え、
//! 割当がなければ `ui.key.via_palette` の文言にする。`{{` は `{` のエスケープ。引数の位置指定は使わない。
//! キーが無ければキー名そのものを返す (空白や例外にしない)。

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

/// 言語ファイルを読めなかった。
#[derive(Debug, PartialEq, Eq)]
pub struct CatalogError(pub String);

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "言語ファイルを読めません: {}", self.0)
    }
}

impl std::error::Error for CatalogError {}

/// 同梱の日本語の言語ファイル (実行時のファイル読込はしない)。
const BUILTIN_JA: &str = include_str!("../../../locales/ja.toml");

/// 文言の表。
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    entries: BTreeMap<String, String>,
}

impl Catalog {
    /// 言語ファイルを読む。`[meta]` は含めない。
    pub fn parse(src: &str) -> Result<Catalog, CatalogError> {
        let table: toml::Table = src
            .parse()
            .map_err(|e: toml::de::Error| CatalogError(e.to_string()))?;
        let mut entries = BTreeMap::new();
        for (k, v) in &table {
            if k != "meta" {
                flatten(k, v, &mut entries)?;
            }
        }
        Ok(Catalog { entries })
    }

    /// 同梱の `ja`。同梱のファイルは CI で検査するので、読めないのは開発中の誤りだけ。
    pub fn builtin_ja() -> &'static Catalog {
        static JA: OnceLock<Catalog> = OnceLock::new();
        JA.get_or_init(|| Catalog::parse(BUILTIN_JA).expect("同梱の locales/ja.toml は読める"))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// すべてのキー (昇順)。
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// キーの文言を引数で埋める。`key_label` は `{key:<command_id>}` のコマンドのキー表記 (割当がなければ `None`)。
    /// キーが無ければキー名そのもの。引数が足りないプレースホルダは `{name}` のまま残す。
    pub fn render(
        &self,
        key: &str,
        args: &[(&str, &str)],
        key_label: &dyn Fn(&str) -> Option<String>,
    ) -> String {
        match self.get(key) {
            Some(template) => self.fill(template, args, key_label),
            None => key.to_owned(),
        }
    }

    fn fill(
        &self,
        template: &str,
        args: &[(&str, &str)],
        key_label: &dyn Fn(&str) -> Option<String>,
    ) -> String {
        let expanded = expand_conditionals(template, args);
        let template = expanded.as_str();
        let mut out = String::with_capacity(template.len() + 16);
        let mut rest = template;
        while let Some(i) = rest.find(['{', '}']) {
            out.push_str(&rest[..i]);
            let tail = &rest[i..];
            if let Some(t) = tail.strip_prefix("{{") {
                out.push('{');
                rest = t;
            } else if let Some(t) = tail.strip_prefix("}}") {
                out.push('}');
                rest = t;
            } else if tail.starts_with('{')
                && let Some(end) = tail.find('}')
            {
                let name = &tail[1..end];
                if let Some(cmd) = name.strip_prefix("key:") {
                    match key_label(cmd) {
                        Some(label) => out.push_str(&label),
                        None => out.push_str(
                            self.get("ui.key.via_palette")
                                .unwrap_or("ui.key.via_palette"),
                        ),
                    }
                } else if let Some((_, v)) = args.iter().find(|(n, _)| *n == name) {
                    out.push_str(v);
                } else {
                    out.push_str(&tail[..=end]);
                }
                rest = &tail[end + 1..];
            } else {
                // 対応のない括弧: そのまま出す
                out.push_str(&tail[..1]);
                rest = &tail[1..];
            }
        }
        out.push_str(rest);
        out
    }
}

/// 条件付きの文 〔 〕 (04 §15.2): 中の引数がすべて値を持つときだけ中身を出し、〔 〕 は表示しない。
/// 1 つでも引数が無い・空なら、〔 〕 ごと省く。入れ子は扱わない。
fn expand_conditionals(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('〔') {
        out.push_str(&rest[..start]);
        let after = &rest[start + '〔'.len_utf8()..];
        let Some(end) = after.find('〕') else {
            // 閉じていない: そのまま出す
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..end];
        if placeholder_names(inner).all(|n| args.iter().any(|(k, v)| *k == n && !v.is_empty())) {
            out.push_str(inner);
        }
        rest = &after[end + '〕'.len_utf8()..];
    }
    out.push_str(rest);
    out
}

/// 文字列の中の `{name}` の name (`{key:…}` と `{{` のエスケープは除く)。
fn placeholder_names(s: &str) -> impl Iterator<Item = &str> {
    let mut names = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        let tail = &rest[i..];
        if tail.starts_with("{{") {
            rest = &tail[2..];
            continue;
        }
        match tail.find('}') {
            Some(end) => {
                let name = &tail[1..end];
                if !name.starts_with("key:") {
                    names.push(name);
                }
                rest = &tail[end + 1..];
            }
            None => break,
        }
    }
    names.into_iter()
}

fn flatten(
    prefix: &str,
    v: &toml::Value,
    out: &mut BTreeMap<String, String>,
) -> Result<(), CatalogError> {
    match v {
        toml::Value::String(s) => {
            out.insert(prefix.to_owned(), s.clone());
            Ok(())
        }
        toml::Value::Table(t) => {
            for (k, v) in t {
                flatten(&format!("{prefix}.{k}"), v, out)?;
            }
            Ok(())
        }
        _ => Err(CatalogError(format!(
            "`{prefix}` は文字列でもテーブルでもない"
        ))),
    }
}

/// 複数形の接尾辞 (`.one` / `.other`) を除いた基底キー (キー集合の比較用、07 §17.2)。
pub fn base_key(key: &str) -> &str {
    key.strip_suffix(".one")
        .or_else(|| key.strip_suffix(".other"))
        .unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(src: &str) -> Catalog {
        Catalog::parse(src).unwrap()
    }

    fn no_keys(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn 平らなキーにして_meta_を除く() {
        let c = cat(
            "[meta]\nlanguage = \"ja\"\n[msg.save.failed]\ntext = \"a\"\nhint = \"b\"\n[info.save]\ndone = \"c\"\n[status]\nuntitled = \"無題-{n}\"\n",
        );
        assert_eq!(
            c.keys().collect::<Vec<_>>(),
            [
                "info.save.done",
                "msg.save.failed.hint",
                "msg.save.failed.text",
                "status.untitled"
            ]
        );
        assert!(!c.contains("meta.language"));
    }

    #[test]
    fn 引数を埋める() {
        let c = cat("[a]\nb = \"{path} を保存しました ({n} 件)\"\n");
        assert_eq!(
            c.render("a.b", &[("path", "x.txt"), ("n", "3")], &no_keys),
            "x.txt を保存しました (3 件)"
        );
        // 足りない引数はそのまま残す
        assert_eq!(
            c.render("a.b", &[("path", "x")], &no_keys),
            "x を保存しました ({n} 件)"
        );
    }

    #[test]
    fn エスケープと対応のない括弧() {
        let c = cat("[a]\nb = \"{{x}} {y} }\"\nc = \"{\"\n");
        assert_eq!(c.render("a.b", &[("y", "1")], &no_keys), "{x} 1 }");
        assert_eq!(c.render("a.c", &[], &no_keys), "{");
    }

    #[test]
    fn キー表記を解決し_割当がなければパレット() {
        let c = cat(
            "[ui.key]\nvia_palette = \"コマンドパレットから実行\"\n[a]\nb = \"{key:file.save} で保存\"\n",
        );
        let labels = |cmd: &str| (cmd == "file.save").then(|| "Ctrl+S".to_owned());
        assert_eq!(c.render("a.b", &[], &labels), "Ctrl+S で保存");
        assert_eq!(
            c.render("a.b", &[], &no_keys),
            "コマンドパレットから実行 で保存"
        );
    }

    #[test]
    fn キーが無ければキー名() {
        assert_eq!(Catalog::default().render("x.y", &[], &no_keys), "x.y");
    }

    #[test]
    fn 文字列でもテーブルでもない値はエラー() {
        assert!(Catalog::parse("a = 1\n").is_err());
        assert!(Catalog::parse("a = [\"x\"]\n").is_err());
        assert!(Catalog::parse("not toml [").is_err());
    }

    #[test]
    fn 基底キー() {
        assert_eq!(base_key("msg.search.count.other"), "msg.search.count");
        assert_eq!(base_key("msg.search.count.one"), "msg.search.count");
        assert_eq!(base_key("msg.save.failed.text"), "msg.save.failed.text");
    }

    #[test]
    fn 条件付きの文は引数があるときだけ出す() {
        let c = cat("[a]
b = \"{path} を開けません〔(一時ファイル {tmp})〕\"
");
        assert_eq!(
            c.render("a.b", &[("path", "x")], &no_keys),
            "x を開けません"
        );
        assert_eq!(
            c.render("a.b", &[("path", "x"), ("tmp", "")], &no_keys),
            "x を開けません",
            "空の値は省く"
        );
        assert_eq!(
            c.render("a.b", &[("path", "x"), ("tmp", "t.tmp")], &no_keys),
            "x を開けません(一時ファイル t.tmp)"
        );
        // 閉じていない 〔 はそのまま
        let d = cat("[a]
b = \"x〔y\"
");
        assert_eq!(d.render("a.b", &[], &no_keys), "x〔y");
    }
}
