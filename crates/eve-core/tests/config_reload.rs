//! 設定の再読込 (`config.reload`、docs/07 §6、01 FR-706): 読み直して差し替え、エラーなら前の設定を保ち、
//! 変わった項目を要約する。

use eve_testkit::Harness;
use eve_view::Effect;

const CONFIG: &str = "/evo/config/config.toml";
const KEYMAP: &str = "/evo/config/keymap.toml";

fn reload(h: &mut Harness) {
    h.clear_log();
    h.cmd("config.reload");
    h.run_jobs();
}

fn message_text(h: &mut Harness) -> String {
    let f = h.frame();
    f.messages
        .current
        .map(|m| m.text.spans.iter().map(|s| s.text.as_str()).collect())
        .unwrap_or_default()
}

#[test]
fn 読み直すと差し替え_変わった項目を要約する() {
    let mut h = Harness::new();
    assert_eq!(h.editor.state().config().tab_width, 4);
    h.fs.add_file(
        CONFIG,
        b"[editor]\ntab_width = 8\n[ui]\nmouse_wheel_lines = 5\n",
    );
    reload(&mut h);
    let c = h.editor.state().config();
    assert_eq!((c.tab_width, c.mouse_wheel_lines), (8, 5));
    assert!(h.messages().iter().any(|m| m == "info.config.reloaded"));
    let text = message_text(&mut h);
    assert!(
        text.contains("2 項目")
            && text.contains("editor.tab_width")
            && text.contains("ui.mouse_wheel_lines"),
        "{text}"
    );
}

#[test]
fn ファイルが無く_変わった項目も無ければそう知らせる() {
    let mut h = Harness::new();
    reload(&mut h);
    assert!(
        h.messages()
            .iter()
            .any(|m| m == "info.config.reload_unchanged")
    );
}

#[test]
fn 構文エラーのファイルは前の設定のままにする() {
    let mut h = Harness::new();
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 8\n");
    reload(&mut h);
    assert_eq!(h.editor.state().config().tab_width, 8);
    h.fs.add_file(CONFIG, b"[editor\ntab_width = 2\n");
    reload(&mut h);
    assert_eq!(h.editor.state().config().tab_width, 8, "前の設定のまま");
    assert!(
        h.messages().iter().any(|m| m == "msg.config.reload_kept"),
        "{:?}",
        h.messages()
    );
}

#[test]
fn 警告の項目は起動時と同じく既定値で差し替える() {
    let mut h = Harness::new();
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 99\nindent_width = 2\n");
    reload(&mut h);
    let c = h.editor.state().config();
    assert_eq!((c.tab_width, c.indent_width), (4, 2));
}

#[test]
fn キーマップも読み直し_変わったら項目に数える() {
    let mut h = Harness::new();
    h.fs.add_file(KEYMAP, b"[keys.editor]\n\"f5\" = \"file.save\"\n");
    reload(&mut h);
    assert!(
        message_text(&mut h).contains("keymap.toml"),
        "{}",
        message_text(&mut h)
    );
    // 割当が効く
    h.text("x");
    h.clear_log();
    h.keys("f5");
    assert_eq!(h.overlay(), "prompt", "無題なので F5 (保存) で名前を尋ねる");
    h.keys("esc");
    // 変えずに読み直すと、変わった項目は無い
    reload(&mut h);
    assert!(
        h.messages()
            .iter()
            .any(|m| m == "info.config.reload_unchanged"),
        "{:?}",
        h.messages()
    );
}

#[test]
fn 再起動が要る項目は別に知らせ_マウスは端末に伝える() {
    let mut h = Harness::new();
    h.fs.add_file(
        CONFIG,
        b"[theme]\ncolor_mode = \"256\"\n[ui]\nmouse = false\n",
    );
    reload(&mut h);
    assert!(
        h.messages()
            .iter()
            .any(|m| m == "info.config.reload_restart"),
        "{:?}",
        h.messages()
    );
    assert!(
        h.effects()
            .iter()
            .any(|e| matches!(e, Effect::SetMouseCapture { enabled: false }))
    );
}

#[test]
fn 折り返しは値が変わったときだけ写す() {
    let mut h = Harness::new();
    h.cmd("view.toggle_wrap");
    let wrapped = |h: &Harness| {
        let w = h.editor.state().first_window().unwrap();
        let v = h.editor.state().window_view(w).unwrap();
        h.editor.state().view(v).unwrap().wrap.is_on()
    };
    assert!(wrapped(&h));
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 2\n");
    reload(&mut h);
    assert!(
        wrapped(&h),
        "折り返しの設定は変わっていないので、切り替えた状態を戻さない"
    );
}
