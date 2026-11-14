//! TOML の構文を読む前の字句の走査 (07 §4.2・§17.4 の「書ける構造の上限」)。
//!
//! TOML パーサの入れ子の上限 (toml 1.2 は 80) は、インラインの表・配列の入れ子と、1 つの点のキーの段数を別々に数える。
//! 組み合わせると数千段の入れ子を作れ、1 MiB のスタック (Windows のメインスレッドの既定) を使い切る。そこで、構文を
//! 読む前に文字列と注釈の外を 1 回だけ見て、入れ子の深さとキーの段数を数える (再帰しない。使うメモリは開いている
//! インラインの表・配列の数に比例し、呼び出し側が上限で止める)。閉じていない文字列や、ほかの書き方の誤りは、この後で
//! パーサが構文エラーにする (ここでは入れ子の深さと量を作る書き方だけを見る)。
//!
//! 設定ファイル・キーマップ・テーマファイル ([`check_structure`]) と、言語ファイル (`locale_file` の `scan_structure`。
//! インラインの表・配列を書けないので、上限が別) が使う。

/// インラインの表 (`{ … }`) と配列 (`[ … ]`) の入れ子の深さの上限 (07 §4.2)。正当なファイルで最も深いのは、テーマの
/// ロールの段 (`selection = { mono = { mods = ["reverse"] } }`) とキーマップの配列の値の `args`
/// (`"f5" = [ { command = "…", args = { … } } ]`) で 3〜4 段。その 4 倍の値。
pub const TOML_MAX_NESTING: usize = 16;

/// 1 つのキーの段数 (表の見出しと点のキーの段の和。インラインの表の中のキーは、その表を持つキーの段数を足す) の上限
/// (07 §4.2)。正当なファイルで最も深いのは、条件付きセクションの中のテーマの部分上書き
/// (`[os.windows.theme.overrides]` の `"ui.selection" = { mono = { mods = […] } }` で 7 段) と、テーマファイルの
/// 構文のスコープ (`[syntax]` の `keyword.control.return = { fallback256 = { fg = … } }` で 6 段)。その約 2 倍の値。
pub const TOML_MAX_KEY_DEPTH: usize = 16;

/// 設定ファイル・キーマップ・テーマファイルの書ける構造の上限を超えた (07 §4.2)。ファイル全体を使わない (構文エラーと
/// 同じ扱い)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureProblem {
    /// インラインの表と配列の入れ子が [`TOML_MAX_NESTING`] 段を超える。位置は超えた `{` か `[`。
    Nesting,
    /// [`TOML_MAX_KEY_DEPTH`] 段を超えるキー。位置はその見出しかキーの先頭。
    KeyDepth,
}

impl StructureProblem {
    /// 理由の語の名前 (`ui.config.syntax_reason.<これ>`、04 §15.2)。
    pub fn reason_name(self) -> &'static str {
        match self {
            StructureProblem::Nesting => "nesting",
            StructureProblem::KeyDepth => "key_depth",
        }
    }
}

/// 設定ファイル・キーマップ・テーマファイルの本文を、構文を読む前に見る (07 §4.2)。書ける構造の上限を超えた最初の所の
/// バイト位置と種類を返す。
pub(crate) fn check_structure(text: &str) -> Option<(usize, StructureProblem)> {
    scan(text, |e| match e {
        Event::Open { at, nesting } if nesting > TOML_MAX_NESTING => {
            Some((at, StructureProblem::Nesting))
        }
        Event::Key { start, depth, .. } if depth > TOML_MAX_KEY_DEPTH => {
            Some((start, StructureProblem::KeyDepth))
        }
        _ => None,
    })
}

/// [`check_structure`] で上限を超えたときの診断 (エラー。ファイル全体を使わない。位置は行と桁で、どちらも 1 始まり、
/// 桁は文字数)。
pub(crate) fn structure_diagnostic(text: &str) -> Option<crate::Diagnostic> {
    let (at, problem) = check_structure(text)?;
    let head = text.get(..at).unwrap_or("");
    let line = head.matches('\n').count() + 1;
    let column = head.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    Some(crate::Diagnostic {
        severity: crate::Severity::Error,
        kind: crate::DiagKind::Structure(problem),
        key: String::new(),
        value: String::new(),
        // 診断の位置の表示用。u32 を超える行・桁は上限で丸める
        line: u32::try_from(line).unwrap_or(u32::MAX),
        column: u32::try_from(column).unwrap_or(u32::MAX),
    })
}

/// 走査で見つけたもの (書かれた順に渡す)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    /// 表の見出し (`[a.b]`・`[[a.b]]`) か、キー (`a.b =`) の終わり。`start` は見出し (`[` の位置) かキーの先頭の
    /// バイト位置、`segments` はその見出し・キー自身の段数、`depth` は根からの段数 (見出しは `segments`、最上位の
    /// キーは直前の見出しの段数を足し、インラインの表の中のキーは、その表を持つキーの段数を足す)。
    Key {
        start: usize,
        segments: usize,
        depth: usize,
    },
    /// 配列の表の見出しの `[[` (`at` は最初の `[` の位置)。続けて、その見出しの [`Event::Key`] を渡す。
    ArrayHeader { at: usize },
    /// インラインの表の `{` か、表の見出しでない `[` (配列の値・見出しの中の `[`)。`nesting` は開いた後の入れ子の深さ。
    /// 表の見出しの `[` は、インラインの表・配列の外の、行の先頭のものだけ。
    Open { at: usize, nesting: usize },
}

/// 字句の走査で見ている部分。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    /// 行の先頭 (空白だけが前にある。インラインの表・配列の外)。
    Line,
    /// 表の見出し `[ … ]` の中。
    Header,
    /// 見出しの後 (注釈だけが書ける)。
    AfterHeader,
    /// 最上位のキー (`=` の前)。
    Key,
    /// インラインの表の中のキー (`{` か `,` の後、`=` の前)。
    InlineKey,
    /// 値 (`=` の後、配列の中)。
    Value,
}

/// 開いているインラインの表・配列。
struct Open {
    /// 表 (`{`) か。
    table: bool,
    /// これを値に持つキーの、根からの段数。
    base: usize,
}

/// 文字列と注釈の外を見て、見つけたものを書かれた順に `f` へ渡す。`f` が `Some` を返したら、そこで止めてそれを返す。
///
/// 文字列は TOML の 4 種 (`"…"`・`'…'`・`"""…"""`・`'''…'''`) を読み飛ばす。インラインの表・配列の閉じ (`}`・`]`) は、
/// 開きと種類が合うときだけ数える (合わないのは構文エラーなので、深さを多めに見る)。改行は、インラインの表・配列の外で
/// だけ行を改める (配列は複数行に書ける)。
pub(crate) fn scan<R>(text: &str, mut f: impl FnMut(Event) -> Option<R>) -> Option<R> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut part = Part::Line;
    // 直前の表の見出しの段数
    let mut header_depth = 0usize;
    // 読んでいる見出し・キーの先頭と段数 (インラインの表の中のキーは、最初の文字で先頭を決める)
    let mut start: Option<usize> = None;
    let mut segments = 0usize;
    // 読んでいる値を持つキーの、根からの段数
    let mut base = 0usize;
    let mut open: Vec<Open> = Vec::new();
    while let Some(&c) = b.get(i) {
        match c {
            b'\n' => {
                if open.is_empty() {
                    part = Part::Line;
                }
                i += 1;
                continue;
            }
            b' ' | b'\t' | b'\r' => {
                i += 1;
                continue;
            }
            b'#' => {
                while b.get(i).is_some_and(|&x| x != b'\n') {
                    i += 1;
                }
                continue;
            }
            _ => {}
        }
        match part {
            Part::Line => {
                start = Some(i);
                segments = 1;
                if c == b'[' {
                    if b.get(i + 1) == Some(&b'[') {
                        if let Some(r) = f(Event::ArrayHeader { at: i }) {
                            return Some(r);
                        }
                        i += 1;
                    }
                    part = Part::Header;
                    i += 1;
                    continue;
                }
                // この文字はキーの最初の文字として、下で読み直す
                part = Part::Key;
            }
            Part::InlineKey if start.is_none() && !matches!(c, b',' | b'}') => start = Some(i),
            _ => {}
        }
        match c {
            b'"' | b'\'' => {
                i = skip_string(b, i);
                continue;
            }
            b'.' if matches!(part, Part::Header | Part::Key | Part::InlineKey) => segments += 1,
            b']' if part == Part::Header => {
                header_depth = segments;
                let ev = Event::Key {
                    start: start.unwrap_or(i),
                    segments,
                    depth: segments,
                };
                if let Some(r) = f(ev) {
                    return Some(r);
                }
                part = Part::AfterHeader;
            }
            b'=' if matches!(part, Part::Key | Part::InlineKey) => {
                let outer = if part == Part::Key {
                    header_depth
                } else {
                    open.last().map_or(0, |o| o.base)
                };
                let depth = outer.saturating_add(segments);
                let ev = Event::Key {
                    start: start.unwrap_or(i),
                    segments,
                    depth,
                };
                if let Some(r) = f(ev) {
                    return Some(r);
                }
                base = depth;
                part = Part::Value;
            }
            b'{' | b'[' => {
                open.push(Open {
                    table: c == b'{',
                    base,
                });
                if let Some(r) = f(Event::Open {
                    at: i,
                    nesting: open.len(),
                }) {
                    return Some(r);
                }
                if c == b'{' {
                    part = Part::InlineKey;
                    start = None;
                    segments = 1;
                } else {
                    part = Part::Value;
                }
            }
            b'}' | b']' if open.last().is_some_and(|o| o.table == (c == b'}')) => {
                open.pop();
                // 閉じた表・配列は、外の表のキーの値か配列の要素 (この後は `,` か閉じ)
                part = Part::Value;
                if let Some(o) = open.last() {
                    base = o.base;
                }
            }
            b',' => {
                if let Some(o) = open.last() {
                    if o.table {
                        part = Part::InlineKey;
                        start = None;
                        segments = 1;
                    } else {
                        part = Part::Value;
                        base = o.base;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// `b[start]` (`"` か `'`) から始まる文字列の次の位置。基本の文字列 (`"`) だけが `\` のエスケープを持つ。1 行の文字列は
/// 改行で終わる (閉じていない。パーサが構文エラーにする)。
fn skip_string(b: &[u8], start: usize) -> usize {
    let q = b[start];
    let basic = q == b'"';
    let triple = b.get(start + 1) == Some(&q) && b.get(start + 2) == Some(&q);
    let mut i = start + if triple { 3 } else { 1 };
    while let Some(&c) = b.get(i) {
        if basic && c == b'\\' {
            i += 2;
            continue;
        }
        if !triple && c == b'\n' {
            return i;
        }
        if c == q {
            if !triple {
                return i + 1;
            }
            if b.get(i + 1) == Some(&q) && b.get(i + 2) == Some(&q) {
                // 閉じの `"""` の後に続く引用符 (2 つまで) は中身
                let mut end = i + 3;
                while end < i + 5 && b.get(end) == Some(&q) {
                    end += 1;
                }
                return end;
            }
        }
        i += 1;
    }
    b.len()
}

#[cfg(test)]
mod tests;
