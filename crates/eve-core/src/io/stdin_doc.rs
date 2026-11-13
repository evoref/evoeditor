//! 標準入力の文書の core の側 (`evoeditor -`、docs/03 §9.2.2。v0.4 の M25 の (8)「標準入力の土台」)。
//!
//! 標準入力の読込は、ジョブ表で **ファイルの読込 (`Pending::Open`) とは別の種類** (`Pending::OpenStdin`) に持つ。
//! 置き換えの読込中の判定 (`open_in_progress`)・`Esc` の取消し (`cancel_loading`)・ステータスバーの割合 (`loading_percent`)
//! はどれも `Pending::Open` だけを見るので、閉じないパイプの読込はそれらに混ざらない。
//!
//! (8) の範囲は、文書を `Loading` で作ってジョブを投入し、進捗 (読んだバイト数) と終端 (`StdinDone`) を文書に写すまで。
//! 入口 (`open_initial_files` の標準入力の種類)・変更ありの扱い・メッセージ・`Esc` と閉じる・終了の確認・緊急退避と
//! スワップの扱いは (9) で足す (08 §2.5 の M25 の行)。それまで CLI の `-` は「未対応」のままで、ここは試験だけが使う。

use eve_view::{DocumentId, MessageLevel, ViewId};

use crate::document::{Document, LoadState, StdinDoc};
use crate::editor::state::EditorState;
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;
use crate::io::pending::Pending;
use crate::jobs::{JobKind, JobResult};
use crate::ui_model::keys;

use super::file::reason_label;

impl EditorState {
    /// 標準入力を読む無題の文書を `Loading` で足し、`JobKind::OpenStdin` を投入する (03 §9.2.2 の「起動の経路」)。
    /// 確認を出さずに読む (`read_only_requested`)。先行表示はしない。`opts` の `forced` (`--encoding`) と
    /// `user_read_only` (`--readonly`) はそのまま使い、設定の値 (改行・上限・長大行の閾値) は指定が無ければ設定から入れる。
    /// 最初の Window が無ければ、この文書を表示する。足した文書を返す。
    ///
    /// M25 の (9) で `open_initial_files` の標準入力の種類から呼ぶ。それまでは試験だけが使う (CLI の `-` は未対応のまま)。
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
    /// 「読むのをやめて開く」意味)。`Failed` (パニック) は読んだ内容を失い、文書を閉じる。
    pub(crate) fn finish_open_stdin(&mut self, doc_id: DocumentId, view: ViewId, r: JobResult) {
        match r {
            JobResult::StdinDone {
                text,
                format,
                detected,
                preserved,
                bytes,
                end: _,
            } => {
                // 読込の終わり方 (`end`) で出し分けるメッセージ (`msg.stdin.*` / `info.stdin.*`) と変更ありの扱いは
                // M25 の (9) で足す。読んだバイト数は開いた量にする (「途中まで ({size})」の size)
                let bytes_read = bytes;
                let mut d = Document::from_loaded(text, format, preserved);
                d.read_only = detected;
                d.stdin = Some(StdinDoc {
                    bytes_read,
                    pristine: true,
                });
                // Revision は読込中の文書の続き、無題の番号は引き継ぐ (`put_document`)
                self.put_document(doc_id, d);
                self.complete_open(doc_id, view, None);
            }
            JobResult::Failed { err } => self.stdin_load_failed(doc_id, &reason_label(&err)),
            // 終端は `StdinDone` か `Failed` だけ (02 §9.3 の採否表)。ほかが届いたら実装の誤りで、文書を `Loading` のまま
            // 残さないよう、失敗として閉じる
            other => {
                debug_assert!(false, "標準入力の読込の終端ではない結果: {other:?}");
                self.stdin_load_failed(doc_id, "unexpected result");
            }
        }
    }

    /// 標準入力の読込の失敗 (パニックなど): 読んだ内容は失い、文書を閉じる (表示していたら §9.8 の順で別の文書を表示する)。
    /// 文言は M25 の (9) で `msg.stdin.*` にする。それまでは `msg.open.failed` (パスの代わりに `-`)。
    fn stdin_load_failed(&mut self, doc_id: DocumentId, reason: &str) {
        self.open_failed(
            doc_id,
            None,
            MessageLevel::Error,
            keys::MSG_OPEN_FAILED,
            &FsPath::from("-"),
            reason,
        );
    }
}
