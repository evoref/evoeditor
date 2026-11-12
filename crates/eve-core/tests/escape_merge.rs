//! エスケープ文字のつながり (`EscapeMerge`、docs/03 §8.3.1) の保存 (M23 の 3 本目)。書く前の確認・承認・強調
//! (M23 の 4 本目) の試験は `escape_merge_confirm.rs`。
//!
//! 不正バイトを含む文書は、今は読み取り専用 (「編集を許可」は M23 の後の PR)。編集が要る場合は、試験の中で文書の
//! `read_only` から `InvalidBytes` を直に外して、解除した後の状態を作る (製品の解除の経路は足さない)。

use eve_core::{Args, CommandResult, Document, Encoding, OpenOptions, ReadOnlyReason};
use eve_encoding::PreserveStatus;
use eve_testkit::Harness;

fn env() -> Harness {
    Harness::builder().defer_start().build()
}

fn run(h: &mut Harness, id: &str, args: Args) -> CommandResult {
    let r = h.cmd_with(id, args);
    h.run_jobs();
    r
}

fn open_as(h: &mut Harness, path: &str, enc: Encoding) {
    h.editor.state_mut().open_path(
        path,
        OpenOptions {
            forced: Some(enc),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
}

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

/// 不正バイトの比率の確認 (FR-127) を出さないための後ろの詰め物。
const PAD: &str = "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\n\
                   0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\n";

/// 「編集を許可」の後の状態を作る (試験だけの近道。製品の解除は M23 の後の PR)。
fn release_invalid_bytes(h: &mut Harness) {
    let id = h.doc_id();
    let d = h.editor.state_mut().document_mut(id).unwrap();
    d.read_only.remove(ReadOnlyReason::InvalidBytes);
    assert!(d.is_editable());
}

/// `Shift_JIS` の `x<0x81>` の行と `@yz` の行。行を結合すると `<0x81>` の直後に `@` が来て、0x8140 (全角の空白) になる。
fn sjis_joined() -> Harness {
    let mut s = env();
    s.fs.add_file("/w/a.txt", &[b"x\x81\n@yz\n", PAD.as_bytes()].concat());
    open_as(&mut s, "/w/a.txt", Encoding::ShiftJis);
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::ShiftJis));
    release_invalid_bytes(&mut s);
    // 1 行目の末尾の改行を消す
    s.keys("ctrl+home");
    s.keys("end");
    s.keys("delete");
    assert_eq!(s.doc_text(), format!("x\u{10FF81}@yz\n{PAD}"));
    s
}

// verifies: NFR-304
#[test]
fn つながる保存は何も書かずに確認を出す() {
    let mut s = sjis_joined();
    let before = s.fs.read("/w/a.txt").unwrap();
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before, "何も書かない");
    assert!(doc(&s).is_dirty(), "保存しなかった");
    assert_eq!(s.overlay(), "dialog", "書く前の確認: {:?}", s.messages());
    // 記録をもともと持たない文書でも、記録を持つ文書でも、つながりでは保持の状態を変えない (VerifyFailed ではない)
    assert!(!matches!(
        doc(&s).format.roundtrip.preserve,
        PreserveStatus::Unavailable(_)
    ));
}

// 全保存では保存せずに、確認が要る文書として一覧に載せる (03 §8.3.1 の「全保存」)
#[test]
fn 全保存ではつながる文書を保存しない() {
    let mut s = sjis_joined();
    let before = s.fs.read("/w/a.txt").unwrap();
    run(&mut s, "file.save_all", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before, "何も書かない");
    assert!(doc(&s).is_dirty());
    assert_eq!(s.overlay(), "dialog", "失敗の一覧: {:?}", s.messages());
}

// つながりを元に戻せば (Undo で改行を戻す) ふつうに保存できる。不正バイトは元のバイトのまま書く
#[test]
fn つながりを戻せば元のバイトのまま保存できる() {
    let mut s = sjis_joined();
    run(&mut s, "file.save", Args::new());
    // 書く前の確認をキャンセルする
    s.keys("esc");
    s.cmd("edit.undo");
    s.keys("ctrl+end");
    s.text("z");
    run(&mut s, "file.save", Args::new());
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        [b"x\x81\n@yz\n", PAD.as_bytes(), b"z"].concat()
    );
    assert!(!doc(&s).is_dirty());
}

// 回帰: BOM 付きの不正バイトを含む UTF-8 / UTF-16 の文書は、確認 (ii) が BOM を読み飛ばさずに毎回 VerifyFailed になって
// いた (03 §8.3.1 の「復号する範囲」)。編集して上書き保存すると、BOM と不正バイトが元のまま残る
// verifies: NFR-304
#[test]
fn bom_付きの不正バイトを含む文書を編集して保存できる() {
    let cases: [(Encoding, Vec<u8>, &[u8]); 2] = [
        (
            Encoding::Utf8,
            [b"\xEF\xBB\xBFa\x80b\n", PAD.as_bytes()].concat(),
            b"z",
        ),
        (
            Encoding::Utf16Le,
            [&b"\xFF\xFEa\x00\x00\xD8b\x00\n\x00"[..], &utf16le(PAD)].concat(),
            b"z\x00",
        ),
    ];
    for (enc, file, z) in cases {
        let mut s = env();
        s.fs.add_file("/w/a.txt", &file);
        open_as(&mut s, "/w/a.txt", enc);
        assert!(doc(&s).format.has_bom, "{enc:?}");
        assert_eq!(doc(&s).format.escape_encoding, Some(enc));
        release_invalid_bytes(&mut s);
        s.keys("ctrl+end");
        s.text("z");
        run(&mut s, "file.save", Args::new());
        assert_eq!(
            s.fs.read("/w/a.txt").unwrap(),
            [file.as_slice(), z].concat(),
            "{enc:?}: {:?}",
            s.messages()
        );
        assert!(!doc(&s).is_dirty(), "{enc:?}");
        assert_eq!(
            doc(&s).format.roundtrip.preserve,
            PreserveStatus::NotApplicable,
            "{enc:?}"
        );
    }
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

// 先頭が別の文字コードの BOM の並びの不正バイトで始まるファイルを、未編集のまま名前を付けて保存すると、元のファイルと
// バイト単位で同じものを書く (03 §8.3.1 の「検出しないもの」。同じ文字コードを指定して開き直せば元に戻る)
// verifies: NFR-304
#[test]
fn 先頭が別の文字コードの_bom_の並びのファイルも未編集なら名前を付けて保存できる() {
    let cases: [(Encoding, &[u8]); 5] = [
        (Encoding::Utf8, b"\xFF\xFE"),
        (Encoding::Utf8, b"\xFE\xFF"),
        (Encoding::ShiftJis, b"\xEF\xBB\xBF"),
        (Encoding::ShiftJis, b"\xFF\xFE"),
        (Encoding::EucJp, b"\xFF\xFE"),
    ];
    for (enc, head) in cases {
        let mut s = env();
        let file = [head, b"ab\n", PAD.as_bytes()].concat();
        s.fs.add_file("/w/a.txt", &file);
        open_as(&mut s, "/w/a.txt", enc);
        assert_eq!(
            doc(&s).format.escape_encoding,
            Some(enc),
            "{enc:?} {head:02X?}"
        );
        run(
            &mut s,
            "file.save_as",
            Args::new().with_str("path", "/w/b.txt"),
        );
        assert_eq!(
            s.fs.read("/w/b.txt").as_deref(),
            Some(file.as_slice()),
            "{enc:?} {head:02X?}: {:?}",
            s.messages()
        );
        assert_eq!(
            s.messages().last().map(String::as_str),
            Some("info.save.done"),
            "{enc:?} {head:02X?}"
        );
    }
}
