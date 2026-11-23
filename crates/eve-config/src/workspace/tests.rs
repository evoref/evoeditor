//! ワークスペース層の読込と分類の試験 (07 §12.3 / §12.5)。

use super::*;
use crate::{
    Conditions, DocLayers, ExternalChangePolicy, IndentUnit, LanguageSet, ListItemProblem, Origin,
    UserLayer, ValueOrigin, compose, doc_values, load_str,
};
use proptest::prelude::*;

/// 読んで、下の層 `lower` と比べて分類した結果。
struct Layered {
    layer: WorkspaceLayer,
    applied: Vec<Written>,
    diagnostics: Vec<Diagnostic>,
    languages: Vec<LanguageDef>,
}

impl Layered {
    fn has_error(&self) -> bool {
        self.layer.has_error()
    }
}

fn ws(text: &str, lower: &Config) -> Layered {
    let layer = load_workspace_str(text, Conditions::default());
    let c = layer.classify(lower);
    Layered {
        languages: layer.languages.clone(),
        layer,
        applied: c.applied,
        diagnostics: c.diagnostics,
    }
}

fn problems(l: &Layered) -> Vec<(String, DiagKind)> {
    l.diagnostics
        .iter()
        .map(|d| (d.key.clone(), d.kind.clone()))
        .collect()
}

fn applied(l: &Layered) -> Vec<&'static str> {
    l.applied.iter().map(|w| w.key).collect()
}

fn layered(l: &Layered, lower: &Config) -> Config {
    let mut c = lower.clone();
    l.layer.apply_to(&mut c, &l.applied);
    c
}

// verifies: AC-FR-705-1
#[test]
fn 安全な項目だけを重ね_準安全と危険は使わない() {
    let lower = Config::default();
    let l = ws(
        "[editor]\ntab_width = 2\n[view]\nwrap = \"soft\"\n[theme]\nname = \"x\"\n\
         [file.swap]\nexclude_secret_files = false\n",
        &lower,
    );
    assert_eq!(applied(&l), ["editor.tab_width", "view.wrap"]);
    let c = layered(&l, &lower);
    assert_eq!((c.tab_width, c.wrap), (2, crate::WrapMode::Soft));
    assert_eq!(c.theme_name, "default", "準安全は使わない");
    assert!(c.swap_exclude_secret_files, "危険は使わない");
    assert_eq!(
        problems(&l),
        [
            (
                "theme.name".to_owned(),
                DiagKind::Workspace(WorkspaceProblem::Pending)
            ),
            (
                "file.swap.exclude_secret_files".to_owned(),
                DiagKind::Workspace(WorkspaceProblem::PendingUnsafe)
            ),
        ]
    );
    // 危険の項目は書かれた値を示す。どれも情報
    assert_eq!(l.diagnostics[1].value, "false");
    assert!(l.diagnostics.iter().all(|d| d.severity == Severity::Info));
    assert_eq!((l.diagnostics[0].line, l.diagnostics[0].column), (6, 1));
}

// verifies: AC-FR-705-2
#[test]
fn 常に無視と効かないの項目は使わずに知らせる() {
    let lower = Config::default();
    let l = ws(
        "[workspace]\nsafe_directories = [\"/tmp\"]\nroot_markers = [\"x\"]\n\
         [view]\nambiguous_width = 2\n[ui]\nlocale = \"en\"\n[theme]\ncolor_mode = \"16\"\n",
        &lower,
    );
    assert!(l.applied.is_empty(), "{:?}", l.applied);
    let kinds: Vec<(String, Severity)> = l
        .diagnostics
        .iter()
        .map(|d| (d.key.clone(), d.severity))
        .collect();
    // 安全な場所は要素の診断を出さず (Windows では `/tmp` は絶対パスでない)、常に無視の 1 件だけ
    let want = [
        ("workspace.safe_directories".to_owned(), Severity::Warning),
        ("workspace.root_markers".to_owned(), Severity::Info),
        ("view.ambiguous_width".to_owned(), Severity::Info),
        ("ui.locale".to_owned(), Severity::Info),
        ("theme.color_mode".to_owned(), Severity::Info),
    ];
    assert_eq!(kinds, want);
    assert!(
        l.diagnostics
            .iter()
            .any(|d| d.kind == DiagKind::Workspace(WorkspaceProblem::Ignored)
                && d.key == "workspace.safe_directories")
    );
    assert_eq!(
        l.diagnostics
            .iter()
            .filter(|d| d.kind == DiagKind::Workspace(WorkspaceProblem::NoEffect))
            .count(),
        4
    );
}

// verifies: AC-FR-705-2
#[test]
fn 方向と値で分ける項目は下の層と比べる() {
    let lower = Config {
        huge_file_warn_bytes: 100 << 20,
        ..Config::default()
    };
    let cases: [(&str, bool); 14] = [
        ("file.huge_file_warn_bytes = 104857600", true),
        ("file.huge_file_warn_bytes = 10485760", true),
        ("file.huge_file_warn_bytes = 209715200", false),
        ("search.workspace.max_file_size = 1024", true),
        ("search.workspace.max_file_size = 4194304", false),
        ("file.swap.interval_ms = 2000", true),
        ("file.swap.interval_ms = 1000", false),
        ("file.swap.max_bytes = 1048576", false),
        ("file.swap.enabled = true", true),
        ("file.swap.enabled = false", false),
        ("search.workspace.respect_gitignore = false", false),
        ("search.workspace.follow_symlinks = true", false),
        ("editor.undo.total_max_bytes = 268435456", false),
        ("clipboard.osc52 = \"never\"", false),
    ];
    for (src, safe) in cases {
        let l = ws(&format!("{src}\n"), &lower);
        assert_eq!(l.applied.len() == 1, safe, "{src}: {:?}", l.diagnostics);
        assert_eq!(l.diagnostics.is_empty(), safe, "{src}");
    }
    // 外に触れる・秘密を写す値は危険 (値を示す)
    for src in [
        "search.workspace.follow_symlinks = true",
        "file.swap.exclude_secret_files = false",
    ] {
        let l = ws(&format!("{src}\n"), &lower);
        assert_eq!(
            l.diagnostics[0].kind,
            DiagKind::Workspace(WorkspaceProblem::PendingUnsafe),
            "{src}"
        );
    }
}

#[test]
fn 外部変更の扱いは_ask_と同じ値だけが安全() {
    use ExternalChangePolicy as E;
    for (lower, value, safe) in [
        (E::AutoIfUnmodified, "ask", true),
        (E::AutoIfUnmodified, "auto_if_unmodified", true),
        (E::AutoIfUnmodified, "never", false),
        (E::Ask, "auto_if_unmodified", false),
        (E::Ask, "never", false),
        (E::Never, "never", true),
        (E::Never, "auto_if_unmodified", true),
    ] {
        let lower = Config {
            reload_on_external_change: lower,
            ..Config::default()
        };
        let l = ws(
            &format!("file.reload_on_external_change = \"{value}\"\n"),
            &lower,
        );
        assert_eq!(l.applied.len() == 1, safe, "{lower:?} → {value}");
    }
}

#[test]
fn 除外はワークスペース層の行を下の層の行の前に足す() {
    let lower = Config {
        exclude: vec!["*.log".to_owned()],
        ..Config::default()
    };
    // 置換の書き方でも足す。`+` も同じ
    for src in [
        "workspace.exclude = [\"dist/\", \"!keep.log\"]\n",
        "workspace.\"exclude+\" = [\"dist/\", \"!keep.log\"]\n",
        "[workspace]\nexclude = [\"dist/\"]\n\"exclude+\" = [\"!keep.log\"]\n",
    ] {
        let l = ws(src, &lower);
        assert!(l.diagnostics.is_empty(), "{src}: {:?}", l.diagnostics);
        assert_eq!(
            layered(&l, &lower).exclude,
            ["dist/", "!keep.log", "*.log"],
            "{src}"
        );
    }
    // 空の配列は何も消さない
    let l = ws("workspace.exclude = []\n", &lower);
    assert_eq!(layered(&l, &lower).exclude, ["*.log"]);
}

#[test]
fn ユーザー層の配列は置換と追加の両方を書ける() {
    let c = load_str("[workspace]\nexclude = [\"a\"]\n\"exclude+\" = [\"b\"]\n").config;
    assert_eq!(c.exclude, ["a", "b"]);
    let c = load_str("workspace.\"exclude+\" = [\"c\"]\n").config;
    assert_eq!(c.exclude, ["**/.git", "**/node_modules", "**/target", "c"]);
    // 配列でないキーの `+` は未知のキー
    let l = load_str("editor.\"tab_width+\" = 1\n");
    assert!(matches!(l.diagnostics[0].kind, DiagKind::UnknownKey { .. }));
}

#[test]
fn 目印と安全な場所の要素を確かめる() {
    let l = load_str("workspace.root_markers = [\".git\", \"a/b\", \"\", \"..\", \"x\\\\y\", 1]\n");
    assert_eq!(l.config.root_markers, [".git"]);
    let kinds: Vec<DiagKind> = l.diagnostics.iter().map(|d| d.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            DiagKind::ListItem(ListItemProblem::NotName),
            DiagKind::ListItem(ListItemProblem::Empty),
            DiagKind::ListItem(ListItemProblem::NotName),
            DiagKind::ListItem(ListItemProblem::NotName),
            DiagKind::TypeMismatch {
                expected: "string",
                found: "integer"
            },
        ]
    );
    assert_eq!(l.diagnostics[0].value, "a/b");
    // 安全な場所: 絶対パスの形は条件の OS の規則で確かめる (COND-4)。OS の決まらないセクションは、真なら今の OS の規則
    let src = "[os.windows.workspace]\nsafe_directories = [\"C:\\\\src\", \"/mnt/c\", \"D:\\\\\", \"*\"]\n\
               [os.linux.workspace]\nsafe_directories = [\"/mnt/c/src\", \"C:\\\\src\", \"/\", \"~/x\"]\n\
               [env.wsl.workspace]\nsafe_directories = [\"anything\"]\n";
    let l = load_str(src);
    let got: Vec<(Option<&str>, ListItemProblem, String)> = l
        .diagnostics
        .iter()
        .map(|d| match &d.kind {
            DiagKind::ListItem(p) => (d.condition.map(crate::Condition::name), *p, d.value.clone()),
            other => panic!("{other:?}"),
        })
        .collect();
    // 並びは表の中の名前の順 (`linux` が `windows` の前)。今の環境で偽の `env.*` のセクションは、パスの形を確かめない
    // (WSL の中で使う値を Windows の規則で誤りにしない)
    assert_eq!(
        got,
        [
            (
                Some("os.linux"),
                ListItemProblem::Relative,
                "C:\\src".to_owned()
            ),
            (Some("os.linux"), ListItemProblem::TooBroad, "/".to_owned()),
            (
                Some("os.linux"),
                ListItemProblem::Relative,
                "~/x".to_owned()
            ),
            (
                Some("os.windows"),
                ListItemProblem::Relative,
                "/mnt/c".to_owned()
            ),
            (
                Some("os.windows"),
                ListItemProblem::TooBroad,
                "D:\\".to_owned()
            ),
            (
                Some("os.windows"),
                ListItemProblem::Wildcard,
                "*".to_owned()
            ),
        ]
    );
    // 広すぎる場所は警告して使う
    let os = if cfg!(windows) { "windows" } else { "linux" };
    let c = crate::load_str_for(Config::default(), src, Conditions::default()).config;
    let want: &[&str] = match os {
        "windows" => &["C:\\src", "D:\\"],
        _ => &["/mnt/c/src", "/"],
    };
    if cfg!(any(windows, target_os = "linux")) {
        assert_eq!(c.safe_directories, want);
    }
    // UNC の共有の直下は広すぎる
    let unc = load_str_for_windows(
        "safe_directories = [\"\\\\\\\\srv\\\\share\", \"\\\\\\\\srv\\\\share\\\\repo\"]",
    );
    assert_eq!(unc, [ListItemProblem::TooBroad]);
}

/// `[os.windows.workspace]` の中の `body` を読み、要素の誤りの種類を返す。
fn load_str_for_windows(body: &str) -> Vec<ListItemProblem> {
    load_str(&format!("[os.windows.workspace]\n{body}\n"))
        .diagnostics
        .iter()
        .filter_map(|d| match d.kind {
            DiagKind::ListItem(p) => Some(p),
            _ => None,
        })
        .collect()
}

// verifies: AC-NFR-502-1
#[test]
fn 外部コマンドと予約のセクションは未対応で使わない() {
    let l = ws(
        "[clipboard]\nprovider = \"command\"\ncommand.copy = [\"sh\", \"-c\", \"x\"]\n\
         [formatter.x]\ncommand = \"rm\"\n[language-server.y]\ncommand = \"z\"\n[plugins]\na = 1\n\
         [keys.editor]\n\"ctrl+q\" = \"app.quit\"\n",
        &Config::default(),
    );
    assert!(l.applied.is_empty(), "{:?}", l.applied);
    assert!(
        l.diagnostics
            .iter()
            .all(|d| d.kind == DiagKind::Unsupported && d.severity == Severity::Info),
        "{:?}",
        l.diagnostics
    );
    let keys: Vec<&str> = l.diagnostics.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "clipboard.provider",
            "clipboard.command",
            "formatter",
            "language-server",
            "plugins",
            "keys"
        ]
    );
}

#[test]
fn 構文エラーのファイルは何も使わない() {
    let l = ws(
        "[editor]\ntab_width = 2\n[[language]]\nname = \"x\"\n[view\n",
        &Config::default(),
    );
    assert!(l.has_error());
    assert!(l.applied.is_empty() && l.languages.is_empty());
}

#[test]
fn 条件付きセクションの中のキーも同じ分類() {
    let l = ws(
        "[os.windows.editor]\ntab_width = 3\n[os.linux.editor]\ntab_width = 5\n\
         [os.windows.theme]\nname = \"x\"\n[os.linux.theme]\nname = \"x\"\n",
        &Config::default(),
    );
    let on = crate::Os::current().is_some();
    assert_eq!(l.applied.len(), usize::from(on), "{:?}", l.applied);
    if let Some(w) = l.applied.first() {
        assert!(w.condition.is_some());
        assert_eq!(
            l.diagnostics[0].kind,
            DiagKind::Workspace(WorkspaceProblem::Pending)
        );
        assert!(l.diagnostics[0].condition.is_some());
    }
}

#[test]
fn 重ねる順はユーザー層_ワークスペース層_set() {
    let user = UserLayer {
        config: Config {
            tab_width: 8,
            ..Config::default()
        },
        ..UserLayer::default()
    };
    let l = ws("[editor]\ntab_width = 2\nindent_width = 3\n", &user.config);
    let c = compose(
        &user,
        Some(("/w/.eve/config.toml", &l.layer)),
        &["editor.indent_width=6".to_owned()],
        Conditions::default(),
    );
    assert_eq!((c.config.tab_width, c.config.indent_width), (2, 6));
    assert!(matches!(
        c.origins.of("editor.tab_width"),
        Origin::Workspace { source, line: 2, .. } if source == "/w/.eve/config.toml"
    ));
    assert!(matches!(
        c.origins.of("editor.indent_width"),
        Origin::Cli { .. }
    ));
    // ワークスペース層を外すとユーザー層に戻る
    let back = compose(&user, None, &[], Conditions::default());
    assert_eq!(back.config.tab_width, 8);
    assert_eq!(back.origins.of("editor.tab_width"), &Origin::Default);
}

#[test]
fn 分類の決め方は全公開キーにある() {
    for key in crate::config::KEYS {
        assert!(crate::workspace_policy(key).is_some(), "{key}");
        let label = crate::workspace_policy(key).map(|p| p.spec_label());
        assert_ne!(label, Some("?"), "{key}");
    }
}

/// 文書ごとの値の段の有無 (`None` は書かない)。
#[derive(Debug, Clone)]
struct Stages {
    general: Option<u8>,
    user_lang: Option<u8>,
    ws_general: Option<u8>,
    ws_lang: Option<u8>,
    set: Option<u8>,
}

fn stage() -> impl Strategy<Value = Option<u8>> {
    prop::option::of(1u8..=16)
}

proptest! {
    // verifies: AC-FR-705-1
    /// §3.6 の LAYER-1: 任意の段の有無の組で、決まる値が「強い段から順に、値を持つ最初の段」と一致する。ワークスペース層の
    /// 段 ((6)(7)) はルートの下の文書だけに効く。
    #[test]
    fn 文書ごとの値はワークスペース層の段を含めて参照モデルと一致する(
        s in (stage(), stage(), stage(), stage(), stage()).prop_map(|(general, user_lang, ws_general, ws_lang, set)| {
            Stages { general, user_lang, ws_general, ws_lang, set }
        }),
        under_root in any::<bool>(),
    ) {
        let mut config = String::new();
        if let Some(v) = s.general {
            config.push_str(&format!("[editor]\nindent_width = {v}\n"));
        }
        if let Some(v) = s.user_lang {
            config.push_str(&format!("[[language]]\nname = \"rust\"\nindent = {{ width = {v} }}\n"));
        }
        let r = crate::resolve(
            &[crate::Source { name: "config.toml".to_owned(), text: Ok(Some(config)) }],
            &[],
            Conditions::default(),
        );
        prop_assert!(r.reports.is_empty(), "{:?}", r.reports);
        let mut wtext = String::new();
        if let Some(v) = s.ws_general {
            wtext.push_str(&format!("[editor]\nindent_width = {v}\n"));
        }
        if let Some(v) = s.ws_lang {
            wtext.push_str(&format!("[[language]]\nname = \"rust\"\nindent = {{ width = {v} }}\n"));
        }
        let layered = ws(&wtext, &r.user.config);
        prop_assert!(layered.diagnostics.is_empty(), "{:?}", layered.diagnostics);
        let layer = layered.layer;
        let sets: Vec<String> = s.set.iter().map(|v| format!("editor.indent_width={v}")).collect();
        let c = compose(&r.user, Some(("ws", &layer)), &sets, Conditions::default());
        let mut wdefs = layer.languages.clone();
        for d in &mut wdefs {
            d.source = "ws".to_owned();
        }
        let (set, _) = LanguageSet::build_layers(&r.languages, &wdefs);
        let v = doc_values(&DocLayers {
            config: &c.config,
            origins: &c.origins,
            user: Some(&r.user),
            language: set.get("rust"),
            workspace: under_root,
        });
        let (ws_lang, ws_general) = if under_root { (s.ws_lang, s.ws_general) } else { (None, None) };
        let want = s.set.or(ws_lang).or(ws_general).or(s.user_lang).or(s.general).unwrap_or(4);
        prop_assert_eq!(v.indent_width.value, want);
        let from_ws = s.set.is_none() && (ws_lang.is_some() || ws_general.is_some());
        let is_ws_origin = matches!(
            &v.indent_width.origin,
            ValueOrigin::Config(Origin::Workspace { .. })
                | ValueOrigin::Language { at: crate::LangOrigin::Workspace { .. }, .. }
        );
        prop_assert_eq!(is_ws_origin, from_ws);
        // 字下げの単位は組込みの `rust` が持たないので層の値のまま
        prop_assert_eq!(v.indent_unit.value, IndentUnit::Space);
    }
}

// verifies: AC-FR-705-2
#[test]
fn 分類は重ねるたびに今のユーザー層と比べる() {
    // 一度読んだ層を、ユーザー層を替えて重ね直す (config.reload でワークスペース層が一時の失敗のとき)
    let layer = load_workspace_str(
        "[file]\nhuge_file_warn_bytes = 104857600\nreload_on_external_change = \"auto_if_unmodified\"\n",
        Conditions::default(),
    );
    let big = Config {
        huge_file_warn_bytes: 200 << 20,
        reload_on_external_change: ExternalChangePolicy::AutoIfUnmodified,
        ..Config::default()
    };
    let keys = |lower: &Config| -> Vec<&'static str> {
        layer
            .classify(lower)
            .applied
            .iter()
            .map(|w| w.key)
            .collect()
    };
    assert_eq!(
        keys(&big),
        [
            "file.huge_file_warn_bytes",
            "file.reload_on_external_change"
        ]
    );
    // ユーザー層が閾値を下げ、`ask` にした: 大きくする向きと緩める向きは使わない
    let small = Config {
        huge_file_warn_bytes: 10 << 20,
        reload_on_external_change: ExternalChangePolicy::Ask,
        ..Config::default()
    };
    assert!(keys(&small).is_empty(), "{:?}", keys(&small));
    let user = UserLayer {
        config: small.clone(),
        ..UserLayer::default()
    };
    let c = compose(&user, Some(("ws", &layer)), &[], Conditions::default());
    assert_eq!(c.config.huge_file_warn_bytes, 10 << 20);
    assert_eq!(
        c.config.reload_on_external_change,
        ExternalChangePolicy::Ask
    );
    assert_eq!(
        c.workspace_diagnostics.len(),
        2,
        "{:?}",
        c.workspace_diagnostics
    );
}

#[test]
fn ワークスペース層の安全な場所は要素の診断を出さない() {
    let l = ws(
        r#"[workspace]
safe_directories = ["/", "rel", 'C:\']
"#,
        &Config::default(),
    );
    assert_eq!(
        problems(&l),
        [(
            "workspace.safe_directories".to_owned(),
            DiagKind::Workspace(WorkspaceProblem::Ignored)
        )]
    );
}

#[test]
fn 診断は_1_つのファイルで上限までにして_ほか_n_件にまとめる() {
    // 256 KiB 近い配列の要素がどれも誤り: 上限の数だけ位置を引き、残りは数える (io のジョブを止めない)
    let items = vec!["1"; 60_000].join(", ");
    let text = format!("workspace.root_markers = [{items}]\n");
    let started = std::time::Instant::now();
    let l = load_str(&text);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(l.diagnostics.len(), crate::CONFIG_MAX_DIAGNOSTICS + 1);
    assert_eq!(
        l.diagnostics.last().map(|d| d.kind.clone()),
        Some(DiagKind::More {
            count: 60_000 - crate::CONFIG_MAX_DIAGNOSTICS
        })
    );
    // 残した診断は書いた位置を持つ
    assert_eq!((l.diagnostics[0].line, l.diagnostics[0].column), (1, 27));
    // keymap.toml も同じ上限
    let keys: String = (0..300).map(|i| format!("\"k{i}\" = 1\n")).collect();
    let k = crate::load_keymap_str(&format!("[keys.editor]\n{keys}"));
    assert_eq!(k.diagnostics.len(), crate::CONFIG_MAX_DIAGNOSTICS + 1);
    assert!(matches!(
        k.diagnostics.last().map(|d| &d.kind),
        Some(DiagKind::More { count: 200 })
    ));
}

#[test]
fn 今の環境で真の_env_のセクションの安全な場所は今の_os_の規則で確かめる() {
    let text = "[env.wsl.workspace]\nsafe_directories = [\"anything\", \"/home/me/src\"]\n";
    // WSL の中の Linux: 真なので Unix の規則
    let wsl = Conditions {
        os: Some(crate::Os::Linux),
        env: crate::EnvConditions {
            wsl: true,
            ..crate::EnvConditions::default()
        },
        ..Conditions::default()
    };
    let l = crate::load_str_for(Config::default(), text, wsl);
    let probs: Vec<(String, DiagKind)> = l
        .diagnostics
        .iter()
        .map(|d| (d.value.clone(), d.kind.clone()))
        .collect();
    assert_eq!(
        probs,
        [(
            "anything".to_owned(),
            DiagKind::ListItem(ListItemProblem::Relative)
        )]
    );
    assert_eq!(l.config.safe_directories, ["/home/me/src"]);
    // Windows (偽): WSL の中で使う値なので、Windows の規則で「相対パス」にしない
    let windows = Conditions {
        os: Some(crate::Os::Windows),
        ..Conditions::default()
    };
    let l = crate::load_str_for(Config::default(), text, windows);
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
}

// verifies: AC-FR-705-3
#[test]
fn ホームディレクトリそのものは広すぎると知らせて使う() {
    let linux = Conditions {
        os: Some(crate::Os::Linux),
        ..Conditions::default()
    };
    let src = |text: &str| crate::Source {
        name: "config.toml".to_owned(),
        text: Ok(Some(text.to_owned())),
    };
    let r = crate::resolve_home(
        &[src(
            "[workspace]\nsafe_directories = [\"/home/me/\", \"/home/me/src\"]\n",
        )],
        &["workspace.\"safe_directories+\"=[\"/home/me\"]".to_owned()],
        linux,
        Some("/home/me"),
    );
    let broad: Vec<(&str, &str)> = r
        .reports
        .iter()
        .flat_map(|(f, ds)| ds.iter().map(move |d| (f.as_str(), d)))
        .filter(|(_, d)| d.kind == DiagKind::ListItem(ListItemProblem::TooBroad))
        .map(|(f, d)| (f, d.value.as_str()))
        .collect();
    assert_eq!(broad, [("config.toml", "/home/me/"), ("--set", "/home/me")]);
    assert_eq!(
        r.config.safe_directories,
        ["/home/me/", "/home/me/src", "/home/me"]
    );
    // ホームを知らなければ知らせない
    let r = crate::resolve(
        &[src("[workspace]\nsafe_directories = [\"/home/me\"]\n")],
        &[],
        linux,
    );
    assert!(r.reports.is_empty(), "{:?}", r.reports);
}

#[test]
fn ワークスペース層の診断は分類を足した合計で上限まで() {
    let mut text: String = (0..120).map(|i| format!("unknown_{i} = 1\n")).collect();
    text.push_str("[theme]\nname = \"x\"\n");
    let l = ws(&text, &Config::default());
    assert_eq!(l.diagnostics.len(), crate::CONFIG_MAX_DIAGNOSTICS + 1);
    assert_eq!(
        l.diagnostics.last().map(|d| d.kind.clone()),
        Some(DiagKind::More { count: 21 })
    );
}

#[test]
fn 上限は保留の知らせを未知のキーより先に残す() {
    // 先頭に未知のキーを 150 個並べ、最後に準安全と危険の項目を書く: 保留の知らせは「ほか N 件」に回さない
    let mut text: String = (0..150).map(|i| format!("unknown_{i} = 1\n")).collect();
    text.push_str("[theme]\nname = \"x\"\n[file.swap]\nexclude_secret_files = false\n");
    let l = ws(&text, &Config::default());
    assert_eq!(l.diagnostics.len(), crate::CONFIG_MAX_DIAGNOSTICS + 1);
    let kinds: Vec<&DiagKind> = l.diagnostics.iter().map(|d| &d.kind).collect();
    assert!(kinds.contains(&&DiagKind::Workspace(WorkspaceProblem::Pending)));
    assert!(kinds.contains(&&DiagKind::Workspace(WorkspaceProblem::PendingUnsafe)));
    assert_eq!(
        kinds.last(),
        Some(&&DiagKind::More { count: 52 }),
        "未知のキー 150 のうち 98 だけ残す"
    );
}

#[test]
fn 相対のホームとは比べない() {
    let linux = Conditions {
        os: Some(crate::Os::Linux),
        ..Conditions::default()
    };
    let r = crate::resolve_home(
        &[crate::Source {
            name: "config.toml".to_owned(),
            text: Ok(Some(
                "[workspace]\nsafe_directories = [\"/me\"]\n".to_owned(),
            )),
        }],
        &[],
        linux,
        Some("me"),
    );
    assert!(r.reports.is_empty(), "{:?}", r.reports);
}
