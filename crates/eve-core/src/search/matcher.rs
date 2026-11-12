//! 検索語の解釈と照合 (`SearchQuery` → `Matcher`、docs/03 §12.1・§12.3・§12.4)。
//!
//! リテラル (memmem)・ASCII の大小を畳み込んだ memmem・行単位の正規表現の 3 つの経路を `Matcher` に閉じる。
//! 大小を区別しないリテラルで ASCII の経路に乗らないものは、検索語をエスケープして正規表現の経路で探す (§12.4 の 3)。

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use eve_text::{ByteOff, ByteRange, TextRead, is_grapheme_boundary, word_boundaries};
use memchr::memmem;

/// 大文字小文字の扱い (FR-402)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// 検索語に大文字を含めば区別し、含まなければ区別しない。
    Smart,
    /// 常に区別する。
    Sensitive,
    /// 常に区別しない。
    Insensitive,
}

impl Case {
    /// `search.toggle_case` の巡回: Smart → Sensitive → Insensitive → Smart。
    pub fn next(self) -> Case {
        match self {
            Case::Smart => Case::Sensitive,
            Case::Sensitive => Case::Insensitive,
            Case::Insensitive => Case::Smart,
        }
    }
}

/// 検索の問い合わせ。構築は `SearchQuery::literal` 経由 (v0.2 以降に項目を足しても呼び出し側を壊さない、03 §12.1)。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SearchQuery {
    pattern: String,
    case: Case,
    /// 正規表現として探す (FR-402、v0.2。行単位で実行する、§12.3)。
    regex: bool,
    /// 一致の両端が単語境界 (§13.3) にあるものだけを採る (FR-402、v0.2)。
    whole_word: bool,
    /// 選択範囲内の検索 (`scope: Selection`、§12.1 / §12.5)。この範囲 (開始位置の昇順で重ならない) の
    /// どれかに収まる一致だけを採る。`None` は文書全体。
    scope: Option<Arc<[ByteRange]>>,
    /// コンパイルの結果の覚え。複製 (検索ジョブに渡すものを含む) と共有し、条件を変えると捨てる (PRF-09)。
    compiled: Compiled,
}

/// `SearchQuery` のコンパイルの結果の覚え。比較と表示では無視する (問い合わせの同一性は条件だけで決まる)。
#[derive(Clone, Default)]
struct Compiled(Arc<OnceLock<Result<Option<Arc<Matcher>>, QueryError>>>);

impl PartialEq for Compiled {
    fn eq(&self, _: &Compiled) -> bool {
        true
    }
}

impl Eq for Compiled {}

impl std::fmt::Debug for Compiled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.get().is_some() {
            "Compiled"
        } else {
            "NotCompiled"
        })
    }
}

/// 検索語をコンパイルできない理由 (§12.3)。入力中は赤色表示のみで、ジョブは投入しない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryError {
    /// 正規表現の構文エラー (`regex` の文言を整形したもの)。
    Syntax(String),
    /// 改行にまたがるパターン (`\n` など)。複数行の正規表現は v0.4 (v0.3 までは未対応)。
    Multiline,
    /// コンパイル後の大きさが上限 (10 MiB) を超える。
    TooLarge,
}

/// 正規表現のコンパイル後の大きさの上限 (§12.3)。
const REGEX_SIZE_LIMIT: usize = 10 * 1024 * 1024;

impl SearchQuery {
    /// リテラル検索 (大小は `Smart`)。
    pub fn literal(pattern: &str) -> SearchQuery {
        SearchQuery {
            pattern: pattern.to_owned(),
            case: Case::Smart,
            regex: false,
            whole_word: false,
            scope: None,
            compiled: Compiled::default(),
        }
    }

    /// 選択範囲内に限る (`None` は文書全体)。範囲は開始位置の昇順で重ならないこと。
    pub fn with_scope(mut self, scope: Option<Arc<[ByteRange]>>) -> SearchQuery {
        self.scope = scope;
        self.compiled = Compiled::default();
        self
    }

    /// 検索の範囲 (バイト範囲の昇順)。`None` は文書全体。
    pub fn scope(&self) -> Option<&[ByteRange]> {
        self.scope.as_deref()
    }

    /// 正規表現として探す (既定は `false`)。
    pub fn with_regex(mut self, regex: bool) -> SearchQuery {
        self.regex = regex;
        self.compiled = Compiled::default();
        self
    }

    /// 一致の両端が単語境界にあるものだけを採る (既定は `false`)。
    pub fn with_whole_word(mut self, whole_word: bool) -> SearchQuery {
        self.whole_word = whole_word;
        self.compiled = Compiled::default();
        self
    }

    /// 正規表現として探す。
    pub fn is_regex(&self) -> bool {
        self.regex
    }

    /// 単語単位で探す。
    pub fn is_whole_word(&self) -> bool {
        self.whole_word
    }

    /// 大文字小文字の扱いを変える。
    pub fn with_case(mut self, case: Case) -> SearchQuery {
        self.case = case;
        self.compiled = Compiled::default();
        self
    }

    /// 検索語。
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// 大文字小文字の扱い。
    pub fn case(&self) -> Case {
        self.case
    }

    /// 大小を実際に区別して探すか (検索バーのトグルを反転して示す)。
    pub fn is_case_sensitive(&self) -> bool {
        match self.case {
            Case::Sensitive => true,
            Case::Insensitive => false,
            Case::Smart => self.smart_has_upper(),
        }
    }

    /// `Smart` の判定: 大文字を含めば区別する。正規表現では `\W` `\S` のようなエスケープの大文字は数えない。
    fn smart_has_upper(&self) -> bool {
        if !self.regex {
            return self.pattern.chars().any(|c| lower_char(c) != c);
        }
        let mut escaped = false;
        for c in self.pattern.chars() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if lower_char(c) != c {
                return true;
            }
        }
        false
    }

    /// 大小を区別せずに探すか (`Smart` は大文字を含まないとき)。
    fn insensitive(&self) -> bool {
        match self.case {
            Case::Sensitive => false,
            Case::Insensitive => true,
            Case::Smart => !self.smart_has_upper(),
        }
    }

    /// 照合器。検索語が空・コンパイルできない・改行にまたがるなら `None` (何にも一致しない)。
    /// 理由が要るときは `compile` を使う。
    pub fn matcher(&self) -> Option<Arc<Matcher>> {
        self.compile().ok().flatten()
    }

    /// 照合器を作る。検索語が空なら `Ok(None)`。正規表現の構文エラーなどは `Err`。
    /// 結果は覚えて、この問い合わせとその複製で使い回す (同じ条件でコンパイルし直さない、PRF-09)。
    ///
    /// # Errors
    ///
    /// 正規表現の構文の誤り (`Syntax`)、改行にまたがるパターン (`Multiline`)、コンパイル後の大きさの上限の超過 (`TooLarge`)。
    pub fn compile(&self) -> Result<Option<Arc<Matcher>>, QueryError> {
        self.compiled
            .0
            .get_or_init(|| self.compile_uncached().map(|m| m.map(Arc::new)))
            .clone()
    }

    fn compile_uncached(&self) -> Result<Option<Matcher>, QueryError> {
        if self.regex {
            return self.compile_regex();
        }
        // 検索語は 1 行の入力欄から来る。改行を含むものは行をまたぐので一致させない (v0.2 の複数行は別途)
        if self.pattern.is_empty() || self.pattern.contains(['\n', '\r']) {
            return Ok(None);
        }
        self.compile_literal().map(Some)
    }

    fn compile_regex(&self) -> Result<Option<Matcher>, QueryError> {
        if self.pattern.is_empty() {
            return Ok(None);
        }
        // 複数行の正規表現は v0.4。改行そのものと、`\n` `\r` のエスケープは未対応 (§12.3)
        let escapes_newline = {
            let mut escaped = false;
            let mut found = false;
            for c in self.pattern.chars() {
                if escaped {
                    found |= matches!(c, 'n' | 'r');
                    escaped = false;
                } else {
                    escaped = c == '\\';
                }
            }
            found
        };
        if escapes_newline || self.pattern.contains(['\n', '\r']) {
            return Err(QueryError::Multiline);
        }
        let re = build_regex(&self.pattern, self.insensitive())?;
        Ok(Some(self.regex_matcher(re, true)))
    }

    /// 行ごとに正規表現 `re` で探す照合器。`expand` が偽 (リテラルをエスケープしたもの) なら置換文字列を展開しない。
    fn regex_matcher(&self, re: regex::Regex, expand: bool) -> Matcher {
        Matcher {
            finder: memmem::Finder::new(b"").into_owned(),
            len: 0,
            fold: false,
            chunked: false,
            regex: Some(re),
            expand,
            whole_word: self.whole_word,
            scope: self.scope.clone(),
        }
    }

    /// リテラルの照合器。経路は §12.4 の 1〜3 の順に選び、どの経路でも大小を区別しないときの意味は
    /// Unicode の単純ケースフォールディング (`regex` の `(?i)` と同じ) になる。
    fn compile_literal(&self) -> Result<Matcher, QueryError> {
        // 1. 大小の区別がある文字を 1 つも含まない検索語 (数字・記号・かな・漢字だけ等) は、
        //    区別しない設定でも区別する設定と同じなので、そのまま memmem で探す
        let fold = self.insensitive() && self.pattern.chars().any(is_cased);
        // 2. ASCII だけで `k` `K` `s` `S` を含まない検索語は、チャンクごとに ASCII を畳み込んで memmem できる。
        //    `k` は K (U+212A KELVIN SIGN)、`s` は ſ (U+017F LATIN SMALL LETTER LONG S) が畳み込まれて一致するので外す
        //    (ASCII 以外から ASCII へ畳み込まれる文字はこの 2 つだけ)。単語単位は一致の両端を行の文脈で調べるので外す
        let ascii_fold = self.pattern.is_ascii()
            && !self.pattern.contains(['k', 'K', 's', 'S'])
            && !self.whole_word;
        if fold && !ascii_fold {
            // 3. それ以外の大小を区別しない検索語は、エスケープして行ごとの正規表現の経路で `(?i)` として探す
            let re = build_regex(&regex::escape(&self.pattern), true)?;
            return Ok(self.regex_matcher(re, false));
        }
        let needle = if fold {
            self.pattern.to_ascii_lowercase()
        } else {
            self.pattern.clone()
        };
        Ok(Matcher {
            finder: memmem::Finder::new(needle.as_bytes()).into_owned(),
            len: needle.len(),
            fold,
            chunked: !self.whole_word,
            regex: None,
            expand: false,
            whole_word: self.whole_word,
            scope: self.scope.clone(),
        })
    }
}

/// 正規表現をコンパイルする (大きさの上限つき、§12.3)。
fn build_regex(pattern: &str, insensitive: bool) -> Result<regex::Regex, QueryError> {
    regex::RegexBuilder::new(pattern)
        .case_insensitive(insensitive)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| match e {
            regex::Error::CompiledTooBig(_) => QueryError::TooLarge,
            regex::Error::Syntax(msg) => QueryError::Syntax(first_line(&msg)),
            _ => QueryError::Syntax(e.to_string()),
        })
}

/// `regex` の構文エラーの文言は複数行になる。最後の `error:` の行 (原因) だけを取る。
fn first_line(msg: &str) -> String {
    msg.lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("error: "))
        .map_or_else(
            || msg.lines().next().unwrap_or("").to_owned(),
            str::to_owned,
        )
}

/// 小文字が 1 文字になる文字だけ小文字にする (`Smart` の大文字の判定用)。
fn lower_char(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// 大小の区別がある文字か (§12.4 の 1 の判定)。小文字・大文字に加えて、大小の対応のある文字 (タイトルケースの `ǅ` など)
/// も含める。多めに含めても遅い経路に回るだけで、結果は変わらない。
pub(super) fn is_cased(c: char) -> bool {
    fn same(mut it: impl Iterator<Item = char>, c: char) -> bool {
        it.next() == Some(c) && it.next().is_none()
    }
    c.is_lowercase() || c.is_uppercase() || !same(c.to_lowercase(), c) || !same(c.to_uppercase(), c)
}

/// コンパイル済みの検索語。
pub struct Matcher {
    pub(super) finder: memmem::Finder<'static>,
    /// memmem で探す検索語 (大小を畳み込むときは ASCII の小文字にしたもの) のバイト長。
    pub(super) len: usize,
    /// ASCII の大小を畳み込んで memmem で探す (§12.4 の 2。検索語は ASCII だけで `k` `K` `s` `S` を含まない)。
    pub(super) fold: bool,
    /// チャンクごとに走査できる (行に切り出さなくてよい)。
    pub(super) chunked: bool,
    /// 正規表現のとき。行ごとに実行する (§12.3)。大小を区別しないリテラルの一部もエスケープしてここで探す (§12.4 の 3)。
    pub(super) regex: Option<regex::Regex>,
    /// 置換文字列の `$1` / `${name}` を展開する (利用者の正規表現のときだけ。リテラルをエスケープしたものは展開しない)。
    expand: bool,
    /// 一致の両端が単語境界にあるものだけを採る。
    whole_word: bool,
    /// この範囲のどれかに収まる一致だけを採る (選択範囲内の検索)。
    scope: Option<Arc<[ByteRange]>>,
}

impl Matcher {
    /// 一致 `r` が範囲内か (範囲が無ければ常に真)。範囲の境界をまたぐ一致は採らない。
    pub(super) fn in_scope(&self, r: ByteRange) -> bool {
        let Some(sc) = &self.scope else {
            return true;
        };
        let i = sc.partition_point(|s| s.end < r.end);
        sc.get(i)
            .is_some_and(|s| s.start <= r.start && r.end <= s.end)
    }

    /// チャンクごとに (memmem で) 走査できる。`false` は行ごとに切り出す遅い経路になる。
    pub fn is_chunked(&self) -> bool {
        self.chunked
    }

    /// 置換文字列の `$1` / `${name}` を一致ごとに展開する (利用者の正規表現のとき)。
    pub(super) fn expands(&self) -> bool {
        self.expand
    }

    /// 1 行の中の一致を左から順に `f` へ渡す。`line_start` は行頭の文書内バイト位置。
    pub(super) fn find_in_line(
        &self,
        text: &dyn TextRead,
        line_start: usize,
        line: &str,
        f: &mut dyn FnMut(ByteRange) -> bool,
    ) -> bool {
        self.find_in_line_inner(text, line_start, line, false, &mut |r, _| f(r))
    }

    /// `find_in_line` の本体。`want_caps` が真で利用者の正規表現なら、一致ごとにキャプチャも渡す (置換文字列の展開用)。
    pub(super) fn find_in_line_inner(
        &self,
        text: &dyn TextRead,
        line_start: usize,
        line: &str,
        want_caps: bool,
        f: &mut dyn FnMut(ByteRange, Option<&regex::Captures<'_>>) -> bool,
    ) -> bool {
        // 単語単位: 一致の両端が行の単語境界 (行頭からのバイト) にあること
        let bounds = self.whole_word.then(|| word_boundaries(line));
        let on_word_boundary = |rel_s: usize, rel_e: usize| {
            bounds
                .as_ref()
                .is_none_or(|b| b.binary_search(&rel_s).is_ok() && b.binary_search(&rel_e).is_ok())
        };
        if let Some(re) = &self.regex {
            let want_caps = want_caps && self.expand;
            let mut pos = 0;
            while pos <= line.len() {
                let (s, e, caps) = if want_caps {
                    let Some(c) = re.captures_at(line, pos) else {
                        break;
                    };
                    let Some(whole) = c.get(0) else {
                        break;
                    };
                    (whole.start(), whole.end(), Some(c))
                } else {
                    let Some(m) = re.find_at(line, pos) else {
                        break;
                    };
                    (m.start(), m.end(), None)
                };
                // 空の一致は採らず、1 文字進める
                if s == e {
                    pos = e + line[e..].chars().next().map_or(1, char::len_utf8);
                    continue;
                }
                let (so, eo) = (line_start + s, line_start + e);
                if is_grapheme_boundary(text, ByteOff(so))
                    && is_grapheme_boundary(text, ByteOff(eo))
                    && on_word_boundary(s, e)
                {
                    if !f(ByteRange::new(ByteOff(so), ByteOff(eo)), caps.as_ref()) {
                        return false;
                    }
                    pos = e;
                } else {
                    // 採らなかった候補と重なる位置の一致を取りこぼさないよう、先頭の次の char 境界から探し直す
                    pos = s + line[s..].chars().next().map_or(1, char::len_utf8);
                }
            }
            return true;
        }
        // ASCII の大小だけを畳み込む (バイト長は変わらないので、位置はそのまま元の行のもの)
        let hay = if self.fold && line.bytes().any(|b| b.is_ascii_uppercase()) {
            Cow::Owned(line.to_ascii_lowercase())
        } else {
            Cow::Borrowed(line)
        };
        let hay = hay.as_bytes();
        let mut pos = 0;
        while pos <= hay.len() {
            let Some(i) = self.finder.find(&hay[pos..]) else {
                break;
            };
            let (s, e) = (pos + i, pos + i + self.len);
            let (so, eo) = (line_start + s, line_start + e);
            if is_grapheme_boundary(text, ByteOff(so))
                && is_grapheme_boundary(text, ByteOff(eo))
                && on_word_boundary(s, e)
            {
                if !f(ByteRange::new(ByteOff(so), ByteOff(eo)), None) {
                    return false;
                }
                pos = e;
            } else {
                // 書記素の途中や単語の途中に当たった候補は捨て、その先頭の次の char 境界から探し直す
                pos = s + char_len(&hay[s..]);
            }
        }
        true
    }
}

fn char_len(b: &[u8]) -> usize {
    match b.first() {
        Some(&x) if x >= 0xF0 => 4,
        Some(&x) if x >= 0xE0 => 3,
        Some(&x) if x >= 0xC0 => 2,
        _ => 1,
    }
}
