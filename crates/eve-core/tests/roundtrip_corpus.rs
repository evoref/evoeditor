//! 往復コーパス (docs/09 §4 の「文字コード」「改行」。01 NFR-303 / NFR-304、03 §8.3 / §8.6 / §8.7 / §8.7.1)。
//! バイト列 → 開く → 1 文字入力 → 保存 で、入力した 1 文字を除いて元のバイト列に戻ること、不正バイトは読み取り専用で、
//! 名前を付けて保存でバイト一致することを、表の全件で確かめる。
//!
//! - 区分 A: そのままバイト一致 (記録は要らない)。
//! - 区分 B (v0.4 の保持方式。03 §8.6 E2): 検出され (件数)、記録され、編集していない範囲は元のバイト列のまま書かれる。
//!   上書き保存も同じ文字コードの名前を付けて保存も、保存の前の確認は出ない。
//! - 区分 D (ISO-2022-JP): ASCII 状態で始まり終わる行は記録されて元のバイト列のまま。そうでない行 (保持できなかった行)
//!   は、保存の前の確認の後、その行だけ正規形で書かれ、ほかの行は元のバイト列のまま。

use eve_core::{Args, Document, Encoding, OpenOptions, ReadOnlyReason};
use eve_encoding::PreserveStatus;
use eve_testkit::Harness;
use eve_view::{Answer, InputEvent, OverlayModel, Query};

/// 起動していない Harness (文書は `open_as` で開く)。
fn env() -> Harness {
    Harness::builder().defer_start().build()
}

fn run(h: &mut Harness, id: &str, args: Args) {
    h.cmd_with(id, args);
    h.run_jobs();
}

/// 文字コードを指定して開く。確認ダイアログ (不正バイトが多い等) が出たら読み取り専用で開く。
fn open_as(h: &mut Harness, path: &str, enc: Option<Encoding>) {
    h.editor.state_mut().open_path(
        path,
        OpenOptions {
            forced: enc,
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    if h.overlay() == "dialog" {
        let w = h.editor.state().first_window().unwrap();
        h.editor.handle_input(w, InputEvent::Text("r".into()));
        h.run_jobs();
    }
}

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

fn type_text(h: &mut Harness, s: &str) {
    let w = h.editor.state().first_window().unwrap();
    h.editor.handle_input(w, InputEvent::Text(s.into()));
}

/// 区分 A: 開いて 1 文字足して保存すると、足した 1 文字以外は元のバイト列のまま。
fn assert_lossless(enc: Encoding, orig: &[u8], label: &str) {
    let mut s = env();
    let mut bytes = b"a".to_vec();
    bytes.extend_from_slice(orig);
    bytes.extend_from_slice(b"b");
    s.fs.add_file("/w/c.txt", &bytes);
    open_as(&mut s, "/w/c.txt", Some(enc));
    assert_eq!(doc(&s).format.encoding, enc, "{label}");
    assert_eq!(
        doc(&s).format.invalid_bytes,
        0,
        "{label}: 不正バイトにならない"
    );
    assert_eq!(
        doc(&s).format.roundtrip.lossy_lines,
        0,
        "{label}: 区分 B に数えない"
    );
    assert!(doc(&s).is_editable(), "{label}");
    type_text(&mut s, "x");
    run(&mut s, "file.save", Args::new());
    let mut want = b"x".to_vec();
    want.extend_from_slice(&bytes);
    assert_eq!(s.fs.read("/w/c.txt").unwrap(), want, "{label}");
}

#[test]
fn shift_jis_の_cp932_互換文字は区分_a_でバイト一致する() {
    for (b, name) in [
        (&[0x81u8, 0x60][..], "0x8160"),
        (&[0x81, 0x61], "0x8161"),
        (&[0x81, 0x7C], "0x817C"),
        (&[0x81, 0x91], "0x8191"),
        (&[0x81, 0x92], "0x8192"),
        (&[0x81, 0xCA], "0x81CA"),
        (&[0x81, 0x5C], "0x815C"),
        (&[0xFA, 0x55], "0xFA55"),
        (&[0xB1], "半角カナ 0xB1"),
    ] {
        assert_lossless(Encoding::ShiftJis, b, name);
    }
}

// verifies: AC-FR-109-2
#[test]
fn shift_jis_の_cp932_外字は往復し_外字以外の編集でも不変() {
    for lead_trail in [
        [0xF0u8, 0x40],
        [0xF0, 0x7E],
        [0xF0, 0x80],
        [0xF0, 0x9F],
        [0xF0, 0xFC],
        [0xF1, 0x40],
        [0xF5, 0x80],
        [0xF9, 0xFC],
    ] {
        assert_lossless(
            Encoding::ShiftJis,
            &lead_trail,
            &format!("外字 {:02X}{:02X}", lead_trail[0], lead_trail[1]),
        );
    }
}

/// 保存の前の確認の題。確認が無ければ `None`。
fn confirm_title(h: &Harness) -> Option<String> {
    let window = h.editor.state().first_window()?;
    match h.query(&Query::Overlay { window }).ok()? {
        Answer::Overlay(Some(OverlayModel::Confirm { title, .. })) => {
            Some(title.spans.iter().map(|sp| sp.text.as_str()).collect())
        }
        _ => None,
    }
}

fn save_as(h: &mut Harness, path: &str) {
    run(h, "file.save_as", Args::new().with_str("path", path));
}

/// 区分 B / D の検出の数 (復号専用の補助漢字・半角カナは `unmappable_lines`、再符号化で別のバイトになるものは `lossy_lines`)。
fn detected_lines(d: &Document) -> u64 {
    let r = &d.format.roundtrip;
    u64::from(r.lossy_lines) + u64::from(r.unmappable_lines)
}

/// 区分 B: `orig` (再符号化で別のバイト列になる 1 文字) を、編集しない所に置いて開く。
/// (1) 同じ行の先頭に 1 文字入力、(2) 別の行 (改行は CRLF) に 1 文字入力、のどちらも、入力した 1 文字を除いて元のバイト列と
/// 一致し (上書き保存と同じ文字コードの名前を付けて保存)、確認は出ない。検出の件数と記録は残る。
fn assert_preserved_b(enc: Encoding, orig: &[u8], label: &str) {
    // (1) 同じ行の、記録の外 (行頭) に入力
    let mut bytes = b"a".to_vec();
    bytes.extend_from_slice(orig);
    bytes.extend_from_slice(b"b");
    let mut s = env();
    s.fs.add_file("/w/c.txt", &bytes);
    open_as(&mut s, "/w/c.txt", Some(enc));
    assert_eq!(doc(&s).format.encoding, enc, "{label}");
    assert_eq!(doc(&s).format.invalid_bytes, 0, "{label}: 不正バイトでない");
    assert!(detected_lines(doc(&s)) > 0, "{label}: 区分 B として検出");
    assert_eq!(
        doc(&s).format.roundtrip.preserve,
        PreserveStatus::Active { unpreserved: 0 },
        "{label}: 保持が効く"
    );
    assert!(!doc(&s).preserved().is_empty(), "{label}: 記録がある");
    assert!(doc(&s).is_editable(), "{label}");
    type_text(&mut s, "x");
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.overlay(), "", "{label}: 確認を出さない");
    let want = [b"x".as_slice(), &bytes].concat();
    assert_eq!(s.fs.read("/w/c.txt").unwrap(), want, "{label}: 上書き保存");
    save_as(&mut s, "/w/d.txt");
    assert_eq!(s.overlay(), "", "{label}: 名前を付けて保存も確認なし");
    assert_eq!(
        s.fs.read("/w/d.txt").unwrap(),
        want,
        "{label}: 名前を付けて保存"
    );
    assert_eq!(
        doc(&s).format.roundtrip.preserve,
        PreserveStatus::Active { unpreserved: 0 },
        "{label}: 保存の後も記録は残る"
    );

    // (2) 別の行 (2 行目。CRLF) の行頭に入力
    let mut bytes = b"a".to_vec();
    bytes.extend_from_slice(orig);
    bytes.extend_from_slice(b"b\r\nedit\r\n");
    let mut s = env();
    s.fs.add_file("/w/c.txt", &bytes);
    open_as(&mut s, "/w/c.txt", Some(enc));
    s.keys("ctrl+home");
    s.keys("down");
    type_text(&mut s, "x");
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.overlay(), "", "{label}: 別の行でも確認なし");
    let mut want = b"a".to_vec();
    want.extend_from_slice(orig);
    want.extend_from_slice(b"b\r\nxedit\r\n");
    assert_eq!(s.fs.read("/w/c.txt").unwrap(), want, "{label}: 別の行");
}

// 01 §5.5 の NFR-303 AC-5 / AC-6 (v0.4 の AC は `tests/ac-checklist.toml` の対象外なので、要件 ID で印を付ける)
// verifies: NFR-303
#[test]
fn shift_jis_の重複符号化は区分_b_として検出され_元のバイト列で保存される() {
    // NEC/IBM 拡張の重複 (¬ の 0xEEF9 / 0xFA54)、NEC 特殊文字の ≒ 0x8790、NEC 選定 IBM 拡張の 0xED40 / 0xEEEF
    // (どれも再符号化で別のバイトになる)
    for (b, name) in [
        (&[0xEEu8, 0xF9][..], "0xEEF9"),
        (&[0xFA, 0x54], "0xFA54"),
        (&[0x87, 0x90], "0x8790"),
        (&[0xED, 0x40], "0xED40"),
        (&[0xEE, 0xEF], "0xEEEF"),
        (&[0xFA, 0x54, 0xFA, 0x54], "0xFA54 の連続"),
        (
            &[0xEE, 0xF9, b'-', 0xFA, 0x54],
            "0xEEF9 と 0xFA54 の間に ASCII",
        ),
    ] {
        assert_preserved_b(Encoding::ShiftJis, b, name);
    }
}

// 01 §5.5 の NFR-303 AC-5 / AC-6 (v0.4 の AC は `tests/ac-checklist.toml` の対象外なので、要件 ID で印を付ける)
// verifies: NFR-303
#[test]
fn euc_jp_の補助漢字と別のコードに写る文字は区分_b_として検出され_元のバイト列で保存される() {
    // JIS X 0212 の補助漢字 (丂 0x8FB0A1。v0.3 までは表現不能で止まった)、0x8FA2AF (˘)、0x8FA2B7 (～ U+FF5E。再符号化で
    // 0xA1C1 になる)、補助漢字の並び
    for (b, name) in [
        (&[0x8Fu8, 0xB0, 0xA1][..], "0x8FB0A1"),
        (&[0x8F, 0xA2, 0xAF], "0x8FA2AF"),
        (&[0x8F, 0xA2, 0xB7], "0x8FA2B7"),
        (&[0x8F, 0xB0, 0xA1, 0x8F, 0xA2, 0xB7], "補助漢字の並び"),
    ] {
        assert_preserved_b(Encoding::EucJp, b, name);
    }
}

#[test]
fn 単独バイトは不正バイトとして読み取り専用で_名前を付けて保存でバイト一致する() {
    // 0xA0 / 0xFD / 0xFE / 0xFF は Shift_JIS の単独バイトとして不正
    for b in [0xA0u8, 0xFD, 0xFE, 0xFF] {
        let mut s = env();
        let bytes = vec![b'a', b, b'b'];
        s.fs.add_file("/w/c.txt", &bytes);
        open_as(&mut s, "/w/c.txt", Some(Encoding::ShiftJis));
        assert!(
            doc(&s).read_only.contains(ReadOnlyReason::InvalidBytes),
            "{b:#04X}: 読み取り専用"
        );
        run(
            &mut s,
            "file.save_as",
            Args::new().with_str("path", "/w/copy.txt"),
        );
        assert_eq!(s.fs.read("/w/copy.txt").unwrap(), bytes, "{b:#04X}");
    }
    // 0x80 は U+0080 で往復する
    assert_lossless(Encoding::ShiftJis, &[0x80], "0x80");
}

#[test]
fn euc_jp_の互換文字は区分_a_でバイト一致する() {
    for (b, name) in [
        (&[0xA1u8, 0xC1][..], "0xA1C1"),
        (&[0xA1, 0xC2], "0xA1C2"),
        (&[0xA1, 0xDD], "0xA1DD"),
        (&[0xA1, 0xF1], "0xA1F1"),
        (&[0xA1, 0xF2], "0xA1F2"),
        (&[0xA2, 0xCC], "0xA2CC"),
        (&[0xA1, 0xBD], "0xA1BD"),
        (&[0xFC, 0xFC], "0xFCFC"),
        (&[0x8E, 0xB1], "半角カナ 0x8EB1"),
    ] {
        assert_lossless(Encoding::EucJp, b, name);
    }
}

#[test]
fn iso_2022_jp_の正規形は往復する() {
    let (orig, _, _) = encoding_rs::ISO_2022_JP.encode("日本語abc\n二行目\n");
    let mut s = env();
    s.fs.add_file("/w/c.txt", &orig);
    open_as(&mut s, "/w/c.txt", Some(Encoding::Iso2022Jp));
    assert_eq!(doc(&s).format.invalid_bytes, 0);
    type_text(&mut s, "x");
    run(&mut s, "file.save", Args::new());
    let (want, _, _) = encoding_rs::ISO_2022_JP.encode("x日本語abc\n二行目\n");
    assert_eq!(s.fs.read("/w/c.txt").unwrap(), want.as_ref());
}

/// ISO-2022-JP の区分 D のうち、ASCII 状態で始まり ASCII 状態で終わる行 (保持できる。改行は LF で書き、CRLF 版も試す)。
const ISO_D_PRESERVED: [(&str, &[u8]); 9] = [
    ("旧 JIS (ESC $ @)", b"\x1B$@$\"$$\x1B(B\n"),
    ("ESC ( J で書かれた ASCII", b"\x1B(Jabc\x1B(B\n"),
    ("ESC ( J の ¥ のあとに ASCII", b"\x1B(J\\\x1B(Babc\n"),
    ("冗長な ESC ( B で始まる行", b"\x1B(Babc\n"),
    ("冗長な ESC $ B", b"\x1B$B$\"\x1B$B$$\x1B(B\n"),
    ("ESC ( I の半角カナ", b"\x1B(I1234\x1B(B\n"),
    ("全角と半角カナの混在", b"\x1B$B$\"\x1B(I1\x1B$B$$\x1B(B\n"),
    (
        "正規形と旧 JIS と半角カナの複数行",
        b"\x1B$B$\"\x1B(B\n\x1B$@$$\x1B(B\n\x1B(I12\x1B(B\nabc\n",
    ),
    ("空の行をはさむ", b"\x1B$@$\"\x1B(B\n\n\x1B(I1\x1B(B\n"),
];

fn to_crlf(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() * 2);
    for &c in b {
        if c == b'\n' {
            out.push(b'\r');
        }
        out.push(c);
    }
    out
}

/// 区分 D: `case` (ASCII 状態で始まり終わる行の並び) の後ろに ASCII の行 `edit me` を置いて開く。`edit me` の行頭に 1 文字
/// 入力して保存すると、`case` は元のバイト列のまま。確認は出ない (上書き保存も同じ文字コードの名前を付けて保存も)。
fn assert_preserved_d(case: &[u8], crlf: bool, label: &str) {
    let eol: &[u8] = if crlf { b"\r\n" } else { b"\n" };
    let case = if crlf { to_crlf(case) } else { case.to_vec() };
    let file = [case.as_slice(), b"edit me", eol].concat();
    let mut s = env();
    s.fs.add_file("/w/c.txt", &file);
    open_as(&mut s, "/w/c.txt", Some(Encoding::Iso2022Jp));
    assert_eq!(doc(&s).format.invalid_bytes, 0, "{label}");
    assert!(doc(&s).is_editable(), "{label}");
    assert_eq!(
        s.doc_text(),
        encoding_rs::ISO_2022_JP
            .decode_without_bom_handling(&file)
            .0,
        "{label}: 本文"
    );
    assert!(
        detected_lines(doc(&s)) > 0,
        "{label}: 区分 B / D として検出"
    );
    assert_eq!(
        doc(&s).format.roundtrip.preserve,
        PreserveStatus::Active { unpreserved: 0 },
        "{label}"
    );
    assert!(!doc(&s).preserved().is_empty(), "{label}");
    // `edit me` の行頭へ (末尾の空行から 1 行上がる)
    s.keys("ctrl+end");
    s.keys("up");
    s.keys("home");
    type_text(&mut s, "x");
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.overlay(), "", "{label}: 確認を出さない");
    let want = [case.as_slice(), b"xedit me", eol].concat();
    assert_eq!(s.fs.read("/w/c.txt").unwrap(), want, "{label}: 上書き保存");
    save_as(&mut s, "/w/d.txt");
    assert_eq!(s.overlay(), "", "{label}: 名前を付けて保存も確認なし");
    assert_eq!(
        s.fs.read("/w/d.txt").unwrap(),
        want,
        "{label}: 名前を付けて保存"
    );
}

// 01 §5.5 の NFR-303 AC-5 / AC-6 (v0.4 の AC は `tests/ac-checklist.toml` の対象外なので、要件 ID で印を付ける)
// verifies: NFR-303
#[test]
fn iso_2022_jp_の区分_d_のうち_ascii_状態で始まり終わる行は元のバイト列で保存される() {
    for (name, case) in ISO_D_PRESERVED {
        assert_preserved_d(case, false, name);
        assert_preserved_d(case, true, &format!("{name} (CRLF)"));
    }
}

/// 保持できなかった行を含む文書: 記録の行 (旧 JIS・冗長な ESC ( B)、ローマ字状態で終わる行と、その状態で始まる行
/// (どちらも保持できない)、ASCII の行。
const ISO_D_PARTIAL: [&[u8]; 7] = [
    b"\x1B$@$\"\x1B(B\n",
    b"abc\n",
    b"\x1B(J\\\n",
    b"x\\y\n",
    b"\x1B(Bok\n",
    b"edit me\n",
    b"\x1B(I1\x1B(B\n",
];

// 01 §5.5 の NFR-303 AC-5 / AC-6 (v0.4 の AC は `tests/ac-checklist.toml` の対象外なので、要件 ID で印を付ける)
// verifies: NFR-303
#[test]
fn iso_2022_jp_で_ascii_状態で始まらない行と終わらない行は確認の後その行だけ正規形で書かれる() {
    let file = ISO_D_PARTIAL.concat();
    // 保持できない 3 行 (3 行目はローマ字状態で終わり、4 行目と 5 行目はその状態で始まる) だけが正規形になる
    let want_normal = [
        ISO_D_PARTIAL[0],
        ISO_D_PARTIAL[1],
        b"\x1B(J\\\x1B(B\n",
        b"x\x1B(J\\y\x1B(B\n",
        b"ok\n",
        b"xedit me\n",
        ISO_D_PARTIAL[6],
    ]
    .concat();
    for in_place in [true, false] {
        let mut s = env();
        s.fs.add_file("/w/c.txt", &file);
        open_as(&mut s, "/w/c.txt", Some(Encoding::Iso2022Jp));
        assert_eq!(doc(&s).format.invalid_bytes, 0);
        assert_eq!(
            doc(&s).format.roundtrip.preserve,
            PreserveStatus::Active { unpreserved: 3 },
            "保持できなかった行は 3 行"
        );
        assert!(!doc(&s).preserved().is_empty());
        // 6 行目 (edit me) の行頭に入力
        s.keys("ctrl+home");
        for _ in 0..5 {
            s.keys("down");
        }
        type_text(&mut s, "x");
        if in_place {
            run(&mut s, "file.save", Args::new());
        } else {
            save_as(&mut s, "/w/d.txt");
        }
        assert_eq!(
            confirm_title(&s).as_deref(),
            Some("エスケープの書き方が変わります"),
            "確認が出る (in_place = {in_place})"
        );
        // 取り消すと何も書かない
        s.keys("esc");
        assert_eq!(s.fs.read("/w/c.txt").unwrap(), file, "取り消し");
        assert!(!s.fs.exists("/w/d.txt"), "取り消し");
        // もう一度保存して、確認に応じる (既定は「保存」)
        if in_place {
            run(&mut s, "file.save", Args::new());
        } else {
            save_as(&mut s, "/w/d.txt");
        }
        assert!(confirm_title(&s).is_some());
        s.keys("enter");
        s.run_jobs();
        let path = if in_place { "/w/c.txt" } else { "/w/d.txt" };
        assert_eq!(
            s.fs.read(path).unwrap(),
            want_normal,
            "保持できなかった行だけ正規形、ほかは元のバイト列 (in_place = {in_place})"
        );
        if !in_place {
            assert_eq!(s.fs.read("/w/c.txt").unwrap(), file, "元のファイルは不変");
        }
    }
}

#[test]
fn iso_2022_jp_で_jis_x_0208_状態のまま改行する行は不正バイトで読み取り専用_保存は拒否される() {
    // `ESC $ B あ LF ESC ( B` (CRLF 版も)。半角カナ状態のまま改行する行も同じ (WHATWG のデコーダが受け付けない)。
    // 不正バイトは他の文字コードと同じく `<0x0A>` などの記号で表示する (03 §8.3 の「状態付き符号化」、09 §4)
    for (name, bytes, shown) in [
        (
            "JIS X 0208 + LF",
            b"\x1B$B$\"\n\x1B(B".as_slice(),
            "あ<0x0A>",
        ),
        (
            "JIS X 0208 + CRLF",
            b"\x1B$B$\"\r\n\x1B(B",
            "あ<0x0D><0x0A>",
        ),
        ("半角カナ + LF", b"\x1B(I1\n\x1B(B", "ｱ<0x0A>"),
    ] {
        let mut s = env();
        s.fs.add_file("/w/c.txt", bytes);
        open_as(&mut s, "/w/c.txt", Some(Encoding::Iso2022Jp));
        assert_eq!(doc(&s).format.encoding, Encoding::Iso2022Jp, "{name}");
        assert!(doc(&s).format.invalid_bytes > 0, "{name}");
        assert!(
            doc(&s)
                .read_only
                .contains(ReadOnlyReason::StatefulEncodingWithInvalidBytes),
            "{name}: 読み取り専用"
        );
        let frame = s.frame_text();
        assert!(frame.contains(shown), "{name}: 記号で表示する\n{frame}");
        assert!(
            !frame
                .chars()
                .any(|c| ('\u{10FF00}'..='\u{10FFFF}').contains(&c)),
            "{name}: 私用文字のまま描かない\n{frame}"
        );
        save_as(&mut s, "/w/d.txt");
        assert!(
            !s.fs.exists("/w/d.txt"),
            "{name}: ISO-2022-JP での保存は拒否"
        );
        assert_eq!(
            s.fs.read("/w/c.txt").unwrap(),
            bytes,
            "{name}: 元のファイルは不変"
        );
    }
}

#[test]
fn bom_付きの_utf8_と_utf16_は_bom_ごと往復する() {
    let le: Vec<u8> = "ab\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let be: Vec<u8> = "ab\n".encode_utf16().flat_map(u16::to_be_bytes).collect();
    let cases: [(Encoding, Vec<u8>); 3] = [
        (Encoding::Utf8, b"\xEF\xBB\xBFab\n".to_vec()),
        (Encoding::Utf16Le, [&[0xFF, 0xFE][..], &le].concat()),
        (Encoding::Utf16Be, [&[0xFE, 0xFF][..], &be].concat()),
    ];
    for (enc, orig) in cases {
        let mut s = env();
        s.fs.add_file("/w/c.txt", &orig);
        open_as(&mut s, "/w/c.txt", None);
        assert_eq!(doc(&s).format.encoding, enc);
        assert!(doc(&s).format.has_bom);
        type_text(&mut s, "x");
        run(&mut s, "file.save", Args::new());
        let saved = s.fs.read("/w/c.txt").unwrap();
        let bom_len = if enc == Encoding::Utf8 { 3 } else { 2 };
        assert_eq!(&saved[..bom_len], &orig[..bom_len], "{enc:?}: BOM が残る");
    }
}

// verifies: AC-FR-113-1, AC-FR-114-1
#[test]
fn 改行の種類はそのまま往復する() {
    let cases: [(&str, &[u8]); 9] = [
        ("LF", b"a\nb\n"),
        ("CRLF", b"a\r\nb\r\n"),
        ("CR", b"a\rb\r"),
        ("混在", b"a\nb\r\nc\rd"),
        ("末尾改行なし", b"abc"),
        ("空", b""),
        ("改行のみ", b"\n\n\n"),
        ("CR CR LF", b"a\r\r\nb"),
        ("CRLF 混在の末尾なし", b"x\r\ny\nz"),
    ];
    for (name, orig) in cases {
        let mut s = env();
        s.fs.add_file("/w/c.txt", orig);
        open_as(&mut s, "/w/c.txt", None);
        type_text(&mut s, "x");
        run(&mut s, "file.save", Args::new());
        let mut want = b"x".to_vec();
        want.extend_from_slice(orig);
        assert_eq!(s.fs.read("/w/c.txt").unwrap(), want, "{name}");
    }
}
