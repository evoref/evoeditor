//! BM-104 / BM-202: サイズ別オープンの時間とメモリ (docs/09-testing-quality.md §5.2、01 NFR-104 / NFR-202、S-02 の最終判定の材料)。
//! 実ファイル (`WinFs`) を core の読込ジョブ (`run_job`) で開く。測るのは (2) 「開く」→ 読込完了 (`Ready` 相当) の区間で、
//! UTF-8 の 1 MB / 10 MB / 100 MB と Shift_JIS の 100 MB。メモリは読込完了から 1 秒アイドルの後の物理メモリ (ワーキングセット) の
//! 増分を、基準 (デコード後の UTF-8 サイズ × 1.5 + 15 MB、許容 ×2.5) と並べて示す。
//!
//! **診断と記録用**: 判定は基準機 (内蔵 NVMe、09 §5.1 / OQ-23) の値で行う。ここは開発機・CI ランナーの値で、目安にしかならない。
//! (1) の「開く → 最初の進捗の Frame」は進捗表示の Frame がまだ無いので測らない。
//!
//! ```text
//! cargo bench -p eve-platform --bench bm_104_202
//! cargo bench -p eve-platform --bench bm_104_202 -- --quick   # 回数を減らす動作確認 (記録には使わない)
//! cargo bench -p eve-platform --bench bm_104_202 -- --json    # 1 行ずつの JSON (xtask bench が集める)
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use eve_core::{
        CancelToken, Clock, FsPath, JobKind, JobPorts, JobRequest, JobResult, OpenOptions, Tick,
        run_job,
    };
    use eve_platform::dirs::{Dirs, WinPaths};
    use eve_platform::fs::WinFs;
    use eve_platform::path::PathCtx;
    use eve_view::{JobId, Revision};

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> Tick {
            Tick(0)
        }
    }

    fn ports() -> JobPorts {
        JobPorts {
            fs: Arc::new(WinFs::default()),
            clock: Arc::new(FixedClock),
            paths: Arc::new(WinPaths::new(
                Dirs::resolve(&|_| None),
                PathCtx {
                    cwd: "C:\\".to_owned(),
                    home: None,
                },
            )),
        }
    }

    /// 平均 80 B の行 (ASCII と日本語が混ざる) を `bytes` 以上になるまで並べた UTF-8。決定的。
    fn corpus(bytes: usize) -> String {
        let mut out = String::with_capacity(bytes + 128);
        let mut i = 0u64;
        while out.len() < bytes {
            if i.is_multiple_of(3) {
                out.push_str(&format!(
                    "{i:08} 日本語の行です。テキストエディタの試験データ。0123456789\n"
                ));
            } else {
                out.push_str(&format!(
                    "{i:08} the quick brown fox jumps over the lazy dog 0123456789 abcdef\n"
                ));
            }
            i += 1;
        }
        out
    }

    fn open(path: &Path, ports: &JobPorts) -> JobResult {
        let req = JobRequest {
            id: JobId(1),
            doc: None,
            base: Revision(0),
            kind: JobKind::Open {
                path: FsPath::new(path.to_path_buf()),
                opts: OpenOptions::default(),
            },
        };
        let mut out = Vec::new();
        run_job(&req, ports, &CancelToken::new(), &mut |r| out.push(r));
        out.pop().expect("結果がある")
    }

    fn percentile(sorted: &[Duration], p: f64) -> Duration {
        let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    fn ms(d: Duration) -> f64 {
        d.as_secs_f64() * 1000.0
    }

    /// 物理メモリ (ワーキングセット) のバイト数。
    fn rss() -> u64 {
        memory_stats::memory_stats().map_or(0, |m| m.physical_mem as u64)
    }

    struct Case {
        id: &'static str,
        label: &'static str,
        /// 目標 (p50 ms)。無ければ記録のみ。
        target_p50_ms: Option<f64>,
        bytes: Vec<u8>,
        /// デコード後の UTF-8 のサイズ (NFR-202 の基準)。
        utf8_len: usize,
        runs: usize,
        warmup: usize,
    }

    pub fn main() {
        let args: Vec<String> = std::env::args().collect();
        // 子プロセス: 1 つのファイルを開いて保持し、1 秒アイドルの後の物理メモリの増分を出す (BM-202。
        // 他のケースや計測の繰り返しで残ったヒープが増分を小さく見せないよう、ケースごとに新しいプロセスで測る)
        if let Some(i) = args.iter().position(|a| a == "--mem-file") {
            let path = PathBuf::from(&args[i + 1]);
            let ports = ports();
            let before = rss();
            let held = open(&path, &ports);
            std::thread::sleep(Duration::from_secs(1));
            let after = rss();
            assert!(matches!(held, JobResult::Complete { .. }));
            println!("{}", after.saturating_sub(before));
            return;
        }
        let quick = args.iter().any(|a| a == "--quick");
        let json = args.iter().any(|a| a == "--json");
        // 回数は 09 §5.2: 1 MB / 10 MB は 1,000 回 (ウォームアップ 100 回を除く)、100 MB は 30 回
        let scale = |full: usize, q: usize| if quick { q } else { full };

        let dir: PathBuf = std::env::temp_dir().join(format!("eve-bm-104-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("作業ディレクトリ");

        let mb = 1024 * 1024;
        let u1 = corpus(mb);
        let u10 = corpus(10 * mb);
        let u100 = corpus(100 * mb);
        let sj100 = {
            let (b, _, _) = encoding_rs::SHIFT_JIS.encode(&u100);
            b.into_owned()
        };
        let cases = vec![
            Case {
                id: "BM-104-1m",
                label: "UTF-8 1 MB",
                target_p50_ms: None,
                utf8_len: u1.len(),
                bytes: u1.into_bytes(),
                runs: scale(1000, 50),
                warmup: scale(100, 5),
            },
            Case {
                id: "BM-104-10m",
                label: "UTF-8 10 MB",
                target_p50_ms: Some(150.0),
                utf8_len: u10.len(),
                bytes: u10.into_bytes(),
                runs: scale(1000, 20),
                warmup: scale(100, 3),
            },
            Case {
                id: "BM-104-100m",
                label: "UTF-8 100 MB",
                target_p50_ms: Some(1000.0),
                utf8_len: u100.len(),
                bytes: u100.clone().into_bytes(),
                runs: scale(30, 3),
                warmup: scale(3, 1),
            },
            Case {
                id: "BM-104-100m-sjis",
                label: "Shift_JIS 100 MB (再エンコード照合込み)",
                target_p50_ms: Some(1500.0),
                utf8_len: u100.len(),
                bytes: sj100,
                runs: scale(30, 3),
                warmup: scale(3, 1),
            },
        ];
        drop(u100);

        let ports = ports();
        for c in cases {
            let path = dir.join(format!("{}.txt", c.id));
            std::fs::write(&path, &c.bytes).expect("コーパスを書けません");
            let size = c.bytes.len();
            drop(c.bytes);

            // BM-104: 開く → 読込完了
            let mut times: Vec<Duration> = Vec::with_capacity(c.runs);
            for i in 0..(c.warmup + c.runs) {
                let t = Instant::now();
                let r = open(&path, &ports);
                let d = t.elapsed();
                assert!(
                    matches!(r, JobResult::Complete { .. }),
                    "{}: 開けませんでした",
                    c.id
                );
                drop(r);
                if i >= c.warmup {
                    times.push(d);
                }
            }
            times.sort();
            let (p50, p99, max) = (
                ms(percentile(&times, 0.50)),
                ms(percentile(&times, 0.99)),
                ms(*times.last().unwrap()),
            );

            // BM-202: 読込完了から 1 秒アイドルの後の物理メモリの増分 (結果を保持したまま、新しいプロセスで測る)
            let delta: u64 =
                std::process::Command::new(std::env::current_exe().expect("自分の実行ファイル"))
                    .arg("--mem-file")
                    .arg(&path)
                    .output()
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .and_then(|t| t.trim().parse().ok())
                    .unwrap_or(0);
            let limit = (c.utf8_len as f64 * 1.5) as u64 + 15 * 1024 * 1024;

            if json {
                println!(
                    "{{\"id\":\"{}\",\"size_bytes\":{size},\"runs\":{},\"p50_ms\":{p50:.1},\"p99_ms\":{p99:.1},\"max_ms\":{max:.1},\"mem_delta_bytes\":{delta},\"mem_limit_bytes\":{limit},\"quick\":{quick}}}",
                    c.id,
                    times.len()
                );
            } else {
                let verdict = c.target_p50_ms.map_or(String::from("(記録のみ)"), |t| {
                    format!(
                        "目標 p50 {t:.0} ms → {}",
                        if p50 <= t { "OK" } else { "超過" }
                    )
                });
                println!(
                    "{} {}: p50 {p50:.1} ms / p99 {p99:.1} ms / max {max:.1} ms ({} 回) {verdict}",
                    c.id,
                    c.label,
                    times.len()
                );
                println!(
                    "    メモリ: 増分 {:.1} MB (基準 {:.1} MB 以下 → {}、許容 ×2.5 = {:.1} MB)",
                    delta as f64 / 1e6,
                    limit as f64 / 1e6,
                    if delta <= limit { "OK" } else { "超過" },
                    limit as f64 * 2.5 / 1e6
                );
            }
            let _ = std::fs::remove_file(&path);
        }
        let _ = std::fs::remove_dir_all(&dir);
        if quick && !json {
            println!("(--quick は動作確認用です。記録には使いません)");
        }
    }
}

// Windows 以外では何もしない (実ファイルの `WinFs` は Windows 専用)
fn main() {
    #[cfg(windows)]
    imp::main();
}
