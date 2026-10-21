//! L5 端末 E2E (docs/09 §3.6): 実バイナリを ConPTY (portable-pty) で起動し、`vt100` で画面を解釈して検査する。
//! 遅く不安定になりやすいので最小限。生バイト列は比較しない (ConPTY は出力を再合成するため)。
//!
//! v0.1 の M2 で成り立つシナリオ: (1) 起動と終了、(2) 未保存の終了の確認ダイアログ、(3) 制御キーがアプリに届く、
//! (5) 端末の復元、(9) 貼り付けの安全。(7) のパニック注入は feature test-hooks のときだけ走る。

#![cfg(windows)]
// 検査の失敗は panic で伝える。
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

static STATE_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

const EXE: &str = env!("CARGO_BIN_EXE_evoeditor");
const COLS: u16 = 100;
const ROWS: u16 = 30;
/// 画面の変化を待つ上限 (失敗として扱う。時間の許容ではない)。
const WAIT: Duration = Duration::from_secs(15);

struct Session {
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    parser: Arc<Mutex<vt100::Parser>>,
    raw: Arc<Mutex<Vec<u8>>>,
    // pty を閉じないよう保持する (`close_console` で手放すと CTRL_CLOSE_EVENT が起きる)
    master: Option<Box<dyn MasterPty + Send>>,
    state: PathBuf,
}

impl Session {
    fn start(args: &[&str]) -> Session {
        Session::start_in(args, None)
    }

    /// `state` を指定すると、その状態ディレクトリで起動する (前の実行が残した退避・フラグを見せる)。
    fn start_in(args: &[&str], state: Option<PathBuf>) -> Session {
        Session::start_with(args, state, &[])
    }

    /// 環境変数を足して起動する (`EVE_TEST_HOOK` など)。
    fn start_with(args: &[&str], state: Option<PathBuf>, envs: &[(&str, &str)]) -> Session {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("ConPTY を開けません");
        let mut cmd = CommandBuilder::new(EXE);
        cmd.args(args);
        cmd.env_remove("NO_COLOR");
        cmd.env_remove("WT_SESSION");
        cmd.env_remove("TERM_PROGRAM");
        // 利用者の状態ディレクトリを汚さない (保存中フラグ・退避はここに書く)
        let state = state.unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "evo-e2e-state-{}-{}",
                std::process::id(),
                STATE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ))
        });
        cmd.env("EVE_STATE_DIR", &state);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let child = pty.slave.spawn_command(cmd).expect("起動できません");
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().expect("reader");
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pty.master.take_writer().expect("writer")));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(ROWS, COLS, 0)));
        let raw = Arc::new(Mutex::new(Vec::new()));
        {
            let (parser, raw, writer) = (parser.clone(), raw.clone(), writer.clone());
            // テストの駆動側の読み取りスレッド (製品のスレッドではない。R11 の対象外)
            #[allow(clippy::disallowed_methods)]
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let chunk = &buf[..n];
                    // ConPTY は起動時にカーソル位置を尋ねる (DSR)。端末として答える
                    if chunk.windows(4).any(|w| w == b"\x1b[6n")
                        && let Ok(mut w) = writer.lock()
                    {
                        let _ = w.write_all(b"\x1b[1;1R");
                        let _ = w.flush();
                    }
                    raw.lock().unwrap().extend_from_slice(chunk);
                    parser.lock().unwrap().process(chunk);
                }
            });
        }
        Session {
            child,
            writer,
            parser,
            raw,
            master: Some(pty.master),
            state,
        }
    }

    fn send(&self, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes).expect("入力を書けません");
        w.flush().expect("flush");
    }

    fn screen(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    fn row(&self, y: u16) -> String {
        let p = self.parser.lock().unwrap();
        p.screen().rows(0, COLS).nth(y as usize).unwrap_or_default()
    }

    /// 画面が条件を満たすまで待つ。満たさなければ画面を添えて失敗。
    fn wait_for(&self, what: &str, pred: impl Fn(&str) -> bool) {
        let t = Instant::now();
        while t.elapsed() < WAIT {
            if pred(&self.screen()) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("待っても {what} にならない。画面:\n{}", self.screen());
    }

    /// プロセスの終了を待つ。終了コードを返す。
    fn wait_exit(&mut self) -> u32 {
        let t = Instant::now();
        while t.elapsed() < WAIT {
            if let Some(st) = self.child.try_wait().expect("try_wait") {
                return st.exit_code();
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("終了しない。画面:\n{}", self.screen());
    }

    /// 疑似コンソールを閉じる (ウィンドウを閉じた状態。プロセスに `CTRL_CLOSE_EVENT` が届く、09 §3.6 (8b))。
    /// 閉じる呼出し (`ClosePseudoConsole`) は版によって子の終了を待ってブロックするので、別スレッドで呼ぶ。
    fn close_console(&mut self) {
        if let Some(m) = self.master.take() {
            #[allow(clippy::disallowed_methods)]
            thread::spawn(move || drop(m));
        }
    }

    /// 終了を待つ (上限つき)。終了しなければ `None`。
    fn exit_within(&mut self, limit: Duration) -> Option<u32> {
        let t = Instant::now();
        while t.elapsed() < limit {
            if let Some(st) = self.child.try_wait().expect("try_wait") {
                return Some(st.exit_code());
            }
            thread::sleep(Duration::from_millis(25));
        }
        None
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().expect("try_wait").is_none()
    }

    fn raw_text(&self) -> String {
        String::from_utf8_lossy(&self.raw.lock().unwrap()).into_owned()
    }

    /// 最初の画面 (ステータスバーが出る) まで待つ。
    fn wait_ready(&self) {
        self.wait_for("最初の画面", |s| s.contains("無題-1"));
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

const CTRL_Q: &[u8] = b"\x11";
const CTRL_C: &[u8] = b"\x03";
const CTRL_S: &[u8] = b"\x13";
const CTRL_Z: &[u8] = b"\x1a";

/// 文字を 1 つずつ (バーストにならないよう間隔をあけて) 送る。
fn type_slowly(s: &Session, text: &str) {
    for ch in text.chars() {
        let mut b = [0u8; 4];
        s.send(ch.encode_utf8(&mut b).as_bytes());
        thread::sleep(Duration::from_millis(40));
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("evo-e2e-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

// (1) 起動 → 終了 (未保存の変更なし)。(5) 端末の復元のシーケンスが出力に含まれる
#[test]
fn 起動して最初の画面が出て_ctrl_q_で終了コード_0() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    assert!(s.screen().contains("終了"), "ヒントバー:\n{}", s.screen());
    s.send(CTRL_Q);
    assert_eq!(s.wait_exit(), 0);
}

#[test]
fn 終了すると代替スクリーンを抜けカーソルを表示に戻す() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    s.send(CTRL_Q);
    assert_eq!(s.wait_exit(), 0);
    thread::sleep(Duration::from_millis(300));
    let raw = s.raw_text();
    assert!(raw.contains("\x1b[?1049h"), "代替スクリーンに入る");
    let leave = raw.rfind("\x1b[?1049l");
    assert!(leave.is_some(), "代替スクリーンを抜ける: {raw:?}");
    let tail = &raw[leave.unwrap()..];
    // 抜けた後にカーソルが非表示のままになっていない
    let hide = raw.rfind("\x1b[?25l");
    let show = raw.rfind("\x1b[?25h");
    assert!(
        show.is_some() && hide.is_none_or(|h| show.unwrap() > h),
        "カーソルを表示に戻す (末尾: {tail:?})"
    );
}

// (2) 未保存で Ctrl+Q → 確認ダイアログが見える。Tab → Enter と d のそれぞれで「破棄」を選べる
#[test]
fn 未保存で_ctrl_q_は確認ダイアログを出し_tab_enter_で破棄できる() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "abc");
    s.wait_for("入力が見える", |t| t.contains("abc"));
    s.send(CTRL_Q);
    s.wait_for("確認ダイアログ", |t| {
        t.contains("[S]") && t.contains("[D]")
    });
    assert!(s.running(), "ダイアログが出ている間は終了しない");
    s.send(b"\t");
    thread::sleep(Duration::from_millis(150));
    s.send(b"\r");
    assert_eq!(s.wait_exit(), 0);
}

#[test]
fn 未保存で_ctrl_q_の確認は_d_で破棄できる() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "abc");
    s.send(CTRL_Q);
    s.wait_for("確認ダイアログ", |t| t.contains("[D]"));
    s.send(b"d");
    assert_eq!(s.wait_exit(), 0);
}

#[test]
fn 確認ダイアログは_esc_でキャンセルして編集に戻る() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "abc");
    s.send(CTRL_Q);
    s.wait_for("確認ダイアログ", |t| t.contains("[D]"));
    s.send(b"\x1b");
    s.wait_for("ダイアログが閉じる", |t| !t.contains("[D]"));
    assert!(s.running());
    assert!(s.screen().contains("abc"));
    s.send(CTRL_Q);
    s.wait_for("再び確認", |t| t.contains("[D]"));
    s.send(b"d");
    assert_eq!(s.wait_exit(), 0);
}

// (3) raw モードで端末の制御キーがアプリに届く
#[test]
fn ctrl_c_はプロセスを殺さず_ctrl_z_は_undo_として届く() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    s.send(CTRL_C);
    thread::sleep(Duration::from_millis(500));
    assert!(s.running(), "Ctrl+C でプロセスが死なない");
    type_slowly(&s, "ab");
    s.wait_for("入力が見える", |t| t.contains("ab"));
    s.send(CTRL_Z);
    s.wait_for("Undo で消える", |t| !t.contains("ab"));
    assert!(s.running());
    s.send(CTRL_Q);
    assert_eq!(s.wait_exit(), 0);
}

#[test]
fn ctrl_s_はコマンドとして届き_無題なら名前の入力が出る() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "x");
    s.send(CTRL_S);
    // 無題の保存は名前を尋ねる (入力プロンプトのヒントは overlay 文脈: Esc が出る)
    s.wait_for("プロンプト", |t| t.contains("Esc"));
    assert!(
        !s.screen().contains("^Q"),
        "overlay 文脈のヒント:\n{}",
        s.screen()
    );
    s.send(b"\x1b");
    s.wait_for("プロンプトが閉じる", |t| {
        t.contains("^Q") || t.contains("Ctrl+Q")
    });
    s.send(CTRL_Q);
    s.wait_for("確認", |t| t.contains("[D]"));
    s.send(b"d");
    assert_eq!(s.wait_exit(), 0);
}

// ファイル引数で開く (日本語の内容とファイル名)
#[test]
fn 日本語のファイルを引数で開いて表示する() {
    let dir = temp_dir("open");
    let path = dir.join("日本語.txt");
    std::fs::write(&path, "こんにちは\nsecond\n").unwrap();
    let mut s = Session::start(&[path.to_str().unwrap()]);
    s.wait_for("本文", |t| {
        t.contains("こんにちは") && t.contains("second")
    });
    assert!(s.screen().contains("日本語.txt"), "{}", s.screen());
    s.send(CTRL_Q);
    assert_eq!(s.wait_exit(), 0, "変更なしなら確認なしで終了");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "こんにちは\nsecond\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

// (1) 起動 → 文字入力 → Ctrl+S → Ctrl+Q、ファイルの内容が一致する (無題は名前を尋ねる)
#[test]
fn 入力して保存して終了するとファイルの内容が一致する() {
    let dir = temp_dir("save");
    let path = dir.join("saved.txt");
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "hello");
    s.send(CTRL_S);
    s.wait_for("名前の入力", |t| t.contains("Esc"));
    s.send(path.to_str().unwrap().as_bytes());
    thread::sleep(Duration::from_millis(300));
    s.send(b"\r");
    s.wait_for("保存した", |t| {
        t.contains("saved.txt") && !t.contains("[+]")
    });
    s.send(CTRL_Q);
    assert_eq!(s.wait_exit(), 0, "保存済みなので確認なしで終了する");
    assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    let _ = std::fs::remove_dir_all(dir);
}

// (8b ii) コンソールを閉じる (CTRL_CLOSE): 未保存の内容を退避して終了する。退避は受信スレッドがメインを待たずに書く
#[test]
fn コンソールを閉じると未保存の内容を退避して終了する() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "unsaved text");
    s.wait_for("入力が見える", |t| t.contains("unsaved text"));
    let recovery = s.state.join("recovery");
    s.close_console();
    let code = s.exit_within(Duration::from_secs(10));
    assert!(
        code.is_some(),
        "コンソールを閉じたら終了する (CTRL_CLOSE は約 5 秒で強制終了される)"
    );
    // 退避ディレクトリ: <UTC timestamp>-<pid>/ に本文と manifest
    let dirs: Vec<_> = std::fs::read_dir(&recovery)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_name().to_string_lossy().contains('Z'))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(dirs.len(), 1, "退避は 1 件: {dirs:?} ({recovery:?})");
    let manifest = std::fs::read_to_string(dirs[0].join("manifest.toml")).unwrap();
    assert!(
        manifest.contains("reason = \"console_close\""),
        "{manifest}"
    );
    assert!(manifest.contains("complete = true"), "{manifest}");
    let body: Vec<_> = std::fs::read_dir(&dirs[0])
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".txt"))
        .collect();
    assert_eq!(body.len(), 1);
    assert_eq!(
        std::fs::read_to_string(body[0].path()).unwrap(),
        "unsaved text"
    );
    let _ = std::fs::remove_dir_all(&s.state);
}

// 起動時に、前の異常終了が残した退避の場所を案内する (msg.recovery.found、02 §11.2.1 の復旧 v0.1)
#[test]
fn 退避が残っていると起動時に場所を案内する() {
    let mut s1 = Session::start(&[]);
    s1.wait_ready();
    type_slowly(&s1, "lost work");
    s1.wait_for("入力が見える", |t| t.contains("lost work"));
    let state = s1.state.clone();
    s1.close_console();
    assert!(s1.exit_within(Duration::from_secs(10)).is_some());
    // 同じ状態ディレクトリで再び起動する
    let mut s2 = Session::start_in(&[], Some(state.clone()));
    // 本文は長いパスの手前で省略される (04 §8.9: 対処 (hint) を残し、本文の詳細から縮める。履歴 UI は v0.2)。案内が出ること自体を確かめる
    s2.wait_for("退避の案内", |t| {
        t.contains("異常終了の時に") && t.contains("そのフォルダを削除してください")
    });
    s2.send(CTRL_Q);
    assert_eq!(s2.wait_exit(), 0);
    let _ = std::fs::remove_dir_all(&state);
}

// 保存すると保存中フラグを書いて消す (05 §4.6): 状態ディレクトリにフラグが残らない
#[test]
fn 保存の後に保存中フラグが残らない() {
    let dir = temp_dir("flag");
    let path = dir.join("f.txt");
    std::fs::write(&path, "abc").unwrap();
    let mut s = Session::start(&[path.to_str().unwrap()]);
    s.wait_for("本文", |t| t.contains("abc"));
    type_slowly(&s, "x");
    s.send(CTRL_S);
    s.wait_for("保存した", |t| !t.contains("[+]"));
    let inflight = s.state.join("recovery").join("inflight");
    let leftovers: Vec<_> = std::fs::read_dir(&inflight)
        .map(|rd| {
            rd.flatten()
                .flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten())
                .collect()
        })
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    s.send(CTRL_Q);
    assert_eq!(s.wait_exit(), 0);
    let _ = std::fs::remove_dir_all(&s.state);
    let _ = std::fs::remove_dir_all(dir);
}

// 配布 exe のスモーク S3 に相当: 日本語のファイルを開く → 1 文字入力 → Ctrl+S → Ctrl+Q、バイト列が期待どおり
#[test]
fn 日本語のファイルを開いて編集して保存する() {
    let dir = temp_dir("save-jp");
    let path = dir.join("日本語.txt");
    std::fs::write(
        &path,
        "こんにちは
",
    )
    .unwrap();
    let mut s = Session::start(&[path.to_str().unwrap()]);
    s.wait_for("本文", |t| t.contains("こんにちは"));
    type_slowly(&s, "x");
    s.wait_for("変更あり", |t| t.contains("[+]"));
    s.send(CTRL_S);
    s.wait_for("保存した", |t| !t.contains("[+]"));
    s.send(CTRL_Q);
    assert_eq!(s.wait_exit(), 0);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        "xこんにちは
"
        .as_bytes()
    );
    // 一時ファイル・退避名が残らない
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["日本語.txt"], "{names:?}");
    let _ = std::fs::remove_dir_all(dir);
}

// 文字コード: Shift_JIS のファイルを判定して開く。`--encoding` の指定と、名前の誤り
#[test]
fn shift_jis_のファイルを開いて文字コードを表示する() {
    let dir = temp_dir("sjis");
    let path = dir.join("sjis.txt");
    let text = "日本語のテキストエディタです。これは文章の見本で、文字コードの判定に使います。
";
    let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode(text);
    std::fs::write(&path, &bytes).unwrap();
    let mut s = Session::start(&[path.to_str().unwrap()]);
    s.wait_for("本文と文字コード", |t| {
        t.contains("日本語のテキストエディタです") && t.contains("Shift_JIS")
    });
    s.send(CTRL_Q);
    assert_eq!(s.wait_exit(), 0);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes.as_ref(),
        "開いて閉じただけではバイトが変わらない"
    );
    // 指定して開く (UTF-8 と指定すると不正バイトを含む読み取り専用)
    let mut s = Session::start(&["--encoding", "utf-8", path.to_str().unwrap()]);
    s.wait_for("UTF-8 と読み取り専用", |t| {
        t.contains("UTF-8") && t.contains("RO")
    });
    s.send(CTRL_Q);
    assert_eq!(s.wait_exit(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn 未知の文字コードの指定は使い方の誤り() {
    let out = std::process::Command::new(EXE)
        .args(["--encoding", "latin1", "x.txt"])
        .output()
        .expect("起動");
    assert_eq!(out.status.code(), Some(2));
}

// (9) 貼り付けの安全: 本文の Ctrl+S (0x13) はコマンドにならず、文字として入る
#[test]
fn 貼り付けの本文の制御文字はコマンドとして実行されない() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    // 1 回の書込 (個別キーのバースト) で送る。0x13 を含む
    let mut body = Vec::new();
    body.extend_from_slice(b"head-");
    body.push(0x13);
    body.extend_from_slice(b"-middle-text-that-makes-a-long-burst-");
    body.extend_from_slice(b"\x1b[201~");
    body.extend_from_slice(b"-tail");
    s.send(&body);
    s.wait_for("本文が入る", |t| {
        t.contains("head-") && t.contains("-tail")
    });
    // 保存の入力プロンプトは出ない (Ctrl+S がコマンドにならなかった)。ヒントバーは normal 文脈のまま
    let hint = s.row(ROWS - 1);
    assert!(
        hint.contains("^Q") || hint.contains("Ctrl+Q"),
        "overlay になっていない: {hint:?}"
    );
    assert!(s.running());
    s.send(CTRL_Q);
    s.wait_for("確認", |t| t.contains("[D]"));
    s.send(b"d");
    assert_eq!(s.wait_exit(), 0);
}

// 配布 exe のスモーク S2: --version (pty 不要)
#[test]
fn version_は版を出して終了コード_0() {
    let out = std::process::Command::new(EXE)
        .arg("--version")
        .output()
        .expect("起動");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.trim() == format!("evoeditor {}", env!("CARGO_PKG_VERSION")),
        "{text:?}"
    );
}

#[test]
fn 使い方の誤りは終了コード_2() {
    let out = std::process::Command::new(EXE)
        .arg("--nope")
        .output()
        .expect("起動");
    assert_eq!(out.status.code(), Some(2));
}

// (7) internal.debug_panic (test-hooks): パニック → 端末復元 + 退避 + クラッシュログ。stderr は 04 §15.2 の順。
// 実行: cargo test -p evoeditor --features test-hooks --test e2e
#[cfg(feature = "test-hooks")]
#[test]
fn パニックすると端末を戻し_未保存の内容を退避してクラッシュログを残す() {
    let mut s = Session::start_with(&[], None, &[("EVE_TEST_HOOK", "panic_command")]);
    s.wait_ready();
    type_slowly(&s, "unsaved text");
    s.wait_for("入力が見える", |t| t.contains("unsaved text"));
    s.send(b"[24~"); // F12 = internal.debug_panic
    assert_eq!(s.wait_exit(), 70, "パニックの終了コード");
    let raw = s.raw_text();
    let pos = |needle: &str| {
        raw.find(needle)
            .unwrap_or_else(|| panic!("{needle} が無い: {raw}"))
    };
    let leave = pos("[?1049l");
    let header = pos("予期しないエラーのため");
    let saved = pos("保存されていない変更を退避しました");
    let next = pos("次に evoeditor を起動したときに");
    let crash = pos("クラッシュログ:");
    let report = pos("問題を報告するときは");
    assert!(
        leave < header && header < saved && saved < next && next < crash && crash < report,
        "端末の復元 → header → 退避 → next → crash_log → report の順: {raw}"
    );
    // 退避: 本文がそのまま、manifest が panic
    let recovery = s.state.join("recovery");
    let dirs: Vec<_> = std::fs::read_dir(&recovery)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains('Z'))
        .map(|e| e.path())
        .collect();
    assert_eq!(dirs.len(), 1, "{dirs:?}");
    let manifest = std::fs::read_to_string(dirs[0].join("manifest.toml")).unwrap();
    assert!(manifest.contains("reason = \"panic\""), "{manifest}");
    assert!(manifest.contains("complete = true"), "{manifest}");
    let body: Vec<_> = std::fs::read_dir(&dirs[0])
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".txt"))
        .collect();
    assert_eq!(body.len(), 1);
    assert_eq!(
        std::fs::read_to_string(body[0].path()).unwrap(),
        "unsaved text"
    );
    // クラッシュログ: パニックの場所はあるが、文書の内容は含まない
    let logs: Vec<_> = std::fs::read_dir(s.state.join("log"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(logs.len(), 1);
    let log = std::fs::read_to_string(logs[0].path()).unwrap();
    assert!(
        log.contains("internal.debug_panic") && !log.contains("unsaved text"),
        "{log}"
    );
    let _ = std::fs::remove_dir_all(&s.state);
}

/// 補助プロセス `eve-test-helpers` の場所 (evoeditor と同じ出力ディレクトリ)。無ければ cargo で作る。
fn helper_exe() -> PathBuf {
    let dir = std::path::Path::new(EXE).parent().unwrap().to_path_buf();
    let exe = dir.join("eve-test-helpers.exe");
    if !exe.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let ok = std::process::Command::new(cargo)
            .args(["build", "-p", "eve-test-helpers"])
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok && exe.exists(), "eve-test-helpers を作れません");
    }
    exe
}

/// 疑似コンソールの evoeditor に Ctrl+Break を送り (Ctrl+C は raw モードでは conhost が配らず入力になるので、ここでは扱わない)、未保存の内容を退避して終了コード 130 で終わることを確かめる (09 §3.6 (8))。
fn コンソールイベントで退避して終了する(kind: &str) {
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "unsaved text");
    s.wait_for("入力が見える", |t| t.contains("unsaved text"));
    let pid = s.child.process_id().expect("pid");
    let st = std::process::Command::new(helper_exe())
        .args(["console-event", &pid.to_string(), kind])
        .status()
        .unwrap();
    assert!(st.success(), "console-event: {st:?}");
    assert_eq!(s.exit_within(Duration::from_secs(5)), Some(130));
    let dirs: Vec<_> = std::fs::read_dir(s.state.join("recovery"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains('Z'))
        .map(|e| e.path())
        .collect();
    assert_eq!(dirs.len(), 1, "{dirs:?}");
    let manifest = std::fs::read_to_string(dirs[0].join("manifest.toml")).unwrap();
    assert!(manifest.contains("reason = \"signal\""), "{manifest}");
    assert!(manifest.contains("complete = true"), "{manifest}");
    let body: Vec<_> = std::fs::read_dir(&dirs[0])
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".txt"))
        .collect();
    assert_eq!(body.len(), 1);
    assert_eq!(
        std::fs::read_to_string(body[0].path()).unwrap(),
        "unsaved text"
    );
    assert!(s.raw_text().contains("[?1049l"), "端末を戻した");
    let _ = std::fs::remove_dir_all(&s.state);
}

#[test]
fn ctrl_break_を受けると未保存の内容を退避して終了する() {
    コンソールイベントで退避して終了する("ctrl-break");
}
