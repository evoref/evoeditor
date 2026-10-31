//! NFR-501 AC-2 (v0.2) の検査の本体: ヘッドレスのシナリオ 1 本 (開く → 編集 → 保存 → 終了) を、本番と同じ Host の実行ループ
//! (`Host::run`)・本物の時計 (`SystemClock`)・本物のワーカースレッド (`ThreadJobSpawner`) で動かす。`cargo xtask no-network`
//! がこのプログラムを `strace -f -e trace=network` の下で走らせ、ネットワーク系のシステムコールが 0 であることを確かめる
//! (docs/09 §3.6 の「無通信」)。
//!
//! FS は偽物 (`MemFs`): Linux の実 FS (eve-platform の Unix 実装) は M7 で入るので、それまでは FS の経路を除いた範囲を確かめる。
//!
//! ```text
//! cargo bench -p eve-testkit --bench no_network        # シナリオだけを実行 (成功で終了コード 0)
//! cargo xtask no-network                               # strace の下で実行して検査する (Linux)
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eve_core::{InitialFile, JobPorts, Ports};
use eve_host::{ExitCode, Host, SystemClock, ThreadJobSpawner};
use eve_testkit::{AsciiOptions, FixedPaths, HeadlessFrontend, MemFs};
use eve_view::{InputEvent, KeyCode, KeyEvent, Modifiers};

/// `cond` が真になるまで待つ (最大 `limit`)。間に合わなければ偽。
fn wait_until(limit: Duration, cond: impl Fn() -> bool) -> bool {
    let end = Instant::now() + limit;
    while !cond() {
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

// verifies: NFR-501
fn main() {
    let clock = Arc::new(SystemClock::new());
    let fs = Arc::new(MemFs::new(clock.clone()));
    fs.add_dir("/w");
    fs.add_file("/w/a.txt", b"hello\n");
    let paths = Arc::new(FixedPaths::default());
    let (tx, rx) = eve_host::channel();
    let jobs = ThreadJobSpawner::new(
        JobPorts {
            fs: fs.clone(),
            clock: clock.clone(),
            paths: paths.clone(),
        },
        tx.clone(),
    )
    .expect("ワーカースレッドを起動できる");
    let ports = Ports {
        fs: fs.clone(),
        clock,
        jobs: Rc::new(jobs),
        paths,
    };
    let mut host = Host::with_channel(ports, (tx, rx));
    let mut fe = HeadlessFrontend::new(80, 24);
    let probe = fe.probe();

    // 入力を送る側: 開いた文書が描かれたら、1 文字打ち、保存して、保存が終わったら終了する
    // 入力を送る側のスレッド (試験の駆動。製品のスレッドではない、deps-policy.toml の R11 の許可)
    #[allow(clippy::disallowed_methods)]
    let driver = std::thread::spawn(move || {
        let ascii = || {
            probe
                .last_ascii(AsciiOptions::default())
                .unwrap_or_default()
        };
        assert!(
            wait_until(Duration::from_secs(10), || probe.sink().is_some()
                && ascii().contains("hello")),
            "文書が描かれない"
        );
        let sink = probe.sink().expect("入力の口");
        sink.send(InputEvent::Text("x".into()));
        assert!(
            wait_until(Duration::from_secs(10), || ascii().contains("xhello")),
            "入力が描かれない"
        );
        sink.send(InputEvent::Key(KeyEvent::press(
            KeyCode::Char('s'),
            Modifiers::CTRL,
        )));
        // 保存の完了 (未保存の印が消える) を待ってから終了する
        let saved = wait_until(Duration::from_secs(10), || {
            let s = probe
                .last_ascii(AsciiOptions {
                    show_chrome: true,
                    ..AsciiOptions::default()
                })
                .unwrap_or_default();
            s.contains("a.txt") && !s.contains("[+]")
        });
        assert!(saved, "保存が終わらない");
        sink.send(InputEvent::Key(KeyEvent::press(
            KeyCode::Char('q'),
            Modifiers::CTRL,
        )));
    });

    let files = [InitialFile {
        path: "/w/a.txt".to_owned(),
        line: None,
        col: None,
    }];
    let code = host.run(&mut fe, &files, false);
    driver.join().expect("入力を送る側が失敗した");
    assert_eq!(code, ExitCode::OK, "正常に終了する");
    assert_eq!(
        fs.read("/w/a.txt").as_deref(),
        Some(&b"xhello\n"[..]),
        "保存した内容"
    );
    println!("no_network のシナリオ: 開く → 編集 → 保存 → 終了 (成功)");
}
