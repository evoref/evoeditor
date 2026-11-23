//! `EditorConfig` の glob の照合 (docs/07 §10.3 の EC-4)。言語定義の `file_globs` (§9.6 の LANG-6 (b)) が使う。
//!
//! 書ける形: `*` (`/` 以外の 0 文字以上)、`**` (`/` を含む 0 文字以上)、`?` (`/` 以外の 1 文字)、`[name]` / `[!name]`
//! (文字の集合と否定。範囲 `a-z`。`/` には当たらない)、`{s1,s2}` (どれか。入れ子にできる)、`{num1..num2}` (整数の範囲。負を
//! 含む。当てる所の数字の並び全体を整数として読んで比べる)、`\` (次の 1 文字をそのまま)。閉じていない `[` と `{`、`,` の無い
//! `{...}` は字そのもの。照らすのは文字 (コードポイント) の単位。
//!
//! 照合は状態の集合を 1 文字ずつ進める (非決定性の有限オートマトンの模擬)。`*` の後戻りをせず、選択肢を前もって展開しない
//! ので、手数は「状態の数 × (文字の数 + 1)」(数の範囲は、その所の数字の並びの長さを掛ける) に収まる。

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
        let chars: Vec<char> = pattern
            .strip_prefix('/')
            .unwrap_or(pattern)
            .chars()
            .collect();
        let mut p = Parser {
            chars: &chars,
            pos: 0,
            alternatives: 0,
            case,
        };
        let nodes = p.seq(0, false)?;
        let mut b = Builder { states: Vec::new() };
        let end = b.push(State::Match);
        let start = b.seq(&nodes, end);
        Ok(Glob {
            states: b.states,
            start,
            case,
            has_slash: pattern.contains('/'),
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

/// `text` の先頭の数字の並び (先頭の `-` を含む) の長さ。並び全体を整数として読んで `lo..=hi` に入るときだけ。
fn number_at(text: &[char], lo: i64, hi: i64) -> Option<usize> {
    let sign = usize::from(text.first() == Some(&'-'));
    let digits = text[sign..]
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    let s: String = text[..sign + digits].iter().collect();
    // 桁が多すぎて i64 に収まらない数は範囲の外
    let v: i64 = s.parse().ok()?;
    (lo <= v && v <= hi).then_some(sign + digits)
}

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
    alternatives: usize,
    case: GlobCase,
}

impl Parser<'_> {
    /// 並びを読む。`in_alt` なら `,` と `}` で止まる (読まない)。
    fn seq(&mut self, depth: usize, in_alt: bool) -> Result<Vec<Node>, GlobError> {
        let mut out = Vec::new();
        while let Some(&c) = self.chars.get(self.pos) {
            if in_alt && (c == ',' || c == '}') {
                break;
            }
            self.pos += 1;
            match c {
                '\\' => match self.chars.get(self.pos) {
                    Some(&n) => {
                        self.pos += 1;
                        out.push(Node::Lit(fold(n, self.case)));
                    }
                    None => out.push(Node::Lit('\\')),
                },
                '?' => out.push(Node::Any),
                '*' => {
                    if self.chars.get(self.pos) == Some(&'*') {
                        while self.chars.get(self.pos) == Some(&'*') {
                            self.pos += 1;
                        }
                        out.push(Node::Star { cross: true });
                    } else {
                        out.push(Node::Star { cross: false });
                    }
                }
                '[' => match self.class() {
                    Some(n) => out.push(n),
                    None => out.push(Node::Lit('[')),
                },
                '{' => out.extend(self.brace(depth)?),
                c => out.push(Node::Lit(fold(c, self.case))),
            }
        }
        Ok(out)
    }

    /// `[` の後ろ。閉じていなければ `None` (読んだ位置を戻す)。
    fn class(&mut self) -> Option<Node> {
        let save = self.pos;
        let mut neg = false;
        if matches!(self.chars.get(self.pos), Some('!' | '^')) {
            neg = true;
            self.pos += 1;
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let Some(&c) = self.chars.get(self.pos) else {
                self.pos = save;
                return None;
            };
            self.pos += 1;
            if c == ']' && !first {
                break;
            }
            first = false;
            let c = if c == '\\' {
                match self.chars.get(self.pos) {
                    Some(&n) => {
                        self.pos += 1;
                        n
                    }
                    None => '\\',
                }
            } else {
                c
            };
            if self.chars.get(self.pos) == Some(&'-')
                && let Some(&hi) = self.chars.get(self.pos + 1)
                && hi != ']'
            {
                self.pos += 2;
                items.push((c.min(hi), c.max(hi)));
            } else {
                items.push((c, c));
            }
        }
        Some(Node::Class { neg, items })
    }

    /// `{` の後ろ。選択肢 (`,` がある) か数の範囲なら、その節。どちらでもなければ `{` を字として返し、続きは並びとして読む。
    fn brace(&mut self, depth: usize) -> Result<Vec<Node>, GlobError> {
        let save = self.pos;
        // 対応する `}` を探す (入れ子と `\` を数える)。無ければ `{` は字
        let Some(close) = self.matching_close() else {
            return Ok(vec![Node::Lit('{')]);
        };
        let inner: String = self.chars[save..close].iter().collect();
        if let Some((lo, hi)) = numeric_range(&inner) {
            self.pos = close + 1;
            return Ok(vec![Node::Num(lo.min(hi), lo.max(hi))]);
        }
        if !self.has_top_comma(save, close) {
            // `{single}`: 括弧は字。中は並びとして読む (中の `}` で外の選択肢を閉じない)
            let mut inside = Parser {
                chars: &self.chars[save..close],
                pos: 0,
                alternatives: self.alternatives,
                case: self.case,
            };
            let mut out = vec![Node::Lit('{')];
            out.extend(inside.seq(depth, false)?);
            out.push(Node::Lit('}'));
            self.alternatives = inside.alternatives;
            self.pos = close + 1;
            return Ok(out);
        }
        if depth + 1 > GLOB_MAX_NESTING {
            return Err(GlobError::TooDeep);
        }
        let mut alts = Vec::new();
        loop {
            let a = self.seq(depth + 1, true)?;
            alts.push(a);
            self.alternatives += 1;
            if self.alternatives > GLOB_MAX_ALTERNATIVES {
                return Err(GlobError::TooManyAlternatives);
            }
            match self.chars.get(self.pos) {
                Some(',') => self.pos += 1,
                _ => {
                    // `}`
                    self.pos += 1;
                    break;
                }
            }
        }
        Ok(vec![Node::Alt(alts)])
    }

    /// `self.pos` (`{` の後ろ) から、対応する `}` の位置。
    fn matching_close(&self) -> Option<usize> {
        let mut depth = 0usize;
        let mut i = self.pos;
        while let Some(&c) = self.chars.get(i) {
            match c {
                '\\' => i += 1,
                '{' => depth += 1,
                '}' if depth == 0 => return Some(i),
                '}' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// `[from, to)` の入れ子の外に `,` があるか。
    fn has_top_comma(&self, from: usize, to: usize) -> bool {
        let mut depth = 0usize;
        let mut i = from;
        while i < to {
            match self.chars[i] {
                '\\' => i += 1,
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => return true,
                _ => {}
            }
            i += 1;
        }
        false
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
}
