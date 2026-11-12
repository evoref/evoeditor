//! エスケープ文字のつながりの確認 (docs/03 §8.3.1 の「つながりの検出」の「利用者への示し方」、§9.7 (e) の
//! `JobResult::EscapeMerge` の行、04 §15.2 `ui.confirm.save.escape_merge`)。
//!
//! 保存のジョブが書く前につながりを見つけると、何も書かずに `JobResult::EscapeMerge` を返す。core は件数・最初の位置・
//! 変わる範囲と、1 件 1 行の一覧 (`details`) を示して尋ねる。選択肢は「このまま保存」とキャンセル (既定)。
//! - このまま保存: 結果の base Revision が今の Revision と同じなら、結果を承認として `SaveApprovals`
//!   (`IoState::escape_merge_ok`) に覚え、同じ保存を投入し直す (投入のたびに承認を引いてジョブに渡す)。違えば覚えずに
//!   今の Snapshot で保存し直す (つながりが残っていれば、もう一度確認する)。
//! - キャンセル: 保存しない。カーソルを最初の位置へ移し、全部の位置 (先頭 100 件) を表現不能と同じ印で強調して
//!   `info.save.escape_merge_cancelled` を出す。後続 (終了・置き換え・開き直し) は行わない。

use std::collections::{BTreeMap, BTreeSet};

use eve_view::{DocumentId, MessageLevel, Revision};

use crate::command::CommandResult;
use crate::editor::state::EditorState;
use crate::io::fs::Expect;
use crate::io::fspath::FsPath;
use crate::jobs::{EscapeMergeApproval, SaveStrategy};
use crate::ui_model::keys;
use crate::ui_model::overlay::Continuation;

use super::pending::NewFormat;

/// 一覧の「→」の左 (強調の範囲の本文) に出す文字数の上限。右 (局所の復号) はジョブが先頭 16 文字に切っている。
const BEFORE_MAX_CHARS: usize = 16;

/// つながりの確認の材料 (03 §9.7 (e))。「このまま保存」で同じ保存を投入し直すのに要るものを持つ。
#[derive(Clone, Debug)]
pub(crate) struct EscapeMergeAsk {
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    /// 保存方式 (直接上書きの承認の後なら直接上書きのまま投入し直す)。
    pub strategy: SaveStrategy,
    pub fmt: Option<NewFormat>,
    /// 結果の base Revision (保存を投入した時点の文書の Revision)。
    pub base: Revision,
    /// 結果 (件数・先頭 100 件・全件のダイジェスト)。「このまま保存」で承認として覚える。
    pub approval: EscapeMergeApproval,
}

/// 開き直すと変わる範囲の合計 (確認の本文に出す)。
#[derive(Clone, Copy, Debug)]
pub(crate) struct MergeExtent {
    pub chars: u64,
    pub lines: u64,
    /// 変わる範囲が文書の末尾まで戻らない (本文は「ファイルの末尾まで」)。
    pub to_eof: bool,
}

/// 一覧に埋め込む文字列の無害化 (03 §14.6): エスケープ文字は `<0xNN>` の表記、制御文字は `^X` (DEL は `^?`)・
/// C1 制御文字は `<U+XXXX>` の記号にする (改行も 1 行の一覧を崩さないよう記号にする)。
fn notation(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let u = c as u32;
        if let Some(b) = eve_encoding::unescape_char(c) {
            out.push_str(&format!("<0x{b:02X}>"));
        } else if u < 0x20 {
            out.push('^');
            out.push(char::from(u8::try_from(u ^ 0x40).unwrap_or(b'?')));
        } else if u == 0x7F {
            out.push_str("^?");
        } else if (0x80..0xA0).contains(&u) {
            out.push_str(&format!("<U+{u:04X}>"));
        } else {
            out.push(c);
        }
    }
    out
}

impl EditorState {
    /// 文書 `doc` の、通し番号 (Unicode スカラー値) `wanted` の文字の先頭のバイト位置。本文の文字数と同じ番号は
    /// 本文の末尾。本文の外の番号は結果に入らない。チャンクを 1 回なめる。
    pub(crate) fn char_offsets(
        &self,
        doc: DocumentId,
        wanted: &BTreeSet<u64>,
    ) -> BTreeMap<u64, usize> {
        use eve_text::TextRead;
        let mut out = BTreeMap::new();
        let Some(d) = self.docs.documents.get(&doc) else {
            return out;
        };
        let Some(&last) = wanted.last() else {
            return out;
        };
        let t = d.text();
        let mut n = 0u64;
        let mut pos = 0usize;
        'chunks: for chunk in t.chunks(eve_text::ByteRange::all(t.len_bytes())) {
            for (i, _) in chunk.char_indices() {
                if wanted.contains(&n) {
                    out.insert(n, pos + i);
                }
                n += 1;
                if n > last {
                    break 'chunks;
                }
            }
            pos += chunk.len();
        }
        if n <= last && wanted.contains(&n) {
            out.insert(n, t.len_bytes());
        }
        out
    }

    /// 文字の範囲 (通し番号の先頭, 文字数) の列を、本文のバイト範囲の列にする (本文の外にかかる範囲は除く)。
    pub(crate) fn char_ranges(
        &self,
        doc: DocumentId,
        ranges: &[(u64, u64)],
    ) -> Vec<(usize, usize)> {
        let wanted: BTreeSet<u64> = ranges
            .iter()
            .flat_map(|&(s, len)| [s, s.saturating_add(len)])
            .collect();
        let offs = self.char_offsets(doc, &wanted);
        ranges
            .iter()
            .filter_map(|&(s, len)| Some((*offs.get(&s)?, *offs.get(&s.saturating_add(len))?)))
            .collect()
    }

    /// つながりの確認を出す (03 §8.3.1 の「利用者への示し方」)。最初の位置が画面に入るようにスクロールする
    /// (カーソルは動かさない)。
    pub(crate) fn open_escape_merge(&mut self, ask: EscapeMergeAsk, extent: MergeExtent) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let ranges: Vec<(u64, u64)> = ask
            .approval
            .recorded
            .iter()
            .map(|m| (m.char_index, u64::from(m.len_chars)))
            .collect();
        let spans = self.char_ranges(ask.doc, &ranges);
        let mut details: Vec<String> = Vec::with_capacity(spans.len() + 1);
        let mut first: Option<(usize, usize, usize)> = None;
        for (m, &(start, end)) in ask.approval.recorded.iter().zip(&spans) {
            let (line, col) = self.pos_of_offset(ask.doc, start);
            if first.is_none() {
                first = Some((start, line, col));
            }
            let before = self.range_text(ask.doc, start, end);
            let mut before_shown: String = before.chars().take(BEFORE_MAX_CHARS).collect();
            if before.chars().count() > BEFORE_MAX_CHARS {
                before_shown.push('…');
            }
            details.push(self.catalog().render(
                keys::UI_CONFIRM_SAVE_ESCAPE_MERGE_ITEM,
                &[
                    ("line", &line.to_string()),
                    ("col", &col.to_string()),
                    ("before", &notation(&before_shown)),
                    ("after", &notation(&m.decoded)),
                ],
                &|_| None,
            ));
        }
        let rest = ask
            .approval
            .total
            .saturating_sub(ask.approval.recorded.len() as u64);
        if rest > 0 {
            details.push(self.catalog().render(
                keys::UI_CONFIRM_SAVE_ESCAPE_MERGE_MORE,
                &[("count", &rest.to_string())],
                &|_| None,
            ));
        }
        let (first_off, line, col) = first.unwrap_or((0, 1, 1));
        if first.is_some() {
            self.reveal_offset(ask.doc, first_off);
        }
        self.open_confirm(ConfirmState {
            name: "save.escape_merge",
            args: vec![
                ("count".to_owned(), ask.approval.total.to_string()),
                ("line".to_owned(), line.to_string()),
                ("col".to_owned(), col.to_string()),
                ("chars".to_owned(), extent.chars.to_string()),
                ("lines".to_owned(), extent.lines.to_string()),
            ],
            choices: vec![ChoiceDef {
                id: "save",
                accel: Some('s'),
                destructive: false,
            }],
            details,
            // 既定はキャンセル (書いた後に開き直すと文字が変わる選択肢を既定にしない、04 §15.2)
            default: 1,
            focused: 1,
            message: extent.to_eof.then_some("message_to_eof"),
            action: ConfirmAction::EscapeMerge(Box::new(ask)),
        });
    }

    /// 本文のバイト範囲 `start..end` の文字列。
    fn range_text(&self, doc: DocumentId, start: usize, end: usize) -> String {
        use eve_text::TextRead;
        let Some(d) = self.docs.documents.get(&doc) else {
            return String::new();
        };
        d.text().slice_to_string(eve_text::ByteRange::new(
            eve_text::ByteOff(start),
            eve_text::ByteOff(end),
        ))
    }

    /// フォーカスのある窓が文書 `doc` を表示していれば、バイト位置 `off` が画面に入るようにスクロールする。
    /// カーソル (選択・目標の桁) は動かさない。
    fn reveal_offset(&mut self, doc: DocumentId, off: usize) {
        let Some(view) = self
            .focused_window()
            .and_then(|w| self.window_view(w))
            .filter(|v| self.docs.views.get(v).is_some_and(|v| v.doc == doc))
        else {
            return;
        };
        let Some(saved) = self
            .docs
            .views
            .get(&view)
            .map(|v| (v.selection.clone(), v.goal_dcol, v.cursor_affinity))
        else {
            return;
        };
        self.set_selection(view, eve_text::Selection::point(eve_text::ByteOff(off)));
        self.ensure_cursor_visible(view);
        if let Some(v) = self.docs.views.get_mut(&view) {
            (v.selection, v.goal_dcol, v.cursor_affinity) = saved;
        }
    }

    /// 「このまま保存」(03 §8.3.1 の「承認」、§9.7 の表の `escape_merge_ok` の列)。結果の base Revision が今の
    /// Revision と同じなら承認を `SaveApprovals` に覚えて投入し直す。違えば (保存の待ちの間に編集した) 覚えずに、
    /// 今の Snapshot で保存し直す (利用者が見ていない位置を承認しない)。ほかの項目 (保存先・期待・書式・方式・後続) は
    /// 変えない。
    pub(crate) fn choose_escape_merge(&mut self, ask: EscapeMergeAsk) -> CommandResult {
        let Some(doc) = self.docs.documents.get(&ask.doc) else {
            return CommandResult::Done;
        };
        // 前のキャンセルの強調は、保存へ進むので要らない
        self.io.unmappable_marks = None;
        if doc.revision() == ask.base {
            self.io.escape_merge_ok = Some((ask.doc, ask.base, ask.approval));
        }
        self.start_save_with(
            ask.doc,
            ask.path,
            ask.expected,
            ask.cont,
            ask.strategy,
            ask.fmt,
        )
    }

    /// つながりの確認のキャンセル: 保存しない (後続も中止)。カーソルを最初の位置へ移し、全部の位置 (先頭 100 件) を
    /// 強調の範囲で強調する (表現不能と同じ印。次の編集で消える)。結果の後に文書が編集されていたら、位置が今の本文と
    /// 合わないので、強調も移動もせずに中止だけを示す。
    pub(crate) fn escape_merge_cancelled(&mut self, ask: &EscapeMergeAsk) {
        let current = self
            .docs
            .documents
            .get(&ask.doc)
            .map(crate::document::Document::revision);
        if current != Some(ask.base) {
            self.show_message(
                MessageLevel::Info,
                keys::INFO_SAVE_ABORTED,
                &[("path", &ask.path.display())],
            );
            return;
        }
        let ranges: Vec<(u64, u64)> = ask
            .approval
            .recorded
            .iter()
            .map(|m| (m.char_index, u64::from(m.len_chars)))
            .collect();
        let marks = self.char_ranges(ask.doc, &ranges);
        if let Some(&(first, _)) = marks.first()
            && let Some(view) = self
                .focused_window()
                .and_then(|w| self.window_view(w))
                .filter(|v| self.docs.views.get(v).is_some_and(|v| v.doc == ask.doc))
        {
            self.set_selection(view, eve_text::Selection::point(eve_text::ByteOff(first)));
            self.ensure_cursor_visible(view);
        }
        let count = marks.len();
        self.set_marks(ask.doc, marks);
        self.show_message(
            MessageLevel::Info,
            keys::INFO_SAVE_ESCAPE_MERGE_CANCELLED,
            &[("count", &count.to_string())],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 一覧の文字列はエスケープ文字と制御文字を記号にする() {
        assert_eq!(
            notation("\u{10FF81}\u{3000}\n\r\t\u{7F}\u{85}a"),
            "<0x81>\u{3000}^J^M^I^?<U+0085>a"
        );
    }
}
