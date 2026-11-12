//! 保存での保持方式の記録の差し替えの、乱数の操作列による試験 (docs/03 §8.6 E2、§8.7.1、§22.1 の保持方式。M22 の 6 の
//! レビューの再現試験を取り込んだもの)。`Shift_JIS` / EUC-JP / ISO-2022-JP の区分 B / D の文書に、入力・改行・削除・行の
//! 結合・Undo・Redo・保存 (保存の間の編集を含む)・名前を付けて保存を乱数で混ぜて、次を確かめる。
//!
//! - 保存したファイルを復号すると、保存の時点の本文になる (保存の間に編集しても)。
//! - 文書が保存点にいる (dirty でない) とき、同じ文字コードで「名前を付けて保存」した出力は、直前に保存したファイルと
//!   バイト単位で一致する (同じ本文と同じ記録からは同じ出力になる)。
//! - 表が本文と合い、表と履歴が食い違わない。
//! - 01 §5.5 の NFR-303 AC-7: 読込の状態まで全部 Undo して、同じ文字コードで名前を付けて保存すると、元のファイルと
//!   バイト一致する (保存を何回はさんでも。保持できなかった行の無い文書)。

use eve_core::{Args, Document, Encoding, OpenOptions};
use eve_testkit::Harness;
use eve_view::{Answer, OverlayModel, Query};

/// 1 つの試験で数えたもの (性質を確かめた回数が少なすぎないことを見る)。
#[derive(Default)]
struct Counts {
    docs: usize,
    saves: usize,
    saves_during: usize,
    compared: usize,
    restored: usize,
}

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

/// カーソルの位置に `text` をそのまま入れる (挿入の入口の規則 (03 §8.3.1) を通さない試験だけの近道)。v0.4 の M23 から、
/// 貼り付け・入力で外から来た `U+10FF00..` は `<U+10FFNN>` の文字列になるので、エスケープ文字が不正バイトの無い文書の
/// 本文にある状況 (違う文字コードで保存した後の Undo など) は、これで作る。
fn inject(h: &mut Harness, text: &str) {
    let id = h.doc_id();
    let st = h.editor.state_mut();
    let at = st
        .first_window()
        .and_then(|w| st.window_view(w))
        .and_then(|v| st.view(v))
        .map(|v| v.selection.primary().head)
        .unwrap();
    let d = st.document(id).unwrap();
    let cs = eve_text::ChangeSet::from_sorted_edits(
        &d.snapshot(),
        [eve_text::Edit {
            range: eve_text::ByteRange::new(at, at),
            insert: text.into(),
        }],
    )
    .unwrap();
    st.apply_transaction(
        id,
        eve_core::Transaction::new(cs, eve_core::Origin::Paste, eve_core::Tick(0)),
    )
    .unwrap();
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

fn saved_message(h: &Harness) -> bool {
    h.messages()
        .iter()
        .any(|m| m == "info.save.done" || m == "info.save.in_place")
}

/// 保存の前の確認には「保存」で答え、ほかのダイアログ (E3 など) は閉じる。保存が成功したか (メッセージ) を返す。
fn settle(h: &mut Harness) -> bool {
    let mut saved = false;
    for _ in 0..4 {
        h.run_jobs();
        saved |= saved_message(h);
        h.clear_log();
        match dialog_title(h) {
            Some(t) if SAVE_CONFIRMS.contains(&t.as_str()) => h.text("s"),
            _ if !h.overlay().is_empty() => h.keys("esc"),
            _ => break,
        }
    }
    h.run_jobs();
    saved |= saved_message(h);
    h.clear_log();
    if !h.overlay().is_empty() {
        h.keys("esc");
    }
    saved
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
    b"\xFA\x55",
    b"\xED\x40",
    b"\xEE\xEF",
    b"\x87\x90",
    b"\x81\xCA",
    b"\xB1",
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

fn random_doc(rng: &mut Rng, enc: Encoding) -> Vec<u8> {
    let atoms = match enc {
        Encoding::ShiftJis => SJIS,
        Encoding::EucJp => EUC,
        _ => ISO,
    };
    let mut b = Vec::new();
    for _ in 0..(2 + rng.next() % 6) {
        for i in 0..(1 + rng.next() % 4) {
            if enc == Encoding::Iso2022Jp && i > 0 {
                b.push(b'x');
            }
            b.extend_from_slice(atoms[rng.below(atoms.len())]);
        }
        b.extend_from_slice(EOLS[rng.below(EOLS.len())]);
    }
    b
}

fn goto(h: &mut Harness, n: usize) {
    h.keys("ctrl+home");
    for _ in 0..n {
        h.keys("right");
    }
}

/// 編集を 1 つ (`join` なら行の結合を多くする)。
fn edit(h: &mut Harness, rng: &mut Rng, log: &mut Vec<String>, join: bool) {
    let n = h.doc_text().chars().count();
    match rng.below(6) {
        0 => {
            let p = rng.below(n + 1);
            goto(h, p);
            h.text("x");
            log.push(format!("insert x @{p}"));
        }
        1 => {
            let p = rng.below(n + 1);
            goto(h, p);
            h.keys("enter");
            log.push(format!("enter @{p}"));
        }
        2 if join => {
            // 行の終わりで消す (行の結合)
            let lines = h.doc_text().lines().count().max(1);
            let l = rng.below(lines);
            h.keys("ctrl+home");
            for _ in 0..l {
                h.keys("down");
            }
            h.keys("end");
            h.cmd("edit.delete_forward");
            log.push(format!("join line {l}"));
        }
        2 | 3 => {
            let p = rng.below(n + 1);
            goto(h, p);
            h.cmd("edit.delete_forward");
            log.push(format!("delete @{p}"));
        }
        4 => {
            h.cmd("edit.undo");
            log.push("undo".into());
        }
        _ => {
            h.cmd("edit.redo");
            log.push("redo".into());
        }
    }
    h.advance_ms(2000);
}

fn decode(enc: Encoding, b: &[u8]) -> String {
    let e = match enc {
        Encoding::ShiftJis => encoding_rs::SHIFT_JIS,
        Encoding::EucJp => encoding_rs::EUC_JP,
        _ => encoding_rs::ISO_2022_JP,
    };
    e.decode_without_bom_handling(b).0.into_owned()
}

/// 1 つの文書で操作の列を走らせる。性質に反したら説明を返す。
fn scenario(enc: Encoding, seed: u64, join: bool, c: &mut Counts) -> Option<String> {
    let mut rng = Rng(seed);
    let bytes = random_doc(&mut rng, enc);
    let mut h = open_as("/w/f0.txt", &bytes, enc);
    if doc(&h).format.encoding != enc || !doc(&h).read_only.is_empty() {
        return None;
    }
    h.clear_log();
    c.docs += 1;
    let mut log: Vec<String> = Vec::new();
    let mut cur = "/w/f0.txt".to_owned();
    let mut saved_once = false;
    let mut serial = 1;
    for _ in 0..30 {
        match rng.below(10) {
            0..=4 => edit(&mut h, &mut rng, &mut log, join),
            5 | 6 => {
                // 保存 (保存の間に編集することもある)
                let during = rng.below(2) == 0;
                let text_at_save = h.doc_text();
                h.cmd_with("file.save", Args::new());
                // 確認が出たら答える (ジョブは走らせない)
                if let Some(t) = dialog_title(&h)
                    && SAVE_CONFIRMS.contains(&t.as_str())
                {
                    h.text("s");
                }
                let mut k = 0;
                if during && h.pending_jobs() > 0 {
                    for _ in 0..(1 + rng.below(3)) {
                        edit(&mut h, &mut rng, &mut log, join);
                        k += 1;
                    }
                }
                let ok = settle(&mut h);
                log.push(format!("save (edits during: {k}) ok={ok}"));
                if ok {
                    c.saves += 1;
                    if k > 0 {
                        c.saves_during += 1;
                    }
                    saved_once = true;
                    // 保存の間に編集しなかった保存で、書いたファイルは保存の時点の本文 (未編集の文書の上書き保存は、照合を
                    // 待つ間に編集すると、編集の後の本文で保存し直す。03 §9.7 の CheckingUnchanged)
                    if k == 0 {
                        let disk = h.fs_read(&cur).unwrap();
                        if decode(enc, &disk) != text_at_save {
                            return Some(format!(
                                "保存したファイルを復号しても保存の時点の本文にならない\n{bytes:02X?}\n{log:#?}"
                            ));
                        }
                    }
                }
            }
            _ => {
                // 保存点で、名前を付けて保存した出力が直前に保存したファイルと一致するか
                if !saved_once || h.is_dirty() {
                    continue;
                }
                let next = format!("/w/f{serial}.txt");
                serial += 1;
                h.cmd_with("file.save_as", Args::new().with_str("path", &next));
                let ok = settle(&mut h);
                log.push(format!("save_as {next} ok={ok}"));
                if !ok {
                    continue;
                }
                let a = h.fs_read(&cur).unwrap();
                let b = h.fs_read(&next).unwrap();
                if a != b {
                    return Some(format!(
                        "保存点で名前を付けて保存した出力が、直前に保存したファイルと違う\n元: {bytes:02X?}\n前: {a:02X?}\n後: {b:02X?}\n本文: {:?}\n{log:#?}",
                        h.doc_text()
                    ));
                }
                c.compared += 1;
                cur = next;
            }
        }
        let d = doc(&h);
        if d.preserved().check(d.text(), d.format.encoding).is_err() || d.preserved_anomalies() != 0
        {
            return Some(format!(
                "表が本文と合わない・表と履歴が食い違う\n{bytes:02X?}\n{log:#?}"
            ));
        }
    }
    // AC-7: 読込の状態まで全部 Undo して、同じ文字コードで名前を付けて保存すると、元のファイルとバイト一致する
    while doc(&h).history().can_undo() {
        h.cmd("edit.undo");
    }
    let complete = doc(&h).format.roundtrip.preserve.is_complete();
    if h.doc_text() == decode(enc, &bytes) && complete {
        h.cmd_with(
            "file.save_as",
            Args::new().with_str("path", "/w/undone.txt"),
        );
        if settle(&mut h) {
            let got = h.fs_read("/w/undone.txt").unwrap();
            if got != bytes {
                return Some(format!(
                    "全部 Undo して保存しても元のファイルにならない\n元: {bytes:02X?}\n後: {got:02X?}\n{log:#?}"
                ));
            }
            c.restored += 1;
        }
    }
    None
}

/// 乱数の種 `seeds` で、`encs` の文書の操作の列を走らせる。
fn run_all(encs: &[Encoding], seeds: impl Iterator<Item = u64>, join: bool) -> Counts {
    let mut c = Counts::default();
    let mut fails = Vec::new();
    for seed in seeds {
        for &enc in encs {
            if let Some(f) = scenario(enc, seed, join, &mut c) {
                fails.push(format!("{enc:?} seed={seed:#x}\n{f}"));
            }
        }
        if fails.len() >= 3 {
            break;
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n\n"));
    c
}

// 01 §5.5 の NFR-303 AC-5〜AC-7
// verifies: NFR-303
#[test]
fn 保存したファイルと保存点の名前を付けて保存が一致し_全部_undo_すると元のファイルになる() {
    let c = run_all(
        &[Encoding::ShiftJis, Encoding::EucJp, Encoding::Iso2022Jp],
        (0..80u64).map(|i| 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i + 1) | 1),
        false,
    );
    assert!(
        c.docs > 200
            && c.saves > 600
            && c.saves_during > 300
            && c.compared > 300
            && c.restored > 120,
        "docs={} saves={} saves_during={} compared={} restored={}",
        c.docs,
        c.saves,
        c.saves_during,
        c.compared,
        c.restored
    );
}

// 01 §5.5 の NFR-303 AC-6・AC-7 (行の結合で行の内容の全体でなくなった記録は、編集した行と同じく正規形で書く)
// verifies: NFR-303
#[test]
fn iso_2022_jp_の行の結合を多くしても_保存点の出力は直前の保存と一致する() {
    let c = run_all(
        &[Encoding::Iso2022Jp, Encoding::ShiftJis],
        (0..80u64).map(|i| 0xD1B5_4A32_D192_ED03u64.wrapping_mul(i + 7) | 1),
        true,
    );
    assert!(
        c.saves > 350 && c.compared > 150 && c.restored > 60,
        "saves={} compared={} restored={}",
        c.saves,
        c.compared,
        c.restored
    );
}

// 前提の確認: 不正バイトを持たない Shift_JIS の文書の本文にあるエスケープ文字は、保存しても元のバイトに戻して (前後と
// つながって別の文字になる並びで) 黙って書かない (03 §8.3.1 の表の「バイトでない文書」。表現不能で止まる)。v0.4 の
// M23 から、貼り付けでは `<U+10FF81>` の文字列になるので (03 §8.3.1)、規則を通さずに入れる
#[test]
fn 本文にあるエスケープ文字は戻さずに表現不能で止める() {
    let bytes = b"a\xFA\x54b\n".to_vec();
    let mut h = open_as("/w/p.txt", &bytes, Encoding::ShiftJis);
    h.keys("ctrl+end");
    h.paste("\u{10FF81}A");
    assert!(
        h.doc_text().ends_with("<U+10FF81>A"),
        "貼り付けは文字列になる"
    );
    inject(&mut h, "\u{10FF81}A");
    h.cmd_with("file.save", Args::new());
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "E3");
    assert_eq!(h.fs_read("/w/p.txt").unwrap(), bytes, "書いていない");
}
