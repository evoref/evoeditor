//! 置換 (置換の文字列の解釈・置換の候補の走査・すべて置換の ChangeSet、docs/03 §12.5)。

use eve_text::{ByteOff, ByteRange, TextRead};

use crate::jobs::cancel::CancelToken;

use super::matcher::Matcher;
use super::matcher::SearchQuery;
use super::scan::scan_bytes_with_captures;

/// `range` (行頭と行末に揃った範囲) を走査の入口 (`scan_bytes` と同じ本体、03 §12.3 の「走査の境界」) で走査し、一致と
/// その置換文字列を `f` へ渡す。正規表現では `replacement` の `$1` / `${name}` を一致ごとに展開し (`$$` は `$`)、リテラルでは
/// そのまま渡す。`f` が `false` を返すか取り消されたら止めて `false`。
pub fn scan_replacements(
    text: &dyn TextRead,
    m: &Matcher,
    range: ByteRange,
    cancel: &CancelToken,
    replacement: &str,
    f: &mut dyn FnMut(ByteRange, String) -> bool,
) -> bool {
    scan_bytes_with_captures(text, m, range, cancel, &mut |r, caps| {
        let mut out = String::new();
        match caps {
            Some(c) => c.expand(replacement, &mut out),
            None => out.push_str(replacement),
        }
        f(r, out)
    })
}

/// 一致 `range` の置換文字列。正規表現では `$1` / `${name}` を展開する。`range` が一致でなければ `None`
/// (1 件置換が、選択範囲が一致そのものかを確かめるのにも使う)。`range` の始端を含む行を走査の入口で探す
/// (一致は行をまたがない。行をまたぎうる検索語の確かめ方は M24 の (3) で正本に合わせる、03 §12.3.1)。
/// 照合器を作るので、正規表現では `bg` のジョブが照合器を作った後 (`SearchQuery::prepared` が `Ready`) に呼ぶ。
pub fn replacement_for(
    text: &dyn TextRead,
    query: &SearchQuery,
    range: ByteRange,
    replacement: &str,
) -> Option<String> {
    let m = query.matcher()?;
    let line = text.byte_to_line(range.start);
    let start = text.line_to_byte(line);
    let end = if line.0 + 1 < text.len_lines() {
        text.line_to_byte(eve_text::LineIdx(line.0 + 1))
    } else {
        ByteOff(text.len_bytes())
    };
    let mut found = None;
    scan_replacements(
        text,
        &m,
        ByteRange::new(start, end),
        &CancelToken::new(),
        replacement,
        &mut |r, s| {
            if r == range {
                found = Some(s);
                return false;
            }
            r.start < range.start
        },
    );
    found
}

/// 正規表現の置換文字列の `\n` を改行 (`eol`) に、`\\` を `\` に直す (03 §12.5)。ほかの `\` はそのまま。
pub fn unescape_replacement(s: &str, eol: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match (c, it.peek()) {
            ('\\', Some('n')) => {
                it.next();
                out.push_str(eol);
            }
            ('\\', Some('\\')) => {
                it.next();
                out.push('\\');
            }
            _ => out.push(c),
        }
    }
    out
}

/// すべて置換の `ChangeSet` を作る (一致を保持せず、走査しながら直接組み立てる、03 §12.5)。
/// 一致の選び方は検索と同じ。返すのは (`ChangeSet`, 置換した件数)。取り消されたら `None`。
pub fn build_replace_all(
    text: &dyn TextRead,
    query: &SearchQuery,
    replacement: &str,
    cancel: &CancelToken,
) -> Option<(eve_text::ChangeSet, u64)> {
    let m = query.matcher()?;
    let mut edits: Vec<eve_text::Edit> = Vec::new();
    let full = ByteRange::new(ByteOff(0), ByteOff(text.len_bytes()));
    // 正規表現では `$1` / `${name}` を一致ごとに展開する (03 §12.5)。リテラルは置換の文字列をそのまま入れる
    let insert: std::sync::Arc<str> = std::sync::Arc::from(replacement);
    let ok = scan_replacements(text, &m, full, cancel, replacement, &mut |r, s| {
        edits.push(eve_text::Edit {
            range: r,
            insert: if m.expands() {
                std::sync::Arc::from(s)
            } else {
                insert.clone()
            },
        });
        true
    });
    if !ok {
        return None;
    }
    let n = edits.len() as u64;
    let cs = eve_text::ChangeSet::from_sorted_edits(text, edits).ok()?;
    Some((cs, n))
}
