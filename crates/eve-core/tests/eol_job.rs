//! 1 MiB を超える文書の改行コードの変換 (docs/03 §8.4 の (3)(4)): ジョブで ChangeSet を作る。

use eve_core::{Args, Origin, Tick, Transaction};
use eve_testkit::Harness;
use eve_text::{ByteOff, ByteRange, ChangeSet, Edit};

fn big_crlf() -> String {
    "line of text\r\n".repeat(80_000)
}

/// CRLF の 1 MiB 超のファイルを開いた状態 (貼り付けは改行を正規化するので、ファイルから開く)。
fn opened() -> Harness {
    let mut h = Harness::new().with_file("/w/big.txt", big_crlf().as_bytes());
    h.open("/w/big.txt");
    h.run_jobs();
    h
}

fn to_lf(h: &mut Harness) {
    h.cmd_with("file.set_line_ending", Args::new().with_str("eol", "lf"));
}

fn insert_z(h: &mut Harness) {
    let doc = h.doc_id();
    let cs = {
        let t = h.editor.state().document(doc).unwrap().text();
        ChangeSet::from_sorted_edits(
            t,
            [Edit {
                range: ByteRange::new(ByteOff(0), ByteOff(0)),
                insert: "Z".into(),
            }],
        )
        .unwrap()
    };
    h.editor
        .state_mut()
        .apply_transaction(doc, Transaction::new(cs, Origin::User, Tick(0)))
        .unwrap();
}

fn message(h: &mut Harness) -> String {
    h.frame()
        .messages
        .current
        .map(|m| m.text.spans.iter().map(|s| s.text.to_string()).collect())
        .unwrap_or_default()
}

#[test]
fn 大きな文書はジョブで変換し_一度の_undo_で戻る() {
    let mut h = opened();
    assert!(h.doc_text().len() > 1 << 20);
    to_lf(&mut h);
    assert!(
        h.doc_text().contains("\r\n"),
        "ジョブの結果を待つ間は変えない"
    );
    h.run_jobs();
    let t = h.doc_text();
    assert!(!t.contains('\r') && t.starts_with("line of text\nline"));
    assert_eq!(
        message(&mut h),
        "80000 個の改行を LF に変換しました (元に戻すで戻せます)"
    );
    h.keys("ctrl+z");
    assert!(
        h.doc_text().starts_with("line of text\r\nline"),
        "本文が戻る"
    );
}

#[test]
fn 準備中に文書が変わったら_作り直して適用する() {
    let mut h = opened();
    to_lf(&mut h);
    insert_z(&mut h);
    h.run_jobs();
    assert!(
        h.doc_text().starts_with("Zline of text\nline"),
        "再投入のジョブで適用した"
    );
}

#[test]
fn 作り直しても古ければ中止する() {
    let mut h = opened();
    to_lf(&mut h);
    insert_z(&mut h);
    // 最初のジョブの結果を受けた直後 (再投入されたジョブが走る前) にもう一度変える
    h.run_jobs_once();
    insert_z(&mut h);
    h.run_jobs();
    assert!(
        h.doc_text().starts_with("ZZline of text\r\n"),
        "変換していない"
    );
    assert!(message(&mut h).contains("中止"), "{}", message(&mut h));
}
