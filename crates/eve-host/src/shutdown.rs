//! 非対話の終了 (docs/02 §9.6 の手順 1〜4、§11.2.1 の「終了の順序と時間予算」、05 §11.2 / §11.4)。
//!
//! シグナル・コンソールイベント・セッション終了の通知を受けたスレッドが呼ぶ。OS に依らない手順 (退避 → メインへの通知と
//! 終了待ち → 置換中の計数の待ち → 端末の緊急復元 → 退避の結果の行) をここに置き、bin は OS のコールバックの登録と、
//! 戻り値 (`ShutdownEnd`) に応じた終わり方 (Unix は同じシグナル、Windows は `exit(130)`・ハンドラからの復帰) だけを行う。
//! 時計と待ちは注入する (試験は `ManualClock` と、時計を進める偽の待ちで回す)。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use eve_core::{Clock, ReplacingCount, RescueOutcome, RescueReason, Tick};

use crate::clock::SystemClock;
use crate::emergency::EmergencyCtl;
use crate::frontend::EmergencyRestoreInfo;
use crate::host::{RESCUE_BUDGET_CLOSE, RESCUE_BUDGET_SESSION, RESCUE_BUDGET_SIGNAL};
use crate::msg::{HostMsg, SignalKind};

/// 手順 2: `HostMsg::Signal` を送った後、メインの終了を待つ上限 (02 §9.6 手順 2、§11.2.1 の表の段 2)。
pub const MAIN_EXIT_WAIT: Duration = Duration::from_secs(2);
/// シグナル・外部からの CTRL_C / CTRL_BREAK の合計の上限 (受信から、02 §11.2.1 の表の「合計の上限」)。
pub const SIGNAL_TOTAL_LIMIT: Duration = Duration::from_secs(5);
/// 手順 4 の端末の緊急復元に残す時間 (使い捨てのスレッドを待つ上限、02 §9.6 手順 4)。
pub const EMERGENCY_RESTORE_RESERVE: Duration = Duration::from_millis(200);
/// シグナル系で置換中の計数を待つ期限 (受信から 5 s − 200 ms、02 §11.2.1 の表の段 3)。
pub const REPLACING_DEADLINE_SIGNAL: Duration =
    SIGNAL_TOTAL_LIMIT.saturating_sub(EMERGENCY_RESTORE_RESERVE);
/// CTRL_CLOSE・セッション終了で置換中の計数を待つ期限 (受信から t = 4.3 s。残りの 0.2 s は復帰の余裕。
/// 合計の上限 4.5 s の内、02 §11.2.1 の表の段 3、05 §11.2 の時系列表)。
pub const REPLACING_DEADLINE_CLOSE: Duration = Duration::from_millis(4300);
/// パニック・`Fatal` のメインが、退避の後に置換中の計数を待つ上限 (02 §11.2.1 の表の段 3)。
pub const REPLACING_WAIT_FATAL: Duration = Duration::from_secs(1);
/// メインの終了を見る間隔。
const MAIN_POLL: Duration = Duration::from_millis(10);
/// 置換中の計数を見る間隔。
const REPLACING_POLL: Duration = Duration::from_millis(5);

/// 非対話の終了の契機 (02 §9.6 の表の 2 行目)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShutdownTrigger {
    /// Unix の SIGINT / SIGTERM / SIGHUP、Windows の CTRL_C / CTRL_BREAK 等 (CTRL_CLOSE 以外)。手順 1〜4。
    Signal(SignalKind),
    /// Windows の `CTRL_CLOSE_EVENT`。1・2 (送るだけ)・3 を行ってハンドラから戻る。
    ConsoleClose,
    /// Windows の `WM_QUERYENDSESSION` (使い捨てのスレッドから)。退避だけを行う。
    SessionQuery,
    /// Windows の `WM_ENDSESSION` (`wParam = TRUE`)。退避の完了 → 置換中の計数 → `Signal(Terminate)` を送って戻る。
    SessionEnd,
}

/// `NonInteractiveShutdown::run` の結果。呼び手 (bin) がこれに応じて終わる。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownEnd {
    /// メインへ送れなかった (受信口が閉じている)。何もせずに戻る (Windows のハンドラは `FALSE`)。
    NotSent,
    /// メインが `MAIN_EXIT_WAIT` の内に終わった (メインが restore・置換中の待ち・stderr を行う)。何もせずに戻る。
    MainFinished,
    /// メインが止まっていたので、受信スレッドが端末を戻し、退避の結果を出した。呼び手がプロセスを終える
    /// (Unix は同じシグナル、Windows は `ExitCode::INTERRUPTED`。07 §18.4、05 §11.5)。
    Exit,
    /// ハンドラから戻る (CTRL_CLOSE・セッション終了。端末はコンソールごと閉じられる)。
    Return,
}

/// 組み立ての材料 (Host から受け取る共有物)。
pub struct ShutdownParts {
    pub emergency: Arc<EmergencyCtl>,
    pub replacing: ReplacingCount,
    pub restore_info: Arc<OnceLock<EmergencyRestoreInfo>>,
    pub run_finished: Arc<AtomicBool>,
    pub tx: Sender<HostMsg>,
}

type SleepFn = dyn Fn(Duration) + Send + Sync;
type RestoreFn = dyn Fn(&EmergencyRestoreInfo) + Send + Sync;
type ReportFn = dyn Fn(&RescueOutcome) + Send + Sync;

/// 非対話の終了の手順 (02 §9.6)。複製は同じ共有物を指す (OS のコールバックごとに 1 つ持たせる)。
#[derive(Clone)]
pub struct NonInteractiveShutdown {
    emergency: Arc<EmergencyCtl>,
    replacing: ReplacingCount,
    restore_info: Arc<OnceLock<EmergencyRestoreInfo>>,
    run_finished: Arc<AtomicBool>,
    tx: Sender<HostMsg>,
    clock: Arc<dyn Clock>,
    sleep: Arc<SleepFn>,
    restore: Arc<RestoreFn>,
    report: Arc<ReportFn>,
}

impl NonInteractiveShutdown {
    /// 本番の部品で作る: 単調時計・`thread::sleep`・eve-platform の `emergency_restore`。退避の結果の行は出さない
    /// (`with_report` で渡す。文言は bin の表にある)。
    pub fn new(parts: ShutdownParts) -> NonInteractiveShutdown {
        NonInteractiveShutdown {
            emergency: parts.emergency,
            replacing: parts.replacing,
            restore_info: parts.restore_info,
            run_finished: parts.run_finished,
            tx: parts.tx,
            clock: Arc::new(SystemClock::new()),
            sleep: Arc::new(std::thread::sleep),
            restore: Arc::new(|info: &EmergencyRestoreInfo| {
                eve_platform::term::emergency_restore(
                    info.handle.0,
                    info.bytes,
                    &info.saved_mode.0,
                    &info.restored.0,
                );
            }),
            report: Arc::new(|_: &RescueOutcome| {}),
        }
    }

    /// 退避の結果を stderr へ出す関数 (受信スレッドが終えるとき、端末の復元の後に 1 回、02 §11.2.1 の表の段 5)。
    pub fn with_report(mut self, f: impl Fn(&RescueOutcome) + Send + Sync + 'static) -> Self {
        self.report = Arc::new(f);
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_sleep(mut self, f: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Arc::new(f);
        self
    }

    pub fn with_restore(
        mut self,
        f: impl Fn(&EmergencyRestoreInfo) + Send + Sync + 'static,
    ) -> Self {
        self.restore = Arc::new(f);
        self
    }

    /// 契機ごとの手順を行う。`ShutdownEnd::Exit` のときは呼び手が直ちにプロセスを終える。
    pub fn run(&self, trigger: ShutdownTrigger) -> ShutdownEnd {
        let t0 = self.clock.now();
        // 退避の予算の起点 (受信の時刻)。書き手は実時間で打ち切るので、注入した時計ではなく `Instant` で持つ
        let received = Instant::now();
        match trigger {
            ShutdownTrigger::Signal(kind) => {
                // 1. 退避 (最大 2.5 s。CAS に負けたら Done を待つ)
                self.emergency
                    .run_since(RescueReason::Signal, received, RESCUE_BUDGET_SIGNAL);
                // 2. メインへ知らせ、最大 2 s 終了を待つ
                if self.tx.send(HostMsg::Signal(kind)).is_err() {
                    return ShutdownEnd::NotSent;
                }
                let start = self.clock.now();
                while self.since(start) < MAIN_EXIT_WAIT {
                    if self.run_finished.load(Ordering::Acquire) {
                        return ShutdownEnd::MainFinished;
                    }
                    (self.sleep)(MAIN_POLL);
                }
                // 3. メインが止まっている: 置換中の計数を、受信から 4.8 s まで (復元の 200 ms を残して) 待つ
                self.wait_replacing(Some(
                    REPLACING_DEADLINE_SIGNAL.saturating_sub(self.since(t0)),
                ));
                // 4. 端末の緊急復元 → 5. 退避の結果 (ベストエフォートで 1 回)
                if let Some(info) = self.restore_info.get() {
                    (self.restore)(info);
                }
                if let Some(o) = self.emergency.outcome() {
                    (self.report)(&o);
                }
                ShutdownEnd::Exit
            }
            ShutdownTrigger::ConsoleClose => {
                // 1. 退避 (最大 3.5 s) → 2. 送るだけ (待たない)
                self.emergency
                    .run_since(RescueReason::ConsoleClose, received, RESCUE_BUDGET_CLOSE);
                if self
                    .tx
                    .send(HostMsg::Signal(SignalKind::ConsoleClose))
                    .is_err()
                {
                    return ShutdownEnd::NotSent;
                }
                // 3. 置換中の計数を受信から 4.3 s まで待って戻る (端末はコンソールごと閉じられる、02 §9.6 手順 2)
                self.wait_replacing(Some(
                    REPLACING_DEADLINE_CLOSE.saturating_sub(self.since(t0)),
                ));
                ShutdownEnd::Return
            }
            ShutdownTrigger::SessionQuery => {
                self.emergency
                    .run_since(RescueReason::SessionEnd, received, RESCUE_BUDGET_SESSION);
                ShutdownEnd::Return
            }
            ShutdownTrigger::SessionEnd => {
                // 問い合わせで始めた退避の完了を待つ (届いていなければここで書く。退避は 1 回だけ)
                self.emergency
                    .run_since(RescueReason::SessionEnd, received, RESCUE_BUDGET_SESSION);
                self.wait_replacing(Some(
                    REPLACING_DEADLINE_CLOSE.saturating_sub(self.since(t0)),
                ));
                // 待った後にメインへ知らせる (05 §11.2 の M9d の実装)。送れなくても戻るだけ
                let _ = self.tx.send(HostMsg::Signal(SignalKind::Terminate));
                ShutdownEnd::Return
            }
        }
    }

    /// 置換中の計数が 0 に戻るまで待つ。`limit` が `None` なら無期限 (通常終了とメインの後始末、02 §9.6)。0 になったら `true`。
    pub fn wait_replacing(&self, limit: Option<Duration>) -> bool {
        let start = self.clock.now();
        loop {
            if self.replacing.is_zero() {
                return true;
            }
            if limit.is_some_and(|l| self.since(start) >= l) {
                return false;
            }
            (self.sleep)(REPLACING_POLL);
        }
    }

    fn since(&self, t: Tick) -> Duration {
        Duration::from_millis(self.clock.now().0.saturating_sub(t.0))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, channel};

    use eve_core::{DocSnapshot, Millis, ReplacingGuard, RescueWriter};
    use eve_testkit::ManualClock;
    use eve_view::{DocumentId, RawTermHandle, SavedTermMode, TermRestoredFlag};

    use super::*;

    /// 出来事の記録 (時刻つき)。
    type Log = Arc<Mutex<Vec<(u64, String)>>>;

    fn push(log: &Log, clock: &ManualClock, s: String) {
        log.lock().unwrap().push((clock.now().0, s));
    }

    /// 書くたびに記録し、`cost` だけ時計を進める退避の書き手。
    struct FakeWriter {
        log: Log,
        clock: ManualClock,
        cost: Millis,
    }

    impl RescueWriter for FakeWriter {
        fn write(
            &self,
            docs: &[Arc<DocSnapshot>],
            reason: RescueReason,
            _received: Instant,
            budget: Duration,
        ) -> RescueOutcome {
            push(
                &self.log,
                &self.clock,
                format!("rescue {reason:?} {}ms", budget.as_millis()),
            );
            self.clock.advance(self.cost);
            RescueOutcome {
                total: docs.len(),
                written: docs.len(),
                stored: docs.len(),
                complete: true,
                ..RescueOutcome::default()
            }
        }
    }

    struct Rig {
        sd: NonInteractiveShutdown,
        clock: ManualClock,
        log: Log,
        rx: Option<Receiver<HostMsg>>,
        run_finished: Arc<AtomicBool>,
        restored: TermRestoredFlag,
        replacing: ReplacingCount,
        /// 偽の待ちが、この時刻 (ms) を過ぎたら下ろすガード。
        guard: Arc<Mutex<Option<(u64, ReplacingGuard)>>>,
        /// 偽の待ちが、この時刻 (ms) を過ぎたら `run_finished` を立てる。
        finish_at: Arc<Mutex<Option<u64>>>,
    }

    fn rig(rescue_cost: u64, terminal: bool) -> Rig {
        let clock = ManualClock::new();
        let log: Log = Arc::default();
        let emergency = EmergencyCtl::new();
        emergency.set_writer(Arc::new(FakeWriter {
            log: log.clone(),
            clock: clock.clone(),
            cost: Millis(rescue_cost),
        }));
        let replacing = ReplacingCount::default();
        let restore_info = Arc::new(OnceLock::new());
        let restored = TermRestoredFlag(Arc::new(AtomicBool::new(false)));
        if terminal {
            let _ = restore_info.set(EmergencyRestoreInfo {
                handle: RawTermHandle(0),
                bytes: b"\x18",
                saved_mode: SavedTermMode(Box::new([])),
                restored: restored.clone(),
            });
        }
        let run_finished = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let guard: Arc<Mutex<Option<(u64, ReplacingGuard)>>> = Arc::default();
        let finish_at: Arc<Mutex<Option<u64>>> = Arc::default();
        let sd = NonInteractiveShutdown::new(ShutdownParts {
            emergency,
            replacing: replacing.clone(),
            restore_info,
            run_finished: run_finished.clone(),
            tx,
        })
        .with_clock(Arc::new(clock.clone()))
        .with_sleep({
            let clock = clock.clone();
            let guard = guard.clone();
            let finish_at = finish_at.clone();
            let run_finished = run_finished.clone();
            move |d| {
                clock.advance(Millis(u64::try_from(d.as_millis()).unwrap()));
                let now = clock.now().0;
                let mut g = guard.lock().unwrap();
                if g.as_ref().is_some_and(|(at, _)| now >= *at) {
                    *g = None;
                }
                if finish_at.lock().unwrap().is_some_and(|at| now >= at) {
                    run_finished.store(true, Ordering::Release);
                }
            }
        })
        .with_restore({
            let log = log.clone();
            let clock = clock.clone();
            move |info| {
                // 本番の emergency_restore と同じく、フラグを立てた側だけが戻す
                if !info.restored.0.swap(true, Ordering::AcqRel) {
                    push(&log, &clock, "restore".to_owned());
                }
            }
        })
        .with_report({
            let log = log.clone();
            let clock = clock.clone();
            move |o| push(&log, &clock, format!("report {}/{}", o.written, o.total))
        });
        Rig {
            sd,
            clock,
            log,
            rx: Some(rx),
            run_finished,
            restored,
            replacing,
            guard,
            finish_at,
        }
    }

    impl Rig {
        fn hold_replacing_until(&self, at: u64) {
            *self.guard.lock().unwrap() = Some((at, self.replacing.guard("T.txt")));
        }

        fn finish_main_at(&self, at: u64) {
            *self.finish_at.lock().unwrap() = Some(at);
        }

        fn events(&self) -> Vec<(u64, String)> {
            self.log.lock().unwrap().clone()
        }

        fn msgs(&self) -> Vec<String> {
            self.rx
                .as_ref()
                .unwrap()
                .try_iter()
                .map(|m| format!("{m:?}"))
                .collect()
        }
    }

    #[test]
    fn 予算の定数は仕様の表の値() {
        assert_eq!(MAIN_EXIT_WAIT, Duration::from_secs(2));
        assert_eq!(REPLACING_DEADLINE_SIGNAL, Duration::from_millis(4800));
        assert_eq!(REPLACING_DEADLINE_CLOSE, Duration::from_millis(4300));
        assert_eq!(REPLACING_WAIT_FATAL, Duration::from_secs(1));
        assert_eq!(RESCUE_BUDGET_SIGNAL, Duration::from_millis(2500));
        assert_eq!(RESCUE_BUDGET_CLOSE, Duration::from_millis(3500));
        assert_eq!(RESCUE_BUDGET_SESSION, Duration::from_millis(3500));
    }

    #[test]
    fn シグナル_メインが_2_秒の内に終われば受信スレッドは復元しない() {
        let r = rig(300, true);
        r.finish_main_at(1000);
        let end = r.sd.run(ShutdownTrigger::Signal(SignalKind::Terminate));
        assert_eq!(end, ShutdownEnd::MainFinished);
        assert_eq!(r.events(), vec![(0, "rescue Signal 2500ms".to_owned())]);
        assert_eq!(r.msgs(), vec!["Signal(Terminate)".to_owned()]);
        assert!(!r.restored.0.load(Ordering::Acquire));
    }

    #[test]
    fn シグナル_メインが止まっていれば_2_秒後に復元して結果を出し_終わらせる() {
        let r = rig(300, true);
        let end = r.sd.run(ShutdownTrigger::Signal(SignalKind::Hangup));
        assert_eq!(end, ShutdownEnd::Exit);
        // 退避 (0〜300 ms) → 送信 → メインを 2 s 待つ → 置換中は 0 なので待たない → 復元 → 結果の行
        assert_eq!(
            r.events(),
            vec![
                (0, "rescue Signal 2500ms".to_owned()),
                (2300, "restore".to_owned()),
                (2300, "report 0/0".to_owned()),
            ]
        );
        assert_eq!(r.msgs(), vec!["Signal(Hangup)".to_owned()]);
    }

    #[test]
    fn シグナル_置換中が下りなければ受信から_4_8_秒で打ち切って復元する() {
        let r = rig(1000, true);
        r.hold_replacing_until(u64::MAX);
        let end = r.sd.run(ShutdownTrigger::Signal(SignalKind::Interrupt));
        assert_eq!(end, ShutdownEnd::Exit);
        let ev = r.events();
        assert_eq!(ev[1], (4800, "restore".to_owned()), "{ev:?}");
        assert_eq!(ev[2].1, "report 0/0");
        assert!(!r.replacing.is_zero(), "打ち切っても計数は触らない");
    }

    #[test]
    fn シグナル_置換中が下りたらすぐに復元する() {
        let r = rig(0, true);
        r.hold_replacing_until(2500);
        let end = r.sd.run(ShutdownTrigger::Signal(SignalKind::Interrupt));
        assert_eq!(end, ShutdownEnd::Exit);
        assert_eq!(r.events()[1], (2500, "restore".to_owned()));
    }

    #[test]
    fn シグナル_退避が予算を超えても先へ進み_置換中の待ちは残りが無ければ待たない() {
        // 書き手が予算 (2.5 s) を守らずに 3 s かかった: 2 の待ち (2 s) の後は 5 s を過ぎるので 3 は待たない
        let r = rig(3000, true);
        r.hold_replacing_until(u64::MAX);
        let end = r.sd.run(ShutdownTrigger::Signal(SignalKind::Terminate));
        assert_eq!(end, ShutdownEnd::Exit);
        assert_eq!(r.events()[1], (5000, "restore".to_owned()));
    }

    #[test]
    fn シグナル_端末が無ければ復元せずに結果だけ出す() {
        let r = rig(0, false);
        let end = r.sd.run(ShutdownTrigger::Signal(SignalKind::Terminate));
        assert_eq!(end, ShutdownEnd::Exit);
        assert_eq!(
            r.events(),
            vec![
                (0, "rescue Signal 2500ms".to_owned()),
                (2000, "report 0/0".to_owned()),
            ]
        );
    }

    #[test]
    fn 受信口が閉じていれば待たずに戻る() {
        let mut r = rig(0, true);
        r.rx = None;
        let end = r.sd.run(ShutdownTrigger::Signal(SignalKind::Terminate));
        assert_eq!(end, ShutdownEnd::NotSent);
        assert_eq!(r.clock.now(), Tick(0));
        assert_eq!(r.events().len(), 1, "退避は書く");
        assert_eq!(
            r.sd.run(ShutdownTrigger::ConsoleClose),
            ShutdownEnd::NotSent
        );
    }

    #[test]
    fn 二度目のシグナルでは退避を書かず_復元も一度だけ() {
        let r = rig(100, true);
        assert_eq!(
            r.sd.run(ShutdownTrigger::Signal(SignalKind::Terminate)),
            ShutdownEnd::Exit
        );
        assert_eq!(
            r.sd.run(ShutdownTrigger::Signal(SignalKind::Interrupt)),
            ShutdownEnd::Exit
        );
        let ev = r.events();
        let names: Vec<&str> = ev.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "rescue Signal 2500ms",
                "restore",
                "report 0/0",
                "report 0/0"
            ]
        );
        assert_eq!(
            r.msgs(),
            vec![
                "Signal(Terminate)".to_owned(),
                "Signal(Interrupt)".to_owned()
            ]
        );
    }

    #[test]
    fn 閉鎖_送るだけでメインを待たず_置換中を受信から_4_3_秒まで待って戻る() {
        let r = rig(1000, true);
        r.hold_replacing_until(u64::MAX);
        let end = r.sd.run(ShutdownTrigger::ConsoleClose);
        assert_eq!(end, ShutdownEnd::Return);
        assert_eq!(r.clock.now(), Tick(4300));
        assert_eq!(
            r.events(),
            vec![(0, "rescue ConsoleClose 3500ms".to_owned())]
        );
        assert_eq!(r.msgs(), vec!["Signal(ConsoleClose)".to_owned()]);
        assert!(
            !r.restored.0.load(Ordering::Acquire),
            "閉鎖では端末を戻さない"
        );
    }

    #[test]
    fn 閉鎖_置換中が無ければ退避の後すぐに戻る() {
        let r = rig(700, true);
        r.finish_main_at(u64::MAX);
        assert_eq!(r.sd.run(ShutdownTrigger::ConsoleClose), ShutdownEnd::Return);
        assert_eq!(r.clock.now(), Tick(700));
    }

    #[test]
    fn セッション終了_問い合わせで退避し_終了の通知では書かずに待ってから知らせる() {
        let r = rig(500, true);
        assert_eq!(r.sd.run(ShutdownTrigger::SessionQuery), ShutdownEnd::Return);
        assert!(r.msgs().is_empty(), "問い合わせではメインへ知らせない");
        r.hold_replacing_until(u64::MAX);
        let t = r.clock.now().0;
        assert_eq!(r.sd.run(ShutdownTrigger::SessionEnd), ShutdownEnd::Return);
        assert_eq!(r.clock.now().0 - t, 4300, "置換中は受信から 4.3 s まで待つ");
        assert_eq!(r.events(), vec![(0, "rescue SessionEnd 3500ms".to_owned())]);
        assert_eq!(r.msgs(), vec!["Signal(Terminate)".to_owned()]);
        assert!(!r.restored.0.load(Ordering::Acquire));
    }

    #[test]
    fn セッション終了_問い合わせが届いていなければ終了の通知で書く() {
        let r = rig(500, true);
        assert_eq!(r.sd.run(ShutdownTrigger::SessionEnd), ShutdownEnd::Return);
        assert_eq!(r.events(), vec![(0, "rescue SessionEnd 3500ms".to_owned())]);
        assert_eq!(r.clock.now(), Tick(500));
    }

    #[test]
    fn 置換中の待ちは期限なしなら下りるまで待つ() {
        let r = rig(0, false);
        assert!(r.sd.wait_replacing(None), "0 なら待たない");
        assert_eq!(r.clock.now(), Tick(0));
        r.hold_replacing_until(60_000);
        assert!(r.sd.wait_replacing(None));
        assert_eq!(r.clock.now(), Tick(60_000));
        r.hold_replacing_until(u64::MAX);
        assert!(!r.sd.wait_replacing(Some(REPLACING_WAIT_FATAL)));
        assert_eq!(r.clock.now(), Tick(61_000));
    }

    #[test]
    fn 退避は公開済みの文書を書く() {
        let r = rig(0, true);
        r.sd.emergency.publish(
            DocumentId(1),
            Some(Arc::new(DocSnapshot {
                doc: DocumentId(1),
                snapshot: eve_text::Text::from("x").snapshot(),
                format: eve_core::TextFormat::new_document(),
                revision: eve_view::Revision(1),
                saved_node_exists: false,
                sensitive: false,
                path: None,
                untitled_name: None,
            })),
        );
        assert_eq!(
            r.sd.run(ShutdownTrigger::Signal(SignalKind::Terminate)),
            ShutdownEnd::Exit
        );
        assert_eq!(r.events()[2].1, "report 1/1");
        assert!(!r.run_finished.load(Ordering::Acquire));
    }
}
