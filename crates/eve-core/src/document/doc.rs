//! `Document` と `Document::apply` (docs/03-core-design.md §6.5、docs/02 §5.1 / §5.2)。
//! 文書テキストを変える唯一の経路。View の Selection・Decoration の写像 (手順 7〜9) と通知 (手順 10・12) は、
//! それらを持つ `EditorState` (C2 / C3) が `Applied` を使って行う。

use std::sync::Arc;

use eve_encoding::{Encoding, NoPreserveReason, PreserveStatus, PuaEscapeMode};
use eve_text::{
    ByteOff, ByteRange, ChangeSet, ChangeSetError, Selection, Snapshot, Text, TextRead,
};
use eve_view::Revision;

use crate::io::fs::FileStamp;
use crate::io::fspath::FsPath;

use super::format::{
    EolStats, FormatChange, FormatRestore, LoadState, PreservedState, ReadOnlyReason,
    ReadOnlyReasons, TextFormat, count_long_lines, count_range, long_lines_touched,
};
use super::history::{NodeId, PreservedUndo, RecordOutcome, UndoHistory};
use super::preserved::{PreservedMap, PreservedSpan};
use super::transaction::{Origin, Transaction};

/// `Document::apply` が返す結果。
#[derive(Clone, Debug)]
pub enum AppliedTx {
    /// 変更のない Transaction (手順 1)。文書・Revision・履歴・通知のどれも変わらない。
    /// 発行 View の Selection だけを `selection` (あれば) に更新する。
    Identity {
        /// 発行 View に設定する Selection (`tx.selection_after`)。無ければ変えない。
        selection: Option<Selection>,
    },
    /// 文書を変えた。
    Changed(Applied),
}

/// 適用した変更。呼び出し側が View・Decoration の写像と通知に使う。
#[derive(Clone, Debug)]
pub struct Applied {
    /// 適用の前の Revision。
    pub old_revision: Revision,
    /// 適用の後の Revision。
    pub revision: Revision,
    /// 適用した `ChangeSet` (`old_revision` の文書に対するもの)。
    pub changes: Arc<ChangeSet>,
    /// 変更の出どころ。
    pub origin: Origin,
    /// Undo による変更。
    pub is_undo: bool,
    /// 発行 View に設定する Selection。通常の編集は `selection_after` (なければ `None` = 写像)、
    /// Undo は `selection_before`、Redo は `selection_after`。
    pub selection: Option<Selection>,
    /// 履歴への記録の結果 (Undo / Redo では `None`)。`pruned` が真なら通知を 1 回出す。
    pub record: Option<RecordOutcome>,
}

/// Transaction を適用できない理由。どの場合も文書は変えない。
#[derive(Debug)]
pub enum ApplyError {
    /// `tx.changes.len_before` が文書の長さと違う (古い Revision に基づく)。
    StaleBase {
        /// Transaction が前提とする文書の長さ (バイト)。
        expected: usize,
        /// 今の文書の長さ (バイト)。
        actual: usize,
    },
    /// 読み取り専用、または読込が終わっていない。
    Rejected,
    /// 操作の境界が char 境界でない (文書は変えない)。
    Text(ChangeSetError),
}

/// 表示の設定 (`editor.tab_width`・`view.ambiguous_width`、07 §5.0)。Editor が文書を持つときに設定から写す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplaySettings {
    /// タブの幅 (表示列。`editor.tab_width`)。
    pub tab_width: u8,
    /// East Asian Width が曖昧な文字を幅 2 で数える (`view.ambiguous_width = 2`)。
    pub ambiguous_wide: bool,
    /// 全角スペースとタブを記号で示す (`view.show_whitespace = "minimal"`、03 §13.2)。
    pub show_whitespace: bool,
}

impl Default for DisplaySettings {
    fn default() -> DisplaySettings {
        DisplaySettings {
            tab_width: 4,
            ambiguous_wide: false,
            show_whitespace: true,
        }
    }
}

/// 1 つの文書 (02 §5.1、03 §6)。本文・Revision・Undo 履歴と、ファイルの形式・読み取り専用の理由・読込状態を持つ。
/// 本文を変えるのは `apply` / `undo` / `redo` だけ。
pub struct Document {
    text: Text,
    /// ファイルのパス。無題の文書は `None`。
    pub path: Option<FsPath>,
    /// 最後に読んだ・保存したときのファイルの Stamp (外部変更の照合に使う)。無題・未保存の新規は `None`。
    pub disk_stamp: Option<FileStamp>,
    /// 文書の形式 (文字コード・BOM・改行など)。
    pub format: TextFormat,
    revision: Revision,
    history: UndoHistory,
    /// 読み取り専用の理由 (空なら編集できる。読込中は `load_state` で別に判定する)。
    pub read_only: ReadOnlyReasons,
    /// 読み取り専用属性 (`FileAttribute`) を `file.toggle_read_only` で解除した。上書き保存は「読み取り専用を解除して保存」の確認を経る (03 §9.6)。
    pub attribute_released: bool,
    /// 不正バイト (`InvalidBytes`) を `file.toggle_read_only` の確認の後に解除した (「編集を許可」、03 §8.3.1。v0.4 の
    /// M23)。同じ文書の再読込 (`file.reload`・外部変更からの再読込) では、新しい読込に不正バイトがあっても
    /// `InvalidBytes` を付けずに解除を持ち越す。文字コードを指定して開き直す・新しく開く (どちらも文書を作り直す)・
    /// 復旧の適用で消す。Undo / Redo の対象外 (03 §7.2)。
    pub invalid_bytes_released: bool,
    /// 外部変更の確認で「無視」を選んだときのファイルの状態 (03 §9.5)。フォーカス復帰で同じ状態を見つけても、もう尋ねない。
    /// 保存の成功・再読込で忘れる。
    pub external_seen: Option<FileStamp>,
    /// 読込の状態。
    pub load_state: LoadState,
    /// 読込中の進捗 (百分率)。ステータスバーの `[読込中 n%]` に出す。
    pub load_percent: u8,
    /// 読込中のファイルの大きさ (バイト。読込ジョブの `Progress.total`)。先行表示の間の行番号の欄の幅の推定に使う
    /// (03 §9.2.1 の「ステータスバー」、§11.4)。読込中でなければ使わない。
    pub load_total: u64,
    /// 表示の設定 (設定から写す)。
    pub display: DisplaySettings,
    /// レイアウトの覚え (長い行の表示単位・長大行の列の索引)。読み書きするのは `layout/` だけ。
    pub(crate) layout_cache: std::cell::RefCell<crate::layout::LayoutCache>,
    /// 無題の通し番号 (`無題-{n}` の n、03 §9.8)。`EditorState::add_document` が振る。名前を付けて保存した後も戻さない。
    pub(crate) untitled_no: Option<u32>,
    /// 保持方式の記録の表 (03 §8.7.1、v0.4)。`TextFormat` には入れない (編集のたびに複製されるため)。状態は
    /// `format.roundtrip.preserve`。更新は `apply` と Undo / Redo の適用 (`apply_step`) だけで行う (03 §6.5 手順 4)。
    preserved: PreservedMap,
    /// Undo / Redo の適用で、表と履歴が食い違った回数 (起きないはず。03 §8.7.1 の「食い違い」)。eve-core には
    /// ログの出口がまだ無い (02 §9.2 の `LogSink` は Host の部品) ので、件数だけを数え、debug ビルドでは止める。
    preserved_anomalies: u32,
    /// 標準入力の文書だけ `Some` (`evoeditor -`、03 §9.2.2。v0.4 の M25)。書くのは core だけ。
    pub stdin: Option<StdinDoc>,
}

/// 標準入力の文書の欄 (02 §5.1 の `Document.stdin`、03 §9.2.2。v0.4 の M25)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StdinDoc {
    /// 読んだバイト数 (読込の間は読込ジョブの `Progress.bytes` (受け取った量)、読み終えた後は `StdinDone.bytes` (開いた量))。`status.loading_bytes` と
    /// 「途中まで ({size})」の size に使う。`eve-stdin` が受け渡し待ちで持つ分は含まない。
    pub bytes_read: u64,
    /// 標準入力から読み込んだだけ (`StdinDone` の適用で立て、本文を変える Transaction の適用で外す。`Origin` を問わない。
    /// Undo で取り消しても戻さない)。スワップの対象の判定に使う (03 §10.2)。
    pub pristine: bool,
}

/// 書式の変更が、保持方式の記録の表を丸ごと入れ替えるか (再読込・復旧)。
fn replaces_preserved(fc: &FormatChange) -> bool {
    matches!(
        fc,
        FormatChange::Reload { .. } | FormatChange::Recovery { .. }
    )
}

/// 粗い逆変換 (03 §12.5) の範囲: 変更の最初の始まりから最後の終わりまでを、触れる記録で広げた範囲 (旧の座標) と、
/// その範囲にある記録の複製 (4 本目の申し送り: 範囲の境をまたぐ記録が無いよう `affected_hull` で広げる)。
fn coarse_hull(map: &PreservedMap, cs: &ChangeSet) -> Option<(ByteRange, Vec<PreservedSpan>)> {
    let mut hunks = cs.changed_ranges();
    let first = hunks.next()?;
    let last_end = hunks.last().map_or(first.0.end, |(o, _)| o.end);
    let hull = map.affected_hull(ByteRange::new(first.0.start, last_end));
    Some((hull, map.spans_in(hull)))
}

/// `hull` (旧の座標) を、`cs` を適用した後の座標へ写す。`hull` は全部の変更を含むので、始まりは変わらず、終わりは
/// 長さの差だけずれる。
fn map_hull(hull: ByteRange, cs: &ChangeSet) -> ByteRange {
    let end = (hull.end.0 + cs.len_after()).saturating_sub(cs.len_before());
    ByteRange::new(hull.start, ByteOff(end))
}

/// `eol_stats` の更新の窓 (03 §6.5): 編集ごとに旧 `[s-1, e+1)`・新 `[s-1, s+|x|+1)` を取り、
/// 旧の窓どうしが重なるか接する (間が 1 バイト以下の) ものを 1 つにまとめる (同じ区切りを二重に数えない)。
/// 文書の範囲への切り詰めと CRLF の間での広げは `count_range` が行う。
fn eol_windows(cs: &ChangeSet) -> Vec<((usize, usize), (usize, usize))> {
    let mut out: Vec<((usize, usize), (usize, usize))> = Vec::new();
    for (old, new) in cs.changed_ranges() {
        let o = (old.start.0.saturating_sub(1), old.end.0 + 1);
        let n = (new.start.0.saturating_sub(1), new.end.0 + 1);
        match out.last_mut() {
            Some((po, pn)) if o.0 <= po.1 + 1 => {
                po.1 = o.1;
                pn.1 = n.1;
            }
            _ => out.push((o, n)),
        }
    }
    out
}

impl Document {
    /// 同じ `DocumentId` の旧文書を差し替えるとき、Revision を旧文書の続き (`prev` の次) にする (03 §17.1: 文書ごとに単調増加)。
    pub(crate) fn continue_revision(&mut self, prev: Revision) {
        self.revision = prev.next();
    }

    /// 新規文書 (無題)。履歴の根が保存点。
    pub fn new_untitled() -> Document {
        Document::from_text(Text::new(), TextFormat::new_document())
    }

    /// 読込完了した文書。`format.eol_stats` と `format.long_lines` は文書から数え直す。履歴の根が保存点。
    /// 保持方式の記録の表は空 (試験・新規文書。読込のジョブの結果は `from_loaded` で表を渡す)。
    pub fn from_text(text: Text, mut format: TextFormat) -> Document {
        format.eol_stats = EolStats::count(&text);
        format.long_lines = count_long_lines(&text, format.long_line_threshold);
        Document::from_loaded(text, format, PreservedMap::new())
    }

    /// 読込ジョブが作った本文と形式と、保持方式の記録の表 (`JobResult::Complete` の `preserved`。ジョブが本文と照合済み)
    /// から文書を作る。`format.eol_stats` と `format.long_lines` は、ジョブが復号しながら数えた値をそのまま使う
    /// (メインスレッドで全文を走査し直さない。docs/review/v0.3-findings.md の PRF-17)。
    pub fn from_loaded(text: Text, format: TextFormat, preserved: PreservedMap) -> Document {
        debug_assert_eq!(
            format.eol_stats,
            EolStats::count(&text),
            "読込ジョブの改行の統計"
        );
        debug_assert_eq!(
            format.long_lines,
            count_long_lines(&text, format.long_line_threshold),
            "読込ジョブの長大行の数"
        );
        debug_assert!(
            format.escape_encoding_ok(),
            "escape_encoding の不変条件 (03 §8.3.1)"
        );
        Document {
            text,
            path: None,
            disk_stamp: None,
            format,
            revision: Revision::INITIAL,
            history: UndoHistory::default(),
            read_only: ReadOnlyReasons::default(),
            attribute_released: false,
            invalid_bytes_released: false,
            external_seen: None,
            load_state: LoadState::Ready,
            load_percent: 0,
            load_total: 0,
            display: DisplaySettings::default(),
            layout_cache: std::cell::RefCell::default(),
            untitled_no: None,
            preserved,
            preserved_anomalies: 0,
            stdin: None,
        }
    }

    /// 標準入力から読み込んだだけの印を外す (本文を変えたとき。戻さない)。
    fn clear_stdin_pristine(&mut self) {
        if let Some(s) = self.stdin.as_mut() {
            s.pristine = false;
        }
    }

    /// 保持方式の記録の表 (03 §8.7.1)。状態は `format.roundtrip.preserve`。
    pub fn preserved(&self) -> &PreservedMap {
        &self.preserved
    }

    /// Undo / Redo の適用で、表と履歴が食い違った回数 (試験用。0 のはず)。
    pub fn preserved_anomalies(&self) -> u32 {
        self.preserved_anomalies
    }

    /// 保持方式の記録を捨てる (違う文字コードで保存が成功した、03 §8.7.1 / §8.8)。状態は呼び出し側が
    /// `format.roundtrip` と一緒に決める。履歴のノードが持つ記録 (外した記録・再読込と復旧の前後の表) も捨てる
    /// (Undo で元の文字コードの記録が表に戻らないように)。
    pub(crate) fn clear_preserved(&mut self) {
        self.preserved = PreservedMap::new();
        self.history.forget_preserved();
    }

    /// 違う文字コードで保存が成功した (03 §8.3.1 の「エスケープ文字の由来の文字コード」の表、§8.8): 本文のエスケープ
    /// 文字はもう元の文字コードのバイトではないので `escape_encoding` を `None` にする。履歴の復旧のノードが持つ前後の値も
    /// `None` にする (Undo / Redo で元の文字コードの `Some` が、今の文字コードの文書に戻らないように)。再読込のノードは
    /// 文字コードごと戻すので変えない (`FormatChange::Reload` の書式は文字コードと組で正しい)。
    pub(crate) fn forget_escape_encoding(&mut self) {
        self.format.escape_encoding = None;
        self.history.forget_escape_encoding();
    }

    /// 復旧 (`Origin::Recovery`) の書式の変更 (03 §8.7.1 / §8.3.1 / §10.3): 保持方式の記録を捨てて「保持なし (復旧した)」に
    /// し、`escape_encoding`・`pua_escape` を渡した値にする。`escape_encoding` が `Some` になるときは、読込と同じく内容に
    /// 由来する読み取り専用の理由 `InvalidBytes` を付ける (前から付いていれば残す)。前の値 (表・状態・`escape_encoding`・
    /// `pua_escape`・`InvalidBytes` の有無) は Undo で戻すために持つ。
    pub(crate) fn recovery_change(
        &self,
        escape_encoding: Option<Encoding>,
        pua_escape: PuaEscapeMode,
    ) -> FormatChange {
        self.recovery_change_with(escape_encoding, pua_escape, true)
    }

    /// `recovery_change` の本体。`mark` が偽なら読み取り専用の理由を変えない (`Document::apply` が、書式の変更を
    /// 渡されなかった復旧について、表と状態だけを変える形で作るとき)。
    fn recovery_change_with(
        &self,
        escape_encoding: Option<Encoding>,
        pua_escape: PuaEscapeMode,
        mark: bool,
    ) -> FormatChange {
        let invalid_now = self.read_only.contains(ReadOnlyReason::InvalidBytes);
        FormatChange::Recovery {
            before: PreservedState {
                map: self.preserved.clone(),
                status: self.format.roundtrip.preserve,
                escape_encoding: self.format.escape_encoding,
                pua_escape: self.format.pua_escape,
                invalid_bytes_reason: invalid_now,
            },
            after: PreservedState {
                map: PreservedMap::new(),
                status: PreserveStatus::Unavailable(NoPreserveReason::Recovered),
                escape_encoding,
                pua_escape,
                invalid_bytes_reason: invalid_now || (mark && escape_encoding.is_some()),
            },
        }
    }

    /// 文字コード `out` で書く保存 (と「置換して保存」の対象の判定) に渡す、保持方式の記録の版 (03 §8.6 E2、§9.3 手順 4。
    /// 複製は O(1))。書く文字コードが記録を集めたときの文字コード (= 文書の文字コード。文字コードを変える経路は表を
    /// 捨てる) と同じで、保持が効いている (`Active`) ときだけ表を渡し、ほかは空 (記録を使わない)。
    pub(crate) fn preserved_for(&self, out: eve_encoding::Encoding) -> PreservedMap {
        if out == self.format.encoding
            && matches!(
                self.format.roundtrip.preserve,
                PreserveStatus::Active { .. }
            )
        {
            self.preserved.clone()
        } else {
            PreservedMap::new()
        }
    }

    /// 保存の出力の確認 (03 §8.6 E2 の確認 (i)・(ii)) で保存が止まった: 表と履歴の記録を全部捨てて「保持なし (照合に
    /// 失敗した)」にする (03 §8.7.1 の「保存」)。同じ表を渡し続けると同じ文字コードで二度と保存できないため。次の保存は
    /// 保存の前の確認 (E2') を経て、記録を使わずに (v0.3 と同じ出力で) 書く。
    ///
    /// 記録をもともと持たない文書 (保持の状態が `NotApplicable`。UTF-8 / UTF-16 の文書など) は状態を変えない (03 §8.3.1 の
    /// 「記録が無い文書の `VerifyFailed`」。変えると以後の保存のたびに E2' の確認が出て、文言が事実と合わない)。
    pub(crate) fn abandon_preserved(&mut self) {
        if self.format.roundtrip.preserve == PreserveStatus::NotApplicable {
            return;
        }
        self.clear_preserved();
        self.format.roundtrip.preserve =
            PreserveStatus::Unavailable(NoPreserveReason::VerifyFailed);
    }

    /// 本文。
    pub fn text(&self) -> &Text {
        &self.text
    }

    /// 本文の Snapshot (ジョブに渡す。以後の編集の影響を受けない)。
    pub fn snapshot(&self) -> Snapshot {
        self.text.snapshot()
    }

    /// 今の Revision (本文が変わるたびに増える。03 §17.1)。
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Undo 履歴。
    pub fn history(&self) -> &UndoHistory {
        &self.history
    }

    /// 保存点から変わっている (未保存の変更がある)。
    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// 編集できるか (`read_only` が空かつ `load_state == Ready`)。
    pub fn is_editable(&self) -> bool {
        self.read_only.is_empty() && self.load_state == LoadState::Ready
    }

    /// `file.toggle_read_only` で読み取り専用を解除できるか (when 条件 `doc.read_only_releasable`、07 §7.3。03 §9.6 の
    /// 「M23 の規則」): 読込が済み、理由があり、どれも `FileAttribute` / `UserRequested` / `InvalidBytes` で、
    /// `InvalidBytes` を含むならバイトの文書 (`pua_escape == Enabled` で `escape_encoding` が `Some`)。
    pub fn read_only_releasable(&self) -> bool {
        self.load_state == LoadState::Ready && self.read_only.releasable(&self.format)
    }

    /// 解除を妨げる読み取り専用の理由 (表示の優先順で最初のもの)。パレットで `file.toggle_read_only` が淡色のときの
    /// 理由と、編集の拒否のメッセージの選び分けに使う (03 §8.3.1 の「読み取り専用のときの案内」)。
    pub fn blocking_read_only_reason(&self) -> Option<ReadOnlyReason> {
        self.read_only.blocking(&self.format)
    }

    /// スワップの復旧を提案・適用できる読み取り専用の状態か (03 §10.3): 読み取り専用でない、または理由が
    /// `InvalidBytes` だけでバイトの文書 (v0.4 の M23。不正バイトを含む文書を解除して編集した後のスワップは、次に開くと
    /// `InvalidBytes` で読み取り専用に戻るため)。読み取り専用の文書では、スワップの文字コードが文書の文字コードと同じ
    /// ことも要る (呼び出し側が見る)。読込の状態と dirty も呼び出し側が見る。
    pub fn recovery_allowed(&self) -> bool {
        self.read_only.is_empty() || self.read_only.only_releasable_invalid_bytes(&self.format)
    }

    /// 解除の記録 (`invalid_bytes_released`) を今の理由に当てる: 記録があり、書式を戻した後の文書がバイトの文書
    /// (`pua_escape == Enabled` で `escape_encoding` が `Some`) なら `InvalidBytes` を外す (03 §8.3.1 の「解除の持ち越し」、
    /// §7.2: 解除は Undo / Redo で戻らない)。再読込・復旧のノードの適用と Undo / Redo で書式を戻した後に呼ぶ。
    /// 衝突の文書 (b) になった内容と、エスケープ文字がバイトでない文書 (違う文字コードで保存した後の復旧の Redo 等。
    /// 理由は安全側の後の値のまま) には当てない。
    fn apply_invalid_bytes_release(&mut self) {
        if self.invalid_bytes_released && self.format.is_bytes_document() {
            self.read_only.remove(ReadOnlyReason::InvalidBytes);
        }
    }

    /// Transaction を適用する (03 §6.5 の手順 1〜6・11)。
    ///
    /// # Errors
    ///
    /// `tx` が古い Revision に基づくなら `StaleBase`、編集できない (読み取り専用・読込中。`Origin::Recovery` / `Reload` は除く)
    /// なら `Rejected`、境界が char 境界でなければ `Text`。いずれも文書は変えない。
    pub fn apply(&mut self, tx: &Transaction) -> Result<AppliedTx, ApplyError> {
        // 1. 前提検査
        if tx.changes.len_before() != self.text.len_bytes() {
            return Err(ApplyError::StaleBase {
                expected: tx.changes.len_before(),
                actual: self.text.len_bytes(),
            });
        }
        let exempt = matches!(tx.origin, Origin::Recovery | Origin::Reload);
        if !self.is_editable() && !exempt {
            return Err(ApplyError::Rejected);
        }
        if tx.changes.is_identity() {
            // 本文が同じ再読込: 書式と理由と保持方式の記録だけ差し替える (履歴のノードは作らない)。本文が同じ復旧は
            // 何も変えない (記録は本文と合ったまま)
            // 復旧の `escape_encoding` も変えない: 本文が読んだときのままなので、エスケープ文字の意味も変わらない
            if let Some(fc) = &tx.format_change
                && !matches!(fc, FormatChange::Recovery { .. })
            {
                self.restore_format(fc.side(true));
            }
            return Ok(AppliedTx::Identity {
                selection: tx.selection_after.clone(),
            });
        }
        // 書式の変更。復旧は、保持方式の記録を捨てて「保持なし (復旧した)」にする変更をここで作る (03 §8.7.1 の
        // 「緊急退避・スワップ・復旧」。経路ごとに作らない)。`escape_encoding` を変える復旧は、呼び出し側が
        // `recovery_change` で作って渡す
        let format = match (&tx.format_change, &tx.origin) {
            (None, Origin::Recovery) if !tx.is_undo => Some(self.recovery_change_with(
                self.format.escape_encoding,
                self.format.pua_escape,
                false,
            )),
            (fc, _) => fc.clone(),
        };
        // 粗い逆変換 (03 §12.5) にするときは、編集の前の記録を取り出しておく (変更の境をまたいで外れる記録も含める)
        let hull = (tx.coarse_undo && !tx.is_undo && !self.preserved.is_empty())
            .then(|| coarse_hull(&self.preserved, &tx.changes))
            .flatten();
        // 2.〜4. 適用前の Snapshot、逆変換、適用。`Text::apply` は検査を先に行い、失敗では何も変えない
        let before = self.text.snapshot();
        self.text.apply(&tx.changes).map_err(ApplyError::Text)?;
        // 本文を変えたので、標準入力から読み込んだだけの印を外す (03 §9.2.2 の「最初の編集」)
        self.clear_stdin_pristine();
        let coarse = tx
            .coarse_undo
            .then(|| super::coarse_inverse(&tx.changes, &before, &self.text))
            .flatten();
        let is_coarse = coarse.is_some();
        let inverse = coarse.unwrap_or_else(|| tx.changes.invert(&before));
        self.update_eol_stats(&before, &tx.changes);
        // 4. 保持方式の記録の写像 (03 §8.7.1 の「編集での写像」)。表を入れ替える変更 (再読込・復旧) は写さない
        let preserved = if format.as_ref().is_some_and(replaces_preserved) {
            PreservedUndo::None
        } else if tx.is_undo {
            // 履歴の外から来た Undo / Redo の Transaction: 外さずに位置だけを写す
            let odd = self.preserved.apply_undo(&tx.changes, &[]);
            self.note_anomaly(&odd);
            PreservedUndo::None
        } else {
            let removed = self.preserved.apply_edit(&tx.changes);
            match hull {
                Some((hull, spans)) if is_coarse => PreservedUndo::Range {
                    after: map_hull(hull, &tx.changes),
                    hull_len: hull.len(),
                    before: spans.into(),
                    removed: removed.into(),
                },
                _ => PreservedUndo::removed(removed),
            }
        };
        // 5. Revision
        let old_revision = self.revision;
        self.revision = self.revision.next();
        // 6. 履歴への記録 (Undo / Redo の Transaction は `undo` / `redo` を使う)
        let after_format = format.as_ref().map(|fc| fc.side(true));
        let record = if tx.is_undo {
            None
        } else {
            Some(self.history.record_with(tx, inverse, format, preserved))
        };
        // 再読込・改行コードの変換・復旧: 書式 (と保持方式の記録) を新しいものに差し替える (前後の値は Undo ノードが持つ)
        if let Some(f) = after_format {
            self.restore_format(f);
        }
        self.carry_layout(&before, &tx.changes, old_revision);
        Ok(AppliedTx::Changed(Applied {
            old_revision,
            revision: self.revision,
            changes: Arc::clone(&tx.changes),
            origin: tx.origin.clone(),
            is_undo: tx.is_undo,
            selection: tx.selection_after.clone(),
            record,
        }))
    }

    /// Undo。取り消すものがなければ `None`。読み取り専用では `Err(Rejected)` (Reload・Recovery のノードは例外にしない:
    /// 03 §7.2 の `edit.undo` の when は `!doc.read_only`)。
    ///
    /// # Errors
    ///
    /// 編集できない (読み取り専用・読込中) なら `Rejected`。
    pub fn undo(&mut self) -> Result<Option<Applied>, ApplyError> {
        if !self.is_editable() {
            return Err(ApplyError::Rejected);
        }
        let Some(step) = self.history.undo_step() else {
            return Ok(None);
        };
        Ok(Some(self.apply_step(
            step.changes,
            step.origin,
            step.selection,
            step.format,
            &step.preserved,
            false,
        )?))
    }

    /// Redo。やり直すものがなければ `None`。
    ///
    /// # Errors
    ///
    /// 編集できない (読み取り専用・読込中) なら `Rejected`。
    pub fn redo(&mut self) -> Result<Option<Applied>, ApplyError> {
        if !self.is_editable() {
            return Err(ApplyError::Rejected);
        }
        let Some(step) = self.history.redo_step() else {
            return Ok(None);
        };
        Ok(Some(self.apply_step(
            step.changes,
            step.origin,
            step.selection,
            step.format,
            &step.preserved,
            true,
        )?))
    }

    /// Undo (`redo == false`) / Redo の適用。本文・書式と一緒に、保持方式の記録を戻す・外す (03 §8.7.1 の「Undo / Redo」)。
    fn apply_step(
        &mut self,
        changes: Arc<ChangeSet>,
        origin: Origin,
        selection: Option<Selection>,
        format: Option<FormatRestore>,
        preserved: &PreservedUndo,
        redo: bool,
    ) -> Result<Applied, ApplyError> {
        let before = self.text.snapshot();
        // 履歴と文書が食い違った場合も、文書は変えない (履歴の不整合は呼び出しの誤り)
        self.text.apply(&changes).map_err(ApplyError::Text)?;
        self.clear_stdin_pristine();
        self.update_eol_stats(&before, &changes);
        // 保持方式の記録: 表を入れ替えるノード (再読込・復旧) は下の書式と一緒に入れ替える。ほかは写像で戻す・外す
        let swaps = matches!(
            format,
            Some(FormatRestore::State(_) | FormatRestore::Preserved(_))
        );
        if !swaps {
            let odd = match (preserved, redo) {
                (PreservedUndo::None, false) => self.preserved.apply_undo(&changes, &[]),
                (PreservedUndo::None, true) => self.preserved.apply_redo(&changes, &[]),
                (PreservedUndo::Removed(r), false) => self.preserved.apply_undo(&changes, r),
                (PreservedUndo::Removed(r), true) => self.preserved.apply_redo(&changes, r),
                (
                    PreservedUndo::Range {
                        after,
                        hull_len,
                        before,
                        ..
                    },
                    false,
                ) => {
                    self.preserved.replace_range(*after, *hull_len, before);
                    Vec::new()
                }
                (PreservedUndo::Range { removed, .. }, true) => {
                    self.preserved.apply_redo(&changes, removed)
                }
            };
            self.note_anomaly(&odd);
        }
        // 再読込・改行コードの変換・復旧のノードは、書式も一緒に戻す (03 §7.2)
        if let Some(f) = format {
            self.restore_format(f);
        }
        let old_revision = self.revision;
        self.revision = self.revision.next();
        self.carry_layout(&before, &changes, old_revision);
        Ok(Applied {
            old_revision,
            revision: self.revision,
            changes,
            origin,
            is_undo: true,
            selection,
            record: None,
        })
    }

    /// 書式を戻す (03 §7.2): 再読込は `TextFormat` 全体と読み取り専用の理由と保持方式の記録、改行コードの変換は
    /// `default_eol` だけ、復旧は保持方式の記録と状態だけ。
    fn restore_format(&mut self, f: FormatRestore) {
        match f {
            FormatRestore::State(st) => {
                self.format = st.format;
                self.read_only = st.read_only;
                self.preserved = st.preserved;
            }
            FormatRestore::DefaultEol(eol) => self.format.default_eol = eol,
            FormatRestore::Preserved(st) => {
                self.preserved = st.map;
                self.format.roundtrip.preserve = st.status;
                self.format.escape_encoding = st.escape_encoding;
                self.format.pua_escape = st.pua_escape;
                if st.invalid_bytes_reason {
                    self.read_only.insert(ReadOnlyReason::InvalidBytes);
                } else {
                    self.read_only.remove(ReadOnlyReason::InvalidBytes);
                }
            }
        }
        // 「編集を許可」は文書の状態で、再読込・復旧のノードの Undo / Redo でも戻らない (03 §7.2)
        self.apply_invalid_bytes_release();
        debug_assert!(
            self.format.escape_encoding_ok(),
            "escape_encoding の不変条件 (03 §8.3.1): {:?} {:?} {:?}",
            self.format.escape_encoding,
            self.format.encoding,
            self.format.pua_escape
        );
    }

    /// Undo / Redo の適用で表と履歴が食い違った (正しく使えば起きない)。本文を含めずに数え、debug ビルドでは止める。
    fn note_anomaly(&mut self, odd: &[PreservedSpan]) {
        if !odd.is_empty() {
            self.preserved_anomalies = self.preserved_anomalies.saturating_add(1);
            debug_assert!(
                false,
                "保持方式の記録が履歴と食い違った ({} 件、{:?})",
                odd.len(),
                odd.first()
            );
        }
    }

    /// 再読込の完了 (03 §9.5): 読み込んだ内容がディスクの状態なので、ここを保存点にして `Ready` に戻す。
    pub fn finish_reload(&mut self, stamp: FileStamp) {
        self.history.seal();
        self.history.mark_saved();
        self.disk_stamp = Some(stamp);
        self.load_state = LoadState::Ready;
    }

    /// 保存の開始 (03 §9.3 手順 3): 履歴を封印し、保存に使う Snapshot とその時点の履歴上の位置を返す。
    pub fn begin_save(&mut self) -> (Snapshot, NodeId) {
        self.history.seal();
        (self.text.snapshot(), self.history.current())
    }

    /// 保存の完了: 保存に使った位置を保存点にする (その位置が履歴に無ければ保存点なし、INV-H3)。
    pub fn finish_save(&mut self, at: NodeId, stamp: Option<FileStamp>) {
        self.history.mark_saved_at(at);
        if stamp.is_some() {
            self.disk_stamp = stamp;
            self.external_seen = None;
        }
    }

    /// プロセス全体の Undo の上限のために、最も古い履歴を 1 つ破棄する。解放した会計値を返す。dirty 表示は変わらない。
    pub fn trim_history_oldest(&mut self) -> Option<usize> {
        self.history.trim_oldest()
    }

    /// 履歴の封印 (カーソル移動など)。
    pub fn seal_history(&mut self) {
        self.history.seal();
    }

    /// 手順 4 の `eol_stats` と `long_lines` の更新 (INV-D5): 窓ごとに、旧文書と新文書の改行を数え直して差し替える。
    fn update_eol_stats(&mut self, before: &Snapshot, cs: &ChangeSet) {
        let windows = eol_windows(cs);
        let mut stats = self.format.eol_stats;
        for (old, new) in &windows {
            let o = count_range(before, old.0, old.1);
            let n = count_range(&self.text, new.0, new.1);
            stats = stats.replaced(o, n);
        }
        self.format.eol_stats = stats;
        // 長大行の数: 同じ窓 (端の前後 1 バイトを含む) が触れた行だけを、変更の前後で数えて差し替える。
        // 窓の端の行を含めるのは、CR と LF の合体・分離で隣の行の境目が動くため (SAF-13)
        let th = self.format.long_line_threshold;
        let old = long_lines_touched(before, windows.iter().map(|(o, _)| *o), th);
        let new = long_lines_touched(&self.text, windows.iter().map(|(_, n)| *n), th);
        self.format.long_lines = (self.format.long_lines + new).saturating_sub(old);
    }

    /// 本文に `cs` を当てて Revision を `old` から進めた後に、長大行の列の索引の覚えを引き継ぐか捨てる (03 §14.4 の
    /// 「編集」、02 §5.1)。閾値は書式を差し替えた (再読込) 後の値。
    fn carry_layout(&mut self, before: &Snapshot, cs: &ChangeSet, old: Revision) {
        let threshold = self.format.long_line_threshold;
        self.layout_cache.get_mut().on_change(
            before,
            &self.text,
            cs,
            (old, self.revision),
            threshold,
        );
    }

    /// 文書全体から数え直した値 (検証用)。
    pub fn recount_eol(&self) -> EolStats {
        EolStats::count(&self.text)
    }

    /// 文書の内容全体 (小さな文書・テスト用)。
    pub fn to_string_lossy(&self) -> String {
        self.text
            .slice_to_string(eve_text::ByteRange::all(self.text.len_bytes()))
    }

    /// 本文の長さ (バイト)。
    pub fn len_bytes(&self) -> usize {
        self.text.len_bytes()
    }

    /// バイト位置 `i` を文書の長さに切り詰めた `ByteOff` (char 境界には揃えない)。
    pub fn byte_off(&self, i: usize) -> ByteOff {
        ByteOff(i.min(self.text.len_bytes()))
    }
}
