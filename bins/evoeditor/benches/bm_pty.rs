//! 実バイナリを ConPTY で駆動する計測 (docs/09-testing-quality.md §5.2、01 §3.0.1 の (b) spawn から / 端末込み)。
//!
//! - **BM-101b**: 起動 (spawn) → 最初の画面 (ステータスバーの `無題-1`) が端末に届くまで (ファイル指定なし)。
//! - **BM-102b**: 同じ区間で、1 MB の UTF-8 ファイルを引数に渡し、その先頭が画面に届くまで。
//! - **BM-103b**: キー入力 (`a`) を送ってから、画面にその文字が現れるまで (端末込み)。
//! - **BM-107**: 100 MB の文書の保存ジョブ (`Ctrl+S`) の実行中に、BM-103b と同じ計測をする (01 NFR-107 AC-1)。
//!
//! **判定は基準機で行う** (開発機の値は目安)。ConPTY を挟むので、実際の Windows Terminal より遅めに出る。
//! 計測用の隠しフラグと `tracing` の span (BM-101a の (a) プロセス内) は、計測用ビルドを入れる版で足す。
//!
//! ```text
//! cargo bench -p evoeditor --bench bm_pty
//! cargo bench -p evoeditor --bench bm_pty -- --runs 50          # 回数を減らす (記録には使わない)
//! cargo bench -p evoeditor --bench bm_pty -- --json             # 1 行ずつの JSON
//! cargo bench -p evoeditor --bench bm_pty -- --only 103b,107    # 一部だけ
//! ```

#![cfg(windows)]
#![allow(clippy::print_stdout, clippy::print_stderr)]
// 計測の駆動側の読み取りスレッド (製品のスレッドではない。R11 の対象外)
#![allow(clippy::disallowed_methods)]

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const EXE: &str = env!("CARGO_BIN_EXE_evoeditor");
const COLS: u16 = 120;
const ROWS: u16 = 40;
const WAIT: Duration = Duration::from_secs(30);

struct Term {
    child: Box<dyn Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    parser: Arc<Mutex<vt100::Parser>>,
    /// 先頭行に現れた `a` の数。
    a_count: Arc<AtomicUsize>,
    /// 画面にステータスバーが出た。
    ready: Arc<AtomicBool>,
    /// 本文の目印が画面に出た。
    marker: Arc<AtomicBool>,
    saving: Arc<AtomicBool>,
    _master: Box<dyn MasterPty + Send>,
    state: PathBuf,
}

impl Term {
    /// spawn の直前の時刻を返す。
    fn start(args: &[&str], marker: &'static str) -> (Term, Instant) {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let state = std::env::temp_dir().join(format!(
            "evo-bench-state-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
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
        cmd.env("EVE_STATE_DIR", &state);
        // 設定ディレクトリは無い (設定ファイルなし)
        cmd.env("EVE_CONFIG_DIR", state.join("no-config"));
        let mut reader = pty.master.try_clone_reader().expect("reader");
        let writer = Arc::new(Mutex::new(pty.master.take_writer().expect("writer")));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(ROWS, COLS, 0)));
        let a_count = Arc::new(AtomicUsize::new(0));
        let ready = Arc::new(AtomicBool::new(false));
        let marker_flag = Arc::new(AtomicBool::new(false));
        let saving = Arc::new(AtomicBool::new(false));
        let t0 = Instant::now();
        let child = pty.slave.spawn_command(cmd).expect("起動できません");
        drop(pty.slave);
        {
            let (parser, a_count, ready, marker_flag, saving) = (
                parser.clone(),
                a_count.clone(),
                ready.clone(),
                marker_flag.clone(),
                saving.clone(),
            );
            let answer = writer.clone();
            thread::spawn(move || {
                let mut buf = [0u8; 16384];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let chunk = &buf[..n];
                    // ConPTY は起動時にカーソル位置を尋ねる (DSR)。端末として答える
                    if chunk.windows(4).any(|w| w == b"\x1b[6n") {
                        let mut w = answer.lock().unwrap();
                        let _ = w.write_all(b"\x1b[1;1R");
                        let _ = w.flush();
                    }
                    let mut p = parser.lock().unwrap();
                    p.process(chunk);
                    let s = p.screen();
                    let all = s.contents();
                    a_count.store(all.matches('a').count(), Ordering::Release);
                    if all.contains("無題-1") || all.contains(".txt") {
                        ready.store(true, Ordering::Release);
                    }
                    if !marker.is_empty() && all.contains(marker) {
                        marker_flag.store(true, Ordering::Release);
                    }
                    saving.store(all.contains("[+]"), Ordering::Release);
                }
            });
        }
        (
            Term {
                child,
                writer,
                parser,
                a_count,
                ready,
                marker: marker_flag,
                saving,
                _master: pty.master,
                state,
            },
            t0,
        )
    }

    fn send(&mut self, b: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        w.write_all(b).expect("入力を書けません");
        w.flush().expect("flush");
    }

    fn wait(&self, what: &str, f: impl Fn() -> bool) {
        let t = Instant::now();
        while !f() {
            assert!(
                t.elapsed() < WAIT,
                "待っても {what} にならない。画面:\n{}",
                self.parser.lock().unwrap().screen().contents()
            );
            std::hint::spin_loop();
        }
    }

    /// `a` を 1 つ送り、画面に現れるまでの時間。
    fn key_latency(&mut self) -> Duration {
        let want = self.a_count.load(Ordering::Acquire) + 1;
        let t = Instant::now();
        self.send(b"a");
        self.wait("入力の表示", || {
            self.a_count.load(Ordering::Acquire) >= want
        });
        t.elapsed()
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = std::fs::remove_dir_all(&self.state);
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn report(id: &str, label: &str, mut v: Vec<Duration>, json: bool, quick: bool) {
    v.sort();
    let (p50, p99, p999, max) = (
        percentile(&v, 0.5),
        percentile(&v, 0.99),
        percentile(&v, 0.999),
        v[v.len() - 1],
    );
    if json {
        println!(
            "{{\"id\":\"{id}\",\"n\":{},\"p50_ms\":{:.2},\"p99_ms\":{:.2},\"p999_ms\":{:.2},\"max_ms\":{:.2},\"quick\":{quick}}}",
            v.len(),
            ms(p50),
            ms(p99),
            ms(p999),
            ms(max)
        );
    } else {
        println!(
            "{id} {label}: n={} p50 {:.2} ms / p99 {:.2} ms / p99.9 {:.2} ms / max {:.2} ms",
            v.len(),
            ms(p50),
            ms(p99),
            ms(p999),
            ms(max)
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let json = args.iter().any(|a| a == "--json");
    let runs: usize = args
        .iter()
        .position(|a| a == "--runs")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let quick = runs < 1000;
    let only: Option<Vec<String>> = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').map(str::to_owned).collect());
    let want = |id: &str| only.as_ref().is_none_or(|o| o.iter().any(|x| x == id));
    let warmup = runs / 10;

    let dir = std::env::temp_dir().join(format!("evo-bench-pty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("作業ディレクトリ");

    // BM-101b: 起動 → 最初の画面
    if want("101b") {
        let mut v = Vec::new();
        for i in 0..runs + warmup {
            let (t, t0) = Term::start(&[], "");
            t.wait("最初の画面", || t.ready.load(Ordering::Acquire));
            let d = t0.elapsed();
            drop(t);
            if i >= warmup {
                v.push(d);
            }
        }
        report("BM-101b", "起動 (spawn → 最初の画面)", v, json, quick);
    }

    // BM-102b: 1 MB のファイル引数
    if want("102b") {
        let path = dir.join("one_mb.txt");
        let mut body = String::new();
        let mut i = 0u64;
        while body.len() < 1024 * 1024 {
            body.push_str(&format!(
                "{i:08} the quick brown fox 日本語の行です 0123456789\n"
            ));
            i += 1;
        }
        std::fs::write(&path, &body).expect("1 MB のファイル");
        let mut v = Vec::new();
        for i in 0..runs + warmup {
            let (t, t0) = Term::start(&[path.to_str().unwrap()], "00000000");
            t.wait("ファイルの先頭", || t.marker.load(Ordering::Acquire));
            let d = t0.elapsed();
            drop(t);
            if i >= warmup {
                v.push(d);
            }
        }
        report(
            "BM-102b",
            "起動 (1 MB のファイル → 先頭の表示)",
            v,
            json,
            quick,
        );
    }

    // BM-103b: キー入力 → 画面
    if want("103b") {
        let (mut t, _) = Term::start(&[], "");
        t.wait("最初の画面", || t.ready.load(Ordering::Acquire));
        let mut v = Vec::new();
        // 行が伸び続けないよう、時々改行する (入力そのものは `a`)
        for i in 0..runs * 5 {
            if i % 40 == 39 {
                t.send(b"\x01\x1b[3~"); // 全選択して削除
                thread::sleep(Duration::from_millis(50));
            }
            let d = t.key_latency();
            if i >= warmup {
                v.push(d);
            }
        }
        report("BM-103b", "キー入力 → 画面 (端末込み)", v, json, quick);
    }

    // BM-107: 100 MB の文書の保存ジョブの実行中の入力
    if want("107") {
        let path = dir.join("hundred_mb.txt");
        let line = "0123456789 the quick brown fox 日本語の行です。0123456789\n";
        let big = line.repeat(100 * 1024 * 1024 / line.len());
        std::fs::write(&path, &big).expect("100 MB のファイル");
        drop(big);
        let (mut t, t0) = Term::start(&[path.to_str().unwrap()], "quick brown");
        t.wait("開く", || t.marker.load(Ordering::Acquire));
        thread::sleep(Duration::from_millis(4000));
        println!(
            "(100 MB を開くまで約 {:.1} s)",
            t0.elapsed().as_secs_f64() - 4.0
        );
        let mut v = Vec::new();
        let mut save_ms = Vec::new();
        for _ in 0..5 {
            t.send(b"a"); // 変更して dirty にする
            thread::sleep(Duration::from_millis(100));
            let s0 = Instant::now();
            t.send(b""); // Ctrl+S
            // 保存が終わる (変更マークが消える) まで、カーソル移動 (文書を変えない) の反映を測る
            while t.saving.load(Ordering::Acquire) && s0.elapsed() < Duration::from_secs(60) {
                v.push(t.move_latency());
                thread::sleep(Duration::from_millis(5));
            }
            save_ms.push(s0.elapsed());
        }
        println!(
            "BM-107 保存ジョブ (100 MB) の完了まで (参考値): {:?}",
            save_ms
        );
        if v.is_empty() {
            println!("BM-107: 保存の間に計測できた入力がありません (保存が速すぎる)");
        } else {
            report("BM-107", "保存中のキー入力 → 画面", v, json, quick);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    if quick && !json {
        println!("(--runs 1000 未満は動作確認用です。記録には使いません)");
    }
}

impl Term {
    /// カーソルを右へ動かし、ステータスバーの桁が変わるまでの時間 (文書は変わらない)。
    fn move_latency(&mut self) -> Duration {
        let col = |t: &Term| {
            let c = t.parser.lock().unwrap().screen().contents();
            let rest = &c[c.rfind("桁 ").map_or(0, |i| i + "桁 ".len())..];
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u32>()
                .unwrap_or(0)
        };
        let before = col(self);
        let t = Instant::now();
        self.send(b"[C");
        self.wait("カーソル移動の表示", || col(self) != before);
        t.elapsed()
    }
}
