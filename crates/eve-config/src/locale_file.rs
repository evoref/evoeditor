//! ユーザーの言語ファイル (`<config_dir>/locales/<lang>.toml`) の検査と、同梱の表への重ね合わせ (docs/07 §17.4、OQ-220)。
//!
//! 言語ファイルを位置付きで読み (診断に行を付ける)、キーごとに 07 §17.4 の検査の表 (1〜9) を掛ける。検査 1〜8 に通らない
//! 文言は使わず (補う言語の同梱の文言が残る)、検査 9 (ヒントの幅) に通らない文言は警告して使う。ファイル全体の問題
//! (1 MiB を超える・UTF-8 でない・書ける構造の上限を超える・TOML の構文エラー) は、エラーの診断 1 件にしてファイル全体を
//! 使わない。どれでもパニックせず、メモリと時間はファイルの大きさに比例する量に収める (NFR-305 / NFR-503)。
//!
//! 入れ子の深さ: TOML パーサの既定の上限 (80) は、インラインの表・配列の入れ子と、1 つの点のキーの段数を別々に数えるので、
//! 組み合わせると数千段の入れ子を作れる (パーサと木の再帰で、1 MiB のスタックを使い切る)。言語ファイルにはインラインの表も
//! 配列も要らないので、構文を読む前の字句の走査 (`scan_structure`) で見つけたらファイル全体の問題にする。同じ走査で、
//! 表の見出しと点のキーを合わせた段数が 8 段を超えるキーと、キーの段の総数が 20,000 を超えるファイルも拒み、入れ子の
//! 深さを 8 段に、パーサの木の大きさ (表とキーの数) を抑える。
//!
//! ファイルを開くこと (候補の一覧・通常のファイルかの確認・1 MiB + 1 バイトまでの読込) は bin が行う。ここは読んだ
//! バイト列だけを受け取る (eve-config は I/O をしない)。検査の関数 [`check_text`] は `cargo xtask i18n` も使う
//! (同梱の言語ファイルに検査 2〜8 と、エラーとしての検査 9 を課す)。

use std::collections::{BTreeMap, BTreeSet};

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::chars::{CharClass, forbidden_char, is_combining_mark, is_joiner};
use crate::{Catalog, DiagKind, Diagnostic, Severity};

/// 言語ファイルの大きさの上限 (バイト。07 §17.4。bin は 1 バイト多く読み、超えたかを見分ける)。
pub const LOCALE_MAX_BYTES: usize = 1024 * 1024;
/// 1 つの文言の長さの上限 (Unicode のスカラー値の数。検査 2)。
pub const LOCALE_MAX_TEXT_CHARS: usize = 1024;
/// 続けて書ける結合文字 (Mn・Me) の数の上限 (検査 5。Stream-Safe Text Format の上限)。
pub const LOCALE_MAX_MARKS: usize = 30;
/// ヒント (`hint.*`) の表示幅の上限 (セル。検査 9)。
pub const HINT_MAX_WIDTH: usize = 8;
/// 1 つの言語ファイルの診断の件数の上限 (07 §17.4。超えた分は「ほか N 件」の 1 件にまとめる)。
pub const LOCALE_MAX_DIAGNOSTICS: usize = 100;
/// 言語ファイルに書けない (eve-host / bin の定数表に置く) キーの接頭辞 (07 §17.1・§17.4)。
pub const LOCALE_RESERVED_PREFIXES: [&str; 3] = ["stderr.", "panic.", "cli."];

/// 1 つのキーの段数 (表の見出しと点のキーの段の和) の上限。同梱のキーは 5 段まで。これを超えるキーのあるファイルは、
/// 構文を読む前に拒む (深い表を数多く作るファイルで、パーサの木の量と深さを抑える)。
const MAX_KEY_DEPTH: usize = 8;

/// 1 つのファイルのキーの段の総数 (表の見出しと、キーごとの点のキーの段数の和) の上限。同梱の全訳を点のキーだけで
/// 書いても約 3,200 段。これを超えるファイルは、構文を読む前に拒む (TOML パーサの木は 1 つの表・キーに数百バイトを使うので、
/// 1 MiB のファイルで数百 MiB を使わないように、表とキーの数を抑える)。
const MAX_KEY_SEGMENTS: usize = 20_000;

/// 組み立てる平らなキーの長さの上限 (バイト)。同梱のキーはこれよりずっと短いので、超えたキーは同梱に無いキーとして扱い、
/// 診断にはここまでを残す (長いキーの表の下に葉を並べたファイルで、キーの文字列の量を際限なく増やさないため)。
const MAX_KEY_BYTES: usize = 256;

/// 言語ファイルの問題の種類 (07 §17.4。文言のキーは `msg.config.<message_name()>`、04 §15.2 の案)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocaleProblem {
    /// ファイル全体: TOML の構文エラー (エラー)。
    Syntax {
        /// TOML パーサの説明文 (ファイル由来の文字列を含み得る。表示の前に無害化する)。
        message: String,
    },
    /// ファイル全体: UTF-8 でない (エラー)。位置は最初の不正なバイト。
    NotUtf8,
    /// ファイル全体: 1 MiB を超える (エラー)。
    TooLarge,
    /// ファイル全体: インラインの表 (`{ … }`) か配列 (`[ … ]`・`[[ … ]]`) がある (エラー)。位置は最初の `{` / `[`。
    InlineValue,
    /// ファイル全体: 表の見出しと点のキーを合わせた段数が上限 (8 段) を超えるキーがある (エラー)。位置はその行の
    /// 見出しかキーの先頭。
    TooDeep,
    /// ファイル全体: キーの段の総数が上限 (20,000) を超える (エラー)。位置は上限を超えた行の見出しかキーの先頭。
    TooManyKeys,
    /// `[meta] language` がファイル名の言語と違う (言語タグの形でない値を含む)。`Diagnostic.value` は書かれた値。
    /// ファイル名の言語として読む。
    LanguageMismatch {
        /// ファイル名の言語。
        lang: String,
    },
    /// 補う言語の同梱の言語ファイルに無いキー。候補は付けない。
    UnknownKey,
    /// 上書きできないキー (`stderr.*` / `panic.*` / `cli.*`)。
    Reserved,
    /// 平らにしたキーの重複。最初に現れたものだけを見て (それが検査に通らなければ同梱の文言)、この文言は使わない。
    DuplicateKey,
    /// 検査 2: 長すぎる。
    TooLong {
        /// 文言の文字数 (スカラー値の数)。
        count: usize,
    },
    /// 検査 3: 目に見える文字が無い (〔 〕 を中身ごと消した形で)。
    Empty,
    /// 検査 4・5: 使えない文字、または結合文字が続きすぎる。
    BadChar {
        /// 理由の種類。
        class: CharClass,
        /// 最初の問題の文字 (検査 5 では上限を超えた結合文字) のコードポイント。
        code: u32,
    },
    /// 検査 6・7: 括弧の対応、プレースホルダの名前と数 (〔 〕 の中と外のそれぞれ)、〔 〕 の有無が合わない。
    Placeholder {
        /// 同梱の文言のプレースホルダの名前 (現れる順。同じ名前は現れる数だけ並ぶ)。
        expected: Vec<String>,
    },
    /// 検査 8: `[X]` の並び・位置が同梱の文言と違う、または全角の似た形を含む。
    Accelerator,
    /// 検査 9: ヒントの表示幅が上限を超える (警告して、そのまま使う)。
    HintWidth {
        /// 表示幅 (セル)。
        width: usize,
    },
    /// 診断の件数の上限を超えた分の要約 (「ほか N 件」)。
    More {
        /// 省いた件数。
        count: usize,
    },
}

impl LocaleProblem {
    /// 文言のキーの種別の部分 (`msg.config.<これ>.text` / `.hint`。04 §15.2 の案)。
    pub fn message_name(&self) -> &'static str {
        match self {
            LocaleProblem::Syntax { .. }
            | LocaleProblem::NotUtf8
            | LocaleProblem::InlineValue
            | LocaleProblem::TooDeep
            | LocaleProblem::TooManyKeys => "locale_syntax",
            LocaleProblem::TooLarge => "locale_too_large",
            LocaleProblem::LanguageMismatch { .. } => "locale_language_mismatch",
            LocaleProblem::UnknownKey => "locale_unknown_key",
            LocaleProblem::Reserved => "locale_reserved",
            LocaleProblem::DuplicateKey => "locale_duplicate_key",
            LocaleProblem::TooLong { .. } => "locale_too_long",
            LocaleProblem::Empty => "locale_empty",
            LocaleProblem::BadChar { .. } => "locale_control_char",
            LocaleProblem::Placeholder { .. } => "locale_placeholder",
            LocaleProblem::Accelerator => "locale_accelerator",
            LocaleProblem::HintWidth { .. } => "locale_hint_width",
            LocaleProblem::More { .. } => "locale_more",
        }
    }
}

/// 文言 1 つの検査の結果 (07 §17.4 の検査 2〜9)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextCheck {
    /// すべての検査に通った。
    Pass,
    /// 検査 9 に通らない: 警告して、そのまま使う。
    Warn(LocaleProblem),
    /// 検査 2〜8 のどれかに通らない: 使わない。
    Reject(LocaleProblem),
}

/// 文言 1 つを検査する (07 §17.4 の検査 2〜9)。`key` は平らにしたキー (検査 9 は `hint.*` だけ)、`builtin` は比べる
/// 補う言語の同梱の文言 (検査 7・8)。
///
/// 見る順は 2 → 4 → 6 → 5 → 3 → 7 → 8 → 9 で、最初に通らなかった検査だけを返す (キーごとの診断は 1 件)。検査 6 (括弧の
/// 対応) を 5・3 の前に置くのは、5・3・8 を 〔 〕 を解いた形でも行うため (〔 〕 の対応が分からないと形を作れない)。
/// 検査 3 を 4・5 の後に置くのは、使えない文字を含む文言を「空」ではなく「使えない文字」として知らせるため。
///
/// 〔 〕 (条件付きの文) は、描くときに中身ごと消えるか、括弧だけが消える。そのため検査 5・8 は、書いたままの形に加えて
/// 「〔 〕 を外して中身を残した形」と「〔 〕 を中身ごと消した形」(どちらもプレースホルダは空の文字列とみなす) でも行い、
/// 検査 3 は「〔 〕 を中身ごと消した形」で行う (〔 〕 の外のプレースホルダは、いつも値が入るので目に見えるとみなす)。
pub fn check_text(key: &str, text: &str, builtin: &str) -> TextCheck {
    // 2. 長さ (後の検査の量もこれで抑える)
    let count = text.chars().count();
    if count > LOCALE_MAX_TEXT_CHARS {
        return TextCheck::Reject(LocaleProblem::TooLong { count });
    }
    // 4. 文字の種類
    if let Some((class, c)) = text.chars().find_map(|c| forbidden_char(c).map(|k| (k, c))) {
        return TextCheck::Reject(LocaleProblem::BadChar {
            class,
            code: u32::from(c),
        });
    }
    // 6. 括弧の対応 (名前の比較の前に必ず行う)
    let base = Template::scan(builtin);
    let expected = || LocaleProblem::Placeholder {
        expected: base
            .as_ref()
            .map(|b| b.names.iter().map(|n| (*n).to_owned()).collect())
            .unwrap_or_default(),
    };
    let Some(user) = Template::scan(text) else {
        return TextCheck::Reject(expected());
    };
    // 5. 続く結合文字の数 (〔 〕・プレースホルダ・ZWJ / ZWNJ は数を戻さない)
    for form in [&user.unwrapped, &user.removed] {
        if let Some(c) = too_many_marks(form) {
            return TextCheck::Reject(LocaleProblem::BadChar {
                class: CharClass::CombiningMarks,
                code: u32::from(c),
            });
        }
    }
    // 3. 目に見える書記素を 1 つ以上含む (〔 〕 を中身ごと消した形で)
    // 空白でない ASCII の文字があれば目に見える (書記素に分けずに済む、よくある場合)
    if !user.outside_placeholder
        && !user.removed.chars().any(|c| c.is_ascii_graphic())
        && !eve_text::grapheme_indices(&user.removed)
            .iter()
            .any(|(_, g)| is_visible(g))
    {
        return TextCheck::Reject(LocaleProblem::Empty);
    }
    // 7. プレースホルダの多重集合 (〔 〕 の中と外のそれぞれ) と 〔 〕 の有無
    let fits = match &base {
        Some(b) => user.fits(b),
        None => user.names.is_empty() && !user.conditional,
    };
    if !fits {
        return TextCheck::Reject(expected());
    }
    // 8. `[X]` の並びと位置、全角の似た形 (3 つの形のそれぞれで同梱の同じ形と比べる)
    let base_forms: [&str; 3] = match &base {
        Some(b) => [builtin, &b.unwrapped, &b.removed],
        None => [builtin; 3],
    };
    let user_forms: [&str; 3] = [text, &user.unwrapped, &user.removed];
    for (mine, theirs) in user_forms.iter().zip(base_forms) {
        if accelerators(mine) != accelerators(theirs)
            || (starts_with_accelerator(theirs) && !mine.starts_with(first_chars(theirs, 3)))
            || has_fullwidth_accelerator(mine)
        {
            return TextCheck::Reject(LocaleProblem::Accelerator);
        }
    }
    // 9. ヒントの表示幅 (eve-text の表示幅。曖昧幅は 1)
    if key.starts_with("hint.") {
        let width = hint_width(text);
        if width > HINT_MAX_WIDTH {
            return TextCheck::Warn(LocaleProblem::HintWidth { width });
        }
    }
    TextCheck::Pass
}

/// ヒントの表示幅 (セル。検査 9 と、`cargo xtask i18n` の 04 §15.2 の目録の検査)。eve-text の表示幅 (03 §13.2) で、
/// 曖昧幅は 1、不可視文字 (ZWNJ など) は可視化の記号の幅で数える (クロームを描く core の `cell_text` と同じ設定)。
pub fn hint_width(text: &str) -> usize {
    eve_text::str_display_width(text, &eve_text::WidthConfig::default())
}

/// 目に見える書記素か (検査 3): 空白・ZWJ・ZWNJ だけでできておらず、不可視文字を除いた表示幅が 1 以上。
fn is_visible(g: &str) -> bool {
    let cfg = eve_text::WidthConfig {
        show_invisible: false,
        ..eve_text::WidthConfig::default()
    };
    !g.chars().all(|c| c.is_whitespace() || is_joiner(c)) && eve_text::grapheme_width(g, &cfg) >= 1
}

/// 結合文字が上限を超えて続くなら、超えた結合文字。ZWJ・ZWNJ は数えず、数も戻さない (書記素を区切る文字を挟んで
/// 数を戻す迂回を防ぐ。ほかの書式文字は検査 4 で拒む)。
fn too_many_marks(s: &str) -> Option<char> {
    let mut run = 0usize;
    for c in s.chars() {
        if is_joiner(c) {
            continue;
        }
        if is_combining_mark(c) {
            run += 1;
            if run > LOCALE_MAX_MARKS {
                return Some(c);
            }
        } else {
            run = 0;
        }
    }
    None
}

/// 文言の括弧の構造 (検査 6)・プレースホルダの名前 (検査 7) と、〔 〕 を解いた 2 つの形 (検査 3・5・8)。読み方は
/// `Catalog::render` と同じ: `{{` と `}}` はエスケープ、`{name}` はプレースホルダ (`{key:<command_id>}` を含む)、
/// 〔 〕 は条件付きの文。
struct Template<'a> {
    /// すべてのプレースホルダの名前 (現れる順)。
    names: Vec<&'a str>,
    /// 〔 〕 の外のプレースホルダの名前。
    outside: Vec<&'a str>,
    /// 〔 〕 の中のプレースホルダの名前。
    inside: Vec<&'a str>,
    /// 〔 〕 があるか。
    conditional: bool,
    /// 〔 〕 の外にプレースホルダがあるか。
    outside_placeholder: bool,
    /// 〔 〕 を外して中身を残した形 (プレースホルダは空、エスケープは解いた文字)。
    unwrapped: String,
    /// 〔 〕 を中身ごと消した形 (プレースホルダは空、エスケープは解いた文字)。
    removed: String,
}

impl<'a> Template<'a> {
    /// 括弧が対応し入れ子が無ければ、その構造。対応しない `{` / `}`・入れ子の `{`・対応しない 〔 〕・入れ子の 〔 なら `None`。
    fn scan(s: &'a str) -> Option<Template<'a>> {
        let mut t = Template {
            names: Vec::new(),
            outside: Vec::new(),
            inside: Vec::new(),
            conditional: false,
            outside_placeholder: false,
            unwrapped: String::with_capacity(s.len()),
            removed: String::with_capacity(s.len()),
        };
        let mut in_cond = false;
        let mut it = s.char_indices().peekable();
        while let Some((i, c)) = it.next() {
            let literal = match c {
                '{' if it.peek().map(|&(_, n)| n) == Some('{') => {
                    it.next();
                    Some('{')
                }
                '{' => {
                    let start = i + 1;
                    loop {
                        match it.next() {
                            Some((j, '}')) => {
                                let name = &s[start..j];
                                t.names.push(name);
                                if in_cond {
                                    t.inside.push(name);
                                } else {
                                    t.outside.push(name);
                                    t.outside_placeholder = true;
                                }
                                break;
                            }
                            Some((_, '{')) | None => return None,
                            Some(_) => {}
                        }
                    }
                    None
                }
                '}' if it.peek().map(|&(_, n)| n) == Some('}') => {
                    it.next();
                    Some('}')
                }
                '}' => return None,
                '〔' if in_cond => return None,
                '〔' => {
                    in_cond = true;
                    t.conditional = true;
                    None
                }
                '〕' if !in_cond => return None,
                '〕' => {
                    in_cond = false;
                    None
                }
                other => Some(other),
            };
            if let Some(ch) = literal {
                t.unwrapped.push(ch);
                if !in_cond {
                    t.removed.push(ch);
                }
            }
        }
        (!in_cond).then_some(t)
    }

    /// 〔 〕 の外と中のそれぞれで、名前の集合が `base` と同じで、各名前の出現数が `base` 以下。〔 〕 の有無が同じ (検査 7)。
    fn fits(&self, base: &Template<'_>) -> bool {
        fn count<'s>(names: &[&'s str]) -> BTreeMap<&'s str, usize> {
            let mut m = BTreeMap::new();
            for n in names {
                *m.entry(*n).or_default() += 1;
            }
            m
        }
        fn within(mine: &[&str], theirs: &[&str]) -> bool {
            let (mine, theirs) = (count(mine), count(theirs));
            mine.keys().eq(theirs.keys())
                && mine
                    .iter()
                    .all(|(n, c)| theirs.get(n).is_some_and(|t| c <= t))
        }
        self.conditional == base.conditional
            && within(&self.outside, &base.outside)
            && within(&self.inside, &base.inside)
    }
}

/// 文言の `[X]` (英大文字 1 字の角括弧。確認ダイアログの選択肢のアクセラレータ、04 §15.2) の並び。
fn accelerators(s: &str) -> Vec<char> {
    if !s.contains('[') {
        return Vec::new();
    }
    let cs: Vec<char> = s.chars().collect();
    cs.windows(3)
        .filter(|w| w[0] == '[' && w[2] == ']' && w[1].is_ascii_uppercase())
        .map(|w| w[1])
        .collect()
}

fn starts_with_accelerator(s: &str) -> bool {
    let mut it = s.chars();
    matches!((it.next(), it.next(), it.next()), (Some('['), Some(c), Some(']')) if c.is_ascii_uppercase())
}

/// 先頭の `n` 文字。
fn first_chars(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(i, _)| &s[..i])
}

/// 全角の似た形 (`［` / `］` と、角括弧の中の全角の英字。`［Ｓ］`・`[Ｓ]` など) を含むか。
fn has_fullwidth_accelerator(s: &str) -> bool {
    if s.contains(['［', '］']) {
        return true;
    }
    if !s.contains('[') {
        return false;
    }
    let cs: Vec<char> = s.chars().collect();
    cs.windows(3)
        .any(|w| w[0] == '[' && w[2] == ']' && matches!(w[1], 'Ａ'..='Ｚ' | 'ａ'..='ｚ'))
}

/// 言語タグの形 (07 §17.4 の「値の形」: 2〜3 字の英字の言語と、`-` / `_` で続く 1〜8 字の英数字)。
fn is_language_tag(s: &str) -> bool {
    let mut parts = s.split(['-', '_']);
    let lang = parts.next().unwrap_or("");
    (2..=3).contains(&lang.len())
        && lang.bytes().all(|b| b.is_ascii_alphabetic())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// 読んだユーザーの言語ファイル。欄は読むだけ (検査していない文言を [`Catalog::overlay`] に渡せないようにする)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleFile {
    language: String,
    texts: BTreeMap<String, String>,
    diagnostics: Vec<Diagnostic>,
}

impl LocaleFile {
    /// ファイル名の言語 (`fr` など)。
    pub fn language(&self) -> &str {
        &self.language
    }

    /// 検査に通った文言 (検査 9 の警告を受けて使うものを含む)。平らにしたキー → 文言。
    pub fn texts(&self) -> &BTreeMap<String, String> {
        &self.texts
    }

    /// 問題 (書かれた順。件数の上限を超えた分は最後の「ほか N 件」の 1 件)。
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// 問題の列を取り出す。
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    /// ファイル全体の問題 (エラー) があって、ファイル全体を使わなかったか。
    pub fn is_unusable(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

/// ユーザーの言語ファイルの本文を読み、検査する。`language` はファイル名の言語、`bytes` は読んだ本文 (先頭の BOM は
/// 読み飛ばす)、`base` は補う言語の同梱の表 ([`Catalog::base_for`])、`max_diagnostics` は診断の件数の上限
/// (通常は [`LOCALE_MAX_DIAGNOSTICS`]。超えた分は「ほか N 件」の 1 件にまとめる)。重ねるのは [`Catalog::overlay`]。
pub fn load_locale_file(
    language: &str,
    bytes: &[u8],
    base: &Catalog,
    max_diagnostics: usize,
) -> LocaleFile {
    let mut out = LocaleFile {
        language: language.to_owned(),
        texts: BTreeMap::new(),
        diagnostics: Vec::new(),
    };
    let file_error = |kind: LocaleProblem, line: u32, column: u32| Diagnostic {
        severity: Severity::Error,
        kind: DiagKind::Locale(kind),
        key: String::new(),
        value: String::new(),
        line,
        column,
    };
    if bytes.len() > LOCALE_MAX_BYTES {
        out.diagnostics
            .push(file_error(LocaleProblem::TooLarge, 0, 0));
        return out;
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) => {
            let valid = &bytes[..e.valid_up_to()];
            // 有効な部分は UTF-8 なので、位置はその文字列で数える
            let head = std::str::from_utf8(valid).unwrap_or("");
            let (line, column) = Cursor::new(head).pos(head.len());
            out.diagnostics
                .push(file_error(LocaleProblem::NotUtf8, line, column));
            return out;
        }
    };
    // 構文を読む前に、入れ子を作るインラインの表と配列を拒む (入れ子の深さを抑える。モジュールの説明)
    if let Some((at, problem)) = scan_structure(text) {
        let (line, column) = Cursor::new(text).pos(at);
        out.diagnostics.push(file_error(problem, line, column));
        return out;
    }
    let root = match DeTable::parse(text) {
        Ok(t) => t,
        Err(e) => {
            let (line, column) = e.span().map_or((0, 0), |s| Cursor::new(text).pos(s.start));
            out.diagnostics.push(file_error(
                LocaleProblem::Syntax {
                    message: e.message().to_owned(),
                },
                line,
                column,
            ));
            return out;
        }
    };

    // 同梱のキーの最長。これより長いキーは同梱に無いので、文字列を組み立てずに決められる
    let base_max = base.keys().map(str::len).max().unwrap_or(0);
    let mut tree = Tree::default();
    for (k, v) in root.get_ref() {
        if k.get_ref().as_ref() == "meta" {
            let node = tree.node(None, k.get_ref());
            match v.get_ref() {
                DeValue::Table(t) => {
                    for (mk, mv) in t {
                        if mk.get_ref().as_ref() == "language" {
                            let n = tree.node(Some(node), mk.get_ref());
                            tree.leaf(n, mk.span().start, mv, LeafKind::MetaLanguage);
                        }
                    }
                }
                _ => tree.leaf(node, k.span().start, v, LeafKind::MetaNotTable),
            }
            continue;
        }
        tree.collect(None, k, v, base, base_max);
    }
    // 書かれた順 (位置の順) に見る: 重複は最初に現れたものを使い、診断も書かれた順に並べる
    tree.leaves.sort_by_key(|l| l.at);

    let mut sink = Sink {
        cursor: Cursor::new(text),
        out: Vec::new(),
        max: max_diagnostics,
        omitted: 0,
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for leaf in &tree.leaves {
        let value = leaf.value.get_ref();
        match leaf.kind {
            LeafKind::MetaNotTable => {
                let key = tree.key(leaf.node);
                sink.type_mismatch(key, "table", value, leaf.at);
            }
            LeafKind::MetaLanguage => match value {
                DeValue::String(s) => {
                    let written = s.as_ref();
                    let lang = written.split(['-', '_']).next().unwrap_or("");
                    if !is_language_tag(written) || !lang.eq_ignore_ascii_case(language) {
                        let key = tree.key(leaf.node);
                        sink.push(
                            Severity::Warning,
                            LocaleProblem::LanguageMismatch {
                                lang: language.to_owned(),
                            },
                            key,
                            written,
                            leaf.at,
                        );
                    }
                }
                other => {
                    let key = tree.key(leaf.node);
                    sink.type_mismatch(key, "string", other, leaf.at);
                }
            },
            LeafKind::Text => {
                let node = &tree.nodes[leaf.node];
                let problem = if tree.is_reserved(leaf.node) {
                    Some(LocaleProblem::Reserved)
                } else if node.over.is_some() || node.len > base_max {
                    Some(LocaleProblem::UnknownKey)
                } else {
                    None
                };
                if let Some(p) = problem {
                    // 診断を残せるときだけキーを組み立てる (残せない分は数えるだけ)
                    let key = if sink.has_room() {
                        tree.key(leaf.node)
                    } else {
                        String::new()
                    };
                    sink.push(Severity::Warning, p, key, "", leaf.at);
                    continue;
                }
                let key = tree.key(leaf.node);
                let problem = if !base.contains(&key) {
                    Some(LocaleProblem::UnknownKey)
                } else if !seen.insert(key.clone()) {
                    Some(LocaleProblem::DuplicateKey)
                } else {
                    None
                };
                if let Some(p) = problem {
                    sink.push(Severity::Warning, p, key, "", leaf.at);
                    continue;
                }
                let DeValue::String(s) = value else {
                    sink.type_mismatch(key, "string", value, leaf.at);
                    continue;
                };
                let text = s.as_ref();
                match check_text(&key, text, base.get(&key).unwrap_or("")) {
                    TextCheck::Pass => {
                        out.texts.insert(key, text.to_owned());
                    }
                    TextCheck::Warn(p) => {
                        sink.push(Severity::Warning, p, key.clone(), "", leaf.at);
                        out.texts.insert(key, text.to_owned());
                    }
                    TextCheck::Reject(p) => sink.push(Severity::Warning, p, key, "", leaf.at),
                }
            }
        }
    }
    out.diagnostics = sink.finish();
    out
}

/// 字句の走査で見ている行の部分。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    /// 行の先頭 (空白だけが前にある)。
    Line,
    /// 表の見出し `[ … ]` の中。
    Header,
    /// 見出しの後 (注釈だけが書ける)。
    AfterHeader,
    /// キー (`=` の前)。
    Key,
    /// 値 (`=` の後)。
    Value,
}

/// 構文を読む前の字句の走査 (07 §17.4 の「ファイル全体が使えないとき」)。文字列と注釈の外を見て、次の最初のものの
/// バイト位置と種類を返す。
/// - インラインの表の `{` と、表の見出しでない `[` (配列の値・配列の表 `[[`・見出しの中の `[`) → `InlineValue`。
///   表の見出しの `[` は、行の先頭の 1 つだけ。
/// - 表の見出しと点のキーを合わせた段数が [`MAX_KEY_DEPTH`] を超えるキー → `TooDeep` (位置はその行の見出しかキーの先頭)。
/// - キーの段の総数が [`MAX_KEY_SEGMENTS`] を超える → `TooManyKeys` (位置は超えた行の見出しかキーの先頭)。
///
/// 文字列は TOML の 4 種 (`"…"`・`'…'`・`"""…"""`・`'''…'''`) を読み飛ばす。閉じていない文字列や、ほかの書き方の誤りは、
/// この後でパーサが構文エラーにする (ここでは入れ子の深さと量を作る書き方だけを拒む)。
fn scan_structure(text: &str) -> Option<(usize, LocaleProblem)> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut part = Part::Line;
    let mut header_depth = 0usize;
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut segments = 0usize;
    while let Some(&c) = b.get(i) {
        match c {
            b'\n' => {
                part = Part::Line;
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
        if part == Part::Line {
            start = i;
            depth = 1;
            match c {
                b'[' if b.get(i + 1) == Some(&b'[') => {
                    return Some((i, LocaleProblem::InlineValue));
                }
                b'[' => {
                    part = Part::Header;
                    i += 1;
                    continue;
                }
                // この文字はキーの最初の文字として、下で読み直す
                _ => part = Part::Key,
            }
        }
        match c {
            b'"' | b'\'' => {
                i = skip_string(b, i);
                continue;
            }
            b'{' | b'[' => return Some((i, LocaleProblem::InlineValue)),
            b'.' if matches!(part, Part::Header | Part::Key) => depth += 1,
            b']' if part == Part::Header => {
                if depth > MAX_KEY_DEPTH {
                    return Some((start, LocaleProblem::TooDeep));
                }
                segments += depth;
                if segments > MAX_KEY_SEGMENTS {
                    return Some((start, LocaleProblem::TooManyKeys));
                }
                header_depth = depth;
                part = Part::AfterHeader;
            }
            b'=' if part == Part::Key => {
                if header_depth + depth > MAX_KEY_DEPTH {
                    return Some((start, LocaleProblem::TooDeep));
                }
                segments += depth;
                if segments > MAX_KEY_SEGMENTS {
                    return Some((start, LocaleProblem::TooManyKeys));
                }
                part = Part::Value;
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

/// 葉の種類。
#[derive(Clone, Copy)]
enum LeafKind {
    /// 文言 (`[meta]` の外のキー)。
    Text,
    /// `[meta] language`。
    MetaLanguage,
    /// `meta` が表でない。
    MetaNotTable,
}

/// 平らにする前の木の節 (親と、その中の名前)。キーの文字列は要る葉でだけ組み立てる。
struct Node<'a> {
    parent: Option<usize>,
    seg: &'a str,
    /// 根の節 (最上位のキー)。
    root: usize,
    /// 平らにしたキーのバイト数 (飽和させる)。
    len: usize,
    /// 根からたどって最初に [`MAX_KEY_BYTES`] を超えた節 (自分を含む)。キーはその節までで組み立てて切る。
    over: Option<usize>,
}

struct Leaf<'a, 'i> {
    node: usize,
    at: usize,
    value: &'a Spanned<DeValue<'i>>,
    kind: LeafKind,
}

#[derive(Default)]
struct Tree<'a, 'i> {
    nodes: Vec<Node<'a>>,
    leaves: Vec<Leaf<'a, 'i>>,
}

impl<'a, 'i> Tree<'a, 'i> {
    fn node(&mut self, parent: Option<usize>, seg: &'a str) -> usize {
        let index = self.nodes.len();
        let (root, len, over) = match parent.map(|p| &self.nodes[p]) {
            Some(p) => (
                p.root,
                p.len.saturating_add(1).saturating_add(seg.len()),
                p.over,
            ),
            None => (index, seg.len(), None),
        };
        let over = over.or((len > MAX_KEY_BYTES).then_some(index));
        self.nodes.push(Node {
            parent,
            seg,
            root,
            len,
            over,
        });
        index
    }

    fn leaf(&mut self, node: usize, at: usize, value: &'a Spanned<DeValue<'i>>, kind: LeafKind) {
        self.leaves.push(Leaf {
            node,
            at,
            value,
            kind,
        });
    }

    /// 表を葉まで降りる。同梱で葉のキーに当たる表は、その場所の葉として扱う (型が違う)。入れ子の深さは、構文を読む前の
    /// 字句の走査でインラインの表・配列と 8 段を超えるキーを拒んだので、8 段に収まる。
    fn collect(
        &mut self,
        parent: Option<usize>,
        k: &'a Spanned<toml::de::DeString<'i>>,
        v: &'a Spanned<DeValue<'i>>,
        base: &Catalog,
        base_max: usize,
    ) {
        let node = self.node(parent, k.get_ref());
        if let DeValue::Table(t) = v.get_ref() {
            let n = &self.nodes[node];
            let maybe_leaf = n.over.is_none() && n.len <= base_max;
            if !(maybe_leaf && base.contains(&self.key(node))) {
                for (ck, cv) in t {
                    self.collect(Some(node), ck, cv, base, base_max);
                }
                return;
            }
        }
        self.leaf(node, k.span().start, v, LeafKind::Text);
    }

    /// 上書きできない名前空間 (`stderr.*` / `panic.*` / `cli.*`) のキーか。根の名前だけで決める。
    fn is_reserved(&self, node: usize) -> bool {
        let root = self.nodes[node].root;
        let r = self.nodes[root].seg;
        (node != root
            && LOCALE_RESERVED_PREFIXES
                .iter()
                .any(|p| p.strip_suffix('.') == Some(r)))
            || LOCALE_RESERVED_PREFIXES.iter().any(|p| r.starts_with(p))
    }

    /// 節の平らなキー。上限を超えたキーは、最初に超えた節までを上限のバイト数で切る。たどる節は上限のバイト数 + 1 個まで
    /// (根の下の段は `.` で 1 バイト以上増える)、写すのは上限のバイト数までで、どちらもキーの長さに依らない。
    fn key(&self, node: usize) -> String {
        let end = self.nodes[node].over.unwrap_or(node);
        let mut segs = Vec::new();
        let mut cur = Some(end);
        while let Some(i) = cur {
            segs.push(self.nodes[i].seg);
            cur = self.nodes[i].parent;
        }
        let mut key = String::new();
        for (n, seg) in segs.iter().rev().enumerate() {
            if n > 0 {
                if key.len() >= MAX_KEY_BYTES {
                    break;
                }
                key.push('.');
            }
            let room = MAX_KEY_BYTES.saturating_sub(key.len());
            if seg.len() > room {
                let cut = (0..=room)
                    .rev()
                    .find(|&i| seg.is_char_boundary(i))
                    .unwrap_or(0);
                key.push_str(&seg[..cut]);
                break;
            }
            key.push_str(seg);
        }
        key
    }
}

/// バイト位置 → (行, 桁) を、前から順に数える (位置は減らない順に渡す。全体で本文の長さに比例する時間)。
struct Cursor<'t> {
    text: &'t str,
    off: usize,
    line: usize,
    col: usize,
}

impl<'t> Cursor<'t> {
    fn new(text: &'t str) -> Cursor<'t> {
        Cursor {
            text,
            off: 0,
            line: 1,
            col: 1,
        }
    }

    /// 行と桁 (どちらも 1 始まり、桁は文字数)。`at` が前に渡した位置より前なら、前の位置のまま。
    fn pos(&mut self, at: usize) -> (u32, u32) {
        let end = at.min(self.text.len());
        if end > self.off
            && let Some(part) = self.text.get(self.off..end)
        {
            for c in part.chars() {
                if c == '\n' {
                    self.line += 1;
                    self.col = 1;
                } else {
                    self.col += 1;
                }
            }
            self.off = end;
        }
        // 診断の位置の表示用。u32 を超える行・桁は上限で丸める
        (
            u32::try_from(self.line).unwrap_or(u32::MAX),
            u32::try_from(self.col).unwrap_or(u32::MAX),
        )
    }
}

/// 診断を件数の上限まで貯める。
struct Sink<'t> {
    cursor: Cursor<'t>,
    out: Vec<Diagnostic>,
    max: usize,
    omitted: usize,
}

impl Sink<'_> {
    /// 次の診断を残せるか (上限に達していれば、数えるだけ)。
    fn has_room(&self) -> bool {
        self.out.len() < self.max
    }

    fn push(&mut self, severity: Severity, p: LocaleProblem, key: String, value: &str, at: usize) {
        self.diag(severity, DiagKind::Locale(p), key, value, at);
    }

    /// 検査 1 (値が文字列) と `[meta]` の型。文言は既存の `msg.config.type_mismatch_no_default` (04 §15.2)。
    fn type_mismatch(&mut self, key: String, expected: &'static str, v: &DeValue<'_>, at: usize) {
        self.diag(
            Severity::Warning,
            DiagKind::TypeMismatch {
                expected,
                found: v.type_str(),
            },
            key,
            "",
            at,
        );
    }

    fn diag(&mut self, severity: Severity, kind: DiagKind, key: String, value: &str, at: usize) {
        if !self.has_room() {
            self.omitted += 1;
            return;
        }
        let (line, column) = self.cursor.pos(at);
        self.out.push(Diagnostic {
            severity,
            kind,
            key,
            value: value.to_owned(),
            line,
            column,
        });
    }

    fn finish(mut self) -> Vec<Diagnostic> {
        if self.omitted > 0 {
            self.out.push(Diagnostic {
                severity: Severity::Warning,
                kind: DiagKind::Locale(LocaleProblem::More {
                    count: self.omitted,
                }),
                key: String::new(),
                value: String::new(),
                line: 0,
                column: 0,
            });
        }
        self.out
    }
}

#[cfg(test)]
mod tests;
