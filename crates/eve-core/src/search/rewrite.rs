//! 検索語の書き換え (docs/03 §12.3.1 の「行の終端に一致する書き方」の表・「CRLF は 1 つの行の終端」・`\n` の糖衣)。
//!
//! 正規表現の検索語を `regex-syntax` で AST に解析し、行の終端 (CR・LF) に一致しうる書き方を表のとおりに書き換えて、
//! そのまま印字する (`\w`・`\p{..}` を範囲に展開しないので、コンパイルの費用が元の検索語に近い)。
//!
//! - CR に一致できるのは `\r\n` (CRLF の全体) と `\r(?mR:^)` (後ろが LF でない CR) だけにする。CRLF モードの `^` は CR と
//!   LF の間には一致しないので、CR の直後で `^` が成り立つのは後ろが LF でないときだけ。これで、どの一致も CRLF の CR と
//!   LF の間では終わらない (間で始まる候補は、書記素の境界の規則 (§12.1) で捨てる)。
//! - 行の終端を明示した書き方 (`\n`・`\r`・要素として `\n` / `\r` を書いたクラス) だけが行の終端に一致する。`(?s)` の `.` と、
//!   集合がすべての文字になるクラス (`[\s\S]` など) は「改行 1 つか、ほかの 1 文字」。それ以外のクラス (`[^\n]`・`\s`・
//!   `\P{L}`・範囲など) と `(?-R)` の `.` は、集合から行の終端を除く。
//! - 足すグループはすべて `(?:…)` で、捕獲グループの番号と名前を変えない (置換の `$1` / `${name}` の意味を保つ)。
//!
//! 書き換えた検索語を解析し直した HIR から、1 つの一致が含みうる行の終端のバイトの数の上限 k (CRLF は 2 と数える) と、
//! 一致の最大の長さを求める。フラグは `m` (複数行) と `R` (CRLF) を常に付ける (§12.3。`regex` の構築でも同じ値を渡す)。
//!
//! CP932 互換文字の同一視 (§12.4.1、`search.cp932_compat_fold`) も同じ段で掛ける: リテラルの互換文字は 2 文字のクラスに
//! (`〜` は `[〜～]`)、クラスは組の片方を含めばもう片方も要素に足す。否定は否定する前の集合に掛けてから否定する (`\P{..}` は
//! 相手を差し引く、`[^〜]` は中の `〜` に `～` を足してから否定する)。`(?i)` が大小に掛かるのと同じ形 (`regex-syntax` は
//! 要素ごとに畳み込んでから集合の演算と否定を行う) で、要素ごとに掛ける。`(?-u)` の範囲 (バイトの意味) には掛けない。

use regex_syntax::ast::{self, Ast};
use regex_syntax::hir::{self, Hir, HirKind};

use super::limits::SEARCH_HEAVY_REPEAT;
use super::matcher::QueryError;

/// AST から HIR への翻訳の誤り。
type TransError = hir::Error;

/// `pattern` を解析し直した AST (誤りの位置を示すために元の検索語を翻訳し直すとき。解析は通ったもの)。
fn ast_of(pattern: &str) -> Ast {
    ast::parse::Parser::new()
        .parse(pattern)
        .unwrap_or_else(|_| Ast::empty(span()))
}

/// 1 つの一致が含みうる行の終端のバイト (`\r` / `\n`) の数の上限 (§12.3.1 の k。CRLF は 2 と数える)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineSpan {
    /// 上限がある (0 は行をまたがない)。
    Bounded(usize),
    /// 上限が無い (`(?s).*`・`\n+` のように、行の終端に一致しうる書き方を上限の無い繰り返しに入れたもの)。
    Unbounded,
}

/// 書き換えの結果。
#[derive(Clone, Debug)]
pub(super) struct Rewritten {
    /// 書き換えた検索語 (`regex` に `multi_line(true)`・`crlf(true)` で渡す)。
    pub pattern: String,
    /// 1 つの一致が含みうる行の終端のバイトの数の上限。
    pub lines: LineSpan,
    /// 一致の最大の長さ (バイト。上限が無ければ `None`)。
    pub max_len: Option<usize>,
    /// 行の終端を明示した書き方 (`\n`・`\r`・要素として `\n` / `\r` を書いたクラス) を含む。
    pub explicit_line_end: bool,
    /// 行の経路では文書全体の意味にならないアサーション (`\A`・`\z`・`(?-m)` の `^` / `$`・`(?-R)` の `^` / `$`) を含む
    /// (k が 0 でも行の窓で探す、§12.3.1)。
    pub doc_anchor: bool,
    /// 重い検索語 (上限のある繰り返しの上限を、入れ子は掛け合わせて、`SEARCH_HEAVY_REPEAT` を超える。§12.3.1 の「重い検索語」)。
    pub heavy: bool,
    /// CP932 互換文字の同一視が効いている (書き換えで一致する文字が変わった。リテラルの互換文字か、組の片方だけを含む
    /// クラス。§12.4.1 の「検索バーの表示」の印)。
    pub compat_folded: bool,
    /// 同一視を切って書き換えた検索語 (`compat_folded` のときだけ)。すべて置換で、同一視で当たった一致を数えるのに使う
    /// (同一視を切った検索語で同じ範囲がちょうど一致しない一致、§12.4.1 の「置換」)。
    pub unfolded: Option<String>,
}

/// CP932 互換文字の同一視の組 (03 §12.4.1。03 §8.6 の互換文字の表から `¥` / `‾` の 2 行を除いた 8 組。JIS 系の写像の側,
/// CP932 の写像の側)。
pub(super) const COMPAT_PAIRS: [(char, char); 8] = [
    ('\u{301C}', '\u{FF5E}'), // 〜 / ～
    ('\u{2016}', '\u{2225}'), // ‖ / ∥
    ('\u{2212}', '\u{FF0D}'), // − / －
    ('\u{00A2}', '\u{FFE0}'), // ¢ / ￠
    ('\u{00A3}', '\u{FFE1}'), // £ / ￡
    ('\u{00AC}', '\u{FFE2}'), // ¬ / ￢
    ('\u{00A6}', '\u{FFE4}'), // ¦ / ￤
    ('\u{2014}', '\u{2015}'), // — / ―
];

/// 互換文字の組の相手 (組に無い文字は `None`)。
pub(super) fn compat_partner(c: char) -> Option<char> {
    COMPAT_PAIRS.iter().find_map(|&(a, b)| {
        if c == a {
            Some(b)
        } else if c == b {
            Some(a)
        } else {
            None
        }
    })
}

/// 検索語 `pattern` を書き換える (同一視なし)。単体試験の性質 (09 §3.2 の (g)。同一視を切ったとき) が使う。
#[cfg(test)]
pub(super) fn rewrite(
    pattern: &str,
    insensitive: bool,
    sugar: bool,
) -> Result<Rewritten, QueryError> {
    rewrite_with(pattern, insensitive, sugar, false)
}

/// 検索語 `pattern` を書き換える。`insensitive` は大小を区別しない (`(?i)` の初期値)、`sugar` は `\n` の糖衣
/// (`search.newline_sugar`)、`compat` は CP932 互換文字の同一視 (`search.cp932_compat_fold`)。構文の誤り (後方参照・
/// 先読み・不正なクラス名など) は `QueryError::Syntax`。
pub(super) fn rewrite_with(
    pattern: &str,
    insensitive: bool,
    sugar: bool,
    compat: bool,
) -> Result<Rewritten, QueryError> {
    let flags = Flags::initial(insensitive);
    let mut ast = ast::parse::Parser::new()
        .parse(pattern)
        .map_err(|e| syntax_error(&regex_syntax::Error::from(e)))?;
    // 翻訳の誤り (不正なクラス名・UTF-8 でない一致など) は、利用者の書いた検索語を翻訳し直して、その位置で示す
    // (`regex` と同じ文言)。誤りの無い検索語では元の検索語を翻訳しない (大小を区別しない `[\s\S]` などの畳み込みは
    // 1 回で数 ms かかるので、打鍵ごとの解析で重ねない)
    let original_error = |e: regex_syntax::Error| {
        let e = match flags.translator().translate(pattern, &ast_of(pattern)) {
            Err(orig) => regex_syntax::Error::from(orig),
            Ok(_) => e,
        };
        syntax_error(&e)
    };
    let mut w = Rewriter {
        pattern,
        sugar,
        explicit: false,
        compat,
        folded: false,
    };
    let mut f = flags;
    if let Err(e) = w.walk(&mut ast, &mut f, false) {
        return Err(original_error(regex_syntax::Error::from(e)));
    }
    let printed = ast.to_string();
    // 印字した検索語を解析し直して HIR を作る (k と最大の長さを求める。`regex` が受け取るのもこの文字列)
    let hir = ast::parse::Parser::new()
        .parse(&printed)
        .map_err(regex_syntax::Error::from)
        .and_then(|a| {
            flags
                .translator()
                .translate(&printed, &a)
                .map_err(regex_syntax::Error::from)
        })
        .map_err(original_error)?;
    // `\A`・`\z`・`(?-m)` の `^` / `$` (`Start` / `End`) と、`(?-R)` の `^` / `$` (CRLF を行の終端と見ない `StartLF` / `EndLF`)
    // は、行ごとに切り出すと行の終端 (CR・CRLF・文書の外) を文字列の端と見るので、文書全体の意味にならない
    let looks = hir.properties().look_set();
    let doc_anchor = [
        hir::Look::Start,
        hir::Look::End,
        hir::Look::StartLF,
        hir::Look::EndLF,
    ]
    .into_iter()
    .any(|l| looks.contains(l));
    // 同一視が効いたときだけ、同一視を切った検索語も作る (すべて置換で同一視で当たった一致を数える。書き換えは 0.1 ms 程度)
    let unfolded = if w.folded {
        Some(rewrite_with(pattern, insensitive, sugar, false)?.pattern)
    } else {
        None
    };
    Ok(Rewritten {
        lines: line_ends(&hir).map_or(LineSpan::Unbounded, LineSpan::Bounded),
        max_len: hir.properties().maximum_len(),
        pattern: printed,
        explicit_line_end: w.explicit,
        doc_anchor,
        heavy: max_repeat(&hir, 1) > SEARCH_HEAVY_REPEAT,
        compat_folded: w.folded,
        unfolded,
    })
}

/// 上限のある繰り返しの上限の最大 (入れ子は掛け合わせる。`outer` は外側の上限の積)。上限の無い繰り返し (`*`・`+`) は掛けない。
fn max_repeat(h: &Hir, outer: u64) -> u64 {
    match h.kind() {
        HirKind::Repetition(r) => {
            let here = r.max.map_or(outer, |m| outer.saturating_mul(u64::from(m)));
            here.max(max_repeat(&r.sub, here))
        }
        HirKind::Capture(c) => max_repeat(&c.sub, outer),
        HirKind::Concat(v) | HirKind::Alternation(v) => v
            .iter()
            .map(|x| max_repeat(x, outer))
            .max()
            .unwrap_or(outer),
        _ => outer,
    }
}

/// `regex` の構文エラーと同じ形の文言 (複数行) から、原因の行だけを取る。
pub(super) fn syntax_error(e: &dyn std::fmt::Display) -> QueryError {
    QueryError::Syntax(first_line(&e.to_string()))
}

/// `regex` の構文エラーの文言は複数行になる。最後の `error:` の行 (原因) だけを取る。
pub(super) fn first_line(msg: &str) -> String {
    msg.lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("error: "))
        .map_or_else(
            || msg.lines().next().unwrap_or("").to_owned(),
            str::to_owned,
        )
}

/// 書き換える位置で効いているフラグ (`(?flags)` と `(?flags:…)` を順にたどって求める)。
#[derive(Clone, Copy, Debug)]
struct Flags {
    case_insensitive: bool,
    multi_line: bool,
    dot_matches_new_line: bool,
    swap_greed: bool,
    unicode: bool,
    crlf: bool,
}

impl Flags {
    /// 検索語の先頭のフラグ。`m` と `R` は常に付ける (§12.3)。
    fn initial(insensitive: bool) -> Flags {
        Flags {
            case_insensitive: insensitive,
            multi_line: true,
            dot_matches_new_line: false,
            swap_greed: false,
            unicode: true,
            crlf: true,
        }
    }

    /// `(?flags)` を当てる (`-` より後ろは外す)。`x` は解析だけに効くので見ない。
    fn apply(&mut self, flags: &ast::Flags) {
        let mut on = true;
        for item in &flags.items {
            match item.kind {
                ast::FlagsItemKind::Negation => on = false,
                ast::FlagsItemKind::Flag(f) => match f {
                    ast::Flag::CaseInsensitive => self.case_insensitive = on,
                    ast::Flag::MultiLine => self.multi_line = on,
                    ast::Flag::DotMatchesNewLine => self.dot_matches_new_line = on,
                    ast::Flag::SwapGreed => self.swap_greed = on,
                    ast::Flag::Unicode => self.unicode = on,
                    ast::Flag::CRLF => self.crlf = on,
                    ast::Flag::IgnoreWhitespace => {}
                },
            }
        }
    }

    /// このフラグで AST を HIR にする翻訳器。
    fn translator(self) -> hir::translate::Translator {
        hir::translate::TranslatorBuilder::new()
            .case_insensitive(self.case_insensitive)
            .multi_line(self.multi_line)
            .dot_matches_new_line(self.dot_matches_new_line)
            .swap_greed(self.swap_greed)
            .unicode(self.unicode)
            .crlf(self.crlf)
            .utf8(true)
            .build()
    }
}

/// 文字の集合が行の終端を含むか・すべての文字か。
#[derive(Clone, Copy, Debug, Default)]
struct LineEndSet {
    cr: bool,
    lf: bool,
    all: bool,
}

struct Rewriter<'p> {
    /// 元の検索語 (翻訳の誤りの位置を示すためだけに使う)。
    pattern: &'p str,
    sugar: bool,
    explicit: bool,
    /// CP932 互換文字の同一視を掛ける。
    compat: bool,
    /// 同一視で一致する文字が変わった。
    folded: bool,
}

impl Rewriter<'_> {
    /// `ast` をその場で書き換える。`in_rep` は繰り返しの対象 (並びに置き換えるときはグループで包む)。
    fn walk(&mut self, ast: &mut Ast, flags: &mut Flags, in_rep: bool) -> Result<(), TransError> {
        match ast {
            Ast::Empty(_) | Ast::Assertion(_) => {}
            Ast::Flags(f) => flags.apply(&f.flags),
            Ast::Group(g) => {
                // `(?flags:…)` と `(…)` の中のフラグはグループの外へ持ち出さない
                let saved = *flags;
                if let ast::GroupKind::NonCapturing(f) = &g.kind {
                    flags.apply(f);
                }
                self.walk(&mut g.ast, flags, false)?;
                *flags = saved;
            }
            Ast::Repetition(r) => self.walk(&mut r.ast, flags, true)?,
            Ast::Alternation(a) => {
                for x in &mut a.asts {
                    self.walk(x, flags, false)?;
                }
            }
            Ast::Concat(c) => {
                let mut i = 0;
                while i < c.asts.len() {
                    // 続けて書いた `\r\n` は CRLF の改行だけに一致する (糖衣を掛けない)
                    if literal_is(&c.asts[i], '\r')
                        && c.asts.get(i + 1).is_some_and(|a| literal_is(a, '\n'))
                    {
                        self.explicit = true;
                        i += 2;
                        continue;
                    }
                    self.walk(&mut c.asts[i], flags, false)?;
                    i += 1;
                }
            }
            Ast::Literal(l) => match l.c {
                '\n' => {
                    self.explicit = true;
                    if self.sugar {
                        *ast = newline();
                    }
                }
                '\r' => {
                    self.explicit = true;
                    *ast = wrap(in_rep, concat(vec![cr(), cr_alone()]));
                }
                c => {
                    // 互換文字は 2 文字のクラスにする (`〜` は `[〜～]`。符号位置で書いた `\x{301C}` も)
                    if self.compat
                        && flags.unicode
                        && let Some(p) = compat_partner(c)
                    {
                        self.folded = true;
                        *ast = Ast::class_bracketed(bracketed(
                            false,
                            ast::ClassSet::union(union(vec![
                                ast::ClassSetItem::Literal((**l).clone()),
                                verbatim_item(p),
                            ])),
                        ));
                    }
                }
            },
            Ast::Dot(_) => {
                if flags.dot_matches_new_line {
                    *ast = any_char();
                } else if !flags.crlf {
                    // `(?-R)` の `.` は CR に一致する。行の終端を集合から除く
                    *ast = Ast::class_bracketed(bracketed(
                        true,
                        ast::ClassSet::union(union(vec![lit_item('\n'), lit_item('\r')])),
                    ));
                }
            }
            Ast::ClassPerl(_) | Ast::ClassUnicode(_) | Ast::ClassBracketed(_) => {
                if self.compat && flags.unicode {
                    self.fold_class(ast, *flags)?;
                }
                self.class(ast, *flags, in_rep)?;
            }
        }
        Ok(())
    }

    /// クラスに CP932 互換文字の同一視を掛ける (§12.4.1 の「正規表現」)。略記のクラス (`\d`・`\s`・`\w` とその否定) は組の
    /// 文字をどちらも含まない (どちらも含む) ので変えない。
    /// 書き換えても一致する文字が変わらない (`[〜～]`・`[〜\p{Sm}]` のように相手を既に含む) クラスは元のまま残し、印の
    /// 判定 (`folded`) にも数えない。
    fn fold_class(&mut self, ast: &mut Ast, flags: Flags) -> Result<(), TransError> {
        let orig = ast.clone();
        let changed = match ast {
            Ast::ClassBracketed(b) => self.fold_set(&mut b.kind, flags)?,
            Ast::ClassUnicode(u) => {
                let mut item = ast::ClassSetItem::Unicode((**u).clone());
                let changed = self.fold_item(&mut item, flags)?;
                if changed {
                    *ast = Ast::class_bracketed(bracketed(false, ast::ClassSet::Item(item)));
                }
                changed
            }
            _ => false,
        };
        if !changed {
            return Ok(());
        }
        let flags = Flags {
            case_insensitive: false,
            ..flags
        };
        let before = flags.translator().translate(self.pattern, &orig)?;
        let after = flags.translator().translate(self.pattern, ast)?;
        if before == after {
            *ast = orig;
        } else {
            self.folded = true;
        }
        Ok(())
    }

    /// 集合の要素ごとに同一視を掛ける (集合の演算は両辺に)。変えたら `true`。
    fn fold_set(&self, set: &mut ast::ClassSet, flags: Flags) -> Result<bool, TransError> {
        match set {
            ast::ClassSet::Item(item) => self.fold_item(item, flags),
            ast::ClassSet::BinaryOp(op) => {
                let l = self.fold_set(&mut op.lhs, flags)?;
                let r = self.fold_set(&mut op.rhs, flags)?;
                Ok(l || r)
            }
        }
    }

    /// クラスの要素 1 つに同一視を掛ける。否定でない要素は、組の片方だけを含めば相手を要素に足す。否定の要素 (`\P{..}`) は、
    /// 否定する前の集合に掛けてから否定する (= 相手を含まない側を差し引く)。入れ子のクラスは中の要素に掛ける (否定は後から
    /// 掛かるので、`[^〜]` は `[^〜～]` になる)。変えたら `true`。
    fn fold_item(&self, item: &mut ast::ClassSetItem, flags: Flags) -> Result<bool, TransError> {
        match item {
            ast::ClassSetItem::Literal(l) => {
                let Some(p) = compat_partner(l.c) else {
                    return Ok(false);
                };
                let own = ast::ClassSetItem::Literal(l.clone());
                *item = ast::ClassSetItem::Union(union(vec![own, verbatim_item(p)]));
                Ok(true)
            }
            ast::ClassSetItem::Range(r) => {
                let (lo, hi) = (r.start.c, r.end.c);
                let has = |c: char| lo <= c && c <= hi;
                let add = one_sided(&has, false);
                if add.is_empty() {
                    return Ok(false);
                }
                let mut items = vec![ast::ClassSetItem::Range(r.clone())];
                items.extend(add.into_iter().map(verbatim_item));
                *item = ast::ClassSetItem::Union(union(items));
                Ok(true)
            }
            ast::ClassSetItem::Unicode(u) => {
                let flags = Flags {
                    case_insensitive: false,
                    ..flags
                };
                let h = flags
                    .translator()
                    .translate(self.pattern, &Ast::class_unicode(u.clone()))?;
                let HirKind::Class(hir::Class::Unicode(c)) = h.kind() else {
                    return Ok(false);
                };
                let has = |ch: char| c.ranges().iter().any(|r| r.start() <= ch && ch <= r.end());
                let negated = u.is_negated();
                // 否定でなければ足す側 (集合に無い相手)、否定なら差し引く側 (集合にあり、相手が無いもの)
                let chars = one_sided(&has, negated);
                if chars.is_empty() {
                    return Ok(false);
                }
                let mut lits: Vec<ast::ClassSetItem> =
                    chars.into_iter().map(verbatim_item).collect();
                let own = ast::ClassSetItem::Unicode(u.clone());
                *item = if negated {
                    let rhs = if lits.len() == 1 {
                        lits.pop().unwrap_or(ast::ClassSetItem::Empty(span()))
                    } else {
                        ast::ClassSetItem::Union(union(lits))
                    };
                    ast::ClassSetItem::Bracketed(Box::new(bracketed(
                        false,
                        ast::ClassSet::BinaryOp(ast::ClassSetBinaryOp {
                            span: span(),
                            kind: ast::ClassSetBinaryOpKind::Difference,
                            lhs: Box::new(ast::ClassSet::Item(own)),
                            rhs: Box::new(ast::ClassSet::Item(rhs)),
                        }),
                    )))
                } else {
                    let mut items = vec![own];
                    items.extend(lits);
                    ast::ClassSetItem::Union(union(items))
                };
                Ok(true)
            }
            ast::ClassSetItem::Bracketed(b) => self.fold_set(&mut b.kind, flags),
            ast::ClassSetItem::Union(u) => {
                let mut changed = false;
                for i in &mut u.items {
                    changed |= self.fold_item(i, flags)?;
                }
                Ok(changed)
            }
            ast::ClassSetItem::Empty(_)
            | ast::ClassSetItem::Ascii(_)
            | ast::ClassSetItem::Perl(_) => Ok(false),
        }
    }

    /// 文字のクラスを書き換える (§12.3.1 の表のクラスの行)。
    fn class(&mut self, ast: &mut Ast, flags: Flags, in_rep: bool) -> Result<(), TransError> {
        let set = self.set_of(ast, flags)?;
        if !set.cr && !set.lf {
            return Ok(());
        }
        let (ex_cr, ex_lf) = match ast {
            Ast::ClassBracketed(b) if !b.negated => explicit_elements(&b.kind),
            _ => (false, false),
        };
        // 集合がすべての文字になるクラスは「明示したどの文字でも」(改行 1 つか、ほかの 1 文字)。要素に `\n` / `\r` を
        // 書いたもの (`[\n\s\S]`) は、書き換えは同じで、行の終端を明示した書き方に数える (08 §2.5 の M24 の (1))
        if set.all {
            if ex_cr || ex_lf {
                self.explicit = true;
            }
            *ast = any_char();
            return Ok(());
        }
        let item = class_item(ast);
        if !ex_cr && !ex_lf {
            // 否定・略記・Unicode のクラスと範囲: 行の終端を集合から除く (1 行の中で一致する)
            *ast = subtract(item, set.cr, set.lf);
            return Ok(());
        }
        self.explicit = true;
        let mut alts = Vec::new();
        if ex_lf && self.sugar {
            alts.push(concat(vec![cr(), lf()]));
        }
        if ex_cr || (ex_lf && self.sugar) {
            alts.push(concat(vec![cr(), cr_alone()]));
        }
        // 残りの 1 文字: 要素の `\r` を除き、ほかの要素 (`\s` など) が持ち込む行の終端を除く (要素の `\n` は残す)
        if let ast::ClassSetItem::Bracketed(b) = item {
            let mut rest = *b;
            if remove_cr_elements(&mut rest.kind) {
                let rest_set = self.set_of(&Ast::class_bracketed(rest.clone()), flags)?;
                let (cut_cr, cut_lf) = (rest_set.cr, rest_set.lf && !ex_lf);
                alts.push(if cut_cr || cut_lf {
                    subtract(ast::ClassSetItem::Bracketed(Box::new(rest)), cut_cr, cut_lf)
                } else {
                    Ast::class_bracketed(rest)
                });
            }
        }
        *ast = match alts.len() {
            1 => wrap(in_rep, alts.pop().unwrap_or_else(|| Ast::empty(span()))),
            _ => group(Ast::alternation(ast::Alternation {
                span: span(),
                asts: alts,
            })),
        };
        Ok(())
    }

    /// クラス `ast` の文字の集合 (フラグ `flags` のもとで)。大小の畳み込みはしない: 行の終端 (CR・LF) は畳み込みの
    /// 相手を持たないので、行の終端を含むかは畳み込みに依らない。すべての文字かも、畳み込まない集合がすべてなら畳み込んでも
    /// すべて (逆は、区別のある文字の片方だけを欠いた範囲のような作為的なクラスだけで、そのクラスは「集合から行の終端を
    /// 除く」側に入る)。大きなクラスの畳み込みは 1 回で数 ms かかるので、打鍵ごとの解析で行わない。
    fn set_of(&self, ast: &Ast, flags: Flags) -> Result<LineEndSet, TransError> {
        let flags = Flags {
            case_insensitive: false,
            ..flags
        };
        let h = flags.translator().translate(self.pattern, ast)?;
        Ok(match h.kind() {
            HirKind::Literal(l) => LineEndSet {
                cr: l.0.contains(&b'\r'),
                lf: l.0.contains(&b'\n'),
                all: false,
            },
            HirKind::Class(hir::Class::Unicode(c)) => {
                let has = |ch: char| c.ranges().iter().any(|r| r.start() <= ch && ch <= r.end());
                LineEndSet {
                    cr: has('\r'),
                    lf: has('\n'),
                    all: matches!(c.ranges(), [r] if r.start() == '\0' && r.end() == char::MAX),
                }
            }
            HirKind::Class(hir::Class::Bytes(c)) => {
                let has = |b: u8| c.ranges().iter().any(|r| r.start() <= b && b <= r.end());
                LineEndSet {
                    cr: has(b'\r'),
                    lf: has(b'\n'),
                    all: false,
                }
            }
            _ => LineEndSet::default(),
        })
    }
}

/// `ast` が文字 `c` の 1 文字のリテラルか。
fn literal_is(ast: &Ast, c: char) -> bool {
    matches!(ast, Ast::Literal(l) if l.c == c)
}

/// 否定でないクラスの一番外の要素に、`\r` / `\n` の 1 文字を書いたものがあるか (`[\n]`・`[ \t\n]`・`[\s\n]`・`[\x0A]`)。
/// 範囲 (`[\x00-\x1F]`)・入れ子のクラス (`[a[\n]]`)・集合の演算 (`[\n&&\s]`) の中のものは要素に数えない
/// (集合から行の終端を除く側に入る)。返すのは (`\r` を書いた, `\n` を書いた)。
fn explicit_elements(set: &ast::ClassSet) -> (bool, bool) {
    let items: &[ast::ClassSetItem] = match set {
        ast::ClassSet::Item(ast::ClassSetItem::Union(u)) => &u.items,
        ast::ClassSet::Item(item) => std::slice::from_ref(item),
        ast::ClassSet::BinaryOp(_) => &[],
    };
    let has = |c: char| {
        items
            .iter()
            .any(|i| matches!(i, ast::ClassSetItem::Literal(l) if l.c == c))
    };
    (has('\r'), has('\n'))
}

/// 一番外の要素から `\r` の 1 文字を除く。要素が残れば `true` (残らなければクラスごと要らない)。
fn remove_cr_elements(set: &mut ast::ClassSet) -> bool {
    match set {
        ast::ClassSet::Item(ast::ClassSetItem::Union(u)) => {
            u.items
                .retain(|i| !matches!(i, ast::ClassSetItem::Literal(l) if l.c == '\r'));
            !u.items.is_empty()
        }
        ast::ClassSet::Item(ast::ClassSetItem::Literal(l)) => l.c != '\r',
        _ => true,
    }
}

/// クラスの AST を、ほかのクラスの要素にできる形にする。
fn class_item(ast: &Ast) -> ast::ClassSetItem {
    match ast {
        Ast::ClassPerl(p) => ast::ClassSetItem::Perl((**p).clone()),
        Ast::ClassUnicode(u) => ast::ClassSetItem::Unicode((**u).clone()),
        Ast::ClassBracketed(b) => ast::ClassSetItem::Bracketed(b.clone()),
        _ => ast::ClassSetItem::Empty(span()),
    }
}

/// クラス `item` から行の終端を除いたクラス (`[item--[\n\r]]`)。
fn subtract(item: ast::ClassSetItem, cut_cr: bool, cut_lf: bool) -> Ast {
    let mut cut = Vec::new();
    if cut_lf {
        cut.push(lit_item('\n'));
    }
    if cut_cr {
        cut.push(lit_item('\r'));
    }
    let rhs = if cut.len() == 1 {
        cut.pop().unwrap_or(ast::ClassSetItem::Empty(span()))
    } else {
        ast::ClassSetItem::Bracketed(Box::new(bracketed(false, ast::ClassSet::union(union(cut)))))
    };
    Ast::class_bracketed(bracketed(
        false,
        ast::ClassSet::BinaryOp(ast::ClassSetBinaryOp {
            span: span(),
            kind: ast::ClassSetBinaryOpKind::Difference,
            lhs: Box::new(ast::ClassSet::Item(item)),
            rhs: Box::new(ast::ClassSet::Item(rhs)),
        }),
    ))
}

/// 改行 1 つ (`\n` の糖衣): `(?:\r\n|\n|\r(?mR:^))`。
fn newline() -> Ast {
    group(Ast::alternation(ast::Alternation {
        span: span(),
        asts: vec![
            concat(vec![cr(), lf()]),
            lf(),
            concat(vec![cr(), cr_alone()]),
        ],
    }))
}

/// 改行 1 つ (CRLF も 1 文字に数える) か、ほかの 1 文字: `(?:\r\n|\r(?mR:^)|[^\r])`。
fn any_char() -> Ast {
    group(Ast::alternation(ast::Alternation {
        span: span(),
        asts: vec![
            concat(vec![cr(), lf()]),
            concat(vec![cr(), cr_alone()]),
            Ast::class_bracketed(bracketed(true, ast::ClassSet::Item(lit_item('\r')))),
        ],
    }))
}

/// 「CR の後ろが LF でない」: `(?mR:^)` (CRLF モードの `^` は CR と LF の間に一致しない)。利用者の `(?-m)` などに左右されない。
fn cr_alone() -> Ast {
    let flag = |f| ast::FlagsItem {
        span: span(),
        kind: ast::FlagsItemKind::Flag(f),
    };
    Ast::group(ast::Group {
        span: span(),
        kind: ast::GroupKind::NonCapturing(ast::Flags {
            span: span(),
            items: vec![flag(ast::Flag::MultiLine), flag(ast::Flag::CRLF)],
        }),
        ast: Box::new(Ast::assertion(ast::Assertion {
            span: span(),
            kind: ast::AssertionKind::StartLine,
        })),
    })
}

fn cr() -> Ast {
    Ast::literal(special('\r'))
}

fn lf() -> Ast {
    Ast::literal(special('\n'))
}

fn special(c: char) -> ast::Literal {
    ast::Literal {
        span: span(),
        kind: ast::LiteralKind::Special(if c == '\r' {
            ast::SpecialLiteralKind::CarriageReturn
        } else {
            ast::SpecialLiteralKind::LineFeed
        }),
        c,
    }
}

fn lit_item(c: char) -> ast::ClassSetItem {
    ast::ClassSetItem::Literal(special(c))
}

/// 文字 `c` をそのまま書いたクラスの要素 (互換文字はどれも構文の記号でない)。
fn verbatim_item(c: char) -> ast::ClassSetItem {
    ast::ClassSetItem::Literal(ast::Literal {
        span: span(),
        kind: ast::LiteralKind::Verbatim,
        c,
    })
}

/// 組の片方だけを含む集合 `has` について、`remove` が偽なら含まない側 (足す文字)、真なら含む側 (差し引く文字) を返す。
fn one_sided(has: &dyn Fn(char) -> bool, remove: bool) -> Vec<char> {
    COMPAT_PAIRS
        .iter()
        .filter(|&&(a, b)| has(a) != has(b))
        .map(|&(a, b)| if has(a) == remove { a } else { b })
        .collect()
}

fn union(items: Vec<ast::ClassSetItem>) -> ast::ClassSetUnion {
    ast::ClassSetUnion {
        span: span(),
        items,
    }
}

fn bracketed(negated: bool, kind: ast::ClassSet) -> ast::ClassBracketed {
    ast::ClassBracketed {
        span: span(),
        negated,
        kind,
    }
}

fn concat(asts: Vec<Ast>) -> Ast {
    Ast::concat(ast::Concat { span: span(), asts })
}

/// 捕獲しないグループ `(?:…)`。
fn group(inner: Ast) -> Ast {
    Ast::group(ast::Group {
        span: span(),
        kind: ast::GroupKind::NonCapturing(ast::Flags {
            span: span(),
            items: Vec::new(),
        }),
        ast: Box::new(inner),
    })
}

/// 繰り返しの対象を並びに置き換えるときは `(?:…)` で包む (`\r+` は `(?:\r(?mR:^))+`)。
fn wrap(in_rep: bool, ast: Ast) -> Ast {
    if in_rep && matches!(ast, Ast::Concat(_)) {
        group(ast)
    } else {
        ast
    }
}

/// 足した節の位置 (印字には使われない。誤りの位置は解析し直した検索語で示す)。
fn span() -> ast::Span {
    ast::Span::splat(ast::Position::new(0, 1, 1))
}

/// HIR の 1 つの一致が含みうる行の終端のバイトの数の上限。上限が無ければ `None`。
fn line_ends(h: &Hir) -> Option<usize> {
    match h.kind() {
        HirKind::Empty | HirKind::Look(_) => Some(0),
        HirKind::Literal(l) => Some(l.0.iter().filter(|b| matches!(b, b'\r' | b'\n')).count()),
        HirKind::Class(hir::Class::Unicode(c)) => {
            let has = |ch: char| c.ranges().iter().any(|r| r.start() <= ch && ch <= r.end());
            Some(usize::from(has('\n') || has('\r')))
        }
        HirKind::Class(hir::Class::Bytes(c)) => {
            let has = |b: u8| c.ranges().iter().any(|r| r.start() <= b && b <= r.end());
            Some(usize::from(has(b'\n') || has(b'\r')))
        }
        HirKind::Repetition(r) => {
            let k = line_ends(&r.sub)?;
            if k == 0 {
                return Some(0);
            }
            let max = usize::try_from(r.max?).ok()?;
            k.checked_mul(max)
        }
        HirKind::Capture(c) => line_ends(&c.sub),
        HirKind::Concat(v) => v
            .iter()
            .try_fold(0usize, |a, h| a.checked_add(line_ends(h)?)),
        HirKind::Alternation(v) => v.iter().try_fold(0usize, |a, h| Some(a.max(line_ends(h)?))),
    }
}

#[cfg(test)]
mod tests;
