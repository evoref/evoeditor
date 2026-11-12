//! 保持方式 (docs/03 §8.7.1) の、大きな文書での回帰の試験 (v0.4 の M22 の 7)。rope のチャンク (1 KiB 前後) を何度もまたぐ大きな
//! 文書で、記録がチャンクの境をまたぐ・行の位置が遠くへ飛ぶ・ISO-2022-JP の長い行の記録・長さ 0 の記録を作り、
//! 読込の照合 (`from_records`)・保存の計画・区間の切り出し (`TextCursor`) を通す。
//!
//! - 読込の後、表が本文と合い、保持が完全。
//! - 編集せずに名前を付けて保存すると、元のファイルとバイト一致する。
//! - 編集して保存すると、復号が本文になる。全部 Undo して保存すると元のファイルとバイト一致する。

use eve_core::{Args, Document, Encoding, OpenOptions};
use eve_testkit::Harness;
use eve_view::{Answer, OverlayModel, Query};

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

fn open_as(path: &str, bytes: &[u8], enc: Encoding) -> Harness {
    let mut h = Harness::new().with_size(100, 16).with_file(path, bytes);
    h.editor.state_mut().open_path(
        path,
        OpenOptions {
            forced: Some(enc),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    h
}

fn dialog_title(h: &Harness) -> Option<String> {
    let window = h.editor.state().first_window()?;
    let text =
        |c: &eve_view::CellText| -> String { c.spans.iter().map(|sp| sp.text.as_str()).collect() };
    match h.query(&Query::Overlay { window }).ok()? {
        Answer::Overlay(Some(OverlayModel::Confirm { title, .. })) => Some(text(&title)),
        _ => None,
    }
}

const SAVE_CONFIRMS: [&str; 3] = [
    "エスケープの書き方が変わります",
    "別のバイト表現で保存されるかもしれません",
    "別のバイト表現で保存されます",
];

/// 保存の前の確認には「保存」で答え、ほかのダイアログ (表現できない文字の E3 など) は閉じる。保存が成功したか。
fn settle(h: &mut Harness) -> bool {
    let done = |h: &Harness| {
        h.messages()
            .iter()
            .any(|m| m == "info.save.done" || m == "info.save.in_place")
    };
    let mut saved = false;
    for _ in 0..4 {
        h.run_jobs();
        saved |= done(h);
        h.clear_log();
        match dialog_title(h) {
            Some(t) if SAVE_CONFIRMS.contains(&t.as_str()) => h.text("s"),
            _ if !h.overlay().is_empty() => h.keys("esc"),
            _ => break,
        }
    }
    h.run_jobs();
    saved |= done(h);
    h.clear_log();
    if !h.overlay().is_empty() {
        h.keys("esc");
    }
    saved
}

fn save_as(h: &mut Harness, path: &str) -> Option<Vec<u8>> {
    h.cmd_with("file.save_as", Args::new().with_str("path", path));
    settle(h).then(|| h.fs_read(path).unwrap())
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n.max(1) as u64).unwrap_or(0)
    }
}

const SJIS: &[&[u8]] = &[
    b"a",
    b"\x93\xFA",
    b"\xFA\x54",
    b"\xED\x40",
    b"\xEE\xEF",
    b"\x87\x90",
    b"\x87\x9A",
    b"\xB1",
    b"\\",
    b"~",
    b"\xF0\x40",
];
const EUC: &[&[u8]] = &[
    b"a",
    b"\xC6\xFC",
    b"\x8E\xB1",
    b"\x8F\xB0\xA1",
    b"\x8F\xA2\xB7",
    b"\xAD\xA1",
    b"\xFC\xFC",
    b"\xA1\xC1",
];
const ISO: &[&[u8]] = &[
    b"ab",
    b"\x1B$B$\"\x1B(B",
    b"\x1B$@$\"\x1B(B",
    b"\x1B(J\\a\x1B(B",
    b"\x1B(I12\x1B(B",
    b"\x1B$BF|K\\\x1B(B",
    b"\x1B(B",
];
const EOLS: &[&[u8]] = &[b"\n", b"\r\n", b"\r"];

/// 大きな文書。記録の密な行・記録の無い行が長く続く区間 (行の位置の引き直し)・長い行 (チャンクをまたぐ記録) を混ぜる。
fn big_doc(rng: &mut Rng, enc: Encoding, target: usize) -> Vec<u8> {
    let atoms = match enc {
        Encoding::ShiftJis => SJIS,
        Encoding::EucJp => EUC,
        _ => ISO,
    };
    let mut b: Vec<u8> = Vec::new();
    // 行の終端を選ぶ。ISO-2022-JP で、単独の CR で終わった行の次の行を LF で終えると、エスケープだけの行が CRLF に結合して
    // 保持できない (仕様どおり) ので、その場合は CRLF にする
    let eol = |rng: &mut Rng, line_start_after_cr: bool| -> &'static [u8] {
        let e = EOLS[rng.below(EOLS.len())];
        if enc == Encoding::Iso2022Jp && line_start_after_cr && e == b"\n" {
            b"\r\n"
        } else {
            e
        }
    };
    while b.len() < target {
        let after_cr = b.last() == Some(&b'\r');
        match rng.below(8) {
            // 記録の無い行が続く
            0 => {
                for _ in 0..(20 + rng.below(40)) {
                    let cr = b.last() == Some(&b'\r');
                    b.extend_from_slice(b"plain ascii line");
                    b.extend_from_slice(eol(rng, cr));
                }
            }
            // 長い行 (数 KiB。ISO-2022-JP では 1 行全体が 1 件の記録になり、チャンクをまたぐ)
            1 => {
                let n = 300 + rng.below(1200);
                for i in 0..n {
                    if enc == Encoding::Iso2022Jp && i > 0 {
                        b.push(b'x');
                    }
                    b.extend_from_slice(atoms[rng.below(atoms.len())]);
                }
                b.extend_from_slice(eol(rng, after_cr));
            }
            // 空行 / 冗長なエスケープだけの行
            2 => {
                if enc == Encoding::Iso2022Jp {
                    // 単独の CR の次の、エスケープだけで LF で終わる行は CRLF に結合して保持できない (仕様どおり)
                    // ので作らない
                    let after_cr = b.last() == Some(&b'\r');
                    b.extend_from_slice(b"\x1B(B");
                    if after_cr {
                        b.extend_from_slice(b"\r\n");
                        continue;
                    }
                }
                b.extend_from_slice(eol(rng, after_cr));
            }
            _ => {
                for i in 0..(1 + rng.below(12)) {
                    if enc == Encoding::Iso2022Jp && i > 0 {
                        b.push(b'x');
                    }
                    b.extend_from_slice(atoms[rng.below(atoms.len())]);
                }
                b.extend_from_slice(eol(rng, after_cr));
            }
        }
    }
    b
}

fn decode(enc: Encoding, b: &[u8]) -> String {
    let e = match enc {
        Encoding::ShiftJis => encoding_rs::SHIFT_JIS,
        Encoding::EucJp => encoding_rs::EUC_JP,
        _ => encoding_rs::ISO_2022_JP,
    };
    e.decode_without_bom_handling(b).0.into_owned()
}

fn scenario(enc: Encoding, seed: u64, target: usize) -> bool {
    let mut rng = Rng(seed);
    let bytes = big_doc(&mut rng, enc, target);
    let mut h = open_as("/w/a.txt", &bytes, enc);
    {
        let d = doc(&h);
        assert_eq!(d.format.encoding, enc);
        assert!(
            d.format.roundtrip.preserve.is_complete(),
            "{enc:?} {seed:#x}: 保持が完全 ({:?})",
            d.format.roundtrip.preserve
        );
        assert!(d.preserved().len() > 10, "記録が多い");
        assert_eq!(d.preserved().check(d.text(), enc), Ok(()));
    }
    // 編集せずに保存すると元のバイト列
    let got = save_as(&mut h, "/w/b.txt").expect("未編集の保存");
    assert!(got == bytes, "{enc:?} {seed:#x}: 未編集の保存が元と違う");
    // 編集 (先頭・末尾・途中の行の末尾で改行・途中で文字の削除)
    h.keys("ctrl+home");
    h.text("X");
    h.keys("ctrl+end");
    h.text("Y");
    for _ in 0..3 {
        h.keys("ctrl+home");
        for _ in 0..rng.below(40) {
            h.keys("pagedown");
        }
        match rng.below(3) {
            0 => {
                h.keys("end");
                h.keys("enter");
            }
            1 => {
                h.keys("home");
                h.cmd("edit.delete_forward");
            }
            _ => {
                h.keys("end");
                h.cmd("edit.delete_forward");
            }
        }
    }
    h.advance_ms(2000);
    let text = h.doc_text();
    // 編集で記録が外れた範囲に、符号化できない文字 (EUC-JP の補助漢字・ISO-2022-JP の半角カナ) があると E3 で止まる
    // (仕様どおり)。止まらなかったときだけ比べる
    let edited_saved = match save_as(&mut h, "/w/c.txt") {
        Some(got) => {
            assert!(
                decode(enc, &got) == text,
                "{enc:?} {seed:#x}: 保存の復号が本文と違う"
            );
            true
        }
        None => false,
    };
    {
        let d = doc(&h);
        assert_eq!(d.preserved().check(d.text(), enc), Ok(()));
        assert_eq!(d.preserved_anomalies(), 0);
    }
    // 全部 Undo して保存すると元のバイト列
    while doc(&h).history().can_undo() {
        h.cmd("edit.undo");
    }
    let got = save_as(&mut h, "/w/d.txt").expect("全部 Undo の保存");
    assert!(
        got == bytes,
        "{enc:?} {seed:#x}: 全部 Undo の保存が元と違う"
    );
    edited_saved
}

#[test]
fn 大きな文書でチャンクをまたぐ記録も読込と保存で変わらない() {
    let mut edited = 0;
    for (i, enc) in [Encoding::ShiftJis, Encoding::EucJp, Encoding::Iso2022Jp]
        .into_iter()
        .enumerate()
    {
        for k in 0..4u64 {
            let seed = 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(k + 1 + 10 * i as u64) | 1;
            if scenario(enc, seed, [20_000, 60_000, 150_000, 300_000][k as usize]) {
                edited += 1;
            }
        }
    }
    // 編集した保存を Shift_JIS では必ず比べている (Shift_JIS の区分 B の文字は符号化できる)
    assert!(edited >= 4, "編集した保存を比べた数 {edited}");
}
