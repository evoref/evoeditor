//! 言語定義と文書ごとの値 (docs/07 §9.6 の LANG-1〜LANG-9、§3.6 の LAYER-1〜LAYER-5、01 FR-704) の core の側:
//! 起動時の言語定義の読込 (`ReadLanguages`)、判定と Tab の字下げ、`--set` の強さ、`lang.set`、`doc.language`、`config.show` の
//! 由来、`config.reload`、名前を付けて保存、誤りの診断。

use eve_config::{Conditions, Frontend, Os};
use eve_core::{Args, ConfigSources, CtxValue, FsPath, InitialFile, StartupConfig};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_view::OverlayModel;

const CONFIG: &str = "/evo/config/config.toml";
const LANGS: &str = "/evo/config/languages";

/// 設定の読込元 (bin が起動時に決める形) を置いて `files` を開いて起動した Harness。`fs` は起動の前に置くファイル。
fn start(fs: &[(&str, &[u8])], sets: &[&str], files: &[&str]) -> Harness {
    let mut b = Harness::builder().defer_start().dir("/evo/config");
    for (p, data) in fs {
        b = b.file(p, data);
    }
    let mut h = b.build();
    // 設定ファイルの言語定義は bin が起動時に読んで渡す (ここでは同じ関数で読む)
    let text = fs
        .iter()
        .find(|(p, _)| *p == CONFIG)
        .map(|(_, d)| String::from_utf8(d.to_vec()).unwrap());
    let resolved = eve_config::resolve(
        &[eve_config::Source {
            name: "config.toml".to_owned(),
            text: Ok(text),
        }],
        &sets.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
        Conditions::default(),
    );
    h.editor.configure(StartupConfig {
        config: Some(resolved.config),
        config_origins: Some(resolved.origins),
        config_languages: Some(resolved.languages),
        config_sources: Some(ConfigSources {
            files: vec![("config.toml".to_owned(), FsPath::new(CONFIG))],
            keymap: None,
            sets: sets.iter().map(|s| (*s).to_owned()).collect(),
            themes: None,
            languages: Some(FsPath::new(LANGS)),
            locales: None,
            conditions: Conditions {
                os: Some(Os::Linux),
                frontend: Frontend::Tui,
                ..Conditions::default()
            },
        }),
        ..Default::default()
    });
    let initial: Vec<InitialFile> = files.iter().map(|f| InitialFile::path(*f)).collect();
    h.editor.open_initial_files(&initial, false, None);
    h.started();
    h.run_jobs();
    h
}

fn language(h: &Harness) -> String {
    match h.editor.state().context_value("doc.language") {
        Some(CtxValue::Str(s)) => s,
        other => panic!("{other:?}"),
    }
}

/// 空の文書で Tab を押して入った字下げ。
fn tab(h: &mut Harness) -> String {
    h.keys("tab");
    let t = h.doc_text();
    h.cmd("edit.undo");
    t
}

/// `config.show` を開いて `query` で絞り込んだ (見出し, 本文, 由来)。
fn show(h: &mut Harness, query: &str) -> (String, Vec<(String, String)>) {
    h.cmd("config.show");
    h.text(query);
    let f = h.frame();
    let OverlayModel::List { title, items, .. } = &f.overlays[0] else {
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
    let title = cell_text(title);
    h.cmd("ui.cancel");
    (title, rows)
}

// verifies: AC-FR-704-1
#[test]
fn 組込みの定義は_go_と_make_の_tab_だけで_ほかは一般の値() {
    let mut h = start(
        &[
            ("/w/a.go", b""),
            ("/w/Makefile", b""),
            ("/w/a.rs", b""),
            ("/w/README", b""),
        ],
        &[],
        &["/w/a.go", "/w/Makefile", "/w/a.rs", "/w/README"],
    );
    let expect = [
        ("a.go", "go", "\t"),
        ("Makefile", "make", "\t"),
        ("a.rs", "rust", "    "),
        ("README", "plain", "    "),
    ];
    for (name, lang, indent) in expect {
        h.open(&format!("/w/{name}"));
        h.run_jobs();
        assert_eq!(language(&h), lang, "{name}");
        assert_eq!(tab(&mut h), indent, "{name}");
    }
    // 無題の文書は plain
    h.cmd("file.new");
    assert_eq!(language(&h), "plain");
}

// verifies: AC-FR-704-1
#[test]
fn ユーザー層の言語定義の字下げが効き_利用者の一般の値より強い() {
    let mut h = start(
        &[
            (
                CONFIG,
                b"[editor]\nindent_width = 8\n[[language]]\nname = \"rust\"\nindent = { width = 2 }\n",
            ),
            (
                "/evo/config/languages/py.toml",
                b"[[language]]\nname = \"python\"\nindent = { unit = \"tab\", tab_width = 3 }\n\
                  [[language]]\nname = \"tmpl\"\nfile_types = [\"tmpl\"]\n",
            ),
            ("/w/a.rs", b""),
            ("/w/a.py", b""),
            ("/w/a.tmpl", b""),
            ("/w/a.txt", b""),
        ],
        &[],
        &["/w/a.rs"],
    );
    assert_eq!(language(&h), "rust");
    assert_eq!(tab(&mut h), "  ", "言語の幅 2 が一般の値 8 に勝つ");
    h.open("/w/a.py");
    h.run_jobs();
    assert_eq!(tab(&mut h), "\t");
    let d = h.editor.state().document(h.doc_id()).unwrap();
    assert_eq!(d.display.tab_width, 3, "タブの表示の幅も言語ごと");
    h.open("/w/a.tmpl");
    h.run_jobs();
    assert_eq!(language(&h), "tmpl", "ファイル名と言語の名前は結び付けない");
    h.open("/w/a.txt");
    h.run_jobs();
    assert_eq!(tab(&mut h), " ".repeat(8), "言語の無い文書は一般の値");
}

// verifies: AC-FR-704-1
#[test]
fn set_は言語の値より強い() {
    let mut h = start(
        &[("/w/a.go", b"")],
        &["editor.indent_unit=space", "editor.indent_width=3"],
        &["/w/a.go"],
    );
    assert_eq!(language(&h), "go");
    assert_eq!(tab(&mut h), "   ");
    let (_, rows) = show(&mut h, "editor.indent_unit");
    assert_eq!(
        rows,
        [(
            "editor.indent_unit = \"space\"".to_owned(),
            "--set".to_owned()
        )]
    );
}

// verifies: AC-FR-704-1
#[test]
fn lang_set_で言語を替えると字下げも替わり_auto_で判定し直す() {
    let mut h = start(&[("/w/a.txt", b"")], &[], &["/w/a.txt"]);
    assert_eq!(language(&h), "plain");
    h.cmd_with("lang.set", Args::new().with_str("language", "go"));
    assert_eq!(language(&h), "go");
    assert_eq!(
        h.messages().last().map(String::as_str),
        Some("info.lang.changed")
    );
    assert!(h.frame_contains("言語を Go (go) にしました"));
    assert_eq!(tab(&mut h), "\t");
    // 明示の指定は名前を付けて保存でも保つ
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/b.rs"));
    h.run_jobs();
    assert_eq!(language(&h), "go");
    // auto は判定し直す (今のパスで)
    h.cmd_with("lang.set", Args::new().with_str("language", "auto"));
    assert_eq!(language(&h), "rust");
    assert_eq!(tab(&mut h), "    ");
    // 定義の無い言語は替えない
    h.cmd_with("lang.set", Args::new().with_str("language", "nope"));
    assert_eq!(language(&h), "rust");
    assert!(!h.is_dirty());
}

#[test]
fn lang_set_は読み取り専用の文書でも使え_undo_の履歴に残らない() {
    let mut h = Harness::builder()
        .defer_start()
        .file(
            "/w/a.txt", b"x
",
        )
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/a.txt")], true, None);
    h.started();
    h.run_jobs();
    let before = {
        let d = h.editor.state().document(h.doc_id()).unwrap();
        assert!(!d.is_editable(), "読み取り専用");
        (d.revision(), d.history().undo_len())
    };
    assert!(matches!(
        h.cmd_with("lang.set", Args::new().with_str("language", "go")),
        eve_core::CommandResult::Done
    ));
    assert_eq!(language(&h), "go");
    let d = h.editor.state().document(h.doc_id()).unwrap();
    assert_eq!((d.revision(), d.history().undo_len()), before);
    assert!(!h.is_dirty());
}

#[test]
fn lang_set_を引数なしで実行すると一覧で尋ねる() {
    let mut h = start(&[("/w/a.go", b"")], &[], &["/w/a.go"]);
    h.cmd("lang.set");
    assert_eq!(h.overlay(), "palette");
    let f = h.frame();
    let OverlayModel::List { title, items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    assert_eq!(cell_text(title), "言語を選ぶ (今: go)");
    let row = |i: usize| {
        (
            cell_text(&items[i].label),
            items[i].detail.as_ref().map(cell_text).unwrap_or_default(),
        )
    };
    assert_eq!(row(0), ("自動で判定".to_owned(), "auto".to_owned()));
    assert_eq!(row(1), ("plain".to_owned(), "plain".to_owned()));
    assert_eq!(row(2), ("Rust".to_owned(), "rust".to_owned()));
    // 今の言語の行には印
    h.text("go");
    let f = h.frame();
    let OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    let go = items
        .iter()
        .find(|i| cell_text(&i.label) == "Go")
        .expect("Go の行");
    assert_eq!(
        go.detail.as_ref().map(cell_text).as_deref(),
        Some("go (今の言語)")
    );
    h.keys("ctrl+a backspace");
    // 絞り込んで選ぶ
    h.text("makef");
    h.keys("enter");
    assert_eq!(language(&h), "make");
}

#[test]
fn doc_language_は_when_で使える() {
    let h = start(&[("/w/a.go", b"")], &[], &["/w/a.go"]);
    assert_eq!(
        h.editor.state().context_value("doc.language"),
        Some(CtxValue::Str("go".to_owned()))
    );
    let e = eve_core::ContextExpr::parse("doc.language == \"go\"").unwrap();
    assert!(e.eval(&|n| h.editor.state().context_value(n)));
}

#[test]
fn config_show_は文書の値と勝った言語を示す() {
    let mut h = start(
        &[
            (
                CONFIG,
                b"[[language]]\nname = \"rust\"\nindent = { width = 2 }\n",
            ),
            ("/w/a.go", b""),
            ("/w/a.rs", b""),
        ],
        &[],
        &["/w/a.go"],
    );
    let (title, rows) = show(&mut h, "editor.");
    assert_eq!(title, "設定 (a.go・go)");
    assert!(rows.contains(&(
        "editor.indent_unit = \"tab\"".to_owned(),
        "言語 go (組込み)".to_owned()
    )));
    assert!(rows.contains(&("editor.tab_width = 4".to_owned(), "既定".to_owned())));
    h.open("/w/a.rs");
    h.run_jobs();
    let (title, rows) = show(&mut h, "indent_width");
    assert_eq!(title, "設定 (a.rs・rust)");
    assert_eq!(
        rows,
        [(
            "editor.indent_width = 2".to_owned(),
            "言語 rust: config.toml:3".to_owned()
        )]
    );
    // Enter で書いた所を開く
    h.cmd("config.show");
    h.text("indent_width");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(
        h.doc_path().as_deref(),
        Some(FsPath::new(CONFIG).display().as_str())
    );
}

// verifies: AC-FR-704-1
#[test]
fn 読み直しは判定し直さず_文書ごとの値を決め直す() {
    let mut h = start(
        &[
            ("/w/a.rs", b""),
            (
                "/evo/config/languages/r.toml",
                b"[[language]]\nname = \"rust\"\nindent = { width = 2 }\n",
            ),
        ],
        &[],
        &["/w/a.rs"],
    );
    assert_eq!(tab(&mut h), "  ");
    // 定義の拡張子を替えても、開いている文書の言語は変えない。字下げは新しい定義で決め直す
    h.fs.add_file(
        "/evo/config/languages/r.toml",
        b"[[language]]\nname = \"rust\"\nfile_types = [\"rsx\"]\nindent = { width = 6 }\n",
    );
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(language(&h), "rust");
    assert_eq!(tab(&mut h), " ".repeat(6));
}

// verifies: AC-FR-704-1
#[test]
fn 名前を付けて保存でパスが変われば判定し直す() {
    let mut h = start(&[("/w/a.txt", b"")], &[], &["/w/a.txt"]);
    assert_eq!(language(&h), "plain");
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/Makefile"));
    h.run_jobs();
    assert_eq!(language(&h), "make");
    assert_eq!(tab(&mut h), "\t");
}

#[test]
fn 先頭行で判定する() {
    let h = start(
        &[("/w/run", b"#!/usr/bin/env python3\nprint(1)\n")],
        &[],
        &["/w/run"],
    );
    assert_eq!(language(&h), "python");
}

// verifies: AC-FR-704-2
#[test]
fn 効かない項目は未対応の情報で起動を妨げず_誤りはファイルと行で警告する() {
    let mut h = start(
        &[
            (
                "/evo/config/languages/a.toml",
                b"[[language]]\nname = \"rust\"\nauto_pairs = { \"(\" = \")\" }\nformatter = \"rustfmt\"\nindent = { width = 0 }\n",
            ),
            ("/w/a.rs", b""),
        ],
        &[],
        &["/w/a.rs"],
    );
    assert_eq!(language(&h), "rust");
    assert_eq!(tab(&mut h), "    ", "誤った幅は使わず、下の層の値");
    // メッセージ行には警告だけが出る (情報は件数に数えない)
    assert!(h.messages().contains(&"msg.config.problem".to_owned()));
    h.cmd("config.show_diagnostics");
    let f = h.frame();
    let OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    let rows: Vec<(String, String)> = items
        .iter()
        .map(|i| {
            (
                cell_text(&i.label),
                i.detail.as_ref().map(cell_text).unwrap_or_default(),
            )
        })
        .collect();
    assert!(
        rows.iter().any(|(l, p)| l.contains("language.auto_pairs")
            && l.contains("このバージョン")
            && p == "languages/a.toml:3"),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|(l, p)| l.contains("language.indent.width") && p == "languages/a.toml:5"),
        "{rows:?}"
    );
}

#[test]
fn 同じ層で重なった言語は後の定義を使って警告する() {
    let mut h = start(
        &[
            (
                "/evo/config/languages/a.toml",
                b"[[language]]\nname = \"a\"\nfile_types = [\"foo\"]\n[[language]]\nname = \"b\"\nfile_types = [\"foo\"]\n",
            ),
            ("/w/x.foo", b""),
        ],
        &[],
        &["/w/x.foo"],
    );
    assert_eq!(language(&h), "b");
    assert!(h.frame_contains("x.foo は言語 a と b の両方に当たるため、b を使います"));
}

#[test]
fn 言語定義の読込は起動時のパスを開くジョブより前() {
    let mut h = Harness::builder()
        .defer_start()
        .file("/w/a.foo", b"")
        .file(
            "/evo/config/languages/f.toml",
            b"[[language]]\nname = \"foo\"\nfile_types = [\"foo\"]\n",
        )
        .build();
    h.editor.configure(StartupConfig {
        config_sources: Some(ConfigSources {
            languages: Some(FsPath::new(LANGS)),
            ..ConfigSources::default()
        }),
        ..Default::default()
    });
    h.editor
        .open_initial_files(&[InitialFile::path("/w/a.foo")], false, None);
    h.started();
    // 1 つ目のジョブが言語定義の読込
    h.run_one_job();
    assert!(h.editor.state().languages().get("foo").is_some());
    h.run_jobs();
    assert_eq!(language(&h), "foo");
}

// verifies: AC-FR-704-1
#[test]
fn 再読込と開き直しは言語を引き継ぎ_明示の指定も保つ() {
    let mut h = start(&[("/w/a.txt", b"x\n")], &[], &["/w/a.txt"]);
    h.cmd_with("lang.set", Args::new().with_str("language", "go"));
    h.cmd("file.reload");
    h.run_jobs();
    assert_eq!(language(&h), "go");
    let d = h.editor.state().document(h.doc_id()).unwrap();
    assert!(d.language.explicit, "明示の指定を保つ");
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    h.run_jobs();
    assert_eq!(language(&h), "go");
    assert!(
        h.editor
            .state()
            .document(h.doc_id())
            .unwrap()
            .language
            .explicit
    );
}

#[test]
fn 標準入力の文書は先頭行で判定する() {
    use eve_core::{InitialTarget, StdinKind};
    let fake = eve_testkit::FakeStdin::chunks(b"#!/usr/bin/env python3\nprint(1)\n", 64);
    let mut h = Harness::builder()
        .defer_start()
        .stdin(std::sync::Arc::new(fake))
        .build();
    h.editor.open_initial_files(
        &[InitialFile {
            target: InitialTarget::Stdin(StdinKind::Stream),
            line: None,
            col: None,
        }],
        false,
        None,
    );
    h.started();
    h.run_jobs();
    assert_eq!(language(&h), "python");
}

// verifies: AC-FR-704-1
#[test]
fn ルートが決まったら斜線を含む_glob_で判定し直す() {
    let mut h = Harness::builder()
        .defer_start()
        .dir("/w/proj/.git")
        .file("/w/proj/sub/a.x", b"")
        .build();
    let r = eve_config::resolve(
        &[eve_config::Source {
            name: "config.toml".to_owned(),
            text: Ok(Some(
                "[[language]]\nname = \"xl\"\nfile_globs = [\"sub/*.x\"]\n".to_owned(),
            )),
        }],
        &[],
        Conditions::default(),
    );
    h.editor.configure(StartupConfig {
        config_languages: Some(r.languages),
        ..Default::default()
    });
    h.editor
        .open_initial_files(&[InitialFile::path("/w/proj/sub/a.x")], false, None);
    h.started();
    // 言語定義 → 開く (ルートはまだ無いので、`/` を含む glob は当たらない)
    h.run_one_job();
    h.run_one_job();
    assert_eq!(language(&h), "plain");
    // ルートが決まると判定し直す
    h.run_jobs();
    assert!(h.editor.state().workspace().is_some());
    assert_eq!(language(&h), "xl");
}

#[test]
fn 開くジョブの結果が定義より先に届いたら_定義が届いたときに判定し直す() {
    let mut h = Harness::builder()
        .defer_start()
        .file("/w/a.foo", b"")
        .file(
            "/evo/config/languages/f.toml",
            b"[[language]]\nname = \"foo\"\nfile_types = [\"foo\"]\n",
        )
        .build();
    h.editor.configure(StartupConfig {
        config_sources: Some(ConfigSources {
            languages: Some(FsPath::new(LANGS)),
            ..ConfigSources::default()
        }),
        ..Default::default()
    });
    h.editor
        .open_initial_files(&[InitialFile::path("/w/a.foo")], false, None);
    h.started();
    let languages = h.run_one_job_held();
    h.run_one_job();
    assert_eq!(language(&h), "plain", "定義が届く前は組込みで判定");
    h.deliver(languages);
    assert_eq!(language(&h), "foo");
}

#[test]
fn 言語定義のファイルの数と大きさと拡張子() {
    let h = Harness::new();
    // 大文字の .TOML は読まない
    h.fs.add_file(
        "/evo/config/languages/A.TOML",
        b"[[language]]\nname = \"up\"\n",
    );
    h.fs.add_file(
        "/evo/config/languages/big.toml",
        &vec![b'#'; usize::try_from(eve_core::LANGUAGE_FILE_MAX_BYTES).unwrap() + 1],
    );
    for i in 0..=eve_core::LANGUAGE_FILES_MAX {
        h.fs.add_file(
            &format!("/evo/config/languages/l{i:03}.toml"),
            format!("[[language]]\nname = \"l{i}\"\n").as_bytes(),
        );
    }
    let (defs, reports) = eve_core::read_language_dir(&*h.fs, &FsPath::new(LANGS));
    assert!(defs.iter().all(|d| d.name != "up"), ".TOML は読まない");
    let kinds: Vec<(String, eve_config::DiagKind)> = reports
        .iter()
        .flat_map(|(f, ds)| ds.iter().map(|d| (f.clone(), d.kind.clone())))
        .collect();
    assert!(kinds.contains(&(
        "languages/big.toml".to_owned(),
        eve_config::DiagKind::Language(eve_config::LanguageProblem::FileTooLarge)
    )));
    assert!(kinds.contains(&(
        "languages/".to_owned(),
        eve_config::DiagKind::Language(eve_config::LanguageProblem::TooManyFiles)
    )));
    // 名前の順に 256 ファイル (big.toml を含む) まで読む
    assert_eq!(defs.len(), eve_core::LANGUAGE_FILES_MAX - 1);
    assert_eq!(defs.last().map(|d| d.name.as_str()), Some("l254"));
}

#[test]
fn 同じ組の警告は_1_回だけ() {
    let mut h = start(
        &[
            (
                "/evo/config/languages/a.toml",
                b"[[language]]\nname = \"a\"\nfile_types = [\"foo\"]\n[[language]]\nname = \"b\"\nfile_types = [\"foo\"]\n",
            ),
            ("/w/x.foo", b""),
            ("/w/y.foo", b""),
        ],
        &[],
        &["/w/x.foo", "/w/y.foo"],
    );
    h.open("/w/y.foo");
    h.run_jobs();
    assert_eq!(language(&h), "b");
    h.cmd("config.show_diagnostics");
    let f = h.frame();
    let OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    let n = items
        .iter()
        .filter(|i| cell_text(&i.label).contains("両方に当たる"))
        .count();
    assert_eq!(n, 1);
}

#[test]
fn 言語定義のファイルの由来で_enter_を押すとそのファイルを開く() {
    let mut h = start(
        &[
            (
                "/evo/config/languages/r.toml",
                b"[[language]]\nname = \"rust\"\nindent = { width = 2 }\n",
            ),
            ("/w/a.rs", b""),
        ],
        &[],
        &["/w/a.rs"],
    );
    let (_, rows) = show(&mut h, "indent_width");
    assert_eq!(
        rows,
        [(
            "editor.indent_width = 2".to_owned(),
            "言語 rust: languages/r.toml:3".to_owned()
        )]
    );
    h.cmd("config.show");
    h.text("indent_width");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(
        h.doc_path().as_deref(),
        Some(
            FsPath::new("/evo/config/languages/r.toml")
                .display()
                .as_str()
        )
    );
}
