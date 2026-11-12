//! BM-104 / BM-202: サイズ別オープンの時間とメモリ (docs/09-testing-quality.md §5.2、01 NFR-104 / NFR-202、S-02 の最終判定の材料)。
//! 実ファイル (Windows は `WinFs`、Unix は `UnixFs`) を core の読込ジョブ (`run_job`) で開く。測るのは (2) 「開く」→ 読込完了
//! (`Ready` 相当) の区間で、UTF-8 の 1 MB / 10 MB / 100 MB と `Shift_JIS` の 100 MB。メモリは読込完了から 1 秒アイドルの後の
//! 物理メモリ (Windows はワーキングセット、Linux は VmRSS。01 NFR-201) の増分を、基準 (デコード後の UTF-8 サイズ × 1.5 + 15 MB、
//! 許容 ×2.5) と並べて示す。
//!
//! **診断と記録用**: 判定は基準機 (内蔵 NVMe、09 §5.1 / OQ-23) の値で行う。ここは開発機・CI ランナーの値で、目安にしかならない。
//! (1) の「開く → 最初の進捗の Frame」は、測る区間に進捗表示の Frame を含めないので測らない。
//! Linux の musl 版と gnu 版の比較 (OQ-112、05 §17.3) は、同じ機械で `--target` だけを替えて測る。
//!
//! **v0.4 (参考、09 §5.2 の BM-104 / BM-202 / BM-107e の行)**: 保持方式 (03 §8.7.1) の記録の費用を見る。
//! - BM-104 / BM-202: 区分 B の箇所が密な 10 MB のファイル 3 種 (`-dense`: `Shift_JIS` の重複符号化が各行に 2 つ、EUC-JP の
//!   JIS X 0212 が各行に 2 つ、ISO-2022-JP の旧 JIS のエスケープが各行) と、比べる相手のふつうの `Shift_JIS` の 10 MB を、
//!   上と同じ区間・同じメモリの指標で測る (30 回)。
//! - BM-107e: 保存のジョブ (乾式走査・確認 (i)(ii)・本番の符号化。書く内容は読み捨てる FS に渡す) の時間。`Shift_JIS` の
//!   100 MB で、ふつうの文書 (`-100m-sjis`)、2 行に 1 つ区分 B の文字がある文書を記録を使って (`-dense`)、同じ文書を記録を
//!   使わずに (`-dense-norec`。v0.3 と同じ経路) 書く。
//!
//! ```text
//! cargo bench -p eve-platform --bench bm_104_202
//! cargo bench -p eve-platform --bench bm_104_202 -- --quick   # 回数を減らす動作確認 (記録には使わない)
//! cargo bench -p eve-platform --bench bm_104_202 -- --json    # 1 行ずつの JSON (xtask bench が集める)
//! cargo bench -p eve-platform --bench bm_104_202 --target x86_64-unknown-linux-musl   # OQ-112 の musl 版
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "計測用の数値変換。値は十分に小さい"
)]

#[cfg(any(windows, unix))]
mod imp {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use eve_core::{
        CancelToken, Clock, Expect, FileMeta, FileStamp, FileSystem, FsError, FsErrorKind, FsPath,
        JobKind, JobPorts, JobRequest, JobResult, OpenOptions, PreservedMap, ReadChunks,
        SaveMethod, SaveOptions, SaveReport, SaveStrategy, Tick, run_job,
    };
    use eve_platform::dirs::Dirs;
    use eve_platform::path::PathCtx;
    use eve_view::{JobId, Revision};

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> Tick {
            Tick(0)
        }
    }

    #[cfg(windows)]
    fn ports() -> JobPorts {
        use eve_platform::dirs::WinPaths;
        use eve_platform::fs::WinFs;
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

    #[cfg(unix)]
    fn ports() -> JobPorts {
        use eve_platform::dirs::UnixPaths;
        use eve_platform::unix_fs::UnixFs;
        JobPorts {
            fs: Arc::new(UnixFs::default()),
            clock: Arc::new(FixedClock),
            paths: Arc::new(UnixPaths::new(
                Dirs::resolve(&|_| None),
                PathCtx {
                    cwd: "/".to_owned(),
                    home: None,
                },
                None,
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

    /// 区分 B の箇所が密なファイル (v0.4 の参考)。`corpus` と同じ行に、`every` 行ごとに区分 B の箇所を入れて `enc` で
    /// 書く。`Shift_JIS` は NEC 選定 IBM 拡張の「纊」0xED40 (再符号化すると 0xFA5C)、EUC-JP は JIS X 0212 の「丂」0x8FB0A1
    /// (符号化器が出力できない) を、行頭の番号の後ろ (`per_line` が 2 のとき) と行末に入れる。ISO-2022-JP は全部の行に
    /// 日本語を入れ、`every` 行ごとに漢字の指示を旧 JIS (`ESC $ @`) にする (行の記録)。決定的。
    fn dense(
        enc: &'static encoding_rs::Encoding,
        bytes: usize,
        every: u64,
        per_line: usize,
    ) -> Vec<u8> {
        let mark: &[u8] = if enc == encoding_rs::SHIFT_JIS {
            b"\xED\x40"
        } else {
            b"\x8F\xB0\xA1"
        };
        let mut out = Vec::with_capacity(bytes + 256);
        let mut i = 0u64;
        while out.len() < bytes {
            let hit = i.is_multiple_of(every);
            if enc == encoding_rs::ISO_2022_JP {
                let line = format!("{i:08} 日本語の行です。テキストエディタの試験データ。abc");
                let e = enc.encode(&line).0;
                if hit {
                    // ESC $ B → ESC $ @ (再符号化すると ESC $ B に戻る、区分 D)
                    let mut k = 0;
                    while k < e.len() {
                        if e[k..].starts_with(b"\x1B$B") {
                            out.extend_from_slice(b"\x1B$@");
                            k += 3;
                        } else {
                            out.push(e[k]);
                            k += 1;
                        }
                    }
                } else {
                    out.extend_from_slice(&e);
                }
            } else {
                let line = if i.is_multiple_of(3) {
                    "日本語の行です。テキストエディタの試験データ。0123456789"
                } else {
                    "the quick brown fox jumps over the lazy dog 0123456789 abcdef"
                };
                out.extend_from_slice(format!("{i:08} ").as_bytes());
                if hit && per_line >= 2 {
                    out.extend_from_slice(mark);
                }
                out.extend_from_slice(&enc.encode(line).0);
                if hit {
                    out.extend_from_slice(mark);
                }
            }
            out.push(b'\n');
            i += 1;
        }
        out
    }

    /// 書く内容を読み捨てる `FileSystem` (読込は実 FS に任せる)。保存のジョブの符号化と確認だけを測る (BM-107e)。
    struct DiscardFs {
        inner: Arc<dyn FileSystem>,
    }

    impl FileSystem for DiscardFs {
        fn metadata(&self, path: &FsPath) -> Result<FileMeta, FsError> {
            self.inner.metadata(path)
        }

        fn canonicalize(&self, path: &FsPath) -> Result<FsPath, FsError> {
            self.inner.canonicalize(path)
        }

        fn open_read(&self, path: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
            self.inner.open_read(path)
        }

        fn write_atomic(
            &self,
            path: &FsPath,
            data: &mut dyn ReadChunks,
            _opts: &SaveOptions,
        ) -> Result<SaveReport, FsError> {
            let mut len = 0u64;
            while let Some(c) = data
                .next_chunk()
                .map_err(|_| FsError::new(FsErrorKind::Other))?
            {
                len += c.len() as u64;
            }
            let mut stamp = FileStamp::from_meta(&self.inner.metadata(path)?, None);
            stamp.len = len;
            Ok(SaveReport {
                method: SaveMethod::Atomic,
                new_stamp: stamp,
                warnings: Vec::new(),
            })
        }
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
        out.pop().expect("結果がある").body
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
        let sj10 = encoding_rs::SHIFT_JIS.encode(&u10).0.into_owned();
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
        // v0.4 (参考): 保持方式の記録の費用。区分 B の箇所が密な 10 MB と、比べる相手のふつうの Shift_JIS の 10 MB
        let mut cases = cases;
        let (sjis, euc, iso) = (
            encoding_rs::SHIFT_JIS,
            encoding_rs::EUC_JP,
            encoding_rs::ISO_2022_JP,
        );
        let v04 = [
            (
                "BM-104-10m-sjis",
                "Shift_JIS 10 MB (v0.4 の参考。区分 B の箇所なし)",
                sjis,
                sj10,
            ),
            (
                "BM-104-10m-sjis-dense",
                "Shift_JIS 10 MB、各行に重複符号化 2 つ (v0.4 の参考)",
                sjis,
                dense(sjis, 10 * mb, 1, 2),
            ),
            (
                "BM-104-10m-eucjp-dense",
                "EUC-JP 10 MB、各行に JIS X 0212 2 つ (v0.4 の参考)",
                euc,
                dense(euc, 10 * mb, 1, 2),
            ),
            (
                "BM-104-10m-iso2022jp-dense",
                "ISO-2022-JP 10 MB、各行が旧 JIS のエスケープ (v0.4 の参考)",
                iso,
                dense(iso, 10 * mb, 1, 1),
            ),
        ];
        for (id, label, enc, bytes) in v04 {
            let utf8_len = enc.decode_without_bom_handling(&bytes).0.len();
            cases.push(Case {
                id,
                label,
                target_p50_ms: None,
                utf8_len,
                bytes,
                runs: scale(30, 3),
                warmup: scale(3, 1),
            });
        }

        let ports = ports();
        for c in cases {
            let path = dir.join(format!("{}.txt", c.id));
            std::fs::write(&path, &c.bytes).expect("コーパスを書けません");
            let size = c.bytes.len();
            drop(c.bytes);

            // BM-104: 開く → 読込完了
            let mut times: Vec<Duration> = Vec::with_capacity(c.runs);
            // 保持方式の記録の件数 (v0.4。区分 B の箇所が密なファイルでは 1 件以上あること)
            let mut records = 0usize;
            for i in 0..(c.warmup + c.runs) {
                let t = Instant::now();
                let r = open(&path, &ports);
                let d = t.elapsed();
                let JobResult::Complete { preserved, .. } = &r else {
                    panic!("{}: 開けませんでした", c.id);
                };
                records = preserved.len();
                assert!(
                    records > 0 || !c.id.ends_with("-dense"),
                    "{}: 保持方式の記録が無い",
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
                    "{{\"id\":\"{}\",\"size_bytes\":{size},\"runs\":{},\"p50_ms\":{p50:.1},\"p99_ms\":{p99:.1},\"max_ms\":{max:.1},\"mem_delta_bytes\":{delta},\"mem_limit_bytes\":{limit},\"records\":{records},\"quick\":{quick}}}",
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
                    "{} {}: p50 {p50:.1} ms / p99 {p99:.1} ms / max {max:.1} ms ({} 回、記録 {records} 件) {verdict}",
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

        // BM-107e (v0.4 の参考): 保存のジョブの時間 (書く内容は読み捨てる)
        let save_ports = JobPorts {
            fs: Arc::new(DiscardFs {
                inner: Arc::clone(&ports.fs),
            }),
            clock: Arc::clone(&ports.clock),
            paths: Arc::clone(&ports.paths),
        };
        let normal = encoding_rs::SHIFT_JIS
            .encode(&corpus(100 * mb))
            .0
            .into_owned();
        let saves = [
            ("BM-107e-100m-sjis", normal, true),
            (
                "BM-107e-100m-sjis-dense",
                dense(encoding_rs::SHIFT_JIS, 100 * mb, 2, 1),
                true,
            ),
            (
                "BM-107e-100m-sjis-dense-norec",
                dense(encoding_rs::SHIFT_JIS, 100 * mb, 2, 1),
                false,
            ),
        ];
        for (id, bytes, use_records) in saves {
            let path = dir.join(format!("{id}.txt"));
            std::fs::write(&path, &bytes).expect("コーパスを書けません");
            let size = bytes.len();
            drop(bytes);
            let JobResult::Complete {
                text,
                format,
                preserved,
                ..
            } = open(&path, &ports)
            else {
                panic!("{id}: 開けませんでした");
            };
            // 記録を使う保存は、読込で作った表を渡す (保持が効いている文書を元の文字コードで書く保存と同じ)
            let table = if use_records {
                preserved
            } else {
                PreservedMap::default()
            };
            assert!(
                !table.is_empty() || !id.ends_with("-dense"),
                "{id}: 保持方式の記録が無い"
            );
            let snapshot = text.snapshot();
            let (runs, warmup) = (scale(5, 1), scale(1, 0));
            let mut times: Vec<Duration> = Vec::with_capacity(runs);
            for i in 0..(warmup + runs) {
                let req = JobRequest {
                    id: JobId(2),
                    doc: None,
                    base: Revision(0),
                    kind: JobKind::Save {
                        path: FsPath::new(path.clone()),
                        snapshot: snapshot.clone(),
                        encoding: format.encoding,
                        bom: false,
                        escape: format.save_escape_policy(format.encoding),
                        expected: Expect::Any,
                        strategy: SaveStrategy::default(),
                        clear_readonly: false,
                        create_link_target: false,
                        preserved: table.clone(),
                    },
                };
                let mut out = Vec::new();
                let t = Instant::now();
                run_job(&req, &save_ports, &CancelToken::new(), &mut |r| out.push(r));
                let d = t.elapsed();
                let body = out.pop().expect("結果がある").body;
                assert!(
                    matches!(body, JobResult::Saved { .. }),
                    "{id}: 保存できませんでした: {body:?}"
                );
                if i >= warmup {
                    times.push(d);
                }
            }
            times.sort();
            let (p50, max) = (ms(percentile(&times, 0.50)), ms(*times.last().unwrap()));
            if json {
                println!(
                    "{{\"id\":\"{id}\",\"size_bytes\":{size},\"runs\":{},\"p50_ms\":{p50:.1},\"max_ms\":{max:.1},\"records\":{},\"quick\":{quick}}}",
                    times.len(),
                    table.len()
                );
            } else {
                println!(
                    "{id} 保存のジョブ (Shift_JIS 100 MB、記録 {} 件を使う): p50 {p50:.1} ms / max {max:.1} ms ({} 回、参考)",
                    table.len(),
                    times.len()
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

// Windows と Unix 以外では何もしない (実ファイルの Fs が無い)
fn main() {
    #[cfg(any(windows, unix))]
    imp::main();
}
