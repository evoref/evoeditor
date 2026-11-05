//! 開く (docs/03 §9.2): 読込の完了と失敗、開く前の確認 (巨大・バイナリ・不正バイト)、壊れた変換 (lossy)、ダングリングリンク、
//! 中断の痕跡 (STR-B1 で io/file.rs から分けた。起動時の退避と前回の版の走査は io/startup.rs)。

use eve_view::{DocumentId, MessageLevel, ViewId};

use crate::document::Document;
use crate::io::fs::{Expect, FsError, FsErrorDetail, FsErrorKind};
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;
use crate::jobs::{ConfirmAsk, JobError, JobKind, JobResult, SaveStrategy};

use super::file::*;
use super::pending::*;
use super::save::*;
use crate::command::CommandResult;
use crate::editor::state::EditorState;
use crate::ui_model::overlay::{Continuation, PromptKind};

/// 読込ジョブの用途。`Reopen` / `Reload` は同じ文書の中身を差し替える (03 §8.8 / §9.5)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenKind {
    Open,
    /// 文字コードを指定して開き直す: 履歴を消して差し替える (Undo できない)。
    Reopen,
    /// 再読込: `Origin::Reload` の Transaction で適用する (Undo できる)。
    Reload,
}

/// 開くときの確認 (`ui.confirm.open.*`、03 §9.2) に答えるための材料。読込中の文書 (Loading) は確認の間も残す。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenAsk {
    pub doc: DocumentId,
    pub view: ViewId,
    pub path: FsPath,
    pub replace: Option<DocumentId>,
    pub opts: OpenOptions,
    pub ask: ConfirmAsk,
    /// 用途 (開き直し・再読込は同じ文書の中身を差し替える)。
    pub kind: OpenKind,
}

/// 区分 B / D の確認 (別のバイト表現で保存される箇所がある、03 §8.6 E2') と、「読み取り専用を解除して保存」の確認 (FR-116)。
/// 承認したら同じ保存を投入し直す。
#[derive(Clone, Debug)]
pub(crate) struct LossyAsk {
    pub doc: DocumentId,
    pub path: FsPath,
    pub expected: Expect,
    pub cont: Option<Continuation>,
    pub strategy: SaveStrategy,
    pub fmt: Option<NewFormat>,
}

impl EditorState {
    /// 保存先がダングリングのシンボリックリンク: リンク先を示して、リンク先に作成するかを尋ねる (FR-106、05 §4.1)。
    pub(crate) fn open_dangling_link(&mut self, ask: SaveFailedAsk, target: String) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        self.open_confirm(ConfirmState {
            name: "save.dangling_link",
            args: vec![
                ("path".to_owned(), ask.path.display()),
                ("target".to_owned(), target),
            ],
            choices: vec![ChoiceDef {
                id: "create",
                accel: Some('c'),
                destructive: false,
            }],
            details: Vec::new(),
            default: 0,
            focused: 0,
            action: ConfirmAction::DanglingLink(ask),
        });
    }

    /// 「リンク先に作成」: 同じ保存を、リンク先を作る指定で投入し直す (キャンセルは呼び側が処理済み)。
    pub(crate) fn create_link_target(&mut self, ask: SaveFailedAsk) -> CommandResult {
        let SaveFailedAsk {
            doc,
            path,
            expected,
            cont,
            fmt,
            ..
        } = ask;
        self.link_create_ok = Some((doc, path.clone()));
        self.start_save_fmt(doc, path, expected, cont, fmt)
    }

    /// 区分 B (ISO-2022-JP は区分 D のエスケープの書き方) の確認を出す。選択肢: 保存 / 別名で保存。
    /// 区分 B は別名が既定、区分 D は文字が変わらないので保存が既定 (04 §15.2)。
    pub(crate) fn open_lossy(&mut self, ask: LossyAsk) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let Some(doc) = self.documents.get(&ask.doc) else {
            return;
        };
        let count = doc.format.roundtrip.lossy_lines;
        let example = doc
            .format
            .roundtrip
            .first_offsets
            .first()
            .map(u64::to_string)
            .unwrap_or_default();
        let (name, default) = if doc.format.encoding.is_stateful() {
            ("save.escape_normalized", 0)
        } else {
            ("save.lossy_roundtrip", 1)
        };
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        self.open_confirm(ConfirmState {
            name,
            args: vec![
                ("count".to_owned(), count.to_string()),
                ("example".to_owned(), example),
            ],
            choices: vec![choice("save", 's'), choice("alt", 'a')],
            details: Vec::new(),
            default,
            focused: default,
            action: ConfirmAction::Lossy(ask),
        });
    }

    /// 区分 B / D の確認の選択 (キャンセルは呼び側が処理済み)。保存は、この Revision では確認を繰り返さずに投入し直す。
    pub(crate) fn choose_lossy(&mut self, ask: LossyAsk, choice: usize) -> CommandResult {
        let LossyAsk {
            doc,
            path,
            expected,
            cont,
            strategy,
            fmt,
        } = ask;
        if choice == 0 {
            if let Some(d) = self.documents.get(&doc) {
                self.lossy_ok = Some((doc, d.revision()));
            }
            return self.start_save_with(doc, path, expected, cont, strategy, fmt);
        }
        // 別名で保存: 名前の入力へ (保存の後に続ける操作は引き継ぐ)
        self.open_prompt(PromptKind::SaveAs { then: cont }, String::new());
        CommandResult::Done
    }

    #[allow(clippy::too_many_arguments)] // 読込ジョブの文脈 (Pending::Open) をそのまま受ける
    pub(crate) fn finish_open(
        &mut self,
        doc_id: DocumentId,
        view: ViewId,
        path: FsPath,
        replace: Option<DocumentId>,
        opts: OpenOptions,
        kind: OpenKind,
        r: JobResult,
    ) {
        if kind != OpenKind::Open {
            self.finish_reopen(doc_id, view, path, opts, kind, r);
            return;
        }
        match r {
            JobResult::Complete {
                text,
                format,
                meta: _,
                stamp,
                changed_during_load,
                detected,
                notes,
                interrupted_save,
                ..
            } => {
                let lossy = format.roundtrip.lossy_lines;
                let encoding = format.encoding;
                let long_lines = format.long_lines;
                let mut d = Document::from_text(text, format);
                d.path = Some(path.clone());
                d.disk_stamp = Some(stamp);
                d.read_only = detected;
                self.put_document(doc_id, d);
                if long_lines > 0 {
                    self.message(MessageLevel::Info, "info.open.long_line", &path.display());
                }
                if changed_during_load {
                    self.message(
                        MessageLevel::Warn,
                        "msg.open.changed_during_load",
                        &path.display(),
                    );
                }
                // 文字コードに関する通知 (最後に出したものが現在のメッセージ)
                if lossy > 0 {
                    self.show_message(
                        MessageLevel::Info,
                        "info.open.lossy_roundtrip",
                        &[("count", &lossy.to_string())],
                    );
                }
                if let Some(e) = notes.redetected {
                    self.show_message(
                        MessageLevel::Info,
                        "info.open.redetected",
                        &[("encoding", e.name())],
                    );
                }
                if let Some(guess) = notes.guess_unsupported {
                    self.show_message(
                        MessageLevel::Info,
                        "info.open.guess_unsupported",
                        &[
                            ("path", &path.display()),
                            ("guess", guess),
                            ("encoding", encoding.name()),
                        ],
                    );
                }
                // 中断された保存の痕跡は最も重要なので最後に出す (現在のメッセージにする)
                if let Some(is) = &interrupted_save {
                    self.show_interrupted(&path, is);
                }
                self.complete_open(doc_id, view, replace);
            }
            JobResult::NewFile {
                parent_missing,
                interrupted_save,
                ..
            } => {
                // 存在しないパス: 空の新規文書 (dirty ではない)。保存すると作成する
                let mut d = self.blank_document();
                d.path = Some(path.clone());
                self.put_document(doc_id, d);
                let key = if parent_missing.is_some() {
                    "msg.open.parent_missing"
                } else {
                    "info.open.new_file"
                };
                let dir = parent_missing
                    .as_ref()
                    .map(FsPath::display)
                    .unwrap_or_default();
                self.show_message(
                    if parent_missing.is_some() {
                        MessageLevel::Warn
                    } else {
                        MessageLevel::Info
                    },
                    key,
                    &[("path", &path.display()), ("dir", &dir)],
                );
                if let Some(is) = &interrupted_save {
                    self.show_interrupted(&path, is);
                }
                self.complete_open(doc_id, view, replace);
            }
            // 確認が要る: 読込中の文書は残したまま確認を出し、回答で先頭から読み直す (03 §9.2)
            JobResult::NeedsConfirm { ask, .. } => {
                self.open_open_confirm(OpenAsk {
                    doc: doc_id,
                    view,
                    path,
                    replace,
                    opts,
                    ask,
                    kind: OpenKind::Open,
                });
            }
            JobResult::Failed { err, .. } => {
                let reason = reason_label(&err);
                let (level, key) = match err {
                    JobError::Cancelled => (MessageLevel::Info, "info.open.cancelled"),
                    JobError::Fs(e) => (MessageLevel::Error, open_error_key(&e)),
                    JobError::OutOfMemory { .. } => (MessageLevel::Error, "msg.open.out_of_memory"),
                    _ => (MessageLevel::Error, "msg.open.failed"),
                };
                self.open_failed(doc_id, replace, level, key, &path, &reason);
            }
            _ => {}
        }
    }

    /// 開くときの確認 (`ui.confirm.open.*`) を出す。
    pub(crate) fn open_open_confirm(&mut self, ask: OpenAsk) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let choice = |id: &'static str, accel: char| ChoiceDef {
            id,
            accel: Some(accel),
            destructive: false,
        };
        let (name, choices, args): (&'static str, Vec<ChoiceDef>, Vec<(String, String)>) =
            match ask.ask {
                ConfirmAsk::Binary => (
                    "open.binary",
                    vec![choice("readonly", 'r'), choice("encoding", 'e')],
                    vec![],
                ),
                ConfirmAsk::UnsupportedBom { .. } => (
                    "open.utf32",
                    vec![choice("readonly", 'r'), choice("encoding", 'e')],
                    vec![],
                ),
                ConfirmAsk::HugeFile { len } => (
                    "open.huge",
                    vec![choice("readonly", 'r')],
                    vec![("size".to_owned(), human_size(len))],
                ),
                ConfirmAsk::ManyInvalidBytes { encoding, permille } => (
                    "open.invalid_bytes",
                    vec![
                        choice("readonly", 'r'),
                        choice("encoding", 'e'),
                        choice("abort", 'c'),
                    ],
                    vec![
                        ("encoding".to_owned(), encoding.name().to_owned()),
                        (
                            "ratio".to_owned(),
                            format!("{}.{}%", permille / 10, permille % 10),
                        ),
                        (
                            "factor".to_owned(),
                            format!("{:.1}", encoding.memory_factor()),
                        ),
                    ],
                ),
            };
        let mut args = args;
        args.insert(0, ("path".to_owned(), ask.path.display()));
        self.open_confirm(ConfirmState {
            name,
            args,
            choices,
            details: Vec::new(),
            default: 0,
            focused: 0,
            action: ConfirmAction::Open(ask),
        });
    }

    /// 開くときの確認の選択 (キャンセルは呼び側が `open_aborted` を呼ぶ)。`id` は選択肢の ID。
    pub(crate) fn choose_open(&mut self, ask: OpenAsk, id: &str) -> CommandResult {
        match id {
            "readonly" => {
                let mut opts = ask.opts;
                opts.read_only_requested = true;
                match ask.ask {
                    ConfirmAsk::HugeFile { .. } => {
                        opts.confirmed_large = true;
                        opts.pending
                            .insert(crate::document::ReadOnlyReason::LargeFileViewer);
                    }
                    ConfirmAsk::Binary | ConfirmAsk::UnsupportedBom { .. } => {
                        opts.pending.insert(crate::document::ReadOnlyReason::Binary);
                    }
                    _ => {}
                }
                self.respawn_open(ask, opts)
            }
            "encoding" => {
                self.open_prompt(PromptKind::Encoding { ask }, String::new());
                CommandResult::Done
            }
            _ => {
                self.open_aborted(&ask);
                CommandResult::Done
            }
        }
    }

    /// 開くのを中止した (確認の `Esc` / 中止、文字コードの入力の `Esc`)。読込中の文書を捨てる。
    pub(crate) fn open_aborted(&mut self, ask: &OpenAsk) {
        if ask.kind != OpenKind::Open {
            // 開き直し・再読込の中止: 旧本文・書式・履歴をそのまま保つ (03 §8.8 / §9.5)
            self.reopen_restore(ask.doc);
            self.message(
                MessageLevel::Info,
                "info.open.cancelled",
                &ask.path.display(),
            );
            return;
        }
        self.open_failed(
            ask.doc,
            ask.replace,
            MessageLevel::Info,
            "info.open.cancelled",
            &ask.path,
            "",
        );
    }

    /// 確認の回答の後、同じ読込中の文書へ `opts` で先頭から読み直すジョブを投入する。
    pub(crate) fn respawn_open(&mut self, ask: OpenAsk, opts: OpenOptions) -> CommandResult {
        let job = self.jobs.spawn(
            Some(ask.doc),
            eve_view::Revision::INITIAL,
            JobKind::Open {
                path: ask.path.clone(),
                opts,
            },
        );
        self.pending.insert(
            job,
            Pending::Open {
                doc: ask.doc,
                view: ask.view,
                path: ask.path,
                replace: ask.replace,
                opts,
                kind: ask.kind,
            },
        );
        CommandResult::Pending(job)
    }

    // ---------- 文字コードを指定して開き直す (03 §8.8) ----------

    /// 中断された保存の痕跡の案内 (04 §15.2 の `msg.open.interrupted_*`、02 §4.5 の写像)。
    pub(crate) fn show_interrupted(&mut self, path: &FsPath, is: &crate::io::fs::InterruptedSave) {
        use crate::io::fs::SaveMethod;
        let key = match (is.method, is.backup.is_some(), is.complete) {
            (SaveMethod::NewFile, _, _) => "msg.open.interrupted_new",
            (_, true, true) => "msg.open.interrupted_save",
            (_, true, false) => "msg.open.interrupted_save_incomplete",
            (_, false, _) => "msg.open.interrupted_save_no_backup",
        };
        let backup = is.backup.as_ref().map(FsPath::display).unwrap_or_default();
        let tmp = is.tmp.as_ref().map(FsPath::display).unwrap_or_default();
        self.show_message(
            MessageLevel::Warn,
            key,
            &[
                ("path", &path.display()),
                ("backup", &backup),
                ("tmp", &tmp),
            ],
        );
    }

    pub(crate) fn complete_open(
        &mut self,
        doc: DocumentId,
        view: ViewId,
        replace: Option<DocumentId>,
    ) {
        // 読込の間に別の文書へ切り替えていなければ、新しいバッファへ切り替える (03 §9.8)
        if let Some(from) = replace
            && self.active_document() == Some(from)
        {
            // 何も入力していない無題のバッファは、開いたファイルで置き換える (何も失わない。起動直後の Ctrl+O で空のバッファが残らない)
            let pristine = self.documents.get(&from).is_some_and(|d| {
                d.path.is_none()
                    && !d.is_dirty()
                    && d.len_bytes() == 0
                    && d.load_state == crate::document::LoadState::Ready
            });
            if pristine {
                self.show_view(view, Some(from));
            } else {
                self.activate_document(doc);
            }
        }
        // 起動時の `+行:桁` / `path:行:桁`: 読み終わった文書の位置へ移る (範囲外は丸める)
        if let Some((line, col)) = self.open_jumps.remove(&doc) {
            self.jump_to_line_col(view, doc, line, col);
        }
        self.mark_snapshot_update(doc);
    }

    pub(crate) fn open_failed(
        &mut self,
        doc: DocumentId,
        replace: Option<DocumentId>,
        level: MessageLevel,
        key: &str,
        path: &FsPath,
        reason: &str,
    ) {
        self.show_message(level, key, &[("path", &path.display()), ("reason", reason)]);
        if self.active_document() == Some(doc) || self.first_window().is_none() {
            // 表示中の文書の読込に失敗 (起動時の文書、または読込中に切り替えて表示していた足したバッファ): 失敗した文書は閉じ、
            // 開く前の文書 (残っていれば)・ほかの文書・無題の文書の順で表示する (再試行できる、03 §9.2 の「失敗」・§9.8)。
            // 表示を移さずに閉じると、Window が消えた View を指したままになる (fuzz_editor で見つけた)
            self.open_jumps.remove(&doc);
            let others = replace
                .filter(|r| *r != doc && self.documents.contains_key(r))
                .or_else(|| self.documents.keys().copied().find(|d| *d != doc));
            match others {
                Some(next) => {
                    self.activate_document(next);
                    self.close_document(doc);
                }
                None => {
                    let blank = self.blank_document();
                    let (_, untitled) = self.add_document(blank);
                    self.show_view(untitled, Some(doc));
                }
            }
        } else {
            // 表示していない文書 (足したバッファ・起動時に追加で開いた文書) の読込の失敗: 表示中の文書はそのままに、閉じるだけ
            self.close_document(doc);
        }
    }
}

/// バイト数を「512.0 MiB」のような短い表記にする。
pub(crate) fn human_size(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// 開く失敗の MessageKey (04 §15.2)。
pub(crate) fn open_error_key(e: &FsError) -> &'static str {
    match (&e.kind, &e.detail) {
        (FsErrorKind::PermissionDenied, _) => "msg.open.permission_denied",
        (FsErrorKind::Busy, _) => "msg.open.busy",
        (_, Some(FsErrorDetail::NotAFile)) => "msg.open.not_a_file",
        (_, Some(FsErrorDetail::ReservedName)) => "msg.open.reserved_name",
        _ => "msg.open.failed",
    }
}
