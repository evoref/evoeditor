//! 起動時に複数のファイルを開く (docs/03 §9.8、07 §18.2、01 FR-101 / FR-102)。
//!
//! 最初のファイルは今までどおり表示する文書として開く。残りは読込中のバッファとして足し、読込が
//! 終わるのを待たない (一覧には載り、切替はできるが、編集はできない)。

use eve_view::{DocumentId, ViewId};

use super::file::{OpenKind, Pending};
use super::motion::line_col_byte;
use super::state::EditorState;
use crate::document::{Document, LoadState};
use crate::io_jobs::OpenOptions;
use crate::jobs::JobKind;
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
    pub(super) fn open_initial_files(&mut self, files: &[InitialFile], opts: OpenOptions) {
        for (i, f) in files.iter().enumerate() {
            let doc = if i == 0 {
                self.open_path(&f.path, opts);
                self.active_document()
            } else {
                self.open_additional(&f.path, opts)
            };
            if let (Some(doc), Some(line)) = (doc, f.line) {
                self.open_jumps.insert(doc, (line, f.col.unwrap_or(1)));
            }
        }
    }

    /// 今の文書を置き換えず、読込中のバッファとして足す。同じパスが既にあれば足さない (03 §9.8)。
    /// 足した文書を返す。
    fn open_additional(&mut self, raw_path: &str, opts: OpenOptions) -> Option<DocumentId> {
        let opts = OpenOptions {
            default_eol: opts.default_eol.or(Some(self.config_eol())),
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
        if self.first_window().is_none() {
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
        self.pending.insert(
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
    pub(super) fn jump_to_line_col(
        &mut self,
        view: ViewId,
        doc: DocumentId,
        line: usize,
        col: usize,
    ) {
        let Some(d) = self.documents.get(&doc) else {
            return;
        };
        let t = d.text();
        let last = t.len_lines().saturating_sub(1);
        let p = line_col_byte(t, line.saturating_sub(1).min(last), col.max(1));
        self.set_selection(view, Selection::point(p));
        self.center_cursor(view);
    }
}
