//! `EditorConfig` の glob の照合 (docs/07 §10.3 の EC-4)。言語定義の `file_globs` (§9.6 の LANG-6 (b)) が使う。
//!
//! 書ける形: `*` (`/` 以外の 0 文字以上)、`**` (`/` を含む 0 文字以上)、`?` (`/` 以外の 1 文字)、`[name]` / `[!name]`
//! (文字の集合と否定。範囲 `a-z`。`/` には当たらない)、`{s1,s2}` (どれか。入れ子にできる)、`{num1..num2}` (整数の範囲。負を
//! 含む。当てる所の数字の並び全体を整数として読んで比べる)、`\` (次の 1 文字をそのまま)。閉じていない `[` と `{`、`,` の無い
//! `{...}` は字そのもの。`/**/` (と先頭の `**/`) は 0 個以上のディレクトリ (`d/**/z.c` は `d/z.c` にも当たる)。照らすのは
//! 文字 (コードポイント) の単位。
//!
//! 照合は状態の集合を 1 文字ずつ進める (非決定性の有限オートマトンの模擬)。`*` の後戻りをせず、選択肢を前もって展開しない
//! ので、手数は「状態の数 × (文字の数 + 1)」(数の範囲は、その所から 21 字までを読む) に収まる。パースも長さに比例する
//! (`{` と `}` の対応は最初に 1 回求め、閉じない `[` の後の `[` は走査し直さない)。

/// `{` の入れ子の上限 (EC-4)。
pub const GLOB_MAX_NESTING: usize = 8;
/// 1 つの glob の選択肢 (`{a,b}` の `a` と `b`) の合計の上限 (EC-4)。
pub const GLOB_MAX_ALTERNATIVES: usize = 1_024;

/// 大文字小文字の扱い。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobCase {
    /// 区別する (`EditorConfig` の仕様どおり、EC-4)。
    Sensitive,
    /// ASCII の英字だけを畳む (言語の判定、LANG-6)。
    AsciiInsensitive,
}

/// glob を組めなかった理由 (資源の上限、EC-4)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobError {
    /// `{` の入れ子が [`GLOB_MAX_NESTING`] を超えた。
    TooDeep,
    /// 選択肢の合計が [`GLOB_MAX_ALTERNATIVES`] を超えた。
    TooManyAlternatives,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Lit(char),
    /// `?`
    Any,
    /// `*` (`cross` は `**`)
    Star {
        cross: bool,
    },
    Class {
        neg: bool,
        items: Vec<(char, char)>,
    },
    Alt(Vec<Vec<Node>>),
    Num(i64, i64),
}

/// 1 文字を消費する照合。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Lit(char),
    Any,
    Class { neg: bool, items: Vec<(char, char)> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    /// 1 文字を消費して `next` へ。
    Consume(Step, usize),
    /// `/` 以外 (`cross` なら何でも) の 0 文字以上。自分に留まるか `next` へ。
    Star { cross: bool, next: usize },
    /// 数字の並び (先頭に `-` を許す) を整数として読み、範囲に入れば `next` へ。
    Num { lo: i64, hi: i64, next: usize },
    /// 消費せずに分かれる。
    Split(Vec<usize>),
    /// 照合の終わり。
    Match,
}

/// 組んだ glob。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glob {
    states: Vec<State>,
    start: usize,
    case: GlobCase,
    has_slash: bool,
}

impl Glob {
    /// `pattern` を組む。
    ///
    /// # Errors
    ///
    /// `{` の入れ子が深すぎる・選択肢が多すぎる (EC-4 の資源の上限)。
    pub fn new(pattern: &str, case: GlobCase) -> Result<Glob, GlobError> {
        // 先頭の `/` は取る (置き場所からの相対パスに当てる印、EC-4)
        let body = pattern.strip_prefix('/');
        let chars: Vec<char> = body.unwrap_or(pattern).chars().collect();
        let (close, comma) = brace_table(&chars);
        let mut p = Parser {
            chars: &chars,
            close: &close,
            comma: &comma,
            pos: 0,
            alternatives: 0,
            case,
            slash: body.is_some(),
            unclosed: None,
            steps: 0,
        };
        let nodes = p.seq(0, false, chars.len())?;
        let has_slash = p.slash;
        let mut b = Builder { states: Vec::new() };
        let end = b.push(State::Match);
        let start = b.seq(&nodes, end);
        Ok(Glob {
            states: b.states,
            start,
            case,
            has_slash,
        })
    }

    /// glob が `/` を含むか (含めば置き場所からの相対パスに、含まなければファイル名に当てる、EC-4 / LANG-6 (b))。
    #[must_use]
    pub fn has_slash(&self) -> bool {
        self.has_slash
    }

    /// `subject` の全体に当たるか。
    #[must_use]
    pub fn is_match(&self, subject: &str) -> bool {
        let mut steps = 0;
        self.is_match_counted(subject, &mut steps)
    }

    /// 組んだ状態の数 (glob の文字数 + 1 を超えない。照合の手数の上限 [`Glob::step_bound`] の元)。
    #[must_use]
    pub fn state_count(&self) -> usize {
        self.states.len()
    }

    /// 長さ `subject_chars` (文字数) の照合の手数の上限: 状態の数 × 23 × (文字の数 + 1)。各位置で状態を 1 回ずつ見て、数の範囲は
    /// その所から符号と 21 字までを読む (EC-4 の「glob の長さ × パスの長さに比例する上限」、09 §3.3 の `fuzz_editorconfig`)。
    #[must_use]
    pub fn step_bound(&self, subject_chars: usize) -> usize {
        self.states
            .len()
            .saturating_mul(NUMBER_MAX_DIGITS + 3)
            .saturating_mul(subject_chars.saturating_add(1))
    }

    /// [`Glob::is_match`] と同じだが、照合の手数 (見た状態の数と、数の範囲で読んだ字の数) を `steps` に足す。
    pub fn is_match_counted(&self, subject: &str, steps: &mut usize) -> bool {
        let text: Vec<char> = subject.chars().map(|c| fold(c, self.case)).collect();
        let n = text.len();
        let mut pending: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
        pending[0].push(self.start);
        let mut seen = vec![usize::MAX; self.states.len()];
        for pos in 0..=n {
            let mut stack = std::mem::take(&mut pending[pos]);
            while let Some(s) = stack.pop() {
                if seen[s] == pos {
                    continue;
                }
                seen[s] = pos;
                *steps += 1;
                match &self.states[s] {
                    State::Match => {
                        if pos == n {
                            return true;
                        }
                    }
                    State::Split(next) => stack.extend(next.iter().copied()),
                    State::Star { cross, next } => {
                        stack.push(*next);
                        if let Some(&c) = text.get(pos)
                            && (*cross || c != '/')
                        {
                            pending[pos + 1].push(s);
                        }
                    }
                    State::Consume(step, next) => {
                        if let Some(&c) = text.get(pos)
                            && step_matches(step, c, self.case)
                        {
                            pending[pos + 1].push(*next);
                        }
                    }
                    State::Num { lo, hi, next } => {
                        *steps += (n - pos).min(NUMBER_MAX_DIGITS + 2);
                        if let Some(len) = number_at(&text[pos..], *lo, *hi) {
                            pending[pos + len].push(*next);
                        }
                    }
                }
            }
        }
        false
    }
}

/// ASCII の英字を小文字にする (`AsciiInsensitive` のとき)。
fn fold(c: char, case: GlobCase) -> char {
    match case {
        GlobCase::Sensitive => c,
        GlobCase::AsciiInsensitive => c.to_ascii_lowercase(),
    }
}

fn step_matches(step: &Step, c: char, case: GlobCase) -> bool {
    match step {
        Step::Lit(l) => *l == c,
        Step::Any => c != '/',
        Step::Class { neg, items } => {
            if c == '/' {
                return false;
            }
            let hit = |x: char| items.iter().any(|&(a, b)| a <= x && x <= b);
            let inside = match case {
                GlobCase::Sensitive => hit(c),
                GlobCase::AsciiInsensitive => {
                    hit(c) || hit(c.to_ascii_uppercase()) || hit(c.to_ascii_lowercase())
                }
            };
            inside != *neg
        }
    }
}

/// `chars[i]` が `\` で逃がされているか (直前の `\` の並びが奇数)。
fn escaped(chars: &[char], i: usize) -> bool {
    chars[..i].iter().rev().take_while(|&&c| c == '\\').count() % 2 == 1
}

/// 数の範囲で読む数字の並びの上限 (桁)。これより長い並びは i64 に収まらないので範囲の外 (照合の手数を並びの長さに比例させない)。
const NUMBER_MAX_DIGITS: usize = 20;

/// `text` の先頭の数字の並び (先頭の `-` を含む) の長さ。並び全体を整数として読んで `lo..=hi` に入るときだけ。
fn number_at(text: &[char], lo: i64, hi: i64) -> Option<usize> {
    let neg = text.first() == Some(&'-');
    let sign = usize::from(neg);
    let mut v: i64 = 0;
    let mut digits = 0usize;
    for c in text[sign..].iter().take(NUMBER_MAX_DIGITS + 1) {
        let Some(d) = c.to_digit(10) else {
            break;
        };
        digits += 1;
        if digits > NUMBER_MAX_DIGITS {
            return None;
        }
        // 桁が多すぎて i64 に収まらない数は範囲の外
        v = v.checked_mul(10)?.checked_add(i64::from(d))?;
    }
    if digits == 0 {
        return None;
    }
    let v = if neg { -v } else { v };
    (lo <= v && v <= hi).then_some(sign + digits)
}

/// `{` の位置ごとの、対応する `}` の位置と、その中のいちばん外に `,` があるか (`\` の次の文字は数えない)。1 回の走査で
/// 求め、パースが同じ所を何度も探さないようにする。
fn brace_table(chars: &[char]) -> (Vec<Option<usize>>, Vec<bool>) {
    let mut close = vec![None; chars.len()];
    let mut comma = vec![false; chars.len()];
    let mut open: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '{' => open.push(i),
            '}' => {
                if let Some(o) = open.pop() {
                    close[o] = Some(i);
                }
            }
            ',' => {
                if let Some(&o) = open.last() {
                    comma[o] = true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (close, comma)
}

struct Parser<'a> {
    chars: &'a [char],
    /// `brace_table` の結果。
    close: &'a [Option<usize>],
    comma: &'a [bool],
    pos: usize,
    alternatives: usize,
    case: GlobCase,
    /// 逃がしていない `/` を読んだ (先頭の `/` を含む)。
    slash: bool,
    /// 閉じなかった `[` の走査 (始めた位置, 範囲の終わり)。同じ範囲でそれより後ろの `[` も閉じない (後ろから始めても、走査は
    /// 同じ字の並びを同じ区切りで読むので、閉じる `]` が無い) ので、走査せずに字にする (パースを長さの 2 乗にしない)。
    unclosed: Option<(usize, usize)>,
    /// `[` の中を読んだ字の数 (試験で手数を確かめる)。
    steps: usize,
}

impl Parser<'_> {
    /// `end` までの並びを読む。`in_alt` なら `,` で止まる (読まない)。
    fn seq(&mut self, depth: usize, in_alt: bool, end: usize) -> Result<Vec<Node>, GlobError> {
        let mut out = Vec::new();
        while self.pos < end {
            let c = self.chars[self.pos];
            if in_alt && c == ',' {
                break;
            }
            self.pos += 1;
            match c {
                '\\' => match self.chars.get(self.pos).filter(|_| self.pos < end) {
                    Some(&n) => {
                        self.pos += 1;
                        out.push(Node::Lit(fold(n, self.case)));
                    }
                    None => out.push(Node::Lit('\\')),
                },
                '?' => out.push(Node::Any),
                '*' => {
                    let at = self.pos - 1;
                    if self.chars.get(self.pos) == Some(&'*') {
                        while self.pos < end && self.chars[self.pos] == '*' {
                            self.pos += 1;
                        }
                        // `/**/` (と先頭の `**/`) は 0 個以上のディレクトリ (`d/**/z.c` は `d/z.c` にも当たる。EditorConfig
                        // と同じ、EC-4)。星がちょうど 2 つで、前の `/` が逃がしでないときだけ (`a/***/b`・`a\/**/b` は違う)
                        let two = self.pos - at == 2;
                        let after_sep =
                            at == 0 || (self.chars[at - 1] == '/' && !escaped(self.chars, at - 1));
                        if two && after_sep && self.pos < end && self.chars[self.pos] == '/' {
                            self.pos += 1;
                            self.slash = true;
                            out.push(Node::Alt(vec![
                                vec![Node::Star { cross: true }, Node::Lit('/')],
                                Vec::new(),
                            ]));
                        } else {
                            out.push(Node::Star { cross: true });
                        }
                    } else {
                        out.push(Node::Star { cross: false });
                    }
                }
                '[' => match self.class(end) {
                    Some(n) => out.push(n),
                    None => out.push(Node::Lit('[')),
                },
                '{' => out.extend(self.brace(depth)?),
                c => {
                    if c == '/' {
                        self.slash = true;
                    }
                    out.push(Node::Lit(fold(c, self.case)));
                }
            }
        }
        Ok(out)
    }

    /// `[` の後ろ (`end` まで)。閉じていなければ `None` (読んだ位置を戻す)。範囲の端にも `\` が効く (`[a-\]]` は `a` から `]`)。
    fn class(&mut self, end: usize) -> Option<Node> {
        let save = self.pos;
        if let Some((from, e)) = self.unclosed
            && e == end
            && save > from
        {
            return None;
        }
        let mut neg = false;
        if self.pos < end && matches!(self.chars[self.pos], '!' | '^') {
            neg = true;
            self.pos += 1;
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            if self.pos >= end {
                self.unclosed = Some((save, end));
                self.pos = save;
                return None;
            }
            self.steps += 1;
            let c = self.chars[self.pos];
            self.pos += 1;
            if c == ']' && !first {
                break;
            }
            first = false;
            let c = if c == '\\' && self.pos < end {
                self.pos += 1;
                self.chars[self.pos - 1]
            } else {
                c
            };
            if self.pos + 1 < end && self.chars[self.pos] == '-' && self.chars[self.pos + 1] != ']'
            {
                // 範囲の端の `\` は次の 1 字 (`[a-\]]`)。`\` が範囲の最後の字なら、`\` そのもの
                let (hi, used) = if self.chars[self.pos + 1] == '\\' && self.pos + 2 < end {
                    (self.chars[self.pos + 2], 3)
                } else {
                    (self.chars[self.pos + 1], 2)
                };
                self.pos += used;
                items.push((c.min(hi), c.max(hi)));
            } else {
                items.push((c, c));
            }
        }
        Some(Node::Class { neg, items })
    }

    /// `{` の後ろ。選択肢 (`,` がある) か数の範囲なら、その節。どちらでもなければ `{` と `}` は字で、中は並びとして読む。
    /// どの場合も `{` の入れ子は [`GLOB_MAX_NESTING`] 段まで (字の `{` も数える。深い入れ子でスタックを使い切らない)。
    fn brace(&mut self, depth: usize) -> Result<Vec<Node>, GlobError> {
        let open = self.pos - 1;
        let Some(close) = self.close[open] else {
            // 閉じていない `{` は字
            return Ok(vec![Node::Lit('{')]);
        };
        if depth + 1 > GLOB_MAX_NESTING {
            return Err(GlobError::TooDeep);
        }
        let start = self.pos;
        if !self.comma[open] {
            let inner: String = self.chars[start..close].iter().collect();
            if let Some((lo, hi)) = numeric_range(&inner) {
                self.pos = close + 1;
                return Ok(vec![Node::Num(lo.min(hi), lo.max(hi))]);
            }
            // `{single}`: 括弧は字。中は並びとして読む (中の `}` で外の選択肢を閉じない)
            let mut out = vec![Node::Lit('{')];
            out.extend(self.seq(depth + 1, false, close)?);
            out.push(Node::Lit('}'));
            self.pos = close + 1;
            return Ok(out);
        }
        let mut alts = Vec::new();
        loop {
            let a = self.seq(depth + 1, true, close)?;
            alts.push(a);
            self.alternatives += 1;
            if self.alternatives > GLOB_MAX_ALTERNATIVES {
                return Err(GlobError::TooManyAlternatives);
            }
            if self.pos < close && self.chars[self.pos] == ',' {
                self.pos += 1;
            } else {
                self.pos = close + 1;
                break;
            }
        }
        Ok(vec![Node::Alt(alts)])
    }
}

/// `num1..num2` (どちらも整数。負を含む)。
fn numeric_range(s: &str) -> Option<(i64, i64)> {
    let (a, b) = s.split_once("..")?;
    let int = |t: &str| -> Option<i64> {
        let digits = t.strip_prefix('-').unwrap_or(t);
        if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        t.parse().ok()
    };
    Some((int(a)?, int(b)?))
}

struct Builder {
    states: Vec<State>,
}

impl Builder {
    fn push(&mut self, s: State) -> usize {
        self.states.push(s);
        self.states.len() - 1
    }

    /// 並び `nodes` を、終わりが `next` へ続く状態に組み、入口を返す (後ろから組む)。
    fn seq(&mut self, nodes: &[Node], next: usize) -> usize {
        let mut next = next;
        for n in nodes.iter().rev() {
            next = match n {
                Node::Lit(c) => self.push(State::Consume(Step::Lit(*c), next)),
                Node::Any => self.push(State::Consume(Step::Any, next)),
                Node::Class { neg, items } => self.push(State::Consume(
                    Step::Class {
                        neg: *neg,
                        items: items.clone(),
                    },
                    next,
                )),
                Node::Star { cross } => self.push(State::Star {
                    cross: *cross,
                    next,
                }),
                Node::Num(lo, hi) => self.push(State::Num {
                    lo: *lo,
                    hi: *hi,
                    next,
                }),
                Node::Alt(alts) => {
                    let entries: Vec<usize> = alts.iter().map(|a| self.seq(a, next)).collect();
                    self.push(State::Split(entries))
                }
            };
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, s: &str) -> bool {
        Glob::new(p, GlobCase::Sensitive).unwrap().is_match(s)
    }

    #[test]
    fn 星と二重の星と疑問符() {
        assert!(m("*.rs", "main.rs"));
        assert!(!m("*.rs", "src/main.rs"), "* は / に当たらない");
        assert!(m("**.rs", "src/main.rs"));
        assert!(m("src/**/x", "src/a/b/x"));
        assert!(m("a?c", "abc"));
        assert!(!m("a?c", "a/c"));
        assert!(m("*", ""));
        assert!(!m("?", ""));
    }

    #[test]
    fn 文字の集合と否定と範囲() {
        assert!(m("[abc].txt", "b.txt"));
        assert!(!m("[!abc].txt", "b.txt"));
        assert!(m("[!abc].txt", "d.txt"));
        assert!(m("[a-c]x", "bx"));
        assert!(!m("[a-c]x", "dx"));
        assert!(m("[x", "[x"), "閉じていない [ は字");
        assert!(!m("[/]", "/"), "集合は / に当たらない");
    }

    /// `EditorConfig` のほかの実装と意図して違う所 (07 §10.3 の EC-4)。
    #[test]
    fn ほかの実装と意図して違う所() {
        // `/` を含む集合も集合のまま (字の並びにしない)。集合は `/` に当たらない
        assert!(m("[a/]", "a"));
        assert!(!m("[a/]", "/"));
        assert!(!m("[a/]", "[a/]"));
        // `[/]a` はどのパスにも当たらない (集合の中の `/` は区切りに数えず、ファイル名だけに当てる)
        assert!(!m("[/]a", "/a"));
        assert!(!m("[/]a", "x/a"));
        // 数の範囲は、その所の数字の並び全体を読む (`{1..3}5` は `25` に当たらない)
        assert!(!m("{1..3}5", "25"));
        assert!(m("{1..3}x5", "2x5"));
    }

    #[test]
    fn 選択肢と入れ子() {
        assert!(m("*.{js,ts}", "a.ts"));
        assert!(m("*.{js,ts}", "a.js"));
        assert!(!m("*.{js,ts}", "a.rs"));
        assert!(m("{a,b{c,d}}", "bd"));
        assert!(m("{single}", "{single}"), ", の無い {{...}} は字");
        assert!(m("{a", "{a"), "閉じていない {{");
        assert!(m("{,x}y", "y"), "空の選択肢");
        assert!(m("{a{x}b,c}", "a{x}b"), "選択肢の中の {{x}} は字");
        assert!(m("{a{x}b,c}", "c"));
    }

    #[test]
    fn 数の範囲は並び全体を整数として読む() {
        assert!(m("f{1..3}", "f2"));
        assert!(!m("f{1..3}", "f4"));
        assert!(!m("f{1..3}", "f12"), "並び全体 (12) で比べる");
        assert!(m("f{-5..5}", "f-3"));
        assert!(m("f{3..1}", "f2"), "逆の順も同じ範囲");
        assert!(m("f{1..3}.txt", "f03.txt"), "先頭の 0 は値に影響しない");
    }

    #[test]
    fn 逃がしと大文字小文字() {
        assert!(m("a\\*b", "a*b"));
        assert!(!m("a\\*b", "axb"));
        assert!(!m("*.RS", "main.rs"));
        let g = Glob::new("*.RS", GlobCase::AsciiInsensitive).unwrap();
        assert!(g.is_match("MAIN.rs"));
        let c = Glob::new("[A-C]x", GlobCase::AsciiInsensitive).unwrap();
        assert!(c.is_match("bX"));
    }

    #[test]
    fn 資源の上限() {
        let deep = "{a,".repeat(9) + &"}".repeat(9);
        assert_eq!(
            Glob::new(&deep, GlobCase::Sensitive),
            Err(GlobError::TooDeep)
        );
        let many = format!("{{{}}}", vec!["a"; 1_100].join(","));
        assert_eq!(
            Glob::new(&many, GlobCase::Sensitive),
            Err(GlobError::TooManyAlternatives)
        );
        assert!(Glob::new(&"{a,".repeat(8), GlobCase::Sensitive).is_ok());
    }

    #[test]
    fn 長いパスでも状態の数と長さに比例する() {
        let g = Glob::new("**a*b*c*d*e", GlobCase::Sensitive).unwrap();
        let s = "x".repeat(10_000);
        assert!(!g.is_match(&s));
    }

    /// `**` の振る舞い (`EditorConfig` の `star_star` の試験と同じ形の場合を、自分で書いた。07 §10.3 の EC-4)。
    #[test]
    fn 二重の星は区切りをまたぎ_斜線で囲めば_0_個のディレクトリにも当たる() {
        // `a**z.c`: 区切りをまたぐ
        for s in [
            "a/z.c", "amnz.c", "am/nz.c", "a/mnz.c", "amn/z.c", "a/mn/z.c",
        ] {
            assert!(m("a**z.c", s), "{s}");
        }
        // `b/**z.c`: `b/` の後
        for (s, want) in [
            ("b/z.c", true),
            ("b/mnz.c", true),
            ("b/mn/z.c", true),
            ("bmnz.c", false),
            ("bm/nz.c", false),
            ("bmn/z.c", false),
        ] {
            assert_eq!(m("b/**z.c", s), want, "{s}");
        }
        // `c**/z.c`: `/z.c` の前
        for (s, want) in [
            ("c/z.c", true),
            ("cmn/z.c", true),
            ("c/mn/z.c", true),
            ("cmnz.c", false),
            ("cm/nz.c", false),
            ("c/mnz.c", false),
        ] {
            assert_eq!(m("c**/z.c", s), want, "{s}");
        }
        // `d/**/z.c`: 0 個以上のディレクトリ
        for (s, want) in [
            ("d/z.c", true),
            ("d/mn/z.c", true),
            ("dmnz.c", false),
            ("dm/nz.c", false),
            ("d/mnz.c", false),
            ("dmn/z.c", false),
        ] {
            assert_eq!(m("d/**/z.c", s), want, "{s}");
        }
        assert!(m(".github/**/*.yml", ".github/a.yml"));
        // 星がちょうど 2 つで、前の `/` が逃がしでないときだけ
        assert!(!m("a/***/b", "a/b"));
        assert!(m("a/***/b", "a/x/b"));
        assert!(!m("a\\/**/b", "a/b"));
        assert!(m("a\\/**/b", "a/x/b"));
        assert!(m("src/**/x", "src/x"));
        assert!(m("**/x", "x"), "先頭の **/ も 0 個");
        assert!(m("**/x", "a/b/x"));
    }

    #[test]
    fn 斜線を含むかは逃がしていない斜線だけで決める() {
        let has = |p: &str| Glob::new(p, GlobCase::Sensitive).unwrap().has_slash();
        assert!(has("a/b"));
        assert!(has("/a"));
        assert!(has("**/a"));
        assert!(!has("[/]a"), "集合の中の / は数えない");
        assert!(!has("a\\/b"), "逃がした / は数えない");
        assert!(!has("*.rs"));
    }

    #[test]
    fn 字の括弧の入れ子と長い数字にも上限がある() {
        let deep = "{".repeat(3_000) + &"}".repeat(3_000);
        assert_eq!(
            Glob::new(&deep, GlobCase::Sensitive),
            Err(GlobError::TooDeep),
            "選択肢でない {{...}} の入れ子もスタックを使い切らない"
        );
        let lit = "{".repeat(8) + "x" + &"}".repeat(8);
        assert!(m(&lit, &lit));
        let g = Glob::new("f{1..9}", GlobCase::Sensitive).unwrap();
        assert!(
            !g.is_match(&format!("f{}", "1".repeat(21))),
            "21 桁は範囲の外"
        );
        assert!(
            !g.is_match(&format!("f{}", "9".repeat(19))),
            "i64 に収まっても範囲の外"
        );
        let long = format!("f{}", "1".repeat(100_000));
        assert!(!g.is_match(&long));
    }

    #[test]
    fn 閉じない角括弧の多い_glob_のパースは長さに比例する() {
        let steps = |p: &str| {
            let chars: Vec<char> = p.chars().collect();
            let (close, comma) = brace_table(&chars);
            let mut parser = Parser {
                chars: &chars,
                close: &close,
                comma: &comma,
                pos: 0,
                alternatives: 0,
                case: GlobCase::Sensitive,
                slash: false,
                unclosed: None,
                steps: 0,
            };
            parser.seq(0, false, chars.len()).unwrap();
            parser.steps
        };
        let n = 4_096;
        assert!(steps(&"[".repeat(n)) <= 2 * n, "{}", steps(&"[".repeat(n)));
        assert!(steps(&"[a".repeat(n)) <= 4 * n);
        // 選択肢の中で閉じない `[` の後でも、範囲の外 (選択肢の後ろ) の `[...]` は読む
        assert!(m("{a,[b}[c]", "[bc"));
        assert!(m("{a,[b}[c]", "ac"));
        assert!(m("[a]", "a"));
    }

    #[test]
    fn 範囲の端の逃がし() {
        assert!(m("[a-\\]]", "]"), "a から ] (逃がした ])");
        assert!(m("[a-\\]]", "^"));
        assert!(!m("[a-\\]]", "b"));
        assert!(m("[a-\\]", "[a-]"), "閉じないので字");
        assert!(m("[\\]]", "]"));
    }

    #[test]
    fn 数の範囲は_21_字までしか読まない() {
        // 0 の長い並びでも、各位置で 21 字までしか読まない (並びの長さの 2 乗にしない)
        let g = Glob::new("*{0..1}", GlobCase::Sensitive).unwrap();
        let s = "0".repeat(100_000);
        let t = std::time::Instant::now();
        assert!(g.is_match(&s), "末尾の 0 が 1 字の並び");
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            t.elapsed()
        );
    }

    #[test]
    fn 選択肢の外の先頭の斜線は置き場所の印にしない() {
        // 先頭の `/` だけが置き場所の印。選択肢を展開すると先頭に来る `/` は字 (`{,}/` は `/` に当たり、空には当たらない)
        assert!(m("{,}/", "/"));
        assert!(!m("{,}/", ""));
    }

    proptest::proptest! {
        /// `{a,b}` を含む glob への照合は、選択肢を展開した glob のどれかへの照合と一致する (09 §3.3 の fuzz の性質)。
        /// 先頭の `/` (置き場所の印) と `**` (区切りとの並びで意味が変わる) は、展開の前後で同じ意味にならないので作らない:
        /// どの部分も `*` の後に `*` 以外が来る形で組み、展開して `*` が並ばないように、`pre` は `*` で終わらず、選択肢と
        /// `post` は `*` で始まらない。
        #[test]
        fn 選択肢の照合は展開したどれかへの照合と一致する(
            pre in "[ab]([ab/?]|\\*[ab/?]){0,3}",
            alts in proptest::collection::vec("([ab/?](\\*[ab/?])?){0,2}", 2..4),
            post in "([ab/?]\\*?){0,3}",
            subject in "[ab/]{0,8}",
        ) {
            let expanded: Vec<String> = alts.iter().map(|a| format!("{pre}{a}{post}")).collect();
            let glob = format!("{pre}{{{}}}{post}", alts.join(","));
            let whole = Glob::new(&glob, GlobCase::Sensitive).unwrap().is_match(&subject);
            let any = expanded
                .iter()
                .any(|e| Glob::new(e, GlobCase::Sensitive).unwrap().is_match(&subject));
            proptest::prop_assert_eq!(whole, any, "{} / {}", glob, subject);
        }
    }
}
