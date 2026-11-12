//! 置換 (置換の文字列の解釈・置換の候補の走査・すべて置換の ChangeSet、docs/03 §12.5)。

use eve_text::{ByteOff, ByteRange, TextRead};

use crate::jobs::cancel::CancelToken;
use crate::time::Clock;

use super::matcher::Matcher;
use super::matcher::SearchQuery;
use super::scan::{ScanCtx, ScanEnd, scan_with_captures};

/// `range` を走査の入口 (`scan_bytes` と同じ本体、03 §12.3 の「走査の境界」) で走査し、一致とその置換文字列を `f` へ渡す。
/// 正規表現では `replacement` の `$1` / `${name}` を一致ごとに展開し (`$$` は `$`)、リテラルではそのまま渡す。終わり方
/// (`f` が `false` を返したら `Stopped`、取り消し・探さない・打ち切りはその理由) を返す。`ctx` の時計は行の窓の核の縮めと
/// 処理の速さの下限に使う (ジョブからは渡す、03 §12.3.1)。
pub fn scan_replacements(
    text: &dyn TextRead,
    m: &Matcher,
    range: ByteRange,
    ctx: &mut ScanCtx<'_>,
    replacement: &str,
    f: &mut dyn FnMut(ByteRange, String) -> bool,
) -> ScanEnd {
    scan_with_captures(text, m, range, ctx, &mut |r, caps| {
        let mut out = String::new();
        match caps {
            Some(c) => c.expand(replacement, &mut out),
            None => out.push_str(replacement),
        }
        f(r, out)
    })
}

/// 一致 `range` の置換文字列。正規表現では `$1` / `${name}` を展開する。`range` が一致でなければ `None`
/// (1 件置換が、選択範囲が一致そのものかを確かめるのにも使う)。`range` の始端を含む行を走査の入口で探す (行をまたがない
/// 検索語の規則、03 §12.5。行をまたぐ検索の規則の確かめ方は `replacement_at`)。
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
    let cancel = CancelToken::new();
    scan_replacements(
        text,
        &m,
        ByteRange::new(start, end),
        &mut ScanCtx::new(&cancel),
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

/// 1 件置換の対象の確かめとキャプチャの取り直し (03 §12.3.1 の「置換」): 走査の入口で `from` から選び始め、`range.start` で
/// 始まる一致が `range` そのものなら、その置換文字列 (正規表現では `$1` / `${name}` を展開する)。違えば `Ok(None)` (対象では
/// ない)。窓は走査と同じ作り方 (始端の前の 1 文字から、延ばす分 (k 行と最大の長さの小さいほう) と後ろの 1 文字まで。上限の
/// 無い検索語は文書の末尾まで) なので、`$`・`\z`・`\b` が窓の端で偽って成り立つことはない。
///
/// `from` は正本の位置にする: 行をまたぐ検索の規則では、正本と分かっている一致 (保持した一致・新しい入力中の現在の一致・
/// 次 / 前で移った一致) の始端 (一致は始まる位置で決まるので、その位置から探すと必ず最初に出る)。k が 0 の検索語では
/// `range.start` を含む行の行頭。重い検索語と上限の無い検索語はジョブ (`JobKind::ReplaceOne`) から時計を渡して呼ぶ。
///
/// # Errors
///
/// 取り消された (`ScanEnd::Cancelled`)、探さなかった (`TooLarge`)、重すぎて打ち切った (`TooHeavy`)。
pub fn replace_one(
    text: &dyn TextRead,
    m: &Matcher,
    from: ByteOff,
    range: ByteRange,
    replacement: &str,
    clock: Option<&dyn Clock>,
    cancel: &CancelToken,
) -> Result<Option<String>, ScanEnd> {
    if range.is_empty() || range.end.0 > text.len_bytes() || from > range.start {
        return Ok(None);
    }
    let mut found = None;
    let mut ctx = ScanCtx {
        cancel,
        clock,
        resume: None,
    };
    let end = scan_replacements(
        text,
        m,
        // 範囲の終わりは文書の末尾 (行の経路は範囲を行の境界に揃えて受ける)。`range.start` 以降の最初の一致で止める
        ByteRange::new(from, ByteOff(text.len_bytes())),
        &mut ctx,
        replacement,
        &mut |r, s| {
            if r.start < range.start {
                return true;
            }
            if r == range {
                found = Some(s);
            }
            false
        },
    );
    match end {
        ScanEnd::Done | ScanEnd::Stopped => Ok(found),
        e => Err(e),
    }
}

/// `replace_one` のジョブの本体: 照合器を作ってから呼ぶ。照合器を作れなければ `Ok(None)`。
///
/// # Errors
///
/// `replace_one` と同じ。
pub fn replace_one_job(
    text: &dyn TextRead,
    query: &SearchQuery,
    from: ByteOff,
    range: ByteRange,
    replacement: &str,
    clock: Option<&dyn Clock>,
    cancel: &CancelToken,
) -> Result<Option<String>, ScanEnd> {
    let Some(m) = query.matcher() else {
        return Ok(None);
    };
    replace_one(text, &m, from, range, replacement, clock, cancel)
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
/// 一致の選び方は検索と同じ (文書の先頭から選んだ正本)。返すのは (`ChangeSet`, 置換した件数)。取り消された・探さなかった・
/// 重すぎて打ち切ったら `None` (理由が要るときは `build_replace_all_in`)。時計を渡さないので、ジョブからは呼ばない。
pub fn build_replace_all(
    text: &dyn TextRead,
    query: &SearchQuery,
    replacement: &str,
    cancel: &CancelToken,
) -> Option<(eve_text::ChangeSet, u64)> {
    build_replace_all_in(text, query, replacement, None, cancel)
        .ok()
        .flatten()
}

/// `build_replace_all` と同じで、時計 (行の窓の核の縮めと処理の速さの下限。ジョブからは渡す、03 §12.3.1) を受け、終わり方の
/// 理由を返す。照合器を作れなければ `Ok(None)`。
///
/// # Errors
///
/// 取り消された (`ScanEnd::Cancelled`)、探さなかった (`TooLarge`)、重すぎて打ち切った (`TooHeavy`)。
pub fn build_replace_all_in(
    text: &dyn TextRead,
    query: &SearchQuery,
    replacement: &str,
    clock: Option<&dyn Clock>,
    cancel: &CancelToken,
) -> Result<Option<(eve_text::ChangeSet, u64)>, ScanEnd> {
    let Some(m) = query.matcher() else {
        return Ok(None);
    };
    let mut edits: Vec<eve_text::Edit> = Vec::new();
    let full = ByteRange::new(ByteOff(0), ByteOff(text.len_bytes()));
    // 正規表現では `$1` / `${name}` を一致ごとに展開する (03 §12.5)。リテラルは置換の文字列をそのまま入れる
    let insert: std::sync::Arc<str> = std::sync::Arc::from(replacement);
    let mut ctx = ScanCtx {
        cancel,
        clock,
        resume: None,
    };
    let end = scan_replacements(text, &m, full, &mut ctx, replacement, &mut |r, s| {
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
    match end {
        ScanEnd::Done => {}
        ScanEnd::Stopped => return Err(ScanEnd::Cancelled),
        e => return Err(e),
    }
    let n = edits.len() as u64;
    Ok(eve_text::ChangeSet::from_sorted_edits(text, edits)
        .ok()
        .map(|cs| (cs, n)))
}
