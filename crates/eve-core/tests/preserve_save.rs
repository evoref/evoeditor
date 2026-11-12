//! 保存での保持方式の記録の差し替えと、保存の前の確認・読込の完了時の通知 (docs/03 §8.6 E2 / E2'、§8.7.1、M22 の 6)。
//! 01 §5.5 の NFR-303 AC-5〜AC-7 の受入基準と、確認・通知の条件の表の各行。保存先は `MemFs`、ジョブは `ManualJobSpawner`。

use eve_core::{
    Args, Document, Encoding, FileTime, FsPath, OpenOptions, SwapBody, SwapEntry, SwapOwner,
};
use eve_encoding::{NoPreserveReason, PreserveStatus};
use eve_testkit::Harness;
use eve_text::LineEnding;
use eve_view::{Answer, OverlayModel, Query};

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

fn status(h: &Harness) -> PreserveStatus {
    doc(h).format.roundtrip.preserve
}

/// 表が本文と合っている (03 §8.7.1 の不変条件) ことと、表と履歴が食い違っていないこと。
fn assert_valid(h: &Harness) {
    let d = doc(h);
    assert_eq!(d.preserved().check(d.text(), d.format.encoding), Ok(()));
    assert_eq!(d.preserved_anomalies(), 0);
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
    assert_eq!(doc(&h).format.encoding, enc);
    h
}

fn run(h: &mut Harness, id: &str, args: Args) {
    h.cmd_with(id, args);
    h.run_jobs();
}

fn save(h: &mut Harness) {
    run(h, "file.save", Args::new());
}

fn save_as(h: &mut Harness, path: &str) {
    run(h, "file.save_as", Args::new().with_str("path", path));
}

fn set_encoding(h: &mut Harness, enc: &str) {
    run(
        h,
        "file.set_encoding",
        Args::new().with_str("encoding", enc),
    );
}

/// これまでに出たメッセージの `MessageKey` (読むと捨てる)。
fn messages(h: &mut Harness) -> Vec<String> {
    let m = h.messages();
    h.clear_log();
    m
}

/// 文書の先頭から `n` 文字目へ動く。
fn goto(h: &mut Harness, n: usize) {
    h.keys("ctrl+home");
    for _ in 0..n {
        h.keys("right");
    }
}

fn sjis(s: &str) -> Vec<u8> {
    encoding_rs::SHIFT_JIS.encode(s).0.into_owned()
}

fn euc(s: &str) -> Vec<u8> {
    encoding_rs::EUC_JP.encode(s).0.into_owned()
}

fn decode(enc: &'static encoding_rs::Encoding, b: &[u8]) -> String {
    enc.decode_without_bom_handling(b).0.into_owned()
}

/// 出ている確認の (題, 本文)。確認が無ければ `None`。
fn dialog(h: &Harness) -> Option<(String, String)> {
    let window = h.editor.state().first_window()?;
    let text =
        |c: &eve_view::CellText| -> String { c.spans.iter().map(|sp| sp.text.as_str()).collect() };
    match h.query(&Query::Overlay { window }).ok()? {
        Answer::Overlay(Some(OverlayModel::Confirm { title, message, .. })) => {
            Some((text(&title), text(&message)))
        }
        _ => None,
    }
}

/// 保存の前の確認の種類 (題で見分ける。04 §15.2 の `ui.confirm.save.*.title`)。確認が無ければ空。
fn confirm(h: &mut Harness) -> &'static str {
    let Some((title, _)) = dialog(h) else {
        return "";
    };
    match title.as_str() {
        "エスケープの書き方が変わります" => "escape_normalized",
        "別のバイト表現で保存されるかもしれません" => "roundtrip_unchecked",
        "別のバイト表現で保存されます" => "lossy_roundtrip",
        _ => "other",
    }
}

// ---------- 受入基準 (01 §5.5 の NFR-303 AC-5〜AC-7) ----------

/// `Shift_JIS` の区分 B のコーパス: NEC 特殊文字 ≒ (0x8790。代表は 0x81E0)、IBM 拡張 ￢ (0xFA54。代表は 0x81CA)、
/// NEC 選定 IBM 拡張 纊 (0xED40。代表は 0xFA5C)・ⅰ (0xEEEF。代表は 0xFA40) を、編集しない行に置く。
fn sjis_b() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let l1 = [
        sjis("区分B: "),
        b"\x87\x90\xFA\x54\xED\x40".to_vec(),
        b"\r\n".to_vec(),
    ]
    .concat();
    let l2 = sjis("編集する行\r\n");
    let l3 = [b"x".to_vec(), b"\xEE\xEF".to_vec(), sjis("おわり\r\n")].concat();
    (l1, l2, l3)
}

// 01 §5.5 の NFR-303 AC-5 (v0.4 の AC は `tests/ac-checklist.toml` の対象外なので、要件 ID で印を付ける)
// verifies: NFR-303
#[test]
fn shift_jis_の区分_b_でない所を編集して保存すると編集していない範囲はバイト一致する() {
    let (l1, l2, l3) = sjis_b();
    let bytes = [l1.clone(), l2.clone(), l3.clone()].concat();
    let mut h = open_as("/w/a.txt", &bytes, Encoding::ShiftJis);
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 0 });
    assert_eq!(
        doc(&h).preserved().len(),
        2,
        "隣り合う区分 B の文字は 1 件にまとまる"
    );
    // 保持が効いている文書では、読込の完了時の通知を出さない
    let m = messages(&mut h);
    assert!(
        !m.iter()
            .any(|k| k.starts_with("info.open.preserve") || k == "info.open.lossy_roundtrip"),
        "{m:?}"
    );
    // 2 行目 (区分 B でない) の行頭に入力して上書き保存: 確認も表現不能も出ない
    h.keys("ctrl+home");
    h.keys("down");
    h.text("追記");
    save(&mut h);
    assert_eq!(
        confirm(&mut h),
        "",
        "保持が効いている文書では確認を出さない"
    );
    let want = [l1.clone(), sjis("追記"), l2.clone(), l3.clone()].concat();
    assert_eq!(
        h.fs.read("/w/a.txt").unwrap(),
        want,
        "編集していない範囲は元のバイト列"
    );
    assert!(!doc(&h).is_dirty());
    assert_valid(&h);
    // 同じ文字コードで名前を付けて保存: 同じバイト列
    save_as(&mut h, "/w/b.txt");
    assert_eq!(confirm(&mut h), "");
    assert_eq!(h.fs.read("/w/b.txt").unwrap(), want);
    // 保存した後も記録と要約は残る (同じ文字コード)
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 0 });
    assert!(doc(&h).format.roundtrip.lossy_lines > 0);
}

// 01 §5.5 の NFR-303 AC-5 (v0.4 の AC は `tests/ac-checklist.toml` の対象外なので、要件 ID で印を付ける)
// verifies: NFR-303
#[test]
fn euc_jp_の補助漢字と別のコードに写る文字は_編集していなければ元のバイト列で書く() {
    // 補助漢字 丂 (0x8FB0A1。復号専用)、0x8FA2B7 (～ U+FF5E。再符号化すると 0xA1C1)
    let l1 = [
        euc("補助"),
        b"\x8F\xB0\xA1\x8F\xA2\xB7".to_vec(),
        b"\n".to_vec(),
    ]
    .concat();
    let l2 = euc("編集する行\n");
    let bytes = [l1.clone(), l2.clone()].concat();
    let mut h = open_as("/w/e.txt", &bytes, Encoding::EucJp);
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 0 });
    // v0.3 までは補助漢字の行を数えるだけで、編集して保存すると表現不能で止まった (FR-112 AC-4 (b))
    h.keys("ctrl+end");
    h.text("末尾");
    save(&mut h);
    assert_eq!(h.overlay(), "", "確認も表現不能の選択肢も出ない");
    assert_eq!(
        h.fs.read("/w/e.txt").unwrap(),
        [l1.clone(), l2.clone(), euc("末尾")].concat()
    );
    save_as(&mut h, "/w/f.txt");
    assert_eq!(
        h.fs.read("/w/f.txt").unwrap(),
        h.fs.read("/w/e.txt").unwrap()
    );
    assert_valid(&h);
}

/// ISO-2022-JP の区分 D のコーパス: 旧 JIS (`ESC $ @`) の行、半角カナ (`ESC ( I`) の行、ローマ字 (`ESC ( J`) で始まり ASCII
/// 状態に戻る行、編集する ASCII の行。
fn iso_d() -> Vec<&'static [u8]> {
    vec![
        b"\x1B$@$\"$$\x1B(B\r\n",
        b"\x1B(I1234\x1B(B\r\n",
        b"\x1B(J\\\x1B(Babc\r\n",
        b"edit me\r\n",
    ]
}

// 01 §5.5 の NFR-303 AC-6 (v0.4 の AC は `tests/ac-checklist.toml` の対象外なので、要件 ID で印を付ける)
// verifies: NFR-303
#[test]
fn iso_2022_jp_の編集していない_ascii_状態で始まり終わる行はバイト一致する() {
    let lines = iso_d();
    let bytes = lines.concat();
    let mut h = open_as("/w/j.txt", &bytes, Encoding::Iso2022Jp);
    assert_eq!(h.doc_text(), "あい\r\nｱｲｳｴ\r\n¥abc\r\nedit me\r\n");
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 0 });
    assert_eq!(doc(&h).preserved().len(), 3);
    // 4 行目の末尾に入力して保存: 半角カナの行も表現不能にならない
    h.keys("ctrl+home");
    for _ in 0..3 {
        h.keys("down");
    }
    h.keys("end");
    h.text("!");
    save(&mut h);
    assert_eq!(h.overlay(), "");
    let want = [lines[0], lines[1], lines[2], b"edit me!\r\n"].concat();
    assert_eq!(h.fs.read("/w/j.txt").unwrap(), want);
    save_as(&mut h, "/w/k.txt");
    assert_eq!(h.fs.read("/w/k.txt").unwrap(), want);
    assert_valid(&h);
}

// 01 §5.5 の NFR-303 AC-7 (v0.4 の AC は `tests/ac-checklist.toml` の対象外なので、要件 ID で印を付ける)
// verifies: NFR-303
#[test]
fn 編集した範囲には元のバイト列を書かず_undo_で戻した範囲は元のバイト列で書く() {
    // a￢￢b (￢ は IBM 拡張の重複 0xFA54。2 つで 1 件の記録)
    let bytes = b"a\xFA\x54\xFA\x54b\n".to_vec();
    let mut h = open_as("/w/a.txt", &bytes, Encoding::ShiftJis);
    assert_eq!(doc(&h).preserved().len(), 1);
    // 記録の中を編集 (1 つ目の ￢ を消す): 記録は外れ、残りの ￢ は代表バイト列 (0x81CA)
    goto(&mut h, 1);
    h.cmd("edit.delete_forward");
    save(&mut h);
    assert_eq!(
        h.overlay(),
        "",
        "編集した範囲が代表バイト列になるのは区分 C (確認しない)"
    );
    assert_eq!(h.fs.read("/w/a.txt").unwrap(), b"a\x81\xCAb\n");
    // 同じ文字を打ち直しても、記録は戻らない
    h.text("￢");
    save(&mut h);
    assert_eq!(h.fs.read("/w/a.txt").unwrap(), b"a\x81\xCA\x81\xCAb\n");
    // Undo で戻した範囲は未編集: 元のバイト列で書く
    h.cmd("edit.undo");
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), "a￢￢b\n");
    save(&mut h);
    assert_eq!(h.fs.read("/w/a.txt").unwrap(), bytes);
    // Redo で再び編集した状態: 代表バイト列
    h.cmd("edit.redo");
    save(&mut h);
    assert_eq!(h.fs.read("/w/a.txt").unwrap(), b"a\x81\xCAb\n");
    assert_valid(&h);
    // 保存したファイルを開き直すと、本文は保存の時点の文書と一致する
    let saved = h.fs.read("/w/a.txt").unwrap();
    assert_eq!(decode(encoding_rs::SHIFT_JIS, &saved), h.doc_text());
}

// ---------- 読込の完了時の通知 (03 §8.7.1) ----------

#[test]
fn 保持できなかった箇所がある文書は読込の完了時に数を言って知らせる() {
    let mut h = Harness::new()
        .with_size(100, 16)
        .with_file("/w/j.txt", ISO_PARTIAL);
    h.clear_log();
    h.editor.state_mut().open_path(
        "/w/j.txt",
        OpenOptions {
            forced: Some(Encoding::Iso2022Jp),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 1 });
    let m = messages(&mut h);
    assert!(
        m.contains(&"info.open.preserve_partial".to_owned()),
        "{m:?}"
    );
    assert!(!m.contains(&"info.open.lossy_roundtrip".to_owned()));
}

#[test]
fn 保持なしの文書は読込の完了時に数を言わずに知らせる() {
    // 1 行目が 4 MiB を超える ISO-2022-JP (検査を打ち切る)。2 行目に旧 JIS
    let mut bytes = vec![b'a'; 4 * 1024 * 1024 + 1];
    bytes.extend_from_slice(b"\n\x1B$@$\"\x1B(B\n");
    let mut h = Harness::new()
        .with_size(100, 16)
        .with_file("/w/j.txt", &bytes);
    h.clear_log();
    h.editor.state_mut().open_path(
        "/w/j.txt",
        OpenOptions {
            forced: Some(Encoding::Iso2022Jp),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    assert_eq!(
        status(&h),
        PreserveStatus::Unavailable(NoPreserveReason::Incomplete)
    );
    let m = messages(&mut h);
    assert!(m.contains(&"info.open.preserve_off".to_owned()), "{m:?}");
    // 未編集でも、同じ文字コードで名前を付けて保存するときは確認を出す (03 §8.6 E2')
    save_as(&mut h, "/w/k.txt");
    assert_eq!(confirm(&mut h), "roundtrip_unchecked");
    assert!(h.fs.read("/w/k.txt").is_none(), "まだ書いていない");
}

#[test]
fn 区分_b_の箇所の無い文書と保持が効いている文書は通知しない() {
    for (bytes, enc) in [
        (sjis("ふつうの文書\n"), Encoding::ShiftJis),
        (b"\x1B$@$\"\x1B(B\n".to_vec(), Encoding::Iso2022Jp),
    ] {
        let mut h = Harness::new()
            .with_size(100, 16)
            .with_file("/w/a.txt", &bytes);
        h.clear_log();
        h.editor.state_mut().open_path(
            "/w/a.txt",
            OpenOptions {
                forced: Some(enc),
                ..OpenOptions::default()
            },
        );
        h.run_jobs();
        let m = messages(&mut h);
        assert!(
            !m.iter()
                .any(|k| k.starts_with("info.open.preserve") || k == "info.open.lossy_roundtrip"),
            "{enc:?} {m:?}"
        );
    }
}

// ---------- 保存の前の確認 (03 §8.6 E2'、§8.7.1) の条件の表 ----------

/// 旧 JIS の行 (記録) と、ASCII の行と、ローマ字状態のまま改行する最後の行 (ASCII 状態で終わらない、保持できなかった
/// 箇所。後ろの行は ASCII 状態で始まらないので、最後に置く)。
const ISO_PARTIAL: &[u8] = b"\x1B$@$\"\x1B(B\nabc\n\x1B(J\\\n";

#[test]
fn 保持できなかった箇所がある文書は_未編集でも同じ文字コードで書くとき確認する() {
    let mut h = open_as("/w/j.txt", ISO_PARTIAL, Encoding::Iso2022Jp);
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 1 });
    // 未編集の上書き保存は書き込まない (確認も出ない)
    messages(&mut h);
    save(&mut h);
    assert_eq!(confirm(&mut h), "");
    assert!(messages(&mut h).contains(&"info.save.unchanged".to_owned()));
    // 未編集の名前を付けて保存 (同じ文字コード): 確認する (v0.3 までは出なかった)
    save_as(&mut h, "/w/k.txt");
    assert_eq!(confirm(&mut h), "escape_normalized");
    h.keys("esc");
    assert!(h.fs.read("/w/k.txt").is_none());
    // `file.set_encoding` で同じ文字コードを選ぶ: 確認する。既定は「保存」(文字は変わらない)
    set_encoding(&mut h, "iso-2022-jp");
    assert_eq!(confirm(&mut h), "escape_normalized");
    h.keys("enter");
    h.run_jobs();
    let saved = h.fs.read("/w/j.txt").unwrap();
    assert_eq!(
        saved, b"\x1B$@$\"\x1B(B\nabc\n\x1B(J\\\x1B(B\n",
        "記録の行は元のバイト列、保持できなかった行は正規形"
    );
    // 承認した Revision では繰り返さない。編集すると、また確認する
    h.keys("ctrl+end");
    h.text("x");
    save(&mut h);
    assert_eq!(confirm(&mut h), "escape_normalized");
    h.text("s");
    h.run_jobs();
    assert!(!doc(&h).is_dirty());
    // 違う文字コードへ書くときは確認しない (E1 / E3 で扱う)
    set_encoding(&mut h, "utf-8");
    assert_eq!(confirm(&mut h), "");
    assert_eq!(doc(&h).format.encoding, Encoding::Utf8);
    assert_eq!(status(&h), PreserveStatus::NotApplicable);
}

/// 旧 JIS の行が 2 つ (どちらも記録) と、ASCII の行。
const ISO_TWO: &[u8] = b"\x1B$@$\"\x1B(B\n\x1B$@$$\x1B(B\nabc\n";

// 行を結合して行の内容の全体でなくなった行の記録は、その行を編集した行と同じ (区分 C): 確認なしに正規形で書く。記録は
// 表に残るので、Undo で行を分けて保存すると元のバイト列に戻る (01 §5.5 の NFR-303 AC-7、03 §8.7.1)
// verifies: NFR-303
#[test]
fn iso_2022_jp_で記録の行を結合すると確認なしに正規形で書き_undo_して保存すると元のファイルに戻る()
{
    let mut h = open_as("/w/j.txt", ISO_TWO, Encoding::Iso2022Jp);
    assert_eq!(h.doc_text(), "あ\nい\nabc\n");
    assert_eq!(doc(&h).preserved().len(), 2);
    // 1 行目の終端を消して 2 行を結合する: 記録は表に残るが、どちらも行の内容の全体でなくなる
    goto(&mut h, 1);
    h.cmd("edit.delete_forward");
    assert_eq!(h.doc_text(), "あい\nabc\n");
    assert_eq!(doc(&h).preserved().len(), 2);
    save(&mut h);
    assert_eq!(confirm(&mut h), "", "編集した行と同じく、確認しない");
    assert_eq!(h.fs.read("/w/j.txt").unwrap(), b"\x1B$B$\"$$\x1B(B\nabc\n");
    // 保存しても記録は表に残る (記録が表すのは範囲の今の文字列の元のバイト列で、ディスクの内容ではない)
    assert_eq!(doc(&h).preserved().len(), 2);
    assert_valid(&h);
    // Undo で行を分けて保存すると、元のファイルとバイト一致する
    h.cmd("edit.undo");
    assert_valid(&h);
    assert_eq!(h.doc_text(), "あ\nい\nabc\n");
    save(&mut h);
    assert_eq!(confirm(&mut h), "");
    assert_eq!(h.fs.read("/w/j.txt").unwrap(), ISO_TWO);
    // Redo で再び結合すると正規形
    h.cmd("edit.redo");
    assert_valid(&h);
    save(&mut h);
    assert_eq!(h.fs.read("/w/j.txt").unwrap(), b"\x1B$B$\"$$\x1B(B\nabc\n");
}

#[test]
fn 保存の間に編集しても_書かれるのは保存を始めた時点の内容で_表は食い違わない() {
    let mut h = open_as("/w/j.txt", ISO_TWO, Encoding::Iso2022Jp);
    // 3 行目を編集して保存を始め、保存のジョブが終わる前に 1 行目の行頭に入力する (1 行目の記録は外れる)
    h.keys("ctrl+end");
    h.text("z");
    h.cmd("file.save");
    assert!(h.pending_jobs() > 0);
    h.keys("ctrl+home");
    h.text("y");
    h.run_jobs();
    assert_eq!(
        h.fs.read("/w/j.txt").unwrap(),
        [ISO_TWO, b"z"].concat(),
        "保存を始めた時点の内容 (記録の行は元のバイト列)"
    );
    assert_eq!(h.doc_text(), "yあ\nい\nabc\nz");
    assert!(doc(&h).is_dirty());
    assert_eq!(doc(&h).preserved().len(), 1);
    assert_valid(&h);
    // 保存の間の編集を Undo すると、保存した時点に戻り、記録も戻る
    h.cmd("edit.undo");
    assert_valid(&h);
    assert!(!doc(&h).is_dirty());
    assert_eq!(doc(&h).preserved().len(), 2);
    save_as(&mut h, "/w/k.txt");
    assert_eq!(
        h.fs.read("/w/k.txt").unwrap(),
        h.fs.read("/w/j.txt").unwrap()
    );
}

/// `a￢￢b` (隣り合う区分 B の文字は 1 件の記録にまとまる。￢ は IBM 拡張の重複 0xFA54)。
const SJIS_RUN: &[u8] = b"a\xFA\x54\xFA\x54b\n";

// 03 §8.7.1: 隣り合う区分 B の文字の並びの内側への挿入は、並びの全体を編集したものと扱う (並びの全部が代表バイト列)。
// 並びの端への挿入は記録を残す
// verifies: NFR-303
#[test]
fn 隣り合う区分_b_の文字の間への挿入は並びの全部を代表バイト列にし_端への挿入は元のバイト列を残す()
{
    let mut h = open_as("/w/a.txt", SJIS_RUN, Encoding::ShiftJis);
    assert_eq!(doc(&h).preserved().len(), 1);
    // 並びの内側 (2 つの ￢ の間) に入力
    goto(&mut h, 2);
    h.text("x");
    save(&mut h);
    assert_eq!(confirm(&mut h), "");
    assert_eq!(h.fs.read("/w/a.txt").unwrap(), b"a\x81\xCAx\x81\xCAb\n");
    h.cmd("edit.undo");
    save(&mut h);
    assert_eq!(h.fs.read("/w/a.txt").unwrap(), SJIS_RUN);
    // 並びの端 (前と後ろ) に入力
    goto(&mut h, 1);
    h.text("p");
    h.advance_ms(2000);
    goto(&mut h, 4);
    h.text("q");
    save(&mut h);
    assert_eq!(h.fs.read("/w/a.txt").unwrap(), b"ap\xFA\x54\xFA\x54qb\n");
    assert_valid(&h);
}

#[test]
fn 復旧した文書は区分_b_の箇所があれば数を言わずに確認する() {
    let swap = |path: &str| SwapEntry {
        dir: FsPath::from("/s/swap/aaaa-4242.swp"),
        path: Some(FsPath::from(path)),
        untitled_name: None,
        revision: 7,
        len: 12,
        pid: 4242,
        owner: SwapOwner::Stale,
        sensitive: false,
        modified: Some(FileTime {
            secs: 4_000_000_000,
            nanos: 0,
        }),
        readable: true,
    };
    // a￢b / ￢￢c (￢ は IBM 拡張の重複 0xFA54。保持方式の記録の結線の試験と同じ文書で、自動判定で Shift_JIS)
    let bytes = b"a\xFA\x54b\n\xFA\x54\xFA\x54c\n".to_vec();
    let mut h = Harness::new()
        .with_size(100, 16)
        .with_file("/w/a.txt", &bytes);
    h.fs.add_swap_entry(
        swap("/w/a.txt"),
        Some(SwapBody {
            text: "a￢b\n復旧\n".to_owned(),
            encoding: Encoding::ShiftJis,
            has_bom: false,
            default_eol: LineEnding::Lf,
            pua_escaped: false,
        }),
    );
    h.open("/w/a.txt");
    h.run_jobs();
    assert_eq!(doc(&h).format.encoding, Encoding::ShiftJis);
    h.advance_ms(0);
    assert_eq!(h.overlay(), "dialog", "復旧の提案");
    h.answer(0);
    h.run_jobs();
    assert_eq!(
        status(&h),
        PreserveStatus::Unavailable(NoPreserveReason::Recovered)
    );
    save(&mut h);
    assert_eq!(confirm(&mut h), "roundtrip_unchecked");
    assert_eq!(h.fs.read("/w/a.txt").unwrap(), bytes, "まだ書いていない");
    // 全保存は、確認が要る文書を保存しない
    h.keys("esc");
    run(&mut h, "file.save_all", Args::new());
    assert_eq!(h.fs.read("/w/a.txt").unwrap(), bytes);
}

#[test]
fn 違う文字コードで保存した後に_undo_で戻った文書は保持なしとして確認する() {
    // 再読込の後に違う文字コードで保存し、再読込の前まで Undo する (書式は Shift_JIS、記録は捨てた)
    let mut h = Harness::new()
        .with_size(100, 16)
        .with_file("/w/a.txt", b"a\xFA\x54b\n");
    h.editor.state_mut().open_path(
        "/w/a.txt",
        OpenOptions {
            forced: Some(Encoding::ShiftJis),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    h.fs.add_file("/w/a.txt", b"zz\xFA\x54\n");
    h.cmd("file.reload");
    h.run_jobs();
    set_encoding(&mut h, "utf-8");
    h.cmd("edit.undo");
    assert_eq!(doc(&h).format.encoding, Encoding::ShiftJis);
    assert_eq!(
        status(&h),
        PreserveStatus::Unavailable(NoPreserveReason::Discarded)
    );
    save_as(&mut h, "/w/b.txt");
    assert_eq!(confirm(&mut h), "roundtrip_unchecked");
}

#[test]
fn 全保存は確認が要る文書だけを残し_保持が効いている文書は保存する() {
    let mut h = open_as("/w/j.txt", ISO_PARTIAL, Encoding::Iso2022Jp);
    h.keys("ctrl+end");
    h.text("x");
    h.fs.add_file("/w/s.txt", b"a\xFA\x54b\n");
    h.editor.state_mut().open_path(
        "/w/s.txt",
        OpenOptions {
            forced: Some(Encoding::ShiftJis),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    h.keys("ctrl+end");
    h.text("y");
    run(&mut h, "file.save_all", Args::new());
    assert_eq!(
        h.fs.read("/w/s.txt").unwrap(),
        b"a\xFA\x54b\ny",
        "保持が効いている文書は確認なしに保存"
    );
    assert_eq!(
        h.fs.read("/w/j.txt").unwrap(),
        ISO_PARTIAL,
        "確認が要る文書は保存しない"
    );
}

// ---------- 置換して保存 (03 §8.6 の E3 の選択肢 2) ----------

#[test]
fn 置換して保存は_編集していない範囲の補助漢字と半角カナを対象にしない() {
    // EUC-JP: 補助漢字の行は編集せず、別の行に表現できない文字を入れる
    let bytes = [euc("補助の行: "), b"\x8F\xB0\xA1\n".to_vec(), euc("編集\n")].concat();
    let mut h = open_as("/w/e.txt", &bytes, Encoding::EucJp);
    h.keys("ctrl+end");
    h.text("😀");
    save(&mut h);
    assert_eq!(h.overlay(), "dialog", "E3 の選択肢");
    h.text("r");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "置換の一覧");
    h.text("r");
    h.run_jobs();
    assert_eq!(h.doc_text(), "補助の行: 丂\n編集\n?");
    assert_eq!(
        h.fs.read("/w/e.txt").unwrap(),
        [bytes.clone(), b"?".to_vec()].concat()
    );
    // ISO-2022-JP: 編集していない半角カナの行は全角にしない
    let bytes = b"\x1B(I123\x1B(B\nabc\n".to_vec();
    let mut h = open_as("/w/j.txt", &bytes, Encoding::Iso2022Jp);
    h.keys("ctrl+end");
    h.text("😀");
    save(&mut h);
    h.text("r");
    h.run_jobs();
    h.text("r");
    h.run_jobs();
    assert_eq!(h.doc_text(), "ｱｲｳ\nabc\n?");
    assert_eq!(
        h.fs.read("/w/j.txt").unwrap(),
        [bytes, b"?".to_vec()].concat()
    );
}

// ---------- 確かめに合わずに止まった保存の後 (03 §8.7.1 の「保存」) ----------

#[test]
fn 確かめに合わずに保存が止まったら記録を捨てて保持なしにし_次の保存は確認の後に記録を使わずに書く()
{
    let bytes = b"a\xFA\x54b\n".to_vec();
    let mut h = open_as("/w/a.txt", &bytes, Encoding::ShiftJis);
    assert_eq!(doc(&h).preserved().len(), 1);
    h.keys("ctrl+home");
    h.text("x");
    // 保存のジョブの結果に、確かめの不一致 (実装の誤りへの備え。ふつうは起きない) を渡す
    let eve_core::CommandResult::Pending(job) = h.cmd("file.save") else {
        panic!("保存のジョブを投入する");
    };
    h.editor.on_job_result(eve_core::JobOutput {
        job,
        body: eve_core::JobResult::Failed {
            err: eve_core::JobError::VerifyFailed,
        },
    });
    h.cancel_jobs();
    h.run_jobs();
    assert_eq!(
        status(&h),
        PreserveStatus::Unavailable(NoPreserveReason::VerifyFailed)
    );
    assert!(doc(&h).preserved().is_empty(), "記録を全部捨てる");
    assert_valid(&h);
    assert_eq!(h.overlay(), "dialog", "保存の失敗の選択肢");
    assert_eq!(h.fs.read("/w/a.txt").unwrap(), bytes, "何も書いていない");
    h.keys("esc");
    // 次の保存は、保存の前の確認を経て、記録を使わずに (代表バイト列で) 書く
    save(&mut h);
    assert_eq!(confirm(&mut h), "roundtrip_unchecked");
    h.text("s");
    h.run_jobs();
    assert_eq!(h.fs.read("/w/a.txt").unwrap(), b"xa\x81\xCAb\n");
    assert!(!doc(&h).is_dirty());
    // Undo しても記録は戻らない (履歴の記録も捨てた)
    h.cmd("edit.undo");
    assert!(doc(&h).preserved().is_empty());
    assert_valid(&h);
}
