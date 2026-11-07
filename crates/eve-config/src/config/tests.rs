use super::*;

fn kinds(l: &Loaded) -> Vec<(&str, &DiagKind)> {
    l.diagnostics
        .iter()
        .map(|d| (d.key.as_str(), &d.kind))
        .collect()
}

#[test]
fn 空のファイルは既定値で診断なし() {
    let l = load_str("");
    assert_eq!(l.config, Config::default());
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
}

#[test]
fn 公開キー_7_個を読む() {
    let l = load_str(
        r#"
[editor]
tab_width = 8
indent_unit = "tab"
indent_width = 2
[file]
default_encoding = "shift_jis"
default_line_ending = "crlf"
[view]
ambiguous_width = 2
[theme]
color_mode = "256"
"#,
    );
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    assert_eq!(
        l.config,
        Config {
            tab_width: 8,
            indent_unit: IndentUnit::Tab,
            indent_width: 2,
            default_encoding: DefaultEncoding::ShiftJis,
            default_line_ending: DefaultLineEnding::Crlf,
            ambiguous_width: 2,
            color_mode: ColorMode::Color256,
            ..Config::default()
        }
    );
}

#[test]
fn ドット記法でも読める() {
    let l = load_str("editor.tab_width = 2\n");
    assert_eq!(l.config.tab_width, 2);
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
}

// verifies: AC-FR-703-1
#[test]
fn 構文エラーはファイル全体を無視して行と桁を返す() {
    let l = load_str("[editor]\ntab_width = 2\nfoo bar\n");
    assert_eq!(l.config, Config::default());
    assert_eq!(l.diagnostics.len(), 1);
    let d = &l.diagnostics[0];
    assert_eq!(d.severity, Severity::Error);
    assert!(matches!(d.kind, DiagKind::Syntax { .. }));
    assert_eq!(d.line, 3);
}

// verifies: AC-FR-703-1
#[test]
fn 未知のキーは警告し綴りの候補を出す() {
    let l = load_str("[editor]\ntab_witdh = 2\n");
    assert_eq!(l.config, Config::default());
    assert_eq!(l.diagnostics.len(), 1);
    assert_eq!(l.diagnostics[0].severity, Severity::Warning);
    assert_eq!((l.diagnostics[0].line, l.diagnostics[0].column), (2, 1));
    assert_eq!(
        kinds(&l),
        vec![(
            "editor.tab_witdh",
            &DiagKind::UnknownKey {
                suggestion: Some("editor.tab_width".to_owned())
            }
        )]
    );
}

#[test]
fn 距離が遠いキーには候補を出さない() {
    let l = load_str("[editor]\nzzzzzzzz = 1\n");
    assert_eq!(
        kinds(&l),
        vec![(
            "editor.zzzzzzzz",
            &DiagKind::UnknownKey { suggestion: None }
        )]
    );
}

#[test]
fn 未知のセクションは警告する() {
    let l = load_str("[editr]\ntab_width = 2\n");
    assert_eq!(
        kinds(&l),
        vec![(
            "editr",
            &DiagKind::UnknownKey {
                suggestion: Some("editor".to_owned())
            }
        )]
    );
}

// verifies: AC-FR-703-1
#[test]
fn 型違いは警告して既定値のまま() {
    let l = load_str("[editor]\ntab_width = \"4\"\ntab_width2 = 1\n");
    assert_eq!(l.config.tab_width, 4);
    assert!(matches!(
        l.diagnostics[0].kind,
        DiagKind::TypeMismatch {
            expected: "integer",
            found: "string"
        }
    ));
    assert_eq!(l.diagnostics[0].line, 2);
}

// verifies: AC-FR-703-1
#[test]
fn 範囲外は既定値へ戻す() {
    for src in [
        "editor.tab_width = 0",
        "editor.tab_width = 99",
        "editor.tab_width = -1",
    ] {
        let l = load_str(src);
        assert_eq!(l.config.tab_width, 4, "{src}");
        assert_eq!(
            kinds(&l),
            vec![(
                "editor.tab_width",
                &DiagKind::OutOfRange { min: 1, max: 16 }
            )],
            "{src}"
        );
    }
    let l = load_str("view.ambiguous_width = 3");
    assert_eq!(l.config.ambiguous_width, 1);
}

// verifies: AC-FR-703-1
#[test]
fn 列挙値の不正は許容値を提示する() {
    let l = load_str("file.default_line_ending = \"lf\\n\"\n");
    assert_eq!(l.config.default_line_ending, DefaultLineEnding::Lf);
    assert_eq!(
        kinds(&l),
        vec![(
            "file.default_line_ending",
            &DiagKind::InvalidValue {
                allowed: &["lf", "crlf", "cr", "native"]
            }
        )]
    );
}

#[test]
fn 文字コードの別名は正規名に寄せる() {
    for (label, want) in [
        ("sjis", DefaultEncoding::ShiftJis),
        ("windows-31j", DefaultEncoding::ShiftJis),
        ("Shift_JIS", DefaultEncoding::ShiftJis),
        ("utf8", DefaultEncoding::Utf8),
        ("utf-8-bom", DefaultEncoding::Utf8Bom),
        ("system", DefaultEncoding::System),
        ("euc-jp", DefaultEncoding::EucJp),
    ] {
        let l = load_str(&format!("file.default_encoding = \"{label}\""));
        assert!(l.diagnostics.is_empty(), "{label}");
        assert_eq!(l.config.default_encoding, want, "{label}");
    }
    let l = load_str("file.default_encoding = \"latin1\"");
    assert_eq!(l.config.default_encoding, DefaultEncoding::Utf8);
    assert_eq!(l.diagnostics.len(), 1);
}

// verifies: AC-FR-703-1
#[test]
fn 全項目を走査し_最初の問題で止まらない() {
    let l = load_str(
        "[editor]\ntab_width = 0\nindent_unit = 1\nindent_width = 3\n[theme]\ncolor_mode = \"x\"\n",
    );
    assert_eq!(l.config.indent_width, 3);
    assert_eq!(l.diagnostics.len(), 3);
}

#[test]
fn セクションが表でないと警告する() {
    let l = load_str("editor = 1\n");
    assert!(matches!(
        l.diagnostics[0].kind,
        DiagKind::TypeMismatch {
            expected: "table",
            ..
        }
    ));
}

#[test]
fn 日本語を含む行でも桁は文字数で数える() {
    let l = load_str("# 設定です\n[editor]\n日本語 = 1\n");
    assert_eq!((l.diagnostics[0].line, l.diagnostics[0].column), (3, 1));
}

// verifies: AC-FR-701-3
#[test]
fn 公開前のキーとセクションは情報診断で_警告にしない() {
    let l = load_str(
        "[editor]\nauto_indent = false\n[editor.undo]\nmax_steps = 5\n[search]\ncase = \"smart\"\n[view]\nline_numbers = \"off\"\n",
    );
    assert_eq!(l.config, Config::default());
    assert_eq!(l.diagnostics.len(), 4, "{:?}", l.diagnostics);
    assert!(
        l.diagnostics
            .iter()
            .all(|d| { d.severity == Severity::Info && d.kind == DiagKind::Unsupported })
    );
    let keys: Vec<&str> = l.diagnostics.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "editor.auto_indent",
            // `editor.undo` は公開キー (total_max_bytes) の接頭辞なので、中に入って個々のキーで報告する
            "editor.undo.max_steps",
            "search",
            "view.line_numbers"
        ]
    );
}

#[test]
fn 綴りの候補は公開前のキーからも探す() {
    let l = load_str("[editor]\nauto_indnt = true\n");
    assert_eq!(
        kinds(&l),
        vec![(
            "editor.auto_indnt",
            &DiagKind::UnknownKey {
                suggestion: Some("editor.auto_indent".to_owned())
            }
        )]
    );
}

/// 配布する設定例 (`dist/config.example.toml`) は、診断なしで読め、既定値と一致する (docs/05 §18.2)。
#[test]
fn 配布する設定例は既定値と一致する() {
    let l = load_str(include_str!("../../../../dist/config.example.toml"));
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    assert_eq!(l.config, Config::default());
}

#[test]
fn スワップの_5_キーを_入れ子のテーブルでも_ドットのキーでも読む() {
    let l = load_str(
        r#"
[file.swap]
enabled = false
interval_ms = 500
min_interval_ms = 3000
max_bytes = 2097152
exclude_secret_files = false
"#,
    );
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    let c = l.config;
    assert!(!c.swap_enabled);
    assert_eq!((c.swap_interval_ms, c.swap_min_interval_ms), (500, 3000));
    assert_eq!(c.swap_max_bytes, 2_097_152);
    assert!(!c.swap_exclude_secret_files);
    // `[file]` の中のドット記法も同じ
    let d = load_str(
        "[file]
swap.interval_ms = 700
",
    );
    assert!(d.diagnostics.is_empty(), "{:?}", d.diagnostics);
    assert_eq!(d.config.swap_interval_ms, 700);
}

#[test]
fn スワップの既定値は仕様どおり() {
    let c = Config::default();
    assert!(c.swap_enabled && c.swap_exclude_secret_files);
    assert_eq!(c.swap_interval_ms, 2000);
    assert_eq!(c.swap_min_interval_ms, 10_000);
    assert_eq!(c.swap_max_bytes, 64 * 1024 * 1024);
}

#[test]
fn スワップの値が不正なら警告して既定値のまま() {
    let l = load_str(
        r#"
[file.swap]
enabled = "yes"
interval_ms = 5
max_bytes = 1
"#,
    );
    assert_eq!(l.config, Config::default());
    let ks = kinds(&l);
    assert_eq!(ks.len(), 3, "{ks:?}");
    assert!(matches!(ks[0].1, DiagKind::TypeMismatch { .. }));
    assert!(matches!(ks[1].1, DiagKind::OutOfRange { .. }));
    assert!(matches!(ks[2].1, DiagKind::OutOfRange { .. }));
}

#[test]
fn スワップの下の未公開キーは情報_未知のキーは警告() {
    let l = load_str(
        "[file.swap]
compact_bytes = 1
enabeld = true
",
    );
    let ks = kinds(&l);
    assert_eq!(ks.len(), 2, "{ks:?}");
    assert!(matches!(ks[0].1, DiagKind::Unsupported));
    assert!(matches!(ks[1].1, DiagKind::UnknownKey { .. }));
}

// NFR-603 AC-3 (v0.2。チェックリストは v0.2 のゲートで作る)
#[test]
fn ui_locale_は言語タグか_auto() {
    use crate::Locale;
    for (text, want) in [
        ("auto", None),
        ("ja", Some(Locale::Ja)),
        ("en", Some(Locale::En)),
        ("en-US", Some(Locale::En)),
        ("JA_jp", Some(Locale::Ja)),
    ] {
        let l = load_str(&format!("[ui]\nlocale = \"{text}\"\n"));
        assert!(l.diagnostics.is_empty(), "{text}: {:?}", l.diagnostics);
        assert_eq!(l.config.ui_locale, want, "{text}");
    }
    // 既定は auto
    assert_eq!(Config::default().ui_locale, None);
}

#[test]
fn ui_locale_の不正な値は警告して_auto_のまま() {
    let l = load_str("[ui]\nlocale = \"fr\"\n");
    assert_eq!(l.config.ui_locale, None);
    assert_eq!(l.diagnostics.len(), 1);
    assert!(matches!(
        &l.diagnostics[0].kind,
        DiagKind::InvalidValue { allowed } if allowed.contains(&"en")
    ));
    assert_eq!(l.diagnostics[0].key, "ui.locale");
    let l = load_str("[ui]\nlocale = 3\n");
    assert!(matches!(
        &l.diagnostics[0].kind,
        DiagKind::TypeMismatch { .. }
    ));
}

#[test]
fn ui_の他のキーは公開前で_ui_locale_だけが公開() {
    let l = load_str("[ui]\nlocale = \"en\"\nbell = true\n");
    assert_eq!(l.config.ui_locale, Some(crate::Locale::En));
    assert_eq!(kinds(&l), vec![("ui.bell", &DiagKind::Unsupported)]);
}

#[test]
fn undo_total_max_bytes_は範囲つきの整数() {
    assert_eq!(Config::default().undo_total_max_bytes, 256 * 1024 * 1024);
    for (text, want) in [
        ("[editor.undo]\ntotal_max_bytes = 1048576\n", 1_048_576),
        ("[editor]\nundo.total_max_bytes = 536870912\n", 536_870_912),
    ] {
        let l = load_str(text);
        assert!(l.diagnostics.is_empty(), "{text}: {:?}", l.diagnostics);
        assert_eq!(l.config.undo_total_max_bytes, want, "{text}");
    }
    let l = load_str("[editor.undo]\ntotal_max_bytes = 1000\n");
    assert_eq!(l.config.undo_total_max_bytes, 256 * 1024 * 1024);
    assert!(matches!(
        &l.diagnostics[0].kind,
        DiagKind::OutOfRange { min: 1_048_576, .. }
    ));
    assert_eq!(l.diagnostics[0].key, "editor.undo.total_max_bytes");
}

#[test]
fn clipboard_osc52_の設定を読む() {
    let l =
        load_str("[clipboard]\nosc52 = \"always\"\nosc52_max_bytes = 5000\nprovider = \"auto\"\n");
    assert_eq!(l.config.osc52, Osc52Policy::Always);
    assert_eq!(l.config.osc52_max_bytes, 5000);
    // `provider` はまだ公開していない: 未対応の情報診断
    assert!(
        l.diagnostics.iter().all(|d| d.severity == Severity::Info),
        "{:?}",
        l.diagnostics
    );
    let l = load_str("[clipboard]\nosc52 = \"yes\"\nosc52_max_bytes = 10\n");
    assert_eq!(l.config.osc52, Osc52Policy::Auto);
    assert_eq!(l.config.osc52_max_bytes, 100_000, "範囲外は既定値");
    assert_eq!(l.diagnostics.len(), 2);
}

#[test]
fn file_reload_on_external_change_を読む() {
    let l = load_str("[file]\nreload_on_external_change = \"ask\"\n");
    assert_eq!(
        l.config.reload_on_external_change,
        crate::ExternalChangePolicy::Ask
    );
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    let l = load_str("[file]\nreload_on_external_change = \"auto\"\n");
    assert_eq!(
        l.config.reload_on_external_change,
        crate::ExternalChangePolicy::AutoIfUnmodified,
        "許容値の外は既定値"
    );
    assert_eq!(l.diagnostics.len(), 1);
}

#[test]
fn file_の閾値を読む() {
    let l = load_str("[file]\nhuge_file_warn_bytes = 2097152\nlong_line_threshold = 65536\n");
    assert_eq!(l.config.huge_file_warn_bytes, 2_097_152);
    assert_eq!(l.config.long_line_threshold, 65_536);
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    // 範囲外 (下限は 1 MiB と 64 KiB) は警告して既定値
    let l = load_str("[file]\nhuge_file_warn_bytes = 1000\nlong_line_threshold = 65535\n");
    assert_eq!(l.config.huge_file_warn_bytes, 512 * 1024 * 1024);
    assert_eq!(l.config.long_line_threshold, 1024 * 1024);
    assert_eq!(l.diagnostics.len(), 2);
}

#[test]
fn changed_keys_は変わった公開キーだけを並べ_全キーを網羅する() {
    let a = Config::default();
    let actual = changed_keys(&a, &a);
    assert!(actual.is_empty(), "{actual:?}");
    let b = Config {
        tab_width: 8,
        mouse: false,
        ..a.clone()
    };
    assert_eq!(changed_keys(&a, &b), ["editor.tab_width", "ui.mouse"]);
    // 公開キーの一覧 (KEYS) と同じ数を数えている (フィールドを足して changed_keys に足し忘れていない)
    let names: std::collections::BTreeSet<&str> = [
        "editor.tab_width",
        "editor.indent_unit",
        "editor.indent_width",
        "file.default_encoding",
        "file.default_line_ending",
        "view.ambiguous_width",
        "theme.color_mode",
        "file.swap.enabled",
        "file.swap.interval_ms",
        "file.swap.min_interval_ms",
        "file.swap.max_bytes",
        "file.swap.exclude_secret_files",
        "ui.locale",
        "editor.undo.total_max_bytes",
        "editor.backspace_unit",
        "view.wrap",
        "view.wrap_indent",
        "view.wrap_kinsoku",
        "view.show_whitespace",
        "theme.name",
        "theme.background",
        "ui.mouse",
        "ui.hint_bar",
        "ui.mouse_wheel_lines",
        "ui.which_key_delay_ms",
        "clipboard.osc52",
        "clipboard.osc52_max_bytes",
        "file.huge_file_warn_bytes",
        "file.long_line_threshold",
        "file.reload_on_external_change",
    ]
    .into_iter()
    .collect();
    let keys: std::collections::BTreeSet<&str> = KEYS.iter().copied().collect();
    assert_eq!(names, keys);
}

/// 既定値の表記 (診断の「既定値 {default} を使います」) を全公開キーで固定する (STR-T7)。宣言の表から生成するので、
/// 表の既定値か TOML の表記を変えたら、ここも変わる。
#[test]
fn 既定値の表記は全公開キーで決まった値() {
    let want = [
        ("editor.tab_width", "4"),
        ("editor.indent_unit", "space"),
        ("editor.indent_width", "4"),
        ("file.default_encoding", "utf-8"),
        ("file.default_line_ending", "lf"),
        ("view.ambiguous_width", "1"),
        ("theme.color_mode", "auto"),
        ("file.swap.enabled", "true"),
        ("file.swap.interval_ms", "2000"),
        ("file.swap.min_interval_ms", "10000"),
        ("file.swap.max_bytes", "67108864"),
        ("file.swap.exclude_secret_files", "true"),
        ("ui.locale", "auto"),
        ("editor.undo.total_max_bytes", "268435456"),
        ("editor.backspace_unit", "grapheme"),
        ("view.wrap", "off"),
        ("view.wrap_indent", "same"),
        ("view.wrap_kinsoku", "true"),
        ("view.show_whitespace", "minimal"),
        ("theme.name", "default"),
        ("theme.background", "auto"),
        ("ui.mouse", "true"),
        ("ui.hint_bar", "true"),
        ("ui.mouse_wheel_lines", "3"),
        ("ui.which_key_delay_ms", "500"),
        ("clipboard.osc52", "auto"),
        ("clipboard.osc52_max_bytes", "100000"),
        ("file.huge_file_warn_bytes", "536870912"),
        ("file.long_line_threshold", "1048576"),
        ("file.reload_on_external_change", "auto_if_unmodified"),
    ];
    assert_eq!(want.len(), KEYS.len());
    for (key, display) in want {
        assert_eq!(default_display(key), display, "{key}");
    }
    // 公開キーでない名前 (セクション・テーマの値など) は空
    for key in ["editor", "ui.selection.fg", "editor.auto_indent", ""] {
        assert_eq!(default_display(key), "", "{key}");
    }
}

/// 列挙値の許容値 (診断の「使える値: …」) の並びを固定する。
#[test]
fn 列挙値の許容値は決まった並び() {
    let cases: [(&[&str], &[&str]); 11] = [
        (IndentUnit::NAMES, &["tab", "space"]),
        (BackspaceUnit::NAMES, &["grapheme", "codepoint"]),
        (ThemeBackground::NAMES, &["auto", "dark", "light"]),
        (ShowWhitespace::NAMES, &["off", "minimal"]),
        (Osc52Policy::NAMES, &["auto", "always", "never"]),
        (
            ExternalChangePolicy::NAMES,
            &["ask", "auto_if_unmodified", "never"],
        ),
        (WrapMode::NAMES, &["off", "soft", "word"]),
        (WrapIndent::NAMES, &["none", "same"]),
        (
            DefaultEncoding::NAMES,
            &[
                "utf-8",
                "utf-8-bom",
                "utf-16le",
                "utf-16be",
                "shift_jis",
                "euc-jp",
                "iso-2022-jp",
                "system",
            ],
        ),
        (DefaultLineEnding::NAMES, &["lf", "crlf", "cr", "native"]),
        (
            ColorMode::NAMES,
            &["auto", "truecolor", "256", "16", "none"],
        ),
    ];
    for (names, want) in cases {
        assert_eq!(names, want);
    }
    // 値と変種は 1 対 1
    for n in ColorMode::NAMES {
        assert_eq!(ColorMode::from_name(n).map(ColorMode::name), Some(*n));
    }
}

#[test]
fn 公開キーのセクションは_sections_と一致する() {
    let mut from_keys: Vec<&str> = Vec::new();
    for k in KEYS {
        let s = k.split('.').next().unwrap();
        if !from_keys.contains(&s) {
            from_keys.push(s);
        }
    }
    from_keys.sort_unstable();
    let mut sections = SECTIONS.to_vec();
    sections.sort_unstable();
    assert_eq!(from_keys, sections);
}

/// 配布する設定例は、宣言の表の公開キーをすべて書き、既定値と同じ値を書く (宣言の表との突合、STR-E2)。
#[test]
fn 配布する設定例は全公開キーを既定値で書く() {
    let text = include_str!("../../../../dist/config.example.toml");
    let table: toml::Table = text.parse().expect("設定例は TOML");
    fn flatten(prefix: &str, t: &toml::Table, out: &mut Vec<(String, String)>) {
        for (k, v) in t {
            let full = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            match v {
                toml::Value::Table(inner) => flatten(&full, inner, out),
                other => out.push((full, other.to_string())),
            }
        }
    }
    let mut written = Vec::new();
    flatten("", &table, &mut written);
    written.sort();
    let mut want: Vec<(String, String)> = declared_values(&Config::default())
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    want.sort();
    assert_eq!(written, want);
}

/// 公開キーを型ごとに分ける (読み方の宣言の表との突合も兼ねる)。
const INT_KEYS: [&str; 12] = [
    "editor.tab_width",
    "editor.indent_width",
    "view.ambiguous_width",
    "file.swap.interval_ms",
    "file.swap.min_interval_ms",
    "file.swap.max_bytes",
    "editor.undo.total_max_bytes",
    "ui.mouse_wheel_lines",
    "ui.which_key_delay_ms",
    "clipboard.osc52_max_bytes",
    "file.huge_file_warn_bytes",
    "file.long_line_threshold",
];
const BOOL_KEYS: [&str; 5] = [
    "file.swap.enabled",
    "file.swap.exclude_secret_files",
    "view.wrap_kinsoku",
    "ui.mouse",
    "ui.hint_bar",
];

/// 読んだ結果の診断が 1 件で、そのキーのもの。
fn only_diag<'a>(l: &'a Loaded, key: &str) -> &'a Diagnostic {
    assert_eq!(l.diagnostics.len(), 1, "{key}: {:?}", l.diagnostics);
    let d = &l.diagnostics[0];
    assert_eq!(d.key, key);
    d
}

#[test]
fn 全公開キーで型の取り違えは警告して既定値に戻す() {
    let string_keys: Vec<&str> = KEYS
        .iter()
        .copied()
        .filter(|k| !INT_KEYS.contains(k) && !BOOL_KEYS.contains(k))
        .collect();
    assert_eq!(
        INT_KEYS.len() + BOOL_KEYS.len() + string_keys.len(),
        KEYS.len()
    );
    // (キー, 期待する型, 取り違えた値の TOML, found, 記録される値)
    let mut cases: Vec<(&str, &str, &str, &str, &str)> = Vec::new();
    for k in INT_KEYS {
        cases.push((k, "integer", "\"7\"", "string", "7"));
        cases.push((k, "integer", "true", "boolean", "true"));
        cases.push((k, "integer", "1.5", "float", "1.5"));
        cases.push((k, "integer", "[1]", "array", "array"));
        cases.push((k, "integer", "{ a = 1 }", "table", "table"));
    }
    for k in BOOL_KEYS {
        cases.push((k, "boolean", "1", "integer", "1"));
        cases.push((k, "boolean", "\"true\"", "string", "true"));
        cases.push((k, "boolean", "0x10", "integer", "0x10"));
    }
    for k in &string_keys {
        cases.push((k, "string", "1", "integer", "1"));
        cases.push((k, "string", "false", "boolean", "false"));
        cases.push((k, "string", "2.5", "float", "2.5"));
    }
    for (key, expected, value, found, shown) in cases {
        let l = load_str(&format!("{key} = {value}\n"));
        assert_eq!(l.config, Config::default(), "{key} = {value}");
        let d = only_diag(&l, key);
        assert_eq!(d.severity, Severity::Warning, "{key} = {value}");
        assert_eq!(
            d.kind,
            DiagKind::TypeMismatch { expected, found },
            "{key} = {value}"
        );
        assert_eq!(d.value, shown, "{key} = {value}");
        assert_eq!((d.line, d.column), (1, key.len() as u32 + 4), "{key}");
    }
}

#[test]
fn 整数は進数と区切りを読み_範囲の端を含む() {
    for (src, want) in [
        ("editor.tab_width = 1", 1),
        ("editor.tab_width = 16", 16),
        ("editor.tab_width = 0x10", 16),
        ("editor.tab_width = 0o10", 8),
        ("editor.tab_width = 0b11", 3),
        ("editor.tab_width = 1_0", 10),
    ] {
        let l = load_str(src);
        assert!(l.diagnostics.is_empty(), "{src}: {:?}", l.diagnostics);
        assert_eq!(l.config.tab_width, want, "{src}");
    }
    for (src, shown) in [
        ("editor.tab_width = 17", "17"),
        ("editor.tab_width = 0", "0"),
        // 書いたままの表記で記録する (進数の接頭辞・区切り・符号を落とさない)
        ("editor.tab_width = 0x11", "0x11"),
        ("editor.tab_width = 1_7", "1_7"),
        ("editor.tab_width = +17", "+17"),
        ("editor.tab_width = 17 # comment", "17"),
        // i64 に収まらないものは、読む前に TOML の解析が落とす場合がある
        (
            "editor.tab_width = 9223372036854775807",
            "9223372036854775807",
        ),
    ] {
        let l = load_str(src);
        assert_eq!(l.config.tab_width, 4, "{src}");
        let d = only_diag(&l, "editor.tab_width");
        assert_eq!(d.kind, DiagKind::OutOfRange { min: 1, max: 16 }, "{src}");
        assert_eq!(d.value, shown, "{src}");
        assert_eq!((d.line, d.column), (1, 20), "{src}");
    }
}

#[test]
fn 型の違う小数は書いたままの表記で記録する() {
    let l = load_str("editor.tab_width = 1_0.50\n");
    assert_eq!(l.config.tab_width, 4);
    let d = only_diag(&l, "editor.tab_width");
    assert_eq!(d.value, "1_0.50");
}

#[test]
fn 真偽値と文字列の正しい値は診断なしで読む() {
    let l = load_str("ui.mouse = false\nview.wrap_kinsoku = false\nview.wrap = \"word\"\n");
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    assert!(!l.config.mouse);
    assert!(!l.config.wrap_kinsoku);
    assert_eq!(l.config.wrap, WrapMode::Word);
}

#[test]
fn 列挙値の不正は書かれた値と位置を記録する() {
    let l = load_str("[view]\n  wrap = \"hard\"\n");
    assert_eq!(l.config.wrap, WrapMode::Off);
    let d = only_diag(&l, "view.wrap");
    assert_eq!(
        d.kind,
        DiagKind::InvalidValue {
            allowed: WrapMode::NAMES
        }
    );
    assert_eq!(d.value, "hard");
    assert_eq!((d.line, d.column), (2, 10));
    // 許容値は 1 つずつ読める
    for name in WrapMode::NAMES {
        let l = load_str(&format!("view.wrap = \"{name}\"\n"));
        assert!(l.diagnostics.is_empty(), "{name}");
        assert_eq!(l.config.wrap.name(), *name);
    }
}

#[test]
fn 診断の位置は行と桁_複数行でも() {
    let src = "# c\n\n[editor]\n\ttab_width = \"x\"\nindent_width = true\n";
    let l = load_str(src);
    let pos: Vec<(&str, u32, u32)> = l
        .diagnostics
        .iter()
        .map(|d| (d.key.as_str(), d.line, d.column))
        .collect();
    assert_eq!(
        pos,
        vec![("editor.indent_width", 5, 16), ("editor.tab_width", 4, 14)]
    );
    // 未知のキーはキーの位置
    let l = load_str("\n\n[editor]\n    zzz = 1\n");
    let d = only_diag(&l, "editor.zzz");
    assert_eq!((d.line, d.column), (4, 5));
}

#[test]
fn view_show_whitespace_の後の版の値は情報診断で_不正な値は警告() {
    for v in ["ideographic", "trailing", "all"] {
        let l = load_str(&format!("view.show_whitespace = \"{v}\"\n"));
        assert_eq!(l.config.show_whitespace, ShowWhitespace::Minimal);
        let d = only_diag(&l, "view.show_whitespace");
        assert_eq!(
            (d.severity, &d.kind),
            (Severity::Info, &DiagKind::Unsupported)
        );
        assert_eq!(d.value, v);
        assert_eq!((d.line, d.column), (1, 24));
    }
    let l = load_str("view.show_whitespace = \"off\"\n");
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    assert_eq!(l.config.show_whitespace, ShowWhitespace::Off);
    let l = load_str("view.show_whitespace = \"bogus\"\n");
    let d = only_diag(&l, "view.show_whitespace");
    assert_eq!(d.severity, Severity::Warning);
    assert_eq!(d.value, "bogus");
    assert_eq!(l.config.show_whitespace, ShowWhitespace::Minimal);
}

#[test]
fn theme_name_と_file_default_encoding_の不正は警告して既定値() {
    let l = load_str("theme.name = \"a/b\"\n");
    assert_eq!(l.config, Config::default());
    let d = only_diag(&l, "theme.name");
    assert_eq!(
        d.kind,
        DiagKind::InvalidValue {
            allowed: crate::theme::BUILTIN_NAMES
        }
    );
    assert_eq!(d.value, "a/b");
    let l = load_str("theme.name = \"my_theme-2\"\n");
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    assert_eq!(l.config.theme_name, "my_theme-2");
    let l = load_str("file.default_encoding = \"klingon\"\n");
    assert_eq!(l.config, Config::default());
    let d = only_diag(&l, "file.default_encoding");
    assert_eq!(
        d.kind,
        DiagKind::InvalidValue {
            allowed: DefaultEncoding::NAMES
        }
    );
    assert_eq!(d.value, "klingon");
}

#[test]
fn テーマの名前は英数字とハイフンと下線の_1_から_64_文字() {
    let ok64 = "a".repeat(64);
    let ng65 = "a".repeat(65);
    for s in ["a", "Z9", "-", "_", "my-theme_2", ok64.as_str()] {
        assert!(is_theme_name(s), "{s}");
    }
    for s in [
        "",
        ng65.as_str(),
        "a b",
        "a.b",
        "a/b",
        "a\\b",
        "..",
        "あ",
        "a\n",
        "a+b",
        "a:b",
        "a@b",
        "a[b",
        "a`b",
        "a{b",
        "a~b",
    ] {
        assert!(!is_theme_name(s), "{s:?}");
    }
    // 長さはバイトで数える (全角 22 字 = 66 バイトは通さない)
    assert!(!is_theme_name(&"あ".repeat(22)));
}

#[test]
fn 編集距離() {
    for (a, b, d) in [
        ("", "", 0),
        ("abc", "abc", 0),
        ("", "abc", 3),
        ("abc", "abd", 1),
        ("abc", "abcd", 1),
        ("kitten", "sitting", 3),
        ("flaw", "lawn", 2),
        ("ab", "ba", 2),
        ("日本語", "日本人", 1),
    ] {
        assert_eq!(distance(a, b), d, "{a} {b}");
        assert_eq!(distance(b, a), d, "{b} {a}");
    }
}

#[test]
fn 候補は編集距離_2_以下で最も近いもの_同点は先頭() {
    assert_eq!(nearest("abc", &[]), None);
    assert_eq!(nearest("abc", &["abc"]), Some("abc"));
    assert_eq!(nearest("abcd", &["abxy"]), Some("abxy")); // 距離 2
    assert_eq!(nearest("abcd", &["axyz"]), None); // 距離 3
    assert_eq!(nearest("ab", &["ac", "ad"]), Some("ac")); // 同点は先頭
    assert_eq!(nearest("ab", &["ad", "ac"]), Some("ad"));
    assert_eq!(nearest("abc", &["xbz", "abd"]), Some("abd")); // 後ろのほうが近い
    assert_eq!(nearest("abc", &["abd", "xbz"]), Some("abd"));
    assert_eq!(nearest("abc", &["xyz", "abx", "abc"]), Some("abc"));
}

#[test]
fn 綴りの候補は距離_2_まで_3_からは出さない() {
    let l = load_str("[editor]\ntab_wdth = 2\ntb_wdth = 2\ntab_w = 2\n");
    assert_eq!(
        kinds(&l),
        vec![
            ("editor.tab_w", &DiagKind::UnknownKey { suggestion: None }),
            (
                "editor.tab_wdth",
                &DiagKind::UnknownKey {
                    suggestion: Some("editor.tab_width".to_owned())
                }
            ),
            (
                "editor.tb_wdth",
                &DiagKind::UnknownKey {
                    suggestion: Some("editor.tab_width".to_owned())
                }
            ),
        ]
    );
}

#[test]
fn 文字コードの別名はすべて正規名に寄せる() {
    use DefaultEncoding::*;
    for (labels, want) in [
        (&["utf8", "unicode-1-1-utf-8", "UTF8"][..], Utf8),
        (&["utf-16", "ucs-2", "unicode", "csunicode"][..], Utf16Le),
        (&["unicodefffe"][..], Utf16Be),
        (
            &[
                "shift-jis",
                "sjis",
                "windows-31j",
                "ms_kanji",
                "csshiftjis",
                "x-sjis",
                "ms932",
                "cp932",
            ][..],
            ShiftJis,
        ),
        (&["x-euc-jp", "cseucpkdfmtjapanese"][..], EucJp),
        (&["csiso2022jp"][..], Iso2022Jp),
    ] {
        for label in labels {
            assert_eq!(encoding(label), Some(want), "{label}");
        }
    }
    assert_eq!(encoding("latin1"), None);
}
