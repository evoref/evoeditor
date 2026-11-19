//! 標準入力の読込 (docs/03 §9.2.2、01 FR-120。v0.4 の M25 の (8)「標準入力の土台」。公開 ((9)) の試験は `stdin_public.rs`): 64 KiB が揃うまで待つ判定
//! (届き方で結果が変わらない)、確認を出さない読込、保持方式の記録 (ファイルと同じ関数)、上限、取消し・EOF・読めなく
//! なったとき、`LoadIncomplete` の付け方。偽の `StdinSource` (`eve_testkit::FakeStdin`) で、ジョブ (`run_job`) と core の
//! ジョブ表 (`Pending::OpenStdin`) を確かめる。

use std::sync::Arc;

use eve_core::{
    CancelToken, Encoding, FsErrorKind, FsPath, JobKind, JobOutput, JobPorts, JobRequest,
    JobResult, LoadState, OpenOptions, PreservedMap, ReadOnlyReason, ReadOnlyReasons, StdinEnd,
    StdinSource, TextFormat, run_job,
};
use eve_testkit::{FakeStdin, FixedPaths, Harness, ManualClock, MemFs, StdinStep};
use eve_text::TextRead;
use eve_view::{JobId, Revision};

/// `StdinDone` の中身 (比べやすい形)。
#[derive(Debug, PartialEq)]
struct Done {
    text: String,
    format: TextFormat,
    detected: ReadOnlyReasons,
    preserved: PreservedMap,
    /// 開いた量 (`StdinDone.bytes`。ファイルでは大きさ)。
    bytes: u64,
    end: String,
}

fn ports(stdin: Option<Arc<dyn StdinSource>>, clock: ManualClock) -> JobPorts {
    let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
    JobPorts {
        stdin,
        fs,
        clock: Arc::new(clock),
        paths: Arc::new(FixedPaths::default()),
    }
}

fn run_stdin_with(
    stdin: Option<FakeStdin>,
    opts: OpenOptions,
    cancel: &CancelToken,
    clock: ManualClock,
) -> (Done, Vec<JobOutput>) {
    let p = ports(stdin.map(|s| Arc::new(s) as Arc<dyn StdinSource>), clock);
    let req = JobRequest {
        id: JobId(1),
        doc: None,
        base: Revision(0),
        kind: JobKind::OpenStdin { opts },
    };
    let mut out = Vec::new();
    run_job(&req, &p, cancel, &mut |r| out.push(r));
    let last = out.pop().expect("終端の結果");
    let JobResult::StdinDone {
        text,
        format,
        detected,
        preserved,
        bytes,
        end,
    } = last.body
    else {
        panic!("終端は StdinDone: {:?}", last.body);
    };
    let done = Done {
        text: text.slice_to_string(eve_text::ByteRange::all(text.len_bytes())),
        format,
        detected,
        preserved,
        bytes,
        end: format!("{end:?}"),
    };
    (done, out)
}

fn run_stdin(stdin: FakeStdin, opts: OpenOptions) -> (Done, Vec<JobOutput>) {
    run_stdin_with(Some(stdin), opts, &CancelToken::new(), ManualClock::new())
}

fn reasons(rs: &[ReadOnlyReason]) -> ReadOnlyReasons {
    let mut r = ReadOnlyReasons::default();
    for x in rs {
        r.insert(*x);
    }
    r
}

/// ファイルとして同じバイト列を開いた結果 (確認を出さない指定で)。
fn open_as_file(bytes: &[u8], opts: OpenOptions) -> Done {
    let clock = ManualClock::new();
    let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
    fs.add_dir("/w");
    fs.add_file("/w/a.txt", bytes);
    fs.set_chunk_size(1 << 20);
    let p = JobPorts {
        stdin: None,
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
            opts: OpenOptions {
                read_only_requested: true,
                ..opts
            },
        },
    };
    let mut out = Vec::new();
    run_job(&req, &p, &CancelToken::new(), &mut |r| out.push(r));
    let Some(JobResult::Complete {
        text,
        format,
        detected,
        preserved,
        ..
    }) = out.pop().map(|o| o.body)
    else {
        panic!("ファイルの読込は Complete");
    };
    Done {
        text: text.slice_to_string(eve_text::ByteRange::all(text.len_bytes())),
        format,
        detected,
        preserved,
        bytes: bytes.len() as u64,
        end: "file".to_owned(),
    }
}

/// `Shift_JIS` の日本語 (`n` 行)。末尾に、往復しない文字 (NEC 選定 IBM 拡張の 0xED40。読むと U+7E8A、書くと 0xFA5C) を置く。
fn sjis_text(n: usize) -> Vec<u8> {
    let line = "日本語のテキストです。標準入力から読む。\r\n";
    let s = line.repeat(n);
    let (b, _, _) = encoding_rs::SHIFT_JIS.encode(&s);
    let mut v = b.into_owned();
    v.extend_from_slice(&[0xED, 0x40, b'\r', b'\n']);
    v
}

// verifies: AC-FR-120-1
#[test]
fn eof_まで読んだ内容を開き_開いた量を返す() {
    let (d, progress) = run_stdin(
        FakeStdin::chunks(b"hello\nworld\n", 4),
        OpenOptions::default(),
    );
    assert_eq!(d.text, "hello\nworld\n");
    assert_eq!(d.end, "Eof");
    assert_eq!(
        d.detected,
        ReadOnlyReasons::default(),
        "読み終えたので理由は無い"
    );
    assert_eq!(d.format.encoding, Encoding::Utf8);
    assert!(matches!(
        progress.first().map(|o| &o.body),
        Some(JobResult::Progress { bytes: 0, total: 0 })
    ));
    assert_eq!(d.bytes, 12, "開いた量");
}

// verifies: AC-FR-120-5
#[test]
fn 一バイトずつ届いても_まとめて届いても_同じ結果() {
    // 64 KiB を超える Shift_JIS (先頭の 1 バイトだけでは判定できない)
    let bytes = sjis_text(3000);
    assert!(bytes.len() > 64 * 1024);
    let whole = run_stdin(
        FakeStdin::chunks(&bytes, bytes.len()),
        OpenOptions::default(),
    )
    .0;
    let one = run_stdin(FakeStdin::chunks(&bytes, 1), OpenOptions::default()).0;
    let odd = run_stdin(FakeStdin::chunks(&bytes, 4093), OpenOptions::default()).0;
    assert_eq!(whole.format.encoding, Encoding::ShiftJis);
    assert_eq!(one, whole);
    assert_eq!(odd, whole);
    // 短い入力 (64 KiB 未満) は全部で判定する
    let short = sjis_text(10);
    let a = run_stdin(FakeStdin::chunks(&short, 1), OpenOptions::default()).0;
    let b = run_stdin(
        FakeStdin::chunks(&short, short.len()),
        OpenOptions::default(),
    )
    .0;
    assert_eq!(a, b);
    assert_eq!(a.format.encoding, Encoding::ShiftJis);
}

// verifies: AC-FR-120-5
#[test]
fn 判定と保持方式の記録はファイルと同じ() {
    for bytes in [sjis_text(3000), sjis_text(10)] {
        let s = run_stdin(FakeStdin::chunks(&bytes, 1000), OpenOptions::default()).0;
        let f = open_as_file(&bytes, OpenOptions::default());
        assert_eq!(s.text, f.text);
        assert_eq!(s.format, f.format);
        assert_eq!(s.detected, f.detected);
        assert_eq!(
            s.preserved, f.preserved,
            "保持方式の記録はファイルと同じ関数で集める"
        );
        assert!(!s.preserved.is_empty(), "往復しない文字の記録がある");
    }
}

// verifies: AC-FR-120-5
#[test]
fn バイナリと不正バイトの多い内容は確認を出さずに読み取り専用で開く() {
    let mut bin = b"\x00\x01\x02binary".to_vec();
    bin.extend(std::iter::repeat_n(0u8, 100));
    let (d, _) = run_stdin(FakeStdin::chunks(&bin, 7), OpenOptions::default());
    assert_eq!(d.end, "Eof");
    assert!(d.detected.contains(ReadOnlyReason::Binary));
    assert_eq!(d.format.encoding, Encoding::Utf8);
    // UTF-8 の中の不正バイトが 1% を超える
    let mut bad = b"abc".repeat(10);
    bad.extend([0xFF, 0xFE, 0x80, 0x81]);
    let (d, _) = run_stdin(
        FakeStdin::chunks(&bad, 3),
        OpenOptions {
            forced: Some(Encoding::Utf8),
            ..OpenOptions::default()
        },
    );
    assert!(d.detected.contains(ReadOnlyReason::InvalidBytes));
    assert_eq!(d.format.invalid_bytes, 4);
}

// verifies: AC-FR-120-5
#[test]
fn encoding_の指定は判定しない_readonly_は利用者の理由() {
    let (b, _, _) = encoding_rs::EUC_JP.encode("日本語");
    let (d, _) = run_stdin(
        FakeStdin::chunks(&b, 1),
        OpenOptions {
            forced: Some(Encoding::EucJp),
            user_read_only: true,
            ..OpenOptions::default()
        },
    );
    assert_eq!(d.format.encoding, Encoding::EucJp);
    assert_eq!(d.text, "日本語");
    assert_eq!(d.detected, reasons(&[ReadOnlyReason::UserRequested]));
}

fn limited(limit: u64) -> OpenOptions {
    OpenOptions {
        huge_file_warn_bytes: Some(limit),
        ..OpenOptions::default()
    }
}

// verifies: AC-FR-120-4
#[test]
fn 上限は読んだバイト数で_文字の境界の手前まで() {
    // 「あ」は 3 バイト: 100 バイトの上限は 33 文字 (99 バイト) で切る (途中の 1 バイトを不正バイトにしない)
    let bytes = "あ".repeat(100).into_bytes();
    for chunk in [1, 7, 300] {
        let (d, _) = run_stdin(FakeStdin::chunks(&bytes, chunk), limited(100));
        assert_eq!(d.text, "あ".repeat(33), "chunk={chunk}");
        assert_eq!(d.end, "Limit");
        assert_eq!(d.detected, reasons(&[ReadOnlyReason::LoadIncomplete]));
        assert_eq!(d.format.invalid_bytes, 0);
        assert_eq!(d.bytes, 99, "開いた量は切った後");
    }
}

// verifies: AC-FR-120-4
#[test]
fn 上限は復号した本文のバイト数でも見る() {
    // 半角カナ (Shift_JIS の 1 バイト) は UTF-8 で 3 バイト: 90 バイトの上限は 30 文字
    let bytes = vec![0xB1u8; 100];
    let opts = OpenOptions {
        forced: Some(Encoding::ShiftJis),
        ..limited(90)
    };
    let a = run_stdin(FakeStdin::chunks(&bytes, 1), opts).0;
    let b = run_stdin(FakeStdin::chunks(&bytes, 100), opts).0;
    assert_eq!(a, b);
    assert_eq!(a.text, "\u{FF71}".repeat(30));
    assert_eq!(a.end, "Limit");
    assert!(a.detected.contains(ReadOnlyReason::LoadIncomplete));
}

// verifies: AC-FR-120-4
#[test]
fn 上限ちょうどで終わる入力は読み終えた扱い() {
    let bytes = vec![b'a'; 100];
    let (d, _) = run_stdin(FakeStdin::chunks(&bytes, 9), limited(100));
    assert_eq!(d.end, "Eof");
    assert_eq!(d.text.len(), 100);
    assert_eq!(d.detected, ReadOnlyReasons::default());
    // 1 バイト多ければ上限
    let bytes = vec![b'a'; 101];
    let (d, _) = run_stdin(FakeStdin::chunks(&bytes, 9), limited(100));
    assert_eq!(d.end, "Limit");
    assert_eq!(d.text.len(), 100);
}

// verifies: AC-FR-120-4
#[test]
fn 上限が判定の前でも切る() {
    // 上限 (10 バイト) が先頭の 64 KiB より小さい: 判定は貯めた分で行い、本文は上限まで
    let (d, _) = run_stdin(FakeStdin::chunks(&[b'x'; 50], 3), limited(10));
    assert_eq!(d.text, "x".repeat(10));
    assert_eq!(d.end, "Limit");
}

/// 閉じないパイプ: `steps` の後でデータが無いまま待ち、待ったところで取消し (`Esc` の確認の「ここまでで開く」) と
/// `wake` が来る。
fn stopped(steps: Vec<StdinStep>) -> (Done, FakeStdin) {
    let mut steps = steps;
    steps.push(StdinStep::Hang);
    let fake = FakeStdin::new(steps);
    let cancel = CancelToken::new();
    let (c, f) = (cancel.clone(), fake.clone());
    fake.on_hang(move || {
        c.cancel();
        f.wake();
    });
    let d = run_stdin_with(
        Some(fake.clone()),
        OpenOptions::default(),
        &cancel,
        ManualClock::new(),
    )
    .0;
    (d, fake)
}

// verifies: AC-FR-120-4
#[test]
fn 取消しは読んだところまでを返す() {
    let (d, fake) = stopped(vec![
        StdinStep::Data(b"line 1\n".to_vec()),
        StdinStep::Data(b"line 2\n".to_vec()),
    ]);
    assert_eq!(d.text, "line 1\nline 2\n");
    assert_eq!(d.end, "Stopped");
    assert_eq!(d.detected, reasons(&[ReadOnlyReason::LoadIncomplete]));
    assert!(
        fake.reader_dropped(),
        "読むのをやめたら読み手を手放す (eve-stdin を止める契機)"
    );
    assert_eq!(fake.wakes(), 1);
}

// verifies: AC-FR-120-4
#[test]
fn 取消し_失敗は文字の境界で切り_切れた多バイト列を開かない() {
    // 「い」(E3 81 84) の 2 バイトまで届いたところで取り消す
    let (d, _) = stopped(vec![StdinStep::Data(b"\xE3\x81\x82\xE3\x81".to_vec())]);
    assert_eq!(d.text, "あ");
    assert_eq!(d.bytes, 3);
    assert_eq!(
        d.format.invalid_bytes, 0,
        "切れた多バイト列を不正バイトにしない"
    );
    assert_eq!(d.detected, reasons(&[ReadOnlyReason::LoadIncomplete]));
    // 読込の失敗も同じ
    let fake = FakeStdin::new([
        StdinStep::Data(b"\xE3\x81\x82\xE3".to_vec()),
        StdinStep::Error(FsErrorKind::Other),
    ]);
    let (d, _) = run_stdin(fake, OpenOptions::default());
    assert_eq!(
        (d.text.as_str(), d.bytes, d.format.invalid_bytes),
        ("あ", 3, 0)
    );
    // ISO-2022-JP の漢字の組の途中: 漢字の状態のまま組の直後で切る (StatefulEncodingWithInvalidBytes を付けない)
    let opts = OpenOptions {
        forced: Some(Encoding::Iso2022Jp),
        ..OpenOptions::default()
    };
    let fake = FakeStdin::new([
        StdinStep::Data(b"\x1B$BF|K\\8".to_vec()),
        StdinStep::Error(FsErrorKind::Other),
    ]);
    let (d, _) = run_stdin(fake, opts);
    assert_eq!(d.text, "日本");
    assert_eq!(d.bytes, 7);
    assert_eq!(d.detected, reasons(&[ReadOnlyReason::LoadIncomplete]));
    // 何も開かなくても、受け取っていれば途中まで (1 バイトだけ受け取って取り消す)
    let (d, _) = stopped(vec![StdinStep::Data(vec![0xE3])]);
    assert_eq!((d.text.as_str(), d.bytes), ("", 0));
    assert_eq!(d.detected, reasons(&[ReadOnlyReason::LoadIncomplete]));
    // EOF では、ファイルの末尾と同じく不正バイトとして出す
    let (d, _) = run_stdin(
        FakeStdin::chunks(b"\xE3\x81\x82\xE3\x81", 2),
        OpenOptions::default(),
    );
    assert_eq!(d.format.invalid_bytes, 2);
    assert_eq!(d.bytes, 5);
}

/// ISO-2022-JP の、64 バイトより長い漢字の区間 (改行で ASCII に戻る)。
fn iso_long_kanji(lines: usize) -> Vec<u8> {
    let line = format!("{}\n", "漢字仮名交じり文".repeat(8));
    let s = line.repeat(lines);
    let (b, _, _) = encoding_rs::ISO_2022_JP.encode(&s);
    b.into_owned()
}

// verifies: AC-FR-120-4
#[test]
fn iso_2022_jp_の漢字の区間で上限に届いても文字の境界で切る() {
    let bytes = iso_long_kanji(30);
    let opts = |limit| OpenOptions {
        forced: Some(Encoding::Iso2022Jp),
        huge_file_warn_bytes: Some(limit),
        ..OpenOptions::default()
    };
    for limit in [1450u64, 1480, 1451, 1452, 1453, 700, 701] {
        for chunk in [1, 13, 4096] {
            let (d, _) = run_stdin(FakeStdin::chunks(&bytes, chunk), opts(limit));
            assert_eq!(d.end, "Limit", "limit={limit}");
            assert!(
                d.text.len() as u64 <= limit,
                "limit={limit}: {}",
                d.text.len()
            );
            assert!(d.bytes <= limit);
            assert_eq!(
                d.detected,
                reasons(&[ReadOnlyReason::LoadIncomplete]),
                "limit={limit} chunk={chunk}: 保存できない理由が付かない"
            );
            assert_eq!(d.format.invalid_bytes, 0);
            let f = open_as_file(&bytes[..usize::try_from(d.bytes).unwrap()], opts(1 << 30));
            assert_eq!(
                d.text, f.text,
                "開いた量の接頭辞をファイルとして開いた結果と同じ"
            );
            assert_eq!(d.preserved, f.preserved);
        }
    }
    // 上限より短い入力は、EOF で読み終えた扱い
    let (d, _) = run_stdin(FakeStdin::chunks(&bytes, 100), opts(1 << 20));
    assert_eq!(d.end, "Eof");
    assert_eq!(d.detected, ReadOnlyReasons::default());
}

// verifies: AC-FR-120-4
#[test]
fn 判定の後の上限も開いた量の接頭辞と同じ() {
    // 64 KiB を超えてから上限に届く (判定は貯めた 64 KiB で行い、その後に上限で切る)
    let encs: [(Encoding, Vec<u8>); 4] = [
        (Encoding::ShiftJis, sjis_text(3000)),
        (Encoding::Iso2022Jp, iso_long_kanji(600)),
        (Encoding::Utf8, "あ😀a\n".repeat(12000).into_bytes()),
        (Encoding::EucJp, {
            let s = "日本語のテキスト\n".repeat(5000);
            encoding_rs::EUC_JP.encode(&s).0.into_owned()
        }),
    ];
    for (enc, bytes) in encs {
        assert!(bytes.len() > 70_000, "{enc:?}");
        for limit in [70_000u64, 70_001, 70_002, 70_003] {
            let opts = OpenOptions {
                huge_file_warn_bytes: Some(limit),
                ..OpenOptions::default()
            };
            let d = run_stdin(FakeStdin::chunks(&bytes, 4093), opts).0;
            assert_eq!(d.format.encoding, enc);
            assert_eq!(d.end, "Limit");
            let f = open_as_file(
                &bytes[..usize::try_from(d.bytes).unwrap()],
                OpenOptions {
                    forced: Some(enc),
                    ..OpenOptions::default()
                },
            );
            assert_eq!(d.text, f.text, "{enc:?} {limit}");
            assert_eq!(d.format, f.format, "{enc:?} {limit}");
            assert_eq!(d.preserved, f.preserved);
            // 読んだバイト数か復号した本文のバイト数のどちらかが、上限の手前の文字の境界まで来ている
            let text_len = d.text.len() as u64;
            assert!(text_len <= limit && d.bytes <= limit);
            assert!(
                limit - d.bytes <= 3 || limit - text_len <= 3,
                "{enc:?} {limit} bytes={} text={text_len}",
                d.bytes
            );
        }
    }
}

// verifies: AC-FR-120-4
#[test]
fn 何も読まずに取り消すと理由の無い空の文書() {
    let (d, _) = stopped(vec![]);
    assert_eq!(d.text, "");
    assert_eq!(d.end, "Stopped");
    assert_eq!(d.detected, ReadOnlyReasons::default());
}

#[test]
fn 取消しの後に始まったジョブも_stdin_done_を返し_受け渡し待ちの分を取り込む() {
    // 取消しを見た後は、もう届いている分 (受け渡し待ち) をブロックせずに取り込んでから終える (03 §9.2.2 の「取消し」)。
    // 閉じないパイプ (`Hang`) の手前までが届いている分
    let cancel = CancelToken::new();
    cancel.cancel();
    let fake = FakeStdin::new([
        StdinStep::Data(b"ab".to_vec()),
        StdinStep::Data(b"c".to_vec()),
        StdinStep::Hang,
        StdinStep::Data(b"never".to_vec()),
    ]);
    let (d, _) = run_stdin_with(
        Some(fake.clone()),
        OpenOptions::default(),
        &cancel,
        ManualClock::new(),
    );
    assert_eq!(d.end, "Stopped");
    assert_eq!(d.text, "abc");
    assert_eq!(d.bytes, 3);
    assert!(d.detected.contains(ReadOnlyReason::LoadIncomplete));
    assert_eq!(fake.wakes(), 0, "閉じないパイプでは待たない");
}

#[test]
fn 取消しで取り込むのは届いている分だけで_失敗の段の先は読まない() {
    let cancel = CancelToken::new();
    cancel.cancel();
    let fake = FakeStdin::new([
        StdinStep::Data(b"ok".to_vec()),
        StdinStep::Error(FsErrorKind::Other),
        StdinStep::Data(b"later".to_vec()),
    ]);
    let (d, _) = run_stdin_with(
        Some(fake),
        OpenOptions::default(),
        &cancel,
        ManualClock::new(),
    );
    assert_eq!(d.end, "Stopped");
    assert_eq!(d.text, "ok");
}

// verifies: AC-FR-120-4
#[test]
fn 途中で読めなくなったら読んだところまで() {
    let fake = FakeStdin::new([
        StdinStep::Data(b"abc\n".to_vec()),
        StdinStep::Error(FsErrorKind::Other),
    ]);
    let (d, _) = run_stdin(fake, OpenOptions::default());
    assert_eq!(d.text, "abc\n");
    assert!(d.end.starts_with("Error"), "{}", d.end);
    assert_eq!(d.detected, reasons(&[ReadOnlyReason::LoadIncomplete]));
    // 最初から読めない: 理由の無い空の文書
    let (d, _) = run_stdin(
        FakeStdin::new([StdinStep::Error(FsErrorKind::Other)]),
        OpenOptions::default(),
    );
    assert_eq!(d.text, "");
    assert!(d.end.starts_with("Error"));
    assert_eq!(d.detected, ReadOnlyReasons::default());
}

#[test]
fn port_が無い_読み手を_2_回取る_ときは最初から読めない() {
    let (d, _) = run_stdin_with(
        None,
        OpenOptions::default(),
        &CancelToken::new(),
        ManualClock::new(),
    );
    assert!(d.end.starts_with("Error"));
    let fake = FakeStdin::chunks(b"abc", 1);
    let _first = fake.take();
    let (d, _) = run_stdin(fake, OpenOptions::default());
    assert!(d.end.starts_with("Error"));
    assert_eq!(d.text, "");
}

// verifies: AC-FR-120-4
#[test]
fn 進捗は_10_hz_以下で読んだバイト数を送る() {
    let clock = ManualClock::new();
    let mut steps = Vec::new();
    for _ in 0..10 {
        steps.push(StdinStep::Data(vec![b'a'; 10]));
        steps.push(StdinStep::Delay(50));
    }
    let fake = FakeStdin::with_clock(steps, clock.clone());
    let (d, progress) = run_stdin_with(
        Some(fake),
        OpenOptions::default(),
        &CancelToken::new(),
        clock,
    );
    assert_eq!(d.text.len(), 100);
    let bytes: Vec<u64> = progress
        .iter()
        .filter_map(|o| match o.body {
            JobResult::Progress { bytes, total } => {
                assert_eq!(total, 0, "大きさは分からない");
                Some(bytes)
            }
            _ => None,
        })
        .collect();
    // 最初 (0)・100 ms ごと (受け取った量)。開いた量は終端の `bytes`
    assert_eq!(bytes.first(), Some(&0));
    assert!(bytes.last().is_some_and(|&b| b <= 100), "{bytes:?}");
    assert_eq!(d.bytes, 100);
    assert!(bytes.windows(2).all(|w| w[0] <= w[1]), "{bytes:?}");
    assert!(bytes.len() <= 2 + 5, "{bytes:?}");
}

// ---- core のジョブ表 (Pending::OpenStdin) と文書の欄 ----

fn harness(fake: &FakeStdin) -> Harness {
    let mut h = Harness::builder()
        .defer_start()
        .stdin(Arc::new(fake.clone()))
        .build();
    let doc = h.editor.state_mut().open_stdin(OpenOptions::default());
    h.started();
    assert_eq!(h.editor.state().active_document(), Some(doc));
    h
}

#[test]
fn 読み終えた文書は読み込んだだけの印と読んだ量を持つ() {
    let fake = FakeStdin::chunks(b"from stdin\n", 3);
    let mut h = harness(&fake);
    let doc = h.editor.state().active_document().expect("文書");
    {
        let d = h.editor.state().document(doc).expect("文書");
        assert_eq!(d.load_state, LoadState::Loading);
        assert!(d.path.is_none(), "無題の文書");
        assert_eq!(d.stdin.map(|s| s.pristine), Some(false));
    }
    h.run_jobs();
    let d = h.editor.state().document(doc).expect("文書");
    assert_eq!(d.load_state, LoadState::Ready);
    assert_eq!(h.doc_text(), "from stdin\n");
    let s = d.stdin.expect("標準入力の欄");
    assert!(s.pristine);
    assert_eq!(s.bytes_read, 11);
    // 最初の編集で印を外し、Undo でも戻さない (03 §9.2.2 の「最初の編集」)
    h.text("x");
    let d = h.editor.state().document(doc).expect("文書");
    assert_eq!(d.stdin.map(|s| s.pristine), Some(false));
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), "from stdin\n");
    let d = h.editor.state().document(doc).expect("文書");
    assert_eq!(d.stdin.map(|s| s.pristine), Some(false));
}

#[test]
fn esc_は確認を出すだけで標準入力の読込を取り消さない() {
    // 表示中の標準入力の文書の `Esc` は確認 (`ui.confirm.stdin_stop`) を出す。答えるまで取り消さない (03 §9.2.2)。
    // 確認の間に読み終えたら確認を閉じる
    let fake = FakeStdin::chunks(b"abc", 1);
    let mut h = harness(&fake);
    h.keys("esc");
    assert_eq!(h.editor.state().confirm_name(), Some("stdin_stop"));
    h.run_jobs();
    assert_eq!(h.editor.state().confirm_name(), None);
    assert_eq!(fake.wakes(), 0, "取り消していない");
    assert_eq!(h.doc_text(), "abc");
    let doc = h.editor.state().active_document().expect("文書");
    let d = h.editor.state().document(doc).expect("文書");
    assert!(!d.read_only.contains(ReadOnlyReason::LoadIncomplete));
}

#[test]
fn 読込中に閉じると取り消して結果を捨てる() {
    let fake = FakeStdin::new([StdinStep::Data(b"abc".to_vec()), StdinStep::Hang]);
    let mut h = harness(&fake);
    let doc = h.editor.state().active_document().expect("文書");
    h.cmd("buffer.close");
    assert_eq!(fake.wakes(), 1, "取消しで待ちを起こす");
    h.run_jobs();
    assert!(
        h.editor.state().document(doc).is_none(),
        "閉じた文書は戻らない"
    );
    assert!(fake.reader_dropped());
}

#[test]
fn 取り消した読込の_stdin_done_は採用する() {
    // `ManualJobSpawner::cancel` は標準入力の読込をキューに残し、取消しを立てて起こす (終端は StdinDone { Stopped })
    let fake = FakeStdin::new([StdinStep::Data(b"abc".to_vec()), StdinStep::Hang]);
    let mut h = harness(&fake);
    let doc = h.editor.state().active_document().expect("文書");
    // 先に読む: キューのジョブを 1 回走らせるとデータの後の Hang で止まるので、取消しを先に立てる試験は下の 1 つだけ
    h.cancel_jobs();
    h.run_jobs();
    let d = h.editor.state().document(doc).expect("文書");
    assert_eq!(d.load_state, LoadState::Ready);
    // 読む前に取り消したが、もう届いていた分 (閉じないパイプの手前) は取り込む
    assert_eq!(h.doc_text(), "abc");
    assert!(d.read_only.contains(ReadOnlyReason::LoadIncomplete));
}

/// `harness` の標準入力の読込のジョブ (`ManualJobSpawner` が最初に振る番号。`defer_start` なので、ほかのジョブは無い)。
const STDIN_JOB: JobId = JobId(1);

#[test]
fn 一バイト以上読んで取り消した読込の_stdin_done_を採用する() {
    // キューの読込は走らせず、同じ読込を別に走らせた結果 (読んでから取り消した) を、取り消した後に渡す
    let fake = FakeStdin::new([StdinStep::Hang]);
    let mut h = harness(&fake);
    let doc = h.editor.state().active_document().expect("文書");
    h.cancel_jobs();
    let (_, mut out) = {
        let mut steps = vec![StdinStep::Data(b"partial\n".to_vec()), StdinStep::Hang];
        steps.truncate(2);
        let other = FakeStdin::new(steps);
        let cancel = CancelToken::new();
        let (c, f) = (cancel.clone(), other.clone());
        other.on_hang(move || {
            c.cancel();
            f.wake();
        });
        let p = ports(Some(Arc::new(other)), ManualClock::new());
        let req = JobRequest {
            id: STDIN_JOB,
            doc: Some(doc),
            base: Revision(0),
            kind: JobKind::OpenStdin {
                opts: OpenOptions::default(),
            },
        };
        let mut out = Vec::new();
        run_job(&req, &p, &cancel, &mut |r| out.push(r));
        ((), out)
    };
    let last = out.pop().expect("終端");
    assert!(matches!(
        last.body,
        JobResult::StdinDone {
            end: StdinEnd::Stopped,
            ..
        }
    ));
    for o in out {
        h.editor.on_job_result(o);
    }
    h.editor.on_job_result(last);
    let d = h.editor.state().document(doc).expect("文書");
    assert_eq!(d.load_state, LoadState::Ready);
    assert_eq!(h.doc_text(), "partial\n");
    assert!(d.read_only.contains(ReadOnlyReason::LoadIncomplete));
    assert_eq!(d.stdin.map(|s| s.bytes_read), Some(8));
}

#[test]
fn パニックで終わった読込は文書を閉じる() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let mut h = harness(&fake);
    let doc = h.editor.state().active_document().expect("文書");
    h.editor.on_job_result(JobOutput {
        job: STDIN_JOB,
        body: JobResult::Failed {
            err: eve_core::JobError::Panic,
        },
    });
    assert!(
        h.editor.state().document(doc).is_none(),
        "読んだ内容は失い、文書を閉じる"
    );
    let shown = h
        .editor
        .state()
        .active_document()
        .expect("別の文書を表示する");
    assert_ne!(shown, doc);
}

#[test]
fn 読み込んだだけの印はどの_origin_の編集でも_redo_でも外れる() {
    use eve_core::{Document, Origin, StdinDoc, TextFormat, Tick, Transaction};
    use eve_text::{ByteRange, ChangeSet, Edit};
    fn insert(d: &Document) -> ChangeSet {
        ChangeSet::from_sorted_edits(
            &d.snapshot(),
            [Edit {
                range: ByteRange::from(0..0),
                insert: "x".into(),
            }],
        )
        .unwrap()
    }
    let pristine = Some(StdinDoc {
        bytes_read: 3,
        pristine: true,
    });
    // 駆動 API の Ai / External は、まだ `Origin` に無い (その版で足す)。どの `Origin` でも外れることを、今ある種類で確かめる
    let origins = [
        Origin::User,
        Origin::Paste,
        Origin::System,
        Origin::Lsp,
        Origin::Command(eve_view::CommandId::from("edit.test")),
    ];
    for origin in origins {
        let mut d = Document::from_text(eve_text::Text::from("abc"), TextFormat::new_document());
        d.stdin = pristine;
        let tx = Transaction::new(insert(&d), origin.clone(), Tick(0));
        d.apply(&tx).expect("適用");
        assert_eq!(d.stdin.map(|s| s.pristine), Some(false), "{origin:?}");
    }
    // Undo / Redo の適用でも外す (戻さない)
    let mut d = Document::from_text(eve_text::Text::from("abc"), TextFormat::new_document());
    let tx = Transaction::new(insert(&d), Origin::User, Tick(0));
    d.apply(&tx).expect("適用");
    d.undo().expect("Undo");
    d.stdin = pristine;
    d.redo().expect("Redo");
    assert_eq!(d.stdin.map(|s| s.pristine), Some(false), "Redo");
    d.stdin = pristine;
    d.undo().expect("Undo");
    assert_eq!(d.stdin.map(|s| s.pristine), Some(false), "Undo");
}

// ---- 届き方と上限の性質 (proptest) ----

mod props {
    use proptest::prelude::*;

    use super::*;

    /// UTF-8 / `Shift_JIS` / 不正バイトが混ざるバイト列 (判定がどれになっても、復号と記録の整合を見る)。
    fn bytes() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(
            prop_oneof![
                Just(b"a".to_vec()),
                Just(b"\r\n".to_vec()),
                Just("あ".as_bytes().to_vec()),
                Just(vec![0x82, 0xA0]),
                Just(vec![0xB1]),
                Just(vec![0xED, 0x40]),
                Just(vec![0xE3]),
                Just(vec![0xFF]),
            ],
            0..200,
        )
        .prop_map(|v| v.concat())
    }

    /// 文字コード `enc` で書いたバイト列 (UTF-16 は `encoding_rs` が書かないので自分で書く)。`junk` は不正なバイト。
    fn encode(enc: Encoding, parts: &[(String, Option<u8>)]) -> Vec<u8> {
        let mut v = Vec::new();
        for (s, junk) in parts {
            match enc {
                Encoding::Utf16Le => v.extend(s.encode_utf16().flat_map(u16::to_le_bytes)),
                Encoding::Utf16Be => v.extend(s.encode_utf16().flat_map(u16::to_be_bytes)),
                Encoding::Utf8 => v.extend_from_slice(s.as_bytes()),
                Encoding::ShiftJis => v.extend_from_slice(&encoding_rs::SHIFT_JIS.encode(s).0),
                Encoding::EucJp => v.extend_from_slice(&encoding_rs::EUC_JP.encode(s).0),
                Encoding::Iso2022Jp => v.extend_from_slice(&encoding_rs::ISO_2022_JP.encode(s).0),
                _ => unreachable!(),
            }
            v.extend(junk);
        }
        v
    }

    fn text_part() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just("a"),
                Just("\n"),
                Just("\r\n"),
                Just("あ"),
                Just("漢字"),
                Just("\u{FF71}"),
                Just("😀"),
                Just("\u{2460}"),
            ],
            0..12,
        )
        .prop_map(|v| v.concat())
    }

    fn encodings() -> impl Strategy<Value = Encoding> {
        prop_oneof![
            Just(Encoding::Utf8),
            Just(Encoding::ShiftJis),
            Just(Encoding::EucJp),
            Just(Encoding::Iso2022Jp),
            Just(Encoding::Utf16Le),
            Just(Encoding::Utf16Be),
        ]
    }

    /// 終わり方: 0 = 取消し、1 = 読込の失敗、2 = 上限、3 = EOF。`at` は受け取る量 (取消し・失敗) か上限。
    fn run_end(bytes: &[u8], enc: Encoding, how: u8, at: usize, chunk: usize) -> Done {
        let opts = OpenOptions {
            forced: Some(enc),
            ..OpenOptions::default()
        };
        let at = at.min(bytes.len());
        let data: Vec<StdinStep> = bytes[..if how < 2 { at } else { bytes.len() }]
            .chunks(chunk)
            .map(|c| StdinStep::Data(c.to_vec()))
            .collect();
        match how {
            0 => {
                let mut steps = data;
                steps.push(StdinStep::Hang);
                let fake = FakeStdin::new(steps);
                let cancel = CancelToken::new();
                let (c, f) = (cancel.clone(), fake.clone());
                fake.on_hang(move || {
                    c.cancel();
                    f.wake();
                });
                run_stdin_with(Some(fake), opts, &cancel, ManualClock::new()).0
            }
            1 => {
                let mut steps = data;
                steps.push(StdinStep::Error(FsErrorKind::Other));
                run_stdin(FakeStdin::new(steps), opts).0
            }
            2 => {
                run_stdin(
                    FakeStdin::new(data),
                    OpenOptions {
                        huge_file_warn_bytes: Some(at as u64),
                        ..opts
                    },
                )
                .0
            }
            _ => run_stdin(FakeStdin::new(data), opts).0,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// 上限・取消し・失敗で切った結果は、開いた量の接頭辞をファイルとして開いた結果と同じ (本文・不正バイトの数・
        /// 理由・保持方式の記録・形式)。正しいバイト列なら、切るのは最後の文字の境界 (捨てるのは最大 3 バイト)。
        #[test]
        fn 切った結果は開いた量の接頭辞をファイルとして開いた結果と同じ(
            parts in prop::collection::vec(
                (text_part(), prop::option::weighted(0.1, prop_oneof![Just(0xFFu8), Just(0x80), Just(0x1B), Just(0x82), Just(0xE3), Just(0xD8)])),
                1..20,
            ),
            enc in encodings(),
            how in 0u8..4,
            at in 0usize..400,
            chunk in 1usize..40,
        ) {
            let bytes = encode(enc, &parts);
            let valid = parts.iter().all(|(_, j)| j.is_none());
            let d = run_end(&bytes, enc, how, at, chunk);
            let n = usize::try_from(d.bytes).unwrap();
            prop_assert!(n <= bytes.len());
            let f = open_as_file(&bytes[..n], OpenOptions { forced: Some(enc), ..OpenOptions::default() });
            prop_assert_eq!(&d.text, &f.text);
            prop_assert_eq!(&d.format, &f.format);
            prop_assert_eq!(&d.preserved, &f.preserved);
            let mut reasons = d.detected;
            reasons.remove(ReadOnlyReason::LoadIncomplete);
            prop_assert_eq!(reasons, f.detected);
            let received = if how < 2 { at.min(bytes.len()) } else { bytes.len() };
            if how < 2 && valid {
                // 受け取った量から捨てたのは、切れた多バイト列 (最大 3 バイト) だけ
                prop_assert!(received - n <= 3, "received={} n={}", received, n);
            }
            if how == 2 && valid {
                prop_assert!((n as u64) <= at as u64);
                prop_assert!(d.text.len() as u64 <= at as u64);
            }
            if how == 3 {
                prop_assert_eq!(n, bytes.len());
            }
            if d.end != "Eof" && received > 0 {
                prop_assert!(d.detected.contains(ReadOnlyReason::LoadIncomplete));
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn 届き方で結果が変わらず_上限で切った本文は全文の接頭辞(
            b in bytes(),
            chunk in 1usize..50,
            limit in 1u64..600,
            enc in prop::option::of(prop_oneof![Just(Encoding::Utf8), Just(Encoding::ShiftJis)]),
        ) {
            let opts = OpenOptions { forced: enc, ..OpenOptions::default() };
            let full = run_stdin(FakeStdin::chunks(&b, b.len().max(1)), opts).0;
            let split = run_stdin(FakeStdin::chunks(&b, chunk), opts).0;
            prop_assert_eq!(&split, &full);
            let lim = OpenOptions { huge_file_warn_bytes: Some(limit), ..opts };
            let a = run_stdin(FakeStdin::chunks(&b, chunk), lim).0;
            let c = run_stdin(FakeStdin::chunks(&b, b.len().max(1)), lim).0;
            prop_assert_eq!(&a, &c);
            prop_assert!(a.text.len() as u64 <= limit);
            if a.end == "Limit" {
                // 文字コードが同じとき (指定したとき) だけ、全文の接頭辞と比べられる
                if enc.is_some() {
                    prop_assert!(full.text.starts_with(&a.text), "{:?} / {:?}", a.text, full.text);
                }
                prop_assert!(a.detected.contains(ReadOnlyReason::LoadIncomplete) || a.text.is_empty());
            } else {
                prop_assert_eq!(&a.end, "Eof");
            }
        }
    }
}
