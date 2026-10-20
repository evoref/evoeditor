//! `Editor::query` と `EditorEvent` (01 NFR-707 AC-4 / AC-5 / AC-6、FR-306 AC-1 の通知、NFR-203 AC-1 の通知)。
//! `Harness` (eve-testkit) で書く。

use eve_testkit::Harness;
use eve_view::{
    Answer, ByteRange, EditorEvent, LoadStateView, OverlayKind, Query, QueryError, StatusKind,
};

fn answer(h: &Harness, q: &Query) -> Answer {
    h.editor.query(q).expect("問い合わせに応答する")
}

fn view_of(h: &Harness) -> eve_view::ViewId {
    match answer(h, &Query::Windows) {
        Answer::Windows(ws) => ws[0].views[0],
        a => panic!("{a:?}"),
    }
}

// verifies: AC-NFR-707-4
#[test]
fn query_で文書一覧_本文_選択_ステータス_オーバーレイを読める() {
    let mut h = Harness::new().with_file("/w/a.txt", "héllo\r\nworld".as_bytes());
    h.open("/w/a.txt");
    h.run_jobs();
    h.keys("ctrl+end shift+home");

    let doc = h.doc_id();
    match answer(&h, &Query::Documents) {
        Answer::Documents(ds) => {
            assert_eq!(ds.len(), 1);
            assert_eq!(ds[0].doc, doc);
            assert_eq!(ds[0].path.as_deref(), Some("/w/a.txt"));
            assert!(!ds[0].dirty);
            assert_eq!(ds[0].load_state, LoadStateView::Ready);
            assert!(ds[0].read_only.is_empty());
            assert_eq!(ds[0].views.len(), 1);
        }
        a => panic!("{a:?}"),
    }
    match answer(&h, &Query::DocumentText { doc, range: None }) {
        Answer::DocumentText { text, range, .. } => {
            assert_eq!(text, "héllo\r\nworld", "改行は文書のまま");
            assert_eq!(range, ByteRange::new(0, 13));
        }
        a => panic!("{a:?}"),
    }
    match answer(
        &h,
        &Query::DocumentText {
            doc,
            range: Some(ByteRange::new(8, 13)),
        },
    ) {
        Answer::DocumentText { text, .. } => assert_eq!(text, "world"),
        a => panic!("{a:?}"),
    }
    match answer(&h, &Query::Selection { view: view_of(&h) }) {
        Answer::Selection {
            ranges, primary, ..
        } => {
            assert_eq!(primary, 0);
            assert_eq!(
                (ranges[0].anchor, ranges[0].head),
                (13, 8),
                "行頭までの選択"
            );
        }
        a => panic!("{a:?}"),
    }
    let window = h.editor.state().first_window().unwrap();
    match answer(&h, &Query::Status { window }) {
        Answer::Status(s) => {
            assert!(s.items.iter().any(|i| i.kind == StatusKind::Path), "{s:?}");
        }
        a => panic!("{a:?}"),
    }
    assert_eq!(
        answer(&h, &Query::Overlay { window }),
        Answer::Overlay(None)
    );
    h.text("x");
    h.keys("ctrl+q");
    match answer(&h, &Query::Overlay { window }) {
        Answer::Overlay(o) => assert!(o.is_some()),
        a => panic!("{a:?}"),
    }
}

#[test]
fn query_のエラー() {
    let mut h = Harness::new().with_file("/w/a.txt", b"abc");
    h.open("/w/a.txt");
    h.run_jobs();
    let doc = h.doc_id();
    assert_eq!(
        h.editor.query(&Query::DocumentText {
            doc: eve_view::DocumentId(999),
            range: None
        }),
        Err(QueryError::UnknownDocument(eve_view::DocumentId(999)))
    );
    assert_eq!(
        h.editor.query(&Query::DocumentText {
            doc,
            range: Some(ByteRange::new(0, 99))
        }),
        Err(QueryError::RangeOutOfBounds { len: 3 })
    );
    assert!(matches!(
        h.editor.query(&Query::Selection {
            view: eve_view::ViewId(999)
        }),
        Err(QueryError::UnknownView(_))
    ));
}

#[test]
fn utf8_の途中を指す範囲はエラー() {
    let mut h = Harness::new().with_file("/w/a.txt", "あ".as_bytes());
    h.open("/w/a.txt");
    h.run_jobs();
    assert_eq!(
        h.editor.query(&Query::DocumentText {
            doc: h.doc_id(),
            range: Some(ByteRange::new(0, 1))
        }),
        Err(QueryError::RangeOutOfBounds { len: 3 })
    );
}

// verifies: AC-NFR-707-6
#[test]
fn 同じ状態の同じ_query_は同じ応答() {
    let mut h = Harness::new().with_file("/w/a.txt", b"a\r\nb\nc\xFF");
    h.open("/w/a.txt");
    h.run_jobs();
    h.keys("ctrl+a");
    let doc = h.doc_id();
    let window = h.editor.state().first_window().unwrap();
    let view = view_of(&h);
    for q in [
        Query::Documents,
        Query::Windows,
        Query::DocumentText { doc, range: None },
        Query::Selection { view },
        Query::Capabilities,
        Query::Status { window },
        Query::Overlay { window },
    ] {
        let a = h.editor.query(&q);
        let b = h.editor.query(&q);
        assert_eq!(a, b, "{q:?}");
        assert!(a.is_ok(), "{q:?}: {a:?}");
    }
    // 本文は改行 (CRLF/LF 混在) と不正バイトの表現を含めて、同じバイト列
    let (Answer::DocumentText { text: t1, .. }, Answer::DocumentText { text: t2, .. }) = (
        answer(&h, &Query::DocumentText { doc, range: None }),
        answer(&h, &Query::DocumentText { doc, range: None }),
    ) else {
        panic!()
    };
    assert_eq!(t1.as_bytes(), t2.as_bytes());
    assert!(t1.starts_with("a\r\nb\nc"));
}

// verifies: AC-NFR-707-5
#[test]
fn 文書の変更とメッセージとオーバーレイの開閉が_editor_event_になる() {
    let mut h = Harness::new().with_file("/w/a.txt", b"abc");
    h.open("/w/a.txt");
    h.run_jobs();
    assert!(
        h.events()
            .iter()
            .any(|e| matches!(e, EditorEvent::DocumentOpened { .. }))
    );
    h.clear_log();

    h.text("x");
    assert!(
        h.events()
            .iter()
            .any(|e| matches!(e, EditorEvent::DocumentChanged { .. }))
    );

    // 確認ダイアログが開いて、閉じる
    h.clear_log();
    h.keys("ctrl+q");
    assert!(h.events().iter().any(|e| matches!(
        e,
        EditorEvent::OverlayChanged {
            overlay: Some(OverlayKind::Confirm),
            ..
        }
    )));
    h.clear_log();
    h.keys("escape");
    assert!(
        h.events()
            .iter()
            .any(|e| matches!(e, EditorEvent::OverlayChanged { overlay: None, .. }))
    );

    // 入力プロンプト
    h.clear_log();
    h.keys("ctrl+g");
    assert!(h.events().iter().any(|e| matches!(
        e,
        EditorEvent::OverlayChanged {
            overlay: Some(OverlayKind::Prompt),
            ..
        }
    )));
    h.keys("escape");

    // メッセージ (保存)
    h.clear_log();
    h.cmd("file.save");
    h.run_jobs();
    assert!(
        h.events()
            .iter()
            .any(|e| matches!(e, EditorEvent::DocumentSaved { .. }))
    );
    assert!(!h.messages().is_empty());

    // v0.1 で発火するのは Document / Message / Overlay の 3 系だけ
    for e in h.events() {
        assert!(
            matches!(
                e,
                EditorEvent::DocumentOpened { .. }
                    | EditorEvent::DocumentChanged { .. }
                    | EditorEvent::DocumentSaved { .. }
                    | EditorEvent::DocumentClosed { .. }
                    | EditorEvent::MessageShown { .. }
                    | EditorEvent::OverlayChanged { .. }
            ),
            "{e:?}"
        );
    }
}

// verifies: AC-FR-306-1
#[test]
fn 範囲外の行番号は最終行に丸めて通知する() {
    let mut h = Harness::new().with_file("/w/a.txt", b"a\nb\nc");
    h.open("/w/a.txt");
    h.run_jobs();
    h.clear_log();
    h.cmd_with(
        "cursor.goto_line",
        eve_core::Args::new().with_int("line", 99),
    );
    assert_eq!(h.messages(), vec!["info.goto_line.clamped".to_owned()]);
    let f = h.frame_text();
    assert!(
        f.contains("行 99 はありません。最後の行 (3) へ移動しました"),
        "{f}"
    );
    assert!(f.contains("行 3,"), "最終行に移る: {f}");
    // 範囲内なら通知しない
    h.clear_log();
    h.cmd_with(
        "cursor.goto_line",
        eve_core::Args::new().with_int("line", 2),
    );
    assert!(h.messages().is_empty());
}

// verifies: AC-NFR-203-1
#[test]
fn 履歴が上限に達したら古い履歴を破棄して_文書ごとに_1_回通知する() {
    let mut h = Harness::new();
    // 1 行が長くなると 1 編集ごとの表示列の計算が重くなるので、改行で区切る
    for i in 0..10_050 {
        let _ = i;
        h.text(
            "a
",
        );
        h.advance_ms(1500); // 結合の時間窓を越えて、1 回の入力ごとに 1 ステップにする
    }
    let trimmed = h
        .messages()
        .into_iter()
        .filter(|k| k == "info.undo.history_trimmed")
        .count();
    assert_eq!(trimmed, 1, "上限に達したら 1 回だけ通知する");
    // 直前の 1 操作は Undo できる
    let before = h.doc_text();
    h.keys("ctrl+z");
    assert_eq!(h.doc_text().len(), before.len() - 2);
}

#[test]
fn ドル記号と0以下の行番号と巨大な行番号() {
    let mut h = Harness::new().with_file(
        "/w/a.txt", b"a
b
c",
    );
    h.open("/w/a.txt");
    h.run_jobs();
    h.clear_log();
    h.cmd_with(
        "cursor.goto_line",
        eve_core::Args::new().with_str("line", "$"),
    );
    assert!(h.messages().is_empty(), "最終行の指定は範囲外ではない");
    h.cmd_with(
        "cursor.goto_line",
        eve_core::Args::new().with_int("line", i64::MAX),
    );
    assert_eq!(h.messages(), vec!["info.goto_line.clamped".to_owned()]);
    h.clear_log();
    h.cmd_with(
        "cursor.goto_line",
        eve_core::Args::new().with_int("line", 0),
    );
    assert!(h.frame_text().contains("行 1,"));
}

#[test]
fn プロンプトから範囲外の行番号を入れると通知する() {
    let mut h = Harness::new().with_file(
        "/w/a.txt", b"a
b
c",
    );
    h.open("/w/a.txt");
    h.run_jobs();
    h.clear_log();
    h.keys("ctrl+g");
    h.text("50");
    h.keys("enter");
    assert_eq!(h.messages(), vec!["info.goto_line.clamped".to_owned()]);
}

#[test]
fn undo_では履歴破棄の通知を出さない() {
    let mut h = Harness::new();
    for _ in 0..10_010 {
        h.text(
            "a
",
        );
        h.advance_ms(1500);
    }
    h.clear_log();
    for _ in 0..5 {
        h.keys("ctrl+z");
    }
    assert!(
        !h.messages()
            .contains(&"info.undo.history_trimmed".to_owned())
    );
}
