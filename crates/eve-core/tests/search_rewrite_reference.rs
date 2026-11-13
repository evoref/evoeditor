//! 書き換えた検索語を、03 §12.3.1 の表から直接組み立てた参照と突き合わせる (09 §3.2 の検索の拡張の行。M24 の (1))。
//!
//! 参照の方式: 文書の行の終端 (LF・CRLF・単独の CR) を、それぞれ `\n` + 私用文字 (U+E001 / U+E002 / U+E003) + `\n` の
//! 1 単位に置き換えた文書の上で、表の意味を直接書いた検索語 (参照) を `m` 付き・`R` なしで走らせる。行の終端の種類が私用文字で
//! 区別できるので、CRLF を CR と LF に分けて一致させる誤りや、行の終端を除くべきクラスが一致する誤りが、位置とキャプチャの
//! 違いとして出る。`(?s)` の扱いは `regex` 自身のフラグに任せる (`.` の参照は `(?:.[U].|[^\nU])` で、`s` のときだけ
//! 1 つ目の枝が単位に一致する)。参照にも同じ書き換えを掛ける差分の試験と違い、書き換えの誤りを見つけられる。
//!
//! 行をまたぐ検索は M24 の (4) まで開かないので、全文を 1 本の文字列として `regex` で走らせ、03 §12.1 の選び方 (重ならない
//! ものを左から・空の一致は採らない・両端が書記素境界) で選んだ列で比べる。proptest の件数は `PROPTEST_CASES` で増やせる。

use std::fmt::Write as _;

use eve_core::{CancelToken, Case, SearchQuery, build_replace_all};
use eve_text::{ByteOff, Text, TextRead, is_grapheme_boundary};
use proptest::prelude::*;

/// 部品の種類 (続けて書いた `\r\n` の判定と、量指定子の付け方に使う)。
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// `\r` (単独の CR)。
    Cr,
    /// `\n` (改行 1 つ)。
    Lf,
    /// 1 文字に一致するもの。
    Char,
    /// 幅の無いもの (アサーションとフラグ)。
    Zero,
}

/// 参照の文書の、行の終端の 1 単位 (`\n` + 私用文字 + `\n`) の私用文字。
const U: &str = r"[\x{E001}-\x{E003}]";
/// 参照の文書の、行の終端の 1 単位 (どの種類でも)。
const UNIT_ANY: &str = r"\n[\x{E001}-\x{E003}]\n";

/// 部品 (元の書き方, 参照 (糖衣 ON), 参照 (糖衣 OFF), 種類)。
fn atoms() -> Vec<(String, String, String, Kind)> {
    let ex = r"[\n\x{E001}-\x{E003}]";
    let cls = |c: &str| format!("[{c}--{ex}]");
    let all = format!(r"(?:{UNIT_ANY}|[^\n\x{{E001}}-\x{{E003}}])");
    let lf_on = format!("(?:{UNIT_ANY})");
    let lf_off = r"(?:\n\x{E001}\n)".to_owned();
    let cr = r"(?:\n\x{E003}\n)".to_owned();
    let same = |s: &str, k: Kind| (s.to_owned(), s.to_owned(), s.to_owned(), k);
    let mut v = vec![
        same("a", Kind::Char),
        same("b", Kind::Char),
        same("あ", Kind::Char),
        same("é", Kind::Char),
        same(" ", Kind::Char),
        same("\"", Kind::Char),
        same("1", Kind::Char),
        same(r"\t", Kind::Char),
        same("^", Kind::Zero),
        same("$", Kind::Zero),
        same(r"\A", Kind::Zero),
        same(r"\z", Kind::Zero),
        same(r"\b", Kind::Zero),
        same(r"\B", Kind::Zero),
        same("(?i)", Kind::Zero),
        same("(?s)", Kind::Zero),
        same("(?-s)", Kind::Zero),
        same("(?-m)", Kind::Zero),
        same("(?U)", Kind::Zero),
    ];
    let dot = format!(r"(?:.{U}.|[^\n\x{{E001}}-\x{{E003}}])");
    v.push((".".into(), dot.clone(), dot, Kind::Char));
    for p in [r"\n", r"\x0A", r"\x{A}", r"\u000A"] {
        v.push((p.into(), lf_on.clone(), lf_off.clone(), Kind::Lf));
    }
    for p in [r"\r", r"\x0D", r"\x{D}"] {
        v.push((p.into(), cr.clone(), cr.clone(), Kind::Cr));
    }
    // 否定・略記・Unicode のクラスと範囲、入れ子のクラス・集合の演算: 行の終端を集合から除く
    for c in [
        r"\s",
        r"\S",
        r"\w",
        r"\W",
        r"\d",
        r"\D",
        r"\p{L}",
        r"\P{L}",
        r"\pN",
        r"\p{Cc}",
        r#"[^"]"#,
        r"[^a]",
        r"[^\n]",
        r"[^\r\n]",
        r"[^\r]",
        r"[\x00-\x1F]",
        r"[[:space:]]",
        r"[a[\n]]",
        r"[\n&&\s]",
        r"[\w--a]",
        r"[\t-\r]",
        r"[\s--\n]",
        r"[^\x{0}-\x{0}]",
        r"[[^a]--b]",
    ] {
        v.push((c.into(), cls(c), cls(c), Kind::Char));
    }
    // すべての文字のクラス: 改行 1 つか、ほかの 1 文字
    for c in [
        r"[\s\S]",
        r"[\w\W]",
        r"[\d\D]",
        r"\p{Any}",
        r"[\n\s\S]",
        r"[\x00-\x{10FFFF}]",
    ] {
        v.push((c.into(), all.clone(), all.clone(), Kind::Char));
    }
    // 要素として `\n` / `\r` を書いたクラス
    let explicit = |orig: &str, on: &str, off: &str| {
        (orig.to_owned(), on.to_owned(), off.to_owned(), Kind::Char)
    };
    let s_rest = cls(r"\s");
    v.push(explicit(
        r"[\n ]",
        &format!("(?:{UNIT_ANY}|[ ])"),
        r"(?:\n\x{E001}\n|[ ])",
    ));
    v.push(explicit(
        r"[\n]",
        &format!("(?:{UNIT_ANY})"),
        r"(?:\n\x{E001}\n)",
    ));
    v.push(explicit(
        r"[\x0A]",
        &format!("(?:{UNIT_ANY})"),
        r"(?:\n\x{E001}\n)",
    ));
    v.push(explicit(
        r"[\r a]",
        r"(?:\n\x{E003}\n|[ a])",
        r"(?:\n\x{E003}\n|[ a])",
    ));
    v.push(explicit(
        r"[\x0D]",
        r"(?:\n\x{E003}\n)",
        r"(?:\n\x{E003}\n)",
    ));
    v.push(explicit(
        r"[\s\n]",
        &format!("(?:{UNIT_ANY}|{s_rest})"),
        &format!(r"(?:\n\x{{E001}}\n|{s_rest})"),
    ));
    v.push(explicit(
        r"[\r\s]",
        &format!(r"(?:\n\x{{E003}}\n|{s_rest})"),
        &format!(r"(?:\n\x{{E003}}\n|{s_rest})"),
    ));
    v.push(explicit(
        r"[\r\n]",
        &format!("(?:{UNIT_ANY})"),
        r"(?:\n[\x{E001}\x{E003}]\n)",
    ));
    v.push(explicit(
        r"[a-c\n]",
        &format!("(?:{UNIT_ANY}|[a-c])"),
        r"(?:\n\x{E001}\n|[a-c])",
    ));
    v.push(explicit(
        r"[\n[^a]]",
        &format!("(?:{UNIT_ANY}|{})", cls("[^a]")),
        &format!(r"(?:\n\x{{E001}}\n|{})", cls("[^a]")),
    ));
    v.push(explicit(
        r"[\r\P{L}]",
        &format!(r"(?:\n\x{{E003}}\n|{})", cls(r"\P{L}")),
        &format!(r"(?:\n\x{{E003}}\n|{})", cls(r"\P{L}")),
    ));
    v
}

/// 検索語の構文木 (部品の位置で葉を持つ)。
#[derive(Clone, Debug)]
enum Node {
    Leaf(usize),
    Concat(Vec<Node>),
    Alt(Box<Node>, Box<Node>),
    Cap(Box<Node>),
    Named(Box<Node>, u32),
    NonCap(Box<Node>),
    FlagGroup(Box<Node>, &'static str),
    Rep(Box<Node>, &'static str),
}

/// 構文木から、元の検索語と参照の検索語を組み立てる。
struct Render<'a> {
    atoms: &'a [(String, String, String, Kind)],
    sugar: bool,
}

impl Render<'_> {
    /// (元, 参照)
    fn render(&self, n: &Node) -> (String, String) {
        match n {
            Node::Leaf(i) => {
                let a = &self.atoms[*i];
                (
                    a.0.clone(),
                    if self.sugar { a.1.clone() } else { a.2.clone() },
                )
            }
            Node::Concat(v) => {
                let mut flat = Vec::new();
                flatten(v, &mut flat);
                let (mut o, mut r) = (String::new(), String::new());
                let mut i = 0;
                while i < flat.len() {
                    // 続けて書いた `\r\n` は CRLF の改行だけ
                    if let (Node::Leaf(a), Some(Node::Leaf(b))) = (flat[i], flat.get(i + 1))
                        && self.atoms[*a].3 == Kind::Cr
                        && self.atoms[*b].3 == Kind::Lf
                    {
                        o.push_str(&self.atoms[*a].0);
                        o.push_str(&self.atoms[*b].0);
                        r.push_str(r"(?:\n\x{E002}\n)");
                        i += 2;
                        continue;
                    }
                    let (co, cr) = self.render(flat[i]);
                    if matches!(flat[i], Node::Alt(..)) {
                        let _ = write!(o, "(?:{co})");
                        let _ = write!(r, "(?:{cr})");
                    } else {
                        o.push_str(&co);
                        r.push_str(&cr);
                    }
                    i += 1;
                }
                (o, r)
            }
            Node::Alt(a, b) => {
                let (ao, ar) = self.render(a);
                let (bo, br) = self.render(b);
                (format!("{ao}|{bo}"), format!("{ar}|{br}"))
            }
            Node::Cap(a) => {
                let (o, r) = self.render(a);
                (format!("({o})"), format!("({r})"))
            }
            Node::Named(a, n) => {
                let (o, r) = self.render(a);
                (format!("(?P<g{n}>{o})"), format!("(?P<g{n}>{r})"))
            }
            Node::NonCap(a) => {
                let (o, r) = self.render(a);
                (format!("(?:{o})"), format!("(?:{r})"))
            }
            Node::FlagGroup(a, f) => {
                let (o, r) = self.render(a);
                (format!("(?{f}:{o})"), format!("(?{f}:{r})"))
            }
            Node::Rep(a, q) => {
                if let Node::Leaf(i) = **a
                    && self.atoms[i].3 != Kind::Zero
                {
                    let (o, r) = self.render(a);
                    // 参照の葉は `(?:…)` か 1 文字かクラスなので、そのまま量指定子を付けられる
                    let r = if r.chars().count() == 1
                        || r.starts_with("(?:")
                        || r.starts_with('[') && !r.contains("--")
                    {
                        r
                    } else {
                        format!("(?:{r})")
                    };
                    return (format!("{o}{q}"), format!("{r}{q}"));
                }
                let (o, r) = self.render(a);
                (format!("(?:{o}){q}"), format!("(?:{r}){q}"))
            }
        }
    }
}

fn flatten<'a>(v: &'a [Node], out: &mut Vec<&'a Node>) {
    for n in v {
        if let Node::Concat(c) = n {
            flatten(c, out);
        } else {
            out.push(n);
        }
    }
}

fn node(n_atoms: usize) -> impl Strategy<Value = Node> {
    let leaf = (0..n_atoms).prop_map(Node::Leaf);
    leaf.prop_recursive(4, 24, 4, |inner| {
        prop_oneof![
            3 => prop::collection::vec(inner.clone(), 1..5).prop_map(Node::Concat),
            1 => (inner.clone(), inner.clone()).prop_map(|(a, b)| Node::Alt(Box::new(a), Box::new(b))),
            1 => inner.clone().prop_map(|a| Node::Cap(Box::new(a))),
            1 => (inner.clone(), 0u32..20).prop_map(|(a, n)| Node::Named(Box::new(a), n)),
            1 => inner.clone().prop_map(|a| Node::NonCap(Box::new(a))),
            1 => (inner.clone(), prop::sample::select(&["i", "s", "-m", "U", "-i", "is", "-s", "s-i"][..]))
                .prop_map(|(a, f)| Node::FlagGroup(Box::new(a), f)),
            2 => (inner, prop::sample::select(&["?", "*", "+", "{2}", "{1,3}", "{0,}", "*?", "+?", "{0}", "??"][..]))
                .prop_map(|(a, q)| Node::Rep(Box::new(a), q)),
        ]
    })
}

/// 文書の部品 (LF・CR・CRLF、結合文字、行の終端でない U+0085・U+2028 を含む)。
const DOC: &[&str] = &[
    "a", "b", "あ", " ", "\"", "\t", "\r", "\n", "\r\n", "é", "e", "\u{301}", "1", "\u{85}",
    "\u{2028}", "_",
];

fn doc() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(DOC), 0..20).prop_map(|v| v.concat())
}

/// 参照の文書と、参照の位置 → 元の位置 (単位の内側は `None`)。
fn ref_doc(doc: &str) -> (String, Vec<Option<usize>>) {
    let b = doc.as_bytes();
    let mut out = String::new();
    let mut map = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let (tok, len) = if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
            (Some('\u{E002}'), 2)
        } else if b[i] == b'\r' {
            (Some('\u{E003}'), 1)
        } else if b[i] == b'\n' {
            (Some('\u{E001}'), 1)
        } else {
            (None, doc[i..].chars().next().map_or(1, char::len_utf8))
        };
        match tok {
            Some(t) => {
                // 1 つ目の `\n` の前が元の位置。単位の内側は `None`。2 つ目の `\n` の後ろは次の要素が持つ
                map.push(Some(i));
                out.push('\n');
                for _ in 0..t.len_utf8() {
                    map.push(None);
                }
                out.push(t);
                map.push(None);
                out.push('\n');
            }
            None => {
                for k in 0..len {
                    map.push(if k == 0 { Some(i) } else { None });
                }
                out.push_str(&doc[i..i + len]);
            }
        }
        i += len;
    }
    map.push(Some(doc.len()));
    (out, map)
}

type Caps = Vec<Vec<Option<(usize, usize)>>>;

/// 全文を 1 本の文字列として、03 §12.1 の選び方で選んだ一致 (キャプチャつき)。
fn select_orig(doc: &str, re: &regex::Regex) -> Caps {
    let t = Text::from(doc);
    let next_char = |i: usize| i + doc[i..].chars().next().map_or(1, char::len_utf8);
    let mut out = Vec::new();
    let mut pos = 0;
    while pos <= doc.len() {
        let Some(c) = re.captures_at(doc, pos) else {
            break;
        };
        let Some(m) = c.get(0) else {
            break;
        };
        let (s, e) = (m.start(), m.end());
        if s == e {
            pos = next_char(e);
            continue;
        }
        if is_grapheme_boundary(&t, ByteOff(s)) && is_grapheme_boundary(&t, ByteOff(e)) {
            out.push(c.iter().map(|g| g.map(|g| (g.start(), g.end()))).collect());
            pos = e;
        } else {
            pos = next_char(s);
        }
    }
    out
}

/// 参照の文書の上で選んだ一致を、元の文書の位置にして返す (単位の内側に端が来たら `Err`)。
fn select_ref(
    doc: &str,
    rdoc: &str,
    map: &[Option<usize>],
    re: &regex::Regex,
) -> Result<Caps, String> {
    let t = Text::from(doc);
    let next_char = |i: usize| i + rdoc[i..].chars().next().map_or(1, char::len_utf8);
    let mp = |i: usize| {
        map.get(i)
            .copied()
            .flatten()
            .ok_or_else(|| format!("単位の内側の位置 {i}"))
    };
    let mut out = Vec::new();
    let mut pos = 0;
    while pos <= rdoc.len() {
        let Some(c) = re.captures_at(rdoc, pos) else {
            break;
        };
        let Some(m) = c.get(0) else {
            break;
        };
        let (s, e) = (m.start(), m.end());
        if s == e {
            pos = next_char(e);
            continue;
        }
        let (so, eo) = (mp(s)?, mp(e)?);
        if is_grapheme_boundary(&t, ByteOff(so)) && is_grapheme_boundary(&t, ByteOff(eo)) {
            let mut row = Vec::new();
            for g in c.iter() {
                row.push(match g {
                    Some(g) => Some((mp(g.start())?, mp(g.end())?)),
                    None => None,
                });
            }
            out.push(row);
            pos = e;
        } else {
            pos = next_char(s);
        }
    }
    Ok(out)
}

/// 書き換えた検索語 (検索語の解析の結果)。
fn rewritten(p: &str, insensitive: bool, sugar: bool) -> Option<String> {
    let q = SearchQuery::literal(p)
        .with_regex(true)
        .with_case(if insensitive {
            Case::Insensitive
        } else {
            Case::Sensitive
        })
        .with_newline_sugar(sugar);
    let plan = q.analyze().ok()??;
    plan.regex_pattern().map(str::to_owned)
}

fn build(p: &str, insensitive: bool, crlf: bool) -> Option<regex::Regex> {
    regex::RegexBuilder::new(p)
        .multi_line(true)
        .crlf(crlf)
        .case_insensitive(insensitive)
        .size_limit(1 << 26)
        .build()
        .ok()
}

/// 元の検索語 `orig` を書き換えて文書 `d` で選んだ一致と、参照 `refp` で選んだ一致を比べる。
fn compare(orig: &str, refp: &str, d: &str, ins: bool, sugar: bool) -> Result<(), String> {
    if build(orig, ins, true).is_none() {
        return Ok(()); // 元が不正 (`regex` が作れない) なら比べない
    }
    let w = rewritten(orig, ins, sugar).ok_or_else(|| format!("{orig:?}: 書き換えが失敗"))?;
    let re = build(&w, ins, true).ok_or_else(|| format!("{orig:?} → {w:?} を作れない"))?;
    let rre = build(refp, ins, false).ok_or_else(|| format!("参照 {refp:?} を作れない"))?;
    let (rd, map) = ref_doc(d);
    let got = select_orig(d, &re);
    let want =
        select_ref(d, &rd, &map, &rre).map_err(|e| format!("{orig:?} 参照 {refp:?} {d:?}: {e}"))?;
    if got != want {
        return Err(format!(
            "不一致: 検索語 {orig:?} (i={ins}, 糖衣={sugar}) 書き換え {w:?}\n 参照 {refp:?}\n 文書 {d:?}\n 実際 {got:?}\n 期待 {want:?}"
        ));
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(256),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    // 09 §3.2 の検索の拡張の行: 表の部品を組み合わせた検索語で、書き換えた検索語の一致とキャプチャが、表から直接組み立てた
    // 参照と同じ (CR・LF・CRLF を含む文書。大小の区別と糖衣の両方)
    #[test]
    fn 表から組み立てた参照と一致する(n in node(atoms().len()), d in doc(), ins: bool, sugar: bool) {
        let atoms = atoms();
        let r = Render { atoms: &atoms, sugar };
        let (o, rp) = r.render(&n);
        if let Err(e) = compare(&o, &rp, &d, ins, sugar) {
            return Err(TestCaseError::fail(e));
        }
    }
}

/// 突き合わせが誤りを見つけられること (参照をわざと違えると不一致になる)。
#[test]
fn 突き合わせは参照の違いを見つける() {
    // 糖衣の `\n` を LF だけとした参照
    assert!(compare(r"\n", r"(?:\n\x{E001}\n)", "a\r\nb", false, true).is_err());
    // 行の終端を除かない `[^\n]`
    assert!(compare(r"[^\n]", r"[^\n]", "a\rb", false, true).is_err());
    // `(?s)` の `.` を行の終端に一致させない参照
    assert!(
        compare(
            r"(?s)a.b",
            r"a[^\n\x{E001}-\x{E003}]b",
            "a\r\nb",
            false,
            true
        )
        .is_err()
    );
    assert!(
        compare(
            r"(?s)a.b",
            r"(?s)a(?:.[\x{E001}-\x{E003}].|[^\n\x{E001}-\x{E003}])b",
            "a\r\nb",
            false,
            true
        )
        .is_ok()
    );
}

/// 書き換えた検索語で、全文を 1 本の文字列として選んだ一致の範囲。
fn found(p: &str, doc: &str) -> Vec<(usize, usize)> {
    let w = rewritten(p, false, true).unwrap_or_else(|| panic!("{p:?}: 書き換えの誤り"));
    let re = build(&w, false, true).unwrap_or_else(|| panic!("{p:?} → {w:?}: 作れない"));
    select_orig(doc, &re)
        .into_iter()
        .filter_map(|c| c[0])
        .collect()
}

/// 固定の例: `(?x)` の空白を挟んだ `\r\n`・量指定子の付いた `\n` の前の `\r`・選択の枝をまたぐフラグ・入れ子のクラス。
/// 書き換えの結果と一致を期待値で書く (ゴールデン `search_rewrite.tsv` の表に無い細目)。
#[test]
fn 表に無い細目の書き換えと一致() {
    let nl = r"(?:\r\n|\n|\r(?mR:^))";
    let any = r"(?:\r\n|\r(?mR:^)|[^\r])";
    // `(?x)` の空白は構文の上で消えるので、続けて書いた `\r\n` と同じ (CRLF だけ)
    assert_eq!(rewritten(r"(?x)\r \n", false, true).unwrap(), r"(?x)\r\n");
    assert_eq!(found(r"(?x)\r \n", "a\nb\r\nc\rd"), [(3, 5)]);
    assert_eq!(
        found(r"(?x)\r \n", "a\nb\r\nc\rd"),
        found(r"\r\n", "a\nb\r\nc\rd")
    );
    // 量指定子の付いた `\n` の前の `\r` は単独の CR (続けて書いたものに数えない)。CRLF の文書では一致しない
    assert_eq!(
        rewritten(r"\r\n?", false, true).unwrap(),
        format!(r"\r(?mR:^){nl}?")
    );
    assert_eq!(found(r"\r\n?", "a\r\nb\r\nc"), []);
    assert_eq!(found(r"\r\n?", "a\rb"), [(1, 2)]);
    assert_eq!(
        found(r"\r\n?", "a\r\r\nb"),
        [(1, 4)],
        "単独の CR と、続く CRLF"
    );
    assert_eq!(found(r"\r\n+", "a\r\n\r\nb"), []);
    assert_eq!(found(r"(?:\r\n)+", "a\r\n\r\nb"), [(1, 5)]);
    assert_eq!(found(r"\n+", "a\r\n\r\nb"), [(1, 5)]);
    // 選択の枝をまたぐ `(?s)`: `regex` ではグループの終わりまで効くので、後ろの枝の `.` は改行 1 つにも一致する
    assert_eq!(
        rewritten(r"a(?s)|.", false, true).unwrap(),
        format!(r"a(?s)|{any}")
    );
    assert_eq!(
        found(r"a(?s)|.", "a\r\nb\rc"),
        [(0, 1), (1, 3), (3, 4), (4, 5), (5, 6)]
    );
    // 入れ子のクラスの中の `\n` は要素に数えないが、一番外の要素の `\n` は数える。残りの 1 文字の側は CR を除く
    assert_eq!(
        rewritten(r"[\n[^a]]", false, true).unwrap(),
        r"(?:\r\n|\r(?mR:^)|[[\n[^a]]--\r])"
    );
    assert_eq!(
        found(r"[\n[^a]]", "ab\r\nc\rd\n"),
        [(1, 2), (2, 4), (4, 5), (5, 6), (6, 7), (7, 8)]
    );
    // どの例も、表から組み立てた参照と同じ
    let atoms = atoms();
    let leaf = |s: &str| {
        atoms
            .iter()
            .position(|a| a.0 == s)
            .unwrap_or_else(|| panic!("{s}"))
    };
    let cases = [
        Node::Concat(vec![
            Node::Leaf(leaf("(?s)")),
            Node::Leaf(leaf(r"\r")),
            Node::Leaf(leaf(r"\n")),
        ]),
        Node::Concat(vec![
            Node::Leaf(leaf(r"\r")),
            Node::Rep(Box::new(Node::Leaf(leaf(r"\n"))), "?"),
        ]),
        Node::Alt(
            Box::new(Node::Concat(vec![
                Node::Leaf(leaf("a")),
                Node::Leaf(leaf("(?s)")),
            ])),
            Box::new(Node::Leaf(leaf("."))),
        ),
        Node::Leaf(leaf(r"[\n[^a]]")),
    ];
    for n in &cases {
        for sugar in [true, false] {
            let (o, rp) = Render {
                atoms: &atoms,
                sugar,
            }
            .render(n);
            for d in ["a\r\nb", "a\nb\rc", "\r\n\r\n", "x\ry\n", "a\r\r\nb"] {
                for ins in [false, true] {
                    compare(&o, &rp, d, ins, sugar).unwrap_or_else(|e| panic!("{e}"));
                }
            }
        }
    }
}

/// 置換の `$1` / `${name}` (k = 0 の検索語。LF だけの文書と CRLF の文書) が、元の検索語で 1 行ずつ展開した結果 (v0.3)
/// と同じ (書き換えは捕獲グループを足さない、03 §12.3.1)。`\A` / `\z` は v0.4 の M24 の (4) から文書の先頭 / 末尾の意味に
/// なった (振る舞いの変化) ので、ここでは比べず、下の `文書の端の記号の置換は文書の先頭と末尾だけ` で見る。
#[test]
fn 置換の展開は_v0_3_と同じ() {
    let pats = [
        (r"(\w+)\s(?P<x>\w+)", "$x-$1"),
        (r"([^\n])(?P<y>[^a])", "${y}$1"),
        (r#""([^"]*)""#, "<$1>"),
        (r"(\s+)|(\d)", "[$1|$2]"),
        (r"(?s)(a.b)", "{$1}"),
        (r"(?i)(K)(?<n>s)", "$n$1"),
        (r"^(\S+)$", "<$1>"),
        (r"\b(\w)", "_$1"),
        (r"(?-R)(.)", "($1)"),
    ];
    let docs = [
        "ab cd\nef gh\n",
        "ab cd\r\nef \"g h\" \r\n",
        "K s ks\rKS 1 2\n",
        "aab\r\naXb\n",
        "a b\t c\r\n",
    ];
    for (p, rep) in pats {
        for d in docs {
            let q = SearchQuery::literal(p)
                .with_regex(true)
                .with_case(Case::Sensitive);
            let t = Text::from(d);
            let Some((cs, _)) = build_replace_all(&t, &q, rep, &CancelToken::new()) else {
                panic!("{p}: 置換を作れない");
            };
            let mut t2 = Text::from(d);
            // 文書テキストの変更は本来 `Document::apply` の仕事。ここでは置換の ChangeSet の結果だけを見るので直接当てる
            #[allow(clippy::disallowed_methods)]
            t2.apply(&cs).unwrap_or_else(|e| panic!("{p}: {e:?}"));
            let got = t2.slice_to_string(eve_text::ByteRange::new(
                ByteOff(0),
                ByteOff(t2.len_bytes()),
            ));
            assert_eq!(got, replace_v0_3(p, rep, d), "{p} / {rep} / {d:?}");
        }
    }
}

/// `\A` / `\z` の置換は、文書の先頭 / 末尾だけで展開する (v0.3 は各行の先頭 / 末尾。03 §12.3.1 の「v0.3 からの振る舞いの変化」)。
#[test]
fn 文書の端の記号の置換は文書の先頭と末尾だけ() {
    for (p, d, want) in [
        (r"\A(.)", "ab cd\nef gh\n", "(a)b cd\nef gh\n"),
        (r"\A(.)", "ab\r\ncd", "(a)b\r\ncd"),
        (r"(.)\z", "ab\ncd", "ab\nc(d)"),
        (r"(.)\z", "ab\r\ncd\r\n", "ab\r\ncd\r\n"),
    ] {
        let q = SearchQuery::literal(p)
            .with_regex(true)
            .with_case(Case::Sensitive);
        let t = Text::from(d);
        let Some((cs, _)) = build_replace_all(&t, &q, "($1)", &CancelToken::new()) else {
            assert_eq!(d, want, "{p}: 一致が無ければ置き換えない");
            continue;
        };
        let mut t2 = Text::from(d);
        // 文書テキストの変更は本来 `Document::apply` の仕事。ここでは置換の ChangeSet の結果だけを見るので直接当てる
        #[allow(clippy::disallowed_methods)]
        t2.apply(&cs).unwrap_or_else(|e| panic!("{p}: {e:?}"));
        let got = t2.slice_to_string(eve_text::ByteRange::new(
            ByteOff(0),
            ByteOff(t2.len_bytes()),
        ));
        assert_eq!(got, want, "{p} / {d:?}");
    }
}

/// v0.3 の置換: 元の検索語 (`m`・`R` なし) で 1 行ずつ探して展開する。
fn replace_v0_3(p: &str, rep: &str, d: &str) -> String {
    let re = regex::Regex::new(p).unwrap_or_else(|e| panic!("{p}: {e}"));
    let mut want = String::new();
    let mut rest = d;
    loop {
        let (line, term, next) = match rest.find(['\r', '\n']) {
            Some(i) => {
                let tl = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
                (&rest[..i], &rest[i..i + tl], &rest[i + tl..])
            }
            None => (rest, "", ""),
        };
        let mut last = 0;
        let mut pos = 0;
        let lt = Text::from(line);
        while pos <= line.len() {
            let Some(c) = re.captures_at(line, pos) else {
                break;
            };
            let Some(m) = c.get(0) else {
                break;
            };
            if m.start() == m.end() {
                pos = m.end() + line[m.end()..].chars().next().map_or(1, char::len_utf8);
                continue;
            }
            if !(is_grapheme_boundary(&lt, ByteOff(m.start()))
                && is_grapheme_boundary(&lt, ByteOff(m.end())))
            {
                pos = m.start() + 1;
                continue;
            }
            want.push_str(&line[last..m.start()]);
            c.expand(rep, &mut want);
            last = m.end();
            pos = m.end();
        }
        want.push_str(&line[last..]);
        want.push_str(term);
        if term.is_empty() {
            break;
        }
        rest = next;
    }
    want
}

/// 印字に敏感な書き方 (`]` で始まるクラス・`-` の位置・集合の演算・Unicode のクラスの書き方・`(?x)`・単語境界の書き方
/// など): 書き換えた検索語が作れて、捕獲グループの数が同じで、CR と LF の無い文書で元と同じ一致とキャプチャになる。
#[test]
fn 印字に敏感な書き方も書き換えの前後で一致が同じ() {
    let pats = [
        r"[]a]",
        r"[^]a]",
        r"[a-]",
        r"[-a]",
        r"[\[\]]",
        r"[[:^alpha:]]",
        r"\p{Greek}",
        r"\p{sc=Greek}",
        r"\p{sc:Greek}",
        r"\pN",
        r"\x7F",
        r"\x{10FFFF}",
        r"\u{1F600}",
        r"\U0001F600",
        r"a{2}?",
        r"a{2,}?",
        r"(?<n>a)",
        r"(?P<n>a)\s",
        r"\<a",
        r"a\>",
        r"\b{start}a",
        r"a\b{end}",
        r"\b{start-half}a",
        r"[\n-\x{10FFFF}]",
        r"\v",
        r"\f",
        r"\a",
        r"(?x)[ a\n]",
        r"(?x) a \s # c",
        r"(?x)\ \n",
        r"(?x:\#\s)",
        r"[a&&[^b]]",
        r"[\w&&\p{Greek}]",
        r"[^\s\S]",
        r"[\d--[^0-4]]",
        r"(?i-u:k\s)",
        r"(?u)\W",
        r"[[a-c]\n[d-f]]",
        r"(?s)(?-s:.)\.",
        r"(?s:a|(?-s:.))",
        r"a(?s)|.",
        r"[\x{A}\x{D}]",
        r"\x{0A}",
        r"[\&\-\~]",
        r"[a~~b]",
        r"[\n--\n]",
    ];
    assert_eq!(pats.len(), 46);
    let docs = [
        "",
        "abc",
        "a b\tc\u{85}d\u{2028}",
        "αβγ ΑΒΓ 123 _",
        "😀]a-[",
        "k K \u{212A} s S \u{17F}",
        "\u{7}\u{c}\u{b}",
    ];
    let all = |re: &regex::Regex, d: &str| -> Vec<Vec<Option<std::ops::Range<usize>>>> {
        re.captures_iter(d)
            .map(|c| c.iter().map(|g| g.map(|g| g.range())).collect())
            .collect()
    };
    for p in pats {
        for ins in [false, true] {
            for sugar in [false, true] {
                // 元が作れないものは無い (どれも `regex` の構文)
                let orig = build(p, ins, true).unwrap_or_else(|| panic!("{p}: 元が作れない"));
                let w = rewritten(p, ins, sugar).unwrap_or_else(|| panic!("{p}: 書き換えの誤り"));
                let re = build(&w, ins, true).unwrap_or_else(|| panic!("{p} → {w}: 作れない"));
                assert_eq!(re.captures_len(), orig.captures_len(), "{p} → {w}");
                for d in docs {
                    assert_eq!(all(&orig, d), all(&re, d), "{p} → {w} / {d:?}");
                }
            }
        }
    }
}
