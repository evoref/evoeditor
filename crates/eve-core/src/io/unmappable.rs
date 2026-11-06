//! 表現できない文字の確認と置換 (docs/03 §8.6 E1〜E4) (STR-B1 で io/file.rs から分けた)。

use eve_view::{DocumentId, MessageLevel};

use crate::io::fs::Expect;
use crate::io::fspath::FsPath;
use crate::jobs::{JobError, JobKind, JobResult};
use eve_encoding::Encoding;

use super::pending::*;
use crate::command::CommandResult;
use crate::editor::state::EditorState;
use crate::ui_model::overlay::Continuation;

/// 表現不能の文字があって保存を中止した後の選択 (`ui.confirm.save.unmappable`、03 §8.6 の「E3 の選択肢」)。
#[derive(Clone, Debug)]
pub(crate) struct UnmappableAsk {
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    /// 最初の表現不能の文字の通し番号 (Unicode スカラー値、0 始まり)。キャンセルでカーソルをここへ移す。
    pub first_char: u64,
    /// 保存先の文字コード (`file.set_encoding` の指定があればそれ)。
    pub encoding: Encoding,
    pub fmt: Option<NewFormat>,
}

impl EditorState {
    /// 表現不能の文字の確認を出す。選択肢: UTF-8 で保存 (エスケープ文字を含まないとき) / キャンセル (既定、`Esc`)。
    /// 「置換して保存」(`replace`) は置換のジョブを投入する。
    pub(crate) fn open_unmappable(
        &mut self,
        ask: UnmappableAsk,
        total: u64,
        encoding: Encoding,
        has_escape: bool,
    ) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let (line, col) = self
            .char_pos(ask.doc, ask.first_char)
            .map_or((1, 1), |(_, l, c)| (l, c));
        let mut choices = Vec::new();
        // UTF-8 でも表せないのはエスケープ文字だけ (E4)。UTF-8 / UTF-16 で保存中のときは、選んでも結果が同じなので出さない
        if !has_escape && !matches!(encoding, Encoding::Utf8) {
            choices.push(ChoiceDef {
                id: "utf8",
                accel: Some('u'),
                destructive: false,
            });
        }
        // 置換して保存: 文書が編集できるときだけ (表現不能の文字にエスケープ文字を含まないとき)
        if !has_escape
            && self
                .docs
                .documents
                .get(&ask.doc)
                .is_some_and(|d| d.is_editable())
        {
            choices.push(ChoiceDef {
                id: "replace",
                accel: Some('r'),
                destructive: false,
            });
        }
        let default = choices.len();
        self.open_confirm(ConfirmState {
            name: "save.unmappable",
            args: vec![
                ("count".to_owned(), total.to_string()),
                ("encoding".to_owned(), encoding.name().to_owned()),
                ("line".to_owned(), line.to_string()),
                ("col".to_owned(), col.to_string()),
            ],
            choices,
            details: Vec::new(),
            default,
            focused: default,
            action: ConfirmAction::Unmappable(ask),
        });
    }

    /// 通し番号 `n` (Unicode スカラー値) の文字の (バイト位置, 行, 桁)。行・桁は 1 始まり (桁は文字数)。
    pub(crate) fn char_pos(&self, doc: DocumentId, n: u64) -> Option<(usize, usize, usize)> {
        use eve_text::TextRead;
        let d = self.docs.documents.get(&doc)?;
        let t = d.text();
        let body = t.slice_to_string(eve_text::ByteRange::all(t.len_bytes()));
        let (off, _) = body.char_indices().nth(usize::try_from(n).ok()?)?;
        let line = t.byte_to_line(eve_text::ByteOff(off)).0;
        let line_start = t.line_to_byte(eve_text::LineIdx(line)).0;
        let col = body[line_start..off].chars().count();
        Some((off, line + 1, col + 1))
    }

    /// 表現不能の確認の選択 (キャンセルは `unmappable_cancelled`)。
    pub(crate) fn choose_unmappable(&mut self, ask: UnmappableAsk, id: &str) -> CommandResult {
        // 保存へ進む選択では強調は要らない (キャンセルのときだけ次の編集まで残す)
        self.io.unmappable_marks = None;
        if id == "replace" {
            return self.spawn_replace(ask, false);
        }
        let UnmappableAsk {
            doc,
            path,
            expected,
            cont,
            ..
        } = ask;
        match id {
            // 同じ保存先へ UTF-8 (BOM なし) の書式で保存し直す。書式は保存が成功したときだけ確定する
            "utf8" => self.start_save_fmt(
                doc,
                path,
                expected,
                cont,
                Some(NewFormat {
                    encoding: Encoding::Utf8,
                    bom: false,
                }),
            ),
            _ => CommandResult::Done,
        }
    }

    /// 表現不能の文字を置換する ChangeSet を作るジョブを投入する (03 §8.6 の選択肢 2)。
    pub(crate) fn spawn_replace(&mut self, ask: UnmappableAsk, retried: bool) -> CommandResult {
        let Some(doc) = self.docs.documents.get(&ask.doc) else {
            return CommandResult::Done;
        };
        let revision = doc.revision();
        let job = self.jobs.spawn(
            Some(ask.doc),
            revision,
            JobKind::ReplaceUnmappable {
                snapshot: doc.snapshot(),
                encoding: ask.encoding,
                escape: doc.format.save_escape_policy(ask.encoding),
            },
        );
        self.io.pending.insert(
            job,
            Pending::ReplaceUnmappable {
                ask,
                revision,
                retried,
            },
        );
        CommandResult::Pending(job)
    }

    /// 置換の ChangeSet ができた: 対象の一覧を示して確認を取る。作った Revision と今の Revision が違えば 1 回だけ作り直す。
    pub(crate) fn finish_replace_unmappable(
        &mut self,
        ask: UnmappableAsk,
        revision: eve_view::Revision,
        retried: bool,
        r: JobResult,
    ) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        match r {
            JobResult::UnmappableReplacement {
                changes,
                targets,
                counts,
                ..
            } => {
                let Some(doc) = self.docs.documents.get(&ask.doc) else {
                    return;
                };
                if doc.revision() != revision {
                    if retried {
                        self.show_message(
                            MessageLevel::Error,
                            "msg.save.failed",
                            &[("path", &ask.path.display()), ("reason", "stale")],
                        );
                    } else {
                        self.spawn_replace(ask, true);
                    }
                    return;
                }
                let mut details: Vec<String> = Vec::with_capacity(targets.len() + 1);
                for t in &targets {
                    let (line, col) = self.pos_of_offset(ask.doc, t.offset);
                    let shown = if t.ch.is_control() { '\u{FFFD}' } else { t.ch };
                    details.push(self.catalog().render(
                        "ui.confirm.save.unmappable_replace.item",
                        &[
                            ("line", &line.to_string()),
                            ("col", &col.to_string()),
                            ("char", &shown.to_string()),
                            ("codepoint", &format!("U+{:04X}", t.ch as u32)),
                            ("replacement", &t.replacement.to_string()),
                        ],
                        &|_| None,
                    ));
                }
                let rest = counts.total().saturating_sub(targets.len() as u64);
                if rest > 0 {
                    details.push(self.catalog().render(
                        "ui.confirm.save.unmappable_replace.more",
                        &[("count", &rest.to_string())],
                        &|_| None,
                    ));
                }
                self.open_confirm(ConfirmState {
                    name: "save.unmappable_replace",
                    args: vec![
                        ("total".to_owned(), counts.total().to_string()),
                        ("encoding".to_owned(), ask.encoding.name().to_owned()),
                        ("kana".to_owned(), counts.kana.to_string()),
                        ("compat".to_owned(), counts.compat.to_string()),
                        ("question".to_owned(), counts.question.to_string()),
                    ],
                    choices: vec![ChoiceDef {
                        id: "replace",
                        accel: Some('r'),
                        destructive: false,
                    }],
                    details,
                    // 既定はキャンセル
                    default: 1,
                    focused: 1,
                    action: ConfirmAction::UnmappableReplace {
                        ask,
                        changes,
                        revision,
                    },
                });
            }
            JobResult::Failed { err, .. } => {
                let (level, key) = match err {
                    JobError::Cancelled => (MessageLevel::Info, "info.save.aborted"),
                    _ => (MessageLevel::Error, "msg.save.failed"),
                };
                self.show_message(
                    level,
                    key,
                    &[("path", &ask.path.display()), ("reason", "replace")],
                );
            }
            _ => {}
        }
    }

    /// バイト位置 `off` の (行, 桁)。どちらも 1 始まり (桁は文字数)。
    pub(crate) fn pos_of_offset(&self, doc: DocumentId, off: usize) -> (usize, usize) {
        use eve_text::TextRead;
        let Some(d) = self.docs.documents.get(&doc) else {
            return (1, 1);
        };
        let t = d.text();
        let line = t.byte_to_line(eve_text::ByteOff(off)).0;
        let line_start = t.line_to_byte(eve_text::LineIdx(line)).0;
        let body = t.slice_to_string(eve_text::ByteRange::new(
            eve_text::ByteOff(line_start),
            eve_text::ByteOff(off),
        ));
        (line + 1, body.chars().count() + 1)
    }

    /// 置換の確認で「置換して保存」: ChangeSet を 1 つの Transaction (`Origin::System`、Undo で戻せる) として適用し、
    /// 同じ文字コードで保存し直す (03 §8.6)。作った Revision が古ければ作り直す。
    pub(crate) fn apply_replacement_and_save(
        &mut self,
        ask: UnmappableAsk,
        changes: eve_text::ChangeSet,
        revision: eve_view::Revision,
    ) -> CommandResult {
        use crate::document::{Origin, Transaction, UndoGroupHint};
        let Some(doc) = self.docs.documents.get(&ask.doc) else {
            return CommandResult::Done;
        };
        if doc.revision() != revision {
            return self.spawn_replace(ask, true);
        }
        // 置換する文書を表示している View の選択を履歴に入れる (別の文書の View を使うと Undo でカーソルが飛ぶ、SAF-19)
        let view = self
            .focused_window()
            .and_then(|w| self.window_view(w))
            .filter(|v| self.docs.views.get(v).is_some_and(|v| v.doc == ask.doc))
            .or_else(|| {
                self.docs
                    .views
                    .iter()
                    .find(|(_, v)| v.doc == ask.doc)
                    .map(|(id, _)| *id)
            });
        let sel = view.and_then(|v| self.docs.views.get(&v).map(|v| v.selection.clone()));
        let mut tx = Transaction::new(changes, Origin::System, self.now)
            .with_undo_group(UndoGroupHint::Break)
            .with_selections(sel, None);
        if let Some(v) = view {
            tx = tx.with_view(v);
        }
        if self.apply_transaction(ask.doc, tx).is_err() {
            self.show_message(
                MessageLevel::Error,
                "msg.save.failed",
                &[("path", &ask.path.display()), ("reason", "replace")],
            );
            return CommandResult::Done;
        }
        self.start_save_fmt(ask.doc, ask.path, ask.expected, ask.cont, ask.fmt)
    }

    /// 表現不能の文字 (通し番号 `indices`) の位置を強調として記録する。文書の Revision が変わると無効になる。
    pub(crate) fn mark_unmappable(&mut self, doc: DocumentId, indices: &[u64]) {
        use eve_text::TextRead;
        let Some(d) = self.docs.documents.get(&doc) else {
            return;
        };
        let t = d.text();
        let want: std::collections::BTreeSet<u64> = indices.iter().copied().collect();
        let last = want.last().copied().unwrap_or(0);
        let mut marks: Vec<(usize, usize)> = Vec::with_capacity(want.len());
        let mut n = 0u64;
        let mut pos = 0usize;
        'chunks: for chunk in t.chunks(eve_text::ByteRange::all(t.len_bytes())) {
            for (i, c) in chunk.char_indices() {
                if want.contains(&n) {
                    marks.push((pos + i, pos + i + c.len_utf8()));
                }
                n += 1;
                if n > last {
                    break 'chunks;
                }
            }
            pos += chunk.len();
        }
        self.io.unmappable_marks = Some((doc, d.revision(), marks));
        self.mark_dirty_render();
    }

    /// 強調する表現不能の文字 (文書が変わっていなければ)。
    pub(crate) fn unmappable_marks_for(&self, doc: DocumentId) -> Vec<(usize, usize)> {
        match &self.io.unmappable_marks {
            Some((d, rev, marks))
                if *d == doc
                    && self
                        .docs
                        .documents
                        .get(d)
                        .is_some_and(|x| x.revision() == *rev) =>
            {
                marks.clone()
            }
            _ => Vec::new(),
        }
    }

    /// 表現不能の確認のキャンセル: 保存しない (後続も中止)。カーソルを最初の表現不能の位置へ移す。
    pub(crate) fn unmappable_cancelled(&mut self, ask: &UnmappableAsk) {
        if let Some((off, _, _)) = self.char_pos(ask.doc, ask.first_char)
            && let Some(view) = self
                .focused_window()
                .and_then(|w| self.window_view(w))
                .filter(|v| self.docs.views.get(v).is_some_and(|v| v.doc == ask.doc))
        {
            self.set_selection(view, eve_text::Selection::point(eve_text::ByteOff(off)));
            self.ensure_cursor_visible(view);
        }
        self.show_message(MessageLevel::Info, "info.save.aborted", &[]);
    }
}

/// §8.3 のエスケープ文字 (不正バイトの保持マーカー、U+10FF00 + byte)。UTF-8 でも表現不能として扱う (E4)。
pub(crate) fn is_escape_char(c: char) -> bool {
    (0x10FF00..=0x10FFFF).contains(&(c as u32))
}
