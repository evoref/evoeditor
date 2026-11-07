//! 無題の採番 (docs/03 §9.8 の「無題の採番」): 起動ごとの通し番号で、閉じても再利用しない。
//! バッファ一覧・ステータスバー・タイトル・`Query::Documents`・緊急退避・スワップが同じ番号を使う。

use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_view::{Answer, DocumentId, OverlayModel, Query};

fn status_name(h: &mut Harness) -> String {
    let f = h.frame();
    cell_text(&f.status.items[0].long)
}

fn untitled_names(h: &Harness) -> Vec<(DocumentId, Option<String>)> {
    match h.editor.query(&Query::Documents).expect("応答する") {
        Answer::Documents(ds) => ds.into_iter().map(|d| (d.doc, d.untitled_name)).collect(),
        a => panic!("{a:?}"),
    }
}

/// 起動時の無題 (1) に、`file.new` で無題を足した状態。
fn two_untitled() -> (Harness, DocumentId, DocumentId) {
    let mut h = Harness::new();
    let first = h.doc_id();
    h.cmd("file.new");
    h.run_jobs();
    let second = h.doc_id();
    assert_ne!(first, second);
    (h, first, second)
}

#[test]
fn 無題を_2_つ開くと無題_1_と無題_2_になる() {
    let (mut h, first, second) = two_untitled();
    assert_eq!(status_name(&mut h), "無題-2 (2/2)");
    assert_eq!(h.frame().title, "無題-2 (2/2) - evoeditor");
    assert_eq!(
        untitled_names(&h),
        vec![
            (first, Some("無題-1".to_owned())),
            (second, Some("無題-2".to_owned())),
        ]
    );
    // バッファ一覧も番号で区別できる
    h.cmd("buffer.list");
    let f = h.frame();
    let OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない: {:?}", f.overlays)
    };
    let rows: Vec<String> = items.iter().map(|i| cell_text(&i.label)).collect();
    assert!(rows.iter().any(|r| r.contains("無題-1")), "{rows:?}");
    assert!(rows.iter().any(|r| r.contains("無題-2")), "{rows:?}");
}

#[test]
fn 閉じても番号を再利用しない() {
    let (mut h, _, second) = two_untitled();
    h.cmd("buffer.close");
    h.run_jobs();
    assert!(h.editor.state().document(second).is_none());
    h.cmd("file.new");
    h.run_jobs();
    assert_eq!(status_name(&mut h), "無題-3 (2/2)", "閉じた 2 は使わない");
    // 最後の 1 件を閉じて作り直す無題も、新しい番号になる
    h.cmd("buffer.close");
    h.run_jobs();
    h.cmd("buffer.close");
    h.run_jobs();
    assert_eq!(h.editor.state().documents_len(), 1);
    assert_eq!(status_name(&mut h), "無題-4");
}

#[test]
fn 英語の_ui_でも番号が入る() {
    let mut h = Harness::new().with_locale(eve_config::Locale::En);
    h.cmd("file.new");
    h.run_jobs();
    assert_eq!(status_name(&mut h), "Untitled-2 (2/2)");
}

#[test]
fn 緊急退避とスワップの_untitled_name_も文書の番号を使う() {
    let (mut h, first, second) = two_untitled();
    h.text("b");
    h.advance_ms(0);
    let snap = h.editor.emergency_snapshot(second).expect("dirty な文書");
    assert_eq!(snap.untitled_name.as_deref(), Some("無題-2"));
    h.advance_ms(2000);
    h.run_jobs();
    assert_eq!(
        h.fs.swap_untitled_name_of(second),
        Some(Some("無題-2".to_owned()))
    );
    assert_eq!(h.fs.swap_untitled_name_of(first), None, "編集していない");
}

#[test]
fn スワップから復旧した無題は_適用した文書の番号のまま() {
    let mut h = Harness::new();
    // 前のセッションの無題-5 のスワップ
    h.fs.add_swap_entry(
        eve_core::SwapEntry {
            dir: eve_core::FsPath::from("/s/swap/untitled-9-4242.swp"),
            path: None,
            untitled_name: Some("無題-5".to_owned()),
            revision: 3,
            len: 5,
            pid: 4242,
            owner: eve_core::SwapOwner::Stale,
            sensitive: false,
            modified: Some(eve_core::FileTime {
                secs: 4_000_000_000,
                nanos: 0,
            }),
            readable: true,
        },
        Some(eve_core::SwapBody {
            text: "hello".to_owned(),
            encoding: eve_core::Encoding::Utf8,
            has_bom: false,
            default_eol: eve_text::LineEnding::Lf,
        }),
    );
    h.run_jobs();
    h.advance_ms(0);
    let doc = h.doc_id();
    let f = h.frame();
    let OverlayModel::Confirm { message, .. } = &f.overlays[0] else {
        panic!("提案が出ていない: {:?}", f.overlays)
    };
    assert!(
        cell_text(message).contains("無題-5"),
        "提案は前のセッションの名前で示す"
    );
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), "hello");
    assert_eq!(
        status_name(&mut h),
        "無題-1",
        "前のセッションの番号は引き継がない"
    );
    h.advance_ms(0);
    h.run_jobs();
    h.advance_ms(2000);
    h.run_jobs();
    assert_eq!(
        h.fs.swap_untitled_name_of(doc),
        Some(Some("無題-1".to_owned()))
    );
}
