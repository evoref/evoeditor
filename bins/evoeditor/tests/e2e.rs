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

/// ビルドしたままの exe。`helper_exe` などの、隣のファイルを探す用。
const BUILT: &str = env!("CARGO_BIN_EXE_evoeditor");

/// 検査する exe。環境変数 `EVE_EXE` があればそれ (配布 zip から展開した exe のスモーク、docs/09 §3.6 / §9.2 手順 6)。
fn exe() -> String {
    std::env::var("EVE_EXE").unwrap_or_else(|_| BUILT.to_owned())
}
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
        let mut cmd = CommandBuilder::new(exe());
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
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            std::num::NonZeroU16::new(ROWS).expect("ROWS は 1 以上"),
            std::num::NonZeroU16::new(COLS).expect("COLS は 1 以上"),
            0,
        )));
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

    /// 端末の大きさを変える (ConPTY と、画面を解釈する側の両方)。
    fn resize(&self, cols: u16, rows: u16) {
        self.master
            .as_ref()
            .expect("pty がある")
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("リサイズ");
        self.parser.lock().unwrap().screen_mut().set_size(
            std::num::NonZeroU16::new(rows).expect("rows は 1 以上"),
            std::num::NonZeroU16::new(cols).expect("cols は 1 以上"),
        );
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

    /// 出力（生）に `needle` が現れるまで待つ。終了の直後は読み取りスレッドがまだ残りを読んでいることがある。
    fn wait_raw(&self, needle: &str) {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if self.raw_text().contains(needle) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
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

// verifies: AC-FR-609-1
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
// verifies: AC-FR-609-1
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

// verifies: AC-FR-609-1
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

// 設定ディレクトリが無い環境でも起動・編集・保存・終了ができ、設定ディレクトリは作られない (FR-702 AC-1、07 §4)
// verifies: AC-FR-702-1
#[test]
fn 設定ディレクトリが無くても使え_設定ディレクトリは作られない() {
    let dir = temp_dir("nocfg");
    let missing = dir.join("no-such-config-dir");
    let path = dir.join("saved.txt");
    let mut s = Session::start_with(&[], None, &[("EVE_CONFIG_DIR", missing.to_str().unwrap())]);
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
    assert_eq!(s.wait_exit(), 0);
    assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    assert!(!missing.exists(), "設定ディレクトリは作らない");
    let _ = std::fs::remove_dir_all(dir);
}

// (8b ii) コンソールを閉じる (CTRL_CLOSE): 未保存の内容を退避して終了する。退避は受信スレッドがメインを待たずに書く
// verifies: AC-FR-107-2
// verifies: AC-NFR-302-2
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
    // 不正バイトが多いので確認が出る (03 §8.3): 読み取り専用で開く
    s.wait_for("不正バイトの確認", |t| {
        t.contains("不正なバイトが多いファイル")
    });
    s.send(b"r");
    s.wait_for("UTF-8 と読み取り専用", |t| {
        t.contains("UTF-8") && t.contains("RO")
    });
    s.send(CTRL_Q);
    assert_eq!(s.wait_exit(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn 未知の文字コードの指定は使い方の誤り() {
    let out = std::process::Command::new(exe())
        .args(["--encoding", "latin1", "x.txt"])
        .output()
        .expect("起動");
    assert_eq!(out.status.code(), Some(2));
}

// (9) 貼り付けの安全: 本文の Ctrl+S (0x13) はコマンドにならず、文字として入る
// verifies: AC-FR-206-2
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
    let out = std::process::Command::new(exe())
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
    let out = std::process::Command::new(exe())
        .arg("--nope")
        .output()
        .expect("起動");
    assert_eq!(out.status.code(), Some(2));
}

// (7) internal.debug_panic (test-hooks): パニック → 端末復元 + 退避 + クラッシュログ。stderr は 04 §15.2 の順。
// 実行: cargo test -p evoeditor --features test-hooks --test e2e
// verifies: AC-NFR-302-1
// verifies: AC-NFR-504-1
#[cfg(feature = "test-hooks")]
#[test]
fn パニックすると端末を戻し_未保存の内容を退避してクラッシュログを残す() {
    let mut s = Session::start_with(&[], None, &[("EVE_TEST_HOOK", "panic_command")]);
    s.wait_ready();
    type_slowly(&s, "unsaved text");
    s.wait_for("入力が見える", |t| t.contains("unsaved text"));
    s.send(b"[24~"); // F12 = internal.debug_panic
    assert_eq!(s.wait_exit(), 70, "パニックの終了コード");
    s.wait_raw("問題を報告するときは");
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
    // 端末に出した案内にも本文は含まない (退避の場所だけを示す)
    assert!(!raw.contains("unsaved text"), "{raw}");
    let _ = std::fs::remove_dir_all(&s.state);
}

/// 補助プロセス `eve-test-helpers` の場所 (evoeditor と同じ出力ディレクトリ)。無ければ cargo で作る。
fn helper_exe() -> PathBuf {
    let dir = std::path::Path::new(BUILT).parent().unwrap().to_path_buf();
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
    s.wait_raw("[?1049l");
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

// 外部からの CTRL_C_EVENT は、端末を生の入力モード (ENABLE_PROCESSED_INPUT なし、04 §6.1) にしているので届かない。
// キーの Ctrl+C はコマンドとして届く (`ctrl_c_はプロセスを殺さず_ctrl_z_は_undo_として届く`)。外部からの終了は CTRL_BREAK と CTRL_CLOSE。
// verifies: AC-NFR-302-2
#[test]
fn ctrl_break_を受けると未保存の内容を退避して終了する() {
    コンソールイベントで退避して終了する("ctrl-break");
}

// (7) recovery_timeout (test-hooks): 退避の段の予算が尽きても、打ち切りの manifest (complete = false) を残し、
// 端末復元・クラッシュログ・終了コード 70 までは進む。
#[cfg(feature = "test-hooks")]
#[test]
fn パニックの退避が時間切れでも終了コードとクラッシュログまで進む() {
    let mut s = Session::start_with(
        &[],
        None,
        &[("EVE_TEST_HOOK", "panic_command,recovery_timeout=0")],
    );
    s.wait_ready();
    type_slowly(&s, "unsaved text");
    s.wait_for("入力が見える", |t| t.contains("unsaved text"));
    s.send(b"\x1b[24~");
    assert_eq!(s.wait_exit(), 70);
    s.wait_raw("問題を報告するときは");
    let raw = s.raw_text();
    assert!(raw.contains("予期しないエラーのため"), "{raw}");
    assert!(raw.contains("クラッシュログ:"), "{raw}");
    let dirs: Vec<_> = std::fs::read_dir(s.state.join("recovery"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains('Z'))
        .map(|e| e.path())
        .collect();
    assert_eq!(dirs.len(), 1, "{dirs:?}");
    let manifest = std::fs::read_to_string(dirs[0].join("manifest.toml")).unwrap();
    assert!(manifest.contains("complete = false"), "{manifest}");
    assert!(
        !manifest.contains("written = true"),
        "時間切れなので本文は書けていない: {manifest}"
    );
    let _ = std::fs::remove_dir_all(&s.state);
}

// (8) メインが止まった状態の Ctrl+Break (test-hooks `main_stall`): 受信スレッドが退避を書き、2 秒待ってもメインが終わらないので
// 端末を緊急に復元して終了コード 130 で終わる (02 §9.6 手順 4、合計 5 s の内)。
#[cfg(feature = "test-hooks")]
#[test]
fn メインが止まっていても_ctrl_break_で端末を戻して終了する() {
    let mut s = Session::start_with(&[], None, &[("EVE_TEST_HOOK", "main_stall")]);
    s.wait_ready();
    type_slowly(&s, "unsaved text");
    s.wait_for("入力が見える", |t| t.contains("unsaved text"));
    let pid = s.child.process_id().expect("pid");
    let sent = Instant::now();
    let st = std::process::Command::new(helper_exe())
        .args(["console-event", &pid.to_string(), "ctrl-break"])
        .status()
        .unwrap();
    assert!(st.success(), "console-event: {st:?}");
    let code = s.exit_within(Duration::from_secs(8));
    let elapsed = sent.elapsed();
    assert_eq!(code, Some(130), "{elapsed:?}");
    assert!(
        elapsed >= Duration::from_secs(2) && elapsed < Duration::from_millis(5500),
        "メインの終了を 2 秒待ち、合計 5 秒の内で終わる: {elapsed:?}"
    );
    s.wait_raw("\x1b[?1049l");
    assert!(s.raw_text().contains("\x1b[?1049l"), "端末を戻した");
    let dirs: Vec<_> = std::fs::read_dir(s.state.join("recovery"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains('Z'))
        .map(|e| e.path())
        .collect();
    assert_eq!(dirs.len(), 1, "{dirs:?}");
    let manifest = std::fs::read_to_string(dirs[0].join("manifest.toml")).unwrap();
    assert!(manifest.contains("complete = true"), "{manifest}");
    let _ = std::fs::remove_dir_all(&s.state);
}

// (8b iii) メインが止まった状態のコンソールの閉鎖 (test-hooks `main_stall`): 受信スレッドが退避を書く。メインを待たずに
// 書き終えるので、CTRL_CLOSE の強制終了 (約 5 秒) の内に本文が残る (02 §9.6 手順 2)。
// verifies: AC-NFR-302-3
#[cfg(feature = "test-hooks")]
#[test]
fn メインが止まっていてもコンソールを閉じると未保存の内容を退避して終了する() {
    let mut s = Session::start_with(&[], None, &[("EVE_TEST_HOOK", "main_stall")]);
    s.wait_ready();
    type_slowly(&s, "unsaved text");
    s.wait_for("入力が見える", |t| t.contains("unsaved text"));
    let recovery = s.state.join("recovery");
    let closed = Instant::now();
    s.close_console();
    let code = s.exit_within(Duration::from_secs(10));
    assert!(
        code.is_some(),
        "強制終了までに終わる: {:?}",
        closed.elapsed()
    );
    assert!(
        closed.elapsed() < Duration::from_millis(4500),
        "受信から 4.5 秒の内に退避が書かれて終わる: {:?}",
        closed.elapsed()
    );
    let dirs: Vec<_> = std::fs::read_dir(&recovery)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_name().to_string_lossy().contains('Z'))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(dirs.len(), 1, "退避は 1 件: {dirs:?}");
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

// 設定ファイルの問題は、最初の画面の後にメッセージ行へ出し、全件をログ (state_dir/log/evoeditor.log) に書く (07 §4.3、05 §7.3)
#[test]
fn 設定の問題はメッセージ行に出てログに書かれる() {
    let cfg = temp_dir("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(
        cfg.join("config.toml"),
        "[editor]
tab_width = 0
indent_width = 99
[search]
case = \"smart\"
",
    )
    .unwrap();
    let mut s = Session::start_with(&[], None, &[("EVE_CONFIG_DIR", cfg.to_str().unwrap())]);
    s.wait_ready();
    s.wait_for("設定の問題の要約", |t| t.contains("設定に 2 件の"));
    // ログ: 警告 2 件が書かれる (未対応の情報診断は既定のレベル warn では書かれない)
    let log = s.state.join("log").join("evoeditor.log");
    s.send(b"");
    let _ = s.wait_exit();
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(
        text.lines().filter(|l| l.contains(" WARN ")).count(),
        2,
        "{text}"
    );
    assert!(text.contains("editor.tab_width"), "{text}");
    assert!(!text.contains("search"), "{text}");
    let _ = std::fs::remove_dir_all(&s.state);
    let _ = std::fs::remove_dir_all(&cfg);
}

// ---- システムクリップボード (05 §8、09 §3.6 / §3.4 の clip-delay)。実クリップボードを使うので、プロセスをまたいで直列にする ----

/// クリップボードを使うテストの排他 (ファイルの排他作成。nextest の別プロセスの間でも直列にする)。
#[cfg(feature = "test-hooks")]
struct ClipLock(PathBuf);

#[cfg(feature = "test-hooks")]
impl ClipLock {
    fn acquire() -> ClipLock {
        let path = std::env::temp_dir().join("evo-e2e-clipboard.lock");
        let t = Instant::now();
        loop {
            if std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .is_ok()
            {
                return ClipLock(path);
            }
            // 異常終了で残った古い鎖は 60 秒で捨てる
            if let Ok(m) = std::fs::metadata(&path)
                && m.modified()
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|e| e > Duration::from_secs(60))
            {
                let _ = std::fs::remove_file(&path);
            }
            assert!(
                t.elapsed() < Duration::from_secs(120),
                "クリップボードの排他を取れない"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }
}

#[cfg(feature = "test-hooks")]
impl Drop for ClipLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// 補助プロセスを起動し、`ready` の行が出るまで待つ。
#[cfg(feature = "test-hooks")]
fn spawn_clip_helper(args: &[&str]) -> std::process::Child {
    use std::io::BufRead;
    use std::process::Stdio;
    let mut child = std::process::Command::new(helper_exe())
        .args(args)
        .stdout(Stdio::piped())
        .spawn()
        .expect("clip 補助プロセス");
    let out = child.stdout.take().unwrap();
    let mut line = String::new();
    std::io::BufReader::new(out).read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "ready", "補助プロセスの準備");
    child
}

/// `clip-read` の出力 (読めるまで 3 秒待つ)。
#[cfg(feature = "test-hooks")]
fn clip_read() -> String {
    let t = Instant::now();
    loop {
        let out = std::process::Command::new(helper_exe())
            .arg("clip-read")
            .output()
            .unwrap();
        if out.status.success() {
            return String::from_utf8_lossy(&out.stdout).into_owned();
        }
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "クリップボードを読めない"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(feature = "test-hooks")]
// verifies: AC-FR-603-1
#[test]
fn コピーは実際のクリップボードに入り_貼り付けはそこから読む() {
    let _lock = ClipLock::acquire();
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "hello 日本語");
    s.send(b"\x01"); // Ctrl+A
    s.send(b"\x03"); // Ctrl+C
    thread::sleep(Duration::from_millis(500));
    assert_eq!(clip_read(), "hello 日本語");
    // 別のアプリがクリップボードへ入れたものを貼り付ける (遅延なしの遅延レンダリング)
    let mut helper = spawn_clip_helper(&[
        "clip-delay",
        "--delay-render",
        "0",
        "--text",
        "FROM-OTHER-APP",
        "--lifetime",
        "10000",
    ]);
    s.send(b"\x1b[F"); // End
    s.send(b"\x16"); // Ctrl+V
    s.wait_for("貼り付けが見える", |t| t.contains("FROM-OTHER-APP"));
    let _ = helper.kill();
    let _ = helper.wait();
    s.send(CTRL_Q);
    let _ = s.exit_within(Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&s.state);
}

#[cfg(feature = "test-hooks")]
#[test]
fn 取得が遅くても入力は止まらず_2_秒で内部レジスタに切り替わる() {
    let _lock = ClipLock::acquire();
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "REG");
    s.send(b"\x01\x03"); // Ctrl+A, Ctrl+C: 内部レジスタに入る
    thread::sleep(Duration::from_millis(500));
    s.send(b"\x1b[F");
    // 他のアプリが遅延レンダリングで所有し、応答を 6 秒遅らせる
    let mut helper = spawn_clip_helper(&[
        "clip-delay",
        "--delay-render",
        "6000",
        "--text",
        "SLOW",
        "--lifetime",
        "15000",
    ]);
    let t = Instant::now();
    s.send(b"\x16"); // Ctrl+V
    // 待つ間も入力を受け付ける (メインスレッドを止めない)
    thread::sleep(Duration::from_millis(300));
    type_slowly(&s, "x");
    s.wait_for("待つ間の入力が見える", |t| t.contains("REGx"));
    assert!(
        t.elapsed() < Duration::from_millis(1800),
        "入力は取得を待たずに反映される"
    );
    // 2 秒で内部レジスタ (REG) に切り替わる
    s.wait_for("内部レジスタの貼り付け", |t| {
        t.contains("REGxREG")
    });
    assert!(
        t.elapsed() >= Duration::from_millis(1800),
        "切り替えは約 2 秒後"
    );
    let _ = helper.kill();
    let _ = helper.wait();
    s.send(CTRL_Q);
    let _ = s.exit_within(Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&s.state);
}

#[cfg(feature = "test-hooks")]
#[test]
fn クリップボードを他のプロセスが握っていると内部レジスタに切り替える() {
    let _lock = ClipLock::acquire();
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "REG");
    s.send(b"\x01\x03");
    thread::sleep(Duration::from_millis(500));
    s.send(b"\x1b[F");
    // クリップボードを開いたまま 4 秒保持する (開く再試行が尽きて Unavailable)
    let mut helper = spawn_clip_helper(&["clip-delay", "--hold", "4000"]);
    s.send(b"\x16");
    s.wait_for("内部レジスタの貼り付け", |t| {
        t.contains("REGREG")
    });
    let _ = helper.kill();
    let _ = helper.wait();
    s.send(CTRL_Q);
    let _ = s.exit_within(Duration::from_secs(5));
    let _ = std::fs::remove_dir_all(&s.state);
}

// 計測用ビルド (docs/09 §3.6): 隠しオプションで最初の Frame の直後に終了し、区間の記録を書く
#[cfg(feature = "bench-trace")]
#[test]
fn 計測用ビルドは最初の_frame_の直後に終了し_区間の記録を書く() {
    let dir = temp_dir("bench-trace");
    let trace = dir.join("trace.jsonl");
    let mut s = Session::start_with(
        &["--bench-exit-after-first-frame"],
        None,
        &[("EVE_TRACE_FILE", trace.to_str().unwrap())],
    );
    assert_eq!(s.wait_exit(), 0);
    let text = std::fs::read_to_string(&trace).expect("記録が書かれる");
    assert!(text.contains("\"name\":\"startup.first_frame\""), "{text}");
    assert!(text.contains("\"wall_end_unix_ns\":"), "{text}");
    assert!(!text.contains("無題"), "本文・パスを記録に入れない");
}

// 配布ビルドには隠しオプションが無く、未知のオプションとして終了コード 2 (xtask check-dist も確かめる)
#[cfg(not(feature = "bench-trace"))]
#[test]
fn 配布ビルドでは計測用の隠しオプションは未知のオプション() {
    let out = std::process::Command::new(exe())
        .arg("--bench-exit-after-first-frame")
        .output()
        .expect("起動");
    assert_eq!(out.status.code(), Some(2));
}

// 検索バー (docs/04 §8): `Ctrl+F` で検索バーが画面に出て、入力した語と一致数が見える。`Esc` で閉じて編集に戻る
// (端末の編集面の大きさと Frame の大きさが、バーの行数ぶん違うことを、描画が許す。以前は描画が止まって画面が固まった)
// verifies: AC-FR-603-1
#[test]
fn ctrl_f_で検索バーが出て_入力した語と一致数が見える() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "hello fox");
    s.send(b""); // Ctrl+F
    s.wait_for("検索バー", |t| t.contains("検索:"));
    s.send(b"fox");
    s.wait_for("入力した語と一致数", |t| {
        t.contains("検索: fox") && t.contains("1/1")
    });
    s.send(b""); // Esc
    s.wait_for("バーが閉じる", |t| !t.contains("検索:"));
    // 編集に戻っている
    type_slowly(&s, "!");
    s.wait_for("文字が入る", |t| t.contains("hello fox!"));
    s.send(CTRL_Q);
    s.send(b"d");
    assert_eq!(s.wait_exit(), 0);
}

// 置換欄つきの検索バー (`Ctrl+R`) も、画面に出る
#[test]
fn ctrl_r_で置換欄つきの検索バーが出る() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "hello fox");
    s.send(b""); // Ctrl+R
    s.wait_for("検索欄と置換欄", |t| {
        t.contains("検索:") && t.contains("置換:")
    });
    s.send(b"");
    s.wait_for("バーが閉じる", |t| !t.contains("置換:"));
    s.send(CTRL_Q);
    s.send(b"d");
    assert_eq!(s.wait_exit(), 0);
}

// verifies: AC-FR-503-1
// verifies: AC-FR-504-1
#[test]
fn 起動直後の画面にステータスバーの項目とヒントバーのキーが出る() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    let t = s.screen();
    // ステータスバー: ファイル名・位置・文字コード・改行コード
    for want in ["無題-1", "行 1, 桁 1", "UTF-8", "LF"] {
        assert!(
            t.contains(want),
            "{want} が無い:
{t}"
        );
    }
    // ヒントバー: 終了・コマンド (パレット)・保存・検索のキー (FR-504)
    for want in ["^Q", "終了", "コマンド", "保存", "検索"] {
        assert!(
            t.contains(want),
            "{want} が無い:
{t}"
        );
    }
    s.send(CTRL_Q);
    assert_eq!(s.wait_exit(), 0);
}

// verifies: AC-FR-512-2
#[test]
fn 連続してリサイズしても最後の大きさの画面は全再描画した画面と一致する() {
    let mut s = Session::start(&[]);
    s.wait_ready();
    type_slowly(&s, "hello resize");
    s.wait_for("入力が見える", |t| t.contains("hello resize"));
    for (cols, rows) in [(100, 30), (60, 12), (120, 40), (40, 10), (90, 25)] {
        s.resize(cols, rows);
        thread::sleep(Duration::from_millis(40));
    }
    thread::sleep(Duration::from_millis(800));
    let before = s.screen();
    assert!(before.contains("hello resize"), "{before}");
    // 手動の全画面の再描画 (Ctrl+K R) をしても、見え方は変わらない
    s.send(b"r");
    thread::sleep(Duration::from_millis(800));
    let after = s.screen();
    assert_eq!(before, after, "リサイズ後の画面が全再描画と違う");
    s.send(CTRL_Q);
    s.wait_for("確認", |t| t.contains("[D]"));
    s.send(b"d");
    assert_eq!(s.wait_exit(), 0);
}

// verifies: AC-NFR-502-2
#[test]
fn 同じフォルダとカレントに置いた偽の_dll_は読み込まれない() {
    // 実行ファイルが起動時に取り込む System32 の DLL (KnownDLLs でないもの) と同じ名前の偽の DLL を、
    // 実行ファイルと同じフォルダとカレントディレクトリに置いて起動する。読み込まれると DllMain が印のファイルを作る。
    let dir = std::env::temp_dir().join(format!(
        "eve-fakedll-{}-{}",
        std::process::id(),
        STATE_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mark = dir.join("LOADED.txt");
    // std を使わない DLL (DllMain の定義が std の実行環境と衝突しないよう、入口を自分で指定する)
    let src = format!(
        r#"
        #![no_std]
        #[panic_handler]
        fn panic(_: &core::panic::PanicInfo) -> ! {{ loop {{}} }}
        #[link(name = "kernel32")]
        unsafe extern "system" {{
            fn CreateFileA(name: *const u8, access: u32, share: u32, sec: *mut u8, disp: u32, flags: u32, tmpl: *mut u8) -> isize;
            fn CloseHandle(h: isize) -> i32;
        }}
        #[unsafe(no_mangle)]
        pub extern "system" fn entry(_h: *mut u8, reason: u32, _r: *mut u8) -> i32 {{
            if reason == 1 {{
                unsafe {{
                    let h = CreateFileA(b"{}\0".as_ptr(), 0x4000_0000, 0, core::ptr::null_mut(), 2, 0x80, core::ptr::null_mut());
                    if h != -1 {{ CloseHandle(h); }}
                }}
            }}
            1
        }}
        // evoeditor が bcryptprimitives.dll から取り込む関数 (読み込まれても起動できるように)
        #[unsafe(no_mangle)]
        pub extern "system" fn ProcessPrng(_p: *mut u8, _n: usize) -> i32 {{ 1 }}
        "#,
        mark.display().to_string().replace(char::from(92), "/")
    );
    let rs = dir.join("fake.rs");
    std::fs::write(&rs, src).unwrap();
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let built = std::process::Command::new(rustc)
        .args([
            "--edition",
            "2024",
            "--crate-type",
            "cdylib",
            "-C",
            "panic=abort",
            "-C",
            "link-arg=/ENTRY:entry",
            "-o",
        ])
        .arg(dir.join("bcryptprimitives.dll"))
        .arg(&rs)
        .output()
        .expect("rustc を起動できる");
    assert!(
        built.status.success(),
        "偽の DLL を作れない: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    // 試験の exe を、偽の DLL と同じフォルダへ写して起動する (カレントも同じフォルダ)
    let exe_copy = dir.join("evoeditor.exe");
    std::fs::copy(BUILT, &exe_copy).unwrap();
    let out = std::process::Command::new(&exe_copy)
        .arg("--version")
        .current_dir(&dir)
        .output()
        .expect("起動できる");
    assert!(out.status.success(), "{:?}", out);
    assert!(
        !mark.exists(),
        "同じフォルダの偽の bcryptprimitives.dll が読み込まれた"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// verifies: AC-NFR-502-2
#[test]
fn 到達できない_unc_パスを開いている間も_ui_は止まらず終了できる() {
    // 名前が引けない UNC パス (OS が応答を待つ)。読込はワーカーで行うので、画面は出て、キーも効く
    let mut s = Session::start(&[r"\\10.255.255.1\share\a.txt"]);
    s.wait_for("最初の画面", |t| t.contains("終了"));
    s.send(CTRL_Q);
    let t = Instant::now();
    let code = s.wait_exit();
    assert!(code == 0 || code == 1, "終了コード {code}\n{}", s.screen());
    assert!(t.elapsed() < WAIT, "Ctrl+Q の後も終了しない");
}
