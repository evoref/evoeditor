//! BM-S02: テキストバッファ (ropey 1.6) の暫定判定のマイクロベンチ (docs/09-testing-quality.md §5.2、docs/08-roadmap.md S-02、ADR-0004)。
//!
//! 入力は `xtask gen-corpus` の決定的データを **メモリ上のバイト列** に読んだもの。ファイル I/O・コマンド・Port・core は
//! 経由しない (実ファイルの読込を含む値は M3 の BM-104 / 202)。判定は暫定で、各値が NFR-104 / 105 / 202 の目標値に収まる
//! 見込みであること。
//!
//! 使い方 (基準機 ENV-W1 で、他の重い作業を止めて実行する):
//! ```text
//! cargo xtask gen-corpus --only large-1mb,large-10mb,large-100mb
//! cargo bench -p eve-text --bench bm_s02
//! ```
//! `--quick` を付けると回数を減らす (動作確認用。判定には使わない)。結果は標準出力と `target/bench/bm_s02.json` に出す。
//!
//! 統計: ウォームアップの後に N 回測り、p50 / p99 / 最大値を出す (09 §3.0: p50 / p99 で判定する)。criterion は p99 を
//! 出力しないので使わず、標準ライブラリの `Instant` で測る。メモリは Windows の Private Bytes (`memory-stats` の
//! `virtual_mem` = PagefileUsage)。

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eve_text::{
    ByteOff, ByteRange, ChangeSet, CharIdx, Edit, LineIdx, Snapshot, Text, TextBuilder, TextRead,
};

// ---------- 補助 ----------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// 測定結果 (ナノ秒)。
struct Stats {
    name: String,
    runs: usize,
    p50: f64,
    p99: f64,
    max: f64,
}

impl Stats {
    fn from(name: &str, mut ns: Vec<f64>) -> Stats {
        ns.sort_by(|a, b| a.total_cmp(b));
        let pick = |q: f64| ns[(((ns.len() as f64) * q).ceil() as usize).clamp(1, ns.len()) - 1];
        Stats {
            name: name.to_owned(),
            runs: ns.len(),
            p50: pick(0.50),
            p99: pick(0.99),
            max: ns[ns.len() - 1],
        }
    }
}

fn fmt_ns(ns: f64) -> String {
    if ns >= 1e6 {
        format!("{:.2} ms", ns / 1e6)
    } else if ns >= 1e3 {
        format!("{:.2} us", ns / 1e3)
    } else {
        format!("{ns:.0} ns")
    }
}

/// `f` を `warmup` 回ならしてから `runs` 回測る。`f` の戻り値は、計測区間の外で drop する
/// (rope の解放の時間を、構築や編集の時間に混ぜない)。
fn measure<T>(name: &str, warmup: usize, runs: usize, mut f: impl FnMut(usize) -> T) -> Stats {
    for i in 0..warmup {
        drop(f(i));
    }
    let mut ns = Vec::with_capacity(runs);
    for i in 0..runs {
        let t = Instant::now();
        let v = f(warmup + i);
        ns.push(t.elapsed().as_nanos() as f64);
        drop(v);
    }
    Stats::from(name, ns)
}

/// 1 回が 100 ns に満たない操作用 (Windows の `Instant` の分解能は 100 ns)。`batch` 回まとめて測り、1 回あたりの平均を 1 標本にする。
/// p99 は「バッチの平均」の p99 であって、1 回ごとの p99 ではない。
fn measure_batched(name: &str, batch: usize, runs: usize, mut f: impl FnMut(usize)) -> Stats {
    for i in 0..batch {
        f(i);
    }
    let mut ns = Vec::with_capacity(runs);
    for r in 0..runs {
        let t = Instant::now();
        for i in 0..batch {
            f(r * batch + i);
        }
        ns.push(t.elapsed().as_nanos() as f64 / batch as f64);
    }
    Stats::from(&format!("{name} (バッチ {batch} 回の平均)"), ns)
}

/// プロセスの Private Bytes (Windows: PagefileUsage)。
fn private_bytes() -> usize {
    memory_stats::memory_stats().map_or(0, |m| m.virtual_mem)
}

fn mib(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/generated")
}

fn load(name: &str) -> String {
    let path = corpus_dir().join(name);
    match fs::read(&path) {
        Ok(b) => String::from_utf8(b).expect("コーパスは UTF-8"),
        Err(e) => {
            eprintln!(
                "{} を読めません: {e}\n先に `cargo xtask gen-corpus --only large-1mb,large-10mb,large-100mb` を実行してください",
                path.display()
            );
            std::process::exit(2);
        }
    }
}

/// 64 KiB ずつ (char 境界で切って) 追記して `Text` を作る。読込時の decode 済みチャンクの追記に相当する。
fn build(s: &str) -> Text {
    let mut b = TextBuilder::new();
    let mut pos = 0;
    while pos < s.len() {
        let mut end = (pos + 64 * 1024).min(s.len());
        while !s.is_char_boundary(end) {
            end += 1;
        }
        b.append(&s[pos..end]);
        pos = end;
    }
    b.finish()
}

struct Report {
    rows: Vec<Stats>,
    notes: Vec<(String, String)>,
}

impl Report {
    fn add(&mut self, s: Stats) {
        println!(
            "  {:<46} n={:<6} p50 {:>10}  p99 {:>10}  max {:>10}",
            s.name,
            s.runs,
            fmt_ns(s.p50),
            fmt_ns(s.p99),
            fmt_ns(s.max)
        );
        self.rows.push(s);
    }
    fn note(&mut self, key: &str, value: String) {
        println!("  {key:<46} {value}");
        self.notes.push((key.to_owned(), value));
    }
}

/// 中央またはランダムな位置への 1 文字挿入を `runs` 回行い (ウォームアップ 100 回を除く)、逆順に Undo する。
/// 挿入 = ChangeSet の作成 + 逆変換の作成 + 適用。Undo = 逆変換の逆 (redo 用) の作成 + 逆変換の適用。
/// `hold` なら、各編集の間 Snapshot を保持する (木の共有により、経路のコピーが起きる)。
fn edit_and_undo(rep: &mut Report, raw: &str, label: &str, hold: bool, random: bool, quick: bool) {
    let mut t = build(raw);
    let total = t.len_bytes();
    let mut rng = Rng(11);
    let warmup = 100;
    let runs = if quick { 100 } else { 1_000 };
    let mut inverses: Vec<ChangeSet> = Vec::with_capacity(warmup + runs);
    let mut insert_ns = Vec::with_capacity(runs);
    for i in 0..warmup + runs {
        let chars = t.len_chars();
        let c = if random { rng.below(chars) } else { chars / 2 };
        let at = t.char_to_byte(CharIdx(c)).0; // カーソル位置は既知なので、計測区間の外
        let held = hold.then(|| t.snapshot());
        let start = Instant::now();
        let cs = ChangeSet::from_sorted_edits(
            &t,
            [Edit {
                range: ByteRange::new(ByteOff(at), ByteOff(at)),
                insert: Arc::from("a"),
            }],
        )
        .expect("挿入");
        let inv = cs.invert(&t.snapshot());
        t.apply(&cs).expect("適用");
        let elapsed = start.elapsed().as_nanos() as f64;
        drop(held);
        inverses.push(inv);
        if i >= warmup {
            insert_ns.push(elapsed);
        }
    }
    rep.add(Stats::from(&format!("挿入 {label}"), insert_ns));
    let mut undo_ns = Vec::with_capacity(runs);
    let mut n = inverses.len();
    while let Some(inv) = inverses.pop() {
        n -= 1;
        let held = hold.then(|| t.snapshot());
        let start = Instant::now();
        let redo = inv.invert(&t.snapshot());
        t.apply(&inv).expect("Undo");
        let elapsed = start.elapsed().as_nanos() as f64;
        drop((held, redo));
        // 先頭のウォームアップ分 (最後に Undo する 100 回) は除く
        if n >= warmup {
            undo_ns.push(elapsed);
        }
    }
    rep.add(Stats::from(&format!("Undo {label}"), undo_ns));
    assert_eq!(t.len_bytes(), total, "Undo で元の長さに戻る");
}

// ---------- 本体 ----------

fn main() {
    let quick = std::env::args().any(|a| a == "--quick");
    let mut rep = Report {
        rows: Vec::new(),
        notes: Vec::new(),
    };
    println!(
        "BM-S02 (ropey 1.6、cr_lines のみ){}",
        if quick {
            "  [--quick: 判定には使わない]"
        } else {
            ""
        }
    );

    // ---- 構築時間 (NFR-104: 全読込完了のうち、メモリ上のバイト列からの構築分) ----
    println!("\n[構築: メモリ上の UTF-8 から TextBuilder で構築 (64 KiB ずつ追記)]");
    for (file, label, runs) in [
        ("large-1mb.txt", "1 MB", 200),
        ("large-10mb.txt", "10 MB", 60),
        ("large-100mb.txt", "100 MB", 30),
    ] {
        let s = load(file);
        let runs = if quick { 3 } else { runs };
        rep.add(measure(&format!("構築 {label}"), 2, runs, |_| build(&s)));
        rep.add(measure(
            &format!("参考: std の UTF-8 検証のみ {label}"),
            1,
            runs.min(10),
            |_| {
                std::hint::black_box(std::str::from_utf8(s.as_bytes()).is_ok());
            },
        ));
    }

    // ---- メモリ倍率 (NFR-202: デコード後サイズ × 1.5 + 15 MB 以下、許容 × 2.5) ----
    println!("\n[メモリ: Private Bytes (読込完了から 1 秒アイドルの後)]");
    for (file, label) in [
        ("large-1mb.txt", "1 MB"),
        ("large-10mb.txt", "10 MB"),
        ("large-100mb.txt", "100 MB"),
    ] {
        let before = private_bytes();
        let s = load(file);
        let size = s.len();
        let text = build(&s);
        drop(s);
        std::thread::sleep(Duration::from_secs(1));
        let after = private_bytes();
        let used = after.saturating_sub(before); // 構築では必ず増える (負なら 0 に丸めて、下の表示で気づける)
        if after < before {
            eprintln!(
                "警告: Private Bytes が減っています ({before} -> {after})。測定を見直してください"
            );
        }
        let limit = size as f64 * 1.5 + 15.0 * 1024.0 * 1024.0;
        let tolerance = size as f64 * 2.5 + 15.0 * 1024.0 * 1024.0;
        rep.note(
            &format!("メモリ {label}"),
            format!(
                "{:.1} MiB (サイズの {:.2} 倍。目標 {:.1} MiB、許容 {:.1} MiB) {}",
                mib(used),
                used as f64 / size as f64,
                mib(limit as usize),
                mib(tolerance as usize),
                if (used as f64) <= limit {
                    "目標内"
                } else if (used as f64) <= tolerance {
                    "許容内"
                } else {
                    "超過"
                }
            ),
        );
        std::hint::black_box(&text);
    }

    // ---- 以降は 100 MB の文書で測る ----
    let raw = load("large-100mb.txt");
    let text = build(&raw);
    let lines = text.len_lines();
    let mut rng = Rng(42);
    let n_ops = if quick { 2_000 } else { 20_000 };
    let positions: Vec<usize> = (0..n_ops)
        .map(|_| text.char_to_byte(CharIdx(rng.below(text.len_chars()))).0)
        .collect();
    let line_ids: Vec<usize> = (0..n_ops).map(|_| rng.below(lines)).collect();

    println!("\n[行の計算 (cr_lines 有効・unicode_lines 無効) 100 MB、{lines} 行]");
    rep.add(measure_batched("len_lines", 1_000, 200, |_| {
        std::hint::black_box(text.len_lines());
    }));
    rep.add(measure("line_to_byte", 100, n_ops, |i| {
        std::hint::black_box(text.line_to_byte(LineIdx(line_ids[i % n_ops])));
    }));
    rep.add(measure("byte_to_line", 100, n_ops, |i| {
        std::hint::black_box(text.byte_to_line(ByteOff(positions[i % n_ops])));
    }));
    rep.add(measure(
        "line_end_byte + line_terminator",
        100,
        n_ops,
        |i| {
            let l = LineIdx(line_ids[i % n_ops]);
            std::hint::black_box((text.line_end_byte(l), text.line_terminator(l)));
        },
    ));

    println!("\n[byte ⇔ char ⇔ UTF-16 の変換 100 MB]");
    rep.add(measure("byte_to_char", 100, n_ops, |i| {
        std::hint::black_box(text.byte_to_char(ByteOff(positions[i % n_ops])));
    }));
    rep.add(measure("char_to_byte", 100, n_ops, |i| {
        std::hint::black_box(text.char_to_byte(CharIdx(positions[i % n_ops] / 2)));
    }));
    rep.add(measure("byte_to_utf16", 100, n_ops, |i| {
        std::hint::black_box(text.byte_to_utf16(ByteOff(positions[i % n_ops])));
    }));

    println!("\n[スナップショット (clone)]");
    rep.add(measure_batched(
        "snapshot (O(1) の clone)",
        1_000,
        200,
        |_| {
            std::hint::black_box(text.snapshot());
        },
    ));

    // ---- NFR-105: 中央への 1 文字挿入と Undo (p50 1 ms / p99 5 ms) ----
    // 条件を 4 通りに振る。判定には悪い方 (Snapshot を保持 / ランダムな位置) を使う。
    //   Snapshot を保持: 描画・保存・構文解析などが Snapshot を持っている間の編集 (木の共有で、経路のコピーが起きる)
    //   ランダムな位置: キャッシュが温まっていない編集
    println!(
        "
[編集: 100 MB への 1 文字挿入と Undo (NFR-105: p50 1 ms / p99 5 ms)。判定は 4 通りのうち悪い方]"
    );
    for (label, hold, random) in [
        ("中央・Snapshot なし", false, false),
        ("中央・Snapshot を保持", true, false),
        ("ランダム・Snapshot なし", false, true),
        ("ランダム・Snapshot を保持", true, true),
    ] {
        edit_and_undo(&mut rep, &raw, label, hold, random, quick);
    }
    println!(
        "
[編集: 中央で 1 文字ずつ入力し続ける (Undo なし。木が育つコスト。ChangeSet の作成 + apply)]"
    );
    {
        let mut t = build(&raw);
        let mid = {
            let c = t.len_chars() / 2;
            t.char_to_byte(CharIdx(c)).0
        };
        let runs = if quick { 200 } else { 5_000 };
        let mut at = mid;
        rep.add(measure(
            "連続入力 (ChangeSet 作成 + apply)",
            100,
            runs,
            |_| {
                let cs = ChangeSet::from_sorted_edits(
                    &t,
                    [Edit {
                        range: ByteRange::new(ByteOff(at), ByteOff(at)),
                        insert: Arc::from("a"),
                    }],
                )
                .expect("挿入");
                t.apply(&cs).expect("適用");
                at += 1;
            },
        ));
    }

    // ---- CRLF がチャンクの境目で分割されない保証 (03 §2.3) ----
    println!("\n[CRLF がチャンクの境目で分割されないこと]");
    {
        let crlf = raw.replace('\n', "\r\n");
        let t = build(&crlf);
        let chunks: Vec<&str> = t.chunks(ByteRange::all(t.len_bytes())).collect();
        let split = chunks
            .windows(2)
            .filter(|w| w[0].ends_with('\r') && w[1].starts_with('\n'))
            .count();
        let lf_count = raw.bytes().filter(|&b| b == b'\n').count();
        rep.note("CRLF の 100 MB: チャンク数", chunks.len().to_string());
        rep.note(
            "CRLF の 100 MB: 境目で分割された CRLF",
            format!(
                "{split} 件 {}",
                if split == 0 {
                    "(合格)"
                } else {
                    "(不合格)"
                }
            ),
        );
        rep.note(
            "CRLF の 100 MB: 行数",
            format!(
                "{} (LF の数 {lf_count} + 1 {})",
                t.len_lines(),
                if t.len_lines() == lf_count + 1 {
                    "一致"
                } else {
                    "不一致"
                }
            ),
        );
    }

    // ---- 1 バイト削除を Snapshot で保持したときの未計上メモリ (02 §14.1 MR1) ----
    println!("\n[1 バイト削除を Snapshot で保持したときの未計上メモリ (MR1 の閾値の根拠)]");
    {
        let edits = if quick { 100 } else { 1_000 };
        // 位置 (char 境界) の 1 文字を削除する。コーパスには日本語も含むので、文字の長さをその位置から求める
        let delete_one = |t: &mut Text, at: usize| {
            let head = t.slice_to_string(ByteRange::new(ByteOff(at), ByteOff(at + 4)));
            let len = head.chars().next().map_or(1, char::len_utf8);
            let cs = ChangeSet::from_sorted_edits(
                t,
                [Edit {
                    range: ByteRange::new(ByteOff(at), ByteOff(at + len)),
                    insert: Arc::from(""),
                }],
            )
            .expect("削除");
            t.apply(&cs).expect("適用");
        };
        // 3 通り: (a) 保持しない (対照)、(b) 同じ位置を削除しつつ毎回 Snapshot を保持、(c) ランダムな位置を削除しつつ保持
        for (label, hold, random) in [
            ("保持しない (対照)", false, false),
            ("同じ位置を削除 + Snapshot を保持", true, false),
            ("ランダムな位置を削除 + Snapshot を保持", true, true),
        ] {
            let mut t = build(&raw);
            let mut held: Vec<Snapshot> = Vec::new();
            let mut r = Rng(7);
            std::thread::sleep(Duration::from_millis(300));
            let base = private_bytes();
            for _ in 0..edits {
                if hold {
                    held.push(t.snapshot());
                }
                let chars = t.len_chars();
                let c = if random {
                    r.below(chars - 2)
                } else {
                    chars / 2
                };
                let at = t.char_to_byte(CharIdx(c)).0;
                delete_one(&mut t, at);
            }
            std::thread::sleep(Duration::from_millis(300));
            let grown = private_bytes().saturating_sub(base);
            rep.note(
                &format!("未計上メモリ: {label}"),
                format!(
                    "{:.2} MiB / {edits} 編集 = {:.1} KiB/編集 (保持した Snapshot {} 個)",
                    mib(grown),
                    grown as f64 / edits as f64 / 1024.0,
                    held.len()
                ),
            );
            std::hint::black_box((&t, &held));
        }
    }

    println!(
        "\n[書記素判定 (チャンクをまたぐ)]\n  (未実装: 書記素・単語境界は後のタスク。実装後に BM-S02 へ足す)"
    );

    // ---- JSON ----
    let mut json = String::from(
        "{\n  \"bench\": \"BM-S02\",\n  \"storage\": \"ropey 1.6.1 (cr_lines)\",\n  \"environment\": \"ENV-W1\",\n  \"quick\": ",
    );
    let _ = write!(json, "{quick},\n  \"rows\": [\n");
    for (i, r) in rep.rows.iter().enumerate() {
        let _ = writeln!(
            json,
            "    {{\"name\": \"{}\", \"runs\": {}, \"p50_ns\": {:.0}, \"p99_ns\": {:.0}, \"max_ns\": {:.0}}}{}",
            r.name,
            r.runs,
            r.p50,
            r.p99,
            r.max,
            if i + 1 < rep.rows.len() { "," } else { "" }
        );
    }
    json.push_str("  ],\n  \"notes\": [\n");
    for (i, (k, v)) in rep.notes.iter().enumerate() {
        let _ = writeln!(
            json,
            "    {{\"key\": \"{}\", \"value\": \"{}\"}}{}",
            k,
            v.replace('"', "'"),
            if i + 1 < rep.notes.len() { "," } else { "" }
        );
    }
    json.push_str("  ]\n}\n");
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/bench");
    if fs::create_dir_all(&out).is_ok()
        && fs::write(
            out.join(if quick {
                "bm_s02.quick.json"
            } else {
                "bm_s02.json"
            }),
            json,
        )
        .is_ok()
    {
        println!(
            "\n結果を target/bench/{} に書きました",
            if quick {
                "bm_s02.quick.json"
            } else {
                "bm_s02.json"
            }
        );
    }
}
