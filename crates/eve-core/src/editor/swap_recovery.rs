//! スワップの復旧の提案と適用 (docs/03 §10.3、FR-118 / NFR-307)。
//!
//! 起動時の `RecoveryScan` が他のプロセスのスワップを列挙する (`swap_scan_done`)。書いたプロセスがもういない (`Stale`) 読めるスワップは
//! 候補になり、今の文書に該当する (同じパス、または無題で空の文書) ものがあれば、文書が読込済みで何も開いていないときに
//! 「適用 / 破棄」を提案する (`swap_offer`。tick で見直すので、後から `file.open` で開いた文書にも出る)。
//! 適用は `Transaction(Origin::Recovery)` で全置換し、dirty・Undo 可にする。保存するまでディスク上のファイルは変わらない。
//! 古いスワップは、新しいスワップが書けるか文書が保存されるまで消さない (その間に落ちても失わないため)。
//!
//! M6 は複数文書 (FR-103、M9a) の前なので「別バッファで開く」は出さない。元のファイルがスワップより後に変わっていれば、
//! 提案にその旨を添える。

use eve_text::{ByteRange, Edit};

use super::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
use super::state::EditorState;
use crate::document::{LoadState, Origin, Transaction};
use crate::{DocumentId, FsPath, JobError, JobKind, JobResult, SwapEntry, SwapOwner};

use super::file::Pending;
use eve_view::MessageLevel;

/// 同じパスか。Windows は大文字小文字を区別しない (05 §3.1)。
fn same_path(a: &FsPath, b: &FsPath) -> bool {
    let (a, b) = (a.display(), b.display());
    if cfg!(windows) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

impl EditorState {
    /// 起動時の走査の結果を受ける。候補を覚え、読めないもの・他のインスタンスのものを知らせる。
    pub(super) fn swap_scan_done(&mut self, swaps: Vec<SwapEntry>) {
        if swaps.is_empty() {
            return;
        }
        // 動いている別のプロセスのスワップ。同じファイルを開いたら (tick で) 警告する
        self.swap.alive.extend(
            swaps
                .iter()
                .filter(|e| e.owner == SwapOwner::Alive)
                .cloned(),
        );
        // 読めない (新しい版・壊れている・書きかけ): 場所だけを示す。移動も削除もしない
        let unreadable: Vec<&SwapEntry> = swaps
            .iter()
            .filter(|e| e.owner == SwapOwner::Stale && !e.readable)
            .collect();
        if let Some(first) = unreadable.first() {
            self.show_message(
                MessageLevel::Warn,
                "msg.swap.unreadable",
                &[
                    ("dir", &first.dir.display()),
                    ("count", &unreadable.len().to_string()),
                ],
            );
        }
        let candidates: Vec<SwapEntry> = swaps
            .into_iter()
            .filter(|e| e.owner == SwapOwner::Stale && e.readable)
            .collect();
        if let Some(first) = candidates.first() {
            self.show_message(
                MessageLevel::Warn,
                "msg.swap.found",
                &[
                    ("count", &candidates.len().to_string()),
                    ("dir", &first.dir.display()),
                ],
            );
        }
        self.swap.candidates.extend(candidates);
    }

    /// 今の文書に該当する候補があれば、復旧を提案する。何かが開いていれば、閉じるまで待つ。
    pub(super) fn swap_offer(&mut self) {
        self.swap_warn_alive();
        if self.swap.candidates.is_empty() || self.overlay_kind() != "" {
            return;
        }
        let Some(id) = self.active_document() else {
            return;
        };
        let Some(d) = self.documents.get(&id) else {
            return;
        };
        // 編集中の内容を上書きしないよう、読込済みで何も編集していない文書にだけ出す
        if d.load_state != LoadState::Ready || d.is_dirty() || !d.read_only.is_empty() {
            return;
        }
        let blank_untitled = d.path.is_none() && d.len_bytes() == 0;
        let matching = |e: &SwapEntry| match (&e.path, &d.path) {
            (Some(a), Some(b)) => same_path(a, b),
            (None, None) => blank_untitled,
            _ => false,
        };
        // 該当する候補のうち、最後に書かれたもの
        let Some(i) = self
            .swap
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, e)| matching(e))
            .max_by_key(|(_, e)| e.modified.map(|t| (t.secs, t.nanos)))
            .map(|(i, _)| i)
        else {
            return;
        };
        let entry = self.swap.candidates.remove(i);
        let revision = d.revision();
        // 元のファイルがスワップを書いた後に変わっている
        let changed = match (d.disk_stamp.as_ref(), entry.modified) {
            (Some(st), Some(m)) => (st.mtime.secs, st.mtime.nanos) > (m.secs, m.nanos),
            _ => false,
        };
        let name = entry
            .path
            .as_ref()
            .map(FsPath::display)
            .or_else(|| entry.untitled_name.clone())
            .unwrap_or_else(|| self.untitled_label());
        let mut details = Vec::new();
        let args = [("path", name.as_str())];
        if changed {
            details.push(
                self.catalog()
                    .render("ui.confirm.swap_recovery.changed", &args, &|_| None),
            );
        }
        if entry.sensitive {
            details.push(self.catalog().render(
                "ui.confirm.swap_recovery.sensitive",
                &args,
                &|_| None,
            ));
        }
        self.open_confirm(ConfirmState {
            name: "swap_recovery",
            args: vec![
                ("path".to_owned(), name),
                ("len".to_owned(), entry.len.to_string()),
            ],
            choices: vec![
                ChoiceDef {
                    id: "apply",
                    accel: Some('a'),
                    destructive: false,
                },
                ChoiceDef {
                    id: "discard",
                    accel: Some('d'),
                    destructive: true,
                },
            ],
            details,
            default: 0,
            focused: 0,
            action: ConfirmAction::SwapRecovery {
                entry: Box::new(entry),
                doc: id,
                revision,
            },
        });
    }

    /// 開いた文書と同じファイルのスワップを、動いている別のプロセスが書いていれば警告する (1 件につき 1 回)。
    fn swap_warn_alive(&mut self) {
        if self.swap.alive.is_empty() {
            return;
        }
        let Some(d) = self
            .active_document()
            .and_then(|id| self.documents.get(&id))
            .filter(|d| d.load_state == LoadState::Ready)
        else {
            return;
        };
        let Some(path) = d.path.clone() else {
            return;
        };
        let (hit, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.swap.alive)
            .into_iter()
            .partition(|e| e.path.as_ref().is_some_and(|p| same_path(p, &path)));
        self.swap.alive = rest;
        for e in hit {
            self.show_message(
                MessageLevel::Warn,
                "msg.swap.other_instance",
                &[("path", &path.display()), ("pid", &e.pid.to_string())],
            );
        }
    }

    fn untitled_label(&self) -> String {
        self.catalog()
            .render("status.untitled", &[("n", "1")], &|_| None)
    }

    /// 提案への答え。`apply` は本文を読むジョブを投入し、`discard` はスワップを消す (この選択が破棄の確認)。
    pub(super) fn choose_swap_recovery(
        &mut self,
        entry: SwapEntry,
        doc: DocumentId,
        revision: eve_view::Revision,
        id: &str,
    ) -> crate::CommandResult {
        if id == "apply" {
            let job = self.jobs.spawn(
                Some(doc),
                revision,
                JobKind::SwapRead {
                    entry: entry.clone(),
                },
            );
            self.pending.insert(
                job,
                Pending::SwapRead {
                    doc,
                    revision,
                    entry: Box::new(entry),
                },
            );
            crate::CommandResult::Pending(job)
        } else {
            let job = self.jobs.spawn(
                None,
                eve_view::Revision::INITIAL,
                JobKind::SwapRemove {
                    entry: entry.clone(),
                },
            );
            self.pending.insert(
                job,
                Pending::SwapRemove {
                    entry: Box::new(entry),
                    notify: true,
                },
            );
            crate::CommandResult::Pending(job)
        }
    }

    /// スワップの本文を読んだ。文書が提案の時点のままなら、全置換の Transaction で適用する。
    pub(super) fn finish_swap_read(
        &mut self,
        doc: DocumentId,
        revision: eve_view::Revision,
        entry: SwapEntry,
        r: JobResult,
    ) {
        let body = match r {
            JobResult::SwapRead { body, .. } => body,
            _ => {
                // 読めなかった: スワップは残す
                self.show_message(
                    MessageLevel::Error,
                    "msg.swap.read_failed",
                    &[("dir", &entry.dir.display())],
                );
                return;
            }
        };
        let Some(d) = self.documents.get_mut(&doc) else {
            return;
        };
        if d.revision() != revision || !d.read_only.is_empty() {
            // 提案の後で文書が変わった: 適用しない。スワップは残し、もう一度提案できるようにする
            self.show_message(MessageLevel::Warn, "msg.swap.changed_since_offer", &[]);
            self.swap.candidates.push(entry);
            return;
        }
        // 無題の文書は、スワップの形式 (文字コード・BOM・改行) を引き継ぐ。パスのある文書は開いた形式のまま
        if d.path.is_none() {
            d.format.encoding = body.encoding;
            d.format.has_bom = body.has_bom;
            d.format.default_eol = body.default_eol;
        }
        let name = entry
            .path
            .as_ref()
            .map(FsPath::display)
            .or_else(|| entry.untitled_name.clone())
            .unwrap_or_else(|| self.untitled_label());
        let Some(d) = self.documents.get(&doc) else {
            return;
        };
        let all = ByteRange::all(d.len_bytes());
        let Ok(cs) = eve_text::ChangeSet::from_sorted_edits(
            d.text(),
            [Edit {
                range: all,
                insert: body.text.into(),
            }],
        ) else {
            self.show_message(
                MessageLevel::Error,
                "msg.swap.read_failed",
                &[("dir", &entry.dir.display())],
            );
            return;
        };
        let tx = Transaction::new(cs, Origin::Recovery, self.now);
        match self.apply_transaction(doc, tx) {
            Ok(_) => {
                self.swap.recovered.entry(doc).or_default().push(entry);
                self.show_message(
                    MessageLevel::Info,
                    "info.swap.recovered",
                    &[("path", &name)],
                );
            }
            Err(_) => {
                self.show_message(
                    MessageLevel::Error,
                    "msg.swap.read_failed",
                    &[("dir", &entry.dir.display())],
                );
                self.swap.candidates.push(entry);
            }
        }
    }

    /// 他のプロセスのスワップを消したジョブの結果。
    pub(super) fn finish_swap_remove(&mut self, entry: &SwapEntry, notify: bool, r: JobResult) {
        match r {
            JobResult::SwapRemoved { .. } => {
                if notify {
                    self.show_message(MessageLevel::Info, "info.swap.discarded", &[]);
                }
            }
            JobResult::Failed {
                err: JobError::Fs(_),
                ..
            } => self.show_message(
                MessageLevel::Warn,
                "msg.swap.remove_failed",
                &[("dir", &entry.dir.display())],
            ),
            _ => {}
        }
    }

    /// 適用した古いスワップを、新しいスワップが書けた・文書が保存された (dirty でなくなった)・文書が閉じられたときに消す。
    pub(super) fn swap_release_recovered(&mut self) {
        if self.swap.recovered.is_empty() {
            return;
        }
        let ready: Vec<DocumentId> = self
            .swap
            .recovered
            .keys()
            .copied()
            .filter(|id| {
                self.documents.get(id).is_none_or(|d| !d.is_dirty()) || self.swap_written(*id)
            })
            .collect();
        for id in ready {
            for entry in self.swap.recovered.remove(&id).unwrap_or_default() {
                let job = self.jobs.spawn(
                    None,
                    eve_view::Revision::INITIAL,
                    JobKind::SwapRemove {
                        entry: entry.clone(),
                    },
                );
                self.pending.insert(
                    job,
                    Pending::SwapRemove {
                        entry: Box::new(entry),
                        notify: false,
                    },
                );
            }
        }
    }

    /// 正常終了のとき、Host が消す古いスワップ (適用したが、まだ消していないもの)。
    pub(super) fn swap_recovered_leftovers(&self) -> Vec<SwapEntry> {
        self.swap.recovered.values().flatten().cloned().collect()
    }
}
