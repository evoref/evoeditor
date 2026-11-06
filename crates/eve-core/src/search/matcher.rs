//! 検索語の解釈と照合 (`SearchQuery` → `Matcher`、docs/03 §12.1・§12.3・§12.4)。
//!
//! リテラル (memmem)・大小を区別しない照合 (1 文字ずつの小文字への畳み込み)・行単位の正規表現の 3 つの経路を
//! `Matcher` に閉じる。

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use eve_text::{ByteOff, ByteRange, TextRead, is_grapheme_boundary, word_boundaries};
use memchr::memmem;

/// 大文字小文字の扱い (FR-402)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// 検索語に大文字を含めば区別し、含まなければ区別しない。
    Smart,
    Sensitive,
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

    pub fn is_regex(&self) -> bool {
        self.regex
    }

    pub fn is_whole_word(&self) -> bool {
        self.whole_word
    }

    pub fn with_case(mut self, case: Case) -> SearchQuery {
        self.case = case;
        self.compiled = Compiled::default();
        self
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

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
            return self.pattern.chars().any(|c| fold_char(c) != c);
        }
        let mut escaped = false;
        for c in self.pattern.chars() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if fold_char(c) != c {
                return true;
            }
        }
        false
    }

    /// 照合器。検索語が空・コンパイルできない・改行にまたがるなら `None` (何にも一致しない)。
    /// 理由が要るときは `compile` を使う。
    pub fn matcher(&self) -> Option<Arc<Matcher>> {
        self.compile().ok().flatten()
    }

    /// 照合器を作る。検索語が空なら `Ok(None)`。正規表現の構文エラーなどは `Err`。
    /// 結果は覚えて、この問い合わせとその複製で使い回す (同じ条件でコンパイルし直さない、PRF-09)。
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
        Ok(Some(self.compile_literal()))
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
        let insensitive = match self.case {
            Case::Sensitive => false,
            Case::Insensitive => true,
            Case::Smart => !self.smart_has_upper(),
        };
        let re = regex::RegexBuilder::new(&self.pattern)
            .case_insensitive(insensitive)
            .size_limit(REGEX_SIZE_LIMIT)
            .build()
            .map_err(|e| match e {
                regex::Error::CompiledTooBig(_) => QueryError::TooLarge,
                regex::Error::Syntax(msg) => QueryError::Syntax(first_line(&msg)),
                _ => QueryError::Syntax(e.to_string()),
            })?;
        Ok(Some(Matcher {
            finder: memmem::Finder::new(b"").into_owned(),
            len: 0,
            fold: false,
            chunked: false,
            regex: Some(re),
            whole_word: self.whole_word,
            scope: self.scope.clone(),
        }))
    }

    fn compile_literal(&self) -> Matcher {
        // 大小の区別がある文字 (小文字・大文字) を 1 つも含まない検索語 (数字・記号・かな・漢字だけ等) は、
        // 区別しない設定でも区別する設定と同じなので、そのまま memmem で探す (§12.4 の 1)
        let cased = self
            .pattern
            .chars()
            .any(|c| c.is_lowercase() || c.is_uppercase());
        let fold = cased
            && match self.case {
                Case::Sensitive => false,
                Case::Insensitive => true,
                Case::Smart => !self.pattern.chars().any(|c| fold_char(c) != c),
            };
        let needle = if fold {
            self.pattern.chars().map(fold_char).collect::<String>()
        } else {
            self.pattern.clone()
        };
        // ASCII だけで `k` を含まない検索語は、チャンクごとに ASCII を畳み込んで memmem できる (§12.4 の 2)。
        // `k` は K (U+212A KELVIN SIGN) が畳み込まれて一致するので、行ごとの経路にする
        // 単語単位は一致の両端を行の文脈で調べるので、行ごとの経路にする
        let chunked = (!fold || (needle.is_ascii() && !needle.contains('k'))) && !self.whole_word;
        Matcher {
            finder: memmem::Finder::new(needle.as_bytes()).into_owned(),
            len: needle.len(),
            fold,
            chunked,
            regex: None,
            whole_word: self.whole_word,
            scope: self.scope.clone(),
        }
    }
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

/// 小文字が 1 文字になる文字だけ畳み込む。
fn fold_char(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// 畳み込んだ文字列と、畳み込み後のバイト位置 → 元のバイト位置の表 (長さが変わらなければ `None`)。
fn fold_line(line: &str) -> (Cow<'_, str>, Option<Vec<usize>>) {
    if line.is_ascii() {
        return if line.bytes().any(|b| b.is_ascii_uppercase()) {
            (Cow::Owned(line.to_ascii_lowercase()), None)
        } else {
            (Cow::Borrowed(line), None)
        };
    }
    let mut out = String::with_capacity(line.len());
    let mut map: Vec<usize> = Vec::with_capacity(line.len() + 1);
    let mut same = true;
    for (i, c) in line.char_indices() {
        let f = fold_char(c);
        same &= f.len_utf8() == c.len_utf8();
        for _ in 0..f.len_utf8() {
            map.push(i);
        }
        out.push(f);
    }
    map.push(line.len());
    (Cow::Owned(out), (!same).then_some(map))
}

/// コンパイル済みの検索語。
pub struct Matcher {
    pub(super) finder: memmem::Finder<'static>,
    /// 畳み込み後の検索語のバイト長。
    pub(super) len: usize,
    pub(super) fold: bool,
    /// チャンクごとに走査できる (行ごとの畳み込みが要らない)。
    pub(super) chunked: bool,
    /// 正規表現のとき。行ごとに実行する (§12.3)。
    pub(super) regex: Option<regex::Regex>,
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

    /// チャンクごとに (memmem で) 走査できる。`false` は行ごとに畳み込む遅い経路 (約 50 MB/s) になる。
    pub fn is_chunked(&self) -> bool {
        self.chunked
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

    /// `find_in_line` の本体。`want_caps` が真で正規表現なら、一致ごとにキャプチャも渡す (置換文字列の展開用)。
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
        let (hay, map) = if self.fold {
            fold_line(line)
        } else {
            (Cow::Borrowed(line), None)
        };
        let hay = hay.as_bytes();
        let orig = |i: usize| map.as_ref().map_or(i, |m| m[i]);
        let mut pos = 0;
        while pos <= hay.len() {
            let Some(i) = self.finder.find(&hay[pos..]) else {
                break;
            };
            let (s, e) = (pos + i, pos + i + self.len);
            let (so, eo) = (line_start + orig(s), line_start + orig(e));
            if is_grapheme_boundary(text, ByteOff(so))
                && is_grapheme_boundary(text, ByteOff(eo))
                && on_word_boundary(orig(s), orig(e))
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
