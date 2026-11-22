//! 条件付きセクション (docs/07 §3.4 の COND-1〜COND-8、01 NFR-405) の解決・由来・診断。
//!
//! 表駆動の試験 (条件の値 × 同じ設定ファイル → 解決した値と由来) と、proptest で参照モデル「条件の無い表に、真の条件の
//! セクションを COND-3 の順にキーごとに上書きする」との一致を確かめる。

use eve_config::{
    Condition, ConditionProblem, Conditions, Config, DiagKind, Diagnostic, EnvConditions, Frontend,
    Origin, Os, Severity, Source, load_keymap_str, load_str, load_str_for, resolve,
};
use proptest::prelude::*;

fn on(os: Os, ssh: bool, tmux: bool, wsl: bool) -> Conditions {
    Conditions {
        os: Some(os),
        frontend: Frontend::Tui,
        env: EnvConditions { ssh, tmux, wsl },
    }
}

fn plain(os: Os) -> Conditions {
    on(os, false, false, false)
}

fn src(name: &str, text: &str) -> Source {
    Source {
        name: name.to_owned(),
        text: Ok(Some(text.to_owned())),
    }
}

/// 共有する設定ファイルの例 (07 §3.4 の例に、`unix` と `env` の重なりを足したもの)。
const SHARED: &str = r#"[editor]
tab_width = 4
indent_width = 4

[os.windows.file]
default_line_ending = "crlf"

[os.unix.editor]
indent_width = 2

[os.linux.view]
ambiguous_width = 2

[os.macos.editor]
indent_width = 3

[env.ssh.clipboard]
osc52 = "always"

[env.tmux.editor]
tab_width = 8

[env.wsl.editor]
tab_width = 6
"#;

/// (条件, `tab_width`, `indent_width`, 改行, 曖昧幅, `osc52`) の期待。
#[allow(clippy::type_complexity)]
fn expected() -> Vec<(Conditions, u8, u8, &'static str, u8, &'static str)> {
    vec![
        (plain(Os::Windows), 4, 4, "crlf", 1, "auto"),
        (plain(Os::Linux), 4, 2, "lf", 2, "auto"),
        // 同じ OS では具体的な名前 (`macos`) が `unix` に勝つ
        (plain(Os::Macos), 4, 3, "lf", 1, "auto"),
        (
            on(Os::Windows, true, false, false),
            4,
            4,
            "crlf",
            1,
            "always",
        ),
        (on(Os::Linux, false, true, false), 8, 2, "lf", 2, "auto"),
        // `env` は ssh → tmux → wsl の順に重ね、後が勝つ
        (on(Os::Linux, true, true, true), 6, 2, "lf", 2, "always"),
        (on(Os::Linux, false, false, true), 6, 2, "lf", 2, "auto"),
        (on(Os::Macos, true, true, false), 8, 3, "lf", 1, "always"),
    ]
}

fn line_ending(c: &Config) -> &'static str {
    match c.default_line_ending {
        eve_config::DefaultLineEnding::Lf => "lf",
        eve_config::DefaultLineEnding::Crlf => "crlf",
        eve_config::DefaultLineEnding::Cr => "cr",
        eve_config::DefaultLineEnding::Native => "native",
    }
}

fn osc52(c: &Config) -> &'static str {
    match c.osc52 {
        eve_config::Osc52Policy::Auto => "auto",
        eve_config::Osc52Policy::Always => "always",
        eve_config::Osc52Policy::Never => "never",
    }
}

// verifies: AC-NFR-405-1
#[test]
fn 同じファイルを条件ごとに読むと真の条件のセクションだけが効く() {
    for (c, tab, indent, eol, amb, osc) in expected() {
        let l = load_str_for(Config::default(), SHARED, c);
        assert!(l.diagnostics.is_empty(), "{c:?}: {:?}", l.diagnostics);
        let got = (
            l.config.tab_width,
            l.config.indent_width,
            line_ending(&l.config),
            l.config.ambiguous_width,
            osc52(&l.config),
        );
        assert_eq!(got, (tab, indent, eol, amb, osc), "{c:?}");
    }
}

/// 同じキーをすべての条件に別の値で書いたファイル (書く順は仕様の重ねる順の逆。`tab_width` はすべての条件、
/// `indent_width` は `os` だけ、`mouse_wheel_lines` は `env` だけ)。
const EVERY: &str = r#"[editor]
tab_width = 1
indent_width = 1
[ui]
mouse_wheel_lines = 1
[env.wsl.editor]
tab_width = 9
[env.wsl.ui]
mouse_wheel_lines = 9
[env.tmux.editor]
tab_width = 8
[env.tmux.ui]
mouse_wheel_lines = 8
[env.ssh.editor]
tab_width = 7
[env.ssh.ui]
mouse_wheel_lines = 7
[frontend.tui.editor]
tab_width = 6
[os.linux.editor]
tab_width = 5
indent_width = 5
[os.macos.editor]
tab_width = 4
indent_width = 4
[os.windows.editor]
tab_width = 3
indent_width = 3
[os.unix.editor]
tab_width = 2
indent_width = 2
"#;

// verifies: AC-NFR-405-1
#[test]
fn 同じキーをすべての条件に書くと仕様の順で後の条件が勝つ() {
    // (条件, tab_width, indent_width, mouse_wheel_lines)
    let cases = [
        // frontend.tui は v0.5 では常に真なので、env が無ければ tab_width は frontend の値
        (plain(Os::Windows), 6, 3, 1),
        // 具体的な OS (linux / macos) が os.unix に勝つ
        (plain(Os::Linux), 6, 5, 1),
        (plain(Os::Macos), 6, 4, 1),
        // env は frontend に勝ち、ssh → tmux → wsl の順に重なる
        (on(Os::Linux, true, false, false), 7, 5, 7),
        (on(Os::Linux, true, true, false), 8, 5, 8),
        (on(Os::Linux, true, false, true), 9, 5, 9),
        (on(Os::Linux, false, true, true), 9, 5, 9),
        (on(Os::Macos, false, true, false), 8, 4, 8),
        (on(Os::Windows, true, true, true), 9, 3, 9),
    ];
    for (c, tab, indent, wheel) in cases {
        let l = load_str_for(Config::default(), EVERY, c);
        assert!(l.diagnostics.is_empty(), "{c:?}: {:?}", l.diagnostics);
        assert_eq!(
            (
                l.config.tab_width,
                l.config.indent_width,
                l.config.mouse_wheel_lines
            ),
            (tab, indent, wheel),
            "{c:?}"
        );
    }
}

// verifies: AC-NFR-405-1
#[test]
fn 値の由来は書いた行と条件を示す() {
    let r = resolve(
        &[src("config.toml", SHARED)],
        &[],
        on(Os::Linux, false, true, false),
    );
    let user = |line, column, condition| Origin::User {
        source: "config.toml".to_owned(),
        line,
        column,
        condition,
    };
    // `[env.tmux.editor]` の tab_width (21 行目) が `[editor]` の値に勝つ
    assert_eq!(
        r.origins.of("editor.tab_width"),
        &user(21, 1, Some(Condition::EnvTmux))
    );
    assert_eq!(
        r.origins.of("editor.indent_width"),
        &user(9, 1, Some(Condition::OsUnix))
    );
    assert_eq!(
        r.origins.of("view.ambiguous_width"),
        &user(12, 1, Some(Condition::OsLinux))
    );
    // 偽の条件 (`os.windows`) の値は使わないので、由来は既定
    assert_eq!(r.origins.of("file.default_line_ending"), &Origin::Default);
    assert_eq!(r.origins.of("ui.mouse"), &Origin::Default);
    // 条件の無い値は条件なし
    let r = resolve(&[src("config.toml", SHARED)], &[], plain(Os::Windows));
    assert_eq!(r.origins.of("editor.tab_width"), &user(2, 1, None));
}

// verifies: AC-NFR-405-1
#[test]
fn 層の順は変えず_ユーザー層の条件付きの値は_set_に負ける() {
    let file = src("config.toml", "[os.windows.editor]\ntab_width = 2\n");
    let r = resolve(
        std::slice::from_ref(&file),
        &["editor.tab_width=3".into()],
        plain(Os::Windows),
    );
    assert_eq!(r.config.tab_width, 3);
    assert_eq!(
        r.origins.of("editor.tab_width"),
        &Origin::Cli { condition: None }
    );
    // 後のファイル (`--config` の 2 つ目) の条件の無い値は、前のファイルの条件付きの値に勝つ
    let r = resolve(
        &[file.clone(), src("b.toml", "[editor]\ntab_width = 5\n")],
        &[],
        plain(Os::Windows),
    );
    assert_eq!(r.config.tab_width, 5);
    // `--set` の条件付きのキーは、条件が真のときだけ効く
    let set = ["os.linux.editor.tab_width=7".to_owned()];
    let r = resolve(std::slice::from_ref(&file), &set, plain(Os::Linux));
    assert_eq!(r.config.tab_width, 7);
    assert_eq!(
        r.origins.of("editor.tab_width"),
        &Origin::Cli {
            condition: Some(Condition::OsLinux)
        }
    );
    let r = resolve(std::slice::from_ref(&file), &set, plain(Os::Windows));
    assert_eq!(r.config.tab_width, 2);
    assert!(r.reports.is_empty(), "{:?}", r.reports);
}

// verifies: AC-NFR-405-1
#[test]
fn 条件付きの値の型は条件の無い表と同じ規則で重ねる() {
    // 表はキーごと (下のキーは残る)、範囲外は下の値のまま
    let text = "[editor]\ntab_width = 8\nindent_width = 2\n[os.linux.editor]\ntab_width = 99\nindent_unit = \"tab\"\n";
    let l = load_str_for(Config::default(), text, plain(Os::Linux));
    assert_eq!(
        (l.config.tab_width, l.config.indent_width),
        (8, 2),
        "範囲外は下の値のまま"
    );
    assert_eq!(l.config.indent_unit, eve_config::IndentUnit::Tab);
    assert_eq!(l.diagnostics.len(), 1, "{:?}", l.diagnostics);
    assert!(matches!(l.diagnostics[0].kind, DiagKind::OutOfRange { .. }));
    assert_eq!(l.diagnostics[0].condition, Some(Condition::OsLinux));
    // 点のキーの書き方も同じ
    let l = load_str_for(
        Config::default(),
        "os.windows.editor.tab_width = 2\n",
        plain(Os::Windows),
    );
    assert_eq!(l.config.tab_width, 2);
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
}

/// 診断 (キー, 重さ, 種類, 条件) の並び。
fn summary(diags: &[Diagnostic]) -> Vec<(&str, Severity, &DiagKind, Option<Condition>)> {
    diags
        .iter()
        .map(|d| (d.key.as_str(), d.severity, &d.kind, d.condition))
        .collect()
}

// verifies: AC-NFR-405-2
#[test]
fn 偽の条件のセクションも検証して診断を出す() {
    let text = "[os.linux.editor]\ntab_witdh = 2\nindent_width = \"2\"\n[os.linux.ai]\nx = 1\n";
    let l = load_str_for(Config::default(), text, plain(Os::Windows));
    assert_eq!(l.config, Config::default(), "偽の条件の値は使わない");
    // 並びは条件の無い表と同じ (表の中の名前の順)
    assert_eq!(
        summary(&l.diagnostics),
        [
            // 公開前のセクションは、条件の中でも情報
            (
                "ai",
                Severity::Info,
                &DiagKind::Unsupported,
                Some(Condition::OsLinux)
            ),
            (
                "editor.indent_width",
                Severity::Warning,
                &DiagKind::TypeMismatch {
                    expected: "integer",
                    found: "string"
                },
                Some(Condition::OsLinux)
            ),
            (
                "editor.tab_witdh",
                Severity::Warning,
                &DiagKind::UnknownKey {
                    suggestion: Some("editor.tab_width".to_owned())
                },
                Some(Condition::OsLinux)
            ),
        ]
    );
    assert_eq!((l.diagnostics[2].line, l.diagnostics[2].column), (2, 1));
    // 真の条件でも同じ診断を 1 回だけ出す (重ねるときに同じ診断を積まない)
    let l = load_str_for(Config::default(), text, plain(Os::Linux));
    assert_eq!(l.diagnostics.len(), 3, "{:?}", l.diagnostics);
}

// verifies: AC-NFR-405-2
#[test]
fn 条件の書き方の誤りはそのセクションを使わずに警告する() {
    let cases: [(&str, &str, ConditionProblem); 9] = [
        (
            "[os.window.editor]\ntab_width = 2\n",
            "os.window",
            ConditionProblem::Unknown {
                suggestion: Some("os.windows"),
            },
        ),
        (
            "[os.win.editor]\ntab_width = 2\n",
            "os.win",
            // 仕様の例 ([os.win]): 書いた名前が接頭辞の名前を候補にする (テーマの候補と同じ規則)
            ConditionProblem::Unknown {
                suggestion: Some("os.windows"),
            },
        ),
        (
            "[frontend.web.editor]\ntab_width = 2\n",
            "frontend.web",
            ConditionProblem::Unknown { suggestion: None },
        ),
        // 名前は小文字だけ。大文字小文字を区別せずに候補を探す
        (
            "[os.LINUX.editor]\ntab_width = 2\n",
            "os.LINUX",
            ConditionProblem::Unknown {
                suggestion: Some("os.linux"),
            },
        ),
        (
            "[os.windows.env.ssh.editor]\ntab_width = 2\n",
            "os.windows.env.ssh",
            ConditionProblem::Nested,
        ),
        (
            "[os.windows]\ntab_width = 2\n",
            "os.windows.tab_width",
            ConditionProblem::BareKey,
        ),
        ("os.windows = 2\n", "os.windows", ConditionProblem::BareKey),
        (
            "[[os.windows.language]]\nname = \"x\"\n",
            "os.windows.language",
            ConditionProblem::UnsupportedTable,
        ),
        (
            "[os.windows.keys.editor]\n\"ctrl+s\" = \"file.save\"\n",
            "os.windows.keys",
            ConditionProblem::UnsupportedTable,
        ),
    ];
    for (text, key, problem) in cases {
        let l = load_str_for(Config::default(), text, plain(Os::Windows));
        assert_eq!(l.config, Config::default(), "{text}");
        assert_eq!(
            summary(&l.diagnostics),
            [(
                key,
                Severity::Warning,
                &DiagKind::Condition(problem.clone()),
                None
            )],
            "{text}"
        );
        // 位置は誤りの名前を書いた所 (直下のキーはそのキーの行)
        let line = if key.ends_with("tab_width") { 2 } else { 1 };
        assert_eq!(l.diagnostics[0].line, line, "{text}");
    }
    // 同じ条件の別のセクションは使う
    let l = load_str_for(
        Config::default(),
        "[os.windows]\ntab_width = 2\n[os.windows.view]\nambiguous_width = 2\n",
        plain(Os::Windows),
    );
    assert_eq!(l.config.ambiguous_width, 2);
    assert_eq!(l.diagnostics.len(), 1);
}

// verifies: AC-NFR-405-2
#[test]
fn 条件付きセクションで選んだテーマと言語の後の診断にも条件を添える() {
    let text = "[os.linux.theme]\nname = \"nothere\"\n[os.linux.ui]\nlocale = \"fr\"\n";
    let mut r = resolve(&[src("config.toml", text)], &[], plain(Os::Linux));
    let at = r.theme_name_at.clone().expect("theme.name を書いた所");
    assert_eq!(
        (at.source.as_str(), at.line, at.condition),
        ("config.toml", 2, Some(Condition::OsLinux))
    );
    // テーマが無い (07 §8.7)
    let (_, reports) = eve_config::theme_file::load_theme_at("nothere", Some(&at), &|_| Ok(None));
    let d = &reports[0].1[0];
    assert!(matches!(d.kind, DiagKind::ThemeNotFound { .. }), "{d:?}");
    assert_eq!((d.line, d.condition), (2, Some(Condition::OsLinux)));
    // 使える言語に当たらない (07 §17.4)
    eve_config::check_ui_locale(&mut r, &[], "", eve_config::Locale::Ja);
    let d = r
        .reports
        .iter()
        .flat_map(|(_, ds)| ds)
        .find(|d| d.key == "ui.locale")
        .expect("言語の診断");
    assert_eq!((d.line, d.condition), (4, Some(Condition::OsLinux)));
    // 条件の無い表で選んだときは添えない
    let r = resolve(
        &[src("config.toml", "[theme]\nname = \"nothere\"\n")],
        &[],
        plain(Os::Linux),
    );
    assert_eq!(r.theme_name_at.expect("書いた所").condition, None);
}

// verifies: AC-NFR-405-2
#[test]
fn 入れ子は入れ子の条件の名前ごとに_1_件警告する() {
    let text = "[os.windows.env.ssh.editor]\ntab_width = 2\n[os.windows.env.tmux.editor]\ntab_width = 3\n[os.windows.frontend.tui.view]\nwrap = \"soft\"\n";
    let l = load_str_for(Config::default(), text, on(Os::Windows, true, true, false));
    assert_eq!(l.config, Config::default(), "入れ子のセクションは使わない");
    let got: Vec<(&str, u32)> = l
        .diagnostics
        .iter()
        .map(|d| {
            assert_eq!(d.kind, DiagKind::Condition(ConditionProblem::Nested));
            (d.key.as_str(), d.line)
        })
        .collect();
    // 表の中の名前の順 (env の下は ssh → tmux)。位置はそれぞれの見出し
    assert_eq!(
        got,
        [
            ("os.windows.env.ssh", 1),
            ("os.windows.env.tmux", 3),
            ("os.windows.frontend.tui", 5),
        ]
    );
}

// verifies: AC-NFR-405-2
#[test]
fn host_と_env_ci_はこのバージョンでは未対応の情報診断() {
    let text = "[host.box.editor]\ntab_width = 2\n[host.other.view]\nwrap = \"soft\"\n[env.ci.editor]\ntab_width = 3\n";
    let all = on(Os::Linux, true, true, true);
    let l = load_str_for(Config::default(), text, all);
    assert_eq!(l.config, Config::default());
    let keys: Vec<(&str, Severity, &DiagKind)> = l
        .diagnostics
        .iter()
        .map(|d| (d.key.as_str(), d.severity, &d.kind))
        .collect();
    // 条件の名前ごとに 1 件 (`host` は中の名前に依らず 1 件)
    assert_eq!(
        keys,
        [
            ("env.ci", Severity::Info, &DiagKind::Unsupported),
            ("host", Severity::Info, &DiagKind::Unsupported),
        ]
    );
}

// verifies: AC-NFR-405-2
#[test]
fn keymap_toml_の条件付きセクションは専用の警告で_ほかの割当は読む() {
    let km = load_keymap_str(
        "[os.macos.keys.editor]\n\"super+s\" = \"file.save\"\n[env.ssh]\nx = 1\n[host.a]\n[keys.editor]\n\"f5\" = \"file.save\"\n",
    );
    assert_eq!(km.entries.len(), 1, "{:?}", km.entries);
    assert_eq!(km.entries[0].keys, "f5");
    let got: Vec<(&str, Severity, &DiagKind)> = km
        .diagnostics
        .iter()
        .map(|d| (d.key.as_str(), d.severity, &d.kind))
        .collect();
    let w = DiagKind::Condition(ConditionProblem::InKeymap);
    assert_eq!(
        got,
        [
            ("env", Severity::Warning, &w),
            ("host", Severity::Warning, &w),
            ("os", Severity::Warning, &w),
        ]
    );
}

// verifies: AC-NFR-405-1
#[test]
fn 条件付きセクションを書かない設定は条件に依らず同じに読む() {
    let text = "[editor]\ntab_width = 3\ntab_witdh = 1\n[view]\nwrap = \"soft\"\n[ai]\nx = 1\n";
    let base = load_str(text);
    for c in [
        plain(Os::Windows),
        plain(Os::Linux),
        plain(Os::Macos),
        on(Os::Linux, true, true, true),
    ] {
        assert_eq!(load_str_for(Config::default(), text, c), base, "{c:?}");
    }
    // 由来は条件なし
    let r = resolve(&[src("config.toml", text)], &[], plain(Os::Linux));
    assert_eq!(r.origins.of("editor.tab_width").condition(), None);
}

/// 試す公開キー (整数で、範囲 1〜16 か 1〜20 に収まる値だけを書く)。
const KEYS: [(&str, &str); 3] = [
    ("editor", "tab_width"),
    ("editor", "indent_width"),
    ("ui", "mouse_wheel_lines"),
];

fn value_of(c: &Config, key: usize) -> u8 {
    match key {
        0 => c.tab_width,
        1 => c.indent_width,
        _ => c.mouse_wheel_lines,
    }
}

/// 1 つのファイルの中で重ねる順 (07 §3.4 の COND-3 から写した名前。実装の `Condition` の並びに頼らない)。
const SPEC_ORDER: [&str; 8] = [
    "os.unix",
    "os.windows",
    "os.macos",
    "os.linux",
    "frontend.tui",
    "env.ssh",
    "env.tmux",
    "env.wsl",
];

/// 条件の名前が真か (07 §3.4 の表と COND-2 から写した規則。実装の `Conditions::holds` に頼らない)。
fn spec_holds(name: &str, os: Os, env: EnvConditions) -> bool {
    match name {
        "os.unix" => matches!(os, Os::Macos | Os::Linux),
        "os.windows" => os == Os::Windows,
        "os.macos" => os == Os::Macos,
        "os.linux" => os == Os::Linux,
        "frontend.tui" => true,
        "env.ssh" => env.ssh,
        "env.tmux" => env.tmux,
        "env.wsl" => env.wsl,
        other => panic!("仕様に無い条件 {other}"),
    }
}

/// 表 (条件なしが `None`、条件は名前) ごとに、キーの番号 → 値。
type Tables = Vec<(Option<&'static str>, Vec<(usize, u8)>)>;

fn tables() -> impl Strategy<Value = Tables> {
    let entries = proptest::collection::vec((0usize..KEYS.len(), 1u8..=16), 0..4);
    let cond = proptest::option::of(proptest::sample::select(SPEC_ORDER.to_vec()));
    proptest::collection::vec((cond, entries), 0..8)
}

fn conditions() -> impl Strategy<Value = Conditions> {
    (
        proptest::sample::select(vec![Os::Windows, Os::Macos, Os::Linux]),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(os, ssh, tmux, wsl)| on(os, ssh, tmux, wsl))
}

/// 表を TOML にする。同じ表 (と同じキー) は TOML に 2 回書けないので、後に書いたものだけを残す。
fn to_toml(tables: &Tables) -> (String, Tables) {
    let mut merged: Tables = Vec::new();
    for (c, entries) in tables {
        let slot = match merged.iter_mut().find(|(m, _)| m == c) {
            Some(s) => s,
            None => {
                merged.push((*c, Vec::new()));
                merged.last_mut().expect("今足した")
            }
        };
        for (k, v) in entries {
            slot.1.retain(|(x, _)| x != k);
            slot.1.push((*k, *v));
        }
    }
    let mut text = String::new();
    for (c, entries) in &merged {
        for (k, v) in entries {
            let (section, name) = KEYS[*k];
            let prefix = c.map_or(String::new(), |c| format!("{c}."));
            text.push_str(&format!("{prefix}{section}.{name} = {v}\n"));
        }
    }
    (text, merged)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // verifies: AC-NFR-405-1
    #[test]
    fn 解決は条件の無い表に真の条件のセクションを順に上書きした結果と一致する(
        t in tables(),
        c in conditions(),
    ) {
        let (text, merged) = to_toml(&t);
        let l = load_str_for(Config::default(), &text, c);
        prop_assert!(l.diagnostics.is_empty(), "{text}\n{:?}", l.diagnostics);
        // 参照モデル: 条件の無い表 → 真の条件のセクションを仕様の順 (`SPEC_ORDER`) に、キーごとに上書き
        let mut want: [(u8, Option<Option<&str>>); 3] = [
            (4, None),
            (4, None),
            (3, None),
        ];
        let os = c.os.expect("試す条件は 3 つの OS のどれか");
        let order = std::iter::once(None)
            .chain(SPEC_ORDER.into_iter().filter(|n| spec_holds(n, os, c.env)).map(Some));
        for cond in order {
            for (k, v) in merged.iter().filter(|(m, _)| *m == cond).flat_map(|(_, e)| e) {
                want[*k] = (*v, Some(cond));
            }
        }
        let r = resolve(&[src("c.toml", &text)], &[], c);
        for (k, (value, from)) in want.iter().enumerate() {
            prop_assert_eq!(value_of(&l.config, k), *value, "{}\n{:?}", text, c);
            let key = format!("{}.{}", KEYS[k].0, KEYS[k].1);
            match from {
                None => prop_assert_eq!(r.origins.of(&key), &Origin::Default),
                Some(cond) => prop_assert_eq!(
                    r.origins.of(&key).condition().map(Condition::name),
                    *cond,
                    "{}",
                    key
                ),
            }
        }
    }
}
