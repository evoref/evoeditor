//! 保持方式の記録 (`PreservedMap`) の、読込・編集・Undo / Redo・再読込・開き直し・文字コードの変更・復旧への組み込み
//! (docs/03 §8.7.1、M22 の 5)。保存での差し替えと、保存の前の確認・読込の完了時の通知は `preserve_save.rs` (M22 の 6)。
//! 保存先は `MemFs`、ジョブは `ManualJobSpawner`。

use eve_core::{
    Args, Document, Encoding, FileTime, FsPath, OpenOptions, SwapBody, SwapEntry, SwapOwner,
};
use eve_encoding::{NoPreserveReason, PreserveReason, PreserveStatus};
use eve_testkit::Harness;
use eve_text::LineEnding;

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

/// 表の記録の範囲 (バイト位置の組)。
fn spans(h: &Harness) -> Vec<(usize, usize)> {
    doc(h)
        .preserved()
        .ranges()
        .map(|(r, _)| (r.start.0, r.end.0))
        .collect()
}

fn reasons(h: &Harness) -> Vec<PreserveReason> {
    doc(h).preserved().ranges().map(|(_, r)| r).collect()
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

fn open(path: &str, bytes: &[u8]) -> Harness {
    let mut h = Harness::new().with_size(80, 12).with_file(path, bytes);
    h.open(path);
    h.run_jobs();
    h
}

fn open_as(path: &str, bytes: &[u8], enc: Encoding) -> Harness {
    let mut h = Harness::new().with_size(80, 12).with_file(path, bytes);
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

/// 文書の先頭から `n` 文字目へ動く。
fn goto(h: &mut Harness, n: usize) {
    h.keys("ctrl+home");
    for _ in 0..n {
        h.keys("right");
    }
}

fn undo(h: &mut Harness) {
    h.cmd("edit.undo");
    assert_valid(h);
}

fn redo(h: &mut Harness) {
    h.cmd("edit.redo");
    assert_valid(h);
}

/// `Shift_JIS` の ￢ (U+FFE2) の IBM 拡張の重複符号化。再符号化すると 0x81CA になる (区分 B)。
const NOT_IBM: &[u8] = b"\xFA\x54";
/// EUC-JP の JIS X 0212 の文字 (復号専用。0x8FB0A1 → U+4E02 丂)。
const EUC_0212: &[u8] = b"\x8F\xB0\xA1";

/// `a￢b` / `￢￢c` の 2 行 (￢ は IBM 拡張の重複)。記録は 2 件 (2 行目の ￢￢ は 1 件にまとまる)。
fn sjis_doc() -> Vec<u8> {
    [b"a", NOT_IBM, b"b\n", NOT_IBM, NOT_IBM, b"c\n"].concat()
}

// spec: 03 §8.7.1 (集める側と持つ側・編集での写像・Undo / Redo)
#[test]
fn shift_jis_の重複符号化は記録になり_編集で位置がずれ_中を編集すると外れ_undo_で戻る() {
    let mut h = open("/w/a.txt", &sjis_doc());
    assert_eq!(doc(&h).format.encoding, Encoding::ShiftJis);
    assert_eq!(h.doc_text(), "a￢b\n￢￢c\n");
    assert_eq!(spans(&h), vec![(1, 4), (6, 12)]);
    assert_eq!(
        reasons(&h),
        vec![
            PreserveReason::DuplicateEncoding,
            PreserveReason::DuplicateEncoding
        ]
    );
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 0 });
    assert_valid(&h);
    let initial = doc(&h).preserved().clone();

    // 記録の外 (文書の先頭) の入力: 記録は残り、位置がずれる
    goto(&mut h, 0);
    h.text("XY");
    assert_eq!(spans(&h), vec![(3, 6), (8, 14)]);
    assert_valid(&h);
    undo(&mut h);
    assert_eq!(doc(&h).preserved(), &initial);

    // 記録の端に接する入力 (a と ￢ の間): 文字の範囲の記録は残る (範囲は挿入した文字を含まない)
    goto(&mut h, 1);
    h.text("Q");
    assert_eq!(h.doc_text(), "aQ￢b\n￢￢c\n");
    assert_eq!(spans(&h), vec![(2, 5), (7, 13)]);
    undo(&mut h);
    assert_eq!(doc(&h).preserved(), &initial);

    // 記録の中を編集 (￢ を消す): 外れる。Undo で戻り、Redo で再び外れる
    goto(&mut h, 1);
    h.cmd("edit.delete_forward");
    assert_eq!(h.doc_text(), "ab\n￢￢c\n");
    assert_eq!(spans(&h), vec![(3, 9)]);
    assert_valid(&h);
    undo(&mut h);
    assert_eq!(h.doc_text(), "a￢b\n￢￢c\n");
    assert_eq!(doc(&h).preserved(), &initial);
    redo(&mut h);
    assert_eq!(spans(&h), vec![(3, 9)]);
    undo(&mut h);

    // 内容が同じ文字列を打ち直しても、記録は戻らない (戻すのは Undo / Redo だけ)
    goto(&mut h, 1);
    h.cmd("edit.delete_forward");
    h.text("￢");
    assert_eq!(h.doc_text(), "a￢b\n￢￢c\n");
    assert_eq!(spans(&h), vec![(6, 12)]);
    undo(&mut h);
    undo(&mut h);
    assert_eq!(doc(&h).preserved(), &initial);
}

// spec: 03 §8.7.1 (編集での写像)
#[test]
fn 行の結合と分割では記録は残り_全選択して削除しても_undo_で戻る() {
    let mut h = open("/w/a.txt", &sjis_doc());
    let initial = doc(&h).preserved().clone();
    // 1 行目の終端を消して結合する
    goto(&mut h, 3);
    h.cmd("edit.delete_forward");
    assert_eq!(h.doc_text(), "a￢b￢￢c\n");
    assert_eq!(spans(&h), vec![(1, 4), (5, 11)]);
    assert_valid(&h);
    // 2 つの記録の間で分割する
    goto(&mut h, 3);
    h.cmd("edit.newline");
    assert_eq!(h.doc_text(), "a￢b\n￢￢c\n");
    assert_eq!(spans(&h), vec![(1, 4), (6, 12)]);
    undo(&mut h);
    undo(&mut h);
    assert_eq!(doc(&h).preserved(), &initial);

    // 全選択して削除 → 全部外れる → Undo で全部戻る → Redo で全部外れる
    h.cmd("select.all");
    h.cmd("edit.delete_backward");
    assert_eq!(h.doc_text(), "");
    assert!(doc(&h).preserved().is_empty());
    undo(&mut h);
    assert_eq!(doc(&h).preserved(), &initial);
    redo(&mut h);
    assert!(doc(&h).preserved().is_empty());
}

/// 検索語と置換文字列を入れて、すべて置換する。
fn replace_all(h: &mut Harness, find: &str, with: &str) {
    h.keys("ctrl+r");
    h.text(find);
    h.keys("tab");
    h.text(with);
    h.keys("alt+a");
    h.run_jobs();
    h.keys("esc");
}

// spec: 03 §8.7.1 (Undo / Redo の、変更の間も置き換えるノード)、03 §12.5
#[test]
fn すべて置換は_小さくても粗い逆変換でも_undo_redo_で記録が戻る() {
    // 小さな置換 (変更ごとの逆変換)
    let mut h = open("/w/a.txt", &sjis_doc());
    let initial = doc(&h).preserved().clone();
    replace_all(&mut h, "b", "bbb");
    assert_eq!(h.doc_text(), "a￢bbb\n￢￢c\n");
    assert_eq!(spans(&h), vec![(1, 4), (8, 14)]);
    assert_valid(&h);
    let after = doc(&h).preserved().clone();
    undo(&mut h);
    assert_eq!(doc(&h).preserved(), &initial);
    redo(&mut h);
    assert_eq!(doc(&h).preserved(), &after);

    // 1 万件以上の置換 (粗い逆変換、03 §12.5)。x￢ を 1 万 2 千回、x を yy に
    let bytes = [b"x".as_slice(), NOT_IBM].concat().repeat(12_000);
    let mut h = open("/w/big.txt", &bytes);
    assert_eq!(doc(&h).format.encoding, Encoding::ShiftJis);
    let initial = doc(&h).preserved().clone();
    assert_eq!(initial.len(), 12_000);
    replace_all(&mut h, "x", "yy");
    assert!(h.doc_text().starts_with("yy￢yy￢"));
    assert_eq!(doc(&h).preserved().len(), 12_000, "端に接する置換では残る");
    assert_valid(&h);
    let after = doc(&h).preserved().clone();
    undo(&mut h);
    assert!(h.doc_text().starts_with("x￢x￢"));
    assert_eq!(doc(&h).preserved(), &initial);
    redo(&mut h);
    assert_eq!(doc(&h).preserved(), &after);
    undo(&mut h);
    assert_eq!(doc(&h).preserved(), &initial);
}

// spec: 03 §8.7.1 (経路を 1 か所にまとめる: 改行コードの変換)
#[test]
fn 改行コードの変換でも記録は本文と合い_undo_で戻る() {
    let mut h = open("/w/a.txt", &sjis_doc());
    let initial = doc(&h).preserved().clone();
    h.cmd_with("file.set_line_ending", Args::new().with_str("eol", "crlf"));
    h.run_jobs();
    assert_eq!(h.doc_text(), "a￢b\r\n￢￢c\r\n");
    assert_eq!(spans(&h), vec![(1, 4), (7, 13)]);
    assert_valid(&h);
    undo(&mut h);
    assert_eq!(h.doc_text(), "a￢b\n￢￢c\n");
    assert_eq!(doc(&h).preserved(), &initial);
}

// spec: 03 §8.7.1 (再読込・開き直し・文字コードの変更)、03 §9.5
#[test]
fn 再読込は記録を新しい読込の結果に入れ替え_undo_redo_で前後の記録と状態に戻る() {
    let mut h = open("/w/a.txt", &sjis_doc());
    let before = doc(&h).preserved().clone();
    // ディスクの内容を変える (記録は 1 件)
    h.fs.add_file("/w/a.txt", &[b"zz", NOT_IBM, b"\n"].concat());
    h.cmd("file.reload");
    if h.overlay() == "dialog" {
        h.keys("l");
    }
    h.run_jobs();
    assert_eq!(h.doc_text(), "zz￢\n");
    assert_eq!(spans(&h), vec![(2, 5)]);
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 0 });
    assert_valid(&h);
    let after = doc(&h).preserved().clone();
    // 再読込の後の編集と Undo
    goto(&mut h, 2);
    h.cmd("edit.delete_forward");
    assert!(doc(&h).preserved().is_empty());
    undo(&mut h);
    assert_eq!(doc(&h).preserved(), &after);
    // 再読込の Undo: 前の本文と記録
    undo(&mut h);
    assert_eq!(h.doc_text(), "a￢b\n￢￢c\n");
    assert_eq!(doc(&h).preserved(), &before);
    redo(&mut h);
    assert_eq!(doc(&h).preserved(), &after);
}

// spec: 03 §8.7.1 (再読込・開き直し・文字コードの変更)、03 §8.8
#[test]
fn 文字コードを指定して開き直すと新しい読込の記録になる() {
    // EUC-JP の JIS X 0212 (復号専用) を含む文書を、Shift_JIS として開いてから EUC-JP で開き直す
    let bytes = [b"ab".as_slice(), EUC_0212, b"cd\n"].concat();
    let mut h = open_as("/w/e.txt", &bytes, Encoding::ShiftJis);
    assert_eq!(doc(&h).format.encoding, Encoding::ShiftJis);
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "euc-jp"),
    );
    if h.overlay() == "dialog" {
        h.keys("d");
    }
    h.run_jobs();
    assert_eq!(doc(&h).format.encoding, Encoding::EucJp);
    assert_eq!(h.doc_text(), "ab丂cd\n");
    assert_eq!(spans(&h), vec![(2, 5)]);
    assert_eq!(reasons(&h), vec![PreserveReason::DecodeOnly]);
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 0 });
    assert_valid(&h);
}

// spec: 03 §8.7.1 (保持の単位: ISO-2022-JP は行の内容。端に接する入力で外れる)
#[test]
fn iso_2022_jp_の行の記録は端に接する入力で外れ_undo_で戻る() {
    // 1 行目: 旧 JIS (ESC $ @) の「あ」。2 行目: 半角カナ (ESC ( I)。3 行目: ASCII だけ (記録なし)
    let bytes = b"\x1B$@$\"\x1B(B\n\x1B(I1\x1B(B\nabc\n";
    let mut h = open_as("/w/j.txt", bytes, Encoding::Iso2022Jp);
    assert_eq!(h.doc_text(), "あ\nｱ\nabc\n");
    assert_eq!(spans(&h), vec![(0, 3), (4, 7)]);
    assert_eq!(
        reasons(&h),
        vec![PreserveReason::StatefulEscape, PreserveReason::DecodeOnly]
    );
    assert_valid(&h);
    let initial = doc(&h).preserved().clone();
    // 1 行目の行頭に入力: 行の記録は外れる
    goto(&mut h, 0);
    h.text("x");
    assert_eq!(spans(&h), vec![(5, 8)]);
    assert_valid(&h);
    undo(&mut h);
    assert_eq!(doc(&h).preserved(), &initial);
    // 3 行目 (記録なし) の編集では、記録は残る
    goto(&mut h, 4);
    h.text("q");
    assert_eq!(spans(&h), vec![(0, 3), (4, 7)]);
    undo(&mut h);
    assert_eq!(doc(&h).preserved(), &initial);
}

// spec: 03 §8.7.1 (`file.set_encoding`)、03 §8.8
#[test]
fn 別の文字コードで保存すると記録を捨て_同じ文字コードなら残す() {
    // 同じ文字コード: 表と状態を残す (記録を使って書くので、未編集の文書は元のファイルと同じバイト列になる)
    let mut h = open("/w/a.txt", &sjis_doc());
    let initial = doc(&h).preserved().clone();
    h.cmd_with(
        "file.set_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    h.run_jobs();
    assert_ne!(
        h.overlay(),
        "dialog",
        "保持が効いている文書では確認は出ない"
    );
    assert_eq!(
        h.fs.read("/w/a.txt").unwrap(),
        sjis_doc(),
        "元のバイト列のまま"
    );
    assert_eq!(doc(&h).preserved(), &initial);
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 0 });

    // 1 つ目の ￢ を消してから (外した記録は Undo のノードが持つ)、別の文字コードで保存する: 捨てる。状態は対象外
    goto(&mut h, 1);
    h.cmd("edit.delete_forward");
    h.cmd_with(
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    h.run_jobs();
    assert_eq!(doc(&h).format.encoding, Encoding::Utf8);
    assert!(doc(&h).preserved().is_empty());
    assert_eq!(status(&h), PreserveStatus::NotApplicable);
    assert_eq!(
        h.fs.read("/w/a.txt").unwrap(),
        "ab\n￢￢c\n".as_bytes(),
        "UTF-8 で書いた"
    );
    // Undo しても、元の文字コードの記録は表に戻らない (ノードが持つ記録も捨てた)
    undo(&mut h);
    assert_eq!(h.doc_text(), "a￢b\n￢￢c\n");
    assert!(doc(&h).preserved().is_empty());
    redo(&mut h);
    assert!(doc(&h).preserved().is_empty());
}

// spec: 03 §8.7.1 (再読込・開き直し・文字コードの変更)
#[test]
fn 別の文字コードで保存した後に再読込の前まで_undo_すると_保持なしになる() {
    let mut h = open("/w/a.txt", &sjis_doc());
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 0 });
    // 再読込 (Shift_JIS のまま。記録は 1 件)
    h.fs.add_file("/w/a.txt", &[b"zz", NOT_IBM, b"\n"].concat());
    h.cmd("file.reload");
    if h.overlay() == "dialog" {
        h.keys("l");
    }
    h.run_jobs();
    assert_eq!(spans(&h), vec![(2, 5)]);
    // 別の文字コードで保存する: 表と、履歴が持つ前後の表を捨てる
    h.cmd_with(
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    h.run_jobs();
    assert_eq!(doc(&h).format.encoding, Encoding::Utf8);
    assert!(doc(&h).preserved().is_empty());
    // 再読込の Undo: 書式は再読込の前 (Shift_JIS) に戻るが、元のバイト列は残っていないので「保持なし」
    undo(&mut h);
    assert_eq!(h.doc_text(), "a￢b\n￢￢c\n");
    assert_eq!(doc(&h).format.encoding, Encoding::ShiftJis);
    assert!(doc(&h).preserved().is_empty());
    assert_eq!(
        status(&h),
        PreserveStatus::Unavailable(NoPreserveReason::Discarded)
    );
    assert_valid(&h);
    // Redo でも同じ (再読込の後の書式。表は空)
    redo(&mut h);
    assert_eq!(doc(&h).format.encoding, Encoding::ShiftJis);
    assert!(doc(&h).preserved().is_empty());
    assert_eq!(
        status(&h),
        PreserveStatus::Unavailable(NoPreserveReason::Discarded)
    );
}

fn swap_entry(path: &str) -> SwapEntry {
    SwapEntry {
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
        encoding: None,
    }
}

// spec: 03 §8.7.1 (緊急退避・スワップ・復旧)、03 §10.3
#[test]
fn 復旧を適用すると保持なしになり_undo_で記録と状態が戻る() {
    let mut h = Harness::new()
        .with_size(80, 12)
        .with_file("/w/a.txt", &sjis_doc());
    h.fs.add_swap_entry(
        swap_entry("/w/a.txt"),
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
    h.advance_ms(0);
    let initial = doc(&h).preserved().clone();
    assert_eq!(initial.len(), 2);
    assert_eq!(h.overlay(), "dialog", "復旧の提案");
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), "a￢b\n復旧\n");
    assert!(
        doc(&h).preserved().is_empty(),
        "スワップには元のバイト列が無い"
    );
    assert_eq!(
        status(&h),
        PreserveStatus::Unavailable(NoPreserveReason::Recovered)
    );
    undo(&mut h);
    assert_eq!(h.doc_text(), "a￢b\n￢￢c\n");
    assert_eq!(doc(&h).preserved(), &initial);
    assert_eq!(status(&h), PreserveStatus::Active { unpreserved: 0 });
    redo(&mut h);
    assert!(doc(&h).preserved().is_empty());
    assert_eq!(
        status(&h),
        PreserveStatus::Unavailable(NoPreserveReason::Recovered)
    );
}

// spec: 03 §7.5 (予算で捨てたノードの記録はノードと一緒に捨てる)
#[test]
fn undo_の底を捨てても表は壊れない() {
    let mut h = open("/w/a.txt", &sjis_doc());
    // 1 つ目の ￢ を消し、離れた位置に入力してノードを分ける
    goto(&mut h, 1);
    h.cmd("edit.delete_forward");
    h.advance_ms(2000);
    goto(&mut h, 0);
    h.text("z");
    let id = h.editor.state().active_document().unwrap();
    // 最も古いノード (外した記録を持つ) を捨てる
    let d = h.editor.state_mut().document_mut(id).unwrap();
    assert!(d.trim_history_oldest().is_some());
    undo(&mut h);
    assert_eq!(h.doc_text(), "ab\n￢￢c\n");
    assert_eq!(spans(&h), vec![(3, 9)]);
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), "ab\n￢￢c\n", "捨てたノードには戻れない");
    assert_valid(&h);
}

// spec: 03 §8.7.1 (区分 A は対象外)
#[test]
fn utf_8_の文書は表が空で状態は対象外() {
    let mut h = open("/w/u.txt", "a￢b\n".as_bytes());
    assert_eq!(doc(&h).format.encoding, Encoding::Utf8);
    assert!(doc(&h).preserved().is_empty());
    assert_eq!(status(&h), PreserveStatus::NotApplicable);
    h.text("x");
    assert!(doc(&h).preserved().is_empty());
}

// spec: 03 §8.7.1 (「保持なし」: 不正バイトを含む ISO-2022-JP)
#[test]
fn 不正バイトを含む_iso_2022_jp_の文書は保持なし() {
    // JIS X 0208 の状態のまま改行する (0x0A が不正バイトになる。03 §8.7 のコーパスの (5))
    let mut h = open_as("/w/j.txt", b"\x1B$B$\"\n\x1B(B", Encoding::Iso2022Jp);
    if h.overlay() == "dialog" {
        h.text("r"); // 不正バイトが多い: 読み取り専用で開く
        h.run_jobs();
    }
    assert_eq!(doc(&h).format.encoding, Encoding::Iso2022Jp);
    assert!(doc(&h).format.invalid_bytes > 0);
    assert!(doc(&h).preserved().is_empty());
    assert_eq!(
        status(&h),
        PreserveStatus::Unavailable(NoPreserveReason::InvalidBytes)
    );
}
