//! 起動時に複数のファイルを開く (docs/03 §9.8、07 §18.2、01 FR-101 / FR-102)。
//!
//! 最初のファイルは今までどおり表示する文書として開く。残りは読込中のバッファとして足し、読込が
//! 終わるのを待たない (一覧には載り、切替はできるが、編集はできない)。

use eve_view::{DocumentId, MessageLevel, ViewId};

use crate::document::{Document, LoadState};
use crate::editor::state::EditorState;
use crate::io::io_jobs::OpenOptions;
use crate::io::open::OpenKind;
use crate::io::pending::Pending;
use crate::jobs::{JobKind, JobResult};
use crate::view::motion::line_col_byte;
use eve_text::{Selection, TextRead};

/// 起動時に開くファイル 1 つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitialFile {
    /// 入力されたままのパス。
    pub path: String,
    /// 開いた直後に移る行 (1 始まり)。
    pub line: Option<usize>,
    /// 同じく桁 (1 始まり、書記素数)。行が無ければ使わない。
    pub col: Option<usize>,
}

impl EditorState {
    /// 先頭のファイルを表示する文書として開き、残りを読込中のバッファとして足す。
    pub(crate) fn open_initial_files(&mut self, files: &[InitialFile], opts: OpenOptions) {
        for (i, f) in files.iter().enumerate() {
            let doc = if i == 0 {
                self.open_path(&f.path, opts);
                self.active_document()
            } else {
                self.open_additional(&f.path, opts)
            };
            if let (Some(doc), Some(line)) = (doc, f.line) {
                self.io.open_jumps.insert(doc, (line, f.col.unwrap_or(1)));
            }
        }
    }

    /// 今の文書を置き換えず、読込中のバッファとして足す。同じパスが既にあれば足さない (03 §9.8)。
    /// 足した文書を返す。
    pub(crate) fn open_additional(
        &mut self,
        raw_path: &str,
        opts: OpenOptions,
    ) -> Option<DocumentId> {
        let opts = OpenOptions {
            default_eol: opts.default_eol.or(Some(self.config_eol())),
            huge_file_warn_bytes: opts
                .huge_file_warn_bytes
                .or(Some(self.config().huge_file_warn_bytes)),
            long_line_threshold: opts
                .long_line_threshold
                .or(Some(self.config_long_line_threshold())),
            ..opts
        };
        let Ok(path) = self.paths.normalize(raw_path) else {
            self.show_message(
                eve_view::MessageLevel::Error,
                "msg.open.failed",
                &[("path", raw_path), ("reason", "invalid path")],
            );
            return None;
        };
        if self
            .docs
            .documents
            .values()
            .any(|d| d.path.as_ref() == Some(&path))
        {
            return None;
        }
        let mut doc = Document::new_untitled();
        doc.path = Some(path.clone());
        doc.load_state = LoadState::Loading;
        let (doc_id, view) = self.add_document(doc);
        // 最初のファイルが開けなかったときは、これが最初に表示する文書になる
        if self.focused_window().is_none() {
            self.show_view(view, None);
        }
        self.spawn_open(doc_id, view, path, opts);
        Some(doc_id)
    }

    fn spawn_open(
        &mut self,
        doc_id: DocumentId,
        view: ViewId,
        path: crate::FsPath,
        opts: OpenOptions,
    ) {
        let job = self.jobs.spawn(
            Some(doc_id),
            eve_view::Revision::INITIAL,
            JobKind::Open {
                path: path.clone(),
                opts,
            },
        );
        self.io.pending.insert(
            job,
            Pending::Open {
                doc: doc_id,
                view,
                path,
                replace: None,
                opts,
                kind: OpenKind::Open,
            },
        );
    }

    /// 読み終わった文書の行・桁へカーソルを移す。範囲外は行末・最終行に丸める。
    pub(crate) fn jump_to_line_col(
        &mut self,
        view: ViewId,
        doc: DocumentId,
        line: usize,
        col: usize,
    ) {
        let Some(d) = self.docs.documents.get(&doc) else {
            return;
        };
        let t = d.text();
        let last = t.len_lines().saturating_sub(1);
        let p = line_col_byte(t, line.saturating_sub(1).min(last), col.max(1));
        self.set_selection(view, Selection::point(p));
        self.center_cursor(view);
    }
}

// ---- (v0.3 の M14、STR-B1 で io/ から移した) ----
impl EditorState {
    /// 起動時に退避ディレクトリを走査するジョブを投入する (`open_initial` が呼ぶ)。前回起動した版の記録 (FR-518) の
    /// ジョブを先に積む (`io` は直列なので、退避の案内のほうが後に出て、メッセージ行に残る)。
    pub(crate) fn start_recovery_scan(&mut self) {
        let job = self.jobs.spawn(
            None,
            eve_view::Revision::INITIAL,
            JobKind::LastVersion {
                current: env!("CARGO_PKG_VERSION").to_owned(),
            },
        );
        self.io.pending.insert(job, Pending::LastVersion);
        let job = self
            .jobs
            .spawn(None, eve_view::Revision::INITIAL, JobKind::RecoveryScan);
        self.io.pending.insert(job, Pending::RecoveryScan);
    }

    /// 退避が残っていれば場所を案内する (`msg.recovery.found` / `found_rescue`、02 §9.3。中身は読まない)。
    /// 案内するのは最も新しい退避 (名前の `<UTC timestamp>-<pid>[-<n>]` の順)。「退避先に保存」の退避だけなら文言を分ける。
    pub(crate) fn finish_recovery_scan(&mut self, r: JobResult) {
        let JobResult::RecoveryFound {
            mut dirs,
            swaps,
            leftovers,
            ..
        } = r
        else {
            return;
        };
        self.io.leftovers = leftovers;
        self.swap_scan_done(swaps);
        if dirs.is_empty() {
            return;
        }
        let only_rescue = dirs.iter().all(|d| d.rescue);
        // 異常終了の退避があれば、その中で最も新しいものを示す (無ければ「退避先に保存」の中で最も新しいもの)
        if !only_rescue {
            dirs.retain(|d| !d.rescue);
        }
        let key = |d: &crate::io::fs::RecoveryDirEntry| {
            d.path
                .as_path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        // 同じ時刻は `-<n>` の大きい方が新しい。名前の長さ → 辞書順で比べる (`-2` は `-` なしより後)
        dirs.sort_by(|a, b| {
            let (ka, kb) = (key(a), key(b));
            ka.get(..16)
                .cmp(&kb.get(..16))
                .then(ka.len().cmp(&kb.len()))
                .then(ka.cmp(&kb))
        });
        let count = dirs.len().to_string();
        let newest = dirs.last().map(|d| d.path.display()).unwrap_or_default();
        self.show_message(
            MessageLevel::Warn,
            if only_rescue {
                "msg.recovery.found_rescue"
            } else {
                "msg.recovery.found"
            },
            &[("dir", &newest), ("count", &count)],
        );
    }

    /// 更新後の初回起動なら、その版の要点を 1 行で出す (`info.app.updated`、07 §18.8)。それ以外は何もしない。
    pub(crate) fn finish_last_version(&mut self, r: JobResult) {
        if let JobResult::LastVersionChecked {
            outcome: crate::LastVersionOutcome::Updated,
            ..
        } = r
        {
            let summary = self
                .catalog()
                .render("info.app.release_summary", &[], &|_| None);
            self.show_message(
                MessageLevel::Info,
                "info.app.updated",
                &[
                    ("version", env!("CARGO_PKG_VERSION")),
                    ("summary", &summary),
                ],
            );
        }
    }
}
