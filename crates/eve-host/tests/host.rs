//! `Host::step` / `Host::run` の駆動 (docs/02 §9.2)。端末は `HeadlessFrontend`、時計・ジョブは手動。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{JobPorts, Ports};
use eve_host::{ExitCode, FatalReason, Host, HostMsg, SignalKind, StepOutcome};
use eve_testkit::{
    AsciiOptions, FixedPaths, HeadlessFrontend, HeadlessProbe, ManualClock, ManualJobSpawner, MemFs,
};
use eve_view::{InputEvent, KeyCode, KeyEvent, Modifiers};

struct Env {
    host: Host,
    fe: HeadlessFrontend,
    probe: HeadlessProbe,
    clock: ManualClock,
    fs: Arc<MemFs>,
    jobs: Rc<ManualJobSpawner>,
}

fn env() -> Env {
    let clock = ManualClock::new();
    let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
    fs.add_dir("/w");
    let paths = Arc::new(FixedPaths::default());
    let jp = JobPorts {
        fs: fs.clone(),
        clock: Arc::new(clock.clone()),
        paths: paths.clone(),
    };
    let jobs = ManualJobSpawner::new(jp);
    let ports = Ports {
        fs: fs.clone(),
        clock: Arc::new(clock.clone()),
        jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
        paths,
    };
    let fe = HeadlessFrontend::new(40, 8);
    let probe = fe.probe();
    Env {
        host: Host::new(ports),
        fe,
        probe,
        clock,
        fs,
        jobs,
    }
}

impl Env {
    fn start(&mut self) {
        self.host.start(&mut self.fe, None, false).unwrap();
    }

    fn input(&mut self, ev: InputEvent) -> StepOutcome {
        let w = self.host.window().unwrap();
        self.host.step(&mut self.fe, HostMsg::Input(w, ev))
    }

    fn ascii(&self) -> String {
        self.probe.last_ascii(AsciiOptions::default()).unwrap()
    }
}

#[test]
fn 起動すると最初の_frame_を描く() {
    let mut e = env();
    e.start();
    assert_eq!(e.probe.render_count(), 1);
    assert!(e.host.window().is_some());
}

#[test]
fn 入力は編集され即時に描かれる() {
    let mut e = env();
    e.start();
    assert_eq!(
        e.input(InputEvent::Text("abc".into())),
        StepOutcome::Continue
    );
    assert_eq!(e.probe.render_count(), 2);
    assert!(e.ascii().contains("abc"), "{}", e.ascii());
}

#[test]
fn 変化が無ければ描かない() {
    let mut e = env();
    e.start();
    e.host.step(&mut e.fe, HostMsg::Tick);
    assert_eq!(e.probe.render_count(), 1);
}

#[test]
fn 終了のコマンドで_restore_して終了コードを返す() {
    let mut e = env();
    e.start();
    let k = InputEvent::Key(KeyEvent::press(KeyCode::Char('q'), Modifiers::CTRL));
    assert_eq!(e.input(k), StepOutcome::Exit(ExitCode::OK));
    assert_eq!(e.probe.restore_count(), 1);
}

#[test]
fn シグナルと致命的エラーは_restore_して終了する() {
    let mut e = env();
    e.start();
    assert_eq!(
        e.host
            .step(&mut e.fe, HostMsg::Signal(SignalKind::Interrupt)),
        StepOutcome::Exit(ExitCode::INTERRUPTED)
    );
    let mut e2 = env();
    e2.start();
    assert_eq!(
        e2.host
            .step(&mut e2.fe, HostMsg::Fatal(FatalReason::InputThreadDied)),
        StepOutcome::Exit(ExitCode::FATAL)
    );
    assert_eq!(e.probe.restore_count(), 1);
    assert_eq!(e2.probe.restore_count(), 1);
}

#[test]
fn 入力口から送ったメッセージが溜まっていれば_1_回の_step_でまとめて処理する() {
    let mut e = env();
    e.start();
    let sink = e.probe.sink().unwrap();
    assert!(sink.send(InputEvent::Text("b".into())));
    assert!(sink.send(InputEvent::Text("c".into())));
    let w = e.host.window().unwrap();
    e.host
        .step(&mut e.fe, HostMsg::Input(w, InputEvent::Text("a".into())));
    assert_eq!(e.probe.render_count(), 2, "描画は 1 回");
    assert!(e.ascii().contains("abc"), "{}", e.ascii());
}

#[test]
fn ジョブの結果による再描画は間引かれ_期限で描かれる() {
    let mut e = env();
    e.fs.set_file(&eve_core::FsPath::from("/w/a.txt"), b"hello");
    e.start();
    // 開く (入力の結果なので即時に描く)
    e.host.editor_mut().open_initial(Some("/w/a.txt"), false);
    e.jobs.run_jobs();
    let before = e.probe.render_count();
    for r in e.jobs.take_results() {
        e.host.step(&mut e.fe, HostMsg::Job(Box::new(r)));
    }
    // 直前の描画から 16 ms 未満: 描かない
    assert_eq!(e.probe.render_count(), before);
    e.clock.advance(eve_core::Millis(20));
    e.host.step(&mut e.fe, HostMsg::Tick);
    assert!(e.probe.render_count() > before);
    assert!(e.ascii().contains("hello"), "{}", e.ascii());
}

#[test]
fn 未保存の文書は退避用のスロットに公開される() {
    let mut e = env();
    e.start();
    assert!(e.host.emergency_slots().is_empty());
    e.input(InputEvent::Text("x".into()));
    assert_eq!(e.host.emergency_slots().len(), 1);
}

#[test]
fn 破棄して置き換えた文書のスロットは消える() {
    let mut e = env();
    e.start();
    e.input(InputEvent::Text("x".into()));
    assert_eq!(e.host.emergency_slots().len(), 1);
    // file.new → 未保存の確認で「破棄」
    e.input(InputEvent::Key(KeyEvent::press(
        KeyCode::Char('n'),
        Modifiers::CTRL,
    )));
    e.input(InputEvent::Key(KeyEvent::press(
        KeyCode::Char('d'),
        Modifiers::NONE,
    )));
    assert!(e.host.emergency_slots().is_empty(), "{}", e.ascii());
}

// ---- 緊急退避 (02 §11.2.1) ----

use std::sync::Mutex;
use std::time::Duration;

use eve_core::{DocSnapshot, RescueOutcome, RescueReason, RescueWriter};

/// 呼ばれた回数・契機・文書の数を記録する書き手。
#[derive(Default)]
struct Spy {
    calls: Mutex<Vec<(RescueReason, usize)>>,
}

impl RescueWriter for Spy {
    fn write(
        &self,
        docs: &[Arc<DocSnapshot>],
        reason: RescueReason,
        _budget: Duration,
    ) -> RescueOutcome {
        self.calls.lock().unwrap().push((reason, docs.len()));
        RescueOutcome {
            total: docs.len(),
            written: docs.len(),
            complete: true,
            ..RescueOutcome::default()
        }
    }
}

fn with_spy(e: &mut Env) -> Arc<Spy> {
    let spy = Arc::new(Spy::default());
    e.host.set_rescue_writer(spy.clone());
    spy
}

#[test]
fn 致命的エラーでは_dirty_な文書を退避してから_restore_する() {
    let mut e = env();
    let spy = with_spy(&mut e);
    e.start();
    e.input(InputEvent::Text("unsaved".into()));
    let out = e
        .host
        .step(&mut e.fe, HostMsg::Fatal(FatalReason::InputThreadDied));
    assert_eq!(out, StepOutcome::Exit(ExitCode::FATAL));
    assert_eq!(*spy.calls.lock().unwrap(), vec![(RescueReason::Fatal, 1)]);
    assert_eq!(e.probe.restore_count(), 1);
    let o = e.host.emergency().outcome().expect("結果を bin が読む");
    assert!(o.complete && o.written == 1);
}

#[test]
fn シグナルとコンソールの閉鎖は契機を区別して退避する() {
    let mut e = env();
    let spy = with_spy(&mut e);
    e.start();
    e.input(InputEvent::Text("x".into()));
    e.host
        .step(&mut e.fe, HostMsg::Signal(SignalKind::ConsoleClose));
    assert_eq!(
        *spy.calls.lock().unwrap(),
        vec![(RescueReason::ConsoleClose, 1)]
    );
}

#[test]
fn 受信スレッドが先に退避していればメインは書き直さない() {
    let mut e = env();
    let spy = with_spy(&mut e);
    e.start();
    e.input(InputEvent::Text("x".into()));
    // コンソールハンドラのスレッドが先に書いた (メインを待たない、02 §11.2.1)
    let ctl = e.host.emergency();
    std::thread::spawn(move || ctl.run(RescueReason::Signal, Duration::from_secs(1)))
        .join()
        .unwrap();
    e.host
        .step(&mut e.fe, HostMsg::Signal(SignalKind::Interrupt));
    assert_eq!(
        spy.calls.lock().unwrap().len(),
        1,
        "退避はプロセスで 1 回だけ"
    );
}

#[test]
fn 保存済みの文書しか無ければ退避する文書は無い() {
    let mut e = env();
    let spy = with_spy(&mut e);
    e.start();
    e.host
        .step(&mut e.fe, HostMsg::Fatal(FatalReason::InputThreadDied));
    assert_eq!(*spy.calls.lock().unwrap(), vec![(RescueReason::Fatal, 0)]);
}

#[test]
fn 設定の診断は最初の_frame_の後にメッセージとして出す() {
    let mut e = env();
    let l = eve_config::load_str("[editor]\ntab_width = 0\n");
    e.host.set_config(l.config, "config.toml", l.diagnostics);
    e.start();
    assert_eq!(e.probe.render_count(), 2, "最初の Frame の後に再描画する");
    let screen = e
        .probe
        .last_ascii(AsciiOptions {
            show_chrome: true,
            ..AsciiOptions::default()
        })
        .unwrap();
    assert!(screen.contains("config.toml:2"), "{screen}");
}
