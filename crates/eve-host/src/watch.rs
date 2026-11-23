//! ファイル監視の本番の実装を組み立てる (02 §4.4 の `FileWatcher`、05 §12.2 の 9。v0.5 の M33 の段 7)。
//!
//! eve-platform は eve-host を知らないので、Host が 2 つの口を渡す: io の口 (`HostTasks::spawn_io_task` を包んだもの。
//! 登録・解除を `io` のレーンで行う) と、知らせの口 (受信口へ `HostMsg::Fs` を送るもの。notify のスレッドと `io` の作業から
//! 呼ぶ)。
//!
//! spec: 02 §4.4, 05 §12.2

use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use eve_core::FileWatcher;
use eve_platform::watch::{FsSink, IoSpawn, NotifyWatcher};

use crate::msg::HostMsg;
use crate::workers::HostTasks;

/// 本番のファイル監視 (notify) を作る。`tasks` は `io` のキューの口 (本番は `ThreadJobSpawner`)、`tx` は Host の受信口への
/// 送信口。watcher (notify) は最初の登録の作業で作る (プロセスに 1 つ、05 §12.2 の 5)。
///
/// `tasks` は弱い参照で持つ (監視が `io` のワーカーの寿命を延ばさない)。`tasks` が既に無いとき・`io` のキューが閉じた
/// 後の要求は、作業を走らせずに捨てる (登録の結果も届かない。02 §4.4 の知らせの順の約束)。
///
/// 手放すと、watcher を手放す作業を `io` に積む (notify のスレッドが止まり、監視のハンドルが閉じる)。`io` に積めなければ、
/// その場で手放す (notify の `Drop` は待たない)。
pub fn file_watcher(tasks: &Rc<dyn HostTasks>, tx: Sender<HostMsg>) -> Box<dyn FileWatcher> {
    let tasks: Weak<dyn HostTasks> = Rc::downgrade(tasks);
    let io: IoSpawn = Box::new(move |task| {
        if let Some(t) = tasks.upgrade() {
            t.spawn_io_task(task);
        }
    });
    let sink: FsSink = Arc::new(move |ev| {
        // 受信口が閉じていれば (終了の後) 捨てる
        let _ = tx.send(HostMsg::Fs(ev));
    });
    Box::new(NotifyWatcher::new(io, sink))
}
