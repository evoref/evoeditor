//! 保存時の変換 (末尾空白の除去・末尾の改行の付与。docs/03 §9.3.1 の SVT-1〜SVT-9、01 FR-216)。
//!
//! - 連鎖の「変換を判定した」印 ([`SaveTransform`]) を文書ごとに持つ。連鎖の入口 (`file.save`・`file.save_as`・
//!   `file.set_encoding`・`file.save_all` の各文書・`AskUnsaved` の「保存」) が [`EditorState::begin_save_chain`] で印を
//!   「未判定」にし、保存ジョブを投入する直前 (`start_save_with` の確認の後) の [`EditorState::apply_save_transform`] が
//!   1 回だけ判定して印を「判定済み」にする。確認の後の投入し直しは同じ所を通るが、印があるので二度は当てない (SVT-4 (c))。
//! - 変換は普通の編集 (1 つの Transaction、`UndoGroupHint::Break`)。書けずに終わっても本文に残し、失敗・中止のメッセージに
//!   付記して知らせる (SVT-8。文言は変換のノードが履歴の今の位置か (`NodeId` で比べる) で選ぶ)。例外は 1 つ: 上書き保存が
//!   書込権限なしで失敗して文書が読み取り専用になる前に、変換のノードが今の位置なら Undo として戻す。保存の成功では
//!   完了のメッセージに付記する (SVT-5 / SVT-6)。Redo のある文書には当てない (Redo を捨てない、SVT-1 (c))。
//! - 区分 B / D の確認 (`lossy_ok`)・読み取り専用を解除して保存 (`readonly_ok`) の承認は、変換で進んだ Revision に
//!   引き継ぐ (SVT-5)。
//!
//! 不変条件 (データを失わないため):
//! - 変換するのは dirty で、パスの変わらない、編集できる文書だけ。値が決まりきっていない (初回の `.editorconfig`・
//!   ワークスペース層を待っている) ときと 1 MiB を超えるときは変換しない (頼んでいない差分を出さない側に倒す)。
//! - `ChangeSet` は消す空白と足す改行だけ。書くバイト列の作り方 (03 §8.6・§8.7・05 §4.1) は変えない。
//!
//! spec: 03 §9.3.1, 03 §9.3, 03 §9.7, 03 §9.8, 04 §8.16, 07 §3.6

use std::sync::Arc;

use eve_text::{ByteOff, ByteRange, ChangeSet, InsertText, MapPolicy, Op, TextRead};
use eve_view::{CommandId, DocumentId, MessageLevel};

use crate::document::transform::trailing_whitespace;
use crate::document::{Document, NodeId, NodeState, Origin, Transaction, UndoGroupHint};
use crate::editor::state::EditorState;
use crate::io::fspath::FsPath;
use crate::io::pending::Pending;
use crate::jobs::cancel::CancelToken;
use crate::ui_model::keys;

/// 保存の連鎖の「変換を判定した」印 (03 §9.3.1 の SVT-4 (c))。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveTransform {
    /// 連鎖を始めた。まだ判定していない (`command` は保存を始めたコマンド。Transaction の `Origin::Command`)。
    Pending { command: &'static str },
    /// 判定した: 変換しなかった (設定が偽・未編集・パスが変わる・値が決まっていない・読み取り専用・変える所が無い)。
    NotApplied,
    /// 判定した: 設定は真だが、文書が 1 MiB を超えたので変換しなかった (SVT-6)。
    SkippedLarge,
    /// 判定した: 変換を当てた (本文が変わった)。`trimmed_lines` は空白を除いた行の数、`node` は変換の Undo のノード。
    Applied {
        trimmed_lines: usize,
        final_newline: bool,
        node: NodeId,
    },
    /// 変換を当てたが、上書き保存が書込権限なしで失敗して文書が読み取り専用になる前に、Undo として戻した (SVT-8 の例外)。
    Reverted,
}

/// 書けずに終わった連鎖の、変換の今の状態 (知らせの文言を選ぶ。SVT-8)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kept {
    /// 変換のノードが履歴の今の位置 (1 回の Undo で戻せる)。
    Undoable,
    /// 本文に残っているが、後に別の編集がある (変換だけを Undo では戻せない)。
    Remains,
    /// 戻した (SVT-8 の例外)。
    Reverted,
}

/// 保存時の変換の `ChangeSet` と、その中身の要約。
#[derive(Debug)]
pub(crate) struct SaveTransformPlan {
    pub changes: ChangeSet,
    pub trimmed_lines: usize,
    pub final_newline: bool,
}

/// 位置 `end` の直前の 1 文字 (`end` は char の境界)。
fn char_before(t: &eve_text::Text, end: usize) -> Option<char> {
    let mut start = end.saturating_sub(4);
    while start < end && !t.is_char_boundary(ByteOff(start)) {
        start += 1;
    }
    t.slice_to_string(ByteRange::new(ByteOff(start), ByteOff(end)))
        .chars()
        .next_back()
}

/// 保存時の変換の `ChangeSet` を作る (SVT-2 / SVT-3)。変える所が無ければ `None`。
///
/// `trim` なら各行の末尾の ASCII の空白とタブを消す (CR と LF をつなぐ行は消さない。テキスト変換と同じ関数)。
/// `final_newline` なら、除去の後の本文が空でなく、最後の文字が区切り (LF・CR) でも不正バイトのエスケープ文字でもなければ、
/// 末尾に `eol` を 1 つ足す。`ChangeSet` は消す範囲と足す改行だけ (行の全体を置き換えない)。
pub(crate) fn build_save_transform(
    t: &eve_text::Text,
    trim: bool,
    final_newline: bool,
    eol: eve_text::LineEnding,
) -> Option<SaveTransformPlan> {
    let len = t.len_bytes();
    let trims: Vec<ByteRange> = if trim {
        let last = t.len_lines().saturating_sub(1);
        // 取り消さないので必ず結果がある
        trailing_whitespace(t, 0, last, &CancelToken::new()).unwrap_or_default()
    } else {
        Vec::new()
    };
    // 除去の後の本文の末尾 (最後の範囲が文書の末尾まで消すなら、その始まり)
    let end = match trims.last() {
        Some(r) if r.end.0 == len => r.start.0,
        _ => len,
    };
    let add_newline = final_newline
        && end > 0
        && char_before(t, end)
            .is_some_and(|c| c != '\n' && c != '\r' && !crate::io::unmappable::is_escape_char(c));
    if trims.is_empty() && !add_newline {
        return None;
    }
    let mut ops: Vec<Op> = Vec::with_capacity(trims.len() * 2 + 2);
    let mut pos = 0usize;
    for r in &trims {
        if r.start.0 > pos {
            ops.push(Op::Retain(r.start.0 - pos));
        }
        ops.push(Op::Delete(r.len()));
        pos = r.end.0;
    }
    if len > pos {
        ops.push(Op::Retain(len - pos));
    }
    if add_newline {
        ops.push(Op::Insert(InsertText::Str(Arc::from(eol.as_str()))));
    }
    Some(SaveTransformPlan {
        changes: ChangeSet::from_ops(ops),
        trimmed_lines: trims.len(),
        final_newline: add_newline,
    })
}

impl EditorState {
    /// 保存の連鎖を始める (03 §9.3.1 の SVT-4 (c))。文書 `doc` の印を「未判定」にする。`command` は保存を始めたコマンド。
    /// 同じ文書の連鎖がまだ続いている (保存・照合・置換の一覧のジョブを待っている) ときは何もしない (続いている連鎖の
    /// 印を消すと、その連鎖の投入し直しで二度変換してしまう)。
    pub(crate) fn begin_save_chain(&mut self, doc: DocumentId, command: &'static str) {
        if self.save_chain_busy(doc) {
            return;
        }
        self.io
            .save_transforms
            .insert(doc, SaveTransform::Pending { command });
    }

    /// 文書 `doc` の保存の連鎖がジョブを待っている (保存・未編集の照合・名前を付けて保存の照合・置換の一覧・退避先への
    /// 保存)。
    fn save_chain_busy(&self, doc: DocumentId) -> bool {
        self.save_in_progress(doc)
            || self.io.pending.values().any(|p| match p {
                Pending::ReplaceUnmappable { ask, .. } => ask.doc == doc,
                Pending::WriteRecovery { doc: d, .. } => *d == doc,
                _ => false,
            })
    }

    /// 保存ジョブを投入する直前に、保存時の変換を判定して当てる (03 §9.3.1)。この連鎖で判定済みなら何もしない。
    /// `path` は書く先。呼び側は、すべての確認 (名前の入力・保存先の照合・上書き・区分 B / D・読み取り専用の解除) を
    /// 済ませてから呼ぶ。
    pub(crate) fn apply_save_transform(&mut self, doc_id: DocumentId, path: &FsPath) {
        let command = match self.io.save_transforms.get(&doc_id) {
            Some(SaveTransform::Pending { command }) => *command,
            Some(_) => return,
            // 入口を経ずに投入した保存 (起きないはず): 上書き保存として判定する
            None => {
                debug_assert!(false, "保存の連鎖の入口を経ずに保存した (文書 {doc_id:?})");
                crate::command::ids::FILE_SAVE
            }
        };
        let outcome = self.judge_save_transform(doc_id, path, command);
        self.io.save_transforms.insert(doc_id, outcome);
    }

    /// 文書 `doc_id` を `path` へ保存する直前の変換を判定し、当てる。
    fn judge_save_transform(
        &mut self,
        doc_id: DocumentId,
        path: &FsPath,
        command: &'static str,
    ) -> SaveTransform {
        let Some(doc) = self.docs.documents.get(&doc_id) else {
            return SaveTransform::NotApplied;
        };
        // SVT-4 (b) 未編集・(e) 読み取り専用・SVT-1 (a) パスが変わる・SVT-1 (c) Redo がある (変換の編集で Redo を捨てない)
        if !doc.is_dirty()
            || !doc.is_editable()
            || doc.path.as_ref() != Some(path)
            || doc.history().can_redo()
        {
            return SaveTransform::NotApplied;
        }
        // SVT-1 (b): 初回の値がまだ無い
        if self.save_transform_values_pending(doc) {
            return SaveTransform::NotApplied;
        }
        let values = self.doc_values_of(doc);
        let trim = values.trim_trailing_whitespace_on_save.value;
        let final_newline = values.insert_final_newline_on_save.value;
        if !trim && !final_newline {
            return SaveTransform::NotApplied;
        }
        // SVT-6: 1 MiB を超える文書はメインスレッドでなめない
        if doc.len_bytes() > self.io.transform_sync_bytes {
            return SaveTransform::SkippedLarge;
        }
        let Some(plan) =
            build_save_transform(doc.text(), trim, final_newline, doc.format.default_eol)
        else {
            return SaveTransform::NotApplied;
        };
        let SaveTransformPlan {
            changes,
            trimmed_lines,
            final_newline,
        } = plan;
        let before_revision = doc.revision();
        // フォーカスのペインがこの文書を出していれば、その View を発行 View にして選択を記録する (Undo で戻す)。
        // すべてのカーソルを足した改行の前に留める (`Bias::Before`。消した空白の中のカーソルは行末へ寄る、SVT-5)
        let view = self
            .focused_window()
            .and_then(|w| self.window_view(w))
            .filter(|v| self.view(*v).is_some_and(|v| v.doc == doc_id));
        let before = view.and_then(|v| self.view(v)).map(|v| v.selection.clone());
        let after = before.as_ref().map(|s| s.map(&changes, MapPolicy::Other));
        let mut tx = Transaction::new(
            changes,
            Origin::Command(CommandId::new(command)),
            self.now(),
        )
        .with_undo_group(UndoGroupHint::Break)
        .with_selections(before, after);
        if let Some(v) = view {
            tx = tx.with_view(v);
        }
        let Ok(revision) = self.apply_transaction(doc_id, tx) else {
            return SaveTransform::NotApplied;
        };
        let node = self
            .docs
            .documents
            .get(&doc_id)
            .map_or(NodeId(0), |d| d.history().current());
        // 承認の引き継ぎ (SVT-5): (文書, Revision) で照らす承認を、変換で進んだ Revision に移す
        if self.io.lossy_ok == Some((doc_id, before_revision)) {
            self.io.lossy_ok = Some((doc_id, revision));
        }
        if self.io.readonly_ok == Some((doc_id, before_revision)) {
            self.io.readonly_ok = Some((doc_id, revision));
        }
        SaveTransform::Applied {
            trimmed_lines,
            final_newline,
            node,
        }
    }

    /// SVT-1 (b): 文書の値が決まりきっていない。(b1) `.editorconfig` を読む起動で、その文書の今のパスの読込を一度も終えて
    /// いない (b2) ワークスペース層を読む起動で、ワークスペースのルートの検出の結果がまだ届いていない、または届いたルートの
    /// ワークスペース層の最初の読込を待っていて、文書がルートの下にある (b3) 言語を判定し直す言語定義の読込 (起動時の読込と
    /// 正規表現の組み立て、起動の後に届いたワークスペース層の `[[language]]` の読込) を待っている。
    fn save_transform_values_pending(&self, doc: &Document) -> bool {
        let reads_project = self.editorconfig_enabled();
        let editorconfig = reads_project && doc.path.is_some() && !doc.editorconfig_resolved;
        let pending = |f: &dyn Fn(&Pending) -> bool| self.io.pending.values().any(f);
        let root_pending = reads_project && pending(&|p| matches!(p, Pending::WorkspaceRoot));
        let workspace_first = self.settings.workspace_safe_dirs.is_none()
            && pending(&|p| matches!(p, Pending::WorkspaceConfig { .. }))
            && self.doc_in_workspace(doc);
        let languages = pending(&|p| {
            matches!(
                p,
                Pending::LanguagesRead { startup: true, .. }
                    | Pending::LanguagesPrecompile { redetect: true }
            )
        });
        editorconfig || root_pending || workspace_first || languages
    }

    /// 保存の成功の完了のメッセージに付ける付記 (SVT-5 / SVT-6、04 §8.16)。印を外す (連鎖が終わった)。
    pub(crate) fn take_save_transform_suffix(&mut self, doc: DocumentId) -> Option<String> {
        match self.io.save_transforms.remove(&doc)? {
            SaveTransform::Applied {
                trimmed_lines,
                final_newline,
                ..
            } => {
                let mut parts = Vec::new();
                if trimmed_lines > 0 {
                    let lines = trimmed_lines.to_string();
                    parts.push(
                        self.message_text(
                            keys::INFO_SAVE_TRANSFORMED_TRIMMED,
                            &[("lines", &lines)],
                        ),
                    );
                }
                if final_newline {
                    parts.push(self.message_text(keys::INFO_SAVE_TRANSFORMED_FINAL_NEWLINE, &[]));
                }
                let sep = self.message_text(keys::INFO_SAVE_TRANSFORMED_SEPARATOR, &[]);
                let detail = parts.join(&sep);
                Some(self.message_text(keys::INFO_SAVE_SUFFIX_TRANSFORMED, &[("detail", &detail)]))
            }
            SaveTransform::SkippedLarge => {
                Some(self.message_text(keys::INFO_SAVE_SUFFIX_TRANSFORM_SKIPPED_LARGE, &[]))
            }
            SaveTransform::Pending { .. } | SaveTransform::NotApplied | SaveTransform::Reverted => {
                None
            }
        }
    }

    /// この連鎖で変換を当てて本文が変わった (全保存の要約の件数、SVT-5)。印は外さない。
    pub(crate) fn save_transform_applied(&self, doc: DocumentId) -> bool {
        matches!(
            self.io.save_transforms.get(&doc),
            Some(SaveTransform::Applied { .. })
        )
    }

    /// この連鎖の変換の今の状態 (書けずに終わったときの知らせ、SVT-8)。変換を当てていない・Undo で戻された・文書が
    /// 閉じられていれば `None`。
    fn kept_state(&self, doc: DocumentId) -> Option<Kept> {
        let d = self.docs.documents.get(&doc)?;
        match self.io.save_transforms.get(&doc)? {
            SaveTransform::Applied { node, .. } => match d.history().node_state(*node) {
                NodeState::Current => Some(Kept::Undoable),
                NodeState::Applied => Some(Kept::Remains),
                NodeState::Undone => None,
            },
            SaveTransform::Reverted => Some(Kept::Reverted),
            _ => None,
        }
    }

    /// 書けずに終わった・中止した連鎖の失敗・中止のメッセージに付ける付記 (SVT-8)。変換のノードが履歴の今の位置なら
    /// `info.save.suffix.transform_kept` ({key:edit.undo} で戻せる)、後に別の編集があれば
    /// `info.save.suffix.transform_remains`、書込権限なしで戻したなら `info.save.suffix.transform_reverted`。変換を
    /// 当てていない・Undo で戻された・文書が閉じられていれば `None`。印は外さない (確認の後の投入し直しで、もう一度は
    /// 変換しない)。
    pub(crate) fn save_transform_kept_suffix(&self, doc: DocumentId) -> Option<String> {
        let key = match self.kept_state(doc)? {
            Kept::Undoable => keys::INFO_SAVE_SUFFIX_TRANSFORM_KEPT,
            Kept::Remains => keys::INFO_SAVE_SUFFIX_TRANSFORM_REMAINS,
            Kept::Reverted => keys::INFO_SAVE_SUFFIX_TRANSFORM_REVERTED,
        };
        Some(self.message_text(key, &[]))
    }

    /// 上書き保存が書込権限なしで失敗し、文書を読み取り専用 (`NoWritePermission`) にする直前に呼ぶ (03 §9.3.1 の SVT-8 の
    /// 例外)。この連鎖で当てた変換のノードが履歴の今の位置で、Redo が無ければ、Undo として戻して Redo に残さない
    /// (読み取り専用になると、利用者が Undo で戻せないため)。今の位置でなければ戻さない (後の編集を巻き込まない)。
    pub(crate) fn revert_save_transform_before_read_only(&mut self, doc: DocumentId) {
        let undoable = self.kept_state(doc) == Some(Kept::Undoable)
            && self
                .docs
                .documents
                .get(&doc)
                .is_some_and(|d| !d.history().can_redo() && d.is_editable());
        if !undoable {
            return;
        }
        if self.undo(doc, None).is_ok_and(|undone| undone) {
            if let Some(d) = self.docs.documents.get_mut(&doc) {
                d.discard_redo();
            }
            self.io.save_transforms.insert(doc, SaveTransform::Reverted);
        }
    }

    /// `show_message` に付記を足したもの (付記が無ければ同じ)。付記は本文 (`.text`) の後ろに空白をはさんで足す。
    pub(crate) fn show_message_with_suffix(
        &mut self,
        level: MessageLevel,
        key: &str,
        args: &[(&str, &str)],
        suffix: Option<&str>,
    ) {
        match suffix {
            Some(s) => self.show_message_appended(level, key, args, s),
            None => self.show_message(level, key, args),
        }
    }

    /// 保存の中止のメッセージ (`info.save.aborted`) を、変換が本文に残っていれば付記を付けて出す (SVT-8)。今の実装で
    /// 中止のメッセージを出さない確認 (外部変更・原子的置換不可などのキャンセル、名前の入力のキャンセル) は、変換が
    /// 残っているときだけ出す (残っていなければ v0.4 と同じく何も出さない)。
    pub(crate) fn notify_transform_kept_on_abort(&mut self, doc: DocumentId) {
        if let Some(suffix) = self.save_transform_kept_suffix(doc) {
            let shown = self
                .document(doc)
                .and_then(|d| d.path.as_ref())
                .map(FsPath::display)
                .unwrap_or_default();
            self.show_message_appended(
                MessageLevel::Info,
                keys::INFO_SAVE_ABORTED,
                &[("path", &shown)],
                &suffix,
            );
        }
    }

    /// 閉じた文書の印を捨てる。
    pub(crate) fn forget_save_transform(&mut self, doc: DocumentId) {
        self.io.save_transforms.remove(&doc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_text::LineEnding;
    use proptest::prelude::*;

    fn apply(s: &str, trim: bool, fin: bool, eol: LineEnding) -> Option<(String, usize, bool)> {
        let t = eve_text::Text::from(s);
        let plan = build_save_transform(&t, trim, fin, eol)?;
        let mut out = String::new();
        let mut pos = 0usize;
        for op in plan.changes.ops() {
            match op {
                Op::Retain(n) => {
                    out.push_str(&s[pos..pos + n]);
                    pos += n;
                }
                Op::Delete(n) => pos += n,
                Op::Insert(i) => out.extend(i.chunks()),
            }
        }
        Some((out, plan.trimmed_lines, plan.final_newline))
    }

    #[test]
    fn 除去の後に末尾の改行を足すかを決める() {
        assert_eq!(
            apply("a\n  ", true, true, LineEnding::Lf),
            Some(("a\n".to_owned(), 1, false))
        );
        assert_eq!(
            apply("a  ", true, true, LineEnding::Crlf),
            Some(("a\r\n".to_owned(), 1, true))
        );
        assert_eq!(
            apply("   ", true, true, LineEnding::Lf),
            Some((String::new(), 1, false))
        );
        assert_eq!(apply("", true, true, LineEnding::Lf), None);
        assert_eq!(apply("a\r", false, true, LineEnding::Lf), None);
        assert_eq!(
            apply("a", false, true, LineEnding::Cr),
            Some(("a\r".to_owned(), 0, true))
        );
        // 最後の文字が不正バイトのエスケープ文字なら足さない
        assert_eq!(apply("a\u{10FF80}", false, true, LineEnding::Lf), None);
        // CR と LF をつなぐ行は消さない
        assert_eq!(apply("a\r \nb", true, false, LineEnding::Lf), None);
    }

    /// 参照モデル (03 §15.9.1 の (c)): 行ごとに末尾の空白とタブを落とし (CR と LF がつながる行は落とさない)、空でなく
    /// 区切りで終わらず、最後の文字がエスケープでなければ `eol` を 1 つ足す。
    fn model(s: &str, trim: bool, fin: bool, eol: LineEnding) -> String {
        // 行 (中身, 区切り) に分ける (§2.4: CRLF・CR・LF)
        let mut lines: Vec<(String, &str)> = Vec::new();
        let mut cur = String::new();
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '\r' if it.peek() == Some(&'\n') => {
                    it.next();
                    lines.push((std::mem::take(&mut cur), "\r\n"));
                }
                '\r' => lines.push((std::mem::take(&mut cur), "\r")),
                '\n' => lines.push((std::mem::take(&mut cur), "\n")),
                c => cur.push(c),
            }
        }
        lines.push((cur, ""));
        let mut out = String::new();
        for i in 0..lines.len() {
            let (content, sep) = (&lines[i].0, lines[i].1);
            let stripped = content.trim_end_matches([' ', '\t']);
            let joins = stripped.is_empty() && i > 0 && lines[i - 1].1 == "\r" && sep == "\n";
            if trim && !joins {
                out.push_str(stripped);
            } else {
                out.push_str(content);
            }
            out.push_str(sep);
        }
        let ends_open = out
            .chars()
            .next_back()
            .is_some_and(|c| c != '\n' && c != '\r' && !crate::io::unmappable::is_escape_char(c));
        if fin && ends_open {
            out.push_str(eol.as_str());
        }
        out
    }

    fn text_strategy() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            proptest::sample::select(vec![
                'a',
                ' ',
                '\t',
                '\u{3000}',
                '\r',
                '\n',
                '\u{10FF80}',
                'あ',
            ]),
            0..40,
        )
        .prop_map(|v| v.into_iter().collect())
    }

    proptest! {
        /// 03 §15.9.1 の (c): 参照モデルと一致し、`ChangeSet` は消す空白と足す改行だけ。2 回目は何も変えない。
        #[test]
        fn 参照モデルと一致し_changeset_は空白の削除と末尾の改行だけ(
            s in text_strategy(),
            trim in any::<bool>(),
            fin in any::<bool>(),
            eol in proptest::sample::select(vec![LineEnding::Lf, LineEnding::Crlf, LineEnding::Cr]),
        ) {
            let want = model(&s, trim, fin, eol);
            match apply(&s, trim, fin, eol) {
                None => prop_assert_eq!(&want, &s),
                Some((out, _, _)) => {
                    prop_assert_eq!(&out, &want);
                    let t = eve_text::Text::from(s.as_str());
                    let plan = build_save_transform(&t, trim, fin, eol).expect("変える");
                    let ops = plan.changes.ops();
                    let mut pos = 0usize;
                    for (i, op) in ops.iter().enumerate() {
                        match op {
                            Op::Retain(n) => pos += n,
                            Op::Delete(n) => {
                                prop_assert!(s.as_bytes()[pos..pos + n].iter().all(|b| *b == b' ' || *b == b'\t'));
                                pos += n;
                            }
                            Op::Insert(ins) => {
                                prop_assert_eq!(i, ops.len() - 1, "足すのは末尾だけ");
                                prop_assert_eq!(pos, s.len());
                                prop_assert_eq!(ins.chunks().collect::<String>(), eol.as_str());
                            }
                        }
                    }
                    prop_assert!(apply(&out, trim, fin, eol).is_none(), "冪等");
                }
            }
        }
    }
}
