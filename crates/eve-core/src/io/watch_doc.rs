//! ファイル監視の本体 (FR-117、docs/03 §9.5.1 の WATCH-1〜WATCH-11。v0.5 の M33 の段 8)。
//!
//! - 監視の張り外し (WATCH-1 / WATCH-2): 文書の canonical のパスの親ディレクトリを数え上げで 1 つの監視にし、最後の文書を
//!   閉じたら外す。失効 (`Lost`) は、開く・保存・再読込の確かめで張り直す。監視できないときは理由の種類ごとに 1 回知らせる。
//! - 待ち合わせ (WATCH-4): 文書ごとに最後の知らせから `WATCH_SETTLE`、最初の知らせから最長 `WATCH_SETTLE_MAX`。打ち切りの
//!   照合は待ち合わせの期限を消さない (期限でもう 1 回照合する)。期限は `next_deadline` に入れ、一定の間隔では起きない。
//! - 照合 (WATCH-5): `Metadata { purpose: WatchCheck }` を `io` に積み、`disk_stamp` と比べる。その文書の保存・読込・照合の
//!   ジョブの間は延ばし、結果が届いてから待たずに照合する。
//! - 知らせ方 (WATCH-6 / WATCH-7 / WATCH-8): 印 (`Document.external_pending`) とメッセージ。表示中の未編集の文書は確認なしで
//!   読み直す。モーダルの確認は出さない。
//!
//! spec: 03 §9.5.1, 05 §12.2, 02 §4.4

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::rc::Rc;

use eve_view::{DocumentId, JobId};

use crate::document::{ExternalPending, LoadState, ReadOnlyReason};
use crate::editor::state::EditorState;
use crate::io::file::same_on_disk;
use crate::io::fs::{FileStamp, FsErrorKind, FsKind};
use crate::io::fspath::FsPath;
use crate::io::open::OpenKind;
use crate::io::pending::Pending;
use crate::io::watch::{FileWatcher, FsEvent, FsEventKind, WatchError, WatchId};
use crate::jobs::{JobError, JobKind, JobResult, MetaPurpose};
use crate::time::{Millis, Tick};
use crate::ui_model::keys;
use eve_config::ExternalChangePolicy;

/// 待ち合わせ: 最後の知らせからこの時間だけ待って照合する (03 §9.5.1 の WATCH-4、05 §12.2 の 2)。
pub const WATCH_SETTLE: Millis = Millis(100);
/// 待ち合わせの上限: 最初の知らせからこの時間で打ち切って照合する (書き続けるログのファイルでも知らせが出る。WATCH-4)。
pub const WATCH_SETTLE_MAX: Millis = Millis(1000);
/// 同じ文書を監視で読み直す最短の間隔 (WATCH-6 の (b)。書き続けるログのファイルで、読み直すたびに旧全文を Undo に積まない)。
pub const WATCH_RELOAD_MIN_INTERVAL: Millis = Millis(5000);
/// 監視で読み直す文書の大きさの上限 (WATCH-6 の (a)。05 §5.5 の内容ハッシュの上限と同じ 16 MiB)。
pub const WATCH_RELOAD_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// 照合の失敗 (共有違反など) のやり直しの間隔 (1 回目の失敗の 0.5 s 後、2 回目の失敗の 2 s 後。WATCH-5)。
const WATCH_RETRY: [Millis; 2] = [Millis(500), Millis(2000)];

/// 監視の状態 (`EditorState::watch`)。
#[derive(Default)]
pub(crate) struct WatchState {
    /// 監視の Port (`Ports.watcher`)。`None` なら監視しない。
    watcher: Option<Rc<dyn FileWatcher>>,
    /// 監視を張っている (`file.watch.enabled` などから決めた、最後に当てた値)。
    enabled: bool,
    /// ディレクトリ (比べる形の canonical のパス) → 監視。
    dirs: BTreeMap<FsPath, DirWatch>,
    /// 要求の識別 → ディレクトリ。外した・張り直した古い識別は載せない (その知らせは捨てる)。
    ids: BTreeMap<WatchId, FsPath>,
    /// 文書 → 監視の登録 (ディレクトリと、照らす名前)。
    docs: BTreeMap<DocumentId, DocWatch>,
    /// 開く・読み直すジョブで、ネットワークの FS (`FsKind::Network`) の上と分かった文書 (`FileWatcher` に頼まない。05 §12.2 の 4)。
    network: BTreeSet<DocumentId>,
    /// 文書ごとの待ち合わせ (WATCH-4)。
    waits: BTreeMap<DocumentId, Wait>,
    /// 時刻を決めた照合 (照合の失敗のやり直し・読み直しの最短の間隔の後の照合)。
    due: BTreeMap<DocumentId, Tick>,
    /// 続けて失敗した照合の回数 (やり直しの間隔を選ぶ)。
    retries: BTreeMap<DocumentId, usize>,
    /// 期限が来たが、その文書のジョブの後に延ばした照合 (値は切替の照合か、WATCH-8 の (a))。
    deferred: BTreeMap<DocumentId, bool>,
    /// 同じ時刻に期限が来た照合の回 (メッセージを 1 件にまとめる、WATCH-6)。
    rounds: BTreeMap<u64, Round>,
    next_round: u64,
    /// 監視からの再読込の読込ジョブと、照合で得た Stamp (取り消したときの印に使う)。
    reloads: BTreeMap<DocumentId, (JobId, FileStamp)>,
    /// 監視で最後に読み直した時刻 (WATCH-6 の (b))。
    reloaded_at: BTreeMap<DocumentId, Tick>,
    /// 知らせた監視できない理由の種類 (プロセスで 1 回、WATCH-2)。
    noted: BTreeSet<&'static str>,
    /// 前に出ていた文書 (切替を見つける、WATCH-8 の (a))。
    front: Option<DocumentId>,
    /// 回が終わった後、ほかの文書の待ち合わせの期限が近いので知らせを保留している印 (文書, 削除か) と、保留を始めた時刻
    /// (WATCH-6 のまとめ方。保留は最長 `WATCH_SETTLE`)。
    held: Vec<(DocumentId, bool)>,
    held_since: Option<Tick>,
    /// 照合の後に知らせが届いたので、結果を当てずに捨てた文書 (続けては捨てない。書き続けるファイルでも当てる)。
    discarded: BTreeSet<DocumentId>,
    /// フォルダが無い所に開いた新規ファイル (保存してフォルダができるまで監視を頼まない、WATCH-1)。
    no_parent: BTreeSet<DocumentId>,
    /// その文書のジョブの間で、やり直しを待っているフォーカス復帰の確認 (03 §9.5 の M9d、WATCH-8 の (b))。
    focus_pending: Option<DocumentId>,
}

/// 1 つのディレクトリの監視。
struct DirWatch {
    /// 要求の識別 (ネットワークの FS で頼まなかったときは `None`)。
    id: Option<WatchId>,
    state: DirState,
    /// このディレクトリの文書。
    docs: BTreeSet<DocumentId>,
    /// 照らす名前 → 文書 (WATCH-3 / WATCH-11 の表引き)。
    names: BTreeMap<OsString, BTreeSet<DocumentId>>,
}

/// 監視の状態 (02 §4.4 の知らせの順の約束)。
#[derive(Clone, Debug, PartialEq, Eq)]
enum DirState {
    /// 頼んで結果を待っている (この間の知らせも受ける)。
    Requested,
    /// 登録できた。
    Active,
    /// 失効した (次の確かめで張り直す)。
    Lost,
    /// 登録できなかった (この識別の知らせは捨てる)。
    Failed(WatchError),
}

/// 文書の監視の登録。
struct DocWatch {
    dir: FsPath,
    key: OsString,
}

/// 待ち合わせ (WATCH-4)。`first` は打ち切りの数え始め (打ち切りの照合の後、次の知らせまで `None`)、`last` は最後の知らせ。
#[derive(Clone, Copy, Debug)]
struct Wait {
    first: Option<Tick>,
    last: Tick,
}

impl Wait {
    fn settle_at(self) -> Tick {
        self.last + WATCH_SETTLE
    }

    fn cap_at(self) -> Option<Tick> {
        self.first.map(|f| f + WATCH_SETTLE_MAX)
    }

    fn due(self) -> Tick {
        self.cap_at()
            .map_or(self.settle_at(), |c| c.min(self.settle_at()))
    }
}

/// 照合の回。
#[derive(Default)]
struct Round {
    outstanding: usize,
    /// 印を付けた文書 (削除か)。
    marked: Vec<(DocumentId, bool)>,
}

/// 照らす名前 (WATCH-3): Windows は大文字小文字を区別しない序数の比較、Unix はバイト列。
fn name_key(name: &OsStr) -> OsString {
    if cfg!(windows) {
        let folded: String = name
            .to_string_lossy()
            .chars()
            .map(|c| {
                let mut up = c.to_uppercase();
                match (up.next(), up.next()) {
                    (Some(u), None) => u,
                    _ => c,
                }
            })
            .collect();
        OsString::from(folded)
    } else {
        name.to_owned()
    }
}

/// Windows の短い名前 (8.3) の形の名前か (WATCH-3): 基の名前は 8 文字以下で、6 文字以下の前の部分・`~`・数字の並び、
/// 拡張子は 3 文字以下 (`LONGNA~1.TXT`)。`~$x.docx` (Office の一時ファイル)・`a.txt~` (バックアップ) は当たらない。
fn maybe_short_name(name: &OsStr) -> bool {
    cfg!(windows) && is_short_name_form(&name.to_string_lossy())
}

fn is_short_name_form(name: &str) -> bool {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, Some(e)),
        None => (name, None),
    };
    if ext.is_some_and(|e| e.is_empty() || e.chars().count() > 3 || e.contains('~')) {
        return false;
    }
    let Some((head, num)) = stem.rsplit_once('~') else {
        return false;
    };
    let head_len = head.chars().count();
    (1..=6).contains(&head_len)
        && !head.contains('~')
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
        && head_len + 1 + num.len() <= 8
}

/// 監視できない理由の種類 (知らせを 1 回にする単位と、`ui.watch.reason.*` の語)。
fn reason_kind(err: &WatchError) -> (&'static str, &'static str, String) {
    match err {
        WatchError::Unsupported { fs } if fs == "network" => {
            ("network", keys::UI_WATCH_REASON_NETWORK, String::new())
        }
        WatchError::Unsupported { fs } => (
            "unsupported_fs",
            keys::UI_WATCH_REASON_UNSUPPORTED_FS,
            fs.clone(),
        ),
        WatchError::Limit => ("limit", keys::UI_WATCH_REASON_LIMIT, String::new()),
        WatchError::Denied => ("denied", keys::UI_WATCH_REASON_DENIED, String::new()),
        WatchError::NotFound => ("not_found", keys::UI_WATCH_REASON_NOT_FOUND, String::new()),
        WatchError::Other => ("other", keys::UI_WATCH_REASON_FAILED, String::new()),
    }
}

/// 照合の結果からの、監視での再読込の可否 (WATCH-6 の上限)。
enum Reloadable {
    Yes,
    /// 最短の間隔の内: この時刻の後にもう一度照合する。
    After(Tick),
    No,
}

impl EditorState {
    /// 監視の Port を渡す (`Editor::new` が `Ports.watcher` から)。
    pub(crate) fn set_watcher(&mut self, watcher: Option<Rc<dyn FileWatcher>>) {
        self.watch.watcher = watcher;
        self.watch_reconfigure();
    }

    /// 監視を張るか: Port があり、`file.watch.enabled` で、`file.reload_on_external_change` が `never` でない (WATCH-1)。
    fn watch_wanted(&self) -> bool {
        self.watch.watcher.is_some()
            && self.config().watch_enabled
            && self.config().reload_on_external_change != ExternalChangePolicy::Never
    }

    /// 設定が変わった: 監視を張る・外す (WATCH-1。`config.reload` で即時に効く)。
    pub(crate) fn watch_reconfigure(&mut self) {
        let on = self.watch_wanted();
        if on == self.watch.enabled {
            return;
        }
        self.watch.enabled = on;
        if on {
            let docs: Vec<DocumentId> = self
                .docs
                .documents
                .iter()
                .filter(|(_, d)| d.load_state == LoadState::Ready && d.path.is_some())
                .map(|(id, _)| *id)
                .collect();
            for doc in docs {
                self.watch_sync_doc(doc);
            }
        } else {
            if let Some(w) = self.watch.watcher.clone() {
                for id in self.watch.ids.keys() {
                    w.unwatch(*id);
                }
            }
            let ws = &mut self.watch;
            ws.dirs.clear();
            ws.ids.clear();
            ws.docs.clear();
            ws.waits.clear();
            ws.due.clear();
            ws.retries.clear();
            ws.deferred.clear();
            ws.reloads.clear();
            ws.rounds.clear();
            ws.held.clear();
            ws.held_since = None;
            ws.discarded.clear();
            // 監視を止めたら、監視で付けた印も外す (監視していない文書は v0.4 の確かめ方に戻る。「無視」の
            // `external_seen` は確認の流れのものなので残す)
            for d in self.docs.documents.values_mut() {
                d.external_pending = None;
            }
            self.mark_dirty_render();
        }
    }

    /// フォルダの無い所に開いた新規ファイル: 保存してフォルダができるまで監視を頼まない (WATCH-1)。
    pub(crate) fn watch_note_no_parent(&mut self, doc: DocumentId) {
        self.watch.no_parent.insert(doc);
    }

    /// 保存できた: フォルダがあるので監視を確かめる (WATCH-1)。
    pub(crate) fn watch_after_save(&mut self, doc: DocumentId) {
        self.watch.no_parent.remove(&doc);
        self.watch_sync_doc(doc);
    }

    /// 開く・読み直すジョブで分かった、文書の置かれた FS の種類を覚える (ネットワークの FS は頼まずに縮退する)。
    pub(crate) fn watch_note_fs(&mut self, doc: DocumentId, fs_kind: FsKind) {
        if fs_kind == FsKind::Network {
            self.watch.network.insert(doc);
        } else {
            self.watch.network.remove(&doc);
        }
    }

    /// 名前を付けて保存で文書のパスが変わった: 前の FS の種類を忘れる (新しい置き場所は監視の Port が判定する)。
    pub(crate) fn watch_forget_fs(&mut self, doc: DocumentId) {
        self.watch.network.remove(&doc);
    }

    /// 文書の監視を確かめる (WATCH-1: 開いた・名前を付けて保存でパスが変わった・再読込した・保存したとき)。登録が無ければ
    /// 張り、パスが変わっていれば古いディレクトリの数を減らし、失効していれば張り直す。
    pub(crate) fn watch_sync_doc(&mut self, doc: DocumentId) {
        if !self.watch.enabled || self.watch.no_parent.contains(&doc) {
            return;
        }
        let target = self.docs.documents.get(&doc).and_then(|d| {
            let path = d.canonical.as_ref().or(d.path.as_ref())?;
            let name = path.as_path().file_name()?.to_owned();
            let dir = path.parent()?;
            Some((
                FsPath::new(crate::io::same_file::canonical_key(&dir)),
                name_key(&name),
            ))
        });
        let Some((dir, key)) = target else {
            self.watch_unregister(doc);
            return;
        };
        let same = self
            .watch
            .docs
            .get(&doc)
            .is_some_and(|r| r.dir == dir && r.key == key);
        if !same {
            self.watch_unregister(doc);
            self.watch_register(doc, dir.clone(), key);
        }
        self.watch_ensure_dir(&dir);
    }

    fn watch_register(&mut self, doc: DocumentId, dir: FsPath, key: OsString) {
        if !self.watch.dirs.contains_key(&dir) {
            let network = self.watch.network.contains(&doc);
            let entry = match (&self.watch.watcher, network) {
                (Some(w), false) => {
                    let id = w.watch_dir(&dir);
                    self.watch.ids.insert(id, dir.clone());
                    DirWatch {
                        id: Some(id),
                        state: DirState::Requested,
                        docs: BTreeSet::new(),
                        names: BTreeMap::new(),
                    }
                }
                // ネットワークの FS (開くジョブで分かった) は頼まない: 応答しない共有で `io` を止めない (05 §12.2 の 4)
                _ => DirWatch {
                    id: None,
                    state: DirState::Failed(WatchError::Unsupported {
                        fs: "network".to_owned(),
                    }),
                    docs: BTreeSet::new(),
                    names: BTreeMap::new(),
                },
            };
            let failed = match &entry.state {
                DirState::Failed(e) => Some(e.clone()),
                _ => None,
            };
            self.watch.dirs.insert(dir.clone(), entry);
            if let Some(e) = failed {
                self.watch_note_unavailable(&e, doc);
            }
        }
        if let Some(entry) = self.watch.dirs.get_mut(&dir) {
            entry.docs.insert(doc);
            entry.names.entry(key.clone()).or_default().insert(doc);
        }
        self.watch.docs.insert(doc, DocWatch { dir, key });
    }

    /// 失効した (`Lost`)・ディレクトリが無くて登録できなかった監視を張り直す (新しい要求を積んでから古い要求を外す。
    /// 05 §12.2 の 9 の (b))。
    fn watch_ensure_dir(&mut self, dir: &FsPath) {
        let Some(w) = self.watch.watcher.clone() else {
            return;
        };
        let Some(entry) = self.watch.dirs.get_mut(dir) else {
            return;
        };
        let renew = entry.id.is_some()
            && matches!(
                entry.state,
                DirState::Lost | DirState::Failed(WatchError::NotFound)
            );
        if !renew {
            return;
        }
        let id = w.watch_dir(dir);
        if let Some(old) = entry.id.replace(id) {
            w.unwatch(old);
            self.watch.ids.remove(&old);
        }
        entry.state = DirState::Requested;
        self.watch.ids.insert(id, dir.clone());
    }

    /// 文書の監視の登録を外す (最後の文書なら監視を外す)。待ち合わせ・延ばした照合も捨てる。
    fn watch_unregister(&mut self, doc: DocumentId) {
        if let Some(reg) = self.watch.docs.remove(&doc)
            && let Some(entry) = self.watch.dirs.get_mut(&reg.dir)
        {
            entry.docs.remove(&doc);
            if let Some(set) = entry.names.get_mut(&reg.key) {
                set.remove(&doc);
                if set.is_empty() {
                    entry.names.remove(&reg.key);
                }
            }
            if entry.docs.is_empty() {
                if let Some(id) = entry.id {
                    if let Some(w) = &self.watch.watcher {
                        w.unwatch(id);
                    }
                    self.watch.ids.remove(&id);
                }
                self.watch.dirs.remove(&reg.dir);
            }
        }
        self.watch.waits.remove(&doc);
        self.watch.due.remove(&doc);
        self.watch.retries.remove(&doc);
        self.watch.deferred.remove(&doc);
    }

    /// 文書を閉じる: 監視の登録と、その文書の監視の覚えをすべて捨てる (閉じた文書の古い知らせ・結果は使わない)。
    /// 照合の回に数えていたジョブは回から外す (`take_pending_for` がジョブを取り消す)。
    pub(crate) fn watch_forget(&mut self, doc: DocumentId) {
        self.watch_unregister(doc);
        self.watch.network.remove(&doc);
        self.watch.reloads.remove(&doc);
        self.watch.reloaded_at.remove(&doc);
        self.watch.discarded.remove(&doc);
        self.watch.no_parent.remove(&doc);
        self.watch.held.retain(|(d, _)| *d != doc);
        if self.watch.front == Some(doc) {
            self.watch.front = None;
        }
        if self.watch.focus_pending == Some(doc) {
            self.watch.focus_pending = None;
        }
        let rounds: Vec<u64> = self
            .io
            .pending
            .values()
            .filter_map(|p| match p {
                Pending::WatchCheck {
                    doc: d,
                    round: Some(r),
                    ..
                } if *d == doc => Some(*r),
                _ => None,
            })
            .collect();
        for r in rounds {
            self.watch_round_done(r, None);
        }
        self.watch_flush_held();
    }

    /// 監視の知らせ (`HostMsg::Fs`、02 §4.4)。
    pub(crate) fn on_fs_event(&mut self, ev: FsEvent) {
        let Some(dir) = self.watch.ids.get(&ev.watch).cloned() else {
            // 外した・張り直した古い要求の知らせ
            return;
        };
        let Some(entry) = self.watch.dirs.get_mut(&dir) else {
            return;
        };
        // 登録できなかった要求の知らせは、`Failed` の後に届いたものも捨てる (02 §4.4 の知らせの順の約束)
        if matches!(entry.state, DirState::Failed(_)) {
            return;
        }
        let now = self.now;
        let targets: Vec<DocumentId> = match ev.kind {
            FsEventKind::Started => {
                entry.state = DirState::Active;
                Vec::new()
            }
            FsEventKind::Failed { err } => {
                entry.state = DirState::Failed(err.clone());
                let docs: Vec<DocumentId> = entry.docs.iter().copied().collect();
                // `Failed` の前に届いた同じ識別の知らせは捨てる (監視していない)
                for d in &docs {
                    self.watch.waits.remove(d);
                }
                if let Some(first) = docs.first() {
                    self.watch_note_unavailable(&err, *first);
                }
                Vec::new()
            }
            FsEventKind::Changed { name } => match entry.names.get(&name_key(&name)) {
                Some(set) => set.iter().copied().collect(),
                // Windows の短い名前で届いたかもしれない: ディレクトリの全文書を照合する (名前を直す I/O をしない)
                None if maybe_short_name(&name) => entry.docs.iter().copied().collect(),
                // 文書でない名前 (自分の保存の一時ファイル・退避名・ほかのファイル) は捨てる
                None => Vec::new(),
            },
            FsEventKind::Rescan => entry.docs.iter().copied().collect(),
            FsEventKind::Lost => {
                entry.state = DirState::Lost;
                entry.docs.iter().copied().collect()
            }
        };
        for doc in targets {
            let w = self.watch.waits.entry(doc).or_insert(Wait {
                first: Some(now),
                last: now,
            });
            w.last = now;
            if w.first.is_none() {
                w.first = Some(now);
            }
        }
    }

    /// 監視の次の期限 (待ち合わせ・時刻を決めた照合)。
    pub(crate) fn watch_deadline(&self) -> Option<Tick> {
        let waits = self.watch.waits.values().map(|w| w.due());
        let due = self.watch.due.values().copied();
        let held = self.watch.held_since.map(|t| t + WATCH_SETTLE);
        waits.chain(due).chain(held).min()
    }

    /// 期限の来た照合を積む (Editor の `tick` から)。前に出た文書が変われば、切替の照合 (WATCH-8 の (a)) も積む。
    pub(crate) fn watch_tick(&mut self) {
        // ジョブの結果無しに `Ready` に戻った文書 (開く前の確認を `Esc` で閉じた再読込など) の延ばした照合も再開する
        self.watch_resume();
        let now = self.now;
        let mut fire: Vec<DocumentId> = Vec::new();
        let mut settled: Vec<DocumentId> = Vec::new();
        for (doc, w) in &mut self.watch.waits {
            if now >= w.settle_at() {
                // 待ち合わせの期限: この文書の知らせはすべて、この照合 (以降) が取り込む
                fire.push(*doc);
                settled.push(*doc);
            } else if w.cap_at().is_some_and(|c| now >= c) {
                // 打ち切りの照合: 待ち合わせの期限は消さない (期限が後ならもう 1 回照合する、WATCH-4)
                fire.push(*doc);
                w.first = None;
            }
        }
        for doc in settled {
            self.watch.waits.remove(&doc);
        }
        let due: Vec<DocumentId> = self
            .watch
            .due
            .iter()
            .filter(|(_, t)| now >= **t)
            .map(|(d, _)| *d)
            .collect();
        for doc in due {
            self.watch.due.remove(&doc);
            if !fire.contains(&doc) {
                fire.push(doc);
            }
        }
        if !fire.is_empty() {
            let round = self.watch_new_round();
            for doc in fire {
                self.watch_check(doc, Some(round), false);
            }
            self.watch_round_done(round, None);
        }
        self.watch_front_check();
        self.watch_flush_held();
    }

    /// 切替 (WATCH-8 の (a)): 前に出た文書に外部の状態があれば、照合のジョブを積む。
    fn watch_front_check(&mut self) {
        let front = self.active_document();
        if front == self.watch.front {
            return;
        }
        self.watch.front = front;
        if !self.watch.enabled {
            return;
        }
        if let Some(doc) = front
            && self
                .docs
                .documents
                .get(&doc)
                .is_some_and(|d| d.external_pending.is_some())
        {
            self.watch_check(doc, None, true);
        }
    }

    /// ジョブの結果を受けた後: 延ばした照合のうち、その文書のジョブが無くなったものを待たずに積む (WATCH-5)。
    pub(crate) fn watch_resume(&mut self) {
        self.resume_focus_check();
        if self.watch.deferred.is_empty() {
            return;
        }
        let ready: Vec<(DocumentId, bool)> = self
            .watch
            .deferred
            .iter()
            .filter(|(d, _)| !self.watch_busy(**d))
            .map(|(d, s)| (*d, *s))
            .collect();
        if ready.is_empty() {
            return;
        }
        let round = self.watch_new_round();
        for (doc, switch) in ready {
            self.watch.deferred.remove(&doc);
            self.watch_check(doc, (!switch).then_some(round), switch);
        }
        self.watch_round_done(round, None);
    }

    fn watch_new_round(&mut self) -> u64 {
        self.watch.next_round += 1;
        let r = self.watch.next_round;
        // 回を閉じる `watch_round_done(r, None)` の分を 1 つ数えておく (積んでいる間に回が終わらないように)
        self.watch.rounds.insert(
            r,
            Round {
                outstanding: 1,
                marked: Vec::new(),
            },
        );
        r
    }

    /// その文書に、内容かディスクの状態に関わるジョブがある、または読込中 (WATCH-5。`Pending::busy_doc`)。
    fn watch_busy(&self, doc: DocumentId) -> bool {
        self.doc_has_job(doc)
            || self
                .docs
                .documents
                .get(&doc)
                .is_some_and(|d| d.load_state != LoadState::Ready)
    }

    /// フォーカス復帰の確認が、その文書のジョブ (監視の照合など) の間で飛ばされた: ジョブが無くなったら、まだその文書が
    /// 前に出ていれば確かめ直す (03 §9.5.1 の WATCH-8 の (b))。
    pub(crate) fn defer_focus_check(&mut self, doc: DocumentId) {
        self.watch.focus_pending = Some(doc);
    }

    fn resume_focus_check(&mut self) {
        let Some(doc) = self.watch.focus_pending else {
            return;
        };
        if self.active_document() != Some(doc) {
            self.watch.focus_pending = None;
            return;
        }
        if self.watch_busy(doc) {
            return;
        }
        self.watch.focus_pending = None;
        self.on_focus_gained();
    }

    /// 照合のジョブを積む。その文書のジョブがあれば延ばす。`round` は同じ時刻の照合の回 (`None` は切替の照合)。
    fn watch_check(&mut self, doc: DocumentId, round: Option<u64>, switch: bool) {
        let Some(d) = self.docs.documents.get(&doc) else {
            return;
        };
        let Some(path) = d.path.clone() else {
            return;
        };
        if self.watch_busy(doc) {
            let prev = self.watch.deferred.get(&doc).copied().unwrap_or(false);
            self.watch.deferred.insert(doc, prev || switch);
            return;
        }
        let job = self.jobs.spawn(
            Some(doc),
            d.revision(),
            JobKind::Metadata {
                path,
                purpose: MetaPurpose::WatchCheck,
                known_ids: Vec::new(),
            },
        );
        let at = self.now;
        self.io.pending.insert(
            job,
            Pending::WatchCheck {
                doc,
                round,
                switch,
                at,
            },
        );
        if let Some(r) = round
            && let Some(rd) = self.watch.rounds.get_mut(&r)
        {
            rd.outstanding += 1;
        }
    }

    /// 照合の回の 1 つが終わった (`marked` は付けた印)。回が全部終われば、印のメッセージを 1 件にまとめて出す。
    fn watch_round_done(&mut self, round: u64, marked: Option<(DocumentId, bool)>) {
        let Some(rd) = self.watch.rounds.get_mut(&round) else {
            return;
        };
        if let Some(m) = marked {
            rd.marked.push(m);
        }
        rd.outstanding = rd.outstanding.saturating_sub(1);
        if rd.outstanding > 0 {
            return;
        }
        let Some(rd) = self.watch.rounds.remove(&round) else {
            return;
        };
        if !rd.marked.is_empty() {
            if self.watch.held.is_empty() {
                self.watch.held_since = Some(self.now);
            }
            self.watch.held.extend(rd.marked);
        }
        self.watch_flush_held();
    }

    /// 保留した印の知らせを出す (WATCH-6): 照合の回が残っていない、かつ、ほかの文書の待ち合わせの期限が `WATCH_SETTLE` の内に
    /// 無いとき (近ければ、その回の印と合わせて 1 件にする)。保留は最長 `WATCH_SETTLE` (書き続けるファイルで知らせが
    /// 出なくならない)。照合は早めない。
    fn watch_flush_held(&mut self) {
        let Some(since) = self.watch.held_since else {
            return;
        };
        if !self.watch.rounds.is_empty() {
            return;
        }
        let horizon = self.now + WATCH_SETTLE;
        let near = self.watch.waits.values().any(|w| w.due() <= horizon);
        if near && self.now < since + WATCH_SETTLE {
            return;
        }
        self.watch.held_since = None;
        let mut marked: Vec<(DocumentId, bool)> = Vec::new();
        for (d, deleted) in std::mem::take(&mut self.watch.held) {
            // 回の間に閉じた文書の印は数えない。同じ文書は後の印で数える
            if !self.docs.documents.contains_key(&d) {
                continue;
            }
            marked.retain(|(x, _)| *x != d);
            marked.push((d, deleted));
        }
        match marked.as_slice() {
            [] => {}
            [(doc, deleted)] => self.watch_notify_one(*doc, *deleted),
            many => self.show_watch_message(
                keys::INFO_EXTERNAL_CHANGED_MANY,
                &[("count", &many.len().to_string())],
            ),
        }
    }

    /// 1 つの文書の外部の変更・削除を知らせる (`info.external.changed` / `.deleted`)。
    fn watch_notify_one(&mut self, doc: DocumentId, deleted: bool) {
        let name = self.watch_doc_name(doc);
        let key = if deleted {
            keys::INFO_EXTERNAL_DELETED
        } else {
            keys::INFO_EXTERNAL_CHANGED
        };
        self.show_watch_message(key, &[("name", &name)]);
    }

    fn watch_doc_name(&self, doc: DocumentId) -> String {
        self.docs
            .documents
            .get(&doc)
            .and_then(|d| d.path.as_ref())
            .and_then(FsPath::file_name)
            .unwrap_or_default()
    }

    /// 監視できない理由を、種類ごとにプロセスで 1 回知らせる (WATCH-2、01 FR-117 AC-9)。
    fn watch_note_unavailable(&mut self, err: &WatchError, doc: DocumentId) {
        let (kind, reason_key, fs) = reason_kind(err);
        if !self.watch.noted.insert(kind) {
            return;
        }
        let reason = self.catalog().render(reason_key, &[("fs", &fs)], &|_| None);
        let path = self
            .docs
            .documents
            .get(&doc)
            .and_then(|d| d.path.as_ref())
            .map(FsPath::display)
            .unwrap_or_default();
        self.show_watch_message(
            keys::INFO_WATCH_UNAVAILABLE,
            &[("path", &path), ("reason", &reason)],
        );
    }

    /// 監視の照合の結果 (WATCH-5〜WATCH-8)。
    pub(crate) fn finish_watch_check(
        &mut self,
        doc: DocumentId,
        round: Option<u64>,
        switch: bool,
        at: Tick,
        r: JobResult,
    ) {
        // 監視を止めた後に届いた結果は使わない (回も捨ててある)
        if !self.watch.enabled {
            return;
        }
        let marked = self.apply_watch_check(doc, switch, at, r);
        if let Some(rd) = round {
            self.watch_round_done(rd, marked.map(|deleted| (doc, deleted)));
        } else if let Some(deleted) = marked {
            self.watch_notify_one(doc, deleted);
        }
        self.mark_dirty_render();
    }

    /// 印を付ける (WATCH-7): 知らせるのは、印の無い文書に付いたとき・変更と削除が入れ替わったとき・切替の照合のときだけ。
    /// 変更の印の stamp が変わっただけ (書き続けるファイル) なら、印の stamp を黙って新しくする。知らせるなら `Some(削除か)`。
    fn watch_mark(&mut self, doc: DocumentId, mark: ExternalPending, switch: bool) -> Option<bool> {
        let d = self.docs.documents.get_mut(&doc)?;
        let before = d.external_pending.replace(mark);
        let deleted = mark == ExternalPending::Deleted;
        let changed_kind = match before {
            None => true,
            Some(ExternalPending::Deleted) => !deleted,
            Some(ExternalPending::Modified(_)) => deleted,
        };
        (changed_kind || switch).then_some(deleted)
    }

    /// 照合の結果を当てる。知らせる印を付けたら `Some(削除か)`。`at` は照合を積んだ時刻。
    fn apply_watch_check(
        &mut self,
        doc: DocumentId,
        switch: bool,
        at: Tick,
        r: JobResult,
    ) -> Option<bool> {
        let d = self.docs.documents.get(&doc)?;
        // 結果が届くまでに、その文書の保存・読込などが始まった: その後で照合し直す (古い `disk_stamp` と比べない)
        if self.watch_busy(doc) {
            let prev = self.watch.deferred.get(&doc).copied().unwrap_or(false);
            self.watch.deferred.insert(doc, prev || switch);
            return None;
        }
        // 照合を積んだ後に知らせが届き、その待ち合わせの照合がまだ来ていない: この (古いかもしれない) 結果は当てずに、
        // 次の照合に任せる。続けては捨てない (書き続けるファイルでも、打ち切りの照合の結果を当てる。WATCH-4 / WATCH-5)
        if !switch
            && self.watch.waits.get(&doc).is_some_and(|w| w.last >= at)
            && self.watch.discarded.insert(doc)
        {
            return None;
        }
        self.watch.discarded.remove(&doc);
        let (disk, seen, pending) = (d.disk_stamp, d.external_seen, d.external_pending);
        match r {
            JobResult::Meta { meta, stamp, .. } => {
                self.watch.retries.remove(&doc);
                if disk.is_some_and(|ds| same_on_disk(&ds, &stamp)) {
                    // 自分の保存・内容の変わらない属性の変更: 何もしない。付けていた印は外す (元の内容に戻された)
                    if pending.is_some()
                        && let Some(d) = self.docs.documents.get_mut(&doc)
                    {
                        d.external_pending = None;
                    }
                    return None;
                }
                // 「無視」を選んだ変更は、もう知らせない (WATCH-7、03 §9.5 の「同じ変更で何度も聞かない」)
                if seen == Some(stamp) {
                    return None;
                }
                match self.watch_reloadable(doc, meta.len) {
                    Reloadable::Yes => {
                        if self.watch_start_reload(doc, stamp) {
                            return None;
                        }
                    }
                    Reloadable::After(when) => {
                        // 間の変更は印を付け、最短の間隔を過ぎたらもう一度照合して読み直す
                        self.watch.due.insert(doc, when);
                    }
                    Reloadable::No => {}
                }
                self.watch_mark(doc, ExternalPending::Modified(stamp), switch)
            }
            JobResult::Failed {
                err: JobError::Fs(e),
                ..
            } if e.kind == FsErrorKind::NotFound => {
                self.watch.retries.remove(&doc);
                // まだ一度も書いていない新規ファイル (「新規として保持」を選んだ文書を含む) は「削除」ではない
                disk?;
                self.watch_mark(doc, ExternalPending::Deleted, switch)
            }
            JobResult::Failed {
                err: JobError::Cancelled,
                ..
            } => None,
            JobResult::Failed { .. } => {
                // 読めない (書き手が排他で開いている間の共有違反など): 0.5 s と 2 s の後にやり直し、それでも読めなければ
                // 何もしない (次の知らせ・切替・フォーカス復帰・保存で確かめる)
                let n = self.watch.retries.get(&doc).copied().unwrap_or(0);
                if let Some(delay) = WATCH_RETRY.get(n) {
                    self.watch.retries.insert(doc, n + 1);
                    self.watch.due.insert(doc, self.now + *delay);
                } else {
                    self.watch.retries.remove(&doc);
                }
                None
            }
            _ => None,
        }
    }

    /// 監視で確認なしに読み直せるか (WATCH-6: 表示中・未編集・`auto_if_unmodified`・上限の内)。
    fn watch_reloadable(&self, doc: DocumentId, len: u64) -> Reloadable {
        let Some(d) = self.docs.documents.get(&doc) else {
            return Reloadable::No;
        };
        let ok = self.config().reload_on_external_change == ExternalChangePolicy::AutoIfUnmodified
            && d.load_state == LoadState::Ready
            && !d.is_dirty()
            && d.disk_stamp.is_some()
            // (c) Redo の履歴が無い (Undo で保存の時点に戻した文書は読み直さない)
            && !d.history().can_redo()
            // (a) 16 MiB 以下・巨大ファイルの閲覧でない
            && len <= WATCH_RELOAD_MAX_BYTES
            && !d.read_only.contains(ReadOnlyReason::LargeFileViewer)
            && !self.panes_showing(doc).is_empty();
        if !ok {
            return Reloadable::No;
        }
        // (b) 同じ文書を監視で読み直してから 5 s
        match self.watch.reloaded_at.get(&doc) {
            Some(at) if self.now < *at + WATCH_RELOAD_MIN_INTERVAL => {
                Reloadable::After(*at + WATCH_RELOAD_MIN_INTERVAL)
            }
            _ => Reloadable::Yes,
        }
    }

    /// 監視からの再読込を始める (03 §9.5 の再読込と同じ経路。Undo で戻せる)。始められなければ `false`。
    fn watch_start_reload(&mut self, doc: DocumentId, stamp: FileStamp) -> bool {
        match self.start_in_place(
            doc,
            OpenKind::WatchReload,
            &[
                ReadOnlyReason::UserRequested,
                ReadOnlyReason::NoWritePermission,
                ReadOnlyReason::Binary,
                ReadOnlyReason::LargeFileViewer,
            ],
            None,
        ) {
            crate::command::CommandResult::Pending(job) => {
                self.watch.reloads.insert(doc, (job, stamp));
                self.watch.reloaded_at.insert(doc, self.now);
                true
            }
            _ => false,
        }
    }

    /// 監視からの再読込の取消し・読み直せない内容のとき: 旧本文のまま `Ready` に戻し、印を付けて知らせる (WATCH-6)。
    fn watch_reload_abandoned(&mut self, doc: DocumentId, deleted: bool) {
        let stamp = self.watch.reloads.remove(&doc).map(|(_, s)| s);
        self.reopen_restore(doc);
        let mark = if deleted {
            Some(ExternalPending::Deleted)
        } else {
            stamp.map(ExternalPending::Modified)
        };
        if let Some(m) = mark
            && let Some(deleted) = self.watch_mark(doc, m, false)
        {
            self.watch_notify_one(doc, deleted);
        }
    }

    /// 貼り付けの応答 (`InputEvent::ClipboardContent`) が待っている貼り付けのものなら (03 §9.5.1 の WATCH-6 の打鍵と
    /// 同じく、監視の読み直しを取り消す)。
    pub(crate) fn watch_cancel_reload_on_paste(&mut self, token: eve_view::ClipboardToken) {
        if self.clipboard_waiting(token) {
            self.watch_cancel_reload_on_input();
        }
    }

    /// 打鍵 (キー・文字・貼り付け): 表示中の文書を監視で読み直している最中なら、読込を取り消して印にする (打った文字を
    /// 読込で捨てない、WATCH-6)。
    pub(crate) fn watch_cancel_reload_on_input(&mut self) {
        let Some(doc) = self.active_document() else {
            return;
        };
        let Some((job, _)) = self.watch.reloads.get(&doc).copied() else {
            return;
        };
        if self.io.pending.remove(&job).is_some() {
            self.jobs.cancel(job);
        }
        self.watch_reload_abandoned(doc, false);
    }

    /// 監視からの再読込の結果 (`OpenKind::WatchReload`)。読み直せない内容 (開く前の確認が要る)・失敗は印だけにする。
    pub(crate) fn finish_watch_reload(
        &mut self,
        doc: DocumentId,
        view: eve_view::ViewId,
        path: FsPath,
        r: JobResult,
    ) {
        match r {
            // 読込のときにも大きさの上限を確かめる (照合の後に大きくなった。WATCH-6 の (a))
            JobResult::Complete { meta, .. } if meta.len > WATCH_RELOAD_MAX_BYTES => {
                self.watch_reload_abandoned(doc, false);
            }
            JobResult::Complete {
                text,
                format,
                preserved,
                meta,
                stamp,
                detected,
                changed_during_load,
                ..
            } => {
                self.watch.reloads.remove(&doc);
                self.apply_reload(
                    doc, view, &path, text, format, preserved, stamp, detected, false, true,
                );
                self.watch_note_fs(doc, meta.fs_kind);
                self.watch_sync_doc(doc);
                // 読んでいる間に変わった: 読んだ内容はディスクと違うかもしれないので印を残し、待ち合わせの後に照合し直す
                if changed_during_load {
                    if let Some(deleted) =
                        self.watch_mark(doc, ExternalPending::Modified(stamp), false)
                    {
                        self.watch_notify_one(doc, deleted);
                    }
                    self.watch.due.insert(doc, self.now + WATCH_SETTLE);
                }
            }
            JobResult::Failed {
                err: JobError::Fs(e),
                ..
            } if e.kind == FsErrorKind::NotFound => self.watch_reload_abandoned(doc, true),
            JobResult::NewFile { .. } => self.watch_reload_abandoned(doc, true),
            // 開く前の確認 (巨大ファイル・不正バイトの比率) が要る内容・読めない・取消し: モーダルの確認を出さずに印にする
            _ => self.watch_reload_abandoned(doc, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_short_name_form;

    #[test]
    fn 短い名前は_8_3_の形だけ() {
        for name in ["LONGNA~1.TXT", "PROGRA~1", "AB~12.C", "LONGN~10.TX"] {
            assert!(is_short_name_form(name), "{name}");
        }
        for name in [
            "~$report.docx",
            "a.txt~",
            "~WRL0001.tmp",
            "LONGNAM~1.TXT",
            "LONGNA~10.TXT",
            "LONGNA~1.TEXT",
            "LONGNA~X.TXT",
            "LONGNA~.TXT",
            "a~1~2.txt",
            "plain.txt",
        ] {
            assert!(!is_short_name_form(name), "{name}");
        }
    }
}
