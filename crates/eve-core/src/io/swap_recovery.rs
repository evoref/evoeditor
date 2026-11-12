//! スワップの復旧の提案と適用 (docs/03 §10.3、FR-118 / NFR-307)。
//!
//! 起動時の `RecoveryScan` が他のプロセスのスワップを列挙する (`swap_scan_done`)。書いたプロセスがもういない (`Stale`) 読めるスワップは
//! 候補になり、今の文書に該当する (同じパス、または無題で空の文書) ものがあれば、文書が読込済みで何も開いていないときに
//! 「適用 / 破棄」を提案する (`swap_offer`。tick で見直すので、後から `file.open` で開いた文書にも出る)。
//! 適用は `Transaction(Origin::Recovery)` で全置換し、dirty・Undo 可にする。保存するまでディスク上のファイルは変わらない。
//! 古いスワップは、新しいスワップが書けるか文書が保存されるまで消さない (その間に落ちても失わないため)。
//!
//! 「別バッファで開く」は出さない。元のファイルがスワップより後に変わっていれば、
//! 提案にその旨を添える。

use eve_text::{ByteRange, Edit};

use crate::document::{LoadState, Origin, Transaction};
use crate::editor::state::EditorState;
use crate::ui_model::keys;
use crate::ui_model::overlay::{ChoiceDef, ConfirmAction, ConfirmState};
use crate::{DocumentId, FsPath, JobError, JobKind, JobResult, SwapEntry, SwapOwner};
use eve_encoding::PuaEscapeMode;

use crate::io::pending::Pending;
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

/// 復旧 (本文の全体を置き換える) を適用した後の `escape_encoding` と `pua_escape` (03 §8.3.1 の「エスケープ文字の由来の
/// 文字コード」の表の「復旧の適用」の行)。`format` は適用する文書の書式 (無題の文書はスワップの文字コード・BOM・改行を
/// 写した後)。本文がスワップのものに替わるので、エスケープ文字の意味はディスクではなくスワップ (manifest) に従う。
///
/// - manifest の `pua_escaped` が偽: スワップの本文のエスケープ文字はバイトではないので `None`。`pua_escape` は変えない。
/// - 真: 衝突の文書 (`DisabledCollision`) は `Enabled` にする (スワップの私用文字は不正バイトの印。衝突の扱い (`AsText`)
///   で私用文字の UTF-8 として書かない)。状態付き (`DisabledStateful`) はそのまま。そのうえで、manifest の `encoding` が
///   文書の文字コードと同じで `Enabled`、状態付きの文字コードでなければ `Some(encoding)` (ディスクのファイルが不正
///   バイトを含まなくなっていても。無題の文書はスワップの文字コードを引き継ぐので、ここに当たる)、ほかは `None`。
///
/// 本文が同じ復旧 (全置換が恒等) は `Document::apply` が書式を変えない (エスケープ文字の意味も今の値のまま)。
pub(crate) fn recovered_escape(
    format: &crate::document::TextFormat,
    pua_escaped: bool,
    manifest_encoding: crate::Encoding,
) -> (Option<crate::Encoding>, PuaEscapeMode) {
    if !pua_escaped {
        return (None, format.pua_escape);
    }
    let mode = match format.pua_escape {
        PuaEscapeMode::DisabledCollision => PuaEscapeMode::Enabled,
        m => m,
    };
    let esc = (manifest_encoding == format.encoding
        && mode == PuaEscapeMode::Enabled
        && !manifest_encoding.is_stateful())
    .then_some(manifest_encoding);
    (esc, mode)
}

impl EditorState {
    /// 起動時の走査の結果を受ける。候補を覚え、読めないもの・他のインスタンスのものを知らせる。
    pub(crate) fn swap_scan_done(&mut self, swaps: Vec<SwapEntry>) {
        if swaps.is_empty() {
            return;
        }
        // 動いている別のプロセスのスワップ。同じファイルを開いたら (tick で) 警告する
        self.io.swap.alive.extend(
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
                keys::MSG_SWAP_UNREADABLE,
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
                keys::MSG_SWAP_FOUND,
                &[
                    ("count", &candidates.len().to_string()),
                    ("dir", &first.dir.display()),
                ],
            );
        }
        self.io.swap.candidates.extend(candidates);
    }

    /// 今の文書に該当する候補があれば、復旧を提案する。何かが開いていれば、閉じるまで待つ。
    pub(crate) fn swap_offer(&mut self) {
        self.swap_warn_alive();
        if self.io.swap.candidates.is_empty() || self.overlay_kind() != "" {
            return;
        }
        let Some(id) = self.active_document() else {
            return;
        };
        let Some(d) = self.docs.documents.get(&id) else {
            return;
        };
        // 編集中の内容を上書きしないよう、読込済みで何も編集していない文書にだけ出す。読み取り専用の文書には出さないが、
        // 理由が `InvalidBytes` だけ (`pua_escape == Enabled`) の文書には出す (03 §10.3 の M23: 不正バイトを含む文書を
        // 解除して編集した後のスワップは、次に開くと読み取り専用に戻るため)
        if d.load_state != LoadState::Ready || d.is_dirty() || !d.recovery_allowed() {
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
            .io
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
        let entry = self.io.swap.candidates.remove(i);
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
            .unwrap_or_else(|| self.untitled_label(d));
        let mut details = Vec::new();
        let args = [("path", name.as_str())];
        if changed {
            details.push(self.catalog().render(
                keys::UI_CONFIRM_SWAP_RECOVERY_CHANGED,
                &args,
                &|_| None,
            ));
        }
        if entry.sensitive {
            details.push(self.catalog().render(
                keys::UI_CONFIRM_SWAP_RECOVERY_SENSITIVE,
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
            message: None,
            action: ConfirmAction::SwapRecovery {
                entry: Box::new(entry),
                doc: id,
                revision,
            },
        });
    }

    /// 開いた文書と同じファイルのスワップを、動いている別のプロセスが書いていれば警告する (1 件につき 1 回)。
    fn swap_warn_alive(&mut self) {
        if self.io.swap.alive.is_empty() {
            return;
        }
        let Some(d) = self
            .active_document()
            .and_then(|id| self.docs.documents.get(&id))
            .filter(|d| d.load_state == LoadState::Ready)
        else {
            return;
        };
        let Some(path) = d.path.clone() else {
            return;
        };
        let (hit, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.io.swap.alive)
            .into_iter()
            .partition(|e| e.path.as_ref().is_some_and(|p| same_path(p, &path)));
        self.io.swap.alive = rest;
        for e in hit {
            self.show_message(
                MessageLevel::Warn,
                keys::MSG_SWAP_OTHER_INSTANCE,
                &[("path", &path.display()), ("pid", &e.pid.to_string())],
            );
        }
    }

    /// 提案への答え。`apply` は本文を読むジョブを投入し、`discard` はスワップを消す (この選択が破棄の確認)。
    pub(crate) fn choose_swap_recovery(
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
            self.io.pending.insert(
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
            self.io.pending.insert(
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
    pub(crate) fn finish_swap_read(
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
                    keys::MSG_SWAP_READ_FAILED,
                    &[("dir", &entry.dir.display())],
                );
                return;
            }
        };
        let Some(d) = self.docs.documents.get_mut(&doc) else {
            return;
        };
        if d.revision() != revision || !d.recovery_allowed() {
            // 提案の後で文書が変わった: 適用しない。スワップは残し、もう一度提案できるようにする
            self.show_message(MessageLevel::Warn, keys::MSG_SWAP_CHANGED_SINCE_OFFER, &[]);
            self.io.swap.candidates.push(entry);
            return;
        }
        let name = entry
            .path
            .as_ref()
            .map(FsPath::display)
            .or_else(|| entry.untitled_name.clone())
            .or_else(|| {
                self.docs
                    .documents
                    .get(&doc)
                    .map(|d| self.untitled_label(d))
            })
            .unwrap_or_default();
        let Some(d) = self.docs.documents.get_mut(&doc) else {
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
                keys::MSG_SWAP_READ_FAILED,
                &[("dir", &entry.dir.display())],
            );
            return;
        };
        // 無題の文書は、スワップの形式 (文字コード・BOM・改行) を引き継ぐ。パスのある文書は開いた形式のまま。
        // 適用に失敗したら元の形式に戻す (下の `Err`)
        let saved_format = d.path.is_none().then(|| d.format.clone());
        // 復旧では「編集を許可」を持ち越さない (03 §8.3.1 の「解除の持ち越し」: 復旧では付け直す)。書式の変更を
        // 適用する前に記録を消す (記録があると `InvalidBytes` を付けないため)。適用に失敗したら戻す
        let saved_released = std::mem::take(&mut d.invalid_bytes_released);
        if d.path.is_none() {
            // 文字コードを変えるので、今の文字コードのものでない `escape_encoding` は残さない (不変条件、03 §8.3.1)
            if d.format.escape_encoding != Some(body.encoding) {
                d.format.escape_encoding = None;
            }
            d.format.encoding = body.encoding;
            d.format.has_bom = body.has_bom;
            d.format.default_eol = body.default_eol;
        }
        // 本文のエスケープ文字がどの文字コードのバイトか (03 §8.3.1 の「エスケープ文字の由来の文字コード」の表の
        // 「復旧の適用」の行)。Undo では前の値に戻る (`FormatChange::Recovery` に持たせる)
        let (escape_encoding, pua_escape) =
            recovered_escape(&d.format, body.pua_escaped, body.encoding);
        let tx = Transaction::new(cs, Origin::Recovery, self.now)
            .with_format_change(d.recovery_change(escape_encoding, pua_escape));
        match self.apply_transaction(doc, tx) {
            Ok(_) => {
                self.io.swap.recovered.entry(doc).or_default().push(entry);
                // 適用した後も読み取り専用 (不正バイトを含む文書) なら、切替で編集を許可すると続けられることを示す
                // (03 §10.3。`Origin::Recovery` は読み取り専用の拒否の例外)
                let read_only = self
                    .docs
                    .documents
                    .get(&doc)
                    .is_some_and(|d| !d.read_only.is_empty());
                let key = if read_only {
                    keys::INFO_SWAP_APPLIED_READ_ONLY
                } else {
                    keys::INFO_SWAP_RECOVERED
                };
                self.show_message(MessageLevel::Info, key, &[("path", &name)]);
            }
            Err(_) => {
                // 文書は変わっていない。無題の文書に写した形式と、解除の記録も戻す
                if let Some(d) = self.docs.documents.get_mut(&doc) {
                    d.invalid_bytes_released = saved_released;
                    if let Some(f) = saved_format {
                        d.format = f;
                    }
                }
                self.show_message(
                    MessageLevel::Error,
                    keys::MSG_SWAP_READ_FAILED,
                    &[("dir", &entry.dir.display())],
                );
                self.io.swap.candidates.push(entry);
            }
        }
    }

    /// 他のプロセスのスワップを消したジョブの結果。
    pub(crate) fn finish_swap_remove(&mut self, entry: &SwapEntry, notify: bool, r: JobResult) {
        match r {
            JobResult::SwapRemoved => {
                if notify {
                    self.show_message(MessageLevel::Info, keys::INFO_SWAP_DISCARDED, &[]);
                }
            }
            JobResult::Failed {
                err: JobError::Fs(_),
                ..
            } => self.show_message(
                MessageLevel::Warn,
                keys::MSG_SWAP_REMOVE_FAILED,
                &[("dir", &entry.dir.display())],
            ),
            _ => {}
        }
    }

    /// 適用した古いスワップを、新しいスワップが書けた・文書が保存された (dirty でなくなった)・文書が閉じられたときに消す。
    pub(crate) fn swap_release_recovered(&mut self) {
        if self.io.swap.recovered.is_empty() {
            return;
        }
        let ready: Vec<DocumentId> = self
            .io
            .swap
            .recovered
            .keys()
            .copied()
            .filter(|id| {
                self.docs.documents.get(id).is_none_or(|d| !d.is_dirty()) || self.swap_written(*id)
            })
            .collect();
        for id in ready {
            for entry in self.io.swap.recovered.remove(&id).unwrap_or_default() {
                let job = self.jobs.spawn(
                    None,
                    eve_view::Revision::INITIAL,
                    JobKind::SwapRemove {
                        entry: entry.clone(),
                    },
                );
                self.io.pending.insert(
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
    pub(crate) fn swap_recovered_leftovers(&self) -> Vec<SwapEntry> {
        self.io.swap.recovered.values().flatten().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::recovered_escape;
    use crate::Encoding;
    use crate::document::TextFormat;
    use eve_encoding::PuaEscapeMode;

    fn fmt(
        encoding: Encoding,
        pua_escape: PuaEscapeMode,
        escape_encoding: Option<Encoding>,
    ) -> TextFormat {
        TextFormat {
            encoding,
            pua_escape,
            escape_encoding,
            ..TextFormat::new_document()
        }
    }

    // 03 §8.3.1 の「エスケープ文字の由来の文字コード」の表の「復旧の適用」の行
    #[test]
    fn 復旧の適用で決まる由来の文字コード() {
        use Encoding::{Iso2022Jp, ShiftJis, Utf8};
        use PuaEscapeMode::{DisabledCollision, DisabledStateful, Enabled};
        // pua_escaped が偽なら None (前の値が Some でも。本文を置き換えるので)。pua_escape は変えない
        for before in [None, Some(Utf8)] {
            assert_eq!(
                recovered_escape(&fmt(Utf8, Enabled, before), false, Utf8),
                (None, Enabled)
            );
        }
        assert_eq!(
            recovered_escape(&fmt(Utf8, DisabledCollision, None), false, Utf8),
            (None, DisabledCollision)
        );
        // 同じ文字コードで Enabled なら Some (ディスクのファイルが不正バイトを含まなくても)
        assert_eq!(
            recovered_escape(&fmt(Utf8, Enabled, None), true, Utf8),
            (Some(Utf8), Enabled)
        );
        assert_eq!(
            recovered_escape(&fmt(ShiftJis, Enabled, Some(ShiftJis)), true, ShiftJis),
            (Some(ShiftJis), Enabled)
        );
        // 文字コードが違えば None (前の値が Some でも)
        assert_eq!(
            recovered_escape(&fmt(Utf8, Enabled, Some(Utf8)), true, ShiftJis),
            (None, Enabled)
        );
        // 衝突の文書は Enabled にして、表のとおり
        assert_eq!(
            recovered_escape(&fmt(Utf8, DisabledCollision, None), true, Utf8),
            (Some(Utf8), Enabled)
        );
        assert_eq!(
            recovered_escape(&fmt(Utf8, DisabledCollision, None), true, ShiftJis),
            (None, Enabled)
        );
        // 状態付きの文書は None のまま
        assert_eq!(
            recovered_escape(&fmt(Iso2022Jp, DisabledStateful, None), true, Iso2022Jp),
            (None, DisabledStateful)
        );
        // 状態付きの文字コードを写した無題の文書 (Enabled) も None
        assert_eq!(
            recovered_escape(&fmt(Iso2022Jp, Enabled, None), true, Iso2022Jp),
            (None, Enabled)
        );
    }
}
