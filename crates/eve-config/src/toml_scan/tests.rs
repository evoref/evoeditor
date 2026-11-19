use super::*;

/// (入れ子の最大の深さ, キーの最大の段数)。
fn max_depths(text: &str) -> (usize, usize) {
    let (mut nesting, mut key) = (0, 0);
    let none: Option<()> = scan(text, |e| {
        match e {
            Event::Open { nesting: n, .. } => nesting = nesting.max(n),
            Event::Key { depth, .. } => key = key.max(depth),
            Event::ArrayHeader { .. } => {}
        }
        None
    });
    assert!(none.is_none());
    (nesting, key)
}

fn dotted(n: usize) -> String {
    vec!["a"; n].join(".")
}

/// `n` 段の配列の入れ子 (`[[…[1]…]]`)。
fn arrays(n: usize) -> String {
    format!("{}1{}", "[".repeat(n), "]".repeat(n))
}

#[test]
fn 入れ子の深さとキーの段数を数える() {
    // (本文, (入れ子の深さ, キーの段数))
    let cases: &[(&str, (usize, usize))] = &[
        ("", (0, 0)),
        ("a = 1\n", (0, 1)),
        ("[a.b]\nc.d = 1\n", (0, 4)),
        ("[[a.b]]\nc = 1\n", (0, 3)),
        // 見出しの段数は次の見出しで変わる
        ("[a.b.c]\n[d]\ne = 1\n", (0, 3)),
        // インラインの表の中のキーは、表を持つキーの段数を足す
        (
            "[ui]\nselection = { fg = \"x\", mono = { mods = [\"reverse\"] } }\n",
            (3, 4),
        ),
        ("x.y = { a.b = { c = 1 } }\n", (2, 5)),
        // 配列の要素の表は、配列を持つキーの段数を足す
        (
            "[keys.editor]\n\"f5\" = [ { command = \"a\", args = { b = 1 } }, \"c\" ]\n",
            (3, 5),
        ),
        // 閉じた後は外の表のキーに戻る
        ("a = { b = { c = 1 }, d = 2 }\n", (2, 3)),
        ("a = [{ b.c.d = 1 }, { e = 1 }]\n", (2, 4)),
        // 配列は複数行に書ける (改行で行を改めない)
        ("a = [\n  [1],\n  [ { b = 1 } ],\n]\nc = 1\n", (3, 2)),
        // TOML 1.1 のインラインの表の改行と注釈
        ("a = {\n  b = 1, # {\n  c = { d = 1 },\n}\n", (2, 3)),
        // 文字列と注釈の中は見ない
        ("a = \"{ [ \\\" { \"\n", (0, 1)),
        ("\"a.b\" = '[x]' # { [\n", (0, 1)),
        ("a = \"\"\"\n{ [\n\"\"\"\"\"\n", (0, 1)),
        ("a = '''\n[x]\n'''''\nb = 1\n", (0, 1)),
        // 小数・日付の `.` はキーの段ではない
        ("a = 1.5\nb = 1979-05-27T07:32:00.999\n", (0, 1)),
        // 種類の合わない閉じは数えない (構文エラー。深さを多めに見る)
        ("a = [ } [ 1 ]\n", (2, 1)),
    ];
    for &(src, want) in cases {
        assert_eq!(max_depths(src), want, "{src:?}");
    }
}

#[test]
fn キーの先頭と自身の段数を渡す() {
    let mut got = Vec::new();
    let none: Option<()> = scan("[a.b]\n  c.d = { e = 1, f.g = 2 }\n[[h]]\n", |e| {
        got.push(e);
        None
    });
    assert!(none.is_none());
    let key = |start, segments, depth| Event::Key {
        start,
        segments,
        depth,
    };
    assert_eq!(
        got,
        [
            key(0, 2, 2),
            key(8, 2, 4),
            Event::Open { at: 14, nesting: 1 },
            key(16, 1, 5),
            key(23, 2, 6),
            Event::ArrayHeader { at: 33 },
            key(33, 1, 1),
        ]
    );
}

// verifies: FR-703
#[test]
fn 上限ちょうどは通し_超えたら最初の所を示す() {
    let n = TOML_MAX_NESTING;
    let k = TOML_MAX_KEY_DEPTH;
    // (本文, 見つける位置と種類)
    let cases: Vec<(String, Option<(usize, StructureProblem)>)> = vec![
        // 配列の入れ子
        (format!("a = {}\n", arrays(n)), None),
        (
            format!("a = {}\n", arrays(n + 1)),
            Some((4 + n, StructureProblem::Nesting)),
        ),
        // インラインの表の入れ子 (キーの段数も 1 段ずつ増える)
        (
            format!("a = {}1{}\n", "{ b = ".repeat(k - 1), " }".repeat(k - 1)),
            None,
        ),
        (
            format!("a = {}1{}\n", "{ b = ".repeat(k), " }".repeat(k)),
            Some((4 + 6 * (k - 1) + 2, StructureProblem::KeyDepth)),
        ),
        // 見出しと点のキーの和
        (
            format!("[{}]\n{} = 1\n", dotted(k / 2), dotted(k / 2)),
            None,
        ),
        (
            format!("[{}]\n{} = 1\n", dotted(k / 2), dotted(k / 2 + 1)),
            Some((2 * (k / 2) + 2, StructureProblem::KeyDepth)),
        ),
        (format!("[[{}]]\n", dotted(k)), None),
        (
            format!("[[{}]]\n", dotted(k + 1)),
            Some((0, StructureProblem::KeyDepth)),
        ),
        // パーサの上限 (80) の内側の、点のキーとインラインの表の組み合わせ (07 §4.2 の理由の形)
        (
            format!(
                "x = {}1{}\n",
                format!("{{ {} = ", dotted(79)).repeat(60),
                " }".repeat(60)
            ),
            Some((6, StructureProblem::KeyDepth)),
        ),
        // 配列の中のインラインの表 (キーの段数は配列を持つキーから数える)
        (
            format!(
                "{} = [{{ {} = {} }}]\n",
                dotted(8),
                dotted(8),
                arrays(n - 2)
            ),
            None,
        ),
        (
            format!("{} = [{{ {} = 1 }}]\n", dotted(8), dotted(9)),
            Some((21, StructureProblem::KeyDepth)),
        ),
    ];
    for (src, want) in &cases {
        assert_eq!(check_structure(src), *want, "{src:?}");
    }
}

/// 今の書式で正当に使う構造 (07 §4・§7・§8 の例と、配布する設定例) は、上限より十分に浅い。
#[test]
fn 正当なファイルは上限の半分以下() {
    let theme = r##"
[meta]
name = "my-theme"
inherits = "default"
[palette]
bg = "#1e1e2e"
[ui]
cursor.secondary= { fg = "bg", bg = "accent" }
selection       = { fg = "#cdd6f4", bg = "#45475a",
                    fallback256 = { fg = "ansi.189", bg = "ansi.238" },
                    fallback16  = { fg = "ansi.bright_white", bg = "ansi.blue" },
                    mono        = { mods = ["reverse"] } }
line_number.active = { fg = "fg", mods = ["bold"] }
virtual_text.ghost = { fg = "#6c7086", mods = ["italic"],
                       fallback256 = { fg = "ansi.243" }, fallback16 = { mods = ["dim"] }, mono = { mods = ["dim"] } }
[syntax]
keyword.control.return = { fg = "#cba6f7", fallback256 = { fg = "ansi.183" }, mono = { mods = ["bold"] } }
"##;
    let keymap = r#"
[keys.editor]
"ctrl+s"        = "file.save"
"alt+up"        = { command = "edit.move_line_up" }
"f5"            = { command = "lang.format", args = { on_save = false } }
"f6"            = [ { command = "a.b", when = "doc.modified", args = { list = [1, 2] } }, "c.d" ]
"#;
    let config = r##"
[os.windows.theme.overrides]
"ui.selection" = { bg = "#444444", mono = { mods = ["reverse"] } }
[os.macos.keys.editor]
"super+s" = "file.save"
"##;
    for (src, want) in [
        (theme, (3, 6)),
        (keymap, (4, 5)),
        (config, (3, 7)),
        (include_str!("../../../../dist/config.example.toml"), (0, 3)),
    ] {
        let got = max_depths(src);
        assert_eq!(got, want, "{src}");
        assert!(got.0 * 2 <= TOML_MAX_NESTING && got.1 * 2 <= TOML_MAX_KEY_DEPTH);
        assert_eq!(check_structure(src), None);
    }
}
