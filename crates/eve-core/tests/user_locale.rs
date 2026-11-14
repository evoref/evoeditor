//! ユーザーの言語ファイルを重ねた文言 (docs/07 §17.4、01 NFR-603 AC-9、09 §3.4 の (2))。
//!
//! 重ねた表は `Harness::with_catalog` で渡す (ファイルを介さない。表は bin の起動時の読込と同じ規則の
//! `eve_testkit::overlay_catalog` で作る)。これらのシナリオだけは、上書きを確かめるために `MessageKey` ではなく解決した
//! 文言を見る (09 §3.4 の規則の例外)。

use eve_config::Locale;
use eve_core::{HostReport, LocaleFiles};
use eve_testkit::helpers::cell_text;
use eve_testkit::{Harness, overlay_catalog};
use eve_view::OverlayModel;

const CONFIG: &str = "/evo/config/config.toml";
const LOCALES: &str = "/evo/config/locales";
const LOCALE_JA: &str = "/evo/config/locales/ja.toml";

/// 表示中のメッセージの本文と対処。
fn message(h: &mut Harness) -> (String, String) {
    let f = h.frame();
    let m = f.messages.current.expect("メッセージ");
    (
        cell_text(&m.text),
        m.hint.as_ref().map(cell_text).unwrap_or_default(),
    )
}

fn reload(h: &mut Harness) {
    h.clear_log();
    h.cmd("config.reload");
    h.run_jobs();
}

// verifies: AC-NFR-603-9
#[test]
fn ja_の_1_つの文言の上書きが画面に出て_書いていないキーは同梱の_ja_になる() {
    let (locale, cat, diags) = overlay_catalog("ja", "[status]\nuntitled = \"名無し-{n}\"\n");
    assert!(diags.is_empty(), "{diags:?}");
    let mut h = Harness::new().with_catalog(locale, cat);
    // (a) 上書きした文言
    let t = h.frame_text();
    assert!(t.contains("名無し-1"), "{t}");
    assert!(!t.contains("無題-1"), "{t}");
    // (b) 書いていないキーは同梱の ja の文言 (キー名を出さない)
    h.cmd("config.show_diagnostics");
    let (text, _) = message(&mut h);
    assert_eq!(text, "設定の診断はありません");
    assert!(!h.frame_text().contains("info.config"));
}

// verifies: AC-NFR-603-9
#[test]
fn 追加言語の表で無いキーは_en_の文言になる() {
    let (locale, cat, diags) = overlay_catalog(
        "fr",
        "[meta]\nlanguage = \"fr-FR\"\n[status]\nuntitled = \"Sans titre-{n}\"\n",
    );
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(locale.code(), "fr");
    let mut h = Harness::new().with_catalog(locale, cat);
    assert!(h.frame_text().contains("Sans titre-1"));
    // (c) 書いていないキーは補う言語 en の文言
    h.cmd("config.show_diagnostics");
    let (text, _) = message(&mut h);
    assert_eq!(text, "There are no settings diagnostics");
}

// verifies: AC-NFR-603-9
// verifies: NFR-503
#[test]
fn 使わなかった文言は同梱のままで_診断は同梱の文言で出る() {
    // 要約 (`msg.config.problem`) を上書きしても、言語ファイルの診断には同梱の文言を使う (07 §17.4)。
    // `status.untitled` は ESC を含むので使わない (検査 4)
    let src = "[msg.config.problem]\ntext = \"上書き ({file}〔:{line}〕): {detail}\"\n\
               [status]\nuntitled = \"\\u001B[31m無題-{n}\"\n";
    let (locale, cat, diags) = overlay_catalog("ja", src);
    assert_eq!(diags.len(), 1, "{diags:?}");
    let mut h = Harness::new().with_catalog(locale, cat);
    let t = h.frame_text();
    assert!(t.contains("無題-1"), "使わなかった文言は同梱のまま: {t}");
    h.editor.report(HostReport::LocaleDiagnostics {
        file: "locales/ja.toml",
        diagnostics: &diags,
    });
    let (text, _) = message(&mut h);
    assert_eq!(
        text,
        "設定の問題 (locales/ja.toml:4): status.untitled の文言に使えない文字があります (制御文字 <U+001B>)"
    );
    // 設定ファイルの診断の要約は、重ねた表 (上書きした文言) で出す
    let l = eve_config::load_str("[editor]\ntab_width = \"4\"\n");
    h.editor.report(HostReport::ConfigDiagnostics {
        file: "config.toml",
        diagnostics: &l.diagnostics,
    });
    let (text, _) = message(&mut h);
    assert!(text.starts_with("上書き (config.toml:2): "), "{text}");
    // 一覧の行も、言語ファイルの診断は同梱の文言
    h.cmd("config.show_diagnostics");
    let f = h.frame();
    let OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    let rows: Vec<String> = items.iter().map(|i| cell_text(&i.label)).collect();
    assert!(
        rows.iter()
            .any(|r| r.contains("status.untitled の文言に使えない文字があります")),
        "{rows:?}"
    );
}

// verifies: AC-NFR-603-9
#[test]
fn パレットの照合は上書きした_title_に依らず英語の_title_で当たる() {
    let (locale, cat, diags) = overlay_catalog(
        "ja",
        "[cmd.file.set_line_ending]\ntitle = \"改行を変える\"\n",
    );
    assert!(diags.is_empty(), "{diags:?}");
    let mut h = Harness::new().with_catalog(locale, cat);
    h.keys("ctrl+p");
    // (e) 英語の title (`Convert Line Endings`) で当たり、表示は上書きした title
    h.text("convert");
    let f = h.frame();
    let OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("パレットが出ていない")
    };
    let labels: Vec<String> = items.iter().map(|i| cell_text(&i.label)).collect();
    assert!(
        labels.iter().any(|l| l.contains("改行を変える")),
        "{labels:?}"
    );
    // 上書きした title でも探せる
    h.keys("ctrl+a");
    h.text("改行を変");
    let f = h.frame();
    let OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("パレットが出ていない")
    };
    assert!(
        items
            .iter()
            .any(|i| i.detail.as_ref().map(cell_text).as_deref() == Some("file.set_line_ending"))
    );
}

// verifies: AC-NFR-603-9
#[test]
fn 言語ファイルが変わっていれば読み直しで再起動を促す() {
    let mut h = Harness::new();
    h.editor.configure(eve_core::StartupConfig {
        locale_files: Some(LocaleFiles {
            available: Vec::new(),
            dir: Some(eve_core::FsPath::new(LOCALES)),
            chosen: Locale::Ja,
            stamp: None,
        }),
        ..Default::default()
    });
    // 変わっていない (無いまま)
    reload(&mut h);
    assert!(
        !h.messages()
            .iter()
            .any(|m| m == "info.config.locale_restart"),
        "{:?}",
        h.messages()
    );
    // 現れた: 要約の後に再起動を促す (中身は読まず、言語も変えない)
    h.fs.add_file(LOCALE_JA, b"[status]\nuntitled = \"x-{n}\"\n");
    reload(&mut h);
    let ms = h.messages();
    assert_eq!(
        ms.last().map(String::as_str),
        Some("info.config.locale_restart"),
        "{ms:?}"
    );
    assert!(!h.frame_text().contains("x-1"), "読み直さない");
    let (text, _) = message(&mut h);
    assert_eq!(text, "言語ファイルの変更は、再起動すると有効になります");
}

// verifies: AC-NFR-603-9
#[test]
fn 起動中に置いた言語を_ui_locale_に書いて読み直すと見つからないとせず再起動を促す() {
    let mut h = Harness::new();
    h.editor.configure(eve_core::StartupConfig {
        locale_files: Some(LocaleFiles {
            available: Vec::new(),
            dir: Some(eve_core::FsPath::new(LOCALES)),
            chosen: Locale::Ja,
            stamp: None,
        }),
        ..Default::default()
    });
    h.fs.add_file("/evo/config/locales/fr.toml", b"");
    h.fs.add_file(CONFIG, b"[ui]\nlocale = \"fr\"\n");
    reload(&mut h);
    let ms = h.messages();
    assert!(
        !ms.iter().any(|m| m == "msg.config.problem"),
        "fr を見つからないとしない: {ms:?}"
    );
    assert!(
        ms.iter().any(|m| m == "info.config.reload_restart"),
        "{ms:?}"
    );
    assert_eq!(
        ms.last().map(String::as_str),
        Some("info.config.locale_restart"),
        "{ms:?}"
    );
}

// verifies: AC-NFR-603-9
#[test]
fn 読み直しでも_ui_locale_を候補と照らし_言語ファイルの診断は残す() {
    let fr = Locale::from_tag("fr").unwrap();
    let mut h = Harness::new();
    h.fs.add_file("/evo/config/locales/fr.toml", b"");
    h.editor.configure(eve_core::StartupConfig {
        locale_files: Some(LocaleFiles {
            available: vec![fr],
            dir: Some(eve_core::FsPath::new(LOCALES)),
            chosen: Locale::Ja,
            stamp: None,
        }),
        ..Default::default()
    });
    // 起動時の言語ファイルの診断
    let (_, _, diags) = overlay_catalog("ja", "[no.such]\nkey = \"x\"\n");
    h.editor.report(HostReport::LocaleDiagnostics {
        file: "locales/ja.toml",
        diagnostics: &diags,
    });
    h.fs.add_file(CONFIG, b"[ui]\nlocale = \"de\"\n");
    reload(&mut h);
    assert!(h.messages().iter().any(|m| m == "msg.config.problem"));
    h.cmd("config.show_diagnostics");
    let f = h.frame();
    let OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    let rows: Vec<String> = items.iter().map(|i| cell_text(&i.label)).collect();
    assert!(
        rows.iter()
            .any(|r| r.contains("言語 de の言語ファイルがありません")
                && r.contains(&format!("(探した場所: {LOCALES})"))),
        "{rows:?}"
    );
    assert!(
        rows.iter().any(|r| r.contains("no.such.key")),
        "言語ファイルの診断は起動時のものを残す: {rows:?}"
    );
}

// verifies: AC-NFR-603-9
#[test]
fn 複数の問題の要約を上書きしても言語ファイルの診断は同梱の文言で出る() {
    let src = "[msg.config.problems]\ntext = \"上書き {count} {detail}\"\n\
               [no.such]\nkey = \"x\"\n";
    let (locale, cat, diags) = overlay_catalog("ja", src);
    assert_eq!(diags.len(), 1, "{diags:?}");
    let mut h = Harness::new().with_catalog(locale, cat);
    let (_, _, more) = overlay_catalog("ja", "[a.b]\nc = \"x\"\n[d.e]\nf = \"y\"\n");
    h.editor.report(HostReport::LocaleDiagnostics {
        file: "locales/ja.toml",
        diagnostics: &more,
    });
    let (text, hint) = message(&mut h);
    assert!(text.starts_with("設定に 2 件の問題があります。"), "{text}");
    assert_eq!(hint, "一覧は「設定の診断を表示」で見られます");
    // 設定ファイルの要約は、重ねた表 (上書きした文言)
    let l = eve_config::load_str("[editor]\ntab_width = 0\nindent_width = 99\n");
    h.editor.report(HostReport::ConfigDiagnostics {
        file: "config.toml",
        diagnostics: &l.diagnostics,
    });
    let (text, _) = message(&mut h);
    assert!(text.starts_with("上書き 2 "), "{text}");
}

// verifies: NFR-503
#[test]
fn 設定ファイルのキーと値の制御文字はメッセージ行と一覧に出ない() {
    let mut h = Harness::new();
    let corpus = [
        "\u{1b}[31m",
        "\u{1b}]0;title\u{7}",
        "\u{9b}2J",
        "\u{1b}P+q\u{1b}\\",
        "\u{202e}",
    ];
    for (i, c) in corpus.iter().enumerate() {
        let toml =
            format!("[editor]\n\"tab{c}_width\" = 1\n[file]\ndefault_encoding = \"x{c}y\"\n");
        let l = eve_config::load_str(&toml);
        assert!(!l.diagnostics.is_empty(), "{i}");
        h.editor.report(HostReport::ConfigDiagnostics {
            file: "config.toml",
            diagnostics: &l.diagnostics,
        });
        let (text, hint) = message(&mut h);
        for s in [&text, &hint] {
            assert!(
                !s.chars().any(|c| c.is_control() || c == '\u{202e}'),
                "{i}: {s:?}"
            );
        }
    }
    h.cmd("config.show_diagnostics");
    let f = h.frame();
    let OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    for it in items {
        let row = cell_text(&it.label);
        assert!(
            !row.chars().any(|c| c.is_control() || c == '\u{202e}'),
            "{row:?}"
        );
        assert!(row.contains("<U+"), "{row}");
    }
}
