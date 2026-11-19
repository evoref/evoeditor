//! BM-106: 長大行 (1 行 10 MB、改行なし) の表示・`End`・横スクロール・垂直スクロール・中央挿入 (docs/09-testing-quality.md §5.2、
//! 01 NFR-106 AC-1 / AC-4)。v0.2 の判定: 閾値 (1 MiB) を超える行は char 近似モード (03 §11.5.1) で扱い、各操作の p99 ≤ 100 ms。
//! v0.4 (M25 の (5)) から書記素の単位の列の索引 (03 §14.4) で扱い、各操作の p99 ≤ 50 ms (Should)。100 ms (Must) は割らない。
//! ASCII / 日本語 / 絵文字混在の 3 種を別のケースとして測る。
//! ヘッドレス (core の区間)。**開発機の値は目安で、判定は基準機で行う**。
//!
//! **v0.4 の M25 の (4) で組み替えた** (09 §5.2 の BM-106 の細目、03 §14.4): 列の索引を要る位置まで遅延して作り、
//! 編集では変更の位置より前を残すので、最も重いのは開いた直後・編集の直後の `End` (行頭、または変更の位置から行末までの
//! 1 回の走査)。そこで次のように測る。
//! - `open`: 開く → 最初の Frame (回ごとに新しく開く)。
//! - `end`: 開いた直後の 1 回目の `End` → Frame (回ごとに新しく開く。開くまでは測らない)。組み替えの前は、索引ができた後の
//!   文書で `Home` → `End` を繰り返し測っていた (行末までの走査を測らない)。
//! - `hscroll` / `vscroll` / `insert_mid`: 1 つの文書で繰り返す (`End` と `Home` を測らずに 1 回してから)。中央挿入は
//!   挿入のたびに Revision が変わり、索引の作り直し (v0.3 までは行の全体、M25 からは変更の位置から画面の分まで) を含む。
//! - `end_after_insert` (参考。判定には使わない): 開いて行の中央に 1 文字挿入した直後の `End` → Frame (回ごとに新しく開く)。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_106
//! cargo bench -p eve-testkit --bench bm_106 -- --quick   # 動作確認 (回数を減らす。記録には使わない)
//! cargo bench -p eve-testkit --bench bm_106 -- --json    # 1 行ずつの JSON
//! cargo bench -p eve-testkit --bench bm_106 -- --only flags     # 1 つのケースだけ
//! cargo bench -p eve-testkit --bench bm_106 -- --no-reference   # 判定の 3 種だけ
//! ```
//!
//! **参考のケース** (M25 の (5) のレビューで足した。判定には使わず、毎版記録する。09 §5.2): 書記素の分割に回る char が
//! 続く実在の文字の行 (韓国語の文・異体字セレクタ付きの絵文字を混ぜた行・国旗・絵文字の ZWJ の列・デーヴァナーガリー・
//! 異体字の IVS)。03 §14.4 の「速さ」のとおり、どれも 100 ms を割らないことを確かめる。

#![allow(clippy::print_stdout, clippy::print_stderr)]
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "計測用の数値変換。値は十分に小さい"
)]

use std::time::{Duration, Instant};

use eve_core::Args;
use eve_testkit::Harness;

const LINE_BYTES: usize = 10 * 1024 * 1024;

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// 文書を開いて最初の Frame まで進めた Harness (ここは測らない)。
fn opened(data: &[u8]) -> Harness {
    let mut h = Harness::new()
        .with_size(120, 40)
        .with_file("/w/long.txt", data);
    h.open("/w/long.txt");
    h.run_jobs();
    let _ = h.frame();
    h
}

/// 行の中央の桁 (書記素数。単位の繰り返しの境目) へ移る (ここは測らない)。
fn goto_mid(h: &mut Harness, unit: &str) {
    let per_unit = eve_text::grapheme_indices(unit).len() as i64;
    let mid_col = (LINE_BYTES / 2 / unit.len()) as i64 * per_unit + 1;
    h.cmd_with(
        "cursor.goto_line",
        Args::new().with_int("line", 1).with_int("col", mid_col),
    );
    let _ = h.frame();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let json = args.iter().any(|a| a == "--json");
    let runs = if quick { 20 } else { 200 };
    // 回ごとに文書を開き直す操作の回数。p99 を意味のある値にするため 100 回 (10 回だと p99 = 最大値で、外れ値 1 回で
    // 合否が変わる)
    let reopen_runs = runs.min(100);

    let only = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|i| args.get(i + 1));
    let no_reference = args.iter().any(|a| a == "--no-reference");
    // (名前, 単位, 参考のケースか)。判定は ascii / japanese / emoji の 3 種 (09 §5.2)
    let cases: [(&str, &str, bool); 9] = [
        ("ascii", "abcdefghij", false),
        ("japanese", "日本語のテキスト", false),
        ("emoji", "a日😀b", false),
        ("korean", "한국어 텍스트입니다. ", true),
        ("emoji_vs16", "a日\u{2764}\u{FE0F}b", true),
        ("flags", "\u{1F1EF}\u{1F1F5}\u{1F1FA}\u{1F1F8}", true),
        ("zwj", "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", true),
        ("devanagari", "नमस्ते क्षत्रिय ", true),
        ("ivs", "葛\u{E0100}", true),
    ];
    for (id, unit, ref_case) in cases {
        if only.is_some_and(|o| o != id) || (no_reference && ref_case) {
            continue;
        }
        let line = unit.repeat(LINE_BYTES / unit.len() + 1);
        let mut data = line.into_bytes();
        data.extend_from_slice(b"\nsecond\n");
        // 開く → 最初の Frame (測るたびに新しい Harness で開く。MemFs の用意は測らない)
        let mut open = Vec::new();
        for _ in 0..reopen_runs {
            let mut h = Harness::new()
                .with_size(120, 40)
                .with_file("/w/long.txt", &data);
            let t = Instant::now();
            h.open("/w/long.txt");
            h.run_jobs();
            let f = h.frame();
            open.push(t.elapsed());
            std::hint::black_box(&f);
        }
        // 開いた直後の 1 回目の End → Frame (行末までの索引の走査を含む)
        let mut end = Vec::new();
        for _ in 0..reopen_runs {
            let mut h = opened(&data);
            let t = Instant::now();
            h.keys("end");
            let f = h.frame();
            end.push(t.elapsed());
            std::hint::black_box(&f);
        }
        // 参考: 中央挿入の直後の End → Frame (変更の位置から行末までの作り直しを含む)
        let mut end_after_insert = Vec::new();
        for _ in 0..reopen_runs {
            let mut h = opened(&data);
            goto_mid(&mut h, unit);
            h.text("x");
            let _ = h.frame();
            let t = Instant::now();
            h.keys("end");
            let f = h.frame();
            end_after_insert.push(t.elapsed());
            std::hint::black_box(&f);
        }
        let mut h = opened(&data);
        drop(data);
        // 組み替えの前と同じ状態 (行末まで索引を作った後) から始める
        h.keys("end");
        let _ = h.frame();
        h.keys("home");
        let _ = h.frame();
        // 横スクロール (右へ 1 文字) → Frame
        let mut hscroll = Vec::new();
        for _ in 0..runs {
            let t = Instant::now();
            h.keys("right");
            let f = h.frame();
            hscroll.push(t.elapsed());
            std::hint::black_box(&f);
        }
        // 垂直スクロール (長い行と次の行の間を行き来) → Frame
        let mut vscroll = Vec::new();
        for i in 0..runs {
            let t = Instant::now();
            h.keys(if i % 2 == 0 { "down" } else { "up" });
            let f = h.frame();
            vscroll.push(t.elapsed());
            std::hint::black_box(&f);
        }
        // 中央挿入 (行の中央に 1 文字) → Frame。挿入のたびに Revision が変わり、列の索引を作り直す
        goto_mid(&mut h, unit);
        let mut insert = Vec::new();
        for _ in 0..runs {
            let t = Instant::now();
            h.text("x");
            let f = h.frame();
            insert.push(t.elapsed());
            std::hint::black_box(&f);
        }
        for (op, mut v) in [
            ("open", open),
            ("end", end),
            ("hscroll", hscroll),
            ("vscroll", vscroll),
            ("insert_mid", insert),
            ("end_after_insert", end_after_insert),
        ] {
            v.sort();
            let (p50, p99, max) = (percentile(&v, 0.5), percentile(&v, 0.99), v[v.len() - 1]);
            // 参考の操作・参考のケースは判定に使わない (09 §5.2)
            let reference = ref_case || op == "end_after_insert";
            if json {
                println!(
                    "{{\"id\":\"BM-106\",\"case\":\"{id}\",\"op\":\"{op}\",\"n\":{},\"p50_ms\":{:.2},\"p99_ms\":{:.2},\"max_ms\":{:.2},\"reference\":{reference},\"quick\":{quick}}}",
                    v.len(),
                    ms(p50),
                    ms(p99),
                    ms(max)
                );
            } else {
                println!(
                    "BM-106 {id:8} {op:16} n={:4} p50 {:7.2} ms  p99 {:7.2} ms  max {:7.2} ms  ({})",
                    v.len(),
                    ms(p50),
                    ms(p99),
                    ms(max),
                    if reference {
                        "参考"
                    } else if ms(p99) <= 100.0 {
                        "目標 p99 ≤ 100 ms → OK"
                    } else {
                        "目標 p99 ≤ 100 ms → 超過"
                    }
                );
            }
        }
    }
    if quick && !json {
        println!("(--quick は動作確認用です。記録には使いません)");
    }
}
