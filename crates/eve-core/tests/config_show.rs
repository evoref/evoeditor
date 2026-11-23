//! 設定の値と由来の一覧 (`config.show`、docs/07 §3.5) と、条件付きセクション (§3.4、01 NFR-405) の core の側:
//! `config.reload` が起動時の条件の値で解くこと、由来の表示と `Enter`、`when` の `os` / `frontend` (§7.3)、
//! 条件付きセクションの中の診断の本文の条件。

use eve_config::{Conditions, EnvConditions, Frontend, Os};
use eve_core::{ConfigSources, CtxValue, FsPath, StartupConfig};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_view::OverlayModel;

const CONFIG: &str = "/evo/config/config.toml";
const KEYMAP: &str = "/evo/config/keymap.toml";

/// 条件の値と `--set` を置いた Harness (bin が起動時に決めて渡す形、07 §3.4 の COND-2)。
fn harness(os: Os, ssh: bool, sets: &[&str]) -> Harness {
    let mut h = Harness::new();
    h.editor.configure(StartupConfig {
        config_sources: Some(ConfigSources {
            files: vec![("config.toml".to_owned(), FsPath::new(CONFIG))],
            keymap: Some(FsPath::new(KEYMAP)),
            sets: sets.iter().map(|s| (*s).to_owned()).collect(),
            themes: None,
            languages: None,
            locales: None,
            conditions: Conditions {
                os: Some(os),
                frontend: Frontend::Tui,
                env: EnvConditions {
                    ssh,
                    tmux: false,
                    wsl: false,
                },
            },
            no_workspace_config: false,
            home: None,
        }),
        ..Default::default()
    });
    h
}

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

/// `config.show` を開いて `query` で絞り込んだ (本文, 由来)。
fn show(h: &mut Harness, query: &str) -> Vec<(String, String)> {
    h.cmd("config.show");
    assert_eq!(h.overlay(), "config_show");
    h.text(query);
    let (_, rows, origins) = list(h);
    rows.into_iter().zip(origins).collect()
}

fn cursor_line(h: &Harness) -> usize {
    let (start, _) = h.selection_range();
    h.doc_text()[..start].matches('\n').count()
}

const SHARED: &[u8] = b"[editor]\ntab_width = 3\n[os.linux.editor]\ntab_width = 2\n[os.windows.editor]\ntab_width = 8\n[env.ssh.editor]\nindent_width = 6\n";

// verifies: AC-NFR-405-1
#[test]
fn 読み直しは起動時の条件で解き_由来に条件を示す() {
    let mut h = harness(Os::Linux, false, &[]);
    h.fs.add_file(CONFIG, SHARED);
    reload(&mut h);
    assert_eq!(h.editor.state().config().tab_width, 2);
    assert_eq!(h.editor.state().config().indent_width, 4, "env.ssh は偽");
    assert_eq!(
        show(&mut h, "editor.tab_width"),
        [(
            "editor.tab_width = 2".to_owned(),
            "ユーザー: config.toml:4 [os.linux]".to_owned()
        )]
    );
    // 同じファイルを Windows と SSH の条件で読むと、そのセクションだけが効く
    let mut h = harness(Os::Windows, true, &[]);
    h.fs.add_file(CONFIG, SHARED);
    reload(&mut h);
    assert_eq!(h.editor.state().config().tab_width, 8);
    assert_eq!(h.editor.state().config().indent_width, 6);
    assert_eq!(
        show(&mut h, "indent_width"),
        [(
            "editor.indent_width = 6".to_owned(),
            "ユーザー: config.toml:8 [env.ssh]".to_owned()
        )]
    );
}

#[test]
fn 一覧は宣言の表の順で_既定と_set_の由来を示す() {
    let mut h = harness(Os::Windows, false, &["editor.indent_width=3"]);
    h.fs.add_file(CONFIG, b"[editor]\ntab_width = 3\n");
    reload(&mut h);
    h.cmd("config.show");
    let (title, rows, origins) = list(&mut h);
    assert_eq!(title, "設定 (無題-1・plain)");
    // 宣言の表 (07 §4.1) の順。表示窓に入る先頭の行
    assert_eq!(
        &rows[..4],
        [
            "editor.tab_width = 3",
            "editor.indent_unit = \"space\"",
            "editor.indent_width = 3",
            "file.default_encoding = \"utf-8\""
        ]
    );
    assert_eq!(
        &origins[..4],
        ["ユーザー: config.toml:2", "既定", "--set", "既定"]
    );
    // 由来でも絞り込める
    h.text("--set");
    let (_, rows, _) = list(&mut h);
    assert_eq!(rows, ["editor.indent_width = 3"]);
    // 由来にファイルの無い行の `Enter` は何もしない (一覧を開いたまま)
    h.keys("enter");
    assert_eq!(h.overlay(), "config_show");
}

#[test]
fn enter_は値を書いたファイルのその行を開く() {
    let mut h = harness(Os::Linux, false, &[]);
    h.fs.add_file(CONFIG, SHARED);
    reload(&mut h);
    show(&mut h, "editor.tab_width");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_path().as_deref(), Some(CONFIG));
    assert_eq!(
        cursor_line(&h),
        3,
        "4 行目 ([os.linux.editor] の tab_width) へ移る"
    );
}

// verifies: AC-NFR-405-2
#[test]
fn 偽の条件のセクションの誤りは本文に条件を添えて一覧に出す() {
    let mut h = harness(Os::Windows, false, &[]);
    h.fs.add_file(
        CONFIG,
        b"[os.linux.editor]\ntab_witdh = 2\n[os.win.editor]\ntab_width = 2\n",
    );
    reload(&mut h);
    assert_eq!(h.editor.state().config().tab_width, 4);
    h.cmd("config.show_diagnostics");
    let (_, rows, places) = list(&mut h);
    assert_eq!(places, ["config.toml:2", "config.toml:3"], "{rows:?}");
    assert!(
        rows[0].contains("editor.tab_witdh") && rows[0].contains(" [os.linux] / "),
        "{rows:?}"
    );
    assert!(
        rows[1].contains("os.win は条件付きセクションの名前ではありません / os.windows ではありませんか? 使える名前は"),
        "{rows:?}"
    );
}

// verifies: AC-NFR-405-2
#[test]
fn keymap_toml_の条件付きセクションは警告し_when_の_os_で割り当てられる() {
    for (os, want) in [(Os::Linux, "config_show"), (Os::Windows, "buffer_list")] {
        let mut h = harness(os, false, &[]);
        h.fs.add_file(
            KEYMAP,
            b"[os.linux.keys.editor]\n\"ctrl+j\" = \"config.show\"\n\n[keys.editor]\n\"ctrl+j\" = [\n  { command = \"config.show\", when = \"os == \\\"linux\\\"\" },\n  { command = \"buffer.list\", when = \"os != \\\"linux\\\" && frontend == \\\"tui\\\"\" },\n]\n",
        );
        reload(&mut h);
        h.cmd("config.show_diagnostics");
        let (_, rows, places) = list(&mut h);
        // `when` の `os` / `frontend` は知らない変数の警告にならない。条件付きセクションは専用の警告 1 件
        assert_eq!(places, ["keymap.toml:1"], "{rows:?}");
        assert!(
            rows[0].contains("keymap.toml では条件付きセクション (os) を使えません"),
            "{rows:?}"
        );
        h.keys("esc");
        h.keys("ctrl+j");
        assert_eq!(h.overlay(), want, "{os:?}");
    }
}

#[test]
fn when_の_os_と_frontend_は条件の値を返す() {
    for (os, name) in [
        (Os::Windows, "windows"),
        (Os::Macos, "macos"),
        (Os::Linux, "linux"),
    ] {
        let h = harness(os, false, &[]);
        assert_eq!(
            h.editor.state().context_value("os"),
            Some(CtxValue::Str(name.to_owned()))
        );
        assert_eq!(
            h.editor.state().context_value("frontend"),
            Some(CtxValue::Str("tui".to_owned()))
        );
    }
}
