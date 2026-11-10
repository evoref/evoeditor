//! 設定の診断の一覧 (`config.show_diagnostics`、docs/07 §4.3、04 §8.5a): 最後に読んだときの診断を一覧
//! (意味的モデル `List`) で示し、`Enter` で該当のファイルの該当の行を開く。

use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_view::OverlayModel;

const CONFIG: &str = "/evo/config/config.toml";
const KEYMAP: &str = "/evo/config/keymap.toml";

fn reload(h: &mut Harness) {
    h.cmd("config.reload");
    h.run_jobs();
}

/// 一覧の (見出し, 各行の本文, 各行の右の列)。
fn list(h: &mut Harness) -> (String, Vec<String>, Vec<String>) {
    let f = h.frame();
    let OverlayModel::List { title, items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    (
        cell_text(title),
        items.iter().map(|i| cell_text(&i.label)).collect(),
        items
            .iter()
            .map(|i| i.detail.as_ref().map(cell_text).unwrap_or_default())
            .collect(),
    )
}

/// カーソルの行 (0 始まり)。
fn cursor_line(h: &Harness) -> usize {
    let (start, _) = h.selection_range();
    h.doc_text()[..start].matches('\n').count()
}

#[test]
fn 診断が無ければ一覧を開かずにそう示す() {
    let mut h = Harness::new();
    h.clear_log();
    h.cmd("config.show_diagnostics");
    assert_eq!(h.overlay(), "");
    assert!(
        h.messages()
            .iter()
            .any(|m| m == "info.config.no_diagnostics"),
        "{:?}",
        h.messages()
    );
}

#[test]
fn 読み直した設定の診断を一覧にし_enter_で該当の行を開く() {
    let mut h = Harness::new();
    h.fs.add_file(CONFIG, b"[editor]\nindent_width = 2\ntab_width = 99\n");
    reload(&mut h);
    h.cmd("config.show_diagnostics");
    assert_eq!(h.overlay(), "config_diagnostics");
    let (title, rows, places) = list(&mut h);
    assert_eq!(title, "設定の診断");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        rows[0].starts_with("警告: ") && rows[0].contains("tab_width"),
        "{rows:?}"
    );
    assert!(rows[0].contains(" / "), "対処も示す (NFR-602): {rows:?}");
    assert_eq!(places, ["config.toml:3"]);

    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_path().as_deref(), Some(CONFIG));
    assert_eq!(cursor_line(&h), 2, "3 行目へ移る");
}

#[test]
fn 開いているファイルならそのバッファへ切り替えて移る() {
    let mut h = Harness::new();
    h.fs.add_file(CONFIG, b"[editor]\n\n\ntab_width = \"4\"\n");
    reload(&mut h);
    h.open(CONFIG);
    h.run_jobs();
    let cfg = h.doc_id();
    h.cmd("file.new");
    assert_ne!(h.doc_id(), cfg);
    h.cmd("config.show_diagnostics");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.doc_id(), cfg, "同じファイルを二重に開かない");
    assert_eq!(cursor_line(&h), 3);
}

#[test]
fn キーマップと設定の診断を並べ_入力で絞り込める() {
    let mut h = Harness::new();
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 99\n");
    h.fs.add_file(KEYMAP, b"[keys.editor]\n\"ctrl+j\" = \"file.svae\"\n");
    reload(&mut h);
    h.cmd("config.show_diagnostics");
    let (_, rows, places) = list(&mut h);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(places, ["config.toml:2", "keymap.toml:2"]);
    h.text("svae");
    let (_, rows, places) = list(&mut h);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(places, ["keymap.toml:2"]);
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.doc_path().as_deref(), Some(KEYMAP));
    assert_eq!(cursor_line(&h), 1);
}

#[test]
fn 未対応の項目の情報診断も一覧に出す() {
    let mut h = Harness::new();
    h.fs.add_file(CONFIG, b"[ai]\nenabled = true\n");
    reload(&mut h);
    h.cmd("config.show_diagnostics");
    let (_, rows, _) = list(&mut h);
    assert!(
        rows.iter()
            .any(|r| r.starts_with("情報: ") && r.contains("未対応")),
        "{rows:?}"
    );
}

#[test]
fn 直した後に読み直すと診断は消える() {
    let mut h = Harness::new();
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 99\n");
    reload(&mut h);
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 8\n");
    reload(&mut h);
    h.clear_log();
    h.cmd("config.show_diagnostics");
    assert_eq!(h.overlay(), "");
    assert!(
        h.messages()
            .iter()
            .any(|m| m == "info.config.no_diagnostics")
    );
}

#[test]
fn 起動時に渡された診断も一覧に出す() {
    let mut h = Harness::new();
    let l = eve_config::load_str("[editor]\ntab_width = 0\n");
    h.editor
        .state_mut()
        .report_config("config.toml", &l.diagnostics);
    h.cmd("config.show_diagnostics");
    let (_, rows, places) = list(&mut h);
    assert_eq!(rows.len(), 1);
    assert_eq!(places, ["config.toml:2"]);
}

#[test]
fn esc_で閉じると何も開かない() {
    let mut h = Harness::new();
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 99\n");
    reload(&mut h);
    let before = h.doc_id();
    h.cmd("config.show_diagnostics");
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_id(), before);
}

#[test]
fn 英語の画面でも文言のキーのまま出さない() {
    let mut h = Harness::new().with_locale(eve_config::Locale::En);
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 99\n");
    reload(&mut h);
    h.cmd("config.show_diagnostics");
    let (title, rows, _) = list(&mut h);
    assert_eq!(title, "Settings Diagnostics");
    assert!(rows[0].starts_with("Warning: "), "{rows:?}");
    assert!(!rows[0].contains("msg.config"), "{rows:?}");
}

#[test]
fn ファイルの無い診断_set_は開かずにそう示す() {
    let mut h = Harness::new();
    let l = eve_config::load_str("[editor]\ntab_width = 0\n");
    h.editor.state_mut().report_config("--set", &l.diagnostics);
    let before = h.doc_id();
    h.cmd("config.show_diagnostics");
    h.clear_log();
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_id(), before);
    assert!(
        h.messages()
            .iter()
            .any(|m| m == "info.config.diagnostic_no_file"),
        "{:?}",
        h.messages()
    );
}
