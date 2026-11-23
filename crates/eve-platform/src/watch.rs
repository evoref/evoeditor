//! ファイル監視の本番の実装 `NotifyWatcher` (docs/02 §4.4 の `FileWatcher`、05 §12。v0.5 の M33 の段 7)。
//!
//! - **登録の呼び出し** (05 §12.2 の 9): `watch_dir` / `unwatch` は主スレッドで要求を作るだけで、OS を呼ぶ作業 (監視できる
//!   FS の判定と notify の `watch` / `unwatch`) は、Host が渡す io の口 (`HostTasks::spawn_io_task` を包んだもの) で `io`
//!   のレーンに積む。notify の watcher (`RecommendedWatcher`) はプロセスに 1 つで、最初の登録の作業で作り、`io` の作業だけが
//!   触る (主スレッドと共有しない)。
//! - **知らせ** (05 §12.2 の 2・6・10): notify のコールバックは notify のスレッドで動き、`EditorState` に触れず、選り分けた
//!   知らせを Host が渡す知らせの口 (受信口へ `HostMsg::Fs` を送るもの) へ送るだけにする。種類 (作成・変更・削除) は渡さない。
//! - **後片付け**: `NotifyWatcher` を手放すと、watcher を手放す作業を `io` に積む。watcher を手放すと notify のスレッドが
//!   止まり、監視のハンドル (Windows のディレクトリのハンドル・inotify の fd) が閉じる (notify の `Drop` は待たない)。`io` の
//!   キューが既に閉じていれば、最後の参照を手放した所で同じことが起きる。
//!
//! 監視しない OS (Linux と Windows 以外) では、どの登録も `Unsupported` で失敗させる (notify の backend の無い OS で
//! `RecommendedWatcher` がポーリングにならないよう、その OS では notify を使わない。05 §12.2 の 13)。
//!
//! spec: 02 §4.4, 05 §12.1, 05 §12.2

// 選り分け (`route`) と FS の判定は Windows と Linux だけで使う (ほかの OS は登録を `Unsupported` で失敗させるだけ)
#[cfg(any(windows, target_os = "linux", test))]
use std::ffi::OsString;
#[cfg(any(windows, target_os = "linux", test))]
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(any(windows, target_os = "linux", test))]
use eve_core::WatchError;
use eve_core::{FileWatcher, FsEvent, FsPath, WatchId};

/// io のレーンに作業を積む口 (Host の `HostTasks::spawn_io_task` を包んだもの。02 §4.4)。主スレッドから呼ぶ。
pub type IoSpawn = Box<dyn Fn(Box<dyn FnOnce() + Send>)>;

/// 知らせの口 (受信口へ `HostMsg::Fs` を送るもの。02 §4.4)。notify のスレッドと `io` の作業から呼ぶ。
pub type FsSink = Arc<dyn Fn(FsEvent) + Send + Sync>;

/// ファイル監視の本番の実装 (02 §4.4。notify を使う。05 §12.2)。主スレッドで使い、`Send` でない。
pub struct NotifyWatcher {
    next: u64,
    io: IoSpawn,
    shared: Arc<backend::Shared>,
}

impl NotifyWatcher {
    /// `io` は io のレーンに作業を積む口、`sink` は知らせの口。watcher はまだ作らない (最初の登録の作業で作る)。
    pub fn new(io: IoSpawn, sink: FsSink) -> NotifyWatcher {
        NotifyWatcher {
            next: 1,
            io,
            shared: Arc::new(backend::Shared::new(sink)),
        }
    }
}

impl std::fmt::Debug for NotifyWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotifyWatcher")
            .field("next", &self.next)
            .finish_non_exhaustive()
    }
}

impl FileWatcher for NotifyWatcher {
    fn watch_dir(&mut self, dir: &FsPath) -> WatchId {
        let id = WatchId(self.next);
        self.next += 1;
        let shared = Arc::clone(&self.shared);
        let dir = dir.as_path().to_path_buf();
        (self.io)(Box::new(move || shared.register(id, dir)));
        id
    }

    fn unwatch(&mut self, id: WatchId) {
        let shared = Arc::clone(&self.shared);
        (self.io)(Box::new(move || shared.unregister(id)));
    }
}

impl Drop for NotifyWatcher {
    fn drop(&mut self) {
        let shared = Arc::clone(&self.shared);
        (self.io)(Box::new(move || shared.shutdown()));
    }
}

/// `io::Error` の種類から登録の失敗の理由 (`statfs`・ディレクトリを開くとき)。
#[cfg(any(windows, target_os = "linux"))]
fn io_watch_error(e: &std::io::Error) -> WatchError {
    match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => WatchError::NotFound,
        std::io::ErrorKind::PermissionDenied => WatchError::Denied,
        _ => WatchError::Other,
    }
}

/// 監視の知らせの元 (notify の 1 つのイベントを選り分けた結果)。純粋な値で、試験で表にできる。
#[cfg(any(windows, target_os = "linux", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
enum Route {
    /// 監視しているディレクトリ `dir` の直下の `name` に何か起きた。
    Changed { dir: PathBuf, name: OsString },
    /// `dir` の取りこぼしの恐れ。
    Rescan { dir: PathBuf },
    /// すべての監視の取りこぼしの恐れ (どのディレクトリか分からないあふれ・誤り)。
    RescanAll,
    /// `dir` の監視が失効した。
    Lost { dir: PathBuf },
}

/// 監視している側から見たイベントの種類 (notify の `EventKind` を、選り分けに要る分だけにしたもの)。
#[cfg(any(windows, target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// 読むだけの開閉など (捨てる)。
    Access,
    /// 書いた後の閉じ (`Access(Close(Write))`。変化として送る)。
    CloseWrite,
    /// 作成。
    Create,
    /// 削除。
    Remove,
    /// 改名。
    Rename,
    /// 内容・属性・種類の分からない変更 (`Modify` のうち改名でないもの)。
    Modify,
    /// 種類の分からないもの (`Any`・`Other`)。
    Unknown,
}

/// 1 つのイベントを選り分ける (05 §12.2 の 10)。`is_dir` は監視しているディレクトリそのものか。
///
/// - 取りこぼしの恐れ (`rescan`): パスのあるものはそのディレクトリ、パスの無いものはすべての監視。
/// - 読むだけの開閉は捨てる。書いた後の閉じは変化として送る。
/// - パスが監視しているディレクトリそのもの: 削除・改名は `Lost`。変更 (`Modify`。属性・更新時刻) と開閉は捨てる
///   (中の項目は変わらない。Windows で親と子の両方を監視すると、親の監視が子のディレクトリの更新時刻の変化を子の
///   パスで届けるので、ここで捨てないと子の中の保存のたびに子の全文書を照合する)。作成も捨てる (親の監視が届ける、
///   子を頼む前の作成の知らせが、子を表に載せた後に届くと、張ったばかりの監視を失効にする。消して作り直したときは、
///   消したときの削除で `Lost` が出ている)。種類の分からないものは、中で何が起きたか分からないので `Rescan`。
/// - パスの親が監視しているディレクトリ: 名前を `Changed` で送る (改名は元と先の両方)。どちらでもないパスは捨てる。
#[cfg(any(windows, target_os = "linux", test))]
fn route(
    kind: Kind,
    rescan: bool,
    paths: &[PathBuf],
    is_dir: &dyn Fn(&Path) -> bool,
) -> Vec<Route> {
    let mut out: Vec<Route> = Vec::new();
    let mut push = |r: Route| {
        if !out.contains(&r) {
            out.push(r);
        }
    };
    if rescan {
        if paths.is_empty() {
            push(Route::RescanAll);
        }
        for p in paths {
            if is_dir(p) {
                push(Route::Rescan { dir: p.clone() });
            } else if let Some(parent) = p.parent().filter(|d| is_dir(d)) {
                push(Route::Rescan {
                    dir: parent.to_path_buf(),
                });
            }
        }
        return out;
    }
    if kind == Kind::Access {
        return out;
    }
    for p in paths {
        if is_dir(p) {
            match kind {
                Kind::Remove | Kind::Rename => push(Route::Lost { dir: p.clone() }),
                Kind::Create | Kind::Modify | Kind::Access | Kind::CloseWrite => {}
                Kind::Unknown => push(Route::Rescan { dir: p.clone() }),
            }
            continue;
        }
        if let (Some(parent), Some(name)) = (p.parent(), p.file_name())
            && is_dir(parent)
        {
            push(Route::Changed {
                dir: parent.to_path_buf(),
                name: name.to_os_string(),
            });
        }
    }
    out
}

/// 誤りの知らせ (`Err`) を選り分ける。パスが監視しているディレクトリなら `Lost` (notify 9.0 の Windows は読取の失敗の後に
/// その監視を外す)、パスの無いものはすべての監視の `Rescan` (inotify の読取の失敗。監視は続く)。
#[cfg(any(windows, target_os = "linux", test))]
fn route_error(paths: &[PathBuf], is_dir: &dyn Fn(&Path) -> bool) -> Vec<Route> {
    let dirs: Vec<Route> = paths
        .iter()
        .filter(|p| is_dir(p))
        .map(|p| Route::Lost { dir: p.clone() })
        .collect();
    if dirs.is_empty() {
        vec![Route::RescanAll]
    } else {
        dirs
    }
}

/// Linux の監視できる FS の判定 (05 §12.2 の 4。`statfs` の `f_type` の許可リスト)。
#[cfg(any(target_os = "linux", test))]
mod linux_fs {
    /// 監視する FS の `f_type` (`linux/magic.h` の `EXT4_SUPER_MAGIC` (ext2 / ext3 も同じ値)・`XFS_SUPER_MAGIC`・
    /// `BTRFS_SUPER_MAGIC`・`TMPFS_MAGIC`・`F2FS_SUPER_MAGIC`・`OVERLAYFS_SUPER_MAGIC`。2026-11-23 に照らした)。
    /// overlayfs は M33 の段 7 で試して足した (マウントを通した書込は、下の層からの copy-up を含めて知らせが届く。
    /// マウントの外から層のディレクトリを直に書いた変更は届かないが、overlayfs はそれを未定義の操作としている)。
    pub(super) const WATCHABLE_FS: [(u32, &str); 6] = [
        (0xEF53, "ext4"),
        (0x5846_5342, "xfs"),
        (0x9123_683E, "btrfs"),
        (0x0102_1994, "tmpfs"),
        (0xF2F5_2010, "f2fs"),
        (0x794C_7630, "overlayfs"),
    ];

    /// 監視しない FS のうち、知らせに名前を出すもの (`linux/magic.h` の `V9FS_MAGIC`・`FUSE_SUPER_MAGIC`・
    /// `NFS_SUPER_MAGIC`・`SMB_SUPER_MAGIC`・`CIFS_SUPER_MAGIC`・`SMB2_SUPER_MAGIC`・`BCACHEFS_SUPER_MAGIC` と、zfs (`OpenZFS` の
    /// `ZFS_SUPER_MAGIC`)。05 §12.2 の 4 の例の FS)。ほかは `f_type` の 16 進で出す。
    const NAMED_FS: [(u32, &str); 8] = [
        (0x0102_1997, "9p"),
        (0x6573_5546, "fuse"),
        (0x6969, "nfs"),
        (0x517B, "smb"),
        (0xFF53_4D42, "cifs"),
        (0xFE53_4D42, "smb2"),
        (0xCA45_1A4E, "bcachefs"),
        (0x2FC1_2FC1, "zfs"),
    ];

    /// 監視できる FS なら `Ok`、できなければ FS の名前。
    pub(super) fn verdict(f_type: u32) -> Result<(), String> {
        if WATCHABLE_FS.iter().any(|(t, _)| *t == f_type) {
            return Ok(());
        }
        Err(NAMED_FS
            .iter()
            .find(|(t, _)| *t == f_type)
            .map_or_else(|| format!("{f_type:#x}"), |(_, n)| (*n).to_owned()))
    }
}

/// 監視できる FS か (05 §12.2 の 4)。`io` の作業の中で呼ぶ (I/O をする)。
#[cfg(target_os = "linux")]
fn check_fs(dir: &Path) -> Result<(), WatchError> {
    let f_type = crate::sys::statfs_type(dir).map_err(|e| io_watch_error(&e))?;
    linux_fs::verdict(f_type).map_err(|fs| WatchError::Unsupported { fs })
}

/// 監視できる FS か (05 §12.2 の 4。Windows は許可リスト): NTFS か `ReFS` で、識別で開ける (ユーザーモードの FS でない。
/// 03 §12.7.6 の段 5 (1) と同じ関数) ボリュームの、遠くでない (ドライブの種類・UNC・ハンドルのリモートの手順の情報・
/// 最終のパスが `\Device\Mup\` の下) ディレクトリ。`io` の作業の中で呼ぶ (I/O をする)。
#[cfg(windows)]
fn check_fs(dir: &Path) -> Result<(), WatchError> {
    use std::os::windows::fs::OpenOptionsExt;
    /// 属性だけを読む権利 (`FILE_READ_ATTRIBUTES`)。中身は読まない。
    const READ_ATTRIBUTES: u32 = 0x80;
    /// ディレクトリを開くための旗 (`FILE_FLAG_BACKUP_SEMANTICS`)。
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    /// 読み・書き・削除のどれも妨げない共有 (`FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`)。
    const SHARE_ALL: u32 = 0x7;
    let network = || WatchError::Unsupported {
        fs: "network".to_owned(),
    };
    // 開く前にパスの形とドライブの種類で見る (応答しない共有のディレクトリを開いて `io` を止めない)
    if crate::sys::is_remote(dir) {
        return Err(network());
    }
    let f = std::fs::OpenOptions::new()
        .access_mode(READ_ATTRIBUTES)
        .share_mode(SHARE_ALL)
        .custom_flags(BACKUP_SEMANTICS)
        .open(dir)
        .map_err(|e| io_watch_error(&e))?;
    if !f.metadata().is_ok_and(|m| m.is_dir()) {
        return Err(WatchError::NotFound);
    }
    // シンボリックリンク・ジャンクションの先が共有フォルダのものは、開いたハンドルで見分ける
    let remote = crate::sys::is_remote_handle(&f)
        || crate::sys::owner::final_path(&f).is_ok_and(|p| crate::read_checked::is_mup(&p));
    if remote {
        return Err(network());
    }
    let (fs, _) = crate::sys::volume_info(&f).ok_or(WatchError::Other)?;
    if crate::sys::trusted_id_fs(&f) {
        Ok(())
    } else {
        Err(WatchError::Unsupported {
            fs: fs.to_ascii_lowercase(),
        })
    }
}

#[cfg(any(windows, target_os = "linux"))]
mod backend {
    //! notify を使う部分 (Windows と Linux)。
    //!
    //! ロックの順は `backend` → `dirs` → `last` で、逆向きには取らない。`dirs` を持ったまま OS の `watch` / `unwatch` を
    //! 呼ばない (notify はそこで監視のスレッドの応答を待ち、監視のスレッドはコールバックで `dirs` を取る)。どのロックも、
    //! 持ち主のパニックで毒になった後も中身をそのまま使う (表は 1 回の操作の途中で壊れる形を持たない)。

    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::time::{Duration, Instant};

    use eve_core::{FsEvent, FsEventKind, WatchError, WatchId};
    use notify::event::{AccessKind, AccessMode, EventKind, ModifyKind};
    use notify::{EventKindMask, RecommendedWatcher, RecursiveMode, Watcher};

    use super::{FsSink, Kind, Route};

    /// 続けて届いた同じ (監視, 名前) の `Changed` を 1 つにまとめる間隔 (05 §12.2 の 2)。core の待ち合わせ
    /// (`WATCH_SETTLE` = 100 ms、03 §9.5.1 の WATCH-4) より短くする。根拠は WATCH-4 の「届けた知らせ e ごとに、e の
    /// `WATCH_SETTLE` 後以降に照合が必ず 1 回ある」(1 s の打ち切りの照合は待ち合わせの期限を消さず、期限が後ならもう 1 回
    /// 照合する): 捨てた知らせの元の書込は、届けた知らせから 50 ms の内に済んでいるので、その照合が取り込む。
    /// 同じディレクトリに要求が 2 つ以上あると (失効の後の張り直しの間)、交互に送るのでまとまらない (まれなので構わない)。
    const COALESCE: Duration = Duration::from_millis(50);

    /// ロックを取る (毒になっていても中身を使う)。
    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 監視の表 (`io` の作業が書き、notify のコールバックが読む)。
    #[derive(Default)]
    struct Dirs {
        by_path: HashMap<PathBuf, Dir>,
        by_id: HashMap<WatchId, PathBuf>,
    }

    /// 監視しているディレクトリ 1 つ。
    struct Dir {
        /// このディレクトリの要求 (失効の後に張り直す間は 2 つになり得る)。
        ids: Vec<WatchId>,
        /// OS の監視が無い・外れた恐れがある (新しい項目と失効したもの)。次の登録で OS の `watch` をし直す。
        lost: bool,
    }

    /// notify のコールバックと `io` の作業が共有するもの。watcher は持たない (watcher → コールバック → これ、の向きだけ
    /// にして、参照の輪を作らない)。
    struct Routing {
        sink: FsSink,
        dirs: Mutex<Dirs>,
        /// 最後に送った `Changed` (監視, 名前, 時刻)。コールバックだけが触る。
        last: Mutex<Option<(WatchId, OsString, Instant)>>,
    }

    /// watcher の状態。
    enum Backend {
        /// まだ作っていない (最初の登録で作る)。
        NotCreated,
        /// 作った。
        Ready(RecommendedWatcher),
        /// 作れなかった (以後の登録はすべて `Other` で失敗させる、05 §12.2 の 5)。
        Failed,
        /// 後片付けを終えた (以後の登録は `Other` で失敗させる)。
        Closed,
    }

    /// `NotifyWatcher` と `io` の作業が共有するもの。`io` の作業だけが `backend` を触る。
    pub(super) struct Shared {
        routing: Arc<Routing>,
        backend: Mutex<Backend>,
    }

    impl Shared {
        pub(super) fn new(sink: FsSink) -> Shared {
            Shared {
                routing: Arc::new(Routing {
                    sink,
                    dirs: Mutex::default(),
                    last: Mutex::new(None),
                }),
                backend: Mutex::new(Backend::NotCreated),
            }
        }

        fn send(&self, watch: WatchId, kind: FsEventKind) {
            (self.routing.sink)(FsEvent { watch, kind });
        }

        /// 登録 (`io` の作業)。結果を `Started` / `Failed` で 1 回だけ送る (途中のパニックも `Failed { Other }` にする)。
        pub(super) fn register(&self, id: WatchId, dir: PathBuf) {
            let done = catch_unwind(AssertUnwindSafe(|| self.register_inner(id, &dir)));
            if done.is_err() {
                // 表に載ったままにしない (毒になったロックも中身を使う)。`watch` の失敗と同じく、残る要求 (失効した古い
                // 要求) があれば張り直しが要るままにしてから外す
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    if let Some(d) = lock(&self.routing.dirs).by_path.get_mut(&dir) {
                        d.lost = true;
                    }
                    self.forget(id)
                }));
                self.send(
                    id,
                    FsEventKind::Failed {
                        err: WatchError::Other,
                    },
                );
            }
        }

        fn register_inner(&self, id: WatchId, dir: &Path) {
            let fail = |err: WatchError| self.send(id, FsEventKind::Failed { err });
            let mut backend = lock(&self.backend);
            if matches!(*backend, Backend::Closed) {
                drop(backend);
                return fail(WatchError::Other);
            }
            if let Err(err) = super::check_fs(dir) {
                drop(backend);
                return fail(err);
            }
            if matches!(*backend, Backend::NotCreated) {
                *backend = match create(Arc::clone(&self.routing)) {
                    Some(w) => Backend::Ready(w),
                    None => Backend::Failed,
                };
            }
            let Backend::Ready(w) = &mut *backend else {
                drop(backend);
                return fail(WatchError::Other);
            };
            // 表に先に載せる (OS の監視が始まった直後の知らせを捨てない)。OS の `watch` が要るなら、その前に `lost` を
            // 外す (`watch` の後に外すと、その間に届いた失効を上書きする)。`watch` が失敗したら表から外す
            let need_os = {
                let mut dirs = lock(&self.routing.dirs);
                dirs.by_id.insert(id, dir.to_path_buf());
                let entry = dirs.by_path.entry(dir.to_path_buf()).or_insert(Dir {
                    ids: Vec::new(),
                    lost: true,
                });
                entry.ids.push(id);
                std::mem::replace(&mut entry.lost, false)
            };
            let result = if need_os {
                // 失効した監視が OS に残っていれば外してから張り直す (外れていれば誤りになるが構わない)
                let _ = w.unwatch(dir);
                w.watch(dir, RecursiveMode::NonRecursive)
            } else {
                Ok(())
            };
            drop(backend);
            match result {
                Ok(()) => {
                    // 表のロックの中で送る: 失効の知らせ (`Lost`。印と送るのを同じロックの中で行う) と前後が入れ替わらない。
                    // `watch` の後に失効していれば、`Started` の後にもう一度 `Lost` を送る (最後の状態を失効にする)
                    let dirs = lock(&self.routing.dirs);
                    self.send(id, FsEventKind::Started);
                    if dirs.by_path.get(dir).is_some_and(|d| d.lost) {
                        self.send(id, FsEventKind::Lost);
                    }
                }
                Err(e) => {
                    if let Some(d) = lock(&self.routing.dirs).by_path.get_mut(dir) {
                        // 残る要求 (失効した古い要求) があれば、張り直しが要るままにする
                        d.lost = true;
                    }
                    self.forget(id);
                    fail(watch_error(&e));
                }
            }
        }

        /// 表から外す。そのディレクトリの最後の要求なら、外したディレクトリを返す。
        fn forget(&self, id: WatchId) -> Option<PathBuf> {
            let mut dirs = lock(&self.routing.dirs);
            let dir = dirs.by_id.remove(&id)?;
            let d = dirs.by_path.get_mut(&dir)?;
            d.ids.retain(|i| *i != id);
            if d.ids.is_empty() {
                dirs.by_path.remove(&dir);
                Some(dir)
            } else {
                None
            }
        }

        /// 解除 (`io` の作業)。そのディレクトリの最後の要求なら OS の監視を外す。
        pub(super) fn unregister(&self, id: WatchId) {
            let mut backend = lock(&self.backend);
            if let Some(dir) = self.forget(id)
                && let Backend::Ready(w) = &mut *backend
            {
                // 失効して OS が既に外していれば誤りになるが構わない
                let _ = catch_unwind(AssertUnwindSafe(|| w.unwatch(&dir)));
            }
        }

        /// 後片付け (`io` の作業): watcher を手放し (notify のスレッドが止まり、監視のハンドルが閉じる)、表を空にする。
        pub(super) fn shutdown(&self) {
            let old = std::mem::replace(&mut *lock(&self.backend), Backend::Closed);
            drop_backend(old);
            *lock(&self.routing.dirs) = Dirs::default();
        }
    }

    impl Drop for Shared {
        fn drop(&mut self) {
            // `io` のキューが閉じていて後片付けの作業が走らなかったとき
            let b = self
                .backend
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner);
            drop_backend(std::mem::replace(b, Backend::Closed));
        }
    }

    /// watcher を手放す。notify の inotify の `Drop` は、監視のスレッドが既に終わっていると panic するので、ここで止める
    /// (後片付けの失敗で Host の作業やプロセスの終了を止めない)。
    fn drop_backend(b: Backend) {
        if let Backend::Ready(w) = b {
            let _ = catch_unwind(AssertUnwindSafe(move || drop(w)));
        }
    }

    /// watcher を作る。読むだけの開く・閉じるは購読しない (Linux の `IN_OPEN`・`IN_CLOSE_NOWRITE` で監視のスレッドを
    /// 起こさない。書いた後の閉じ `IN_CLOSE_WRITE` は購読する)。作れなければ `None`。
    fn create(routing: Arc<Routing>) -> Option<RecommendedWatcher> {
        let config = notify::Config::default()
            .with_event_kinds(EventKindMask::CORE | EventKindMask::ACCESS_CLOSE);
        RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                // コールバックのパニックで notify のスレッドを止めない (その知らせだけを捨てる)
                let _ = catch_unwind(AssertUnwindSafe(|| routing.handle(res)));
            },
            config,
        )
        .ok()
    }

    /// notify の登録の誤りの理由 (05 §12.2 の 5)。
    fn watch_error(e: &notify::Error) -> WatchError {
        match &e.kind {
            notify::ErrorKind::MaxFilesWatch => WatchError::Limit,
            notify::ErrorKind::PathNotFound => WatchError::NotFound,
            notify::ErrorKind::Io(io) => match io.raw_os_error() {
                // inotify の監視の数の上限 (`ENOSPC`)
                #[cfg(target_os = "linux")]
                Some(libc::ENOSPC) => WatchError::Limit,
                _ => super::io_watch_error(io),
            },
            _ => WatchError::Other,
        }
    }

    /// notify の `EventKind` を選り分けの種類にする。
    fn kind_of(k: &EventKind) -> Kind {
        match k {
            EventKind::Access(AccessKind::Close(AccessMode::Write)) => Kind::CloseWrite,
            EventKind::Access(_) => Kind::Access,
            EventKind::Create(_) => Kind::Create,
            EventKind::Remove(_) => Kind::Remove,
            EventKind::Modify(ModifyKind::Name(_)) => Kind::Rename,
            EventKind::Modify(_) => Kind::Modify,
            _ => Kind::Unknown,
        }
    }

    impl Routing {
        /// notify のコールバック (notify のスレッド)。選り分けて知らせの口へ送るだけにする。
        fn handle(&self, res: notify::Result<notify::Event>) {
            let routes = {
                let dirs = lock(&self.dirs);
                let is_dir = |p: &Path| dirs.by_path.contains_key(p);
                match &res {
                    Ok(ev) => super::route(kind_of(&ev.kind), ev.need_rescan(), &ev.paths, &is_dir),
                    Err(e) => super::route_error(&e.paths, &is_dir),
                }
            };
            let now = Instant::now();
            for r in routes {
                self.deliver(r, now);
            }
        }

        fn ids_of(&self, dir: &Path) -> Vec<WatchId> {
            lock(&self.dirs)
                .by_path
                .get(dir)
                .map(|d| d.ids.clone())
                .unwrap_or_default()
        }

        /// 知らせを送る。`now` は知らせを受けた時刻 (まとめの判定に使う。試験は時刻を渡す)。
        fn deliver(&self, r: Route, now: Instant) {
            let (ids, kind) = match r {
                Route::Changed { dir, name } => {
                    for id in self.ids_of(&dir) {
                        let mut last = lock(&self.last);
                        let dup = last.as_ref().is_some_and(|(w, n, at)| {
                            *w == id && *n == name && now.saturating_duration_since(*at) < COALESCE
                        });
                        if dup {
                            continue;
                        }
                        *last = Some((id, name.clone(), now));
                        drop(last);
                        (self.sink)(FsEvent {
                            watch: id,
                            kind: FsEventKind::Changed { name: name.clone() },
                        });
                    }
                    return;
                }
                Route::Lost { dir } => {
                    *lock(&self.last) = None;
                    // 失効の印と送るのを表のロックの中で行う (登録の `Started` と前後が入れ替わらない)
                    let mut dirs = lock(&self.dirs);
                    if let Some(d) = dirs.by_path.get_mut(&dir) {
                        d.lost = true;
                        for id in &d.ids {
                            (self.sink)(FsEvent {
                                watch: *id,
                                kind: FsEventKind::Lost,
                            });
                        }
                    }
                    return;
                }
                Route::Rescan { dir } => (self.ids_of(&dir), FsEventKind::Rescan),
                Route::RescanAll => {
                    let mut ids: Vec<WatchId> = lock(&self.dirs).by_id.keys().copied().collect();
                    ids.sort_unstable();
                    (ids, FsEventKind::Rescan)
                }
            };
            *lock(&self.last) = None;
            for id in ids {
                (self.sink)(FsEvent {
                    watch: id,
                    kind: kind.clone(),
                });
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn routing() -> (Routing, Arc<Mutex<Vec<FsEvent>>>, PathBuf) {
            let got: Arc<Mutex<Vec<FsEvent>>> = Arc::default();
            let sink: FsSink = {
                let got = Arc::clone(&got);
                Arc::new(move |ev| got.lock().expect("試験の記録").push(ev))
            };
            let r = Routing {
                sink,
                dirs: Mutex::default(),
                last: Mutex::new(None),
            };
            let w = PathBuf::from("/w");
            {
                let mut d = r.dirs.lock().expect("表");
                d.by_path.insert(
                    w.clone(),
                    Dir {
                        ids: vec![WatchId(1)],
                        lost: false,
                    },
                );
                d.by_id.insert(WatchId(1), w.clone());
            }
            (r, got, w)
        }

        fn c(n: &str) -> FsEventKind {
            FsEventKind::Changed {
                name: OsString::from(n),
            }
        }

        #[test]
        fn 続けて届いた同じ名前はまとめる() {
            let (r, got, w) = routing();
            let ch = |n: &str| Route::Changed {
                dir: w.clone(),
                name: OsString::from(n),
            };
            let t0 = Instant::now();
            let at = |ms: u64| t0 + Duration::from_millis(ms);
            r.deliver(ch("a"), at(0));
            r.deliver(ch("a"), at(49)); // 送ってから 50 ms の内: 捨てる
            r.deliver(ch("a"), at(50)); // 送ってから 50 ms: 送る
            r.deliver(ch("b"), at(51));
            r.deliver(ch("a"), at(52)); // 間に別の名前: 送る
            r.deliver(Route::Rescan { dir: w.clone() }, at(53));
            r.deliver(ch("a"), at(54)); // Rescan の後: 送る
            r.deliver(Route::Lost { dir: w.clone() }, at(55));
            r.deliver(Route::RescanAll, at(56));
            // 監視していないディレクトリは捨てる
            r.deliver(
                Route::Changed {
                    dir: PathBuf::from("/x"),
                    name: OsString::from("a"),
                },
                at(57),
            );
            let kinds: Vec<FsEventKind> = got
                .lock()
                .expect("試験の記録")
                .iter()
                .map(|e| e.kind.clone())
                .collect();
            assert_eq!(
                kinds,
                vec![
                    c("a"),
                    c("a"),
                    c("b"),
                    c("a"),
                    FsEventKind::Rescan,
                    c("a"),
                    FsEventKind::Lost,
                    FsEventKind::Rescan,
                ]
            );
            assert!(r.dirs.lock().expect("表").by_path[&w].lost, "失効の印");
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod backend {
    //! 監視しない OS: どの登録も `Unsupported` で失敗させる (05 §12.2 の 13)。

    use std::path::PathBuf;

    use eve_core::{FsEvent, FsEventKind, WatchError, WatchId};

    use super::FsSink;

    pub(super) struct Shared {
        sink: FsSink,
    }

    impl Shared {
        pub(super) fn new(sink: FsSink) -> Shared {
            Shared { sink }
        }

        pub(super) fn register(&self, id: WatchId, _dir: PathBuf) {
            (self.sink)(FsEvent {
                watch: id,
                kind: FsEventKind::Failed {
                    err: WatchError::Unsupported {
                        fs: "unsupported-os".to_owned(),
                    },
                },
            });
        }

        pub(super) fn unregister(&self, _id: WatchId) {}

        pub(super) fn shutdown(&self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn route_in(kind: Kind, rescan: bool, paths: &[&str]) -> Vec<Route> {
        let paths: Vec<PathBuf> = paths.iter().map(|s| p(s)).collect();
        let is_dir = |d: &Path| d == Path::new("/w") || d == Path::new("/v");
        route(kind, rescan, &paths, &is_dir)
    }

    fn changed(dir: &str, name: &str) -> Route {
        Route::Changed {
            dir: p(dir),
            name: OsString::from(name),
        }
    }

    #[test]
    fn 直下の項目の変化は名前で送る() {
        for k in [
            Kind::Create,
            Kind::Remove,
            Kind::Rename,
            Kind::Modify,
            Kind::CloseWrite,
            Kind::Unknown,
        ] {
            assert_eq!(
                route_in(k, false, &["/w/a.txt"]),
                vec![changed("/w", "a.txt")],
                "{k:?}"
            );
        }
        // 改名は元と先の両方 (別のディレクトリへの移動も、監視していれば両方)
        assert_eq!(
            route_in(Kind::Rename, false, &["/w/a.txt", "/v/b.txt"]),
            vec![changed("/w", "a.txt"), changed("/v", "b.txt")]
        );
        // 同じ名前が 2 回あれば 1 つにする
        assert_eq!(
            route_in(Kind::Rename, false, &["/w/a", "/w/a"]),
            vec![changed("/w", "a")]
        );
    }

    #[test]
    fn 読むだけの開閉と監視の外は捨てる() {
        assert_eq!(
            route_in(Kind::Access, false, &["/w/a.txt"]),
            Vec::<Route>::new()
        );
        assert_eq!(
            route_in(Kind::Modify, false, &["/x/a.txt"]),
            Vec::<Route>::new()
        );
        // 孫 (非再帰なので本来は届かない) も捨てる
        assert_eq!(
            route_in(Kind::Modify, false, &["/w/sub/a.txt"]),
            Vec::<Route>::new()
        );
    }

    #[test]
    fn 監視しているディレクトリそのものの知らせ() {
        assert_eq!(
            route_in(Kind::Remove, false, &["/w"]),
            vec![Route::Lost { dir: p("/w") }]
        );
        assert_eq!(
            route_in(Kind::Rename, false, &["/w"]),
            vec![Route::Lost { dir: p("/w") }]
        );
        // 変更 (Windows の入れ子の監視で、親の監視が子のディレクトリの更新時刻の変化を子のパスで届ける)・開閉・作成
        // (親の監視が届ける、子を頼む前の作成の知らせ) は捨てる
        for k in [Kind::Create, Kind::Modify, Kind::Access, Kind::CloseWrite] {
            assert_eq!(route_in(k, false, &["/w"]), Vec::<Route>::new(), "{k:?}");
        }
        // 入れ子: 監視している /w/sub そのものの変更は、/w にも /w/sub にも何も送らない
        let nested = |d: &Path| d == Path::new("/w") || d == Path::new("/w/sub");
        assert_eq!(
            route(Kind::Modify, false, &[p("/w/sub")], &nested),
            Vec::<Route>::new()
        );
        assert_eq!(
            route_in(Kind::Unknown, false, &["/w"]),
            vec![Route::Rescan { dir: p("/w") }]
        );
    }

    #[test]
    fn 取りこぼしの恐れ() {
        assert_eq!(route_in(Kind::Unknown, true, &[]), vec![Route::RescanAll]);
        assert_eq!(
            route_in(Kind::Unknown, true, &["/w"]),
            vec![Route::Rescan { dir: p("/w") }]
        );
        assert_eq!(
            route_in(Kind::Unknown, true, &["/w/a.txt"]),
            vec![Route::Rescan { dir: p("/w") }]
        );
    }

    #[test]
    fn 誤りの知らせ() {
        let is_dir = |d: &Path| d == Path::new("/w");
        assert_eq!(route_error(&[], &is_dir), vec![Route::RescanAll]);
        assert_eq!(route_error(&[p("/x")], &is_dir), vec![Route::RescanAll]);
        assert_eq!(
            route_error(&[p("/w")], &is_dir),
            vec![Route::Lost { dir: p("/w") }]
        );
    }

    #[test]
    fn linux_の監視できる_fs() {
        for (t, _) in linux_fs::WATCHABLE_FS {
            assert_eq!(linux_fs::verdict(t), Ok(()));
        }
        assert_eq!(linux_fs::verdict(0x0102_1997), Err("9p".to_owned()));
        assert_eq!(linux_fs::verdict(0x2FC1_2FC1), Err("zfs".to_owned()));
        assert_eq!(linux_fs::verdict(0x1234), Err("0x1234".to_owned()));
    }

    /// 許可リストの値を libc の `linux/magic.h` の定数と照らす (05 §12.2 の 4 の「段 7 の PR で照らす」)。
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_の許可リストは_magic_h_の値() {
        let want: Vec<u32> = [
            libc::EXT4_SUPER_MAGIC,
            libc::XFS_SUPER_MAGIC,
            libc::BTRFS_SUPER_MAGIC,
            libc::TMPFS_MAGIC,
            libc::F2FS_SUPER_MAGIC,
            libc::OVERLAYFS_SUPER_MAGIC,
        ]
        .into_iter()
        .map(|m| u32::try_from(m).expect("magic の値は 32 bit に収まる"))
        .collect();
        let have: Vec<u32> = linux_fs::WATCHABLE_FS.iter().map(|(t, _)| *t).collect();
        assert_eq!(have, want);
    }
}
