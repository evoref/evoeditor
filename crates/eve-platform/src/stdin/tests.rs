//! 標準入力の読取の試験 (docs/05 §9.1 / §9.2)。パイプ (`std::io::pipe`) の読み端を私有のハンドル / fd として使う。
//! 標準入力そのものの移し替え (`from_process_stdin`) は、この試験のバイナリを子プロセスとして起こして確かめる
//! (試験のプロセスの標準入力を書き換えないため)。

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use super::*;

fn private_of(r: std::io::PipeReader) -> os::Private {
    #[cfg(unix)]
    {
        os::Private::from_fd(r.into())
    }
    #[cfg(windows)]
    {
        os::Private::from_handle(r.into())
    }
}

/// 読み手から EOF まで読む (空のチャンク = `wake` の印は数える)。
fn read_all(r: &mut dyn ReadChunks) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let mut wakes = 0;
    while let Some(c) = r.next_chunk().expect("読める") {
        if c.is_empty() {
            wakes += 1;
        }
        out.extend_from_slice(c);
    }
    (out, wakes)
}

/// 書き端への書込が失敗する (読み手が閉じた) まで待つ。
fn wait_broken(w: &mut impl Write) -> Duration {
    let t = Instant::now();
    loop {
        if w.write_all(b"x").and_then(|()| w.flush()).is_err() {
            return t.elapsed();
        }
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "私有のハンドル / fd が閉じない"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn パイプを_eof_まで読む() {
    let (r, mut w) = std::io::pipe().expect("パイプ");
    let s = PlatformStdin::start(private_of(r)).expect("始める");
    w.write_all(b"hello\nworld\n").expect("書く");
    drop(w);
    let mut rd = s.take().expect("1 回目は取れる");
    assert!(s.take().is_none(), "2 回目は取れない");
    let (got, wakes) = read_all(&mut *rd);
    assert_eq!(got, b"hello\nworld\n");
    assert_eq!(wakes, 0);
}

#[test]
fn wake_は待ちを起こし_印は_1_つに畳む() {
    let (r, mut w) = std::io::pipe().expect("パイプ");
    let s = PlatformStdin::start(private_of(r)).expect("始める");
    let mut rd = s.take().expect("読み手");
    s.wake();
    s.wake();
    s.wake();
    assert_eq!(
        rd.next_chunk().expect("起きる"),
        Some(&[][..]),
        "空のチャンクで戻る"
    );
    w.write_all(b"abc").expect("書く");
    drop(w);
    let (got, wakes) = read_all(&mut *rd);
    assert_eq!(got, b"abc");
    assert_eq!(wakes, 0, "3 回の wake は 1 つに畳む");
    // 受けた後の wake はまた届く
    s.wake();
}

/// 読み手と、`eve-stdin` が終わったかの印 (試験だけが中を見る)。
fn take_with_finished(s: &PlatformStdin) -> (StdinChunks, Arc<AtomicBool>) {
    let rd = s
        .reader
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .expect("読み手");
    let finished = rd
        .stopper
        .as_ref()
        .map(|st| st.finished.clone())
        .expect("止める部品");
    (rd, finished)
}

/// `eve-stdin` が終わるまで待つ (書き手は何も書かない)。
fn wait_finished(finished: &AtomicBool) -> Duration {
    let t = Instant::now();
    while !finished.load(Ordering::Acquire) {
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "eve-stdin が終わらない"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    t.elapsed()
}

#[test]
fn 読み手を手放すと_eve_stdin_が止まって私有のハンドルを閉じる() {
    // 閉じないパイプ (書き手は開いたまま、何も書かない): eve-stdin は読取 (Unix は poll、Windows は ReadFile) で止まって
    // いる。読み手を手放すと OS ごとの方法 (Unix は停止の自己パイプ、Windows は CancelSynchronousIo) で起こされ、
    // 私有のハンドル / fd を閉じて終わる。書き手は何も書かないので、データで起きたのではない
    let (r, mut w) = std::io::pipe().expect("パイプ");
    let s = PlatformStdin::start(private_of(r)).expect("始める");
    let (rd, finished) = take_with_finished(&s);
    std::thread::sleep(Duration::from_millis(50));
    assert!(!finished.load(Ordering::Acquire), "読取で待っている");
    let t = Instant::now();
    drop(rd);
    let stop = t.elapsed();
    // Windows の繰り返しは eve-stdin が終われば戻る (最大の回数に届くと約 0.5 秒かかる)
    assert!(stop < Duration::from_millis(300), "{stop:?}");
    wait_finished(&finished);
    // 閉じた後の書き手の書込は失敗する (Unix は EPIPE、Windows は ERROR_NO_DATA)
    wait_broken(&mut w);
}

#[test]
fn データを読んだ後でも止められる() {
    let (r, mut w) = std::io::pipe().expect("パイプ");
    let s = PlatformStdin::start(private_of(r)).expect("始める");
    let mut rd = s.take().expect("読み手");
    w.write_all(b"first").expect("書く");
    assert_eq!(rd.next_chunk().expect("読める"), Some(&b"first"[..]));
    drop(rd);
    wait_broken(&mut w);
}

#[cfg(windows)]
#[test]
fn 標準入力のハンドルが無効なら最初の読取が失敗する() {
    let s = PlatformStdin::start(os::Private::none()).expect("始める");
    let mut rd = s.take().expect("読み手");
    assert!(matches!(rd.next_chunk(), Err(ChunkError::Io(_))));
    assert_eq!(rd.next_chunk().expect("以後は EOF"), None);
}

#[cfg(unix)]
mod unix_only {
    use super::*;
    use crate::stdin::unix::{Next, after_poll};

    #[test]
    fn poll_の結果の読み方() {
        assert_eq!(
            after_poll(libc::POLLIN, libc::POLLIN),
            Next::Stop,
            "止める指示を先に見る"
        );
        assert_eq!(after_poll(0, libc::POLLHUP), Next::Stop, "書き端を閉じた");
        assert_eq!(after_poll(libc::POLLIN, 0), Next::Read);
        assert_eq!(
            after_poll(libc::POLLHUP, 0),
            Next::Read,
            "書き手が閉じた: read で EOF を見る"
        );
        assert_eq!(after_poll(libc::POLLERR, 0), Next::Read);
        assert_eq!(
            after_poll(libc::POLLNVAL, 0),
            Next::Blocking,
            "macOS のデバイスの退路"
        );
        assert_eq!(after_poll(0, 0), Next::Wait);
    }

    #[test]
    fn pollnval_の退路はブロックする_read_で読む() {
        let (r, mut w) = std::io::pipe().expect("パイプ");
        let s =
            PlatformStdin::start_with(private_of(r), os::stop_pair().expect("停止のパイプ"), true)
                .expect("始める");
        w.write_all(b"device").expect("書く");
        drop(w);
        let mut rd = s.take().expect("読み手");
        let (got, _) = read_all(&mut *rd);
        assert_eq!(got, b"device");
    }

    #[test]
    fn dev_null_は読んで空() {
        let f = std::fs::File::open("/dev/null").expect("/dev/null");
        let s = PlatformStdin::start(os::Private::from_fd(f.into())).expect("始める");
        let mut rd = s.take().expect("読み手");
        let (got, _) = read_all(&mut *rd);
        assert_eq!(got, b"");
    }
}

#[test]
fn 取られないまま手放すときは止めるのを待たない() {
    // 閉じないパイプで読取に入ったまま、読み手を取らずに `PlatformStdin` を手放す (Host が使わずに終わる)
    let (r, mut w) = std::io::pipe().expect("パイプ");
    let s = PlatformStdin::start(private_of(r)).expect("始める");
    std::thread::sleep(Duration::from_millis(50));
    let t = Instant::now();
    drop(s);
    let took = t.elapsed();
    assert!(took < Duration::from_millis(100), "{took:?}");
    // 印を立て (Windows は CancelSynchronousIo を 1 回送り)、切り離した eve-stdin は自分で終わる
    wait_broken(&mut w);
}

/// 子プロセスの印 (この試験のバイナリを子として起こしたときに立てる環境変数。値は子の役割)。
pub(super) const CHILD_ENV: &str = "EVE_TEST_STDIN_CHILD";

/// 子の役割を確かめる試験の名前 (子として起こすときに `--exact` で選ぶ)。
const CHILD_TEST: &str = "stdin::tests::子_標準入力の試験の子プロセス";

/// この試験のバイナリを子として起こす (`mode` は子の役割)。標準出力は親が読む。
pub(super) fn spawn_child(
    test: &str,
    mode: &str,
    stdin: std::process::Stdio,
) -> std::process::Child {
    let exe = std::env::current_exe().expect("試験のバイナリ");
    std::process::Command::new(exe)
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, mode)
        .stdin(stdin)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("子を起こす")
}

/// 子の標準出力から `@@` の印の付いた行を 1 つ読み、印の後ろを返す。
fn child_line(r: &mut impl std::io::BufRead) -> String {
    let mut line = String::new();
    loop {
        line.clear();
        let n = r.read_line(&mut line).expect("子の出力");
        assert!(n > 0, "子が印の行を出さずに終わった");
        // 試験の名前の行 (`test … ... `) の続きに出ることがある
        if let Some((_, rest)) = line.trim_end().split_once("@@") {
            return rest.to_owned();
        }
    }
}

/// 子の終わりを待つ (上限つき)。
fn wait_child(child: &mut std::process::Child, limit: Duration) -> std::process::ExitStatus {
    let t = Instant::now();
    loop {
        if let Some(st) = child.try_wait().expect("子の状態") {
            return st;
        }
        if t.elapsed() > limit {
            let _ = child.kill();
            panic!("子が {limit:?} のうちに終わらない");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn say(msg: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "@@{msg}");
    let _ = out.flush();
}

/// 子: `CHILD_ENV` の役割に応じて、標準入力を私有のハンドル / fd へ移して確かめ、結果を標準出力に書く。親から起こされた
/// ときだけ動く。
#[test]
fn 子_標準入力の試験の子プロセス() {
    let Some(mode) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    match mode.to_str().unwrap_or("") {
        // 読み切る。標準入力の位置は空 (std の stdin は呼ぶたびに引き直すので、すぐ EOF)
        "read" => {
            let s = PlatformStdin::from_process_stdin().expect("移し替え");
            let mut b = [0u8; 16];
            let n = std::io::stdin().read(&mut b).expect("標準入力の位置を読む");
            let mut rd = s.take().expect("読み手");
            let (got, _) = read_all(&mut *rd);
            say(&format!(
                "got={} std_stdin={n}",
                String::from_utf8_lossy(&got)
            ));
        }
        // 孫を起こしてから読むのをやめる: 孫がパイプの読み手を持たなければ、親の書込が失敗する
        "grandchild" => {
            let s = PlatformStdin::from_process_stdin().expect("移し替え");
            let exe = std::env::current_exe().expect("試験のバイナリ");
            let mut grand = std::process::Command::new(exe)
                .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
                .env(CHILD_ENV, "sleep")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("孫を起こす");
            let rd = s.take().expect("読み手");
            drop(rd);
            say("stopped");
            // 親が書込の失敗を確かめる間、孫を生かしておく
            let _ = grand.wait();
        }
        "sleep" => std::thread::sleep(Duration::from_secs(3)),
        // 読取で止まったまま終わる (終了のときに eve-stdin を待たない、02 §9.6)
        "exit" => {
            let s = PlatformStdin::from_process_stdin().expect("移し替え");
            let rd = s.take().expect("読み手");
            std::thread::sleep(Duration::from_millis(100));
            say("exiting");
            std::mem::forget(rd);
            std::process::exit(0);
        }
        _ => {}
    }
}

#[test]
fn 標準入力を私有のハンドルへ移し_位置には空のものを置く() {
    if std::env::var_os(CHILD_ENV).is_some() {
        return;
    }
    let mut child = spawn_child(CHILD_TEST, "read", std::process::Stdio::piped());
    let mut si = child.stdin.take().expect("子の標準入力");
    si.write_all(b"piped").expect("書く");
    drop(si);
    let out = child.wait_with_output().expect("子の終わり");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("@@got=piped std_stdin=0"), "子の出力: {text}");
}

#[test]
fn 標準入力が空のもの_ファイルのときも読める() {
    if std::env::var_os(CHILD_ENV).is_some() {
        return;
    }
    // NUL / /dev/null: 読んで空 (理由の無い空の文書になる)
    let child = spawn_child(CHILD_TEST, "read", std::process::Stdio::null());
    let out = child.wait_with_output().expect("子の終わり");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("@@got= std_stdin=0"), "子の出力: {text}");
    // ファイル
    let path = std::env::temp_dir().join(format!("eve-stdin-test-{}.txt", std::process::id()));
    std::fs::write(&path, b"from file").expect("一時ファイル");
    let f = std::fs::File::open(&path).expect("開く");
    let child = spawn_child(CHILD_TEST, "read", std::process::Stdio::from(f));
    let out = child.wait_with_output().expect("子の終わり");
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("@@got=from file std_stdin=0"),
        "子の出力: {text}"
    );
}

#[test]
fn 孫プロセスにパイプの読み手が漏れない() {
    if std::env::var_os(CHILD_ENV).is_some() {
        return;
    }
    let mut child = spawn_child(CHILD_TEST, "grandchild", std::process::Stdio::piped());
    let mut w = child.stdin.take().expect("子の標準入力");
    let mut out = std::io::BufReader::new(child.stdout.take().expect("子の標準出力"));
    assert_eq!(child_line(&mut out), "stopped");
    // 孫 (3 秒眠る) が生きている間に、パイプの読み手が無くなって書込が失敗する
    let took = wait_broken(&mut w);
    assert!(took < Duration::from_secs(2), "{took:?}");
    drop(w);
    let _ = wait_child(&mut child, Duration::from_secs(10));
}

#[test]
fn 読取で止まったまま終了しても止まらない() {
    if std::env::var_os(CHILD_ENV).is_some() {
        return;
    }
    // 書き手は開いたまま何も書かない (閉じないパイプ)
    let mut child = spawn_child(CHILD_TEST, "exit", std::process::Stdio::piped());
    let si = child.stdin.take().expect("子の標準入力");
    let mut out = std::io::BufReader::new(child.stdout.take().expect("子の標準出力"));
    assert_eq!(child_line(&mut out), "exiting");
    let st = wait_child(&mut child, Duration::from_secs(5));
    assert!(st.success(), "{st:?}");
    drop(si);
}
