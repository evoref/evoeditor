//! ワークスペース層 (`<root>/.eve/config.toml`・`.eve/keymap.toml`) の読込と重ね方 (docs/07 §12.5 の WSC-1〜WSC-7、
//! §12.3 の分類、§12.1 の所有者の検査、§3.6 の LAYER-1 の (6)(7)、01 FR-705 AC-1〜AC-3・NFR-502 AC-1) の core の側。
//!
//! 偽 FS のカレントディレクトリは `/w`。ワークスペースのルートは `--workspace /w/repo` で決め、`/w/other` はルートの外。

use eve_config::{Conditions, Frontend, Os};
use eve_core::{
    ConfigSources, CtxValue, FsErrorKind, FsPath, InitialFile, OwnerVerdict, StartupConfig,
};
use eve_testkit::helpers::cell_text;
use eve_testkit::{FsOp, Harness};
use eve_view::OverlayModel;

const CONFIG: &str = "/evo/config/config.toml";
const WS_CONFIG: &str = "/w/repo/.eve/config.toml";
const WS_KEYMAP: &str = "/w/repo/.eve/keymap.toml";
const IN_ROOT: &str = "/w/repo/a.txt";
const OUTSIDE: &str = "/w/other/b.txt";

/// 起動の材料。
#[derive(Default)]
struct Setup<'a> {
    user: &'a str,
    ws: Option<&'a str>,
    keymap: Option<&'a str>,
    sets: &'a [&'a str],
    no_workspace_config: bool,
    /// 起動の前に偽 FS に手を加える。
    before: Option<&'a dyn Fn(&Harness)>,
    files: &'a [&'a str],
}

/// 条件の値 (偽 FS のパスは `/` で始まるので、パスの形は Unix の規則で確かめる)。
fn linux() -> Conditions {
    Conditions {
        os: Some(Os::Linux),
        frontend: Frontend::Tui,
        ..Conditions::default()
    }
}

/// `.eve` の中のファイルの表示名 (Port の組み立てと同じ `join`)。
fn eve(name: &str) -> String {
    let dir = FsPath::from("/w/repo").join(".eve");
    if name.is_empty() {
        dir.display()
    } else {
        dir.join(name).display()
    }
}

fn start(s: &Setup<'_>) -> Harness {
    let mut b = Harness::builder()
        .defer_start()
        .dir("/evo/config")
        .dir("/w/repo/.git")
        .dir("/w/other")
        .file(CONFIG, s.user.as_bytes())
        .file(IN_ROOT, b"text\n")
        .file(OUTSIDE, b"text\n");
    if let Some(t) = s.ws {
        b = b.file(WS_CONFIG, t.as_bytes());
    }
    if let Some(t) = s.keymap {
        b = b.file(WS_KEYMAP, t.as_bytes());
    }
    let mut h = b.build();
    if let Some(f) = s.before {
        f(&h);
    }
    let sets: Vec<String> = s.sets.iter().map(|x| (*x).to_owned()).collect();
    let resolved = eve_config::resolve(
        &[eve_config::Source {
            name: "config.toml".to_owned(),
            text: Ok(Some(s.user.to_owned())),
        }],
        &sets,
        linux(),
    );
    h.editor.configure(StartupConfig {
        config: Some(resolved.config),
        config_origins: Some(resolved.origins),
        config_user: Some(resolved.user),
        config_languages: Some(resolved.languages),
        config_sources: Some(ConfigSources {
            files: vec![("config.toml".to_owned(), FsPath::new(CONFIG))],
            keymap: None,
            sets,
            themes: None,
            languages: None,
            locales: None,
            conditions: linux(),
            no_workspace_config: s.no_workspace_config,
        }),
        workspace: Some("/w/repo".to_owned()),
        ..Default::default()
    });
    let files = if s.files.is_empty() {
        &[IN_ROOT][..]
    } else {
        s.files
    };
    let initial: Vec<InitialFile> = files.iter().map(|f| InitialFile::path(*f)).collect();
    h.editor.open_initial_files(&initial, false, None);
    h.started();
    h.run_jobs();
    h
}

/// 一覧 (`config.show` か `config.show_diagnostics`) を `query` で絞り込んだ (本文, 右の列)。
fn list(h: &mut Harness, command: &str, query: &str) -> Vec<(String, String)> {
    h.cmd(command);
    if h.frame().overlays.is_empty() {
        return Vec::new();
    }
    h.text(query);
    let f = h.frame();
    let OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    let rows = items
        .iter()
        .map(|i| {
            (
                cell_text(&i.label),
                i.detail.as_ref().map(cell_text).unwrap_or_default(),
            )
        })
        .collect();
    h.cmd("ui.cancel");
    rows
}

fn show(h: &mut Harness, key: &str) -> (String, String) {
    list(h, "config.show", key)
        .into_iter()
        .find(|(l, _)| l.starts_with(&format!("{key} =")))
        .unwrap_or_else(|| panic!("{key} の行が無い"))
}

fn diagnostics(h: &mut Harness) -> Vec<(String, String)> {
    list(h, "config.show_diagnostics", "")
}

fn ws_file(line: u32) -> String {
    format!("{}:{line}", eve("config.toml"))
}

fn language(h: &Harness) -> String {
    match h.editor.state().context_value("doc.language") {
        Some(CtxValue::Str(s)) => s,
        other => panic!("{other:?}"),
    }
}

// ---- AC-1: 安全な項目の適用と由来 ----

// verifies: AC-FR-705-1
#[test]
fn 安全な項目はユーザー層の上に重なり_由来はワークスペース層() {
    let mut h = start(&Setup {
        user: "[editor]\ntab_width = 8\n[view]\nwrap = \"off\"\n",
        ws: Some("# project\n[editor]\ntab_width = 2\n[view]\nwrap = \"soft\"\n"),
        files: &[IN_ROOT, OUTSIDE],
        ..Setup::default()
    });
    h.open(IN_ROOT);
    h.run_jobs();
    assert_eq!(
        show(&mut h, "editor.tab_width"),
        (
            "editor.tab_width = 2".to_owned(),
            "ワークスペース: .eve/config.toml:3".to_owned()
        )
    );
    assert_eq!(
        show(&mut h, "view.wrap"),
        (
            "view.wrap = \"soft\"".to_owned(),
            "ワークスペース: .eve/config.toml:5".to_owned()
        )
    );
    // 起動時に届いたときは要約を出さない
    assert!(
        !h.messages().iter().any(|m| m == "info.config.reloaded"),
        "{:?}",
        h.messages()
    );
    // ルートの外の文書には、文書ごとのキー (字下げ) を効かせない (07 §3.6 の LAYER-1)。文書に依らないキーは効く
    h.open(OUTSIDE);
    h.run_jobs();
    assert_eq!(
        show(&mut h, "editor.tab_width"),
        (
            "editor.tab_width = 8".to_owned(),
            "ユーザー: config.toml:2".to_owned()
        )
    );
    assert_eq!(show(&mut h, "view.wrap").0, "view.wrap = \"soft\"");
}

// verifies: AC-FR-705-1
#[test]
fn set_はワークスペース層より強い() {
    let mut h = start(&Setup {
        ws: Some("[editor]\ntab_width = 2\n"),
        sets: &["editor.tab_width=6"],
        ..Setup::default()
    });
    assert_eq!(
        show(&mut h, "editor.tab_width"),
        ("editor.tab_width = 6".to_owned(), "--set".to_owned())
    );
}

// verifies: AC-FR-705-1
#[test]
fn ワークスペース層の言語定義はルートの下の文書だけに効く() {
    let mut h = start(&Setup {
        ws: Some(
            "[[language]]\nname = \"foo\"\nfile_types = [\"foo\"]\nindent = { unit = \"tab\" }\n",
        ),
        before: Some(&|h: &Harness| {
            h.fs.add_file("/w/repo/x.foo", b"");
            h.fs.add_file("/w/other/y.foo", b"");
        }),
        files: &["/w/repo/x.foo", "/w/other/y.foo"],
        ..Setup::default()
    });
    // 起動時に開いた文書も、届いた後に判定し直す (07 §9.6 の LANG-7)
    h.open("/w/repo/x.foo");
    h.run_jobs();
    assert_eq!(language(&h), "foo");
    let (row, origin) = show(&mut h, "editor.indent_unit");
    assert_eq!(row, "editor.indent_unit = \"tab\"");
    assert_eq!(origin, "言語 foo: .eve/config.toml:4");
    h.open("/w/other/y.foo");
    h.run_jobs();
    assert_eq!(language(&h), "plain");
}

// verifies: AC-FR-705-1
#[test]
fn キーの割当は_eve_keymap_toml_から読み_使わない割当を知らせる() {
    let mut h = start(&Setup {
        keymap: Some(
            "[keys.editor]\n\"ctrl+alt+shift+f12\" = \"cursor.move_down\"\n\"ctrl+s\" = \"cursor.move_up\"\n",
        ),
        ..Setup::default()
    });
    let d = diagnostics(&mut h);
    let keymap = eve("keymap.toml");
    assert!(
        d.iter()
            .any(|(t, f)| t.contains("ctrl+s") && f == &format!("{keymap}:3")),
        "{d:?}"
    );
    assert!(
        !d.iter().any(|(t, _)| t.contains("ctrl+alt+shift+f12")),
        "{d:?}"
    );
}

// ---- AC-2: 保留・無視・効かない ----

// verifies: AC-FR-705-2
#[test]
fn 準安全と危険と常に無視と効かないは使わず_項目ごとに知らせる() {
    let mut h = start(&Setup {
        ws: Some(
            "[theme]\nname = \"x\"\n[file.swap]\nexclude_secret_files = false\n\
             [workspace]\nsafe_directories = [\"/srv\"]\n[view]\nambiguous_width = 2\n",
        ),
        ..Setup::default()
    });
    let c = h.editor.state().config().clone();
    assert_eq!(c.theme_name, "default");
    assert!(c.swap_exclude_secret_files);
    assert!(c.safe_directories.is_empty(), "{:?}", c.safe_directories);
    assert_eq!(c.ambiguous_width, 1);
    let d = diagnostics(&mut h);
    let at = |line| ws_file(line);
    let want = [
        (
            "theme.name はワークスペース設定からは使いません (準安全)",
            at(2),
        ),
        (
            "file.swap.exclude_secret_files = false はこのリポジトリが求めた値ですが、安全のため使っていません",
            at(4),
        ),
        (
            "workspace.safe_directories はワークスペース設定に書けません",
            at(6),
        ),
        (
            "view.ambiguous_width はワークスペース設定では効きません",
            at(8),
        ),
    ];
    for (text, file) in want {
        assert!(
            d.iter().any(|(t, f)| t.contains(text) && *f == file),
            "{text} @ {file}: {d:?}"
        );
    }
    // 情報の診断はメッセージ行に出さない (件数に数えない)。常に無視の警告 1 件だけを出す
    assert_eq!(h.messages(), ["msg.config.problem"]);
}

// verifies: AC-NFR-502-1
#[test]
fn ワークスペース設定の外部コマンドは使われず_コピーと保存でプロセスを起動しない() {
    let mut h = start(&Setup {
        ws: Some(
            "[clipboard]\nprovider = \"command\"\ncommand.copy = [\"sh\", \"-c\", \"touch /tmp/pwned\"]\n\
             [formatter.x]\ncommand = \"rm\"\n[language-server.y]\ncommand = \"evil\"\n",
        ),
        ..Setup::default()
    });
    let d = diagnostics(&mut h);
    for item in [
        "clipboard.provider",
        "clipboard.command",
        "formatter",
        "language-server",
    ] {
        assert!(
            d.iter()
                .any(|(t, _)| t.contains(item) && t.contains("未対応")),
            "{item}: {d:?}"
        );
    }
    // コピーは Effect (frontend のクリップボード) だけで、保存はファイルの書込だけ。core にはプロセスを起動する経路が無い
    h.keys("shift+end");
    h.cmd("clipboard.copy");
    h.keys("end");
    h.text("x");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(
        h.fs.read(IN_ROOT).as_deref(),
        Some(
            &b"textx
"[..]
        )
    );
    assert!(
        h.effects().iter().all(|e| matches!(
            e,
            eve_view::Effect::SetClipboard { .. } | eve_view::Effect::SetMouseCapture { .. }
        )),
        "{:?}",
        h.effects()
    );
    assert!(!h.fs.exists("/tmp/pwned"));
}

// ---- AC-3: 所有者の検査 ----

fn owner_case(verdict: OwnerVerdict, user: &str) -> Harness {
    let before = move |h: &Harness| {
        h.fs.set_owner_verdict("/w/repo/.eve", Some(verdict.clone()))
    };
    start(&Setup {
        user,
        ws: Some("[editor]\ntab_width = 2\n"),
        before: Some(&before),
        ..Setup::default()
    })
}

// verifies: AC-FR-705-3
#[test]
fn 所有者が違う_他人が書ける_判定できない置き場所の設定は読まずに理由を示す() {
    let dir = eve("");
    for (verdict, text) in [
        (
            OwnerVerdict::NotOwner {
                owner: "mallory".to_owned(),
            },
            format!("{dir} を読みませんでした: 所有者が mallory です"),
        ),
        (
            OwnerVerdict::OthersCanWrite {
                who: vec!["Everyone".to_owned()],
            },
            format!("{dir} を読みませんでした: ほかのユーザー (Everyone) が書き込めます"),
        ),
        (
            OwnerVerdict::Unknown {
                fs: "exFAT".to_owned(),
            },
            format!("{dir} を読みませんでした: この FS (exFAT) では所有者を判定できません"),
        ),
    ] {
        let mut h = owner_case(verdict, "");
        assert_eq!(h.editor.state().config().tab_width, 4, "{text}");
        let d = diagnostics(&mut h);
        assert!(d.iter().any(|(t, _)| t.contains(&text)), "{text}: {d:?}");
        assert!(
            h.messages() == ["msg.config.problem"],
            "警告としてメッセージ行にも出す: {:?}",
            h.messages()
        );
    }
}

// verifies: AC-FR-705-3
#[test]
fn ユーザー層の安全な場所の下は所有者の検査を掛けずに読む() {
    let mut h = owner_case(
        OwnerVerdict::Unknown {
            fs: "exFAT".to_owned(),
        },
        "[workspace]\nsafe_directories = [\"/w/repo\"]\n",
    );
    assert_eq!(h.editor.state().config().tab_width, 2);
    let d = diagnostics(&mut h);
    assert!(d.is_empty(), "{d:?}");
}

// verifies: AC-FR-705-3
#[test]
fn eve_のシンボリックリンクは辿らない() {
    let before = |h: &Harness| {
        h.fs.add_file("/w/evil/config.toml", b"[editor]\ntab_width = 2\n");
        h.fs.add_link("/w/repo/.eve", "/w/evil");
    };
    let mut h = start(&Setup {
        before: Some(&before),
        ..Setup::default()
    });
    assert_eq!(h.editor.state().config().tab_width, 4);
    let d = diagnostics(&mut h);
    assert!(
        d.iter()
            .any(|(t, _)| t.contains("ふつうのファイルではありません")),
        "{d:?}"
    );
}

// ---- 読む時期と CLI ----

#[test]
fn no_workspace_config_ではワークスペース層を読まない() {
    let mut h = start(&Setup {
        ws: Some("[editor]\ntab_width = 2\n"),
        no_workspace_config: true,
        ..Setup::default()
    });
    assert_eq!(h.editor.state().config().tab_width, 4);
    let d = diagnostics(&mut h);
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn config_reload_でワークスペース層も読み直して要約を出す() {
    let mut h = start(&Setup {
        ws: Some("[editor]\ntab_width = 2\n"),
        ..Setup::default()
    });
    h.fs.set_file(&FsPath::new(WS_CONFIG), b"[editor]\ntab_width = 3\n");
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(h.editor.state().config().tab_width, 3);
    assert!(
        h.messages().iter().any(|m| m == "info.config.reloaded"),
        "{:?}",
        h.messages()
    );
    // 一時の失敗 (共有違反・リース) では前に読んだ層のまま、診断を出さない
    h.fs.set_file(&FsPath::new(WS_CONFIG), b"[editor]\ntab_width = 5\n");
    h.fs.fail_next(FsOp::ReadChecked, FsErrorKind::Busy);
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(h.editor.state().config().tab_width, 3);
    let d = diagnostics(&mut h);
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn 未対応の置き場所は情報診断を_1_件ずつ出す() {
    let before = |h: &Harness| {
        h.fs.add_dir("/w/repo/.eve/languages");
        h.fs.add_dir("/w/repo/.eve/themes");
    };
    let mut h = start(&Setup {
        ws: Some(""),
        before: Some(&before),
        ..Setup::default()
    });
    let d = diagnostics(&mut h);
    for item in [".eve/languages", ".eve/themes"] {
        assert!(
            d.iter()
                .any(|(t, _)| t.contains(item) && t.contains("未対応")),
            "{item}: {d:?}"
        );
    }
}
