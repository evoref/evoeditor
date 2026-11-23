use super::*;
use crate::{Conditions, Source, load_language_str, resolve};
use proptest::prelude::*;

fn defs(text: &str) -> Vec<LanguageDef> {
    let (d, diags) = load_language_str("languages/a.toml", text);
    assert!(diags.is_empty(), "{diags:?}");
    d
}

fn detect<'a>(set: &'a LanguageSet, name: &str) -> Option<&'a str> {
    set.detect(&DetectInput {
        file_name: Some(name),
        ..DetectInput::default()
    })
    .language
    .map(Language::name)
}

#[test]
fn 組込みの名前は重ならず_どれも名前の規則に合う() {
    let mut seen = std::collections::BTreeMap::new();
    for b in BUILTIN_LANGUAGES {
        assert!(is_language_name(b.id), "{}", b.id);
        for n in b.file_types.iter().chain(b.file_names) {
            let key = (b.file_types.contains(n), n.to_ascii_lowercase());
            if let Some(other) = seen.insert(key, b.id) {
                panic!("{n} が {other} と {} の両方にある", b.id);
            }
        }
    }
}

// verifies: AC-FR-704-1
#[test]
fn 判定は名前_拡張子の順で_大文字小文字を区別しない() {
    let set = LanguageSet::builtin();
    assert_eq!(detect(&set, "main.rs"), Some("rust"));
    assert_eq!(detect(&set, "MAIN.RS"), Some("rust"));
    assert_eq!(detect(&set, "Makefile"), Some("make"));
    assert_eq!(detect(&set, "makefile"), Some("make"));
    assert_eq!(detect(&set, "Cargo.lock"), Some("toml"));
    assert_eq!(detect(&set, ".bashrc"), Some("bash"));
    assert_eq!(
        detect(&set, ".rs"),
        None,
        "先頭の . は拡張子の区切りではない"
    );
    assert_eq!(detect(&set, "README"), None);
    assert_eq!(detect(&set, "x.h"), Some("c"));
}

#[test]
fn 長い拡張子が勝つ() {
    let (set, _) = LanguageSet::build(&defs(
        "[[language]]\nname = \"dts\"\nfile_types = [\"d.ts\"]\n",
    ));
    assert_eq!(detect(&set, "a.d.ts"), Some("dts"));
    assert_eq!(detect(&set, "a.ts"), Some("typescript"));
}

#[test]
fn 先頭行は名前で決まらないときだけ() {
    let set = LanguageSet::builtin();
    let d = |name: Option<&str>, line: &str| {
        set.detect(&DetectInput {
            file_name: name,
            rel_path: None,
            first_line: Some(line),
        })
        .language
        .map(Language::name)
    };
    assert_eq!(d(Some("run"), "#!/usr/bin/env python3"), Some("python"));
    assert_eq!(d(None, "#!/bin/bash"), Some("bash"));
    assert_eq!(d(None, "#!/bin/sh -e"), Some("bash"));
    assert_eq!(d(None, "#!/usr/bin/env zsh"), Some("zsh"));
    assert_eq!(d(None, "#!/usr/bin/env node"), Some("javascript"));
    assert_eq!(d(Some("a.rs"), "#!/bin/sh"), Some("rust"));
    assert_eq!(d(None, "hello"), None);
}

#[test]
fn 行頭に要る文字列は記号の前まで() {
    assert_eq!(anchored_prefix(r"^#!.*\bpython[0-9.]*\b"), "#!");
    assert_eq!(anchored_prefix("^abc"), "abc");
    assert_eq!(anchored_prefix("^ab*c"), "a", "量指定の付いた文字は省ける");
    assert_eq!(anchored_prefix("^ab?"), "a");
    assert_eq!(anchored_prefix("^a{2}"), "");
    assert_eq!(anchored_prefix("^(a|b)"), "");
    assert_eq!(
        anchored_prefix("(?i)^ab"),
        "",
        "フラグで始まれば当て方を変えない"
    );
    assert_eq!(anchored_prefix("ab"), "");
    assert_eq!(anchored_prefix(r"^\#"), "");
    // 前置きで外れる行は正規表現に当てない。当たる行は正規表現の結果
    let (set, _) = LanguageSet::build(&defs("[[language]]\nname = \"x\"\nfirst_line = \"^@X\"\n"));
    let d = |line: &str| {
        set.detect(&DetectInput {
            file_name: None,
            rel_path: None,
            first_line: Some(line),
        })
        .language
        .map(Language::name)
    };
    assert_eq!(d("@X tmpl"), Some("x"));
    assert_eq!(d("@x tmpl"), None);
}

#[test]
fn glob_は斜線の有無で当てる所を変え_ルートの外には当てない() {
    let (set, _) = LanguageSet::build(&defs(
        "[[language]]\nname = \"ci\"\nfile_globs = [\".github/**/*.yml\"]\n\
         [[language]]\nname = \"tmpl\"\nfile_globs = [\"*.TMPL\"]\n",
    ));
    let d = |name: &str, rel: Option<&str>| {
        set.detect(&DetectInput {
            file_name: Some(name),
            rel_path: rel,
            first_line: None,
        })
        .language
        .map(Language::name)
    };
    assert_eq!(d("a.yml", Some(".github/workflows/a.yml")), Some("ci"));
    assert_eq!(d("a.yml", None), Some("yaml"), "ルートの外");
    assert_eq!(d("a.yml", Some("src/a.yml")), Some("yaml"));
    assert_eq!(d("x.tmpl", Some("deep/x.tmpl")), Some("tmpl"));
    assert_eq!(d("X.Tmpl", None), Some("tmpl"), "大文字小文字を区別しない");
}

// verifies: AC-FR-704-1
#[test]
fn 同じ層の重なりは後の定義が勝って知らせ_強い層は黙って勝つ() {
    let (set, _) = LanguageSet::build(&defs(
        "[[language]]\nname = \"a\"\nfile_types = [\"foo\"]\n\
         [[language]]\nname = \"b\"\nfile_types = [\"foo\"]\n\
         [[language]]\nname = \"myh\"\nfile_types = [\"h\"]\n",
    ));
    let r = set.detect(&DetectInput {
        file_name: Some("x.foo"),
        ..DetectInput::default()
    });
    assert_eq!(r.language.map(Language::name), Some("b"));
    let amb = r.ambiguous.expect("知らせる");
    assert_eq!((amb.first.as_str(), amb.second.as_str()), ("a", "b"));
    assert_eq!(
        amb.origin,
        LangOrigin::File {
            source: "languages/a.toml".to_owned(),
            line: 6,
            column: 1
        }
    );
    let r = set.detect(&DetectInput {
        file_name: Some("x.h"),
        ..DetectInput::default()
    });
    assert_eq!(r.language.map(Language::name), Some("myh"));
    assert!(
        r.ambiguous.is_none(),
        "ユーザー層が組込みに勝つのは知らせない"
    );
}

#[test]
fn 併合は項目ごとで_配列は置換か追加() {
    let (set, _) = LanguageSet::build(&defs(
        "[[language]]\nname = \"rust\"\nindent = { width = 2 }\n\
         [[language]]\nname = \"c\"\n\"file_types+\" = [\"inc\"]\n\
         [[language]]\nname = \"cpp\"\nfile_types = [\"cpp\"]\n",
    ));
    let rust = set.get("rust").unwrap();
    assert_eq!(rust.indent_width.as_ref().map(|(w, _)| *w), Some(2));
    assert_eq!(
        rust.comment_line.as_ref().map(|(c, _)| c.as_str()),
        Some("//"),
        "書かない項目は組込みのまま"
    );
    assert_eq!(detect(&set, "a.rs"), Some("rust"));
    assert_eq!(detect(&set, "a.inc"), Some("c"));
    assert_eq!(detect(&set, "a.c"), Some("c"));
    assert_eq!(detect(&set, "a.hpp"), None, "置換した");
}

// verifies: AC-FR-704-2
#[test]
fn 効かない項目は未対応の情報で_誤りは項目だけを使わない() {
    let text = "[[language]]\nname = \"rust\"\nauto_pairs = { \"(\" = \")\" }\nformatter = \"rustfmt\"\n\
                indent = { unit = \"tabs\", width = 20, tab_width = 8 }\ncomment = { line = \"# \", block = [\"/*\", \"*/\"] }\n\
                file_typs = [\"rs\"]\n";
    let (d, diags) = load_language_str("languages/rust.toml", text);
    let kinds: Vec<(&str, Severity, &DiagKind)> = diags
        .iter()
        .map(|x| (x.key.as_str(), x.severity, &x.kind))
        .collect();
    assert!(kinds.contains(&(
        "language.auto_pairs",
        Severity::Info,
        &DiagKind::Unsupported
    )));
    assert!(kinds.contains(&("language.formatter", Severity::Info, &DiagKind::Unsupported)));
    assert!(kinds.contains(&(
        "language.comment.block",
        Severity::Info,
        &DiagKind::Unsupported
    )));
    assert!(kinds.iter().any(|(k, s, kind)| *k == "language.indent.unit"
        && *s == Severity::Warning
        && matches!(kind, DiagKind::InvalidValue { .. })));
    assert!(
        kinds
            .iter()
            .any(|(k, _, kind)| *k == "language.indent.width"
                && matches!(kind, DiagKind::OutOfRange { min: 1, max: 16 }))
    );
    assert!(kinds.contains(&(
        "language.comment.line",
        Severity::Warning,
        &DiagKind::Language(LanguageProblem::BadComment)
    )));
    assert!(kinds.iter().any(|(k, _, kind)| *k == "language.file_typs"
        && matches!(kind, DiagKind::UnknownKey { suggestion: Some(s) } if s == "language.file_types")));
    // 行は書いた所
    let unit = diags
        .iter()
        .find(|x| x.key == "language.indent.unit")
        .unwrap();
    assert_eq!((unit.line, unit.value.as_str()), (5, "tabs"));
    // 誤りでない項目は使う
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].tab_width.as_ref().map(|v| v.value), Some(8));
    assert!(d[0].indent_unit.is_none() && d[0].indent_width.is_none());
    assert!(d[0].comment_line.is_none());
}

#[test]
fn 名前の誤りと無い名前は定義ごと使わない() {
    let (d, diags) = load_language_str(
        "c.toml",
        "[[language]]\nname = \"Rust\"\n[[language]]\nname = \"plain\"\n[[language]]\nfile_types = [\"x\"]\n\
         [[language]]\nname = \"ok-1+\"\n",
    );
    assert_eq!(
        d.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
        ["ok-1+"]
    );
    let p: Vec<&DiagKind> = diags.iter().map(|x| &x.kind).collect();
    assert_eq!(
        p,
        [
            &DiagKind::Language(LanguageProblem::InvalidName),
            &DiagKind::Language(LanguageProblem::InvalidName),
            &DiagKind::Language(LanguageProblem::MissingName),
        ]
    );
    assert_eq!(diags[0].value, "Rust");
    assert!(!is_language_name(&"a".repeat(33)));
    assert!(is_language_name(&"a".repeat(32)));
}

#[test]
fn 長さの上限と正規表現の誤り() {
    let long = "x".repeat(257);
    let text = format!(
        "[[language]]\nname = \"a\"\ndisplay_name = \"{}\"\nfile_types = [\"{long}\"]\nfirst_line = \"(\"\n",
        "あ".repeat(65)
    );
    let (d, diags) = load_language_str("languages/a.toml", &text);
    assert_eq!(
        diags
            .iter()
            .filter(|x| x.kind == DiagKind::Language(LanguageProblem::TooLong))
            .count(),
        2
    );
    let (set, reports) = LanguageSet::build(&d);
    assert!(
        reports.is_empty(),
        "重ねるときには正規表現を組まない: {reports:?}"
    );
    let reports = set.precompile();
    let (file, ds) = &reports[0];
    assert_eq!(file, "languages/a.toml");
    assert!(matches!(
        &ds[0].kind,
        DiagKind::Language(LanguageProblem::BadFirstLine { reason }) if !reason.is_empty()
    ));
    assert_eq!(ds[0].line, 5);
}

#[test]
fn 言語定義のファイルのほかの表は未知のキーで_条件付きセクションも同じ() {
    let (_, diags) = load_language_str(
        "languages/a.toml",
        "[editor]\ntab_width = 2\n[os.windows.editor]\ntab_width = 3\n",
    );
    assert!(
        diags
            .iter()
            .all(|d| matches!(d.kind, DiagKind::UnknownKey { .. })),
        "{diags:?}"
    );
    assert_eq!(diags.len(), 2);
}

#[test]
fn 設定ファイルの言語定義は表示名を付けて層の順に集める() {
    let r = resolve(
        &[
            Source {
                name: "config.toml".to_owned(),
                text: Ok(Some(
                    "[editor]\ntab_width = 8\n[[language]]\nname = \"go\"\nindent = { width = 8 }\n"
                        .to_owned(),
                )),
            },
            Source {
                name: "b.toml".to_owned(),
                text: Ok(Some("[[language]]\nname = \"x\"\n".to_owned())),
            },
        ],
        &[],
        Conditions::default(),
    );
    assert!(r.reports.is_empty(), "{:?}", r.reports);
    let names: Vec<(&str, &str)> = r
        .languages
        .iter()
        .map(|d| (d.source.as_str(), d.name.as_str()))
        .collect();
    assert_eq!(names, [("config.toml", "go"), ("b.toml", "x")]);
    // 条件の中の言語定義は使わない (COND-5 (d))
    let r = resolve(
        &[Source {
            name: "config.toml".to_owned(),
            text: Ok(Some("[[os.windows.language]]\nname = \"x\"\n".to_owned())),
        }],
        &[],
        Conditions::default(),
    );
    assert!(r.languages.is_empty(), "{:?}", r.languages);
}

#[test]
fn 出力を読み直すと組込みと同じ判定になる() {
    let (set, _) = LanguageSet::build(&defs(
        "[[language]]\nname = \"go\"\nindent = { width = 8, tab_width = 8 }\n\
         [[language]]\nname = \"tmpl\"\nfile_globs = [\"*.tmpl\"]\nfirst_line = \"^@tmpl\"\n",
    ));
    let dumped = set.dump();
    let (again, diags) = load_language_str("dump", &dumped);
    assert!(diags.is_empty(), "{diags:?}\n{dumped}");
    let (set2, reports) = LanguageSet::build(&again);
    assert!(reports.is_empty(), "{reports:?}");
    assert_eq!(set2.dump(), dumped);
    assert!(dumped.contains("[[language]]\nname = \"make\"\ndisplay_name = \"Makefile\"\nfile_types = [\"mk\", \"mak\"]\nfile_names = [\"Makefile\", \"GNUmakefile\"]\nfile_globs = []\nindent = { unit = \"tab\" }\ncomment = { line = \"#\" }\n"), "{dumped}");
}

#[test]
fn 最初の行は_1_kib_までの改行の前() {
    assert_eq!(first_line_of("#!/bin/sh\r\necho"), "#!/bin/sh");
    assert_eq!(first_line_of("abc"), "abc");
    let long = "あ".repeat(400);
    let l = first_line_of(&long);
    assert!(l.len() <= FIRST_LINE_BYTES && long.starts_with(l));
}

/// 文書ごとの値の段 (LAYER-1 の (2) ユーザー層の一般の値・(3) 組込みの言語・(4) ユーザー層の言語・(8) `--set`)。
#[derive(Debug, Clone)]
struct Stages {
    general: Option<u8>,
    builtin: bool,
    user_lang: Option<u8>,
    set: Option<u8>,
}

fn stages() -> impl Strategy<Value = Stages> {
    (
        proptest::option::of(1u8..=16),
        any::<bool>(),
        proptest::option::of(1u8..=16),
        proptest::option::of(1u8..=16),
    )
        .prop_map(|(general, builtin, user_lang, set)| Stages {
            general,
            builtin,
            user_lang,
            set,
        })
}

proptest! {
    // verifies: AC-FR-704-1
    /// 参照モデル「LAYER-1 の順で最後に値を持つ段の値」と一致する (07 §12.6.1)。字下げの幅と単位とタブの幅のそれぞれで、
    /// 組込みの言語の値は `make` の `indent.unit = "tab"` だけ (幅は組込みに無い)。
    #[test]
    fn 文書ごとの値は最後に値を持つ段の値(w in stages(), t in stages(), u in stages()) {
        // 言語は 1 つの文書で 1 つ: 組込みの段の有無は、文書の言語を make にするか、組込みの無い zz にするかで作る
        let builtin = u.builtin;
        let lang = if builtin { "make" } else { "zz" };
        let mut config = String::from("[editor]\n");
        let mut sets = Vec::new();
        let mut user = format!("[[language]]\nname = \"{lang}\"\nindent = {{ ");
        let mut items = Vec::new();
        for (key, s) in [("indent_width", &w), ("tab_width", &t)] {
            if let Some(v) = s.general {
                config.push_str(&format!("{key} = {v}\n"));
            }
            if let Some(v) = s.set {
                sets.push(format!("editor.{key}={v}"));
            }
            if let Some(v) = s.user_lang {
                items.push(format!("{} = {v}", if key == "indent_width" { "width" } else { "tab_width" }));
            }
        }
        let unit_name = |v: u8| if v.is_multiple_of(2) { "tab" } else { "space" };
        if let Some(v) = u.general {
            config.push_str(&format!("indent_unit = \"{}\"\n", unit_name(v)));
        }
        if let Some(v) = u.set {
            sets.push(format!("editor.indent_unit={}", unit_name(v)));
        }
        if let Some(v) = u.user_lang {
            items.push(format!("unit = \"{}\"", unit_name(v)));
        }
        user.push_str(&items.join(", "));
        user.push_str(" }\n");
        let r = resolve(
            &[Source { name: "config.toml".to_owned(), text: Ok(Some(config)) }],
            &sets,
            Conditions::default(),
        );
        prop_assert!(r.reports.is_empty(), "{:?}", r.reports);
        let (set, rep) = LanguageSet::build(&defs(&user));
        prop_assert!(rep.is_empty());
        let v = doc_values(&r.config, &r.origins, set.get(lang));
        // 参照モデル
        let width = |s: &Stages, default: u8| s.set.or(s.user_lang).or(s.general).unwrap_or(default);
        prop_assert_eq!(v.indent_width.value, width(&w, 4));
        prop_assert_eq!(v.tab_width.value, width(&t, 4));
        let unit = u.set.map(unit_name)
            .or(u.user_lang.map(unit_name))
            .or(builtin.then_some("tab"))
            .or(u.general.map(unit_name))
            .unwrap_or("space");
        prop_assert_eq!(v.indent_unit.value.name(), unit);
        // 由来: 言語の段で決まったものは言語の由来
        let from_lang = u.set.is_none() && (u.user_lang.is_some() || builtin);
        prop_assert_eq!(matches!(v.indent_unit.origin, ValueOrigin::Language { .. }), from_lang);
    }
}

// verifies: AC-FR-704-1
#[test]
fn 同じ層の後の定義は書いた順で決まり_ファイルをまたいで組込みを拡張しても同じ() {
    // a.toml で新しい言語 tmpl に tpl を、後の b.toml で組込みの html に tpl を足す。書いた順で後の html が勝つ
    let (a, _) = load_language_str(
        "languages/a.toml",
        "[[language]]\nname = \"tmpl\"\nfile_types = [\"tpl\"]\n",
    );
    let (b, _) = load_language_str(
        "languages/b.toml",
        "[[language]]\nname = \"html\"\n\"file_types+\" = [\"tpl\"]\n",
    );
    let all: Vec<LanguageDef> = a.into_iter().chain(b).collect();
    let (set, _) = LanguageSet::build(&all);
    let r = set.detect(&DetectInput {
        file_name: Some("x.tpl"),
        ..DetectInput::default()
    });
    assert_eq!(r.language.map(Language::name), Some("html"));
    let amb = r.ambiguous.expect("知らせる");
    assert_eq!((amb.first.as_str(), amb.second.as_str()), ("tmpl", "html"));
    // 逆の順なら tmpl
    let (b2, _) = load_language_str(
        "languages/a.toml",
        "[[language]]\nname = \"html\"\n\"file_types+\" = [\"tpl\"]\n",
    );
    let (a2, _) = load_language_str(
        "languages/b.toml",
        "[[language]]\nname = \"tmpl\"\nfile_types = [\"tpl\"]\n",
    );
    let all: Vec<LanguageDef> = b2.into_iter().chain(a2).collect();
    let (set, _) = LanguageSet::build(&all);
    assert_eq!(detect(&set, "x.tpl"), Some("tmpl"));
}

#[test]
fn 組めない先頭行は正規表現を組む前と後で同じ判定になる() {
    // 行頭の `@x` が合わない行でも、下の層 (組込みの python) が当たるなら、組めない上の層の値は使わずに python
    let (set, _) = LanguageSet::build(&defs(
        "[[language]]\nname = \"python\"\nfirst_line = \"^@x(\"\n",
    ));
    let d = |set: &LanguageSet| {
        set.detect(&DetectInput {
            file_name: None,
            rel_path: None,
            first_line: Some("#!/usr/bin/env python3"),
        })
        .language
        .map(Language::name)
        .map(str::to_owned)
    };
    let before = d(&set);
    let reports = set.precompile();
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(before, d(&set));
    assert_eq!(before.as_deref(), Some("python"));
}

#[test]
fn 上の層に隠れた組めない先頭行も警告する() {
    let (set, _) = LanguageSet::build(&defs(
        "[[language]]\nname = \"x\"\nfirst_line = \"(\"\n[[language]]\nname = \"x\"\nfirst_line = \"^ok\"\n",
    ));
    let reports = set.precompile();
    let lines: Vec<u32> = reports
        .iter()
        .flat_map(|(_, ds)| ds.iter().map(|d| d.line))
        .collect();
    assert_eq!(lines, [3], "{reports:?}");
}

#[test]
fn auto_と_plain_は言語の名前に使えない() {
    assert!(!is_language_name("auto"));
    assert!(!is_language_name("plain"));
    assert!(is_language_name("autox"));
}

#[test]
fn 定義の数の上限を超えた分は使わずに_1_件警告する() {
    let text: String = (0..LANGUAGE_DEFS_MAX + 2)
        .map(|i| format!("[[language]]\nname = \"l{i}\"\nfile_types = [\"e{i}\"]\n"))
        .collect();
    let (d, diags) = load_language_str("languages/a.toml", &text);
    assert!(diags.is_empty(), "{diags:?}");
    let (set, reports) = LanguageSet::build(&d);
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].1.iter().map(|x| &x.kind).collect::<Vec<_>>(),
        [&DiagKind::Language(LanguageProblem::TooManyDefinitions)]
    );
    assert_eq!(detect(&set, "a.e255"), Some("l255"));
    assert_eq!(detect(&set, "a.e256"), None, "上限を超えた定義は使わない");
}

#[test]
fn 拡張子に先頭の点を書くと警告する() {
    let (d, diags) = load_language_str(
        "languages/a.toml",
        "[[language]]\nname = \"x\"\nfile_types = [\"x1\", \".x2\"]\n",
    );
    assert_eq!(diags.len(), 1);
    assert_eq!(
        (&diags[0].kind, diags[0].value.as_str(), diags[0].line),
        (&DiagKind::Language(LanguageProblem::FileTypeDot), ".x2", 3)
    );
    assert_eq!(d.len(), 1, "定義は使う");
}
