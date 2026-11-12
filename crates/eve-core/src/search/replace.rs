//! 置換 (置換の文字列の解釈・置換の候補の走査・すべて置換の ChangeSet、docs/03 §12.5)。

use std::ops::Range;

use eve_text::{ByteOff, ByteRange, TextRead};

use crate::jobs::cancel::CancelToken;

use super::lines::for_each_line;
use super::matcher::Matcher;
use super::matcher::SearchQuery;
use super::scan::scan_bytes;
/// 行 `lines` を順に走査し、一致とその置換文字列を `f` へ渡す。正規表現では `replacement` の `$1` / `${name}` を
/// 一致ごとに展開し (`$$` は `$`)、リテラルではそのまま渡す。`f` が `false` を返すか取り消されたら止めて `false`。
pub fn scan_replacements(
    text: &dyn TextRead,
    m: &Matcher,
    lines: Range<usize>,
    cancel: &CancelToken,
    replacement: &str,
    f: &mut dyn FnMut(ByteRange, String) -> bool,
) -> bool {
    for_each_line(text, lines, &mut |ls, line| {
        if cancel.is_cancelled() {
            return false;
        }
        m.find_in_line_inner(text, ls, line, true, &mut |r, caps| {
            if !m.in_scope(r) {
                return true;
            }
            let mut out = String::new();
            match caps {
                Some(c) => c.expand(replacement, &mut out),
                None => out.push_str(replacement),
            }
            f(r, out)
        })
    })
}

/// 一致 `range` の置換文字列。正規表現では `$1` / `${name}` を展開する。`range` が一致でなければ `None`
/// (1 件置換が、選択範囲が一致そのものかを確かめるのにも使う)。
pub fn replacement_for(
    text: &dyn TextRead,
    query: &SearchQuery,
    range: ByteRange,
    replacement: &str,
) -> Option<String> {
    let m = query.matcher()?;
    let line = text.byte_to_line(range.start).0;
    let mut found = None;
    scan_replacements(
        text,
        &m,
        line..line + 1,
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
    let insert: std::sync::Arc<str> = std::sync::Arc::from(replacement);
    let mut edits: Vec<eve_text::Edit> = Vec::new();
    let full = ByteRange::new(ByteOff(0), ByteOff(text.len_bytes()));
    let ok = if m.expands() {
        // 正規表現: `$1` / `${name}` を一致ごとに展開する (03 §12.5)
        scan_replacements(
            text,
            &m,
            0..text.len_lines(),
            cancel,
            replacement,
            &mut |r, s| {
                edits.push(eve_text::Edit {
                    range: r,
                    insert: std::sync::Arc::from(s),
                });
                true
            },
        )
    } else {
        scan_bytes(text, &m, full, cancel, &mut |r| {
            edits.push(eve_text::Edit {
                range: r,
                insert: insert.clone(),
            });
            true
        })
    };
    if !ok {
        return None;
    }
    let n = edits.len() as u64;
    let cs = eve_text::ChangeSet::from_sorted_edits(text, edits).ok()?;
    Some((cs, n))
}
