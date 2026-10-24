//! 色数・拡張キーを無効にした Capabilities での Must 操作 (01 NFR-403 AC-1)。
//! 到達性検査 (FR-606) は `default_keymap` のテスト。ここは開く・編集・保存・検索・取り消し・パレットのシナリオ。

use eve_testkit::Harness;
use eve_view::{Capabilities, ColorDepth};

// verifies: AC-NFR-403-1
#[test]
fn 色数と拡張キーを無効にした能力でも_must_の操作が通る() {
    for depth in [ColorDepth::Mono, ColorDepth::Ansi16] {
        let caps = Capabilities {
            color_depth: depth,
            enhanced_keys: false,
            mouse: false,
            ascii_only: true,
            ..Capabilities::default()
        };
        let mut h = Harness::new()
            .with_caps(caps)
            .with_size(60, 12)
            .with_file("/a.txt", b"hello fox\n");
        h.open("/a.txt");
        h.run_jobs();
        h.keys("ctrl+home");
        h.text("X");
        h.keys("ctrl+s");
        h.run_jobs();
        assert!(!h.is_dirty(), "{depth:?}: 保存できる");
        assert_eq!(h.fs_read("/a.txt").as_deref(), Some(&b"Xhello fox\n"[..]));
        // 検索
        h.keys("ctrl+f");
        h.text("fox");
        h.run_jobs();
        let bar = h.frame().search.expect("検索バーが開く");
        let count: String = bar
            .count_text
            .expect("件数が出る")
            .spans
            .iter()
            .map(|s| s.text.to_string())
            .collect();
        assert_eq!(count, "1/1", "{depth:?}");
        h.keys("esc");
        // 取り消し・やり直し
        h.keys("ctrl+z");
        assert_eq!(h.doc_text(), "hello fox\n", "{depth:?}");
        h.keys("ctrl+y");
        assert_eq!(h.doc_text(), "Xhello fox\n", "{depth:?}");
        // パレットを開いて閉じる
        h.keys("ctrl+p");
        assert_eq!(h.overlay(), "palette", "{depth:?}");
        h.keys("esc");
        assert_eq!(h.overlay(), "", "{depth:?}");
        // 画面が出る (色の無い段でも本文と状態が見える)
        let t = h.frame_text();
        assert!(
            t.contains("Xhello fox") && t.contains("UTF-8"),
            "{depth:?}\n{t}"
        );
    }
}

// verifies: AC-FR-511-1
#[test]
fn パレット_確認ダイアログ_入力プロンプトは_上下_enter_esc_で同じように動く() {
    let mut h = Harness::new()
        .with_size(60, 12)
        .with_file("/a.txt", b"one\ntwo\nthree\n");
    h.open("/a.txt");
    h.run_jobs();

    // 入力プロンプト (行へ移動): Enter で確定、Esc で取り消し
    h.keys("ctrl+g");
    assert_eq!(h.overlay(), "prompt");
    h.keys("esc");
    assert_eq!(h.overlay(), "", "Esc で閉じる");
    h.keys("ctrl+g");
    h.text("3");
    h.keys("enter");
    assert_eq!(h.overlay(), "", "Enter で確定して閉じる");

    // パレット: 下で選択が動き、Esc で閉じる。Enter で選んだコマンドを実行する
    h.keys("ctrl+p");
    assert_eq!(h.overlay(), "palette");
    let first = selected(&mut h);
    h.keys("down");
    assert_ne!(selected(&mut h), first, "下で選択が動く");
    h.keys("up");
    assert_eq!(selected(&mut h), first, "上で戻る");
    h.keys("esc");
    assert_eq!(h.overlay(), "");
    h.keys("ctrl+p");
    h.text("select.all");
    h.keys("enter");
    assert_eq!(h.overlay(), "", "Enter で実行して閉じる");

    // 確認ダイアログ (未保存で終了): 下 / 上でボタンが動き、Esc でキャンセル、Enter で実行
    h.keys("ctrl+end");
    h.text("x");
    h.keys("ctrl+q");
    assert_eq!(h.overlay(), "dialog");
    let focus = selected(&mut h);
    h.keys("down");
    assert_ne!(selected(&mut h), focus, "下でフォーカスが動く");
    h.keys("up");
    assert_eq!(selected(&mut h), focus);
    h.keys("esc");
    assert_eq!(h.overlay(), "", "Esc でキャンセルして編集に戻る");
    assert!(h.quit_code().is_none());
}

/// 開いているオーバーレイの選択中の項目 (一覧の選択行、確認のフォーカス中のボタン)。
fn selected(h: &mut Harness) -> String {
    let f = h.frame();
    match &f.overlays[0] {
        eve_view::OverlayModel::List { selected, .. } => format!("list:{selected}"),
        eve_view::OverlayModel::Confirm { focused, .. } => format!("confirm:{focused:?}"),
        other => format!("{other:?}"),
    }
}
