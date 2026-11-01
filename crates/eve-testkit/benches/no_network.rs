//! NFR-501 AC-2 (v0.2) の検査の本体: ヘッドレスのシナリオ 1 本 (開く → 編集 → 保存 → 終了) を、本番と同じ Host の実行ループ
//! (`Host::run`)・本物の時計 (`SystemClock`)・本物のワーカースレッド (`ThreadJobSpawner`) で動かす。`cargo xtask no-network`
//! がこのプログラムを `strace -f -e trace=network` の下で走らせ、ネットワーク系のシステムコールが 0 であることを確かめる
//! (docs/09 §3.6 の「無通信」)。
//!
//! FS とパスは本物 (Linux は eve-platform の `UnixFs` / `UnixPaths`、Windows は `WinFs` / `WinPaths`): 一時ディレクトリの
//! ファイルを開いて、原子的保存で書く (v0.2 の M7。それまでは偽物の `MemFs` で FS の経路を除いていた)。
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
use eve_platform::dirs::Dirs;
#[cfg(unix)]
use eve_platform::dirs::UnixPaths as NativePaths;
#[cfg(windows)]
use eve_platform::dirs::WinPaths as NativePaths;
#[cfg(windows)]
use eve_platform::fs::WinFs as NativeFs;
use eve_platform::path::PathCtx;
#[cfg(unix)]
use eve_platform::unix_fs::UnixFs as NativeFs;
use eve_testkit::{AsciiOptions, HeadlessFrontend};
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
    // 作業ディレクトリ (開くファイル) と状態ディレクトリ (保存中フラグ) は一時ディレクトリの下に作る
    let work = std::env::temp_dir().join(format!("eve-no-network-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("作業ディレクトリを作れる");
    let file = work.join("a.txt");
    std::fs::write(&file, b"hello\n").expect("ファイルを書ける");
    let fs = Arc::new(NativeFs::new(work.join("state")));
    // パスも本物 (Windows のパスを解釈できる)。環境変数は読まない (開発者の設定に依らない)
    let ctx = PathCtx {
        cwd: work.to_string_lossy().into_owned(),
        home: None,
    };
    #[cfg(windows)]
    let paths = Arc::new(NativePaths::new(Dirs::resolve(&|_| None), ctx));
    #[cfg(unix)]
    let paths = Arc::new(NativePaths::new(
        Dirs::resolve_unix(&|_| None, None),
        ctx,
        None,
    ));
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
        let chrome = || {
            probe
                .last_ascii(AsciiOptions {
                    show_chrome: true,
                    ..AsciiOptions::default()
                })
                .unwrap_or_default()
        };
        // 待ちが間に合わなければ、最後の画面を出して終わる (panic ではメインの実行ループが終わらず、止まったままになる)
        let expect = |ok: bool, what: &str| {
            if !ok {
                eprintln!("{what}。最後の画面:\n{}", chrome());
                std::process::exit(1);
            }
        };
        expect(
            wait_until(Duration::from_secs(10), || {
                probe.sink().is_some() && ascii().contains("hello")
            }),
            "文書が描かれない",
        );
        let sink = probe.sink().expect("入力の口");
        sink.send(InputEvent::Text("x".into()));
        expect(
            wait_until(Duration::from_secs(10), || ascii().contains("xhello")),
            "入力が描かれない",
        );
        sink.send(InputEvent::Key(KeyEvent::press(
            KeyCode::Char('s'),
            Modifiers::CTRL,
        )));
        // 保存の完了 (未保存の印が消える) を待ってから終了する
        expect(
            wait_until(Duration::from_secs(10), || {
                let s = chrome();
                s.contains("a.txt") && !s.contains("[+]")
            }),
            "保存が終わらない",
        );
        sink.send(InputEvent::Key(KeyEvent::press(
            KeyCode::Char('q'),
            Modifiers::CTRL,
        )));
    });

    let files = [InitialFile {
        path: file.to_string_lossy().into_owned(),
        line: None,
        col: None,
    }];
    let code = host.run(&mut fe, &files, false);
    driver.join().expect("入力を送る側が失敗した");
    assert_eq!(code, ExitCode::OK, "正常に終了する");
    assert_eq!(
        std::fs::read(&file).expect("保存したファイルを読める"),
        b"xhello\n",
        "保存した内容"
    );
    let _ = std::fs::remove_dir_all(&work);
    println!("no_network のシナリオ: 開く → 編集 → 保存 → 終了 (成功)");
}
