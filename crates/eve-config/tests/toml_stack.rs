//! 設定ファイル・キーマップ・テーマファイルの読込が、1 MiB のスタック (Windows のメインスレッドの既定。bin は端末の
//! 初期化の前にメインスレッドでこれらを読む、02 §9.5 の手順 2) で深い入れ子のファイルを読んでも、スタックを使い切らない
//! こと (docs/07 §4.2 の書ける構造の上限)。TOML パーサの入れ子の上限 (80) は、インラインの表・配列の入れ子と点のキーの
//! 段数を別々に数えるので、組み合わせると数千段の入れ子を作れる。構文を読む前の字句の走査で、入れ子の深さとキーの
//! 段数を抑える。言語ファイルは `locale_stack.rs`。

// 1 MiB のスタックのスレッドで読むための試験のスレッド (R11 の例外。src の外の試験だけで使う)
#![allow(clippy::disallowed_methods)]

use eve_config::{
    Config, DiagKind, Diagnostic, Severity, StructureProblem, TOML_MAX_KEY_DEPTH, TOML_MAX_NESTING,
    load_keymap_str, load_set, load_str, theme_file::load_theme,
};

/// 1 MiB のスタックのスレッドで `f` を走らせる (スタックを使い切ると、プロセスごと異常終了して試験が失敗する)。
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(f)
        .expect("試験のスレッドを作れる")
        .join()
        .expect("試験のスレッドが終わる")
}

fn dotted(n: usize) -> String {
    vec!["a"; n].join(".")
}

/// 設定ファイル・キーマップ・テーマファイルとして読んだ、それぞれの診断。
fn load_all(src: String) -> [Vec<Diagnostic>; 3] {
    on_small_stack(move || {
        let config = load_str(&src).diagnostics;
        let keymap = load_keymap_str(&src).diagnostics;
        let read = |_: &str| -> Result<Option<String>, String> { Ok(Some(src.clone())) };
        let (_, reports) = load_theme("deep", &read);
        let theme = reports.into_iter().flat_map(|(_, d)| d).collect();
        [config, keymap, theme]
    })
}

fn structure(d: &[Diagnostic]) -> Option<StructureProblem> {
    d.iter().find_map(|d| match d.kind {
        DiagKind::Structure(p) => Some(p),
        _ => None,
    })
}

/// 上限ちょうどの最悪の組み合わせ (入れ子の深さ 16 とキーの段数 16 を、それぞれ使い切る形) と、その 1 段深い形。
fn worst_cases() -> Vec<(&'static str, String, String)> {
    let n = TOML_MAX_NESTING;
    let k = TOML_MAX_KEY_DEPTH;
    let arrays = |m: usize| format!("{}1{}", "[".repeat(m), "]".repeat(m));
    // インラインの表の中に点のキーを書き、キーの段数を 4 段ずつ使う: `a.a.a.a = { a.a.a.a = { … = 値 } }`
    let tables = |levels: usize, last: usize, value: &str| {
        let mut s = format!("{} = ", dotted(4));
        for _ in 1..levels {
            s.push_str(&format!("{{ {} = ", dotted(4)));
        }
        s.push_str(&format!("{{ {} = {value}", dotted(last)));
        s.push_str(&" }".repeat(levels));
        s.push('\n');
        s
    };
    vec![
        (
            "見出しと点のキーの段数を使い切り、値は配列の入れ子",
            format!("[{}]\n{} = {}\n", dotted(k / 2), dotted(k / 2), arrays(n)),
            format!(
                "[{}]\n{} = {}\n",
                dotted(k / 2),
                dotted(k / 2),
                arrays(n + 1)
            ),
        ),
        (
            "点のキーを持つインラインの表と配列の入れ子",
            tables(3, 4, &arrays(n - 3)),
            tables(3, 4, &arrays(n - 2)),
        ),
        (
            "点のキーを持つインラインの表の入れ子 (キーの段数)",
            tables(3, 4, "1"),
            tables(3, 5, "1"),
        ),
        (
            "1 段のキーのインラインの表の入れ子",
            format!("a = {}1{}\n", "{ a = ".repeat(k - 1), " }".repeat(k - 1)),
            format!("a = {}1{}\n", "{ a = ".repeat(k), " }".repeat(k)),
        ),
        (
            "配列とインラインの表を交互に",
            format!(
                "{} = {}1{}\n",
                dotted(k - n / 2),
                "[{ a = ".repeat(n / 2),
                " }]".repeat(n / 2)
            ),
            format!(
                "{} = {}[1]{}\n",
                dotted(k - n / 2),
                "[{ a = ".repeat(n / 2),
                " }]".repeat(n / 2)
            ),
        ),
        (
            "配列の表の見出し",
            format!("[[{}]]\n", dotted(k)),
            format!("[[{}]]\n", dotted(k + 1)),
        ),
    ]
}

// verifies: FR-703
#[test]
fn 上限ちょうどの入れ子は_1_mib_のスタックで読め_1_段深いと読む前に拒む() {
    for (name, ok, over) in worst_cases() {
        for (i, d) in load_all(ok.clone()).iter().enumerate() {
            assert_eq!(structure(d), None, "{name} ({i}): {d:?}");
        }
        for (i, d) in load_all(over).iter().enumerate() {
            assert!(structure(d).is_some(), "{name} ({i}): {d:?}");
            assert_eq!(d.len(), 1, "{name} ({i}): {d:?}");
            assert_eq!(d[0].severity, Severity::Error);
        }
    }
}

// verifies: FR-703
#[test]
fn インラインの表と点のキーを組み合わせた入れ子は読む前に拒む() {
    // `{ a.a.…(79 段) = { a.a.…(79 段) = … } }` を 60 段。パーサの上限に掛からず、約 4,800 段の入れ子になる形
    let n = 60;
    let src = format!(
        "x = {}\"v\"{}\n",
        format!("{{ {} = ", dotted(79)).repeat(n),
        " }".repeat(n)
    );
    for d in load_all(src) {
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].severity, Severity::Error);
        assert_eq!(d[0].kind, DiagKind::Structure(StructureProblem::KeyDepth));
        assert_eq!((d[0].line, d[0].column), (1, 7));
    }
    // キーを使わない配列の入れ子は、17 段目の `[` で止まる
    let src = format!("x = {}1{}\n", "[".repeat(5000), "]".repeat(5000));
    for d in load_all(src) {
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].kind, DiagKind::Structure(StructureProblem::Nesting));
        assert_eq!((d[0].line, d[0].column), (1, 5 + 16));
    }
}

// verifies: FR-703
#[test]
fn set_の値の深い入れ子も読む前に拒む() {
    let value = format!("{}1{}", "{ a.a = ".repeat(3000), " }".repeat(3000));
    let l = on_small_stack(move || load_set(Config::default(), &format!("editor.x={value}")))
        .expect("= がある");
    assert_eq!(l.config, Config::default());
    // TOML の値として読めない (書ける構造の上限を超える) ので文字列として読み直す (`editor.x` は無いキーなので警告)
    assert_eq!(l.diagnostics.len(), 1, "{:?}", l.diagnostics);
    assert!(
        matches!(l.diagnostics[0].kind, DiagKind::UnknownKey { .. }),
        "{:?}",
        l.diagnostics
    );
}
