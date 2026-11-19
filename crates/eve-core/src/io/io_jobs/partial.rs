//! 先行表示 (Partial) の先頭 (docs/03 §9.2.1。v0.4 の M25、FR-122 AC-2〜AC-4)。
//!
//! 読込が [`PARTIAL_AFTER`] を過ぎても終わらないとき、開くジョブは復号した本文の固定の先頭を `JobResult::Partial` で 1 回
//! だけ送る (`OpenOptions.partial` が立っているときだけ)。先頭は、木 (`TextBuilder`) に積むのと同じ復号の出力を別に複写した
//! もの ([`HeadCopy`]。1 MiB と 1 チャンク分まで) から作るので、常に全文の接頭辞になる。切り方は [`head_cut`]。

use eve_encoding::{Encoding, PuaEscapeMode};
use eve_text::{
    ByteOff, GRAPHEME_CONTEXT, LineEnding, Text, TextBuilder, is_grapheme_boundary,
    prev_grapheme_boundary,
};

use crate::document::{LineStatsScanner, TextFormat, count_escape_chars};
use crate::time::{Millis, Tick};

/// 本文の読込を始めてから先行表示を出すまでの時間 (03 §9.2.1 の「出す条件」。設定キーにしない)。
pub(crate) const PARTIAL_AFTER: Millis = Millis(200);

/// 先頭の大きさの上限 (復号した本文の UTF-8 のバイト数。03 §9.2.1 の「先頭」)。
pub(crate) const PARTIAL_HEAD: usize = 1024 * 1024;

/// 判定のやり直し (03 §8.2.1) で先頭から読み直すときに、前の読込から引き継ぐもの。
#[derive(Clone, Copy, Debug)]
pub(super) struct Redo {
    /// やり直しで使う文字コード (完了の `notes.redetected`)。
    pub encoding: Encoding,
    /// 先行表示の時間の起点 (最初の読込が `open_read` を始めた時刻)。`OpenOptions.partial` が立っていないときは `None`。
    pub started: Option<Tick>,
    /// 前の読込が先頭を送った (やり直しでは時間の条件を待たずに送り直す)。
    pub partial_sent: bool,
}

/// 復号した本文の先頭の複写 (1 MiB と 1 チャンク分まで)。送ったら手放す。
pub(super) struct HeadCopy {
    buf: String,
    started: Tick,
    /// 前の読込が先頭を送ったので、時間の条件を待たずに、先頭が揃った時点で送り直す。
    resend: bool,
    sent: bool,
}

/// 先頭の形式に要る、判定の値 (03 §9.2.1 の「結果の形」)。
pub(super) struct HeadParams {
    pub encoding: Encoding,
    pub has_bom: bool,
    pub pua_escape: PuaEscapeMode,
    /// 復号器がここまでに数えた不正バイトの数 (先頭の分はこれを超えない)。
    pub invalid_so_far: u32,
    pub long_line_threshold: usize,
    /// 改行が無い・同数のときの `default_eol`。
    pub default_eol: LineEnding,
}

impl HeadCopy {
    pub(super) fn new(started: Tick, resend: bool) -> HeadCopy {
        HeadCopy {
            buf: String::new(),
            started,
            resend,
            sent: false,
        }
    }

    pub(super) fn started(&self) -> Tick {
        self.started
    }

    pub(super) fn sent(&self) -> bool {
        self.sent
    }

    /// やり直しの読込で、まだ送り直していない。
    pub(super) fn resend_pending(&self) -> bool {
        self.resend && !self.sent
    }

    /// 復号の出力の続き `out` を複写する。1 MiB を超えたら (1 MiB の位置の次の char まで揃ったら) それ以上は持たない。
    pub(super) fn feed(&mut self, out: &str) {
        if !self.sent && self.buf.len() <= PARTIAL_HEAD {
            self.buf.push_str(out);
        }
    }

    /// チャンクの区切りで、今送るか。`more` はまだ読む分がある (読み終えていない)。
    /// 通常は 200 ms を過ぎて読み終えていないとき。やり直しの読込は先頭が揃った時点。
    pub(super) fn due(&self, now: Tick, more: bool) -> bool {
        if self.sent {
            return false;
        }
        if self.resend {
            return self.buf.len() > PARTIAL_HEAD;
        }
        more && now.saturating_sub(self.started) >= PARTIAL_AFTER
    }

    /// 先頭を切り出して、送る結果 (先頭の本文と形式) を作る。`complete` は複写がファイルの末尾まで (読み終えた) である。
    /// 複写は手放す (先頭は 1 回だけ送る)。
    pub(super) fn take(&mut self, complete: bool, p: &HeadParams) -> (Text, TextFormat) {
        self.sent = true;
        let buf = std::mem::take(&mut self.buf);
        let head = &buf[..head_cut(&buf, complete)];
        let mut b = TextBuilder::new();
        b.append(head);
        let text = b.finish();
        let format = head_format(head, &text, p);
        (text, format)
    }
}

/// 先頭の形式 (03 §9.2.1 の「結果の形」): 文字コード・BOM・エスケープの扱いは判定の値、改行の統計・不正バイトの数・
/// 長大行の数は先頭の分。往復の要約・保持方式の記録は送らない (完了で決まる)。
fn head_format(head: &str, text: &Text, p: &HeadParams) -> TextFormat {
    let mut scan = LineStatsScanner::new(p.long_line_threshold);
    scan.feed(head);
    let (eol_stats, long_lines) = scan.finish();
    // 衝突 (真の私用文字) の文書では、私用文字の数は不正バイトの数より多いことがある
    let escapes = u32::try_from(count_escape_chars(text)).unwrap_or(u32::MAX);
    let invalid_bytes = escapes.min(p.invalid_so_far);
    TextFormat {
        encoding: p.encoding,
        has_bom: p.has_bom,
        default_eol: eol_stats.majority(p.default_eol),
        eol_stats,
        invalid_bytes,
        pua_escape: p.pua_escape,
        escape_encoding: TextFormat::escape_encoding_on_load(
            p.encoding,
            p.pua_escape,
            invalid_bytes,
        ),
        roundtrip: eve_encoding::RoundtripSummary::default(),
        long_line_threshold: p.long_line_threshold,
        long_lines,
    }
}

/// 先頭の切る位置 (03 §9.2.1 の「先頭」)。`s` は復号した本文の先頭 (ここまでに復号した全部、または 1 MiB を超えた所までの
/// 複写)、`complete` は `s` がファイルの末尾までである。
///
/// 1. 1 MiB (`s` がそれより短ければ `s` の末尾) までの最後の行の終端 (LF・CRLF・後ろが LF でないと分かっている CR) の直後。
///    `s` の末尾の CR は、次が LF かもしれないので数えない (CRLF を分けない)。1 MiB をまたぐ CRLF も数えない。
/// 2. 行の終端が無ければ (長大行の途中)、1 MiB の位置から書記素の境界 (§13.1) まで戻す (判定には次の char まで見る)。
///    1 MiB の位置が `s` の末尾なら、最後の書記素を含めない (後ろに結合する文字が続くかもしれない)。戻した所の直前が
///    CR なら、その CR の前で切る。
///
/// `complete` で `s` が 1 MiB 以下なら、全部を返す (ファイルの末尾は行の境界)。
pub(crate) fn head_cut(s: &str, complete: bool) -> usize {
    head_cut_at(s, complete, PARTIAL_HEAD)
}

/// [`head_cut`] の 1 MiB を `head` バイトにしたもの (試験で小さな本文を使うため)。
fn head_cut_at(s: &str, complete: bool, head: usize) -> usize {
    let mut limit = head.min(s.len());
    while !s.is_char_boundary(limit) {
        limit -= 1;
    }
    if complete && limit == s.len() {
        return limit;
    }
    if let Some(cut) = last_line_end(s.as_bytes(), limit, complete) {
        return cut;
    }
    let mut cut = grapheme_cut(s, limit);
    if cut > 0 && s.as_bytes()[cut - 1] == b'\r' {
        cut -= 1;
    }
    cut
}

/// `b[..limit]` の中の最後の行の終端の直後 (無ければ `None`)。
fn last_line_end(b: &[u8], limit: usize, complete: bool) -> Option<usize> {
    let mut end = limit;
    while let Some(i) = memchr::memrchr2(b'\r', b'\n', &b[..end]) {
        if b[i] == b'\n' {
            return Some(i + 1);
        }
        match b.get(i + 1) {
            // CRLF が 1 MiB の位置をまたぐ (`i + 1 < limit` の LF なら先に見つかっている)
            Some(b'\n') => {}
            Some(_) => return Some(i + 1),
            None if complete => return Some(i + 1),
            // 末尾の CR は数えない
            None => {}
        }
        end = i;
    }
    None
}

/// 行の終端が無いときの切る位置: `limit` から書記素の境界まで戻す。
///
/// 判定は `limit` の前の 2 × `GRAPHEME_CONTEXT` バイトと `limit` の次の char だけの小さな本文で行う。文書の書記素の境界の
/// 判定 (`is_grapheme_boundary` など) は位置の前後 `GRAPHEME_CONTEXT` バイトの窓で行い、境界かどうかは位置の前の文脈と
/// 次の char で決まるので、全文で判定したのと同じ結果になる。
fn grapheme_cut(s: &str, limit: usize) -> usize {
    let mut lo = limit.saturating_sub(2 * GRAPHEME_CONTEXT);
    while !s.is_char_boundary(lo) {
        lo -= 1;
    }
    let hi = s[limit..]
        .chars()
        .next()
        .map_or(limit, |c| limit + c.len_utf8());
    let w = Text::from(&s[lo..hi]);
    let p = ByteOff(limit - lo);
    let at = if hi > limit && is_grapheme_boundary(&w, p) {
        p
    } else {
        // 1 MiB の位置が復号済みの末尾: 最後の書記素を含めない
        prev_grapheme_boundary(&w, p)
    };
    lo + at.0
}

#[cfg(test)]
mod tests;
