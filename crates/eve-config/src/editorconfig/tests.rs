//! `.editorconfig` の読み方と重ね方の試験 (07 §10.1・§10.3 の EC-3〜EC-6)。仕様の例 (editorconfig.org の Specification の
//! 例のファイル) と、自分で書いた表の試験。EditorConfig の公式の試験の集まりの例は写していない (07 §10.3 の「ライセンス」)。

use super::*;

fn values(files: &[(&str, &str, &str)]) -> EditorConfigValues {
    let parsed: Vec<EditorConfigFile> = files
        .iter()
        .map(|(_, t, _)| parse_editorconfig(t))
        .collect();
    let sources: Vec<EditorConfigSource<'_>> = files
        .iter()
        .zip(&parsed)
        .map(|((name, _, rel), f)| (*name, f, *rel))
        .collect();
    resolve_editorconfig(&sources)
}

fn one(text: &str, rel: &str) -> EditorConfigValues {
    values(&[("/p/.editorconfig", text, rel)])
}

fn width(v: &EditorConfigValues) -> Option<EcWidth> {
    v.indent_width.as_ref().map(|(w, _)| *w)
}

fn tab(v: &EditorConfigValues) -> Option<u8> {
    v.tab_width.as_ref().map(|(w, _)| *w)
}

fn unit(v: &EditorConfigValues) -> Option<IndentUnit> {
    v.indent_unit.as_ref().map(|(u, _)| *u)
}

/// `EditorConfig` の仕様の文書 (editorconfig.org の Specification) が例に挙げるセクションとプロパティの組。注釈は写さず、
/// 自分で書いた。
const SPEC_EXAMPLE: &str = "\
; 一番上のファイル
root = true

; どのファイルも
[*]
end_of_line = lf
insert_final_newline = true

; 選択肢
[*.{js,py}]
charset = utf-8

; Python は空白 4 つ
[*.py]
indent_style = space
indent_size = 4

; Makefile はタブ
[Makefile]
indent_style = tab

; lib の下の JS
[lib/**.js]
indent_style = space
indent_size = 2

; ちょうどこの 2 つの名前
[{package.json,.travis.yml}]
indent_style = space
indent_size = 2
";

// verifies: AC-FR-705-4
#[test]
fn 仕様の例のファイルを仕様どおりに読む() {
    let f = parse_editorconfig(SPEC_EXAMPLE);
    assert!(f.root());
    assert_eq!(f.section_count(), 6);
    // insert_final_newline は v0.5 で写さない (未対応の情報診断)
    assert_eq!(f.diagnostics().len(), 1);
    assert_eq!(f.diagnostics()[0].kind, DiagKind::Unsupported);
    assert_eq!(f.diagnostics()[0].key, "insert_final_newline");
    assert_eq!(f.diagnostics()[0].line, 7);

    let py = one(SPEC_EXAMPLE, "src/a.py");
    assert_eq!(unit(&py), Some(IndentUnit::Space));
    assert_eq!(width(&py), Some(EcWidth::Width(4)));
    assert_eq!(tab(&py), Some(4), "tab_width は indent_size の値");
    assert_eq!(
        py.line_ending.as_ref().map(|v| v.0),
        Some(DefaultLineEnding::Lf)
    );
    assert_eq!(
        py.encoding.as_ref().map(|v| v.0),
        Some(DefaultEncoding::Utf8)
    );
    let (_, at) = py.indent_width.as_ref().unwrap();
    assert_eq!(
        (at.file.as_str(), at.line, at.column, at.section.as_str()),
        ("/p/.editorconfig", 16, 1, "*.py")
    );

    let make = one(SPEC_EXAMPLE, "Makefile");
    assert_eq!(unit(&make), Some(IndentUnit::Tab));
    assert_eq!(width(&make), None);
    assert_eq!(make.encoding, None);

    let js = one(SPEC_EXAMPLE, "lib/x/y.js");
    assert_eq!(width(&js), Some(EcWidth::Width(2)));
    assert_eq!(
        js.encoding.as_ref().map(|v| v.0),
        Some(DefaultEncoding::Utf8)
    );
    let js_top = one(SPEC_EXAMPLE, "a.js");
    assert_eq!(width(&js_top), None, "lib の外");

    for name in ["package.json", ".travis.yml", "sub/package.json"] {
        assert_eq!(
            width(&one(SPEC_EXAMPLE, name)),
            Some(EcWidth::Width(2)),
            "{name}"
        );
    }
    assert_eq!(width(&one(SPEC_EXAMPLE, "package.jsonx")), None);
}

#[test]
fn 注釈と空行と前置きと大文字小文字() {
    let text = "\u{feff}; c\r\n  # c\r\nROOT = TRUE\r\nindent_style = tab\r\n[*]\r\nIndent_Style = SPACE\r\nnot a property\r\nfoo = bar\r\nindent_size=  3  \r\n";
    let f = parse_editorconfig(text);
    assert!(f.root(), "BOM・CRLF・大文字の root");
    assert!(f.diagnostics().is_empty(), "{:?}", f.diagnostics());
    let v = one(text, "a");
    assert_eq!(
        unit(&v),
        Some(IndentUnit::Space),
        "前置きの indent_style は読まない"
    );
    assert_eq!(width(&v), Some(EcWidth::Width(3)));
    // 行の途中の # と ; は値の一部 (charset には写せない値)
    let g = parse_editorconfig("[*]\ncharset = utf-8 # c\n");
    assert_eq!(
        g.diagnostics()[0].kind,
        DiagKind::EditorConfig(EditorConfigProblem::CharsetUnmapped)
    );
    assert_eq!(g.diagnostics()[0].value, "utf-8 # c");
    // root = false は前の root = true を取り消す。セクションの中の root は読まない
    assert!(!parse_editorconfig("root = true\nroot = false\n").root());
    assert!(!parse_editorconfig("[*]\nroot = true\n").root());
}

#[test]
fn 近いファイルと後のセクションが勝ち_unset_は取り消す() {
    let far = "[*]\nindent_style = tab\nindent_size = 8\ncharset = utf-16le\n";
    let near = "[*.rs]\nindent_size = 2\n[*]\ncharset = unset\n";
    let v = values(&[
        ("/a/.editorconfig", far, "b/x.rs"),
        ("/a/b/.editorconfig", near, "x.rs"),
    ]);
    assert_eq!(unit(&v), Some(IndentUnit::Tab));
    assert_eq!(width(&v), Some(EcWidth::Width(2)));
    assert_eq!(v.tab_width.as_ref().unwrap().1.file, "/a/b/.editorconfig");
    assert_eq!(v.encoding, None, "unset");
    // 同じファイルの中は後のセクションが勝つ
    let w = one(
        "[*]\nindent_size = 2\n[*.rs]\nindent_size = 3\n[*]\nindent_size = unset\n",
        "a.rs",
    );
    assert_eq!(width(&w), None);
    // tab_width を書けば indent_size から決めない。indent_size = tab は幅を決めない
    let t = one("[*]\nindent_size = 2\ntab_width = 8\n", "a");
    assert_eq!((width(&t), tab(&t)), (Some(EcWidth::Width(2)), Some(8)));
    let t = one("[*]\nindent_size = tab\n", "a");
    assert_eq!((width(&t), tab(&t)), (Some(EcWidth::Tab), None));
    // 写せない charset は unset と同じ (遠いファイルの値を使わない)
    let c = values(&[
        ("/a/.editorconfig", "[*]\ncharset = utf-8-bom\n", "x"),
        ("/a/b/.editorconfig", "[*]\ncharset = latin1\n", "x"),
    ]);
    assert_eq!(c.encoding, None);
    // 誤った値はその値だけを使わない (前の値のまま)
    let e = one(
        "[*]\nindent_size = 4\n[*]\nindent_size = 0\nend_of_line = CRLF\n",
        "a",
    );
    assert_eq!(width(&e), Some(EcWidth::Width(4)));
    assert_eq!(e.line_ending.map(|v| v.0), Some(DefaultLineEnding::Crlf));
}

#[test]
fn 誤った値と写さないプロパティの診断() {
    let text = "root = maybe\n[*]\nindent_style = tabs\nindent_size = 17\ntab_width = -1\nend_of_line = lfcr\ncharset = latin1\nmax_line_length = 80\ntrim_trailing_whitespace = true\nspelling_language = en\nij_any = 1\n";
    let f = parse_editorconfig(text);
    let got: Vec<(String, String, u32, Severity)> = f
        .diagnostics()
        .iter()
        .map(|d| (d.key.clone(), d.value.clone(), d.line, d.severity))
        .collect();
    let w = Severity::Warning;
    let i = Severity::Info;
    assert_eq!(
        got,
        [
            ("root".into(), "maybe".into(), 1, w),
            ("indent_style".into(), "tabs".into(), 3, w),
            ("indent_size".into(), "17".into(), 4, w),
            ("tab_width".into(), "-1".into(), 5, w),
            ("end_of_line".into(), "lfcr".into(), 6, w),
            ("charset".into(), "latin1".into(), 7, i),
            ("max_line_length".into(), String::new(), 8, i),
            ("trim_trailing_whitespace".into(), String::new(), 9, i),
        ]
    );
    assert_eq!(
        f.diagnostics()[2].kind,
        DiagKind::EditorConfig(EditorConfigProblem::InvalidValue {
            allowed: vec!["1〜16".into(), "tab".into(), "unset".into()]
        })
    );
    // 当たらないセクションの誤りも診断する (中身だけで決まる)
    assert_eq!(
        parse_editorconfig("[nothing]\nindent_size = x\n")
            .diagnostics()
            .len(),
        1
    );
}

#[test]
fn 斜線の無い_glob_はどの深さのファイル名にも当て_斜線があれば置き場所からの相対パス() {
    let t = "[*.rs]\nindent_size = 2\n[src/*.c]\nindent_size = 3\n[/top.h]\nindent_size = 4\n";
    assert_eq!(width(&one(t, "a/b/c.rs")), Some(EcWidth::Width(2)));
    assert_eq!(width(&one(t, "src/x.c")), Some(EcWidth::Width(3)));
    assert_eq!(width(&one(t, "a/src/x.c")), None, "置き場所からの相対パス");
    assert_eq!(width(&one(t, "top.h")), Some(EcWidth::Width(4)));
    assert_eq!(width(&one(t, "a/top.h")), None);
    // 大文字小文字を区別する (EC-4)
    assert_eq!(width(&one(t, "A.RS")), None);
    assert_eq!(width(&one(t, "Src/x.c")), None);
    // 数の範囲と否定の集合
    let n = "[file{1..3}.txt]\nindent_size = 5\n[[!a]*.md]\nindent_size = 6\n";
    assert_eq!(width(&one(n, "d/file2.txt")), Some(EcWidth::Width(5)));
    assert_eq!(width(&one(n, "file4.txt")), None);
    assert_eq!(width(&one(n, "b.md")), Some(EcWidth::Width(6)));
    assert_eq!(width(&one(n, "a.md")), None);
}

#[test]
fn 資源の上限を超える_glob_のセクションは使わず_ほかのセクションに影響しない() {
    let deep = format!("[{}{}]", "{a,".repeat(9), "}".repeat(9));
    let text = format!("[*]\nindent_size = 2\n{deep}\nindent_size = 7\n[*.x]\ntab_width = 3\n");
    let f = parse_editorconfig(&text);
    assert_eq!(
        f.diagnostics()[0].kind,
        DiagKind::EditorConfig(EditorConfigProblem::BadGlob(GlobError::TooDeep))
    );
    assert_eq!(f.diagnostics()[0].line, 3);
    let v = one(&text, "a");
    assert_eq!(
        width(&v),
        Some(EcWidth::Width(2)),
        "使わないセクションの値を前のセクションに付けない"
    );
    let v = one(&text, "b.x");
    assert_eq!(tab(&v), Some(3));
    // 長すぎる見出しは飛ばす (診断なし)。続くプロパティも使わない
    let long = format!(
        "[*]\nindent_size = 2\n[{}]\nindent_size = 9\n",
        "a".repeat(EDITORCONFIG_GLOB_MAX_BYTES + 1)
    );
    assert_eq!(parse_editorconfig(&long).diagnostics(), []);
    assert_eq!(width(&one(&long, "a")), Some(EcWidth::Width(2)));
    // 長すぎるキー・値の行は飛ばす
    let kv = format!(
        "[*]\nindent_size = 2\nindent_size = {}\n{} = 1\n",
        "9".repeat(EDITORCONFIG_VALUE_MAX_BYTES + 1),
        "k".repeat(EDITORCONFIG_KEY_MAX_BYTES + 1)
    );
    let f = parse_editorconfig(&kv);
    assert!(f.diagnostics().is_empty(), "{:?}", f.diagnostics());
    assert_eq!(width(&one(&kv, "a")), Some(EcWidth::Width(2)));
}

#[test]
fn 行とセクションの数の上限で止める() {
    let many: String = (0..EDITORCONFIG_FILE_MAX_SECTIONS + 5)
        .map(|i| format!("[f{i}]\nindent_size = 2\n"))
        .collect();
    let f = parse_editorconfig(&many);
    assert_eq!(f.section_count(), EDITORCONFIG_FILE_MAX_SECTIONS);
    assert_eq!(width(&one(&many, "f999")), Some(EcWidth::Width(2)));
    assert_eq!(width(&one(&many, "f1000")), None);
    let lines = format!(
        "{}[*]\nindent_size = 2\n",
        "#\n".repeat(EDITORCONFIG_FILE_MAX_LINES)
    );
    assert_eq!(parse_editorconfig(&lines).section_count(), 0);
}

#[test]
fn 誤った値の許される値と理由の名前() {
    assert_eq!(NotReadReason::TooLarge.reason_name(), "too_large");
    assert_eq!(NotReadReason::NotUtf8.reason_name(), "not_utf8");
    assert_eq!(NotReadReason::TotalLimit.reason_name(), "total_limit");
    let d = not_read_diagnostic(NotReadReason::NotUtf8, "/a/.editorconfig");
    assert_eq!(d.value, "/a/.editorconfig");
    assert_eq!(d.severity, Severity::Warning);
    assert_eq!(
        EditorConfigProblem::BadGlob(GlobError::TooManyAlternatives).message_name(),
        "bad_glob"
    );
    assert!(EditorConfigValues::default().is_empty());
}
