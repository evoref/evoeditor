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
    assert!(l.diagnostics.is_empty());
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
        }
    );
}

#[test]
fn ドット記法でも読める() {
    let l = load_str("editor.tab_width = 2\n");
    assert_eq!(l.config.tab_width, 2);
    assert!(l.diagnostics.is_empty());
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
                allowed: LINE_ENDINGS
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
            "editor.undo",
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
