//! 緊急退避の共有状態 (docs/02 §11.2.1 の「公開」「ロックの規則」「1 回だけ実行する規則」)。
//!
//! メインスレッドは Editor の状態を変えるたびに、変わった文書の `DocSnapshot` を **スロットに公開** する。公開物は別スレッド
//! (コンソールハンドラ・メイン・パニックの後始末) から **ブロックするロックを取らずに** 読める (`ArcSwap`)。退避は
//! `EmergencyState { Idle, Writing, Done }` の CAS で **プロセスで 1 回だけ** 実行し、負けた経路は `Done` になる (または予算が尽きる)
//! まで待ってから先へ進む。書き手は独立の `catch_unwind` で包み、パニックしても `Done` を立てる。

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use arc_swap::{ArcSwap, ArcSwapOption};
use eve_core::{DocSnapshot, RescueOutcome, RescueReason, RescueWriter};
use eve_view::DocumentId;

const IDLE: u8 = 0;
const WRITING: u8 = 1;
const DONE: u8 = 2;

/// 退避の状態 (02 §11.2.1 の `EmergencyState`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmergencyState {
    Idle,
    Writing,
    Done,
}

pub struct EmergencyCtl {
    /// 文書ごとの公開物 (dirty な文書だけ)。
    slots: ArcSwap<Vec<Arc<DocSnapshot>>>,
    state: AtomicU8,
    /// `Done` の通知。待つ側 (CAS に負けた経路) だけが使う短いロックで、書き手は持たない。
    done: (Mutex<bool>, Condvar),
    writer: OnceLock<Arc<dyn RescueWriter>>,
    outcome: ArcSwapOption<RescueOutcome>,
}

impl EmergencyCtl {
    pub fn new() -> Arc<EmergencyCtl> {
        Arc::new(EmergencyCtl {
            slots: ArcSwap::from_pointee(Vec::new()),
            state: AtomicU8::new(IDLE),
            done: (Mutex::new(false), Condvar::new()),
            writer: OnceLock::new(),
            outcome: ArcSwapOption::empty(),
        })
    }

    /// 書き手を設定する (起動時に 1 回。他のスレッドを作る前に呼ぶ)。
    pub fn set_writer(&self, w: Arc<dyn RescueWriter>) {
        let _ = self.writer.set(w);
    }

    /// 文書のスロットを更新する (`None` で空にする)。ロックを取らない (`rcu`)。
    pub fn publish(&self, doc: DocumentId, snap: Option<DocSnapshot>) {
        let new = snap.map(Arc::new);
        self.slots.rcu(|cur| {
            let mut v: Vec<Arc<DocSnapshot>> =
                cur.iter().filter(|s| s.doc != doc).cloned().collect();
            if let Some(n) = &new {
                v.push(Arc::clone(n));
            }
            v
        });
    }

    /// 公開済みの文書。
    pub fn docs(&self) -> Vec<Arc<DocSnapshot>> {
        self.slots.load().as_ref().clone()
    }

    pub fn state(&self) -> EmergencyState {
        match self.state.load(Ordering::Acquire) {
            IDLE => EmergencyState::Idle,
            WRITING => EmergencyState::Writing,
            _ => EmergencyState::Done,
        }
    }

    /// 退避の結果 (`Done` の後)。
    pub fn outcome(&self) -> Option<Arc<RescueOutcome>> {
        self.outcome.load_full()
    }

    /// 退避を実行する。`Idle → Writing` の CAS に勝った経路だけが書く。負けた経路は `Done` になるまで (最大 `budget`) 待つ。
    /// 結果を返す (待ちが尽きて書込中のままなら `None`)。
    pub fn run(&self, reason: RescueReason, budget: Duration) -> Option<Arc<RescueOutcome>> {
        let won = self
            .state
            .compare_exchange(IDLE, WRITING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        if won {
            let docs = self.docs();
            let outcome = match self.writer.get() {
                Some(w) => catch_unwind(AssertUnwindSafe(|| w.write(&docs, reason, budget)))
                    .unwrap_or_else(|_| RescueOutcome {
                        total: docs.len(),
                        error: Some("退避の書き手が異常終了しました".to_owned()),
                        ..RescueOutcome::default()
                    }),
                None => RescueOutcome {
                    total: docs.len(),
                    complete: docs.is_empty(),
                    error: (!docs.is_empty()).then(|| "退避の書き手がありません".to_owned()),
                    ..RescueOutcome::default()
                },
            };
            self.outcome.store(Some(Arc::new(outcome)));
            self.state.store(DONE, Ordering::Release);
            let (m, cv) = &self.done;
            if let Ok(mut g) = m.lock() {
                *g = true;
            }
            cv.notify_all();
            return self.outcome();
        }
        // 負けた: Done まで待つ (予算の内)
        self.wait_done(budget);
        self.outcome()
    }

    /// 別の経路が退避を書いている最中なら、終わるまで (最大 `budget`) 待つ。書いていなければ何もしない。
    /// プロセスを終える前に呼ぶ (入力の終端などで先にメインが終わっても、受信スレッドの書込を打ち切らない)。
    pub fn wait_if_writing(&self, budget: Duration) {
        if self.state.load(Ordering::Acquire) == WRITING {
            self.wait_done(budget);
        }
    }

    fn wait_done(&self, budget: Duration) {
        let (m, cv) = &self.done;
        if let Ok(g) = m.lock() {
            let _ = cv.wait_timeout_while(g, budget, |done| !*done);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    use super::*;

    struct Counting {
        calls: AtomicUsize,
        delay: Duration,
        panic: bool,
    }

    impl RescueWriter for Counting {
        fn write(
            &self,
            docs: &[Arc<DocSnapshot>],
            _reason: RescueReason,
            _budget: Duration,
        ) -> RescueOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.delay);
            assert!(!self.panic, "書き手のパニックの再現");
            RescueOutcome {
                total: docs.len(),
                written: docs.len(),
                complete: true,
                ..RescueOutcome::default()
            }
        }
    }

    fn ctl(delay_ms: u64, panic: bool) -> (Arc<EmergencyCtl>, Arc<Counting>) {
        let c = EmergencyCtl::new();
        let w = Arc::new(Counting {
            calls: AtomicUsize::new(0),
            delay: Duration::from_millis(delay_ms),
            panic,
        });
        c.set_writer(w.clone());
        (c, w)
    }

    #[test]
    fn 同時に呼んでも書くのは_1_回で_負けた経路は結果を待つ() {
        let (c, w) = ctl(150, false);
        let mut hs = Vec::new();
        for _ in 0..4 {
            let c = c.clone();
            hs.push(std::thread::spawn(move || {
                c.run(RescueReason::Signal, Duration::from_secs(5))
            }));
        }
        for h in hs {
            let o = h.join().unwrap().expect("全経路が結果を受け取る");
            assert!(o.complete);
        }
        assert_eq!(w.calls.load(Ordering::SeqCst), 1);
        assert_eq!(c.state(), EmergencyState::Done);
        // 2 回目以降は書かない
        assert!(c.run(RescueReason::Panic, Duration::from_secs(1)).is_some());
        assert_eq!(w.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn 書き手がパニックしても_done_になり待つ側が進める() {
        let (c, _) = ctl(100, true);
        let c2 = c.clone();
        let loser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            c2.run(RescueReason::ConsoleClose, Duration::from_secs(5))
        });
        let o = c.run(RescueReason::Panic, Duration::from_secs(5)).unwrap();
        assert!(o.error.is_some() && !o.complete, "{o:?}");
        let lo = loser.join().unwrap().unwrap();
        assert_eq!(*lo, *o);
        assert_eq!(c.state(), EmergencyState::Done);
    }

    #[test]
    fn 負けた経路は予算が尽きたら待つのをやめる() {
        let (c, w) = ctl(600, false);
        let c2 = c.clone();
        let winner =
            std::thread::spawn(move || c2.run(RescueReason::Signal, Duration::from_secs(5)));
        // 勝つ側が書き始めるのを待つ (固定の待ちだと、負荷の高い CI で先に呼ばれて勝敗が逆になる)
        let wait = Instant::now();
        while w.calls.load(Ordering::SeqCst) == 0 {
            assert!(
                wait.elapsed() < Duration::from_secs(5),
                "書き手が始まらない"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let t = Instant::now();
        let o = c.run(RescueReason::Fatal, Duration::from_millis(100));
        assert!(o.is_none(), "書込中のまま予算が尽きた");
        assert!(t.elapsed() < Duration::from_millis(400));
        assert!(winner.join().unwrap().is_some());
    }

    #[test]
    fn 公開と取り消し() {
        let c = EmergencyCtl::new();
        assert!(c.docs().is_empty());
        let snap = |id: u64| DocSnapshot {
            doc: DocumentId(id),
            snapshot: eve_text::Text::from("x").snapshot(),
            format: eve_core::TextFormat::new_document(),
            revision: eve_view::Revision(1),
            saved_node_exists: false,
            sensitive: false,
            path: None,
            untitled_name: None,
        };
        c.publish(DocumentId(1), Some(snap(1)));
        c.publish(DocumentId(2), Some(snap(2)));
        c.publish(DocumentId(1), Some(snap(1))); // 更新は置き換え
        assert_eq!(c.docs().len(), 2);
        c.publish(DocumentId(1), None);
        assert_eq!(c.docs().len(), 1);
        assert_eq!(c.docs()[0].doc, DocumentId(2));
    }

    #[test]
    fn 書き手が無いときは文書が無ければ完了_あればエラー() {
        let c = EmergencyCtl::new();
        let o = c.run(RescueReason::Signal, Duration::from_secs(1)).unwrap();
        assert!(o.complete && o.error.is_none());
        let c = EmergencyCtl::new();
        c.publish(
            DocumentId(1),
            Some(DocSnapshot {
                doc: DocumentId(1),
                snapshot: eve_text::Text::from("x").snapshot(),
                format: eve_core::TextFormat::new_document(),
                revision: eve_view::Revision(1),
                saved_node_exists: false,
                sensitive: false,
                path: None,
                untitled_name: None,
            }),
        );
        let o = c.run(RescueReason::Signal, Duration::from_secs(1)).unwrap();
        assert!(o.error.is_some());
    }
}
