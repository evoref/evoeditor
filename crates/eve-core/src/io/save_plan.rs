//! 保存の計画 (docs/03 §8.6 E2、§8.7.1 の「保存」、v0.4 の M22 の 6): 保持方式の記録の版 (保存を始めた時点のもの) と
//! Snapshot から、記録ごとに「元のバイト列を書く」か「ふつうに符号化する」かを決める。
//!
//! 乾式走査・本番の書き込み (`io_jobs::SaveStream`) と、「置換して保存」の対象の判定 (`unmap::build_replacement`) が、
//! この 1 つの計画を使う。判断は記録と Snapshot だけで決まり、rope のチャンクの区切りに依らない。
//!
//! 元のバイト列を書く記録は、次を全部満たすもの (03 §8.6 E2 の (a)〜(d)。(a) 書く文字コードが記録を集めたときの文字コード
//! と同じことは、表を渡す側 (`EditorState::start_save_with`) が確かめる):
//! - 表の不変条件どおりに、範囲が本文の中にあり、char 境界にあり、前の記録と重ならない (`Decision::Skip`。起きないはず)。
//! - ISO-2022-JP では、範囲が 1 つの行の内容の全体である (行頭で始まり、行の終端の直前か文書の末尾で終わる。前後の行の
//!   終端を消して行が変わった記録は使わず、その行を編集した行と同じく正規形で書く。記録は表に残す)。
//! - 確認 (i): 範囲の今の文字列が、元のバイト列をその文字コードの初期状態から復号したものと一致し、初期状態で終わる
//!   (`eve_encoding::preserved_matches`。起きないはず)。
//!
//! 表の不変条件・確認 (i) に反する記録は実装の誤りで、保存は書く前に止まる (`JobError::VerifyFailed`。数えるのは
//! `io_jobs::SaveStream`)。
//!
//! 確認 (i) を通った記録は、その結果を `eve_encoding::PreservedBytes` (確認を経ずには作れない型) で符号化器の口
//! (`Encoder::write_checked`) へ渡す (口で同じ復号をやり直さない)。範囲の文字列・char 境界・行の全体の判定は、本文を
//! 前から順に読む位置 (`TextCursor`) で引く (記録ごとに rope を降りない。範囲が 1 つのチャンクに収まれば文字列を借りる)。

use std::borrow::Cow;

use eve_encoding::{Encoding, PreservedBytes};
use eve_text::{ByteOff, TextRead};

use crate::document::PreservedMap;
use crate::document::preserved::cursor::TextCursor;
use crate::document::preserved::{Iter, SpanRef};

/// 記録を使わなかった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unused {
    /// ISO-2022-JP の行の記録が、1 つの行の内容の全体でない (前後の行の終端を消して行が変わった)。
    Line,
    /// 確認 (i) に合わない (範囲の今の文字列が、元のバイト列を復号したものと違う)。表の不変条件に反する (起きないはず)。
    Mismatch,
}

/// 1 件の記録の扱い。
pub(crate) enum Decision<'a> {
    /// 元のバイト列をそのまま書く。`text` は範囲の今の文字列、`bytes` は確認 (i) を通った元のバイト列。
    Raw {
        span: SpanRef<'a>,
        text: Cow<'a, str>,
        bytes: PreservedBytes<'a>,
    },
    /// 記録を使わず、範囲の文字列 `text` をふつうに符号化する (範囲を区切りにする)。
    Encode {
        span: SpanRef<'a>,
        text: Cow<'a, str>,
        why: Unused,
    },
    /// 記録を使わず、範囲を区切りにもしない (範囲が本文の外・char 境界でない・前の記録と重なる。表の不変条件に反する。
    /// 起きないはず)。範囲の文字は前後の区間と一緒にふつうに符号化する。
    Skip { span: SpanRef<'a> },
}

impl Decision<'_> {
    /// 記録の範囲の始まり (区間をここまで符号化してから、この記録を扱う)。
    pub(crate) fn start(&self) -> usize {
        match self {
            Decision::Raw { span, .. }
            | Decision::Encode { span, .. }
            | Decision::Skip { span } => span.range.start.0,
        }
    }
}

/// 保存の計画を、本文の前から順に引く。
pub(crate) struct Planner<'a> {
    text: TextCursor<'a>,
    enc: Encoding,
    iter: Iter<'a>,
}

impl<'a> Planner<'a> {
    pub(crate) fn new(text: &'a dyn TextRead, enc: Encoding, map: &'a PreservedMap) -> Planner<'a> {
        Planner {
            text: TextCursor::new(text),
            enc,
            iter: map.iter(),
        }
    }

    /// 次の記録の扱い。`pos` は、そこまでの本文の扱いが決まった位置 (前の記録の終わり)。記録が尽きたら `None`。
    pub(crate) fn next(&mut self, pos: usize) -> Option<Decision<'a>> {
        let span = self.iter.next()?;
        let (s, e) = (span.range.start.0, span.range.end.0);
        // 表の不変条件に反する記録 (起きないはず): 区切りにしない。範囲が本文の中で逆順でなければ、文字列を引けない
        // のは端が char 境界でないときだけ
        if s < pos || e < s || e > self.text.len() {
            return Some(Decision::Skip { span });
        }
        let Some(text) = self.text.slice(s, e) else {
            return Some(Decision::Skip { span });
        };
        if self.enc.is_stateful() && !(span.line && self.text.is_whole_line(s, e)) {
            return Some(Decision::Encode {
                span,
                text,
                why: Unused::Line,
            });
        }
        // 確認 (i)。通った記録は、その結果を型で符号化器の口へ運ぶ
        match PreservedBytes::check(self.enc, span.bytes, &text) {
            Some(bytes) => Some(Decision::Raw { span, text, bytes }),
            None => Some(Decision::Encode {
                span,
                text,
                why: Unused::Mismatch,
            }),
        }
    }
}

/// 元のバイト列を書く記録の範囲 (昇順)。「置換して保存」の対象の判定に使う (03 §8.6 E2: 元のバイト列を書く範囲の文字は
/// 置換の対象にしない)。保存の符号化と同じ計画から求める。
pub(crate) fn raw_ranges(
    text: &dyn TextRead,
    enc: Encoding,
    map: &PreservedMap,
) -> Vec<(ByteOff, ByteOff)> {
    let mut out = Vec::new();
    let mut planner = Planner::new(text, enc, map);
    let mut pos = 0usize;
    while let Some(d) = planner.next(pos) {
        match d {
            Decision::Raw { span, .. } => {
                out.push((span.range.start, span.range.end));
                pos = span.range.end.0;
            }
            Decision::Encode { span, .. } => pos = span.range.end.0,
            Decision::Skip { .. } => {}
        }
    }
    out
}
