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
    /// 起動の前にファインダを開く (ルートとワークスペース層が届く前)。
    finder: bool,
    /// ホームディレクトリ (bin が環境変数から決める値)。
    home: Option<&'a str>,
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
    let resolved = eve_config::resolve_home(
        &[eve_config::Source {
            name: "config.toml".to_owned(),
            text: Ok(Some(s.user.to_owned())),
        }],
        &sets,
        linux(),
        s.home,
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
            home: s.home.map(str::to_owned),
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
    if s.finder {
        h.cmd("workspace.open_file");
    }
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
/// ワークスペース設定に書いた外部コマンド (`clipboard.provider = "command"`・`clipboard.command.*`・`[formatter.*]`・
/// `[language-server.*]`) は未対応の情報診断で使われず、コピー・貼り付け・保存の結果は frontend へのクリップボードの
/// Effect と偽 FS への書込だけ (Effect の種類の検査)。回帰の見張り: Harness (core) にはプロセスを起動する経路がそもそも
/// 無いので、印のファイルの検査は、後の版で core に起動の経路 (`ProcessRunner`) を足したときに初めて検出力を持つ。今の版で
/// 主張できるのは「設定の値が使われない」ことと「Effect にプロセスの起動が無い」ことまで。
#[test]
fn ワークスペース設定の外部コマンドは使われず_コピーと保存でプロセスを起動しない() {
    // 記録する偽のコマンド: 実行されると、実際の一時ディレクトリの印のファイルに 1 行足す (07 §12.6.1)
    // (実行の有無を見るので、偽 FS ではなく実際の一時ディレクトリ。名前はプロセスごとに違う)
    let marker = std::env::temp_dir().join(format!("eve-nfr502-{}", std::process::id()));
    assert!(
        !marker.exists(),
        "前の実行の印が残っている: {}",
        marker.display()
    );
    let m = marker.display().to_string();
    let cmd = if cfg!(windows) {
        format!("['cmd', '/C', 'echo x>>{m}']")
    } else {
        format!("['sh', '-c', 'echo x >> {m}']")
    };
    let ws = format!(
        "[clipboard]\nprovider = \"command\"\ncommand.copy = {cmd}\ncommand.paste = {cmd}\n\
         [formatter.x]\ncommand = {cmd}\n[language-server.y]\ncommand = {cmd}\n"
    );
    let mut h = start(&Setup {
        ws: Some(&ws),
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
    // コピー・貼り付け・保存。コピーは frontend のクリップボード (Effect) だけで、保存はファイルの書込だけ
    h.keys("shift+end");
    h.cmd("clipboard.copy");
    h.keys("end");
    h.cmd("clipboard.paste");
    h.text("x");
    h.cmd("file.save");
    h.run_jobs();
    assert!(
        h.fs.read(IN_ROOT).is_some_and(|b| b.ends_with(b"x\n")),
        "{:?}",
        h.fs.read(IN_ROOT)
    );
    assert!(
        h.effects().iter().all(|e| matches!(
            e,
            eve_view::Effect::SetClipboard { .. }
                | eve_view::Effect::RequestClipboard { .. }
                | eve_view::Effect::SetMouseCapture { .. }
        )),
        "{:?}",
        h.effects()
    );
    // 偽のコマンドの実行は 0 回
    assert!(
        !marker.exists(),
        "コマンドが実行された: {}",
        marker.display()
    );
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

// ---- レビューで直した点 (M33 の段 5 のレビュー) ----

/// ユーザーの設定ファイルを書き換えて `config.reload` する。
fn workspace_commands(h: &Harness) -> Vec<String> {
    h.editor
        .keymap()
        .listed_bindings()
        .into_iter()
        .filter(|(layer, _)| *layer == "workspace")
        .map(|(_, b)| b.command.as_str().to_owned())
        .collect()
}

fn reload_with_user(h: &mut Harness, user: &str) {
    h.fs.set_file(&FsPath::new(CONFIG), user.as_bytes());
    h.cmd("config.reload");
    h.run_jobs();
}

fn tab_width(h: &Harness) -> u8 {
    h.editor.state().config().tab_width
}

// verifies: AC-FR-705-3
#[test]
fn config_reload_で足した安全な場所と外した安全な場所は_1_回目から効く() {
    let before = |h: &Harness| {
        h.fs.set_owner_verdict(
            "/w/repo/.eve",
            Some(OwnerVerdict::Unknown {
                fs: "exFAT".to_owned(),
            }),
        );
    };
    let mut h = start(&Setup {
        ws: Some("[editor]\ntab_width = 2\n"),
        before: Some(&before),
        ..Setup::default()
    });
    assert_eq!(tab_width(&h), 4);
    // 足した: 読み直したユーザー層の値で、その回の読込から検査を省く
    reload_with_user(&mut h, "[workspace]\nsafe_directories = [\"/w/repo\"]\n");
    assert_eq!(tab_width(&h), 2);
    // 外した: その回から読まない
    reload_with_user(&mut h, "");
    assert_eq!(tab_width(&h), 4);
}

// verifies: AC-FR-705-2
#[test]
fn 一時の失敗で保った層も_読み直したユーザー層と比べて分類し直す() {
    let mut h = start(&Setup {
        user: "[file]\nhuge_file_warn_bytes = 209715200\n",
        ws: Some("[file]\nhuge_file_warn_bytes = 104857600\n"),
        ..Setup::default()
    });
    assert_eq!(h.editor.state().config().huge_file_warn_bytes, 100 << 20);
    // ユーザー層が閾値を下げた。ワークスペース層は一時の失敗で前のまま: 大きくする向きになったので使わない
    h.fs.fail_next(FsOp::ReadChecked, FsErrorKind::Busy);
    reload_with_user(&mut h, "[file]\nhuge_file_warn_bytes = 10485760\n");
    assert_eq!(h.editor.state().config().huge_file_warn_bytes, 10 << 20);
    let d = diagnostics(&mut h);
    assert!(
        d.iter()
            .any(|(t, f)| t.contains("file.huge_file_warn_bytes") && *f == ws_file(2)),
        "{d:?}"
    );
}

// verifies: AC-FR-705-1
#[test]
fn config_reload_で壊れたワークスペース設定は前の層を保ち_無くなったら外す() {
    let mut h = start(&Setup {
        ws: Some("[editor]\ntab_width = 2\n"),
        keymap: Some("[keys.editor]\n\"ctrl+alt+shift+f12\" = \"cursor.move_down\"\n"),
        ..Setup::default()
    });
    assert_eq!(tab_width(&h), 2);
    h.fs.set_file(&FsPath::new(WS_CONFIG), b"[editor\ntab_width = 3\n");
    h.fs.set_file(&FsPath::new(WS_KEYMAP), b"[keys.editor\n");
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(tab_width(&h), 2, "前の層を保つ");
    let kept = h
        .messages()
        .iter()
        .filter(|m| *m == "msg.config.reload_kept")
        .count();
    assert_eq!(kept, 2, "{:?}", h.messages());
    let d = diagnostics(&mut h);
    assert!(
        d.iter().any(|(_, f)| f.starts_with(&eve("config.toml"))),
        "{d:?}"
    );
    // キーの割当も保つ (ヘルプのキーの一覧に残る)
    assert!(
        workspace_commands(&h).contains(&"cursor.move_down".to_owned()),
        "{:?}",
        workspace_commands(&h)
    );
    // ファイルが無くなったら外す
    h.fs.remove(&FsPath::new(WS_CONFIG));
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(tab_width(&h), 4);
}

// verifies: AC-FR-705-2
#[test]
fn 一時の失敗では層と一緒に診断の一覧も保つ() {
    let mut h = start(&Setup {
        ws: Some("[theme]\nname = \"x\"\n"),
        ..Setup::default()
    });
    let before = diagnostics(&mut h);
    assert!(
        before.iter().any(|(t, _)| t.contains("theme.name")),
        "{before:?}"
    );
    h.fs.fail_next(FsOp::ReadChecked, FsErrorKind::Busy);
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(diagnostics(&mut h), before);
}

// verifies: AC-FR-705-1
#[test]
fn キーの割当の上限は書いた位置の順で数える() {
    // 表のキーの順 (名前の順) では後に来るキーを先に書き、上限の 1,000 件の内に入れる
    let mut text = String::from("[keys.editor]\n\"ctrl+alt+shift+f12\" = \"cursor.move_down\"\n");
    // どれも使える割当 (3 打のチョード。2 打目以降は文字のキーでよい) にして、上限の警告が診断の上限に隠れないようにする
    let keys: Vec<char> = ('0'..='9').chain('a'..='z').collect();
    for i in 0..1_000 {
        let (a, b) = (keys[i / keys.len()], keys[i % keys.len()]);
        text.push_str(&format!(
            "\"ctrl+alt+shift+a {a} {b}\" = \"cursor.move_up\"\n"
        ));
    }
    let mut h = start(&Setup {
        keymap: Some(&text),
        ..Setup::default()
    });
    let d = list(&mut h, "config.show_diagnostics", "1000");
    let keymap = eve("keymap.toml");
    // 1,001 件目 (位置の順で最後の行) で上限の警告
    assert!(
        d.iter()
            .any(|(t, f)| t.contains("1000") && *f == format!("{keymap}:1002")),
        "{:?}",
        d.iter()
            .filter(|(t, _)| t.contains("1000"))
            .collect::<Vec<_>>()
    );
    // 最初に書いた割当は使う
    assert!(
        workspace_commands(&h).contains(&"cursor.move_down".to_owned()),
        "{:?}",
        workspace_commands(&h)
    );
}

// verifies: AC-FR-704-2
#[test]
fn 起動時に届いたワークスペース層の言語定義で_設定ファイルの言語の診断が_2_重にならない() {
    let mut h = start(&Setup {
        user: "[[language]]\nname = \"bar\"\nfirst_line = \"(\"\n",
        ws: Some("[[language]]\nname = \"foo\"\nfile_types = [\"foo\"]\n"),
        ..Setup::default()
    });
    let d = list(&mut h, "config.show_diagnostics", "first_line");
    let n = d
        .iter()
        .filter(|(_, f)| f.starts_with("config.toml"))
        .count();
    assert_eq!(n, 1, "{d:?}");
}

// verifies: AC-FR-704-2
#[test]
fn ワークスペース層の先頭行の正規表現は_io_のジョブで組み_組めなければファイルの名前で知らせる() {
    let mut h = start(&Setup {
        ws: Some("[[language]]\nname = \"foo\"\nfirst_line = \"(\"\n"),
        ..Setup::default()
    });
    // 文書の判定に使う前に (この文書は foo に当たらない)、io のジョブで組んで、組めない値を知らせている
    let d = list(&mut h, "config.show_diagnostics", "first_line");
    assert!(
        d.iter()
            .any(|(t, f)| t.contains("first_line") && *f == ws_file(3)),
        "{d:?}"
    );
}

// verifies: AC-FR-705-1
#[test]
fn 無題の文書と新しいファイルに_届いた後でワークスペース層の改行を当てる() {
    let mut h = start(&Setup {
        ws: Some("[file]\ndefault_line_ending = \"crlf\"\n"),
        files: &["/w/repo/new.txt"],
        ..Setup::default()
    });
    h.keys("x");
    h.keys("enter");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.fs.read("/w/repo/new.txt").as_deref(), Some(&b"x\r\n"[..]));
    // 無題の文書 (ルートの外でもパスが無いので (6) の段が効く)
    h.cmd("file.new");
    h.keys("y");
    h.keys("enter");
    assert_eq!(h.doc_text(), "y\r\n");
}

// verifies: AC-FR-705-1
#[test]
fn 起動直後のファインダの走査は_届いたワークスペース層の除外でやり直す() {
    let before = |h: &Harness| {
        h.fs.add_file("/w/repo/keep.txt", b"");
        h.fs.add_file("/w/repo/skip.log", b"");
    };
    let h = start(&Setup {
        ws: Some("[workspace]\nexclude = [\"*.log\"]\n"),
        before: Some(&before),
        finder: true,
        ..Setup::default()
    });
    let (_, _, rows, _) = h
        .editor
        .state()
        .finder_rows()
        .expect("ファインダが開いている");
    assert!(rows.iter().any(|r| r.contains("keep.txt")), "{rows:?}");
    assert!(!rows.iter().any(|r| r.contains("skip.log")), "{rows:?}");
}

// verifies: AC-FR-705-1
#[test]
fn ワークスペース層のキーの割当は確認のダイアログに足せない() {
    let mut h = start(&Setup {
        keymap: Some("[keys.dialog]\n\"pagedown\" = \"ui.confirm\"\n"),
        ..Setup::default()
    });
    let d = diagnostics(&mut h);
    assert!(
        d.iter()
            .any(|(t, _)| t.contains("pagedown") && t.contains("確認のダイアログ")),
        "{d:?}"
    );
}

// ---- 再レビューで直した点 ----

// verifies: AC-FR-705-3
#[test]
fn ホームディレクトリそのものを安全な場所に書くと_起動時も読み直した後も_1_件知らせる() {
    let user = "[workspace]\nsafe_directories = [\"/home/u\"]\n";
    let mut h = start(&Setup {
        user,
        home: Some("/home/u"),
        ..Setup::default()
    });
    let count = |h: &mut Harness| {
        list(h, "config.show_diagnostics", "広すぎます")
            .into_iter()
            .filter(|(_, f)| f == "config.toml:2")
            .count()
    };
    // 起動時の診断は bin が resolve_home の結果を渡す (ここでは同じ関数の結果を報告する)
    let resolved = eve_config::resolve_home(
        &[eve_config::Source {
            name: "config.toml".to_owned(),
            text: Ok(Some(user.to_owned())),
        }],
        &[],
        linux(),
        Some("/home/u"),
    );
    for (file, diags) in &resolved.reports {
        h.editor.report(eve_core::HostReport::ConfigDiagnostics {
            file,
            diagnostics: diags,
        });
    }
    assert_eq!(count(&mut h), 1);
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(count(&mut h), 1);
}

// verifies: AC-FR-705-1
#[test]
fn 明示的に選んだ改行は_読み直しとワークスペース層で決め直さない() {
    let mut h = start(&Setup {
        ws: Some("[file]\ndefault_line_ending = \"crlf\"\n"),
        files: &["/w/repo/new.txt"],
        ..Setup::default()
    });
    h.cmd_with(
        "file.set_line_ending",
        eve_core::Args::new().with_str("eol", "lf"),
    );
    h.cmd("config.reload");
    h.run_jobs();
    h.keys("x");
    h.keys("enter");
    assert_eq!(h.doc_text(), "x\n");
    // 無題の文書も同じ
    h.cmd("file.new");
    h.cmd_with(
        "file.set_line_ending",
        eve_core::Args::new().with_str("eol", "lf"),
    );
    h.cmd("config.reload");
    h.run_jobs();
    h.keys("y");
    h.keys("enter");
    assert_eq!(h.doc_text(), "y\n");
}

// verifies: AC-FR-705-1
#[test]
fn ワークスペース層のキーの割当の診断は_1_つのファイルで上限まで() {
    let mut text = String::from("[keys.editor]\n");
    for i in 0..150 {
        text.push_str(&format!(
            "\"ctrl+alt+shift+f12 {i}\" = {{ command = \"cursor.move_down\", args = {{ x = 1 }} }}\n"
        ));
    }
    let mut h = Harness::new();
    let d = h
        .editor
        .set_workspace_keymap(Some(&eve_config::load_keymap_str(&text)));
    assert_eq!(d.len(), eve_config::CONFIG_MAX_DIAGNOSTICS + 1);
    assert!(matches!(
        d.last().map(|d| &d.kind),
        Some(eve_config::DiagKind::More { count: 50 })
    ));
}

// verifies: AC-FR-705-3
#[test]
fn config_reload_で置き場所を読めなくなったら層を外す() {
    let mut h = start(&Setup {
        ws: Some("[editor]\ntab_width = 2\n"),
        ..Setup::default()
    });
    assert_eq!(tab_width(&h), 2);
    h.fs.set_unreadable("/w/repo/.eve", true);
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(tab_width(&h), 4, "信頼の取り消し (権限を外した) を通す");
    assert!(
        !h.messages().iter().any(|m| m == "msg.config.reload_kept"),
        "{:?}",
        h.messages()
    );
}

// ---- 再々レビューで直した点 ----

/// 診断の一覧を `query` で絞り込んだ件数 (一覧の下の `{一致}/{全体}` の一致の数)。
fn matched(h: &mut Harness, query: &str) -> usize {
    h.cmd("config.show_diagnostics");
    if h.frame().overlays.is_empty() {
        return 0;
    }
    h.text(query);
    let f = h.frame();
    let OverlayModel::List { footer, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    let text = cell_text(footer);
    h.cmd("ui.cancel");
    text.split('/')
        .next()
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("件数が読めない: {text}"))
}

// verifies: AC-FR-705-2
#[test]
fn 言語定義の診断が最初の報告でも_1_つのファイルの上限を超えない() {
    let ws: String = (0..150)
        .map(|i| format!("[[language]]\nname = \"l{i}\"\nfirst_line = \"(\"\n"))
        .collect();
    let mut h = start(&Setup {
        ws: Some(&ws),
        ..Setup::default()
    });
    assert_eq!(
        matched(&mut h, "first_line"),
        eve_config::CONFIG_MAX_DIAGNOSTICS
    );
    assert_eq!(matched(&mut h, "ほかに"), 1);
}

// verifies: AC-FR-705-3
#[test]
fn 一時の失敗で前の層を保つときは知らせ_安全な場所が変わったら外す() {
    let mut h = start(&Setup {
        user: "[workspace]\nsafe_directories = [\"/w/repo\"]\n",
        ws: Some("[editor]\ntab_width = 2\n"),
        ..Setup::default()
    });
    assert_eq!(tab_width(&h), 2);
    // 一時の失敗: 前の層を保ち、知らせる
    h.fs.fail_next(FsOp::ReadChecked, FsErrorKind::Busy);
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(tab_width(&h), 2);
    assert!(
        h.messages().iter().any(|m| m == "msg.config.reload_kept"),
        "{:?}",
        h.messages()
    );
    // 逃げ道を外した読み直しでの一時の失敗: 前の層を保たない (信頼の取り消しを黙って止められない)
    h.fs.set_file(&FsPath::new(CONFIG), b"");
    h.fs.fail_next(FsOp::ReadChecked, FsErrorKind::Busy);
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(tab_width(&h), 4);
}

// verifies: AC-FR-705-1
#[test]
fn 明示的に改行を選んだ新しいファイルは_後から届いたワークスペース層の改行と文字コードを当てない() {
    let mut h = start(&Setup {
        files: &["/w/repo/new.txt"],
        ..Setup::default()
    });
    h.cmd_with(
        "file.set_line_ending",
        eve_core::Args::new().with_str("eol", "lf"),
    );
    // ワークスペース層が後から届く (config.reload で読む)
    h.fs.add_dir("/w/repo/.eve");
    h.fs.add_file(
        WS_CONFIG,
        b"[file]\ndefault_line_ending = \"crlf\"\ndefault_encoding = \"shift_jis\"\n",
    );
    h.cmd("config.reload");
    h.run_jobs();
    h.text("あ");
    h.keys("enter");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(
        h.fs.read("/w/repo/new.txt").as_deref(),
        Some("あ\n".as_bytes()),
        "改行も文字コードも選んだ時のまま (UTF-8・LF)"
    );
}
