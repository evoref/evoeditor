//! gitignore の 1 行のパターンの照合 (docs/03 §12.7.3 の「照合」)。
//!
//! 振る舞いは gitignore(5) の記述と git の実行結果 (差分試験、docs/09 §3.4) に合わせて自分で組んだ。git のソースは写していない
//! (GPL-2.0。03 §12.7.3 の「ライセンス」)。パターンは字句 (`Tok`) の列にし、照らすときは字句の位置の集合を 1 バイトずつ進める
//! (非決定性の有限オートマトンの模擬)。1 回の照合の手数は「(字句の数 + 1) × パスの長さ」を超えない (`*` の後戻りをしない。
//! docs/09 §3.3 の `fuzz_gitignore` の性質)。

/// ASCII の英字の大文字小文字の扱い (リポジトリの `core.ignorecase`、03 §12.7.3)。ASCII の外の文字は畳まない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CaseFold {
    /// 区別する (`core.ignorecase = false`。リポジトリが無いとき)。
    #[default]
    Sensitive,
    /// ASCII の英字だけを畳む (`core.ignorecase = true`)。
    Ascii,
}

/// パターンを何に当てるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// パスの最後の要素 (名前) に当てる (パターンに `/` が無い)。
    Basename,
    /// 置き場所からの相対パスに当てる (先頭か途中に `/` がある)。
    Path,
}

/// 照合の字句。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok {
    /// 1 バイトの一致 (`\` で逃がした文字を含む)。
    Byte(u8),
    /// `?`: `/` 以外の 1 バイト。
    AnyButSlash,
    /// `[...]`: 集合に入る、`/` 以外の 1 バイト。
    Class(Box<ClassSets>),
    /// `*` (と、境目に無い `**`): `/` 以外の 0 バイト以上。
    Star,
    /// 境目の `**`: `/` を含む 0 バイト以上。
    AnyStar,
    /// バイトを消費せずに、次の字句と `n` 個先の字句の両方へ進む分かれ道 (`**/` が 0 個のディレクトリに当たる道。
    /// `Fork(3), AnyStar, Byte(b'/')` の形で使う)。
    Fork(usize),
}

/// 文字の集合を、畳まないときと畳むときの 2 通りで前もって引いた表 (256 ビットずつ)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClassSets {
    sensitive: [u64; 4],
    folded: [u64; 4],
}

impl ClassSets {
    fn contains(&self, b: u8, fold: CaseFold) -> bool {
        let set = match fold {
            CaseFold::Sensitive => &self.sensitive,
            CaseFold::Ascii => &self.folded,
        };
        set[usize::from(b >> 6)] & (1 << (b & 63)) != 0
    }
}

/// 照合の形 (よくある形は字句の模擬を通さずに比べる)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Program {
    /// 当たらない (閉じていない `[`・知らない POSIX の文字の集合の名前・末尾の `\`。git と同じく、その行だけ当たらない)。
    Never,
    /// ワイルドカードもエスケープも無い: 全体の一致。
    Literal(Box<[u8]>),
    /// 名前に当てる `*` + ワイルドカードの無い文字列: 末尾の一致。
    Suffix(Box<[u8]>),
    /// 字句の列。
    Tokens(Box<[Tok]>),
}

/// 字句の数の上限を超えないように長さを抑えるための値ではなく、照合の手数の上限の式 (試験と fuzz が使う)。
pub(crate) fn step_bound(pattern_len: usize, subject_len: usize) -> u64 {
    let p = u64::try_from(pattern_len).unwrap_or(u64::MAX);
    let s = u64::try_from(subject_len).unwrap_or(u64::MAX);
    p.saturating_add(1).saturating_mul(s.saturating_add(1))
}

impl Program {
    /// パターンの本文 (否定の `!`・末尾の `/`・先頭の `/` を外したもの) を組む。
    pub(crate) fn compile(pat: &[u8], mode: Mode) -> Program {
        let special = |b: &u8| matches!(b, b'*' | b'?' | b'[' | b'\\');
        if !pat.iter().any(special) {
            return Program::Literal(pat.into());
        }
        if mode == Mode::Basename
            && pat[0] == b'*'
            && pat.len() > 1
            && !pat[1..].iter().any(special)
        {
            return Program::Suffix(pat[1..].into());
        }
        match tokenize(pat, mode) {
            Some(toks) => Program::Tokens(toks.into()),
            None => Program::Never,
        }
    }

    /// 字句の数 (手数の上限の式に使う)。
    pub(crate) fn len(&self) -> usize {
        match self {
            Program::Never => 0,
            Program::Literal(s) | Program::Suffix(s) => s.len() + 1,
            Program::Tokens(t) => t.len(),
        }
    }

    /// `subject` (名前か相対パス) に当たるか。`steps` に手数を足す。
    pub(crate) fn matches(&self, subject: &[u8], fold: CaseFold, steps: &mut u64) -> bool {
        match self {
            Program::Never => false,
            Program::Literal(lit) => {
                *steps += u64::try_from(lit.len().min(subject.len())).unwrap_or(u64::MAX);
                eq_bytes(lit, subject, fold)
            }
            Program::Suffix(lit) => {
                *steps += u64::try_from(lit.len().min(subject.len())).unwrap_or(u64::MAX);
                subject.len() >= lit.len()
                    && eq_bytes(lit, &subject[subject.len() - lit.len()..], fold)
            }
            Program::Tokens(toks) => run(toks, subject, fold, steps),
        }
    }
}

fn eq_bytes(a: &[u8], b: &[u8], fold: CaseFold) -> bool {
    match fold {
        CaseFold::Sensitive => a == b,
        CaseFold::Ascii => a.eq_ignore_ascii_case(b),
    }
}

fn eq_byte(t: u8, p: u8, fold: CaseFold) -> bool {
    match fold {
        CaseFold::Sensitive => t == p,
        CaseFold::Ascii => t.eq_ignore_ascii_case(&p),
    }
}

/// パターンを字句に分ける。閉じていない `[` などで当たらない行は `None`。
fn tokenize(pat: &[u8], mode: Mode) -> Option<Vec<Tok>> {
    let mut toks = Vec::with_capacity(pat.len());
    let mut i = 0;
    while i < pat.len() {
        let c = pat[i];
        match c {
            b'\\' => {
                let next = *pat.get(i + 1)?;
                toks.push(Tok::Byte(next));
                i += 2;
            }
            b'?' => {
                toks.push(Tok::AnyButSlash);
                i += 1;
            }
            b'[' => {
                let (sets, end) = parse_class(pat, i + 1)?;
                toks.push(Tok::Class(Box::new(sets)));
                i = end;
            }
            b'*' => {
                let start = i;
                while i < pat.len() && pat[i] == b'*' {
                    i += 1;
                }
                let run = i - start;
                let before_ok = start == 0 || pat[start - 1] == b'/';
                let after = &pat[i..];
                let slash_next = after.first() == Some(&b'/');
                let after_ok = after.is_empty() || slash_next || after.starts_with(b"\\/");
                if run >= 2 && before_ok && after_ok && mode == Mode::Path {
                    if slash_next {
                        toks.push(Tok::Fork(3));
                    }
                    toks.push(Tok::AnyStar);
                } else {
                    toks.push(Tok::Star);
                }
            }
            _ => {
                toks.push(Tok::Byte(c));
                i += 1;
            }
        }
    }
    Some(toks)
}

/// POSIX の文字の集合 (ASCII で判定する)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Posix {
    Alnum,
    Alpha,
    Blank,
    Cntrl,
    Digit,
    Graph,
    Lower,
    Print,
    Punct,
    Space,
    Upper,
    Xdigit,
}

impl Posix {
    fn from_name(name: &[u8]) -> Option<Posix> {
        Some(match name {
            b"alnum" => Posix::Alnum,
            b"alpha" => Posix::Alpha,
            b"blank" => Posix::Blank,
            b"cntrl" => Posix::Cntrl,
            b"digit" => Posix::Digit,
            b"graph" => Posix::Graph,
            b"lower" => Posix::Lower,
            b"print" => Posix::Print,
            b"punct" => Posix::Punct,
            b"space" => Posix::Space,
            b"upper" => Posix::Upper,
            b"xdigit" => Posix::Xdigit,
            _ => return None,
        })
    }

    /// `t` は畳んだ後の文字 (畳むときは大文字を小文字にしたもの)。
    fn contains(self, t: u8, fold: CaseFold) -> bool {
        match self {
            Posix::Alnum => t.is_ascii_alphanumeric(),
            Posix::Alpha => t.is_ascii_alphabetic(),
            Posix::Blank => t == b' ' || t == b'\t',
            Posix::Cntrl => t < 0x20 || t == 0x7f,
            Posix::Digit => t.is_ascii_digit(),
            Posix::Graph => (0x21..=0x7e).contains(&t),
            Posix::Lower => t.is_ascii_lowercase(),
            Posix::Print => (0x20..=0x7e).contains(&t),
            Posix::Punct => t.is_ascii_punctuation(),
            // 垂直タブ・改ページは含めない (git の文字の分類に揃える)
            Posix::Space => matches!(t, b' ' | b'\t' | b'\n' | b'\r'),
            Posix::Upper => {
                t.is_ascii_uppercase() || (fold == CaseFold::Ascii && t.is_ascii_lowercase())
            }
            Posix::Xdigit => t.is_ascii_hexdigit(),
        }
    }
}

/// 集合の要素。
#[derive(Debug, Clone, Copy)]
enum Member {
    Byte(u8),
    Range(u8, u8),
    Posix(Posix),
}

/// `[` の直後 (`start`) から集合を読み、表と `]` の次の位置を返す。閉じていない・知らない POSIX の名前なら `None`。
fn parse_class(pat: &[u8], start: usize) -> Option<(ClassSets, usize)> {
    let mut j = start;
    let negated = matches!(pat.get(j), Some(b'!' | b'^'));
    if negated {
        j += 1;
    }
    let mut members: Vec<Member> = Vec::new();
    // 範囲の始まりになれる直前の要素 (範囲と POSIX の集合の後は、なれない)
    let mut prev: Option<u8> = None;
    let mut first = true;
    loop {
        let c = *pat.get(j)?;
        if c == b']' && !first {
            j += 1;
            break;
        }
        first = false;
        if c == b'\\' {
            let b = *pat.get(j + 1)?;
            members.push(Member::Byte(b));
            prev = Some(b);
            j += 2;
            continue;
        }
        if c == b'-'
            && let Some(lo) = prev
            && let Some(&next) = pat.get(j + 1)
            && next != b']'
        {
            let (hi, used) = if next == b'\\' {
                (*pat.get(j + 2)?, 3)
            } else {
                (next, 2)
            };
            members.push(Member::Range(lo, hi));
            prev = None;
            j += used;
            continue;
        }
        if c == b'[' && pat.get(j + 1) == Some(&b':') {
            let s = j + 2;
            let close = s + pat[s..].iter().position(|&b| b == b']')?;
            if close == s || pat[close - 1] != b':' {
                // `:]` で閉じていない: `[` を 1 文字の要素として読み、`:` から続ける
                members.push(Member::Byte(b'['));
                prev = Some(b'[');
                j += 1;
                continue;
            }
            members.push(Member::Posix(Posix::from_name(&pat[s..close - 1])?));
            prev = None;
            j = close + 1;
            continue;
        }
        members.push(Member::Byte(c));
        prev = Some(c);
        j += 1;
    }
    let mut sets = ClassSets {
        sensitive: [0; 4],
        folded: [0; 4],
    };
    for b in 0..=255u8 {
        if b == b'/' {
            continue;
        }
        for (fold, set) in [
            (CaseFold::Sensitive, &mut sets.sensitive),
            (CaseFold::Ascii, &mut sets.folded),
        ] {
            if class_matches(&members, b, fold) != negated {
                set[usize::from(b >> 6)] |= 1 << (b & 63);
            }
        }
    }
    Some((sets, j))
}

/// 集合の要素のどれかに当たるか。畳むときは、照らす文字を先に小文字にし、範囲だけは大文字に戻した文字でも確かめる
/// (git の振る舞い。`[A]` のような大文字 1 文字の要素は、畳むときは当たらない)。
fn class_matches(members: &[Member], b: u8, fold: CaseFold) -> bool {
    let t = match fold {
        CaseFold::Sensitive => b,
        CaseFold::Ascii => b.to_ascii_lowercase(),
    };
    members.iter().any(|m| match *m {
        Member::Byte(p) => t == p,
        Member::Range(lo, hi) => {
            (lo..=hi).contains(&t)
                || (fold == CaseFold::Ascii
                    && t.is_ascii_lowercase()
                    && (lo..=hi).contains(&t.to_ascii_uppercase()))
        }
        Member::Posix(p) => p.contains(t, fold),
    })
}

/// 字句の位置の集合 (0..=字句の数)。
struct States {
    words: Vec<u64>,
}

impl States {
    fn new(n: usize) -> States {
        States {
            words: vec![0; n / 64 + 1],
        }
    }

    fn has(&self, s: usize) -> bool {
        self.words[s / 64] & (1 << (s % 64)) != 0
    }

    fn set(&mut self, s: usize) {
        self.words[s / 64] |= 1 << (s % 64);
    }

    fn clear(&mut self) {
        self.words.fill(0);
    }

    fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }
}

/// 位置 `s` とそこから字句を消費せずに行ける位置を足す。
fn close(toks: &[Tok], set: &mut States, s: usize) {
    let mut s = s;
    loop {
        if set.has(s) {
            return;
        }
        set.set(s);
        match toks.get(s) {
            Some(Tok::Star | Tok::AnyStar) => s += 1,
            Some(Tok::Fork(n)) => {
                close(toks, set, s + n);
                s += 1;
            }
            _ => return,
        }
    }
}

fn run(toks: &[Tok], text: &[u8], fold: CaseFold, steps: &mut u64) -> bool {
    let n = toks.len();
    let mut cur = States::new(n);
    let mut next = States::new(n);
    close(toks, &mut cur, 0);
    for &b in text {
        next.clear();
        for (w, &word) in cur.words.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                let s = w * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                *steps += 1;
                let Some(tok) = toks.get(s) else { continue };
                match tok {
                    Tok::Byte(p) => {
                        if eq_byte(b, *p, fold) {
                            close(toks, &mut next, s + 1);
                        }
                    }
                    Tok::AnyButSlash => {
                        if b != b'/' {
                            close(toks, &mut next, s + 1);
                        }
                    }
                    Tok::Class(sets) => {
                        if sets.contains(b, fold) {
                            close(toks, &mut next, s + 1);
                        }
                    }
                    Tok::Star => {
                        if b != b'/' {
                            close(toks, &mut next, s);
                        }
                    }
                    Tok::AnyStar => close(toks, &mut next, s),
                    Tok::Fork(_) => {}
                }
            }
        }
        if next.is_empty() {
            return false;
        }
        std::mem::swap(&mut cur, &mut next);
    }
    cur.has(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pat: &str, mode: Mode, subject: &str, fold: CaseFold) -> bool {
        let mut steps = 0;
        let p = Program::compile(pat.as_bytes(), mode);
        let hit = p.matches(subject.as_bytes(), fold, &mut steps);
        assert!(
            steps <= step_bound(p.len(), subject.len()),
            "手数 {steps} が上限を超えた: {pat:?} / {subject:?}"
        );
        hit
    }

    fn path(pat: &str, subject: &str) -> bool {
        m(pat, Mode::Path, subject, CaseFold::Sensitive)
    }

    fn name(pat: &str, subject: &str) -> bool {
        m(pat, Mode::Basename, subject, CaseFold::Sensitive)
    }

    #[test]
    fn 星は区切りをまたがない() {
        assert!(path("a/*.c", "a/b.c"));
        assert!(!path("a/*.c", "a/b/c.c"));
        assert!(path("a/*", "a/"));
        assert!(!path("a*", "ab/c"));
    }

    #[test]
    fn 境目の二重の星() {
        assert!(path("**/foo", "foo"));
        assert!(path("**/foo", "x/y/foo"));
        assert!(path("a/**/b", "a/b"));
        assert!(path("a/**/b", "a/x/y/b"));
        assert!(!path("a/**/b", "a/xb"));
        assert!(path("a/**", "a/x/y"));
        assert!(!path("a/**", "a"));
        // 境目に無い `**` は `*` と同じ
        assert!(path("a/x**y", "a/xzy"));
        assert!(!path("a/x**y/z", "a/xq/ry/z"));
        // 逃がした `/` の前の `**` は 0 個のディレクトリに当たらない
        assert!(!path("**\\/x", "x"));
        assert!(path("**\\/x", "a/x"));
    }

    #[test]
    fn 字の並びの直後の二重の星は境目ではない() {
        assert!(!path("x/ab**", "x/abc/def"));
        assert!(path("x/ab**", "x/abc"));
        assert!(!path("a**/z", "abd/x/y/z"));
        assert!(path("a**/z", "abd/z"));
    }

    #[test]
    fn 疑問符と集合は区切りに当たらない() {
        assert!(path("a?b", "axb"));
        assert!(!path("a?b", "a/b"));
        assert!(!path("a[/]b", "a/b"));
        assert!(path("a[!x]b", "ayb"));
        assert!(path("a[^x]b", "ayb"));
        assert!(!path("a[!x]b", "a/b"));
    }

    #[test]
    fn 集合の読み方() {
        assert!(name("[]]", "]"));
        assert!(name("[!]]", "a"));
        assert!(!name("[!]]", "]"));
        assert!(name("[a-]", "-"));
        assert!(name("[-a]", "-"));
        assert!(name("[a-c]", "b"));
        assert!(!name("[c-a]", "b"));
        assert!(name("[\\]]", "]"));
        assert!(name("[[:digit:]x]", "7"));
        assert!(name("[[:digit:]x]", "x"));
        assert!(name("[[:alpha:]-z]", "-"));
        // `:]` で閉じていない `[:`
        assert!(name("[[:a]", ":"));
        assert!(name("[[:a]", "["));
        // 閉じていない・知らない名前・末尾の `\` は当たらない
        assert!(!name("[abc", "a"));
        assert!(!name("[[:foo:]]", "a"));
        assert!(!name("abc\\", "abc"));
        assert!(!name("[a-\\", "a"));
    }

    #[test]
    fn 大文字小文字を畳む() {
        let f = CaseFold::Ascii;
        assert!(m("*.log", Mode::Basename, "FOO.LOG", f));
        assert!(m("Build", Mode::Basename, "build", f));
        assert!(m("[a-z]x", Mode::Basename, "Ax", f));
        assert!(m("[A-Z]x", Mode::Basename, "ax", f));
        assert!(m("[[:upper:]]", Mode::Basename, "a", f));
        assert!(m("[[:lower:]]", Mode::Basename, "A", f));
        // ASCII の外は畳まない
        assert!(!m("é", Mode::Basename, "É", f));
        assert!(!m("*.log", Mode::Basename, "FOO.LOG", CaseFold::Sensitive));
    }

    #[test]
    fn バイト単位で照らす() {
        // `?` は 1 バイト (UTF-8 の 1 文字ではない)
        assert!(!name("?.txt", "あ.txt"));
        assert!(name("???.txt", "あ.txt"));
    }

    #[test]
    fn 星の連続でも手数は線形() {
        let pat = "*a*a*a*a*a*a*a*a*a*a*a*a*b";
        let subject = "a".repeat(400);
        assert!(!name(pat, &subject));
        assert!(!path(&format!("**/{pat}"), &format!("x/{subject}")));
    }
}
