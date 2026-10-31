//! ヒントバーの表示の切り替え (01 FR-504 AC-2、04 §8.5): 設定 `ui.hint_bar` と、実行中の `ui.toggle_hint_bar`。
//! 隠した行を本文に回すのは frontend (Frame の `hint_bar` を見る、04 §3.5)。

use eve_testkit::Harness;

// verifies: FR-504
#[test]
fn ui_toggle_hint_bar_で隠して戻せ_そのたびに知らせる() {
    let mut h = Harness::new();
    assert!(h.frame().hint_bar, "既定は表示");
    h.clear_log();
    h.cmd("ui.toggle_hint_bar");
    assert!(!h.frame().hint_bar);
    assert!(h.messages().contains(&"info.hint_bar.hidden".to_owned()));
    h.cmd("ui.toggle_hint_bar");
    assert!(h.frame().hint_bar);
    assert!(h.messages().contains(&"info.hint_bar.shown".to_owned()));
}

// verifies: FR-504
#[test]
fn 設定_ui_hint_bar_が偽なら隠して始まる() {
    let l = eve_config::load_str("[ui]\nhint_bar = false\n");
    assert!(
        l.diagnostics.is_empty(),
        "公開した設定キー: {:?}",
        l.diagnostics
    );
    assert!(!l.config.hint_bar);
    let mut h = Harness::new();
    h.editor.state_mut().set_config(l.config);
    assert!(!h.frame().hint_bar);
    // パレットから選べる (コマンドの一覧にある)
    assert!(
        h.editor
            .registry()
            .iter()
            .any(|s| s.id.as_str() == "ui.toggle_hint_bar")
    );
}
