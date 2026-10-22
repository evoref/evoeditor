//! `decode_pipeline` のハング・極端な遅さを探す簡易ファザー (docs/09 §3.3 の `fuzz_decode` の補助)。
//! libFuzzer を使えない Windows で、構造を知った入力 (ISO-2022-JP のエスケープ列・UTF-16 の BOM・不正バイト・改行) を大量に流し、
//! 1 入力ごとに時間を測る。普段は `#[ignore]` で、環境変数 `EVE_STRESS_SECS` (既定 20 秒) の間だけ回す。
//!
//! ```text
//! EVE_STRESS_SECS=300 cargo test -p eve-encoding --release --test stress_hang -- --ignored --nocapture
//! ```

#![allow(clippy::print_stdout)]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use eve_encoding::fuzzing::decode_pipeline;

/// 小さな決定的乱数 (xorshift64*)。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// 構造を知った入力を作る。
fn gen_input(r: &mut Rng) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::new();
    // 先頭の制御 4 バイト
    v.extend_from_slice(&[
        r.below(8) as u8,
        r.below(256) as u8,
        r.below(256) as u8,
        r.below(256) as u8,
    ]);
    // 短い入力が多いが、libFuzzer の既定の最大長 (4096) まで届く入力も混ぜる
    let len = if r.below(5) == 0 {
        r.below(4200)
    } else {
        r.below(700)
    };
    let alphabets: [&[u8]; 8] = [
        b"\x1b$B\x1b(B\x1b(J\x1b$@\x1b(I\r\n abc",
        b"\x0e\x0f\x1b\x1b$$((BJI@\n\r",
        &[
            0xFF, 0xFE, 0xFE, 0xFF, 0x00, 0x00, 0xEF, 0xBB, 0xBF, b'a', b'\n',
        ],
        &[
            0x81, 0x40, 0x82, 0xA0, 0x83, 0x80, 0xFC, 0xFD, 0xA0, 0xA1, 0xFE, 0x8E, 0x8F,
        ],
        &[
            0xC2, 0xC3, 0xE3, 0x81, 0x82, 0xF0, 0x9F, 0x98, 0x80, 0x80, 0xBF, 0xC0, 0xF8,
        ],
        &[0x00, 0x01, 0x02, 0x7F, b'\r', b'\n', 0x1b],
        b"\x1b$B$\"$$\x1b(B\n\x1b$B0\xa1\n",
        b"",
    ];
    let a = alphabets[r.below(alphabets.len())];
    let mix = r.below(3);
    for _ in 0..len {
        let b = match (mix, a.is_empty()) {
            (0, false) => a[r.below(a.len())],
            (1, false) if r.below(4) != 0 => a[r.below(a.len())],
            _ => r.below(256) as u8,
        };
        v.push(b);
    }
    v
}

#[test]
#[ignore = "簡易ファザー。EVE_STRESS_SECS の間だけ回す"]
fn ハングや極端な遅さを探す() {
    let secs: u64 = std::env::var("EVE_STRESS_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let seed: u64 = std::env::var("EVE_STRESS_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mut r = Rng(seed);
    let start = Instant::now();
    let mut n = 0u64;
    let mut slowest = Duration::ZERO;
    while start.elapsed() < Duration::from_secs(secs) {
        let input = gen_input(&mut r);
        let (tx, rx) = mpsc::channel();
        let copy = input.clone();
        let t0 = Instant::now();
        // 1 入力を別スレッドで処理し、2 秒で終わらなければハングとして報告する (スレッドは放棄)
        #[allow(clippy::disallowed_methods)]
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| decode_pipeline(&copy));
            let _ = tx.send(r.is_ok());
        });
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(true) => {}
            Ok(false) => {
                println!("PANIC 入力 ({} バイト): {input:02X?}", input.len());
                panic!("decode_pipeline がパニックした");
            }
            Err(_) => {
                println!("HANG (2 秒超) 入力 ({} バイト): {input:02X?}", input.len());
                panic!("decode_pipeline が 2 秒たっても終わらない");
            }
        }
        slowest = slowest.max(t0.elapsed());
        n += 1;
    }
    println!("{n} 入力、最も遅い 1 件 {slowest:?}");
}

/// 短い入力 (最大 4 バイト) を、文字コードごとに全探索する。`last` の付く末尾・チャンクの境目・状態付き符号化の途中で終わる
/// 入力など、乱数では当たりにくい形を漏らさない。進みが 2 秒止まったら、止まった入力を示す。
#[test]
#[ignore = "全探索。数十秒かかる"]
fn 短い入力を全探索してハングを探す() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use eve_encoding::{Decoder, Encoding};

    const ALPHA: [u8; 22] = [
        0x00, 0x0A, 0x0D, 0x0E, 0x0F, 0x1B, b'$', b'(', b'B', b'J', b'I', b'@', b'a', 0x80, 0x8E,
        0x8F, 0xA1, 0xA4, 0x81, 0xFE, 0xFF, 0xC3,
    ];
    let progress = Arc::new(AtomicUsize::new(0));
    let current: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (p2, c2) = (progress.clone(), current.clone());
    let (tx, rx) = mpsc::channel();
    #[allow(clippy::disallowed_methods)]
    std::thread::spawn(move || {
        for enc in Encoding::ALL {
            let mut idx = [0usize; 4];
            for len in 0..=4usize {
                let total = ALPHA.len().pow(len as u32);
                for n in 0..total {
                    let mut m = n;
                    for slot in idx.iter_mut().take(len) {
                        *slot = m % ALPHA.len();
                        m /= ALPHA.len();
                    }
                    let bytes: Vec<u8> = idx[..len].iter().map(|&i| ALPHA[i]).collect();
                    *c2.lock().unwrap() = [&[enc as u8][..], &bytes].concat();
                    for chunk in [0usize, 1, 2, 3] {
                        for pre_cap in [0usize, 1, 64] {
                            let mut d = Decoder::new(enc, false);
                            let mut out = String::with_capacity(pre_cap);
                            let step = if chunk == 0 {
                                bytes.len().max(1)
                            } else {
                                chunk
                            };
                            let mut it = bytes.chunks(step).peekable();
                            if it.peek().is_none() {
                                d.decode(&[], true, &mut out);
                            }
                            while let Some(c) = it.next() {
                                d.decode(c, it.peek().is_none(), &mut out);
                            }
                        }
                    }
                    p2.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        let _ = tx.send(());
    });
    let mut last = (0usize, Instant::now());
    loop {
        if rx.recv_timeout(Duration::from_millis(500)).is_ok() {
            println!(
                "全探索が終わりました ({} 入力)",
                progress.load(Ordering::Relaxed)
            );
            return;
        }
        let now = progress.load(Ordering::Relaxed);
        if now != last.0 {
            last = (now, Instant::now());
        } else if last.1.elapsed() > Duration::from_secs(2) {
            let cur = current.lock().unwrap().clone();
            println!("HANG 文字コード番号 {} 入力 {:02X?}", cur[0], &cur[1..]);
            panic!("進みが止まった");
        }
    }
}

/// `decode_pipeline` 全体 (判定・再判定・復号・往復の検査) を、短い本文 × 先頭の制御の組で全探索する。
#[test]
#[ignore = "全探索。数十秒かかる"]
fn パイプライン全体を短い本文で全探索してハングを探す() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const ALPHA: [u8; 22] = [
        0x00, 0x0A, 0x0D, 0x0E, 0x0F, 0x1B, b'$', b'(', b'B', b'J', b'I', b'@', b'a', 0x80, 0x8E,
        0x8F, 0xA1, 0xA4, 0x81, 0xFE, 0xFF, 0xC3,
    ];
    let progress = Arc::new(AtomicUsize::new(0));
    let current: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (p2, c2) = (progress.clone(), current.clone());
    let (tx, rx) = mpsc::channel();
    #[allow(clippy::disallowed_methods)]
    std::thread::spawn(move || {
        for len in 0..=4usize {
            let total = ALPHA.len().pow(len as u32);
            for n in 0..total {
                let mut m = n;
                let mut body = Vec::with_capacity(len);
                for _ in 0..len {
                    body.push(ALPHA[m % ALPHA.len()]);
                    m /= ALPHA.len();
                }
                // len 4 は組が多いので先頭の制御を絞る
                let (forced, chunks, sniffs): (&[u8], &[u8], &[u8]) = if len == 4 {
                    (&[0, 6], &[0, 1], &[64])
                } else {
                    (&[0, 1, 2, 3, 4, 5, 6, 7], &[0, 1, 2, 3], &[1, 2, 64])
                };
                for &f in forced {
                    for &c in chunks {
                        for &sn in sniffs {
                            let mut input = vec![f, sn, 0, c];
                            input.extend_from_slice(&body);
                            *c2.lock().unwrap() = input.clone();
                            decode_pipeline(&input);
                        }
                    }
                }
                p2.fetch_add(1, Ordering::Relaxed);
            }
        }
        let _ = tx.send(());
    });
    let mut last = (0usize, Instant::now());
    loop {
        if rx.recv_timeout(Duration::from_millis(500)).is_ok() {
            println!(
                "全探索が終わりました ({} 本文)",
                progress.load(Ordering::Relaxed)
            );
            return;
        }
        let now = progress.load(Ordering::Relaxed);
        if now != last.0 {
            last = (now, Instant::now());
        } else if last.1.elapsed() > Duration::from_secs(2) {
            let cur = current.lock().unwrap().clone();
            println!("HANG 入力 {cur:02X?}");
            panic!("進みが止まった");
        }
    }
}

/// fuzz が見つけた入力 (復号したファイル) を Windows でそのまま再生する。環境変数 `EVE_REPRO_FILE` にファイルのパスを渡す。
/// パニックならそのメッセージ、20 秒たっても終わらなければハングとして報告する (libFuzzer と同じ `decode_pipeline` を呼ぶ)。
///
/// ```text
/// set EVE_REPRO_FILE=C:\path\to\timeout-001.bin
/// cargo test -p eve-encoding --release --test stress_hang -- --ignored --nocapture 再生
/// ```
#[test]
#[ignore = "EVE_REPRO_FILE で指したファイルを再生する"]
fn fuzz_が見つけた入力を再生する() {
    let Ok(path) = std::env::var("EVE_REPRO_FILE") else {
        println!("EVE_REPRO_FILE が未設定です");
        return;
    };
    let data = std::fs::read(&path).expect("入力のファイルを読めません");
    println!(
        "入力 {} バイト: 先頭 {:02X?}",
        data.len(),
        &data[..data.len().min(16)]
    );
    let (tx, rx) = mpsc::channel();
    let t0 = Instant::now();
    #[allow(clippy::disallowed_methods)]
    std::thread::spawn(move || {
        let r = std::panic::catch_unwind(|| decode_pipeline(&data));
        let _ = tx.send(r.is_ok());
    });
    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(true) => println!("正常に終わりました ({:?})", t0.elapsed()),
        Ok(false) => panic!("decode_pipeline がパニックした (上のメッセージを参照)"),
        Err(_) => panic!("HANG: 20 秒たっても終わらない"),
    }
}
