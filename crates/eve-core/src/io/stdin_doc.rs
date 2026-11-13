//! 標準入力の文書の core の側 (`evoeditor -`、docs/03 §9.2.2。v0.4 の M25 の (8)「標準入力の土台」と (9)「標準入力の公開」)。
//!
//! 標準入力の読込は、ジョブ表で **ファイルの読込 (`Pending::Open`) とは別の種類** (`Pending::OpenStdin`) に持つ。
//! 置き換えの読込中の判定 (`open_in_progress`)・ファイルの読込の取消し・ステータスバーの割合 (`loading_percent`)
//! はどれも `Pending::Open` だけを見るので、閉じないパイプの読込はそれらに混ざらない。
//!
//! - **入口**: `open_initial_files` の `InitialTarget::Stdin` (02 §5.4)。端末なら読まずに空の無題の文書と
//!   `info.stdin.terminal`、端末でなければ `Loading` の無題の文書と `JobKind::OpenStdin`。
//! - **終端** (`StdinDone`): 読み終えた (`end` のどれでも、1 バイト以上読んだ) 文書は変更あり (保存点の無い履歴)。
//!   `--readonly` (理由 `UserRequested`) は変更なしで、解除した時点で変更ありにする (`io/file_cmds.rs`)。`end` ごとに
//!   メッセージを出し分け、書く手段の無い内容には `msg.stdin.unsaveable` を足す。
//! - **止める・捨てる操作の確認** (OQ-219 の 10): 表示中の `Esc` は `ui.confirm.stdin_stop`、1 バイト以上読んだ読込中の
//!   文書を閉じる・終了するときは `ui.confirm.stdin_loading` (閉じる・終了の連鎖に入る)。
//! - **パニック** (`Failed`): 読んだ内容は失い、文書を閉じる (`msg.stdin.failed`)。

use eve_view::{DocumentId, JobId, MessageLevel, ViewId};

use crate::command::CommandResult;
use crate::document::{Document, LoadState, ReadOnlyReason, StdinDoc};
use crate::editor::state::EditorState;
use crate::io::io_jobs::OpenOptions;
use crate::io::open::human_size;
use crate::io::pending::Pending;
use crate::jobs::{JobKind, JobResult, StdinEnd};
use crate::ui_model::keys;
use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState, Continuation, Overlay};

use super::file::{fs_reason, reason_label};

/// `ui.confirm.stdin_stop` の選択肢 (読み続ける (既定。`Esc` と同じ) / ここまでで開く)。
const STOP_CHOICES: [ChoiceDef; 2] = [
    ChoiceDef {
        id: "continue",
        accel: None,
        destructive: false,
    },
    ChoiceDef {
        id: "open",
        accel: Some('o'),
        destructive: false,
    },
];

/// `ui.confirm.stdin_loading` の選択肢 (ここまでで開く (既定) / 破棄。キャンセルは共通の `ui.confirm.cancel`)。
const LOADING_CHOICES: [ChoiceDef; 2] = [
    ChoiceDef {
        id: "open",
        accel: Some('o'),
        destructive: false,
    },
    ChoiceDef {
        id: "discard",
        accel: Some('d'),
        destructive: true,
    },
];

impl EditorState {
    /// 標準入力を読む無題の文書を `Loading` で足し、`JobKind::OpenStdin` を投入する (03 §9.2.2 の「起動の経路」)。
    /// 確認を出さずに読む (`read_only_requested`)。先行表示はしない。`opts` の `forced` (`--encoding`) と
    /// `user_read_only` (`--readonly`) はそのまま使い、設定の値 (改行・上限・長大行の閾値) は指定が無ければ設定から入れる。
    /// 最初の Window が無ければ、この文書を表示する。足した文書を返す。
    ///
    /// `open_initial_files` の `InitialTarget::Stdin { terminal: false }` から呼ぶ (試験も直接使う)。
    #[doc(hidden)]
    pub fn open_stdin(&mut self, opts: OpenOptions) -> DocumentId {
        let opts = OpenOptions {
            read_only_requested: true,
            partial: false,
            default_eol: opts.default_eol.or(Some(self.config_eol())),
            huge_file_warn_bytes: opts
                .huge_file_warn_bytes
                .or(Some(self.config().huge_file_warn_bytes)),
            long_line_threshold: opts
                .long_line_threshold
                .or(Some(self.config_long_line_threshold())),
            ..opts
        };
        let mut doc = self.blank_document();
        doc.load_state = LoadState::Loading;
        doc.stdin = Some(StdinDoc::default());
        let (doc_id, view) = self.add_document(doc);
        if self.focused_window().is_none() {
            self.show_view(view, None);
        }
        let job = self.jobs.spawn(
            Some(doc_id),
            eve_view::Revision::INITIAL,
            JobKind::OpenStdin { opts },
        );
        self.io
            .pending
            .insert(job, Pending::OpenStdin { doc: doc_id, view });
        doc_id
    }

    /// 標準入力が端末 (`InitialTarget::Stdin { terminal: true }`): 読まずに空の無題の文書を開き、`info.stdin.terminal`
    /// を出す (03 §9.2.2 の「標準入力が端末」、01 FR-120 AC-2)。最初の Window が無ければ、この文書を表示する。
    pub(crate) fn open_stdin_terminal(&mut self) -> DocumentId {
        let doc = self.blank_document();
        let (doc_id, view) = self.add_document(doc);
        if self.focused_window().is_none() {
            self.show_view(view, None);
        }
        self.show_message(MessageLevel::Info, keys::INFO_STDIN_TERMINAL, &[]);
        doc_id
    }

    /// 文書 `doc` の標準入力の読込のジョブ (読込中なら)。
    pub(crate) fn stdin_job_of(&self, doc: DocumentId) -> Option<JobId> {
        self.io.pending.iter().find_map(|(id, p)| match p {
            Pending::OpenStdin { doc: d, .. } if *d == doc => Some(*id),
            _ => None,
        })
    }

    /// 1 バイト以上読んだ、読込中の標準入力の文書か (閉じる・終了の確認の対象、03 §9.2.2 の「閉じる・終了」)。
    pub(crate) fn stdin_loading_with_bytes(&self, doc: DocumentId) -> bool {
        self.stdin_job_of(doc).is_some()
            && self
                .docs
                .documents
                .get(&doc)
                .and_then(|d| d.stdin)
                .is_some_and(|s| s.bytes_read > 0)
    }

    /// 標準入力の文書の読んだ量 (`{size}`。読込中は受け取った量、読み終えた後は開いた量)。
    pub(crate) fn stdin_size_label(&self, doc: DocumentId) -> String {
        let bytes = self
            .docs
            .documents
            .get(&doc)
            .and_then(|d| d.stdin)
            .map_or(0, |s| s.bytes_read);
        human_size(bytes)
    }

    /// 途中までの文書 (`LoadIncomplete`) の「途中まで ({size})」の大きさ。途中までの文書でなければ `None`
    /// (03 §9.2.2 の「`LoadIncomplete` の外し方」。名前を付けて保存の入力の案内と FR-107 の確認の文書名に足す)。
    pub(crate) fn partial_size(&self, doc: DocumentId) -> Option<String> {
        let d = self.docs.documents.get(&doc)?;
        d.read_only
            .contains(ReadOnlyReason::LoadIncomplete)
            .then(|| self.stdin_size_label(doc))
    }

    /// `Esc` (`ui.cancel`) の標準入力の層 (03 §9.2.2 の「取消し (`Esc`)」、OQ-219 の 10 (i)): 表示中の文書が読込中の
    /// 標準入力の文書なら、読むのをやめるかの確認 (`ui.confirm.stdin_stop`) を出して `true`。ほかの文書を表示している
    /// ときは標準入力の読込に触れない (`false`)。
    pub(crate) fn confirm_stdin_stop(&mut self) -> bool {
        let Some(doc) = self.active_document() else {
            return false;
        };
        if self.stdin_job_of(doc).is_none() {
            return false;
        }
        self.open_confirm(ConfirmState {
            name: "stdin_stop",
            args: vec![("size".to_owned(), self.stdin_size_label(doc))],
            choices: STOP_CHOICES.to_vec(),
            details: Vec::new(),
            default: 0,
            focused: 0,
            message: None,
            action: ConfirmAction::StdinStop(doc),
        });
        true
    }

    /// `ui.confirm.stdin_stop` の答え。「ここまでで開く」で読むのをやめる (終端は `StdinDone { Stopped }`)。
    /// 「読み続ける」と `Esc` は何もしない。確認の間に読み終えていれば何もしない。
    pub(crate) fn choose_stdin_stop(&mut self, doc: DocumentId, id: &str) -> CommandResult {
        if id == "open"
            && let Some(job) = self.stdin_job_of(doc)
        {
            self.jobs.cancel(job);
        }
        CommandResult::Done
    }

    /// 閉じる・終了で、1 バイト以上読んだ読込中の標準入力の文書なら確認 (`ui.confirm.stdin_loading`) を出して `true`
    /// (03 §9.2.2 の「閉じる・終了」、OQ-219 の 10 (ii))。終了の連鎖で 2 件以上を尋ねるときは何件目かを前置する。
    pub(crate) fn confirm_stdin_loading(&mut self, doc: DocumentId, cont: Continuation) -> bool {
        if !self.stdin_loading_with_bytes(doc) {
            return false;
        }
        let mut args = vec![("size".to_owned(), self.stdin_size_label(doc))];
        if cont == Continuation::Quit
            && let Some((index, total)) = self.quit_chain_position()
        {
            args.push(("index".to_owned(), index.to_string()));
            args.push(("total".to_owned(), total.to_string()));
        }
        self.open_confirm(ConfirmState {
            name: "stdin_loading",
            args,
            choices: LOADING_CHOICES.to_vec(),
            details: Vec::new(),
            default: 0,
            focused: 0,
            message: None,
            action: ConfirmAction::StdinLoading { doc, cont },
        });
        true
    }

    /// `ui.confirm.stdin_loading` の答え (キャンセルは呼び側)。「ここまでで開く」は読むのをやめ、読み終えた
    /// (`StdinDone`) ところで続き (`cont`) を進める: 開いた文書は変更ありなので、閉じる・終了は FR-107 の確認に進む。
    /// 「破棄」は、閉じるなら読むのをやめて内容を捨て、終了なら終了の連鎖の破棄の印を付ける (終了が確定するまで
    /// 読み続ける)。確認の間に読み終えていたら、続きをそのまま進める。
    pub(crate) fn choose_stdin_loading(
        &mut self,
        doc: DocumentId,
        cont: Continuation,
        id: &str,
    ) -> CommandResult {
        match (id, &cont) {
            ("open", _) => match self.stdin_job_of(doc) {
                Some(job) => {
                    self.ui.continuations.insert(job, cont);
                    self.jobs.cancel(job);
                    CommandResult::Done
                }
                None => self.run_stdin_continuation(doc, cont),
            },
            (_, Continuation::Quit) => self.quit_discard_head(),
            (_, Continuation::Close(d)) => self.close_buffer(*d),
            (_, Continuation::Reopen(..)) => CommandResult::Done,
        }
    }

    /// 「ここまでで開く」の後の続き。オーバーレイ・検索バーの間は進めずに待ち、閉じた後 (`flush_open_switches` と同じ時期)
    /// に進める (開いているほかの確認・入力を FR-107 の確認で置き換えない。03 §9.2.2 の「閉じる・終了」、§9.2.1 の
    /// 「表示の切り替え」と同じ保留)。
    fn continue_after_stdin(&mut self, doc: DocumentId, cont: Continuation) -> CommandResult {
        if self.auto_switch_blocked() {
            self.io.stdin_conts.push((doc, cont));
            return CommandResult::Done;
        }
        self.run_stdin_continuation(doc, cont)
    }

    /// 保留した「ここまでで開く」の後の続きを進める (オーバーレイ・検索バーが閉じた後)。
    pub(crate) fn flush_stdin_continuations(&mut self) {
        while !self.auto_switch_blocked() && !self.io.stdin_conts.is_empty() {
            let (doc, cont) = self.io.stdin_conts.remove(0);
            self.run_stdin_continuation(doc, cont);
        }
    }

    /// 「ここまでで開く」の後の続き: 閉じるなら閉じる要求をもう一度 (今度は FR-107 の確認)、終了なら連鎖を進める。
    /// 開いた文書が変更なし (`--readonly` で 1 バイト以上読んだ文書) なら、続きは進めずに文書を残す (終了なら連鎖を
    /// 止める)。FR-107 の確認が出ずに閉じると「ここまでで開く」が「破棄」と同じになるため (03 §9.2.2 の「閉じる・終了」)。
    /// 文書が既に無い (パニックで閉じた) ときは、終了なら連鎖の次へ進み、閉じるなら何もしない。
    fn run_stdin_continuation(&mut self, doc: DocumentId, cont: Continuation) -> CommandResult {
        if !self.docs.documents.contains_key(&doc) {
            return match cont {
                Continuation::Quit => self.advance_quit_chain(),
                Continuation::Close(_) | Continuation::Reopen(..) => CommandResult::Done,
            };
        }
        let kept = self
            .docs
            .documents
            .get(&doc)
            .is_some_and(|d| d.stdin_read_any() && !d.is_dirty());
        if kept {
            if cont == Continuation::Quit {
                self.ui.quit_chain = None;
            }
            return CommandResult::Done;
        }
        match cont {
            Continuation::Quit => self.advance_quit_chain(),
            Continuation::Close(_) => self.request_close(doc),
            Continuation::Reopen(..) => CommandResult::Done,
        }
    }

    /// 標準入力の読込の進捗: 読んだバイト数を文書に写す (割合は出さない。03 §9.2.2 の「文書」)。
    pub(crate) fn on_stdin_progress(&mut self, doc: DocumentId, bytes: u64) {
        if let Some(s) = self
            .docs
            .documents
            .get_mut(&doc)
            .and_then(|d| d.stdin.as_mut())
        {
            s.bytes_read = bytes;
            self.mark_dirty_render();
        }
    }

    /// 標準入力の読込の終端 (02 §9.3 の採否表の `OpenStdin` の行)。`StdinDone` は取消しの後でも採用する (取消しは
    /// 「読むのをやめて開く」意味)。`Failed` (パニック) は読んだ内容を失い、文書を閉じる。`cont` は閉じる・終了の確認で
    /// 「ここまでで開く」を選んだときの続き。
    pub(crate) fn finish_open_stdin(
        &mut self,
        doc_id: DocumentId,
        view: ViewId,
        cont: Option<Continuation>,
        r: JobResult,
    ) {
        match r {
            JobResult::StdinDone {
                text,
                format,
                detected,
                preserved,
                bytes,
                end,
            } => {
                let mut d = Document::from_loaded(text, format, preserved);
                d.read_only = detected;
                // 読んだバイト数は開いた量にする (「途中まで ({size})」の size)
                d.stdin = Some(StdinDoc {
                    bytes_read: bytes,
                    pristine: true,
                });
                let read_any = d.stdin_read_any();
                // 変更あり (03 §9.2.2 の「変更あり」。OQ-219 の 4): 1 バイト以上読んだら保存点の無い履歴にする。
                // `--readonly` (理由 `UserRequested`) は変更なし (解除した時点で変更ありにする)
                if read_any && !d.read_only.contains(ReadOnlyReason::UserRequested) {
                    d.clear_save_point();
                }
                let unsaveable = unsaveable(&d);
                // Revision は読込中の文書の続き、無題の番号は引き継ぐ (`put_document`)
                self.put_document(doc_id, d);
                self.complete_open(doc_id, view, None);
                self.close_stdin_stop_confirm(doc_id);
                self.stdin_end_message(&end, bytes, read_any);
                if unsaveable {
                    self.show_message(MessageLevel::Warn, keys::MSG_STDIN_UNSAVEABLE, &[]);
                }
                if let Some(c) = cont {
                    self.continue_after_stdin(doc_id, c);
                }
                // 保存を中止して終了の最中: 読み終えた文書 (変更あり) を退避の対象に入れてから終了する (03 §9.7 (f) 3)
                self.quit_abort_check();
            }
            JobResult::Failed { err } => {
                self.stdin_load_failed(doc_id, &reason_label(&err));
                // 閉じる・終了の確認で「ここまでで開く」を選んでいたら、続きを進める (文書は閉じたので、終了なら連鎖の次へ、
                // 閉じるなら済み。`StdinDone` と同じ規則)
                if let Some(c) = cont {
                    self.continue_after_stdin(doc_id, c);
                }
                // 保存を中止して終了の最中: 読んだ内容は失ったまま終了へ進む (03 §9.7 (f) 3)
                self.quit_abort_check();
            }
            // 終端は `StdinDone` か `Failed` だけ (02 §9.3 の採否表)。ほかが届いたら実装の誤りで、文書を `Loading` のまま
            // 残さないよう、失敗として閉じる
            other => {
                debug_assert!(false, "標準入力の読込の終端ではない結果: {other:?}");
                self.stdin_load_failed(doc_id, "unexpected result");
            }
        }
    }

    /// 読み終えたときのメッセージ (03 §9.2.2 の「大きさの上限」「取消し」「読込の失敗」、04 §15.2)。EOF は出さない。
    /// 何も読まずに取り消したときも出さない (理由の無い空の文書)。
    fn stdin_end_message(&mut self, end: &StdinEnd, bytes: u64, read_any: bool) {
        let size = human_size(bytes);
        match end {
            StdinEnd::Eof => {}
            StdinEnd::Stopped if read_any => {
                self.show_message(
                    MessageLevel::Info,
                    keys::INFO_STDIN_STOPPED,
                    &[("size", &size)],
                );
            }
            StdinEnd::Stopped => {}
            StdinEnd::Limit => {
                let limit = human_size(self.config().huge_file_warn_bytes);
                self.show_message(
                    MessageLevel::Warn,
                    keys::MSG_STDIN_TRUNCATED,
                    &[("size", &limit)],
                );
            }
            // 1 バイトも受け取らずに読めなかった・確保に失敗した: 開いた文書は理由の無い空の文書なので、「読み取り専用で
            // 開いた」と言わずに `msg.stdin.unreadable` (03 §9.2.2 の「読込の失敗」)
            StdinEnd::Error(e) if !read_any => {
                let reason = fs_reason(e);
                self.show_message(
                    MessageLevel::Error,
                    keys::MSG_STDIN_UNREADABLE,
                    &[("reason", &reason)],
                );
            }
            StdinEnd::OutOfMemory { .. } if !read_any => {
                self.show_message(
                    MessageLevel::Error,
                    keys::MSG_STDIN_UNREADABLE,
                    &[("reason", "out of memory")],
                );
            }
            StdinEnd::Error(e) => {
                let reason = fs_reason(e);
                self.show_message(
                    MessageLevel::Error,
                    keys::MSG_STDIN_READ_FAILED,
                    &[("size", &size), ("reason", &reason)],
                );
            }
            StdinEnd::OutOfMemory { .. } => {
                self.show_message(
                    MessageLevel::Error,
                    keys::MSG_STDIN_OUT_OF_MEMORY,
                    &[("size", &size)],
                );
            }
        }
    }

    /// 読み終えたら、その文書の `ui.confirm.stdin_stop` は答える意味が無いので閉じる。
    fn close_stdin_stop_confirm(&mut self, doc: DocumentId) {
        if matches!(
            &self.ui.overlay,
            Some(Overlay::Confirm(c)) if matches!(c.action, ConfirmAction::StdinStop(d) if d == doc)
        ) {
            self.take_overlay();
            self.mark_dirty_render();
        }
    }

    /// 標準入力の読込の失敗 (パニック): 読んだ内容は失い、文書を閉じる (表示していたら §9.8 の順で別の文書を表示する)。
    fn stdin_load_failed(&mut self, doc_id: DocumentId, reason: &str) {
        self.show_message(
            MessageLevel::Error,
            keys::MSG_STDIN_FAILED,
            &[("reason", reason)],
        );
        self.drop_failed_open(doc_id, None);
    }
}

/// 書く手段の無い内容か (03 §9.2.2 の「書く手段の無い文書」): 不正バイトを含む状態付き符号化 (ISO-2022-JP。どの文字
/// コードでも書けない) と、私用文字と衝突した文書 (b) (不正バイトと真の私用文字を区別できない)。ファイルでは開き直しで
/// 回復するが、標準入力は開き直せない。
fn unsaveable(d: &Document) -> bool {
    d.format.invalid_bytes > 0
        && (d.format.encoding.is_stateful()
            || d.format.pua_escape == eve_encoding::PuaEscapeMode::DisabledCollision)
}
