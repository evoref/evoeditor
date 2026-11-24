//! テキストオブジェクト (docs/03 §5.7): Selection の候補になる範囲 (単語・括弧の中・括弧を含む・行の中身・行の全体・文書の
//! 全体) を返す関数群。選択の拡張 (FR-309、§5.7 の SEL-1 / SEL-2) が使い、ダブル / トリプルクリック (FR-311、§5.8.2 の
//! CLK-2 / CLK-3) も単位 (`eve_text::unit_at` に窓 `WORD_WINDOW` を渡す) と行の全体 (`line_whole`) をここから使う。
//! Vim 風キーマップ (FR-607) の `select.word` などもこの関数群から作る (コマンドの名前はその版で決める)。
//!
//! どれも文書を変えない純粋な関数で、読む量に上限がある: 単語は 64 KiB の窓 (§4.5.1 の MCUR-2 (h))、括弧は前へ 1 MiB・対応を
//! 1 MiB (§5.7 の BRK-4) で、全 Range の括弧の走査の合計は `ScanBudget` で 8 MiB に抑える (SEL-5)。
//!
//! spec: 03 §5.7, 03 §5.8.2, 03 §15.4, 03 §13.3

use eve_text::{
    Bias, ByteOff, ByteRange, LineIdx, Range, Selection, TextRead, snap_to_grapheme, word_at,
};

use crate::view::bracket::{self, GOTO_SCAN_TOTAL_MAX_BYTES};
use crate::view::motion::indent_end_text;

/// 単語・単位を探す窓の上限 (バイト。03 §4.5.1 の MCUR-2 (h)、§5.8.2 の CLK-2。長大行で行の全体を読まない)。
pub(crate) const WORD_WINDOW: usize = 64 * 1024;

/// 1 回のコマンドで、全 Range の括弧の走査に使う読む量の残り (03 §5.7 の SEL-5。主の Range を先に探す)。使い切った後の
/// Range は括弧の候補を使わない。
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScanBudget {
    left: usize,
}

impl ScanBudget {
    /// `GOTO_SCAN_TOTAL_MAX_BYTES` (8 MiB) の予算。
    pub(crate) fn new() -> ScanBudget {
        ScanBudget::with_limit(GOTO_SCAN_TOTAL_MAX_BYTES)
    }

    /// 上限を指定した予算 (試験が小さな上限で使い切りを確かめる)。
    pub(crate) fn with_limit(limit: usize) -> ScanBudget {
        ScanBudget { left: limit }
    }

    fn spend(&mut self, read: usize) {
        self.left = self.left.saturating_sub(read);
    }
}

impl Default for ScanBudget {
    fn default() -> ScanBudget {
        ScanBudget::new()
    }
}

/// 1 回の `select.expand` の間に Range をまたいで使い回すもの (03 §5.7 の SEL-5 と細目 (i))。括弧の走査の予算と、前の Range
/// の単語 (続く Range がその中にあれば読み直さない。§4.5.1 の MCUR-2 (a) の続くカーソルと同じ) と、前の Range の最初の行の
/// 字下げの終わり。1 万個の Range が同じ長い単語・長い字下げの行にあっても、読む量は文書の大きさの程度に収まる。
#[derive(Clone, Debug, Default)]
pub(crate) struct ExpandCtx {
    budget: ScanBudget,
    word: Option<ByteRange>,
    indent: Option<(usize, usize)>,
}

impl ExpandCtx {
    /// 括弧の走査の予算が 8 MiB の文脈。
    pub(crate) fn new() -> ExpandCtx {
        ExpandCtx::default()
    }

    /// 括弧の走査の予算を指定した文脈 (試験が予算の配り方を確かめる)。
    #[cfg(test)]
    pub(crate) fn with_budget(limit: usize) -> ExpandCtx {
        ExpandCtx {
            budget: ScanBudget::with_limit(limit),
            ..ExpandCtx::default()
        }
    }
}

/// Range が触れる行 `(最初, 最後)` (03 §15.4 の取り方: 前の行から始まって行頭で終わる選択は、その最後の行を含めない)。
pub(crate) fn touched_lines(t: &dyn TextRead, r: &Range) -> (usize, usize) {
    let first = t.byte_to_line(r.start()).0;
    let mut last = t.byte_to_line(r.end()).0;
    if !r.is_empty() && last > first && t.line_to_byte(LineIdx(last)) == r.end() {
        last -= 1;
    }
    (first, last)
}

/// 行 `first..=last` の全体: `first` の行頭から `last` の区切りの後ろまで (最後の行に区切りが無ければ文書の末尾まで)。
pub(crate) fn lines_whole(t: &dyn TextRead, first: usize, last: usize) -> ByteRange {
    let start = t.line_to_byte(LineIdx(first));
    let end = if last + 1 < t.len_lines() {
        t.line_to_byte(LineIdx(last + 1))
    } else {
        ByteOff(t.len_bytes())
    };
    ByteRange::new(start, end)
}

/// 行 `line` の全体 (03 §5.8.2 の CLK-3 のトリプルクリックの単位。§5.7 の SEL-1 の行の全体と同じ)。
pub(crate) fn line_whole(t: &dyn TextRead, line: usize) -> ByteRange {
    lines_whole(t, line, line)
}

/// Range が **行の全体** (空でなく、始まりが行頭で、終わりが次の行の行頭か文書の末尾。03 §5.7 の SEL-1)。
fn is_whole_lines(t: &dyn TextRead, r: &Range) -> bool {
    let at_line_start = |p: ByteOff| t.line_to_byte(t.byte_to_line(p)) == p;
    !r.is_empty()
        && at_line_start(r.start())
        && (r.end().0 == t.len_bytes() || at_line_start(r.end()))
}

/// `select.line` の 1 つの Range (03 §5.7 の SEL-1)。行の全体でなければ、触れる行の全体を前向きに選ぶ。行の全体なら次の 1 行を
/// 足す (向きは前向き)。文書の末尾まで選んでいれば変えない。区切りで終わる文書の最後の空の行では行の全体が空なので変わらない。
pub(crate) fn select_line(t: &dyn TextRead, r: Range) -> Range {
    if is_whole_lines(t, &r) {
        if r.end().0 >= t.len_bytes() {
            return r;
        }
        let next = line_whole(t, t.byte_to_line(r.end()).0);
        return Range::new(r.start(), next.end);
    }
    let w = lines_of(t, &r);
    Range::new(w.start, w.end)
}

/// SEL-2 の (1) **単語**: Range を含む §13.3 の単位 (空の Range は 03 §4.5.1 の MCUR-2 (a) の単語。空白のラン・改行は単語に
/// しない)。単語が Range を含まなければ `None`。
///
/// 前の Range の単語 (`last`) が Range の始まりを含めば、それを使う (始まりの直後の書記素はその単語の中なので、探しても同じ
/// 単語になる)。見つけた単語は `last` に覚える。
pub(crate) fn word(t: &dyn TextRead, r: &Range, last: &mut Option<ByteRange>) -> Option<ByteRange> {
    let w = match *last {
        Some(l) if l.start <= r.start() && r.start() < l.end => l,
        _ => {
            let w = word_at(t, r.start(), WORD_WINDOW)?;
            *last = Some(w);
            w
        }
    };
    (w.start <= r.start() && w.end >= r.end()).then_some(w)
}

/// SEL-2 の (2) **括弧の中** と (3) **括弧を含む** 範囲 `(中, 含む)`。Range を囲む対応のある括弧の組のうち最も内側のもの
/// (`bracket::enclosing_pair`)。括弧の書記素の全体を含む・除く (結合文字の付いた括弧は書記素の終わりまでを括弧とする)。
/// 読んだ量を `budget` から引き、使い切っていれば探さない。
pub(crate) fn brackets(
    t: &dyn TextRead,
    r: &Range,
    budget: &mut ScanBudget,
) -> Option<(ByteRange, ByteRange)> {
    if budget.left == 0 {
        return None;
    }
    let (pair, read) = bracket::enclosing_pair(t, ByteRange::new(r.start(), r.end()), budget.left);
    budget.spend(read);
    let (o, c) = pair?;
    let inner_start = snap_to_grapheme(t, o.end, Bias::After);
    let inner_end = snap_to_grapheme(t, c.start, Bias::Before).max(inner_start);
    let outer = ByteRange::new(
        snap_to_grapheme(t, o.start, Bias::Before),
        snap_to_grapheme(t, c.end, Bias::After),
    );
    Some((ByteRange::new(inner_start, inner_end), outer))
}

/// SEL-2 の (4) **行の中身**: Range の最初の行の字下げの後ろから、最後の行 (03 §15.4 の取り方) の区切りの前まで。字下げは
/// 行頭の空白とタブだけを読み、`(行, 字下げの終わり)` を `indent` に覚えて、同じ行の Range では読み直さない。
pub(crate) fn line_content(
    t: &dyn TextRead,
    r: &Range,
    indent: &mut Option<(usize, usize)>,
) -> ByteRange {
    let (first, last) = touched_lines(t, r);
    let start = match *indent {
        Some((line, ie)) if line == first => ie,
        _ => {
            let ls = t.line_to_byte(LineIdx(first)).0;
            let ie = indent_end_text(t, ls, t.line_end_byte(LineIdx(first)).0);
            *indent = Some((first, ie));
            ie
        }
    };
    let end = t.line_end_byte(LineIdx(last)).0.max(start);
    ByteRange::new(ByteOff(start), ByteOff(end))
}

/// SEL-2 の (5) **行の全体**: SEL-1 の行の全体 (Range が触れる行の全体)。
pub(crate) fn lines_of(t: &dyn TextRead, r: &Range) -> ByteRange {
    let (first, last) = touched_lines(t, r);
    lines_whole(t, first, last)
}

/// `select.expand` の 1 つの Range (03 §5.7 の SEL-2)。候補 (1) 単語 (2) 括弧の中 (3) 括弧を含む (4) 行の中身 (5) 行の全体
/// (6) 文書の全体 のうち、Range を真に含む (等しくない) 最小のもの (長さの短い順。同じ長さなら番号の小さい順) に広げる。
/// 向きは保ち、空の Range からは前向き。どの候補も Range を真に含まなければ変えない。
pub(crate) fn expand(t: &dyn TextRead, r: Range, ctx: &mut ExpandCtx) -> Range {
    let (s, e) = (r.start(), r.end());
    let mut best: Option<ByteRange> = None;
    let mut consider = |c: ByteRange| {
        let contains = c.start <= s && c.end >= e && (c.start, c.end) != (s, e);
        if contains && best.is_none_or(|b| c.end.0 - c.start.0 < b.end.0 - b.start.0) {
            best = Some(c);
        }
    };
    if let Some(w) = word(t, &r, &mut ctx.word) {
        consider(w);
    }
    if let Some((inner, outer)) = brackets(t, &r, &mut ctx.budget) {
        consider(inner);
        consider(outer);
    }
    consider(line_content(t, &r, &mut ctx.indent));
    consider(lines_of(t, &r));
    consider(ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())));
    match best {
        None => r,
        Some(c) if r.is_forward() => Range::new(c.start, c.end),
        Some(c) => Range::new(c.end, c.start),
    }
}

/// `select.expand` の全 Range (03 §5.7 の SEL-2 / SEL-5)。主の Range を先に、残りを文書の順に広げる (括弧の走査の予算を主の
/// Range が先に使う)。結果は元の Range の順で、併合はしない (併合は Selection を置くときの `normalized()`)。
pub(crate) fn expand_all(t: &dyn TextRead, sel: &Selection, ctx: &mut ExpandCtx) -> Vec<Range> {
    let primary = sel.primary_index();
    let mut out = sel.ranges().to_vec();
    let order = std::iter::once(primary).chain((0..out.len()).filter(|&i| i != primary));
    for i in order {
        out[i] = expand(t, out[i], ctx);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_text::Text;

    /// 印 `«` `»` (前向き) / `‹` `›` (後ろ向き) / `|` (空) の Range と本文。
    fn parse(marked: &str) -> (String, Range) {
        let mut text = String::new();
        let (mut a, mut h) = (None, None);
        for c in marked.chars() {
            let p = ByteOff(text.len());
            match c {
                '|' => (a, h) = (Some(p), Some(p)),
                '«' => a = Some(p),
                '»' => h = Some(p),
                '‹' => h = Some(p),
                '›' => a = Some(p),
                c => text.push(c),
            }
        }
        (
            text,
            Range::new(a.expect("anchor の印"), h.expect("head の印")),
        )
    }

    fn render(text: &str, r: Range) -> String {
        let (s, e) = (r.start().0, r.end().0);
        if r.is_empty() {
            return format!("{}|{}", &text[..s], &text[s..]);
        }
        let (o, c) = if r.is_forward() {
            ('«', '»')
        } else {
            ('‹', '›')
        };
        format!("{}{o}{}{c}{}", &text[..s], &text[s..e], &text[e..])
    }

    fn line(marked: &str) -> String {
        let (text, r) = parse(marked);
        let t = Text::from(text.as_str());
        render(&text, select_line(&t, r))
    }

    // spec: 03 §5.7
    #[test]
    fn 行の全体の判定と次の行の足し方() {
        assert_eq!(line("a|b\ncd\n"), "«ab\n»cd\n");
        assert_eq!(line("«ab\n»cd\n"), "«ab\ncd\n»");
        assert_eq!(line("«ab\ncd\n»"), "«ab\ncd\n»", "文書の末尾まで");
        assert_eq!(line("ab\ncd|"), "ab\n«cd»", "区切りの無い最後の行");
        assert_eq!(line("ab\n|"), "ab\n|", "区切りで終わる文書の最後の空の行");
        assert_eq!(line("|"), "|", "空の文書");
        // 前の行から始まって行頭で終わる選択は、その最後の行を含めない (03 §15.4) が、行の全体なので次の行を足す
        assert_eq!(line("a«b\n»cd\n"), "«ab\n»cd\n");
        assert_eq!(
            line("‹ab\n›cd\n"),
            "«ab\ncd\n»",
            "後ろ向きの行の全体も前向きに足す"
        );
        assert_eq!(line("ab\r\n|cd\r\nef"), "ab\r\n«cd\r\n»ef", "CRLF");
        assert_eq!(line("ab\r|cd\ref"), "ab\r«cd\r»ef", "CR");
    }

    fn grow(marked: &str) -> String {
        let (text, r) = parse(marked);
        let t = Text::from(text.as_str());
        render(&text, expand(&t, r, &mut ExpandCtx::new()))
    }

    // spec: 03 §5.7
    #[test]
    fn 候補のうち真に含む最小のものに広げる() {
        // 03 §15.9.1 の `foo(bar baz)` の各位置
        assert_eq!(grow("x = foo(b|ar baz);"), "x = foo(«bar» baz);");
        assert_eq!(grow("x = foo(«bar» baz);"), "x = foo(«bar baz»);");
        assert_eq!(grow("x = foo(«bar baz»);"), "x = foo«(bar baz)»;");
        assert_eq!(grow("x = foo«(bar baz)»;"), "«x = foo(bar baz);»");
        assert_eq!(
            grow("«x = foo(bar baz);»"),
            "«x = foo(bar baz);»",
            "文書の全体"
        );
        assert_eq!(grow("x = foo(bar |baz);"), "x = foo(bar «baz»);");
        assert_eq!(
            grow("x = foo(bar| baz);"),
            "x = foo(«bar» baz);",
            "直後が空白なら直前の単語"
        );
        assert_eq!(
            grow("x = foo(bar |  baz);"),
            "x = foo(«bar   baz»);",
            "空白の上は単語が無い"
        );
        assert_eq!(grow("x = |foo(bar);"), "x = «foo»(bar);");
        // 直後の記号は 1 書記素の単語 (03 §4.5.1 の MCUR-2 (a))
        assert_eq!(grow("x = foo|(bar);"), "x = foo«(»bar);");
        assert_eq!(grow("x = foo(|);"), "x = foo(«)»;");
        // 閉じ括弧を含む Range も、その組に囲まれる (閉じ括弧の終わりが Range の終わり以降。SEL-2)
        assert_eq!(grow("x = foo(«)»;"), "x = foo«()»;");
        assert_eq!(grow("x = foo«()»;"), "«x = foo();»");
        assert_eq!(grow("foo(bar baz«)»"), "foo«(bar baz)»");
        assert_eq!(grow("「a«」»"), "«「a」»", "全角の閉じ括弧");
        // 開き括弧を含む Range も、その組に囲まれる (開き括弧の始まりが Range の始まり以前。SEL-2)
        assert_eq!(grow("foo«(»bar)"), "foo«(bar)»");
        assert_eq!(grow("«(ba»r)"), "«(bar)»");
        assert_eq!(grow("x«「」»"), "«x「」»", "組の全体からは外へ");
        assert_eq!(grow("(«(»a))"), "(«(a)»)", "内側の (");
        assert_eq!(grow("«(»(a))"), "«((a))»", "外側の (");
    }

    // spec: 03 §5.7
    #[test]
    fn 行の中身と行の全体と複数行の括弧() {
        assert_eq!(grow("  «ab» cd\nef\n"), "  «ab cd»\nef\n");
        assert_eq!(grow("  «ab cd»\nef\n"), "«  ab cd\n»ef\n");
        assert_eq!(grow("«  ab cd\n»ef\n"), "«  ab cd\nef\n»");
        assert_eq!(grow("  |  \nef"), "«    \n»ef", "空白だけの行は中身が空");
        assert_eq!(grow("f {\n    «a»;\n}\n"), "f {\n    «a;»\n}\n", "行の中身");
        assert_eq!(grow("f {\n    «a;»\n}\n"), "f {\n«    a;\n»}\n");
        assert_eq!(
            grow("f {\n«    a;\n»}\n"),
            "f {«\n    a;\n»}\n",
            "複数行の括弧の中"
        );
        assert_eq!(grow("f {«\n    a;\n»}\n"), "f «{\n    a;\n}»\n");
        assert_eq!(grow("f «{\n    a;\n}»\n"), "«f {\n    a;\n}»\n", "行の中身");
    }

    // spec: 03 §5.7
    #[test]
    fn 向きを保ち_空の範囲からは前向き() {
        assert_eq!(grow("foo(‹bar› baz)"), "foo(‹bar baz›)");
        assert_eq!(grow("foo(b|ar)"), "foo(«bar»)");
    }

    // spec: 03 §5.7
    #[test]
    fn 括弧の組と文字列の扱い() {
        // ほかの組は数えない (BRK-3)。交差する組は、近い開き括弧の組から候補にする
        assert_eq!(grow("「a(|b」)"), "「a(«b»」)");
        assert_eq!(grow("「a(«b»」)"), "「a(«b」»)");
        assert_eq!(grow("「a(«b」»)"), "「a«(b」)»");
        assert_eq!(grow("「a«(b」)»"), "«「a(b」)»");
        // 対応しない括弧は候補にしない (外側の組を使う)
        assert_eq!(grow("[a ( b|c ]"), "[a ( «bc» ]");
        assert_eq!(grow("[a ( «bc» ]"), "[«a ( bc »]");
        // 文字列 (引用符の中) は候補にしない (SEL-6)
        assert_eq!(grow("f(\"a «b» c\")"), "f(«\"a b c\"»)");
        // 全角の括弧
        assert_eq!(grow("「あ|い」"), "「«あい»」");
        assert_eq!(grow("「«あい»」"), "«「あい」»");
        // 入れ子の内側から外側へ
        assert_eq!(grow("((a) «b»)"), "(«(a) b»)");
        assert_eq!(grow("(«(a) b»)"), "«((a) b)»");
    }

    // spec: 03 §5.7
    #[test]
    fn 結合文字の付いた括弧は書記素の全体を括弧とする() {
        assert_eq!(grow("(\u{301}a|b)"), "(\u{301}«ab»)");
        assert_eq!(grow("(\u{301}«ab»)"), "«(\u{301}ab)»");
    }

    // spec: 03 §5.7
    #[test]
    fn 予算を使い切ると括弧の候補を使わない() {
        let t = Text::from("x (ab cd) y");
        let r = Range::new(ByteOff(3), ByteOff(8));
        assert_eq!(
            expand(&t, r, &mut ExpandCtx::with_budget(0)),
            Range::new(ByteOff(0), ByteOff(11)),
            "括弧を探さないので行の中身"
        );
        let mut ctx = ExpandCtx::with_budget(1 << 20);
        assert_eq!(
            expand(&t, r, &mut ctx),
            Range::new(ByteOff(2), ByteOff(9)),
            "括弧を含む"
        );
        assert!(ctx.budget.left < 1 << 20, "読んだ量を引く");
    }

    /// 全 Range を広げた様子 (併合の前。主の番号は `primary`)。
    fn grow_all(
        text: &str,
        ranges: &[(usize, usize)],
        primary: usize,
        budget: usize,
    ) -> Vec<String> {
        let t = Text::from(text);
        let rs = ranges
            .iter()
            .map(|&(a, h)| Range::new(ByteOff(a), ByteOff(h)))
            .collect();
        let sel = Selection::new(rs, primary).expect("Selection");
        expand_all(&t, &sel, &mut ExpandCtx::with_budget(budget))
            .into_iter()
            .map(|r| text[r.start().0..r.end().0].to_owned())
            .collect()
    }

    // spec: 03 §5.7
    #[test]
    fn 括弧の予算は主の_range_が先に使い_残りを文書の順に配る() {
        let text = "(aa) (bb) (cc)";
        let ranges = [(1, 3), (6, 8), (11, 13)];
        // 予算が十分なら、どの Range も括弧の中 (= 単語と同じなので括弧を含む範囲)
        assert_eq!(
            grow_all(text, &ranges, 0, 1 << 20),
            ["(aa)", "(bb)", "(cc)"]
        );
        // 主が文書の後ろ (cc)。主は前へ 11 バイト・対応を 3 バイト読み、予算 14 を使い切る。残りは括弧の候補を使わず行の中身
        assert_eq!(grow_all(text, &ranges, 2, 14), [text, text, "(cc)"]);
        // 主が文書の先頭 (aa)。主は前へ 1 バイト・対応を 13 バイト読む。続く (bb) は使い切った後
        assert_eq!(grow_all(text, &ranges, 0, 14), ["(aa)", text, text]);
        // 予算が主のぶんを超えて残れば、次の Range (文書の順) が使う。残りが前へ読んだ量 (1) 以下なら対応を探さない
        assert_eq!(grow_all(text, &ranges, 2, 15), [text, text, "(cc)"]);
        assert_eq!(grow_all(text, &ranges, 2, 16), ["(aa)", text, "(cc)"]);
    }

    /// 読む量を数える `TextRead` (チャンクとして渡した量と、文字列に写した量。03 §5.7 の細目 (i) の確かめ)。
    struct Counting<'a> {
        inner: &'a Text,
        read: std::cell::Cell<usize>,
    }

    impl Counting<'_> {
        fn add(&self, n: usize) {
            self.read.set(self.read.get() + n);
        }
    }

    impl TextRead for Counting<'_> {
        fn len_bytes(&self) -> usize {
            self.inner.len_bytes()
        }
        fn len_chars(&self) -> usize {
            self.inner.len_chars()
        }
        fn len_lines(&self) -> usize {
            self.inner.len_lines()
        }
        fn byte_to_char(&self, b: ByteOff) -> eve_text::CharIdx {
            self.inner.byte_to_char(b)
        }
        fn char_to_byte(&self, c: eve_text::CharIdx) -> ByteOff {
            self.inner.char_to_byte(c)
        }
        fn byte_to_line(&self, b: ByteOff) -> LineIdx {
            self.inner.byte_to_line(b)
        }
        fn line_to_byte(&self, l: LineIdx) -> ByteOff {
            self.inner.line_to_byte(l)
        }
        fn line_end_byte(&self, l: LineIdx) -> ByteOff {
            self.inner.line_end_byte(l)
        }
        fn line_terminator(&self, l: LineIdx) -> Option<eve_text::LineEnding> {
            self.inner.line_terminator(l)
        }
        fn byte_to_utf16(&self, b: ByteOff) -> eve_text::Utf16Off {
            self.inner.byte_to_utf16(b)
        }
        fn utf16_to_byte(&self, u: eve_text::Utf16Off) -> ByteOff {
            self.inner.utf16_to_byte(u)
        }
        fn is_char_boundary(&self, b: ByteOff) -> bool {
            self.inner.is_char_boundary(b)
        }
        fn chunks(&self, range: ByteRange) -> eve_text::Chunks<'_> {
            self.add(range.end.0.saturating_sub(range.start.0));
            self.inner.chunks(range)
        }
        fn chunk_at(&self, b: ByteOff) -> (&str, ByteOff) {
            let (c, s) = self.inner.chunk_at(b);
            self.add(c.len());
            (c, s)
        }
        fn slice_to_string(&self, range: ByteRange) -> String {
            self.add(range.end.0.saturating_sub(range.start.0));
            self.inner.slice_to_string(range)
        }
    }

    /// ropey のチャンクの大きさの上限の目安 (バイト)。`Counting` は `chunk_at` で渡したチャンクの全体を数える。
    const CHUNK_MAX: usize = 1024;

    /// 読む量の上限: 括弧の走査の予算 (8 MiB) と前へ読む 1 回ぶん (1 MiB)、単語と字下げは文書の大きさの程度、Range ごとに
    /// 始まりの文字を見るチャンク 1 つ (Range の数に比例。文書の大きさ × Range の数にはならない)。
    fn read_bound(len: usize, ranges: usize) -> usize {
        GOTO_SCAN_TOTAL_MAX_BYTES + (1 << 20) + 4 * len + ranges * CHUNK_MAX
    }

    /// `text` の `positions` に空の Range を置いて広げたときに読んだ量。
    fn read_for(text: &str, positions: impl Iterator<Item = usize>) -> usize {
        let t = Text::from(text);
        let c = Counting {
            inner: &t,
            read: std::cell::Cell::new(0),
        };
        let rs: Vec<Range> = positions.map(|p| Range::point(ByteOff(p))).collect();
        let sel = Selection::new(rs, 0).expect("Selection");
        let out = expand_all(&c, &sel, &mut ExpandCtx::new());
        assert_eq!(out.len(), sel.len());
        c.read.get()
    }

    // spec: 03 §5.7
    #[test]
    fn 同じ長い単語の_1_万個の_range_は単語を読み直さない() {
        // 1 行の 100 KB の単語 (括弧なし) に 1 万個の空の Range
        let text = "a".repeat(100_000);
        let read = read_for(&text, (0..10_000).map(|i| i * 10));
        let bound = read_bound(text.len(), 10_000);
        assert!(read <= bound, "読んだ量 {read} > {bound}");
    }

    // spec: 03 §5.7
    #[test]
    fn 長い字下げの行の_1_万個の_range_は字下げを読み直さない() {
        // 1 MB の字下げの後ろの単語に 1 万個の空の Range
        let text = format!("{}{}", " ".repeat(1 << 20), "b".repeat(20_000));
        let base = 1 << 20;
        let read = read_for(&text, (0..10_000).map(|i| base + i * 2));
        let bound = read_bound(text.len(), 10_000);
        assert!(read <= bound, "読んだ量 {read} > {bound}");
    }
}
