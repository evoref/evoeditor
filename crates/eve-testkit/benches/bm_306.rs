//! BM-306: 長時間の soak (docs/09-testing-quality.md §5.2 の NFR-306 の行、§6.3)。ヘッドレスの `Harness` で、
//! ランダムな編集 (入力・改行・削除・範囲の削除)・Undo / Redo・カーソル移動・検索・保存を決定的な乱数で繰り返し、
//! 一定の間隔ごとに RSS とキー入力 (入力 → Frame 生成、BM-103a と同じ区間) の p50 / p99 を記録する。
//! v0.2〜v0.4 は夜間に 1 時間回して **傾向を観察する** (ゲートにしない)。8 時間の判定は v1.0 (01 NFR-306)。
//!
//! 履歴の上限 (MR8: メッセージ履歴・検索履歴、元に戻す履歴の全体の上限) が効いていれば、RSS は最初の数分で
//! 頭打ちになり、入力の遅延も伸びない。文書の大きさは 512 KiB〜2 MiB に保つ (伸び続けると遅延の推移が読めない)。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_306 -- --minutes 60 [--interval-secs 60] [--seed N] [--out FILE]
//! cargo bench -p eve-testkit --bench bm_306 -- --minutes 1 --interval-secs 10   # 動作確認
//! ```
//!
//! 出力: 間隔ごとに 1 行の JSON (`"kind":"window"`) と、最後に要約の JSON (`"kind":"summary"`)。`--out` があれば同じ行を
//! ファイルにも書く。RSS は Linux の `/proc/self/statm` から読む (ほかの OS では `null`)。

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::Write as _;
use std::time::{Duration, Instant};

use eve_testkit::Harness;

const PATH: &str = "/w/soak.txt";
const MIN_BYTES: usize = 512 * 1024;
const MAX_BYTES: usize = 2 * 1024 * 1024;

/// 決定的な乱数 (xorshift64*)。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const WORDS: &[&str] = &[
    "alpha",
    "beta",
    "gamma",
    "delta",
    "日本語",
    "編集",
    "検索",
    "保存",
    "😀",
    "テキスト",
    "fox",
    "lazy",
];

/// 平均 80 B の行 (ASCII と日本語が混ざる) を `bytes` 以上になるまで並べる。決定的。
fn corpus(bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes + 128);
    let mut i = 0u64;
    while out.len() < bytes {
        let line = if i.is_multiple_of(3) {
            format!("{i:08} 日本語の行です。テキストエディタの試験データ。0123456789\n")
        } else {
            format!("{i:08} the quick brown fox jumps over the lazy dog 0123456789 abcdef\n")
        };
        out.extend_from_slice(line.as_bytes());
        i += 1;
    }
    out
}

/// 今のプロセスの RSS (MiB)。Linux 以外は `None`。
fn rss_mib() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: f64 = s.split_whitespace().nth(1)?.parse().ok()?;
    // ページの大きさは 4 KiB とみなす (x86_64 / aarch64 の Linux の既定。傾向を見るだけなので十分)
    Some(pages * 4096.0 / 1_048_576.0)
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

/// 最小二乗の傾き (y / x)。
fn slope(points: &[(f64, f64)]) -> f64 {
    let n = points.len() as f64;
    if points.len() < 2 {
        return 0.0;
    }
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let sxx: f64 = points.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
    if sxx == 0.0 { 0.0 } else { sxy / sxx }
}

#[derive(Default)]
struct Counts {
    ops: u64,
    undo: u64,
    redo: u64,
    searches: u64,
    saves: u64,
    trims: u64,
}

struct Soak {
    h: Harness,
    rng: Rng,
    counts: Counts,
    /// 今の間隔の、入力 → Frame 生成の時間
    samples: Vec<Duration>,
}

impl Soak {
    fn typed(&mut self, s: &str) {
        let t = Instant::now();
        self.h.text(s);
        let f = self.h.frame();
        self.samples.push(t.elapsed());
        std::hint::black_box(&f);
    }

    fn keys(&mut self, k: &str, times: u64) {
        for _ in 0..times {
            self.h.keys(k);
        }
        let _ = self.h.frame();
    }

    fn step(&mut self) {
        self.counts.ops += 1;
        let word = WORDS[self.rng.below(WORDS.len() as u64) as usize];
        match self.rng.below(100) {
            0..=39 => {
                let w = format!("{word} ");
                self.typed(&w);
            }
            40..=49 => self.typed("\n"),
            50..=59 => {
                let n = 1 + self.rng.below(10);
                self.keys("backspace", n);
            }
            60..=69 => {
                let k = [
                    "up",
                    "down",
                    "left",
                    "right",
                    "pagedown",
                    "pageup",
                    "ctrl+end",
                    "ctrl+home",
                ][self.rng.below(8) as usize];
                let n = 1 + self.rng.below(3);
                self.keys(k, n);
            }
            70..=77 => {
                self.counts.undo += 1;
                let n = 1 + self.rng.below(20);
                self.keys("ctrl+z", n);
            }
            78..=84 => {
                self.counts.redo += 1;
                let n = 1 + self.rng.below(20);
                self.keys("ctrl+y", n);
            }
            85..=91 => {
                // 毎回違う語を検索して、検索履歴を上限まで埋める
                self.counts.searches += 1;
                self.h.cmd("search.open");
                let q = format!("{word}{}", self.counts.searches % 500);
                self.h.text(&q);
                self.h.keys("enter");
                self.h.keys("esc");
                let _ = self.h.frame();
            }
            92..=94 => {
                // 保存 (保存のたびにメッセージが出て、メッセージ履歴を上限まで埋める)
                self.counts.saves += 1;
                self.h.cmd("file.save");
                self.h.run_jobs();
                let _ = self.h.frame();
            }
            _ => {
                // 範囲の削除 (文書が伸び続けないように)
                let n = 1 + self.rng.below(15);
                self.keys("shift+down", n);
                self.keys("backspace", 1);
            }
        }
    }

    /// 文書の大きさを MIN_BYTES〜MAX_BYTES に保つ (間隔ごとに見る)。
    fn keep_size(&mut self) -> usize {
        let len = self.h.doc_text().len();
        if len > MAX_BYTES {
            // 末尾の約半分を消す (1 回の範囲削除)
            self.counts.trims += 1;
            self.h.keys("ctrl+end");
            self.keys("shift+pageup", 400);
            self.keys("backspace", 1);
        } else if len < MIN_BYTES {
            self.counts.trims += 1;
            self.h.keys("ctrl+end");
            self.h.paste(&String::from_utf8_lossy(&corpus(MIN_BYTES)));
            let _ = self.h.frame();
        }
        self.h.doc_text().len()
    }
}

fn arg<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let minutes: f64 = arg(&args, "--minutes", 60.0);
    let interval = Duration::from_secs(arg(&args, "--interval-secs", 60u64).max(1));
    let seed: u64 = arg(&args, "--seed", 306u64).max(1);
    let out_path: Option<String> = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1).cloned());
    let mut out = out_path
        .as_ref()
        .map(|p| std::fs::File::create(p).unwrap_or_else(|e| panic!("{p} を作れません: {e}")));
    let mut emit = |line: String| {
        println!("{line}");
        if let Some(f) = out.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    };

    let data = corpus(1024 * 1024);
    let mut h = Harness::new().with_size(120, 40).with_file(PATH, &data);
    drop(data);
    h.open(PATH);
    h.run_jobs();
    let _ = h.frame();
    let mut s = Soak {
        h,
        rng: Rng(seed),
        counts: Counts::default(),
        samples: Vec::new(),
    };

    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(minutes * 60.0);
    let mut next = start + interval;
    // (経過分, RSS MiB) と (経過分, p99 ms)
    let mut rss_points: Vec<(f64, f64)> = Vec::new();
    let mut p99s: Vec<f64> = Vec::new();
    loop {
        s.step();
        let now = Instant::now();
        if now < next && now < deadline {
            continue;
        }
        // 間隔の終わり: 記録を書き、ログを捨て (Harness がイベントを溜め続けないように)、大きさを整える
        s.h.clear_log();
        let doc_bytes = s.keep_size();
        let mut v = std::mem::take(&mut s.samples);
        v.sort();
        let t_min = start.elapsed().as_secs_f64() / 60.0;
        let rss = rss_mib();
        let (p50, p99) = (percentile(&v, 0.5), percentile(&v, 0.99));
        if let Some(r) = rss {
            rss_points.push((t_min, r));
        }
        p99s.push(p99.as_secs_f64() * 1e3);
        emit(format!(
            "{{\"kind\":\"window\",\"t_min\":{t_min:.2},\"rss_mib\":{},\"n\":{},\"p50_ms\":{:.3},\"p99_ms\":{:.3},\"doc_bytes\":{doc_bytes},\"overlay\":\"{}\",\"ops\":{}}}",
            rss.map_or("null".to_owned(), |r| format!("{r:.1}")),
            v.len(),
            p50.as_secs_f64() * 1e3,
            p99.as_secs_f64() * 1e3,
            s.h.overlay(),
            s.counts.ops,
        ));
        if now >= deadline {
            break;
        }
        next += interval;
    }

    // 傾きは最初の 10% (ウォームアップ。キャッシュと履歴が上限まで埋まる間) を除いて求める
    let skip = rss_points.len() / 10;
    let rss_slope_per_hour = slope(&rss_points[skip..]) * 60.0;
    let p99_first = p99s.get(skip).copied().unwrap_or(0.0);
    let p99_last = p99s.last().copied().unwrap_or(0.0);
    // 観察の目安 (ゲートではない): 観察した時間の中での RSS の増加 (傾き × 時間) が 5 MiB と最初の RSS の 10% の
    // 大きいほう未満で、最後の p99 が最初の 1.5 倍以下なら「平坦」。短い実行では傾きが揺れるので、増加量で見る
    let span_h = rss_points
        .last()
        .zip(rss_points.get(skip))
        .map_or(0.0, |(l, f)| (l.0 - f.0) / 60.0);
    let rss_first = rss_points.get(skip).map_or(0.0, |p| p.1);
    let growth = rss_slope_per_hour * span_h;
    let flat = growth < (rss_first * 0.1).max(5.0) && p99_last <= p99_first.max(0.001) * 1.5;
    let c = &s.counts;
    emit(format!(
        "{{\"kind\":\"summary\",\"id\":\"BM-306\",\"minutes\":{:.1},\"seed\":{seed},\"rss_first_mib\":{},\"rss_last_mib\":{},\"rss_slope_mib_per_hour\":{rss_slope_per_hour:.2},\"rss_growth_mib\":{growth:.2},\"p99_first_ms\":{p99_first:.3},\"p99_last_ms\":{p99_last:.3},\"flat\":{flat},\"ops\":{},\"undo\":{},\"redo\":{},\"searches\":{},\"saves\":{},\"trims\":{}}}",
        start.elapsed().as_secs_f64() / 60.0,
        rss_points
            .get(skip)
            .map_or("null".to_owned(), |p| format!("{:.1}", p.1)),
        rss_points
            .last()
            .map_or("null".to_owned(), |p| format!("{:.1}", p.1)),
        c.ops,
        c.undo,
        c.redo,
        c.searches,
        c.saves,
        c.trims,
    ));
}
