//! 保持方式 (docs/03 §8.6 E2、§8.7.1) の性質テスト: 任意のバイト列 × 文字コード (`Shift_JIS` / EUC-JP / ISO-2022-JP)。
//! `fuzz_decode` (docs/09 §3.3) の「往復は v0.4 で保持方式」の、保存まで含めた部分。`eve-encoding` の `fuzz_decode` は復号と
//! 記録の不変条件までを見るので、記録を使って書く側 (保存のジョブ) はここで見る。
//!
//! 不正バイトが無く、読込の完了時に「全部保持できた」(`Active { unpreserved: 0 }`) 文書について:
//!
//! - 何も編集せずに同じ文字コードで名前を付けて保存すると、元のバイト列と一致する (確認は出ない)。
//! - 末尾 (最後の改行の後) に 1 文字入力して上書き保存すると、「元のバイト列 + 入力した 1 文字」になる (確認は出ない)。
//!
//! 保持が効かない文書 (不正バイト・保持できなかった行・保持なし) は対象から除く (09 §4 の往復コーパスの除外と同じ)。

use eve_core::{Args, Document, Encoding, OpenOptions};
use eve_encoding::PreserveStatus;
use eve_testkit::Harness;
use proptest::prelude::*;

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

/// `bytes` を `enc` を指定して開く。文書が開かなかったとき (不正バイトの確認が出たとき) は `None`。
fn open_as(bytes: &[u8], enc: Encoding) -> Option<Harness> {
    let mut h = Harness::new()
        .with_size(100, 16)
        .with_file("/w/a.txt", bytes);
    h.editor.state_mut().open_path(
        "/w/a.txt",
        OpenOptions {
            forced: Some(enc),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    if h.overlay() != "" || h.editor.state().active_document().is_none() {
        return None;
    }
    Some(h)
}

/// 性質を確かめる。対象外の文書 (保持が効かない) は `false`、確かめたら `true`。
fn check(bytes: &[u8], enc: Encoding) -> bool {
    let Some(mut h) = open_as(bytes, enc) else {
        return false;
    };
    let d = doc(&h);
    if d.format.encoding != enc
        || d.format.invalid_bytes != 0
        || !d.is_editable()
        || d.format.roundtrip.preserve != (PreserveStatus::Active { unpreserved: 0 })
    {
        return false;
    }
    // 何も編集せずに名前を付けて保存: 元のバイト列
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/b.txt"));
    h.run_jobs();
    assert_eq!(h.overlay(), "", "{enc:?} {bytes:02X?}: 確認を出さない");
    assert_eq!(
        h.fs.read("/w/b.txt").as_deref(),
        Some(bytes),
        "{enc:?}: 未編集の名前を付けて保存は元のバイト列"
    );
    // 末尾 (最後の改行の後の空の行) に 1 文字入力して上書き保存
    h.keys("ctrl+end");
    h.text("x");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "", "{enc:?} {bytes:02X?}: 確認を出さない");
    let want = [bytes, b"x"].concat();
    assert_eq!(
        h.fs.read("/w/b.txt").as_deref(),
        Some(want.as_slice()),
        "{enc:?} {bytes:02X?}: 入力した 1 文字を除いて元のバイト列"
    );
    true
}

/// 断片を並べて、最後に LF を足す。
fn lines(frag: impl Strategy<Value = Vec<u8>>) -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(frag, 0..24).prop_map(|frags| {
        let mut v = frags.concat();
        v.push(b'\n');
        v
    })
}

/// `Shift_JIS` の断片 (ASCII・全角・半角カナ・区分 B の重複符号化・外字・LF) を並べる (最後は LF)。
fn sjis_lines() -> impl Strategy<Value = Vec<u8>> {
    lines(prop_oneof![
        Just(b"abc".to_vec()),
        Just(b"\n".to_vec()),
        Just(b"\xFA\x54".to_vec()),
        Just(b"\xEE\xF9".to_vec()),
        Just(b"\x87\x90".to_vec()),
        Just(b"\xED\x40".to_vec()),
        Just(b"\x81\xCA".to_vec()),
        (0xA1u8..0xE0).prop_map(|a| vec![a]),
        (0x81u8..0xA0, 0x40u8..0x7F).prop_map(|(a, b)| vec![a, b]),
        (0xE0u8..0xFD, 0x80u8..0xFD).prop_map(|(a, b)| vec![a, b]),
        (0x20u8..0x7F).prop_map(|a| vec![a]),
    ])
}

/// EUC-JP の断片 (ASCII・全角・半角カナ・補助漢字・LF) を並べる (最後は LF)。
fn euc_lines() -> impl Strategy<Value = Vec<u8>> {
    lines(prop_oneof![
        Just(b"abc".to_vec()),
        Just(b"\n".to_vec()),
        Just(b"\x8F\xB0\xA1".to_vec()),
        Just(b"\x8F\xA2\xB7".to_vec()),
        (0xA1u8..0xFF, 0xA1u8..0xFF).prop_map(|(a, b)| vec![a, b]),
        (0xA1u8..0xE0).prop_map(|a| vec![0x8E, a]),
        (0xA1u8..0xF5, 0xA1u8..0xFF).prop_map(|(a, b)| vec![0x8F, a, b]),
        (0x20u8..0x7F).prop_map(|a| vec![a]),
    ])
}

/// ISO-2022-JP の行 (1 行 = 状態の切り替えと中身の組を並べ、たいてい `ESC ( B` で ASCII に戻して LF で終える。
/// 戻さない行は、保持できない行・不正バイトになる)。旧 JIS・ローマ字・半角カナ・冗長なエスケープを含む。
fn iso_lines() -> impl Strategy<Value = Vec<u8>> {
    let segment = prop_oneof![
        Just(b"abc".to_vec()),
        Just(b"\x1B(Babc".to_vec()),
        Just(b"\x1B(J\\~".to_vec()),
        (0x21u8..0x7E, 0x21u8..0x7E).prop_map(|(a, b)| [b"\x1B$B".as_slice(), &[a, b]].concat()),
        (0x21u8..0x7E, 0x21u8..0x7E).prop_map(|(a, b)| [b"\x1B$@".as_slice(), &[a, b]].concat()),
        (0x21u8..0x5F, 0x21u8..0x5F).prop_map(|(a, b)| [b"\x1B(I".as_slice(), &[a, b]].concat()),
    ];
    let line = (
        proptest::collection::vec(segment, 0..5),
        proptest::bool::weighted(0.9),
    )
        .prop_map(|(segs, close)| {
            let mut v = segs.concat();
            if close {
                v.extend_from_slice(b"\x1B(B");
            }
            v.push(b'\n');
            v
        });
    proptest::collection::vec(line, 0..6).prop_map(|ls| ls.concat())
}

/// 確かめた (対象になった) 件数を数えて、性質が空振りしていないことを見る。
fn run(enc: Encoding, strat: impl Strategy<Value = Vec<u8>>, cases: u32, min_checked: u32) {
    let checked = std::cell::Cell::new(0u32);
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    });
    runner
        .run(&strat, |bytes| {
            if check(&bytes, enc) {
                checked.set(checked.get() + 1);
            }
            Ok(())
        })
        .unwrap();
    assert!(
        checked.get() >= min_checked,
        "{enc:?}: 対象になったのは {} 件だけ (>= {min_checked} のはず)",
        checked.get()
    );
}

// verifies: AC-NFR-303-5
#[test]
fn shift_jis_は保持が効く文書なら未編集の保存も末尾への入力の後の保存も元のバイト列を保つ() {
    run(Encoding::ShiftJis, sjis_lines(), 400, 100);
}

// verifies: AC-NFR-303-5
#[test]
fn euc_jp_は保持が効く文書なら未編集の保存も末尾への入力の後の保存も元のバイト列を保つ() {
    run(Encoding::EucJp, euc_lines(), 400, 100);
}

// verifies: AC-NFR-303-6
#[test]
fn iso_2022_jp_は保持が効く文書なら未編集の保存も末尾への入力の後の保存も元のバイト列を保つ() {
    run(Encoding::Iso2022Jp, iso_lines(), 400, 100);
}
