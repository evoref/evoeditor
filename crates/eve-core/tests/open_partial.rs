//! 先行表示のジョブの側 (docs/03 §9.2.1、v0.4 の M25 の (1)、01 FR-122 AC-2〜AC-4): 読込が 200 ms を過ぎても終わらない
//! とき、開くジョブは固定の先頭 (全文の接頭辞) を `JobResult::Partial` で 1 回送る。遅い読込は `MemFs` のチャンクごとに
//! `ManualClock` を進めて作る。core の受け手 (表示・できる操作・全文への差し替え) の試験は `partial_view.rs`。

use std::sync::Arc;

use eve_core::{
    Args, CancelToken, CommandResult, Encoding, FsPath, JobKind, JobOutput, JobPorts, JobRequest,
    JobResult, Millis, OpenOptions, TextFormat, run_job,
};
use eve_testkit::{FixedPaths, Harness, ManualClock, MemFs};
use eve_text::{ByteOff, ByteRange, Text, TextRead, is_grapheme_boundary};
use eve_view::{JobId, Revision};
use proptest::prelude::*;

const MIB: usize = 1024 * 1024;

fn s(t: &Text) -> String {
    t.slice_to_string(ByteRange::all(t.len_bytes()))
}

/// `data` を `/w/a.txt` に置き、チャンク `chunk` バイトごとに時計を `per_chunk_ms` 進める読込で開く。
fn open_job(data: &[u8], chunk: usize, per_chunk_ms: u64, opts: OpenOptions) -> Vec<JobOutput> {
    let clock = ManualClock::new();
    let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
    fs.add_dir("/w");
    fs.add_file("/w/a.txt", data);
    fs.set_chunk_size(chunk);
    fs.set_read_delay(Some((clock.clone(), Millis(per_chunk_ms))));
    let ports = JobPorts {
        fs,
        clock: Arc::new(clock),
        paths: Arc::new(FixedPaths::default()),
    };
    let req = JobRequest {
        id: JobId(1),
        doc: None,
        base: Revision(0),
        kind: JobKind::Open {
            path: FsPath::from("/w/a.txt"),
            opts,
        },
    };
    let mut out = Vec::new();
    run_job(&req, &ports, &CancelToken::new(), &mut |r| out.push(r));
    out
}

fn partial() -> OpenOptions {
    OpenOptions {
        partial: true,
        ..OpenOptions::default()
    }
}

/// 送った先頭 (本文と形式)。
fn heads(out: &[JobOutput]) -> Vec<(String, &TextFormat)> {
    out.iter()
        .filter_map(|o| match &o.body {
            JobResult::Partial { head, format } => Some((s(head), format)),
            _ => None,
        })
        .collect()
}

/// 終端の結果の本文 (`Complete` でなければ panic)。
fn full(out: &[JobOutput]) -> String {
    match &out.last().expect("結果がある").body {
        JobResult::Complete { text, .. } => s(text),
        other => panic!("Complete でない: {other:?}"),
    }
}

/// 先頭は全文の接頭辞で、1 MiB 以下で、切る位置は行の境界か書記素の境界で、CR と LF の間でない。
fn assert_head(full: &str, head: &str) {
    assert!(head.len() <= MIB, "{}", head.len());
    assert!(full.starts_with(head), "全文の接頭辞");
    let h = head.len();
    let b = full.as_bytes();
    assert!(
        !(h > 0 && h < b.len() && b[h - 1] == b'\r' && b[h] == b'\n'),
        "CR と LF の間で切らない"
    );
    let at_line = h == 0 || b[h - 1] == b'\n' || b[h - 1] == b'\r' || h == b.len();
    assert!(at_line || is_grapheme_boundary(&Text::from(full), ByteOff(h)));
}

fn lines(n: usize) -> Vec<u8> {
    (0..n)
        .flat_map(|i| format!("line {i:06}\n").into_bytes())
        .collect()
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 先頭は_200_ms_を過ぎた最初のチャンクの区切りで_1_回だけ送る() {
    let data = lines(400_000); // 12 バイトの行。約 4.6 MiB
    let chunk = 512 * 1024;
    let out = open_job(&data, chunk, 60, partial());
    let hs = heads(&out);
    assert_eq!(hs.len(), 1, "1 回だけ");
    let at = out
        .iter()
        .position(|o| matches!(o.body, JobResult::Partial { .. }))
        .unwrap();
    // 60 ms ごと: 4 つ目のチャンク (240 ms) の区切りで送る。その直前の進捗は 4 チャンク分
    assert!(
        matches!(out[at - 1].body, JobResult::Progress { bytes, .. } if bytes == 4 * chunk as u64),
        "{:?}",
        out[at - 1].body
    );
    let (head, format) = &hs[0];
    let all = full(&out);
    assert_head(&all, head);
    assert!(head.ends_with('\n'), "行の区切りで切る");
    assert!(
        head.len() > MIB - 12,
        "1 MiB の中の最後の行の終端: {}",
        head.len()
    );
    assert_eq!(format.encoding, Encoding::Utf8);
    assert_eq!(format.eol_stats.lf, head.len() / 12, "改行の統計は先頭の分");
    assert_eq!(format.roundtrip, Default::default(), "往復の要約は送らない");
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 二百_ms_の前に読み終える読込は先頭を送らない() {
    let data = lines(1000);
    // 3 チャンク × 60 ms = 180 ms
    let out = open_job(&data, data.len() / 3 + 1, 60, partial());
    assert_eq!(heads(&out).len(), 0);
    assert!(matches!(
        out.last().unwrap().body,
        JobResult::Complete { .. }
    ));
    // ちょうど 200 ms の区切りで読み終えた (残りが無い) ときも送らない
    let out = open_job(&data, data.len() / 4, 50, partial());
    assert_eq!(data.len() % 4, 0);
    assert_eq!(heads(&out).len(), 0);
    // 時計が進まない (速い読込)
    let out = open_job(&lines(200_000), 64 * 1024, 0, partial());
    assert_eq!(heads(&out).len(), 0);
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn partial_が立っていなければ送らない() {
    let out = open_job(&lines(200_000), 256 * 1024, 100, OpenOptions::default());
    assert_eq!(heads(&out).len(), 0);
}

#[test]
fn 一_mib_に満たない先頭は復号済みの末尾の行の途中を含めない() {
    // 遅いディスク: 1000 バイトのチャンクに 100 ms。2 つ目の区切りで、復号済みの 2000 バイトの最後の行の終端まで
    let data = lines(1000);
    let out = open_job(&data, 1000, 100, partial());
    let hs = heads(&out);
    assert_eq!(hs.len(), 1);
    assert_eq!(hs[0].0.len(), 2000 / 12 * 12);
    assert_head(&full(&out), &hs[0].0);
}

#[test]
fn crlf_を先頭と残りに分けない() {
    // 5 バイトの行。1 つ目のチャンク (1004 バイト) は CR で終わる
    let data = b"abc\r\n".repeat(2000);
    let out = open_job(&data, 1004, 250, partial());
    let hs = heads(&out);
    assert_eq!(
        hs[0].0.len(),
        1000,
        "末尾の CR は数えず、その前の CRLF の直後"
    );
    assert!(hs[0].0.ends_with("\r\n"));
    assert_eq!(hs[0].1.eol_stats.crlf, 200);
    assert_head(&full(&out), &hs[0].0);
    // CR だけの改行: チャンクの末尾の CR は次が LF かもしれないので数えない
    let data = b"abc\r".repeat(2000);
    let out = open_job(&data, 1004, 250, partial());
    let hs = heads(&out);
    assert_eq!(hs[0].0.len(), 1000);
    assert_eq!(hs[0].1.eol_stats.cr, 250);
    assert_head(&full(&out), &hs[0].0);
}

#[test]
fn 長大行は書記素の境界まで戻す_結合文字と絵文字の列() {
    // 「a + 結合文字」(3 バイト) の繰り返しの 1 行。1 MiB の位置は a と結合文字の間 (2^20 = 3k + 1)
    let data = "a\u{301}".repeat(MIB).into_bytes();
    for chunk in [MIB, 2 * MIB] {
        let out = open_job(&data, chunk, 250, partial());
        let hs = heads(&out);
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].0.len(), MIB - 1, "chunk={chunk}");
        assert_eq!(
            hs[0].1.long_lines, 0,
            "先頭の分 (既定の閾値 1 MiB を超えない)"
        );
        assert_head(&full(&out), &hs[0].0);
    }
    // ZWJ 列 (18 バイト) の繰り返し。1 MiB の位置は列の途中 (2^20 = 18k + 4) なので、列の先頭へ戻す
    let family = "👨\u{200D}👩\u{200D}👧";
    let data = family.repeat(MIB / 9).into_bytes();
    let out = open_job(&data, MIB, 250, partial());
    let hs = heads(&out);
    assert_eq!(hs[0].0.len(), MIB - 4);
    assert_head(&full(&out), &hs[0].0);
}

#[test]
fn 長大行の不正バイトのエスケープ文字も先頭の分を数える() {
    // 200 バイトごとに不正バイト (0xFF) を挟んだ長い 1 行 (比率 0.5% なので確認は出ない)。UTF-8 を指定して開く
    let mut data = Vec::new();
    for _ in 0..12_000 {
        data.extend_from_slice(&[b'x'; 199]);
        data.push(0xFF);
    }
    let opts = OpenOptions {
        forced: Some(Encoding::Utf8),
        ..partial()
    };
    let out = open_job(&data, 256 * 1024, 60, opts);
    let hs = heads(&out);
    assert_eq!(hs.len(), 1);
    let (head, format) = &hs[0];
    let all = full(&out);
    assert_head(&all, head);
    let escapes = head
        .chars()
        .filter(|c| ('\u{10FF00}'..='\u{10FFFF}').contains(c))
        .count();
    assert!(escapes > 0);
    assert_eq!(format.invalid_bytes as usize, escapes, "先頭の分");
    assert_eq!(format.escape_encoding, Some(Encoding::Utf8));
}

#[test]
fn shift_jis_utf16_euc_jp_でも先頭は全文の接頭辞() {
    const JP: &str =
        "日本語のテキストエディタです。これは文章の見本で、文字コードの判定に使います。\r\n";
    // Shift_JIS (判定する)
    let sjis = encoding_rs::SHIFT_JIS
        .encode(&JP.repeat(30_000))
        .0
        .into_owned();
    let out = open_job(&sjis, 256 * 1024, 60, partial());
    let hs = heads(&out);
    assert_eq!(hs.len(), 1);
    assert_eq!(hs[0].1.encoding, Encoding::ShiftJis);
    assert!(hs[0].0.ends_with("\r\n") && hs[0].0.len() > MIB - JP.len());
    assert_head(&full(&out), &hs[0].0);
    // EUC-JP (指定して開く)
    let euc = encoding_rs::EUC_JP
        .encode(&JP.repeat(30_000))
        .0
        .into_owned();
    let opts = OpenOptions {
        forced: Some(Encoding::EucJp),
        ..partial()
    };
    let out = open_job(&euc, 256 * 1024, 60, opts);
    let hs = heads(&out);
    assert_eq!(hs[0].1.encoding, Encoding::EucJp);
    assert_head(&full(&out), &hs[0].0);
    // UTF-16LE の BOM 付き。チャンクの大きさを奇数にして、符号単位の途中でチャンクを切る
    let mut u16 = vec![0xFF, 0xFE];
    for u in "abcあいう\r\n".repeat(200_000).encode_utf16() {
        u16.extend_from_slice(&u.to_le_bytes());
    }
    let out = open_job(&u16, 100_001, 60, partial());
    let hs = heads(&out);
    assert_eq!(hs.len(), 1);
    assert_eq!(hs[0].1.encoding, Encoding::Utf16Le);
    assert!(hs[0].1.has_bom);
    assert!(!hs[0].0.starts_with('\u{FEFF}'), "BOM は本文に入れない");
    assert!(hs[0].0.ends_with("\r\n"));
    assert_head(&full(&out), &hs[0].0);
}

/// 先頭 1.5 MiB が ASCII で、後ろに `Shift_JIS` が現れるファイル (信頼度「低」の UTF-8 と判定し、後で判定し直す)。
fn ascii_then_sjis(ascii_lines: usize, jp: usize) -> Vec<u8> {
    let mut b = b"plain ascii line\n".repeat(ascii_lines);
    b.extend(
        encoding_rs::SHIFT_JIS
            .encode(
                &"日本語のテキストエディタです。これは文章の見本で、文字コードの判定に使います。\n"
                    .repeat(jp),
            )
            .0
            .into_owned(),
    );
    b
}

/// 判定し直したか (完了の `notes.redetected`)。
fn redetected(out: &[JobOutput]) -> Option<Encoding> {
    match &out.last().unwrap().body {
        JobResult::Complete { notes, .. } => notes.redetected,
        _ => None,
    }
}

#[test]
fn 判定のやり直しの後は先頭が揃った時点で送り直す() {
    let data = ascii_then_sjis(92_600, 5000);
    let out = open_job(&data, 256 * 1024, 60, partial());
    assert_eq!(redetected(&out), Some(Encoding::ShiftJis));
    let hs = heads(&out);
    assert_eq!(hs.len(), 2, "最初の読込で 1 回、やり直しで送り直す");
    assert_eq!(hs[0].1.encoding, Encoding::Utf8);
    assert_eq!(hs[1].1.encoding, Encoding::ShiftJis);
    let all = full(&out);
    assert_head(&all, &hs[1].0);
    assert!(hs[1].0.len() > MIB - 17, "揃った先頭: {}", hs[1].0.len());
    // 送り直しは、やり直しの読込が 1 MiB を超えた最初の区切り (5 つ目のチャンク) で、終端の結果より前
    let second = out
        .iter()
        .rposition(|o| matches!(o.body, JobResult::Partial { .. }))
        .unwrap();
    assert!(second + 2 < out.len());
}

#[test]
fn 判定のやり直しが先頭の揃う前に読み終えたら終端の直前に送り直す() {
    // 300 KiB の ASCII の後ろに Shift_JIS。4 つ目のチャンクの区切り (240 ms) で先頭を送った後、5 つ目でやり直す
    let data = ascii_then_sjis(18_100, 1200);
    let out = open_job(&data, 64 * 1024, 60, partial());
    assert_eq!(redetected(&out), Some(Encoding::ShiftJis));
    let hs = heads(&out);
    assert_eq!(hs.len(), 2);
    let all = full(&out);
    assert_eq!(hs[1].0, all, "読み終えた全文 (1 MiB 未満) が先頭");
    assert_eq!(hs[1].1.encoding, Encoding::ShiftJis);
    assert!(
        matches!(out[out.len() - 2].body, JobResult::Partial { .. }),
        "終端の結果の直前"
    );
}

#[test]
fn 先頭を送っていなければ判定のやり直しでも時間の条件のとおり() {
    // 速い読込: どちらの読込でも送らない
    let data = ascii_then_sjis(92_600, 5000);
    let out = open_job(&data, 256 * 1024, 0, partial());
    assert_eq!(redetected(&out), Some(Encoding::ShiftJis));
    assert_eq!(heads(&out).len(), 0);
}

// ---------- core が partial を立てる所と、受け手の待ちの記録 ----------

/// 遅い読込 (64 バイトのチャンクごとに 100 ms) の Harness。
fn slow(h: &Harness) {
    h.fs.set_chunk_size(64);
    h.fs.set_read_delay(Some((h.clock.clone(), Millis(100))));
}

/// ジョブを走らせ、送られた先頭の数を返す。
fn run_count_partials(h: &mut Harness) -> usize {
    let mut n = 0;
    h.run_jobs_with(|o| n += usize::from(matches!(o.body, JobResult::Partial { .. })));
    n
}

// 01 §5.5 の FR-122 AC-4
#[test]
fn file_open_は先頭を送り_再読込と開き直しは送らない() {
    let data = lines(100);
    let mut h = Harness::new().with_file("/w/a.txt", &data);
    slow(&h);
    h.open("/w/a.txt");
    assert_eq!(run_count_partials(&mut h), 1, "新しく開く読込");
    assert_eq!(h.doc_text().as_bytes(), data, "完了で全文");
    h.cmd("file.reload");
    assert_eq!(run_count_partials(&mut h), 0, "再読込");
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    assert_eq!(run_count_partials(&mut h), 0, "開き直し");
}

// 01 §5.5 の FR-122 AC-4
#[test]
fn 起動時のパスは先頭を送る() {
    let mut h = Harness::builder()
        .file("/w/a.txt", &lines(100))
        .file("/w/b.txt", &lines(100))
        .defer_start()
        .build();
    slow(&h);
    h.editor.open_initial_files(
        &[
            eve_core::InitialFile {
                path: "/w/a.txt".into(),
                line: None,
                col: None,
            },
            eve_core::InitialFile {
                path: "/w/b.txt".into(),
                line: None,
                col: None,
            },
        ],
        false,
        None,
    );
    h.started();
    assert_eq!(
        run_count_partials(&mut h),
        2,
        "最初のパスと 2 つ目以降のパス"
    );
}

#[test]
fn 確認からの再投入は先行表示の指定を引き継ぐ() {
    // 不正バイトが多い (確認が出る) ファイル: 確認は最初のチャンクで出るので、その前に先頭は送らない
    let mut data = Vec::new();
    for _ in 0..100 {
        data.extend_from_slice(b"abc \xFF\xFE\n");
    }
    let mut h = Harness::new().with_file("/w/a.txt", &data);
    slow(&h);
    h.open("/w/a.txt");
    assert_eq!(run_count_partials(&mut h), 0);
    assert_eq!(h.overlay(), "dialog");
    h.keys("r");
    assert_eq!(run_count_partials(&mut h), 1, "読み取り専用で開く");
}

#[test]
fn 受け手は先頭を表示し_待ちの記録を残して完了で全文にする() {
    let mut h = Harness::new().with_file("/w/a.txt", b"hello\nworld\n");
    let CommandResult::Pending(job) = h.open("/w/a.txt") else {
        panic!("読込ジョブを投入する");
    };
    let mut format = TextFormat::new_document();
    format.eol_stats.lf = 1;
    h.editor.on_job_result(JobOutput {
        job,
        body: JobResult::Partial {
            head: Text::from("hello\n"),
            format,
        },
    });
    assert_eq!(h.doc_text(), "hello\n", "先頭を表示する");
    assert!(!h.frame_contains("world"));
    // 待ちの記録は残っていて、完了で全文になる
    h.run_jobs();
    assert_eq!(h.doc_text(), "hello\nworld\n");
}

// ---------- 性質 ----------

fn any_char() -> impl Strategy<Value = char> {
    prop_oneof![
        8 => proptest::char::range('a', 'c'),
        3 => Just('\r'),
        3 => Just('\n'),
        3 => Just('あ'),
        2 => Just('漢'),
        2 => Just('\u{301}'),
        1 => Just('\u{3099}'),
        1 => Just('\u{200D}'),
        1 => Just('👨'),
        1 => Just('\u{1F1EF}'),
        1 => Just('\u{0915}'),
        1 => Just('\u{094D}'),
    ]
}

fn encode(text: &str, enc: Encoding) -> Vec<u8> {
    let e = match enc {
        Encoding::ShiftJis => encoding_rs::SHIFT_JIS,
        Encoding::EucJp => encoding_rs::EUC_JP,
        Encoding::Utf16Le => {
            return text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        }
        Encoding::Utf16Be => {
            return text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        }
        _ => return text.as_bytes().to_vec(),
    };
    e.encode(text).0.into_owned()
}

/// 1 件が読込ジョブを 1 回走らせる試験の件数。既定は 96 件、`PROPTEST_CASES` があればその 1/20 (夜間の 10000 なら 500)。
/// `proptest!` のマクロは `PROPTEST_CASES` で件数を上書きする (夜間に時間を超えた、Issue #453) ので、`TestRunner` を
/// 直に使って件数を決める。
fn heavy_config() -> proptest::test_runner::Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map_or(96, |n| (n / 20).max(96));
    proptest::test_runner::Config {
        cases,
        source_file: Some(file!()),
        ..proptest::test_runner::Config::default()
    }
}

/// 先頭は全文の接頭辞で、切る位置は行の境界か書記素の境界で、CR と LF の間でない (チャンクの大きさ・遅さ・文字コード・
/// 不正バイトに依らない)。
#[test]
fn 先頭は全文の接頭辞で行か書記素の境界で切れる() {
    let mut runner = proptest::test_runner::TestRunner::new(heavy_config());
    let strategy = (
        proptest::collection::vec(any_char(), 0..3000),
        prop_oneof![
            Just(Encoding::Utf8),
            Just(Encoding::ShiftJis),
            Just(Encoding::EucJp),
            Just(Encoding::Utf16Le),
            Just(Encoding::Utf16Be),
        ],
        proptest::collection::vec(
            (
                0usize..10_000,
                prop_oneof![Just(0xFFu8), Just(0x80), Just(0xE3)],
            ),
            0..4,
        ),
        1usize..400,
        0u64..120,
    );
    let result = runner.run(&strategy, |(chars, enc, invalid, chunk, per_chunk)| {
        let text: String = chars.into_iter().collect();
        let mut data = encode(&text, enc);
        for (at, b) in invalid {
            let at = at % (data.len() + 1);
            data.insert(at, b);
        }
        let opts = OpenOptions {
            forced: Some(enc),
            read_only_requested: true,
            ..partial()
        };
        let out = open_job(&data, chunk, per_chunk, opts);
        let all = full(&out);
        let hs = heads(&out);
        prop_assert!(hs.len() <= 1, "文字コードを指定した読込は判定し直さない");
        for (head, format) in &hs {
            assert_head(&all, head);
            prop_assert_eq!(format.encoding, enc);
        }
        Ok(())
    });
    if let Err(e) = result {
        panic!(
            "{e}
{runner}"
        );
    }
}
