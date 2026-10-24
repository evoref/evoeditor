//! システムクリップボードの経路 (docs/05 §8.1、02 §6.3): 書込・読込の使い捨てスレッドと、取得の 2 秒のタイムアウト。
//! 提供者は偽物 (失敗と遅延を注入する)。ワーカーが Host の送信口へ送るメッセージは、テストが受けて `step` へ渡す。

use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eve_core::{JobPorts, Ports};
use eve_host::{ClipboardFailure, ClipboardProvider, Host, HostMsg, channel};
use eve_testkit::{
    AsciiOptions, FixedPaths, HeadlessFrontend, ManualClock, ManualJobSpawner, MemFs,
};
use eve_view::{InputEvent, KeyCode, KeyEvent, Modifiers};

/// 書いた文字列を送り、読む値は門が開くまで返さない偽物。
struct Fake {
    written: Sender<String>,
    read_value: Mutex<Result<Option<String>, ClipboardFailure>>,
    gate: Mutex<Option<Receiver<()>>>,
    write_error: Mutex<Option<ClipboardFailure>>,
}

impl ClipboardProvider for Fake {
    fn set_text(&self, text: &str) -> Result<(), ClipboardFailure> {
        if let Some(e) = *self.write_error.lock().unwrap() {
            return Err(e);
        }
        let _ = self.written.send(text.to_owned());
        Ok(())
    }

    fn get_text(&self) -> Result<Option<String>, ClipboardFailure> {
        let gate = self.gate.lock().unwrap().take();
        if let Some(g) = gate {
            let _ = g.recv_timeout(Duration::from_secs(10));
        }
        self.read_value.lock().unwrap().clone()
    }
}

struct Env {
    host: Host,
    fe: HeadlessFrontend,
    clock: ManualClock,
    /// ワーカーが Host の送信口へ送ったメッセージ。
    from_workers: Receiver<HostMsg>,
    fake: Arc<Fake>,
    written: Receiver<String>,
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
        fs,
        clock: Arc::new(clock.clone()),
        jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
        paths,
    };
    let (to_host, from_workers) = channel();
    let (_unused_tx, host_inbox) = channel();
    let mut host = Host::with_channel(ports, (to_host, host_inbox));
    let (wtx, written) = mpsc::channel();
    let fake = Arc::new(Fake {
        written: wtx,
        read_value: Mutex::new(Ok(Some("SYS".to_owned()))),
        gate: Mutex::new(None),
        write_error: Mutex::new(None),
    });
    host.set_clipboard(fake.clone());
    Env {
        host,
        fe: HeadlessFrontend::new(40, 8),
        clock,
        from_workers,
        fake,
        written,
    }
}

impl Env {
    fn start(&mut self) {
        self.host.start(&mut self.fe, None, false).unwrap();
    }

    fn input(&mut self, ev: InputEvent) {
        let w = self.host.window().unwrap();
        self.host.step(&mut self.fe, HostMsg::Input(w, ev));
    }

    fn key(&mut self, code: KeyCode, mods: Modifiers) {
        self.input(InputEvent::Key(KeyEvent::press(code, mods)));
    }

    fn text(&mut self, s: &str) {
        self.input(InputEvent::Text(s.into()));
    }

    /// ワーカーからのメッセージを 1 つ待って `step` へ渡す。
    fn pump_worker(&mut self) -> bool {
        match self.from_workers.recv_timeout(Duration::from_secs(5)) {
            Ok(m) => {
                self.host.step(&mut self.fe, m);
                true
            }
            Err(_) => false,
        }
    }

    /// 描画の間引き (16 ms) を過ぎさせてから、最後の画面を返す。
    fn screen(&mut self) -> String {
        self.clock.advance(eve_core::Millis(20));
        self.host.step(&mut self.fe, HostMsg::Tick);
        self.fe.probe().last_ascii(AsciiOptions::default()).unwrap()
    }
}

#[test]
fn コピーはシステムのクリップボードへ書かれる() {
    let mut e = env();
    e.start();
    e.text("hello");
    e.key(KeyCode::Char('a'), Modifiers::CTRL);
    e.key(KeyCode::Char('c'), Modifiers::CTRL);
    let got = e
        .written
        .recv_timeout(Duration::from_secs(5))
        .expect("書かれる");
    assert_eq!(got, "hello");
}

#[test]
fn 貼り付けはシステムのクリップボードから読む() {
    let mut e = env();
    e.start();
    e.key(KeyCode::Char('v'), Modifiers::CTRL);
    assert!(e.pump_worker(), "取得の結果が届く");
    assert!(e.screen().contains("SYS"), "{}", e.screen());
}

// verifies: AC-FR-205-3
#[test]
fn 取得が_2_秒かかると内部レジスタに切り替え_遅れて届いた結果は捨てる() {
    let mut e = env();
    e.start();
    // 先にコピーして内部レジスタに入れておく
    e.text("REG");
    e.key(KeyCode::Char('a'), Modifiers::CTRL);
    e.key(KeyCode::Char('c'), Modifiers::CTRL);
    e.written.recv_timeout(Duration::from_secs(5)).unwrap();
    e.key(KeyCode::End, Modifiers::NONE);
    // 取得を門で止める
    let (gate_tx, gate_rx) = mpsc::channel();
    *e.fake.gate.lock().unwrap() = Some(gate_rx);
    e.key(KeyCode::Char('v'), Modifiers::CTRL);
    // 2 秒たつ前は何も貼られない
    e.clock.advance(eve_core::Millis(1500));
    e.input(InputEvent::Focus(true));
    assert!(!e.screen().contains("REGREG"));
    // 2 秒たつと内部レジスタから貼られる
    e.clock.advance(eve_core::Millis(600));
    e.input(InputEvent::Focus(true));
    assert!(e.screen().contains("REGREG"), "{}", e.screen());
    // 放棄したスレッドが後から結果を返しても捨てる (二重に貼られない)
    gate_tx.send(()).unwrap();
    assert!(e.pump_worker());
    assert!(!e.screen().contains("REGREGSYS") && !e.screen().contains("REGREGREG"));
}

// verifies: AC-FR-205-2
#[test]
fn 書込の失敗は内部レジスタに切り替えたと知らせる() {
    let mut e = env();
    e.start();
    *e.fake.write_error.lock().unwrap() = Some(ClipboardFailure::Unavailable);
    e.text("abc");
    e.key(KeyCode::Char('a'), Modifiers::CTRL);
    e.key(KeyCode::Char('c'), Modifiers::CTRL);
    assert!(e.pump_worker(), "失敗の報告が届く");
    let keys: Vec<String> = e
        .host
        .take_events()
        .into_iter()
        .filter_map(|ev| match ev {
            eve_view::EditorEvent::MessageShown { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert!(
        keys.contains(&"msg.clipboard.fallback".to_owned()),
        "{keys:?}"
    );
}

// verifies: AC-FR-205-2
#[test]
fn 取得の失敗は内部レジスタを貼り付ける() {
    let mut e = env();
    e.start();
    e.text("xy");
    e.key(KeyCode::Char('a'), Modifiers::CTRL);
    e.key(KeyCode::Char('c'), Modifiers::CTRL);
    e.written.recv_timeout(Duration::from_secs(5)).unwrap();
    e.key(KeyCode::End, Modifiers::NONE);
    *e.fake.read_value.lock().unwrap() = Err(ClipboardFailure::Unavailable);
    e.key(KeyCode::Char('v'), Modifiers::CTRL);
    assert!(e.pump_worker());
    assert!(e.screen().contains("xyxy"), "{}", e.screen());
}

// verifies: AC-FR-205-4
#[test]
fn 上限を超える全選択のコピーは文字列化せず内部レジスタだけに入れて知らせる() {
    let mut e = env();
    e.start();
    // 64 MiB (05 §8.4) を少し超える本文
    let big = "0123456789 the quick brown fox 0123456789 0123456789 0123456789 0123456789
"
    .repeat(66 * 1024 * 1024 / 73);
    e.input(InputEvent::Paste(big));
    e.key(KeyCode::Char('a'), Modifiers::CTRL);
    let t0 = std::time::Instant::now();
    e.key(KeyCode::Char('c'), Modifiers::CTRL);
    // 判定は選択範囲の総バイト数で行う。文字列化もシステムへの書込も起きない
    assert!(
        e.written.recv_timeout(Duration::from_millis(500)).is_err(),
        "上限を超えたのにシステムのクリップボードへ書いた"
    );
    assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
    let keys: Vec<String> = e
        .host
        .take_events()
        .into_iter()
        .filter_map(|ev| match ev {
            eve_view::EditorEvent::MessageShown { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert!(
        keys.contains(&"msg.clipboard.fallback".to_owned()),
        "{keys:?}"
    );
    // 内部レジスタには入っていて、貼り付けで取り出せる (システム側の値は SYS)
    *e.fake.read_value.lock().unwrap() = Err(ClipboardFailure::Unavailable);
    e.key(KeyCode::End, Modifiers::CTRL);
    e.key(KeyCode::Char('v'), Modifiers::CTRL);
    assert!(e.pump_worker());
}
