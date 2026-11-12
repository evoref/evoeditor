//! 開く (docs/03 §9.2): 読込の完了と失敗、開く前の確認 (巨大・バイナリ・不正バイト)、壊れた変換 (lossy)、ダングリングリンク、
//! 中断の痕跡 (STR-B1 で io/file.rs から分けた。起動時の退避と前回の版の走査は io/startup.rs)。

use eve_view::{DocumentId, MessageLevel, ViewId};

use crate::document::Document;
use crate::io::fs::{Expect, FsError, FsErrorDetail, FsErrorKind};
use crate::io::fspath::FsPath;
use crate::io::io_jobs::OpenOptions;
use crate::jobs::{ConfirmAsk, JobError, JobKind, JobResult, SaveStrategy};
use crate::ui_model::keys;

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

/// 保存の前の確認 (03 §8.6 E2'、§8.7.1) の文言の選び方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreConfirm {
    /// 保持できなかった箇所の数を言える: `Shift_JIS` / EUC-JP は `ui.confirm.save.lossy_roundtrip`、ISO-2022-JP は
    /// `ui.confirm.save.escape_normalized`。
    Count(u64),
    /// 数を言えない (「保持なし」の文書、検査を打ち切った文書): `ui.confirm.save.roundtrip_unchecked`。
    Unchecked,
}

/// 文書 `doc` を文字コード `out` で書く前に、確認 (03 §8.6 E2') が要るか。要らなければ `None`。
///
/// 元の文字コードで書くときだけ出す (別の文字コードへの変換は E1 / E3 で扱う)。未編集かどうかは見ない (名前を付けて
/// 保存・`file.set_encoding` では未編集でも書く。未編集の上書き保存は手順 0 で書かないので、ここへ来ない)。保持方式の
/// 状態ごとの条件:
/// - 保持が効いている (`Active`): 読込の時点で保持できなかった箇所 (`unpreserved`。ISO-2022-JP の、ASCII 状態で
///   始まらない・終わらない差のある行) が 1 以上なら、その数で出す。0 なら出さない (編集した範囲の区分 B の文字が
///   代表バイト列になるのは区分 C で、確認しない。行を結合して行の内容の全体でなくなった ISO-2022-JP の行の記録も、
///   編集した行と同じ区分 C)。
/// - 「保持なし」(`Unavailable`): 理由が `VerifyFailed` (読込・保存の確かめに失敗した。記録があった文書だけ) なら
///   無条件で、ほかは読込時の検査で区分 B / D の箇所を見つけた (`lossy_lines > 0`) か検査を打ち切った (`incomplete`)
///   文書だけ、数を言わずに出す。補助漢字・半角カナなど復号専用の文字の行 (`unmappable_lines`) だけの文書は出さない
///   (保存は必ず E3 で止まり、確認の文が事実と合わない。v0.3 と同じく E3 に任せる)。理由が `InvalidBytes` (不正バイト
///   を含む ISO-2022-JP) は、保存の連鎖の開始で拒否する (03 §8.3) ので出さない。
/// - 対象外 (`NotApplicable`。区分 A の文字コード・新規文書・違う文字コードで保存した後): 出さない。
pub(crate) fn preconfirm(doc: &Document, out: eve_encoding::Encoding) -> Option<PreConfirm> {
    if out != doc.format.encoding {
        return None;
    }
    preserve_kind(&doc.format)
}

/// 保存の前の確認と読込の完了時の通知に共通の条件 (`preconfirm` の説明)。
fn preserve_kind(format: &crate::document::TextFormat) -> Option<PreConfirm> {
    use eve_encoding::{NoPreserveReason, PreserveStatus};
    let rt = &format.roundtrip;
    match rt.preserve {
        PreserveStatus::Active { unpreserved } => {
            (unpreserved > 0).then_some(PreConfirm::Count(u64::from(unpreserved)))
        }
        PreserveStatus::Unavailable(NoPreserveReason::InvalidBytes) => None,
        PreserveStatus::Unavailable(NoPreserveReason::VerifyFailed) => Some(PreConfirm::Unchecked),
        PreserveStatus::Unavailable(_) => {
            (rt.lossy_lines > 0 || rt.incomplete).then_some(PreConfirm::Unchecked)
        }
        PreserveStatus::NotApplicable => None,
    }
}

/// 読込の完了時の、保持方式の通知 (03 §8.7.1 の「読込の完了時の通知」): (`MessageKey`, 数)。条件は保存の前の確認
/// (`preconfirm`) と同じ: 保持できなかった箇所がある文書は数を言う `info.open.preserve_partial`、「保持なし」は数を
/// 言わない `info.open.preserve_off`。保持が効いている文書 (保持できなかった箇所が無い) と、区分 B / D の箇所の無い
/// 文書では `None` (v0.3 までの `info.open.lossy_roundtrip` は出さない)。
pub(crate) fn preserve_note(format: &crate::document::TextFormat) -> Option<(&'static str, u64)> {
    match preserve_kind(format)? {
        PreConfirm::Count(n) => Some((keys::INFO_OPEN_PRESERVE_PARTIAL, n)),
        PreConfirm::Unchecked => Some((keys::INFO_OPEN_PRESERVE_OFF, 0)),
    }
}

impl EditorState {
    /// 読込の完了時の保持方式の通知を出す (`preserve_note`)。
    pub(crate) fn show_preserve_note(&mut self, note: Option<(&'static str, u64)>) {
        if let Some((key, count)) = note {
            self.show_message(MessageLevel::Info, key, &[("count", &count.to_string())]);
        }
    }

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
        self.io.link_create_ok = Some((doc, path.clone()));
        self.start_save_fmt(doc, path, expected, cont, fmt)
    }

    /// 保存の前の確認 (03 §8.6 E2') を出す。選択肢: 保存 / 別名で保存。条件と文言の選び方は `preconfirm`。
    /// 区分 B は別名が既定、区分 D (ISO-2022-JP のエスケープの書き方) は文字が変わらないので保存が既定 (04 §15.2)。
    /// 数を言えない文書 (「保持なし」・検査を打ち切った) は専用の文言にし、別名を既定にする (03 §8.7.1、Issue #495)。
    pub(crate) fn open_lossy(&mut self, ask: LossyAsk, kind: PreConfirm) {
        use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
        let Some(doc) = self.docs.documents.get(&ask.doc) else {
            return;
        };
        let example = doc
            .format
            .roundtrip
            .first_offsets
            .first()
            .map(u64::to_string)
            .unwrap_or_default();
        let (name, default, count) = match kind {
            PreConfirm::Unchecked => ("save.roundtrip_unchecked", 1, 0),
            PreConfirm::Count(n) if doc.format.encoding.is_stateful() => {
                ("save.escape_normalized", 0, n)
            }
            PreConfirm::Count(n) => ("save.lossy_roundtrip", 1, n),
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
            if let Some(d) = self.docs.documents.get(&doc) {
                self.io.lossy_ok = Some((doc, d.revision()));
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
                preserved,
                meta: _,
                stamp,
                changed_during_load,
                detected,
                notes,
                interrupted_save,
                ..
            } => {
                let note = preserve_note(&format);
                let encoding = format.encoding;
                let long_lines = format.long_lines;
                let mut d = Document::from_loaded(text, format, preserved);
                d.path = Some(path.clone());
                d.disk_stamp = Some(stamp);
                d.read_only = detected;
                self.put_document(doc_id, d);
                if long_lines > 0 {
                    self.message(
                        MessageLevel::Info,
                        keys::INFO_OPEN_LONG_LINE,
                        &path.display(),
                    );
                }
                if changed_during_load {
                    self.message(
                        MessageLevel::Warn,
                        keys::MSG_OPEN_CHANGED_DURING_LOAD,
                        &path.display(),
                    );
                }
                // 文字コードに関する通知 (最後に出したものが現在のメッセージ)
                self.show_preserve_note(note);
                if let Some(e) = notes.redetected {
                    self.show_message(
                        MessageLevel::Info,
                        keys::INFO_OPEN_REDETECTED,
                        &[("encoding", e.name())],
                    );
                }
                if let Some(guess) = notes.guess_unsupported {
                    self.show_message(
                        MessageLevel::Info,
                        keys::INFO_OPEN_GUESS_UNSUPPORTED,
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
                    keys::MSG_OPEN_PARENT_MISSING
                } else {
                    keys::INFO_OPEN_NEW_FILE
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
                    JobError::Cancelled => (MessageLevel::Info, keys::INFO_OPEN_CANCELLED),
                    JobError::Fs(e) => (MessageLevel::Error, open_error_key(&e)),
                    JobError::OutOfMemory { .. } => {
                        (MessageLevel::Error, keys::MSG_OPEN_OUT_OF_MEMORY)
                    }
                    _ => (MessageLevel::Error, keys::MSG_OPEN_FAILED),
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
                    // 不正バイトの比率の確認の理由は `InvalidBytes` (ISO-2022-JP なら `StatefulEncodingWithInvalidBytes`) だけ
                    // (03 §9.2 の表)。読込の結果 (`Complete.detected`) が付けるので、ここでは足さない (再判定 (03 §8.2.1) で
                    // 不正バイトが 0 になった本文に理由を残さないため)。`UserRequested` も付けない (`--readonly` だけの理由)
                    ConfirmAsk::ManyInvalidBytes { .. } => {}
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
                keys::INFO_OPEN_CANCELLED,
                &ask.path.display(),
            );
            return;
        }
        self.open_failed(
            ask.doc,
            ask.replace,
            MessageLevel::Info,
            keys::INFO_OPEN_CANCELLED,
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
        self.io.pending.insert(
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
            (SaveMethod::NewFile, _, _) => keys::MSG_OPEN_INTERRUPTED_NEW,
            (_, true, true) => keys::MSG_OPEN_INTERRUPTED_SAVE,
            (_, true, false) => keys::MSG_OPEN_INTERRUPTED_SAVE_INCOMPLETE,
            (_, false, _) => keys::MSG_OPEN_INTERRUPTED_SAVE_NO_BACKUP,
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
            let pristine = self.docs.documents.get(&from).is_some_and(|d| {
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
        if let Some((line, col)) = self.io.open_jumps.remove(&doc) {
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
        if self.active_document() == Some(doc) || self.focused_window().is_none() {
            // 表示中の文書の読込に失敗 (起動時の文書、または読込中に切り替えて表示していた足したバッファ): 失敗した文書は閉じ、
            // 開く前の文書 (残っていれば)・ほかの文書・無題の文書の順で表示する (再試行できる、03 §9.2 の「失敗」・§9.8)。
            // 表示を移さずに閉じると、Window が消えた View を指したままになる (fuzz_editor で見つけた)
            self.io.open_jumps.remove(&doc);
            let others = replace
                .filter(|r| *r != doc && self.docs.documents.contains_key(r))
                .or_else(|| self.docs.documents.keys().copied().find(|d| *d != doc));
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

/// 開く失敗の `MessageKey` (04 §15.2)。
pub(crate) fn open_error_key(e: &FsError) -> &'static str {
    match (&e.kind, &e.detail) {
        (FsErrorKind::PermissionDenied, _) => keys::MSG_OPEN_PERMISSION_DENIED,
        (FsErrorKind::Busy, _) => keys::MSG_OPEN_BUSY,
        (_, Some(FsErrorDetail::NotAFile)) => keys::MSG_OPEN_NOT_A_FILE,
        (_, Some(FsErrorDetail::ReservedName)) => keys::MSG_OPEN_RESERVED_NAME,
        _ => keys::MSG_OPEN_FAILED,
    }
}
