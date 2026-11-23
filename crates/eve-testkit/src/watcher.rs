//! `FakeWatcher`: 偽のファイル監視 (`FileWatcher`、docs/02 §4.4、09 §3.4。v0.5 の M33 の段 7)。
//!
//! `watch_dir` / `unwatch` の要求を記録し、試験が `FsEvent` (`Started`・`Failed`・`Changed`・`Rescan`・`Lost`) を積む。
//! 積んだ知らせは `take_events` で取り出し、試験 (または Harness) が core の受け口へ渡す (本番の受信口の `HostMsg::Fs` の
//! 代わり)。OS を呼ばず、スレッドも作らない。`ManualClock` と組めば、待ち合わせ (03 §9.5.1 の WATCH-4) を決定的に越える・
//! 越えないを作れる。
//!
//! 複製は同じ記録を指す (Port として `Box<dyn FileWatcher>` で渡した後も、試験の手元の複製から要求を見て知らせを積める)。
//!
//! spec: 02 §4.4, 09 §3.4

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::rc::Rc;

use eve_core::{FileWatcher, FsEvent, FsEventKind, FsPath, WatchError, WatchId};

/// 記録した要求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchRequest {
    /// `watch_dir(dir)` が `id` を返した。
    Watch {
        /// 振った識別。
        id: WatchId,
        /// 頼まれたディレクトリ。
        dir: FsPath,
    },
    /// `unwatch(id)`。
    Unwatch(WatchId),
}

/// 登録の要求に自動で返す結果。
#[derive(Clone, Debug, Default)]
enum Reply {
    /// 何も返さない (試験が `push` で返す)。
    #[default]
    Manual,
    /// すぐ `Started` を積む。
    Started,
    /// すぐこの理由の `Failed` を積む。
    Failed(WatchError),
}

#[derive(Default)]
struct Inner {
    next: u64,
    requests: Vec<WatchRequest>,
    active: BTreeMap<WatchId, FsPath>,
    events: VecDeque<FsEvent>,
    reply: Reply,
}

/// 偽のファイル監視。`new` は登録に何も返さない (試験が `push` で `Started` などを積む)。
#[derive(Clone, Default)]
pub struct FakeWatcher {
    inner: Rc<RefCell<Inner>>,
}

impl std::fmt::Debug for FakeWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.borrow();
        f.debug_struct("FakeWatcher")
            .field("requests", &inner.requests)
            .field("events", &inner.events)
            .finish_non_exhaustive()
    }
}

impl FakeWatcher {
    /// 登録に何も返さない偽の監視。
    pub fn new() -> FakeWatcher {
        FakeWatcher::default()
    }

    /// 登録の要求のたびに、すぐ `Started` を積む。
    #[must_use]
    pub fn auto_started(self) -> FakeWatcher {
        self.inner.borrow_mut().reply = Reply::Started;
        self
    }

    /// 登録の要求のたびに、すぐ `Failed { err }` を積む (監視できない FS・上限などの縮退の試験)。
    #[must_use]
    pub fn auto_failed(self, err: WatchError) -> FakeWatcher {
        self.inner.borrow_mut().reply = Reply::Failed(err);
        self
    }

    /// Port として渡す箱 (同じ記録を指す複製)。
    pub fn boxed(&self) -> Box<dyn FileWatcher> {
        Box::new(self.clone())
    }

    /// `Ports.watcher` に渡す共有の参照 (同じ記録を指す複製)。
    pub fn shared(&self) -> Rc<dyn FileWatcher> {
        Rc::new(self.clone())
    }

    /// これまでの要求 (順に)。
    pub fn requests(&self) -> Vec<WatchRequest> {
        self.inner.borrow().requests.clone()
    }

    /// これまでの要求を取り出して空にする。
    pub fn take_requests(&self) -> Vec<WatchRequest> {
        std::mem::take(&mut self.inner.borrow_mut().requests)
    }

    /// 頼まれて、まだ外されていない監視 (識別の順)。
    pub fn active(&self) -> Vec<(WatchId, FsPath)> {
        self.inner
            .borrow()
            .active
            .iter()
            .map(|(id, dir)| (*id, dir.clone()))
            .collect()
    }

    /// `dir` の、まだ外されていない監視の識別 (最も新しいもの)。
    pub fn id_of(&self, dir: &str) -> Option<WatchId> {
        let want = FsPath::from(dir);
        self.inner
            .borrow()
            .active
            .iter()
            .rev()
            .find(|(_, d)| **d == want)
            .map(|(id, _)| *id)
    }

    /// 知らせを積む。
    pub fn push(&self, watch: WatchId, kind: FsEventKind) {
        self.inner
            .borrow_mut()
            .events
            .push_back(FsEvent { watch, kind });
    }

    /// `Changed { name }` を積む。
    pub fn changed(&self, watch: WatchId, name: &str) {
        self.push(
            watch,
            FsEventKind::Changed {
                name: OsString::from(name),
            },
        );
    }

    /// 積んだ知らせを順に取り出す。
    pub fn take_events(&self) -> Vec<FsEvent> {
        self.inner.borrow_mut().events.drain(..).collect()
    }
}

impl FileWatcher for FakeWatcher {
    fn watch_dir(&self, dir: &FsPath) -> WatchId {
        let mut inner = self.inner.borrow_mut();
        inner.next += 1;
        let id = WatchId(inner.next);
        inner.requests.push(WatchRequest::Watch {
            id,
            dir: dir.clone(),
        });
        inner.active.insert(id, dir.clone());
        let kind = match &inner.reply {
            Reply::Manual => None,
            Reply::Started => Some(FsEventKind::Started),
            Reply::Failed(err) => Some(FsEventKind::Failed { err: err.clone() }),
        };
        if let Some(kind) = kind {
            inner.events.push_back(FsEvent { watch: id, kind });
        }
        id
    }

    fn unwatch(&self, id: WatchId) {
        let mut inner = self.inner.borrow_mut();
        inner.requests.push(WatchRequest::Unwatch(id));
        inner.active.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 要求を記録し知らせを積む() {
        let fake = FakeWatcher::new();
        let port = fake.boxed();
        let a = port.watch_dir(&FsPath::from("/w"));
        let b = port.watch_dir(&FsPath::from("/v"));
        assert_ne!(a, b);
        assert_eq!(fake.id_of("/w"), Some(a));
        port.unwatch(a);
        assert_eq!(fake.active(), vec![(b, FsPath::from("/v"))]);
        assert_eq!(
            fake.requests(),
            vec![
                WatchRequest::Watch {
                    id: a,
                    dir: FsPath::from("/w")
                },
                WatchRequest::Watch {
                    id: b,
                    dir: FsPath::from("/v")
                },
                WatchRequest::Unwatch(a),
            ]
        );
        assert_eq!(
            fake.take_events(),
            Vec::<FsEvent>::new(),
            "手動では何も返さない"
        );
        fake.changed(b, "a.txt");
        fake.push(b, FsEventKind::Rescan);
        assert_eq!(
            fake.take_events(),
            vec![
                FsEvent {
                    watch: b,
                    kind: FsEventKind::Changed {
                        name: OsString::from("a.txt")
                    }
                },
                FsEvent {
                    watch: b,
                    kind: FsEventKind::Rescan
                },
            ]
        );
        assert_eq!(fake.take_events(), Vec::<FsEvent>::new());
        assert_eq!(fake.take_requests().len(), 3);
        assert_eq!(fake.requests(), Vec::<WatchRequest>::new());
    }

    #[test]
    fn 登録に自動で返す() {
        let fake = FakeWatcher::new().auto_started();
        let id = fake.boxed().watch_dir(&FsPath::from("/w"));
        assert_eq!(
            fake.take_events(),
            vec![FsEvent {
                watch: id,
                kind: FsEventKind::Started
            }]
        );
        let fake = FakeWatcher::new().auto_failed(WatchError::Unsupported {
            fs: "9p".to_owned(),
        });
        let id = fake.boxed().watch_dir(&FsPath::from("/mnt/c"));
        assert_eq!(
            fake.take_events(),
            vec![FsEvent {
                watch: id,
                kind: FsEventKind::Failed {
                    err: WatchError::Unsupported {
                        fs: "9p".to_owned()
                    }
                }
            }]
        );
    }
}
