//! 実バイナリを ConPTY で駆動する計測 (docs/09-testing-quality.md §5.2、01 §3.0.1)。
//!
//! **計測用ビルド** (`--features bench-trace`) のとき、プロセス内の区間の記録 (`EVE_TRACE_FILE`、eve-tui の `trace`) を使って、
//! 仕様 (01 §3.0.1) の区間で測る。これが判定に使う値:
//! - **BM-101a / 102a**: `main` 入口 → 最初の Frame の `write` が戻るまで (起動 (a) プロセス内)。ファイル指定なし / 1 MB のファイル。
//! - **BM-101b / 102b**: 計測ハーネスが spawn した時刻 → 同じ終点 (起動 (b) spawn から。壁時計で比べる)。
//! - **BM-103b**: 入力スレッドが入力を読み終えた時刻 → 描画の `write` が戻るまで (判定区間 (b))。打鍵間隔 30 ms。
//! - **BM-107**: 100 MB の文書の保存中のカーソル移動の同じ区間 (01 NFR-107 AC-1。NFR-103 の値を維持するか)。
//!
//! どのビルドでも測る、端末込みの参考値 (ConPTY を挟むので実際の Windows Terminal より遅めに出る。判定には使わない):
//! - **BM-101c / 102c**: spawn → 最初の画面 (ステータスバーの `無題-1` / ファイルの先頭) が端末に届くまで。
//! - **BM-103c**: キー入力 (`a`) を送ってから、画面にその文字が現れるまで。
//!
//! **判定は基準機で行う** (開発機の値は目安)。
//!
//! ```text
//! cargo bench -p evoeditor --features bench-trace --bench bm_pty   # 計測用ビルド (判定に使う区間も測る)
//! cargo bench -p evoeditor --bench bm_pty                          # 端末込みの参考値だけ
//! cargo bench -p evoeditor --bench bm_pty -- --runs 50             # 回数を減らす (記録には使わない)
//! cargo bench -p evoeditor --bench bm_pty -- --json                # 1 行ずつの JSON
//! cargo bench -p evoeditor --bench bm_pty -- --only 103,107        # 一部だけ (101 102 103 107)
//! ```

#![cfg(windows)]
#![allow(clippy::print_stdout, clippy::print_stderr)]
// 計測の駆動側の読み取りスレッド (製品のスレッドではない。R11 の対象外)
#![allow(clippy::disallowed_methods)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    /// ステータスバーに読込中の表示が出ている。
    loading: Arc<AtomicBool>,
    _master: Box<dyn MasterPty + Send>,
    state: PathBuf,
    /// spawn の直前の壁時計 (UNIX 紀元からの ns)。起動 (b) の始点。
    spawn_unix_ns: u128,
}

impl Term {
    /// spawn の直前の時刻を返す。`trace` があれば、区間の記録をそのファイルに出させる (計測用ビルドのとき)。
    fn start(args: &[&str], marker: &'static str, trace: Option<&Path>) -> (Term, Instant) {
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
        if let Some(t) = trace {
            cmd.env("EVE_TRACE_FILE", t);
        }
        let mut reader = pty.master.try_clone_reader().expect("reader");
        let writer = Arc::new(Mutex::new(pty.master.take_writer().expect("writer")));
        let parser = Arc::new(Mutex::new(vt100::Parser::new(ROWS, COLS, 0)));
        let a_count = Arc::new(AtomicUsize::new(0));
        let ready = Arc::new(AtomicBool::new(false));
        let marker_flag = Arc::new(AtomicBool::new(false));
        let saving = Arc::new(AtomicBool::new(false));
        let loading = Arc::new(AtomicBool::new(false));
        let loading_t = loading.clone();
        let t0 = Instant::now();
        let spawn_unix_ns = unix_ns();
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
                    loading_t.store(all.contains("読込中"), Ordering::Release);
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
                loading,
                _master: pty.master,
                state,
                spawn_unix_ns,
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

    /// 端末の大きさを変える (ConPTY へ。画面の解釈 (vt100) の大きさは変えない。計測は記録で行う)。
    fn resize(&mut self, cols: u16, rows: u16) {
        self._master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("リサイズ");
    }

    /// 終了を待つ。終了コード (待ちきれなければ `None`)。
    fn wait_exit(&mut self, limit: Duration) -> Option<u32> {
        let t = Instant::now();
        while t.elapsed() < limit {
            if let Ok(Some(st)) = self.child.try_wait() {
                return Some(st.exit_code());
            }
            thread::sleep(Duration::from_millis(1));
        }
        None
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

fn unix_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

/// 区間の記録 (eve-tui の `trace`) の 1 行。
struct Span {
    name: String,
    start_ns: u64,
    end_ns: u64,
    vk: Option<u64>,
    wall_end_unix_ns: Option<u128>,
    /// 区間の内訳の時刻 (`input.to_flush` のとき)。
    sent_ns: Option<u64>,
    render_ns: Option<u64>,
    emit_ns: Option<u64>,
}

/// `"key":123` の数を取り出す (記録は形が決まっているので、簡単な走査で足りる)。
fn number_after(line: &str, key: &str) -> Option<u128> {
    let pat = format!("\"{key}\":");
    let i = line.find(&pat)? + pat.len();
    let digits: String = line[i..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn read_spans(path: &Path) -> Vec<Span> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let name = l.split("\"name\":\"").nth(1)?.split('"').next()?.to_owned();
            Some(Span {
                name,
                start_ns: number_after(l, "start_ns")? as u64,
                end_ns: number_after(l, "end_ns")? as u64,
                vk: number_after(l, "vk").map(|v| v as u64),
                wall_end_unix_ns: number_after(l, "wall_end_unix_ns"),
                sent_ns: number_after(l, "sent_ns").map(|v| v as u64),
                render_ns: number_after(l, "render_ns").map(|v| v as u64),
                emit_ns: number_after(l, "emit_ns").map(|v| v as u64),
            })
        })
        .collect()
}

fn ns_dur(ns: u64) -> Duration {
    Duration::from_nanos(ns)
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

/// 1 MB の UTF-8 (日本語混在) のファイル。
fn make_one_mb(dir: &Path) -> PathBuf {
    let path = dir.join("one_mb.txt");
    if !path.exists() {
        let mut body = String::new();
        let mut i = 0u64;
        while body.len() < 1024 * 1024 {
            body.push_str(&format!(
                "{i:08} the quick brown fox 日本語の行です 0123456789\n"
            ));
            i += 1;
        }
        std::fs::write(&path, &body).expect("1 MB のファイル");
    }
    path
}

/// 計測用ビルドか (隠しオプションを受け付け、区間の記録が書かれる)。
fn is_traced_build(dir: &Path) -> bool {
    let trace = dir.join("probe.jsonl");
    let (mut t, _) = Term::start(&["--bench-exit-after-first-frame"], "", Some(&trace));
    let code = t.wait_exit(Duration::from_secs(10));
    let ok = code == Some(0)
        && read_spans(&trace)
            .iter()
            .any(|s| s.name == "startup.first_frame");
    let _ = std::fs::remove_file(&trace);
    ok
}

/// 起動を `runs` 回測る。(a) `main` 入口 → 最初の Frame、(b) spawn → 同じ終点 (壁時計)。
/// ファイル指定のときは本文が見える最初の Frame まで (画面に目印が出てから `Ctrl+Q` で終了させ、記録を書き出させる)。
fn startup(
    dir: &Path,
    file: Option<&Path>,
    runs: usize,
    warmup: usize,
) -> (Vec<Duration>, Vec<Duration>) {
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for i in 0..runs + warmup {
        let trace = dir.join(format!("startup-{i}.jsonl"));
        let (mut t, name) = match file {
            None => (
                Term::start(&["--bench-exit-after-first-frame"], "", Some(&trace)).0,
                "startup.first_frame",
            ),
            Some(f) => {
                let t = Term::start(&[f.to_str().unwrap()], "00000000", Some(&trace)).0;
                t.wait("ファイルの先頭", || t.marker.load(Ordering::Acquire));
                let mut t = t;
                t.send(b"\x11"); // Ctrl+Q (未編集なので確認なしで終わる)
                (t, "startup.first_content")
            }
        };
        let code = t.wait_exit(WAIT);
        assert_eq!(code, Some(0), "正常に終了しません");
        let spans = read_spans(&trace);
        let hit = spans
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("{name} の記録がありません"));
        if i >= warmup {
            a.push(ns_dur(hit.end_ns - hit.start_ns));
            let end = hit.wall_end_unix_ns.expect("壁時計");
            b.push(Duration::from_nanos((end - t.spawn_unix_ns) as u64));
        }
        let _ = std::fs::remove_file(&trace);
    }
    (a, b)
}

/// キー入力の判定区間 (入力の読取完了 → 描画の `write`) の記録。1 MB の文書の中央で、打鍵間隔 30 ms で `a` を送る。
fn key_latency_trace(dir: &Path, file: &Path, runs: usize, warmup: usize) -> Vec<Span> {
    const VK_A: u64 = 0x41;
    let trace = dir.join("keys.jsonl");
    let (mut t, _) = Term::start(&[file.to_str().unwrap()], "00000000", Some(&trace));
    t.wait("ファイルの先頭", || t.marker.load(Ordering::Acquire));
    // 文書の中央 (行へ移動: Ctrl+G → 行番号 → Enter)
    t.send(b"\x07");
    thread::sleep(Duration::from_millis(100));
    t.send(b"9000\r");
    thread::sleep(Duration::from_millis(300));
    let n = runs + warmup;
    for _ in 0..n {
        t.send(b"a");
        thread::sleep(Duration::from_millis(30));
    }
    // 未保存の確認で破棄 (d) して終了し、記録を書き出させる
    t.send(b"\x11");
    thread::sleep(Duration::from_millis(200));
    t.send(b"d");
    let code = t.wait_exit(WAIT);
    assert_eq!(code, Some(0), "正常に終了しません");
    let spans: Vec<Span> = read_spans(&trace)
        .into_iter()
        .filter(|s| s.name == "input.to_flush" && s.vk == Some(VK_A))
        .collect();
    let _ = std::fs::remove_file(&trace);
    spans.into_iter().skip(warmup).collect()
}

/// 内訳 4 区間: 読取完了 → 受信口へ送り終え / → 描画の呼出し (core の処理と Frame 生成) / → 出力の文字列 / → `write` が戻る。
fn breakdown(spans: &[Span]) -> [Vec<Duration>; 4] {
    let mut out: [Vec<Duration>; 4] = Default::default();
    for s in spans {
        let (Some(sent), Some(render), Some(emit)) = (s.sent_ns, s.render_ns, s.emit_ns) else {
            continue;
        };
        // 内訳の時刻が欠けている (別の描画に使われた) 記録は飛ばす
        if !(s.start_ns <= sent && sent <= render && render <= emit && emit <= s.end_ns) {
            continue;
        }
        out[0].push(ns_dur(sent - s.start_ns));
        out[1].push(ns_dur(render - sent));
        out[2].push(ns_dur(emit - render));
        out[3].push(ns_dur(s.end_ns - emit));
    }
    out
}

/// 大きい UTF-8 のファイル (`mb` MB 以上。日本語混在、改行つき)。同じ名前で作り直さない。
fn make_big(dir: &Path, name: &str, mb: usize) -> PathBuf {
    let path = dir.join(name);
    if !path.exists() {
        let line = "0123456789 the quick brown fox 日本語の行です。0123456789\n";
        let big = line.repeat(mb * 1024 * 1024 / line.len());
        std::fs::write(&path, &big).expect("大きいファイル");
    }
    path
}

/// 保存中のカーソル移動の判定区間 (入力の読取完了 → 描画の `write`)。`mb` MB の文書を開いて 1 文字入れ、`Ctrl+S` の実行中にカーソルを動かす。
/// 100 MB は NFR-107 (保存中も NFR-103 を維持)、16 MiB 以下は保存前の照合 (内容ハッシュの読み直し) 中の入力 (FR-117 AC-2)。
fn save_window(dir: &Path, mb: usize, id: &str, traced: bool, json: bool, quick: bool) {
    let path = make_big(dir, &format!("big_{mb}mb.txt"), mb);
    let trace = dir.join(format!("save-{mb}.jsonl"));
    let (mut t, t0) = Term::start(
        &[path.to_str().unwrap()],
        "quick brown",
        traced.then_some(trace.as_path()),
    );
    t.wait("開く", || t.marker.load(Ordering::Acquire));
    thread::sleep(Duration::from_millis(if mb >= 50 { 4000 } else { 1000 }));
    println!(
        "({mb} MB を開くまで約 {:.1} s)",
        t0.elapsed().as_secs_f64() - if mb >= 50 { 4.0 } else { 1.0 }
    );
    let mut v = Vec::new();
    let mut save_ms = Vec::new();
    for _ in 0..(if mb >= 50 { 5 } else { 40 }) {
        t.send(b"a"); // 変更して dirty にする
        thread::sleep(Duration::from_millis(100));
        let s0 = Instant::now();
        t.send(b"\x13"); // Ctrl+S
        // 保存が終わる (変更マークが消える) まで、カーソル移動 (文書を変えない) の反映を測る
        while t.saving.load(Ordering::Acquire) && s0.elapsed() < Duration::from_secs(60) {
            v.push(t.move_latency());
            thread::sleep(Duration::from_millis(5));
        }
        save_ms.push(s0.elapsed());
    }
    println!("{id} 保存ジョブ ({mb} MB) の完了まで (参考値): {save_ms:?}");
    if v.is_empty() {
        println!("{id}: 保存の間に計測できた入力がありません (保存が速すぎる)");
    } else {
        let cid = format!("{id}c");
        let cid = if traced { cid.as_str() } else { id };
        report(
            cid,
            "保存中のカーソル移動 → 画面 (端末込み)",
            v,
            json,
            quick,
        );
    }
    if traced {
        // 判定に使う値: 保存中に送ったカーソル移動 (→) の、入力の読取完了 → 描画の write
        const VK_RIGHT: u64 = 0x27;
        t.send(b"\x11"); // Ctrl+Q (保存は終わっている)
        assert_eq!(t.wait_exit(WAIT), Some(0), "正常に終了しません");
        let spans: Vec<Duration> = read_spans(&trace)
            .into_iter()
            .filter(|s| s.name == "input.to_flush" && s.vk == Some(VK_RIGHT))
            .map(|s| ns_dur(s.end_ns - s.start_ns))
            .collect();
        if spans.is_empty() {
            println!("{id}: 記録された区間がありません");
        } else {
            report(
                id,
                "保存中のカーソル移動 (入力の読取完了 → 描画の write)",
                spans,
                json,
                quick,
            );
        }
    }
}

/// `mb` MB の文書を開いて全選択し、`Ctrl+C` の間と後にカーソルを左右に動かす。キー入力の判定区間 (入力の読取完了 → 描画の `write`)。
/// 60 MB は文字列化とシステムのクリップボードへの書込を含む。100 MB は上限 (64 MiB) を超えるので内部レジスタだけ (FR-205 AC-4)。
fn clip_window(dir: &Path, mb: usize, id: &str, json: bool, quick: bool) {
    const VK_LEFT: u64 = 0x25;
    const VK_RIGHT: u64 = 0x27;
    let path = make_big(dir, &format!("big_{mb}mb.txt"), mb);
    let trace = dir.join(format!("clip-{mb}.jsonl"));
    let (mut t, _) = Term::start(&[path.to_str().unwrap()], "quick brown", Some(&trace));
    t.wait("開く", || t.marker.load(Ordering::Acquire));
    thread::sleep(Duration::from_millis(if mb >= 50 { 4000 } else { 1000 }));
    let rounds = if quick { 3 } else { 10 };
    let mut keys = 0;
    for _ in 0..rounds {
        t.send(b"\x01"); // Ctrl+A
        thread::sleep(Duration::from_millis(100));
        t.send(b"\x03"); // Ctrl+C
        // コピーの間 (文字列化・書込) に、30 ms ごとに左右へ動かす
        for i in 0..60 {
            t.send(if i % 2 == 0 { b"\x1b[C" } else { b"\x1b[D" });
            keys += 1;
            thread::sleep(Duration::from_millis(30));
        }
    }
    let _ = keys;
    t.send(b"\x11");
    assert_eq!(t.wait_exit(WAIT), Some(0), "正常に終了しません");
    let spans: Vec<Duration> = read_spans(&trace)
        .into_iter()
        .filter(|s| s.name == "input.to_flush" && matches!(s.vk, Some(VK_LEFT | VK_RIGHT)))
        .map(|s| ns_dur(s.end_ns - s.start_ns))
        .collect();
    let _ = std::fs::remove_file(&trace);
    report(
        id,
        &format!("{mb} MB の全選択のコピー中のキー入力 (入力の読取完了 → 描画の write)"),
        spans,
        json,
        quick,
    );
}

/// 「開く → 進捗」(01 §3.0.1): `Ctrl+O` → パス → `Enter` の、`Enter` の読取完了 → 進捗を含む最初の Frame の `write`。
/// 2 つのファイルを交互に開く (同じファイルの再読込と区別するため)。
fn open_progress(dir: &Path, a: &Path, b: &Path, runs: usize, warmup: usize) -> Vec<Duration> {
    const VK_ENTER: u64 = 0x0D;
    let trace = dir.join("open.jsonl");
    let (mut t, _) = Term::start(&[], "", Some(&trace));
    t.wait("最初の画面", || t.ready.load(Ordering::Acquire));
    for i in 0..runs + warmup {
        let p = if i % 2 == 0 { a } else { b };
        t.send(b"\x0f"); // Ctrl+O
        thread::sleep(Duration::from_millis(80));
        t.send(p.to_str().unwrap().as_bytes());
        thread::sleep(Duration::from_millis(60));
        t.send(b"\r");
        // 読込が始まって終わるまで待つ (小さいファイルは見逃すので、待ちは上限つき)
        let w = Instant::now();
        while !t.loading.load(Ordering::Acquire) && w.elapsed() < Duration::from_millis(400) {
            thread::sleep(Duration::from_millis(2));
        }
        let w = Instant::now();
        while t.loading.load(Ordering::Acquire) && w.elapsed() < WAIT {
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(150));
    }
    t.send(b"\x11");
    assert_eq!(t.wait_exit(WAIT), Some(0), "正常に終了しません");
    let spans: Vec<Span> = read_spans(&trace)
        .into_iter()
        .filter(|s| s.name == "open.progress" && s.vk == Some(VK_ENTER))
        .collect();
    let _ = std::fs::remove_file(&trace);
    spans
        .iter()
        .skip(warmup)
        .map(|s| ns_dur(s.end_ns - s.start_ns))
        .collect()
}

/// 「リサイズ → 再描画」(01 §3.0.1): ウィンドウの大きさを 50 ms ごとに変え、最後のリサイズイベントの読取完了 → 新しい大きさの Frame の `write`。
fn resize_redraw(dir: &Path, runs: usize, warmup: usize) -> Vec<Duration> {
    let trace = dir.join("resize.jsonl");
    let (mut t, _) = Term::start(&[], "", Some(&trace));
    t.wait("最初の画面", || t.ready.load(Ordering::Acquire));
    for i in 0..runs + warmup {
        let (cols, rows) = if i % 2 == 0 { (100, 30) } else { (120, 40) };
        t.resize(cols, rows);
        thread::sleep(Duration::from_millis(50));
    }
    t.send(b"\x11");
    assert_eq!(t.wait_exit(WAIT), Some(0), "正常に終了しません");
    let spans: Vec<Span> = read_spans(&trace)
        .into_iter()
        .filter(|s| s.name == "resize.to_flush")
        .collect();
    let _ = std::fs::remove_file(&trace);
    spans
        .iter()
        .skip(warmup)
        .map(|s| ns_dur(s.end_ns - s.start_ns))
        .collect()
}

/// アイドル時のメモリ (最初の Frame の 10 秒後の Private Bytes) と CPU (その後 `cpu_secs` 秒間の使用率)。PowerShell で読む。
fn idle_stats(dir: &Path, cpu_secs: u64) -> (u64, f64) {
    let (t, _) = Term::start(&[], "", None);
    t.wait("最初の画面", || t.ready.load(Ordering::Acquire));
    thread::sleep(Duration::from_secs(10));
    let pid = t.child.process_id().expect("pid");
    let (mem, cpu0) = proc_stat(pid);
    thread::sleep(Duration::from_secs(cpu_secs));
    let (_, cpu1) = proc_stat(pid);
    let _ = dir;
    (mem, (cpu1 - cpu0) / cpu_secs as f64 * 100.0)
}

/// (Private Bytes, 累積の CPU 時間 (秒))。
fn proc_stat(pid: u32) -> (u64, f64) {
    let script = format!(
        "$p = Get-Process -Id {pid}; \"$($p.PrivateMemorySize64) $($p.TotalProcessorTime.TotalSeconds)\""
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .expect("powershell");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut it = text.split_whitespace();
    let mem = it.next().and_then(|v| v.parse().ok()).expect("メモリ");
    let cpu = it.next().and_then(|v| v.parse().ok()).expect("CPU");
    (mem, cpu)
}

/// 100 MB の文書の検索バーで文字を打ち続け (1 文字ごとに全件の検索が走る)、その間のキー入力の判定区間 (FR-404 AC-1)。
fn search_load(dir: &Path, runs: usize, warmup: usize) -> Vec<Duration> {
    let path = make_big(dir, "big_100mb.txt", 100);
    let trace = dir.join("search.jsonl");
    let (mut t, _) = Term::start(&[path.to_str().unwrap()], "quick brown", Some(&trace));
    t.wait("開く", || t.marker.load(Ordering::Acquire));
    thread::sleep(Duration::from_millis(4000));
    t.send(b"\x06"); // Ctrl+F
    thread::sleep(Duration::from_millis(300));
    // 一致しない文字を打つ (全件を走査する。`k` `s` を含めない: 含むと行ごとに畳み込む経路になる、03 §12.4)。30 ms ごと
    for i in 0..runs + warmup {
        let c = b"qzxjwv"[i % 6];
        t.send(&[c]);
        thread::sleep(Duration::from_millis(30));
    }
    t.send(b"\x1b"); // 検索バーを閉じる
    thread::sleep(Duration::from_millis(300));
    t.send(b"\x11");
    assert_eq!(t.wait_exit(WAIT), Some(0), "正常に終了しません");
    let spans: Vec<Span> = read_spans(&trace)
        .into_iter()
        .filter(|s| s.name == "input.to_flush" && s.vk.is_some_and(|v| (0x41..=0x5A).contains(&v)))
        .collect();
    let _ = std::fs::remove_file(&trace);
    spans
        .iter()
        .skip(warmup)
        .map(|s| ns_dur(s.end_ns - s.start_ns))
        .collect()
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

    // ---- 計測用ビルド: プロセス内の区間の記録で、仕様の区間を測る (判定に使う値) ----
    let traced = is_traced_build(&dir);
    if !traced && (want("101") || want("102") || want("103") || want("107")) {
        println!(
            "(計測用ビルドではないので、BM-101a/b・102a/b・103b・107 は測りません。`--features bench-trace` を付けてください)"
        );
    }
    if traced && want("101") {
        let (a, b) = startup(&dir, None, runs, warmup);
        report(
            "BM-101a",
            "起動 (a) main 入口 → 最初の Frame",
            a,
            json,
            quick,
        );
        report("BM-101b", "起動 (b) spawn → 最初の Frame", b, json, quick);
    }
    if traced && want("102") {
        let file = make_one_mb(&dir);
        let (a, b) = startup(&dir, Some(&file), runs, warmup);
        report(
            "BM-102a",
            "起動 (a) main 入口 → 本文の最初の Frame (1 MB)",
            a,
            json,
            quick,
        );
        report(
            "BM-102b",
            "起動 (b) spawn → 本文の最初の Frame (1 MB)",
            b,
            json,
            quick,
        );
    }
    if traced && want("103") {
        let file = make_one_mb(&dir);
        let spans = key_latency_trace(&dir, &file, runs, warmup);
        let v: Vec<Duration> = spans
            .iter()
            .map(|s| ns_dur(s.end_ns - s.start_ns))
            .collect();
        report(
            "BM-103b",
            "キー入力 (入力の読取完了 → 描画の write) 打鍵間隔 30 ms",
            v,
            json,
            quick,
        );
        // 内訳 (切り分け用。判定には使わない)
        let names = [
            (
                "BM-103b-1-read-to-sent",
                "内訳 1: 読取完了 → 受信口へ送り終え",
            ),
            (
                "BM-103b-2-sent-to-render",
                "内訳 2: → 描画の呼出し (core の処理と Frame 生成)",
            ),
            ("BM-103b-3-render-to-emit", "内訳 3: → 差分の出力の文字列"),
            ("BM-103b-4-emit-to-write", "内訳 4: → write が戻る"),
        ];
        for (series, (id, label)) in breakdown(&spans).into_iter().zip(names) {
            if !series.is_empty() {
                report(id, label, series, json, quick);
            }
        }
    }

    // BM-101c: 起動 → 最初の画面
    if want("101") {
        let mut v = Vec::new();
        for i in 0..runs + warmup {
            let (t, t0) = Term::start(&[], "", None);
            t.wait("最初の画面", || t.ready.load(Ordering::Acquire));
            let d = t0.elapsed();
            drop(t);
            if i >= warmup {
                v.push(d);
            }
        }
        report(
            "BM-101c",
            "起動 (spawn → 最初の画面、端末込み)",
            v,
            json,
            quick,
        );
    }

    // BM-102b: 1 MB のファイル引数
    if want("102") {
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
            let (t, t0) = Term::start(&[path.to_str().unwrap()], "00000000", None);
            t.wait("ファイルの先頭", || t.marker.load(Ordering::Acquire));
            let d = t0.elapsed();
            drop(t);
            if i >= warmup {
                v.push(d);
            }
        }
        report(
            "BM-102c",
            "起動 (1 MB のファイル → 先頭の表示)",
            v,
            json,
            quick,
        );
    }

    // BM-103b: キー入力 → 画面
    if want("103") {
        let (mut t, _) = Term::start(&[], "", None);
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
        report("BM-103c", "キー入力 → 画面 (端末込み)", v, json, quick);
    }

    // BM-107: 100 MB の文書の保存ジョブの実行中の入力
    if want("107") {
        save_window(&dir, 100, "BM-107", traced, json, quick);
    }
    // ---- 計測用ビルド: 負荷の下のキー入力と、開く・リサイズ・アイドルの区間 (判定に使う値) ----
    if traced && want("104p") {
        let a10 = make_big(&dir, "p10_a.txt", 10);
        let b10 = make_big(&dir, "p10_b.txt", 10);
        let v = open_progress(&dir, &a10, &b10, runs, warmup);
        report(
            "BM-104p-10m",
            "開く → 進捗 (10 MB。Enter の読取完了 → 進捗を含む最初の Frame の write)",
            v,
            json,
            quick,
        );
        let a100 = make_big(&dir, "big_100mb.txt", 100);
        let b100 = make_big(&dir, "p100_b.txt", 100);
        let n = if quick { 10 } else { 30 };
        let v = open_progress(&dir, &a100, &b100, n, 3);
        report(
            "BM-104p-100m",
            "開く → 進捗 (100 MB。30 回)",
            v,
            json,
            quick,
        );
    }
    if traced && want("512") {
        let v = resize_redraw(&dir, runs, warmup);
        report(
            "BM-512",
            "リサイズ → 再描画 (最後のリサイズの読取完了 → 新しい大きさの Frame の write)",
            v,
            json,
            quick,
        );
    }
    if traced && want("404") {
        let v = search_load(&dir, runs, warmup);
        report(
            "BM-103b-search",
            "100 MB の全件検索中のキー入力 (入力の読取完了 → 描画の write)",
            v,
            json,
            quick,
        );
    }
    if traced && want("117") {
        save_window(&dir, 15, "BM-103b-reconcile", traced, json, quick);
    }
    if traced && want("205") {
        // FR-205 AC-4: 60 MB (システムへ書く) と 100 MB (上限超過で内部レジスタだけ) の全選択のコピー中のキー入力
        clip_window(&dir, 60, "BM-103b-clip60", json, quick);
        clip_window(&dir, 100, "BM-103b-clip100", json, quick);
    }
    if want("201") || want("204") {
        let n = if quick { 1 } else { 5 };
        let cpu_secs = if quick { 10 } else { 60 };
        let mut mems = Vec::new();
        let mut cpu = 0.0;
        for i in 0..n {
            let (m, c) = idle_stats(&dir, if i == 0 { cpu_secs } else { 1 });
            mems.push(m);
            if i == 0 {
                cpu = c;
            }
        }
        mems.sort_unstable();
        let (mem_med, mem_max) = (mems[mems.len() / 2], mems[mems.len() - 1]);
        let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
        if json {
            println!(
                "{{\"id\":\"BM-201\",\"n\":{n},\"value\":{:.2},\"max\":{:.2},\"unit\":\"MB\",\"quick\":{quick}}}",
                mb(mem_med),
                mb(mem_max)
            );
            println!(
                "{{\"id\":\"BM-204\",\"n\":1,\"value\":{cpu:.4},\"unit\":\"%\",\"seconds\":{cpu_secs},\"quick\":{quick}}}"
            );
        } else {
            println!(
                "BM-201 アイドル時のメモリ (Private Bytes、最初の Frame の 10 秒後): n={n} 中央値 {:.2} MB / 最大 {:.2} MB",
                mb(mem_med),
                mb(mem_max)
            );
            println!("BM-204 アイドル時の CPU 使用率 ({cpu_secs} 秒間): {cpu:.4} %");
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
