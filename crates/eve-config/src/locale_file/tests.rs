//! ユーザーの言語ファイルの検査と重ね合わせの L0 の表駆動の試験 (docs/09 §3.4 の (1)、07 §17.4、01 NFR-603 AC-9)。

use super::*;

fn base() -> Catalog {
    Catalog::parse(
        r#"
[meta]
language = "ja"

[msg.save.failed]
text = "{path} を保存できませんでした: {reason}"
hint = "別名で保存 ({key:file.save_as}) してください"

[msg.open.twice]
text = "{path} と {path} は同じです"
hint = "閉じてください〔 (一時ファイル {tmp})〕"

[ui.confirm.unsaved]
save = "[S] 保存して終了"
discard = "[D] 保存せずに終了"
message = "[S] か [D] を押します"

[hint.app]
quit = "終了"

[status]
untitled = "無題-{n}"
"#,
    )
    .unwrap()
}

/// 検査に通ったか (`Pass` か `Warn`)。
fn used(c: &TextCheck) -> bool {
    !matches!(c, TextCheck::Reject(_))
}

fn problem(c: &TextCheck) -> Option<&LocaleProblem> {
    match c {
        TextCheck::Pass => None,
        TextCheck::Warn(p) | TextCheck::Reject(p) => Some(p),
    }
}

fn bad(class: CharClass, c: char) -> Option<LocaleProblem> {
    Some(LocaleProblem::BadChar {
        class,
        code: u32::from(c),
    })
}

fn placeholder(names: &[&str]) -> Option<LocaleProblem> {
    Some(LocaleProblem::Placeholder {
        expected: names.iter().map(|s| (*s).to_owned()).collect(),
        issue: PlaceholderIssue::Unbalanced,
    })
}

/// 表の比べ方: プレースホルダの問題は、何が合わないか (`issue`、別の試験で見る) を除いて比べる。
fn erase_issue(p: Option<&LocaleProblem>) -> Option<LocaleProblem> {
    p.cloned().map(|p| match p {
        LocaleProblem::Placeholder { expected, .. } => LocaleProblem::Placeholder {
            expected,
            issue: PlaceholderIssue::Unbalanced,
        },
        other => other,
    })
}

// verifies: AC-NFR-603-9
#[test]
fn 検査_2_から_9_の表() {
    let marks31 = format!("e{}", "\u{0301}".repeat(31));
    let m20 = "\u{0301}".repeat(20);
    let marks_cond = format!("e{m20}〔{m20}〕");
    let marks_ph = format!("e{m20}{{x}}{m20}");
    let marks_zwj = format!("e{m20}\u{200C}{m20}");
    // 〔 〕 の中身が消えると、前後の結合文字が続く
    let marks_removed = format!("e{m20}〔a〕{m20}");
    // 基底の文字を挟めば数を戻す
    let marks_reset = format!("e{m20}a{m20}");
    let marks30 = format!("e{}", "\u{0301}".repeat(30));
    let long = "あ".repeat(1025);
    let max = "あ".repeat(1024);
    // (キー, 文言, 同梱の文言, 使うか, 期待する問題)
    let cases: Vec<(&str, &str, &str, bool, Option<LocaleProblem>)> = vec![
        // 2. 長さ
        ("a.b", &max, "x", true, None),
        (
            "a.b",
            &long,
            "x",
            false,
            Some(LocaleProblem::TooLong { count: 1025 }),
        ),
        // 3. 空でない
        ("a.b", "", "x", false, Some(LocaleProblem::Empty)),
        ("a.b", "   ", "x", false, Some(LocaleProblem::Empty)),
        ("a.b", "\u{3000}", "x", false, Some(LocaleProblem::Empty)),
        (
            "a.b",
            "\u{200D}\u{200C}",
            "x",
            false,
            Some(LocaleProblem::Empty),
        ),
        ("a.b", " x ", "x", true, None),
        ("a.b", "\u{0301}", "x", true, None),
        // 4. 文字の種類
        ("a.b", "a\tb", "x", false, bad(CharClass::Control, '\t')),
        ("a.b", "a\nb", "x", false, bad(CharClass::Control, '\n')),
        (
            "a.b",
            "\u{1B}[2J",
            "x",
            false,
            bad(CharClass::Control, '\u{1B}'),
        ),
        (
            "a.b",
            "a\u{7F}",
            "x",
            false,
            bad(CharClass::Control, '\u{7F}'),
        ),
        (
            "a.b",
            "a\u{9B}",
            "x",
            false,
            bad(CharClass::Control, '\u{9B}'),
        ),
        (
            "a.b",
            "a\u{2028}",
            "x",
            false,
            bad(CharClass::Control, '\u{2028}'),
        ),
        (
            "a.b",
            "a\u{202E}b",
            "x",
            false,
            bad(CharClass::Format, '\u{202E}'),
        ),
        (
            "a.b",
            "a\u{2067}b",
            "x",
            false,
            bad(CharClass::Format, '\u{2067}'),
        ),
        (
            "a.b",
            "a\u{200F}",
            "x",
            false,
            bad(CharClass::Format, '\u{200F}'),
        ),
        (
            "a.b",
            "a\u{061C}",
            "x",
            false,
            bad(CharClass::Format, '\u{061C}'),
        ),
        (
            "a.b",
            "a\u{200B}",
            "x",
            false,
            bad(CharClass::Format, '\u{200B}'),
        ),
        (
            "a.b",
            "a\u{FEFF}",
            "x",
            false,
            bad(CharClass::Format, '\u{FEFF}'),
        ),
        (
            "a.b",
            "\u{FDD0}",
            "x",
            false,
            bad(CharClass::Noncharacter, '\u{FDD0}'),
        ),
        (
            "a.b",
            "\u{10FFFF}",
            "x",
            false,
            bad(CharClass::Noncharacter, '\u{10FFFF}'),
        ),
        (
            "a.b",
            "\u{E000}",
            "x",
            false,
            bad(CharClass::PrivateUse, '\u{E000}'),
        ),
        (
            "a.b",
            "\u{100000}",
            "x",
            false,
            bad(CharClass::PrivateUse, '\u{100000}'),
        ),
        // 制御文字だけの文言は、空ではなく使えない文字として知らせる
        ("a.b", "\t", "x", false, bad(CharClass::Control, '\t')),
        // ZWJ の絵文字の列と ZWNJ は使える
        ("a.b", "👨\u{200D}👩\u{200D}👧", "x", true, None),
        ("a.b", "می\u{200C}خواهم", "x", true, None),
        // 5. 結合文字の数
        ("a.b", &marks30, "x", true, None),
        (
            "a.b",
            &marks31,
            "x",
            false,
            bad(CharClass::CombiningMarks, '\u{0301}'),
        ),
        // 6. 括弧の対応 (名前の比較の前)
        ("a.b", "{path", "{path}", false, placeholder(&["path"])),
        ("a.b", "path}", "{path}", false, placeholder(&["path"])),
        ("a.b", "{pa{th}}", "{path}", false, placeholder(&["path"])),
        ("a.b", "{{path}", "{path}", false, placeholder(&["path"])),
        ("a.b", "{{x}} {path}", "{path}", true, None),
        ("a.b", "〔{tmp}", "〔{tmp}〕", false, placeholder(&["tmp"])),
        ("a.b", "{tmp}〕", "〔{tmp}〕", false, placeholder(&["tmp"])),
        (
            "a.b",
            "〔〔{tmp}〕〕",
            "〔{tmp}〕",
            false,
            placeholder(&["tmp"]),
        ),
        // 7. プレースホルダの多重集合と 〔 〕
        (
            "a.b",
            "{path} だめ",
            "{path} {reason}",
            false,
            placeholder(&["path", "reason"]),
        ),
        (
            "a.b",
            "{path} {other}",
            "{path}",
            false,
            placeholder(&["path"]),
        ),
        (
            "a.b",
            "{path}{path}",
            "{path}",
            false,
            placeholder(&["path"]),
        ),
        ("a.b", "{path}", "{path} {path}", true, None),
        ("a.b", "{reason}: {path}", "{path} {reason}", true, None),
        ("a.b", "{key:file.save} で", "{key:file.save}", true, None),
        (
            "a.b",
            "{key:file.open} で",
            "{key:file.save}",
            false,
            placeholder(&["key:file.save"]),
        ),
        ("a.b", "{tmp}", "〔{tmp}〕", false, placeholder(&["tmp"])),
        ("a.b", "x 〔{tmp}〕", "{tmp}", false, placeholder(&["tmp"])),
        // 〔 〕 の中と外の多重集合をそれぞれ比べる (外に出すと、引数が無いとき `{tmp}` のまま出る)
        (
            "a.b",
            "〔x〕 {tmp}",
            "〔({tmp})〕 ok",
            false,
            placeholder(&["tmp"]),
        ),
        (
            "a.b",
            "〔({tmp})〕 ok",
            "{tmp} 〔x〕",
            false,
            placeholder(&["tmp"]),
        ),
        ("a.b", "ok 〔[{tmp}]〕", "〔({tmp})〕 ok", true, None),
        ("a.b", "x 〔({tmp})〕", "〔{tmp}〕", true, None),
        // 8. `[X]` の並びと位置、全角の似た形
        ("a.b", "[S] Save", "[S] 保存", true, None),
        (
            "a.b",
            "[D] Save",
            "[S] 保存",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        (
            "a.b",
            "Save [S]",
            "[S] 保存",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        (
            "a.b",
            "Save",
            "[S] 保存",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        ("a.b", "[S] or [D]", "[S] か [D]", true, None),
        (
            "a.b",
            "[D] or [S]",
            "[S] か [D]",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        (
            "a.b",
            "Press [S] or [D]",
            "[S] か [D] を押す",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        (
            "a.b",
            "[S] [X] Save",
            "[S] 保存",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        (
            "a.b",
            "［S］ Save",
            "保存",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        (
            "a.b",
            "[Ｓ] Save",
            "保存",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        (
            "a.b",
            "[S] ［S］",
            "[S] 保存",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        ("a.b", "[Esc] Cancel", "[Esc] キャンセル", true, None),
        // 9. ヒントの幅 (警告して使う)
        ("hint.app.quit", "終了する", "終了", true, None),
        ("hint.app.quit", "Quit app", "終了", true, None),
        (
            "hint.app.quit",
            "終了します",
            "終了",
            true,
            Some(LocaleProblem::HintWidth { width: 10 }),
        ),
        (
            "hint.app.quit",
            "Quit now!",
            "終了",
            true,
            Some(LocaleProblem::HintWidth { width: 9 }),
        ),
        // ヒントでないキーには幅の上限が無い
        ("ui.a", "終了します終了します", "終了", true, None),
        // 曖昧幅は 1 で数える (`→` は東アジアの曖昧幅)
        ("hint.app.quit", "→→→→→→→→", "終了", true, None),
        (
            "hint.app.quit",
            "→→→→→→→→→",
            "終了",
            true,
            Some(LocaleProblem::HintWidth { width: 9 }),
        ),
        // 異体字セレクタ・タグ文字・Default_Ignorable でない Cf も拒む (検査 4)
        (
            "a.b",
            "\u{26A0}\u{FE0F} 注意",
            "x",
            false,
            bad(CharClass::Format, '\u{FE0F}'),
        ),
        (
            "a.b",
            "葛\u{E0100}",
            "x",
            false,
            bad(CharClass::Format, '\u{E0100}'),
        ),
        (
            "a.b",
            "\u{1F3F4}\u{E0067}\u{E0062}\u{E007F}",
            "x",
            false,
            bad(CharClass::Format, '\u{E0067}'),
        ),
        (
            "a.b",
            "a\u{0600}",
            "x",
            false,
            bad(CharClass::Format, '\u{0600}'),
        ),
        (
            "a.b",
            "a\u{FFF9}b",
            "x",
            false,
            bad(CharClass::Format, '\u{FFF9}'),
        ),
        (
            "a.b",
            "a\u{13430}",
            "x",
            false,
            bad(CharClass::Format, '\u{13430}'),
        ),
        // 〔 〕 を使った迂回 (検査 3・5・8 は 〔 〕 を解いた形でも行う)
        (
            "a.b",
            "〔{x}〕",
            "〔{x}〕",
            false,
            Some(LocaleProblem::Empty),
        ),
        ("a.b", "〔 〕 {x}", "〔 〕 {x}", true, None),
        (
            "a.b",
            "[〔〕S] Save",
            "保存〔x〕",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        (
            "a.b",
            "[〔{x}〕S] Save",
            "保存〔{x}〕",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        (
            "a.b",
            "[{x}S] Save",
            "保存 {x}",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        (
            "a.b",
            "〔[S]〕 Save",
            "保存〔x〕",
            false,
            Some(LocaleProblem::Accelerator),
        ),
        ("a.b", "〔[S] 保存〕 ok", "〔[S] 保存〕 ok", true, None),
        (
            "a.b",
            "〔[S] 保存〕",
            "〔[S] 保存〕",
            false,
            Some(LocaleProblem::Empty),
        ),
        // 5. 結合文字は書記素ごとではなく続く数で数え、〔 〕・プレースホルダ・ZWJ / ZWNJ では数を戻さない
        (
            "a.b",
            &marks_cond,
            "〔x〕",
            false,
            bad(CharClass::CombiningMarks, '\u{0301}'),
        ),
        (
            "a.b",
            &marks_ph,
            "{x}",
            false,
            bad(CharClass::CombiningMarks, '\u{0301}'),
        ),
        (
            "a.b",
            &marks_zwj,
            "x",
            false,
            bad(CharClass::CombiningMarks, '\u{0301}'),
        ),
        (
            "a.b",
            &marks_removed,
            "〔x〕",
            false,
            bad(CharClass::CombiningMarks, '\u{0301}'),
        ),
        ("a.b", &marks_reset, "x", true, None),
    ];
    for (key, text, builtin, want_used, want) in &cases {
        let got = check_text(key, text, builtin);
        assert_eq!(used(&got), *want_used, "{key} {text:?}: {got:?}");
        assert_eq!(erase_issue(problem(&got)), want.clone(), "{key} {text:?}");
        if let Some(LocaleProblem::HintWidth { .. }) = want {
            assert!(matches!(got, TextCheck::Warn(_)), "{got:?}");
        }
    }
}

fn load(src: &str) -> LocaleFile {
    load_locale_file("ja", src.as_bytes(), &base(), LOCALE_MAX_DIAGNOSTICS)
}

fn kinds(f: &LocaleFile) -> Vec<(String, DiagKind, u32)> {
    f.diagnostics
        .iter()
        .map(|d| (d.key.clone(), d.kind.clone(), d.line))
        .collect()
}

fn locale(p: LocaleProblem) -> DiagKind {
    DiagKind::Locale(p)
}

// verifies: AC-NFR-603-9
#[test]
fn 一つの文言だけを上書きし_ほかは同梱のまま() {
    let f = load("[msg.save.failed]\ntext = \"{path}: 保存失敗 ({reason})\"\n");
    assert!(f.diagnostics.is_empty(), "{:?}", f.diagnostics);
    let c = base().overlay(&f);
    assert_eq!(
        c.get("msg.save.failed.text"),
        Some("{path}: 保存失敗 ({reason})")
    );
    // text だけを変え、hint は同梱のまま (別のキーとして重ねる)
    assert_eq!(
        c.get("msg.save.failed.hint"),
        base().get("msg.save.failed.hint")
    );
    assert_eq!(c.get("status.untitled"), Some("無題-{n}"));
    assert_eq!(c.len(), base().len());
}

// verifies: AC-NFR-603-9
#[test]
fn 重ね方の表() {
    // (本文, 期待する診断 (キー, 種類, 行), 重ねた後に確かめるキーと文言)
    type Case = (
        &'static str,
        Vec<(&'static str, DiagKind, u32)>,
        Vec<(&'static str, &'static str)>,
    );
    let cases: Vec<Case> = vec![
        // 同梱に無いキー (綴りの誤り・新しい版のキー) は使わず、候補は付けない
        (
            "[status]\nuntitle = \"x\"\nuntitled = \"無-{n}\"\n",
            vec![("status.untitle", locale(LocaleProblem::UnknownKey), 2)],
            vec![("status.untitled", "無-{n}")],
        ),
        // 複数形の接尾辞は、同梱がその形で持つキーにだけ書ける
        (
            "[status.untitled]\nother = \"x\"\n",
            vec![(
                "status.untitled",
                DiagKind::TypeMismatch {
                    expected: "string",
                    found: "table",
                },
                1,
            )],
            vec![("status.untitled", "無題-{n}")],
        ),
        // 上書きできない名前空間
        (
            "[stderr.quit]\nwaiting_save = \"x\"\n[panic]\nmessage = \"y\"\n[cli.help]\nusage = \"z\"\n",
            vec![
                (
                    "stderr.quit.waiting_save",
                    locale(LocaleProblem::Reserved),
                    2,
                ),
                ("panic.message", locale(LocaleProblem::Reserved), 4),
                ("cli.help.usage", locale(LocaleProblem::Reserved), 6),
            ],
            vec![],
        ),
        // 平らにしたキーの重複: 最初に現れた文言を使う
        (
            "[hint]\n\"app.quit\" = \"一\"\n[hint.app]\nquit = \"二\"\n",
            vec![("hint.app.quit", locale(LocaleProblem::DuplicateKey), 4)],
            vec![("hint.app.quit", "一")],
        ),
        // 最初の文言が検査に通らなくても、後の文言は使わない (同梱の文言になる)
        (
            "[hint]\n\"app.quit\" = \"\\t\"\n[hint.app]\nquit = \"二\"\n",
            vec![
                (
                    "hint.app.quit",
                    locale(LocaleProblem::BadChar {
                        class: CharClass::Control,
                        code: 9,
                    }),
                    2,
                ),
                ("hint.app.quit", locale(LocaleProblem::DuplicateKey), 4),
            ],
            vec![("hint.app.quit", "終了")],
        ),
        // 検査 1: 値が文字列でない
        (
            "[status]\nuntitled = 3\n[hint.app]\nquit = true\n",
            vec![
                (
                    "status.untitled",
                    DiagKind::TypeMismatch {
                        expected: "string",
                        found: "integer",
                    },
                    2,
                ),
                (
                    "hint.app.quit",
                    DiagKind::TypeMismatch {
                        expected: "string",
                        found: "boolean",
                    },
                    4,
                ),
            ],
            vec![("status.untitled", "無題-{n}"), ("hint.app.quit", "終了")],
        ),
        // 同梱に無いキーなら、型より先に同梱に無いことを知らせる
        (
            "x = 1\n",
            vec![("x", locale(LocaleProblem::UnknownKey), 1)],
            vec![],
        ),
        // 検査 9 は警告して使う
        (
            "[hint.app]\nquit = \"終了します\"\n",
            vec![(
                "hint.app.quit",
                locale(LocaleProblem::HintWidth { width: 10 }),
                2,
            )],
            vec![("hint.app.quit", "終了します")],
        ),
        // 検査 8 に通らない文言は使わない
        (
            "[ui.confirm.unsaved]\nsave = \"保存 [S]\"\ndiscard = \"[D] 破棄\"\n",
            vec![(
                "ui.confirm.unsaved.save",
                locale(LocaleProblem::Accelerator),
                2,
            )],
            vec![
                ("ui.confirm.unsaved.save", "[S] 保存して終了"),
                ("ui.confirm.unsaved.discard", "[D] 破棄"),
            ],
        ),
        // 検査 7: 同じ名前を同梱より多く書いた文言 (差し込む文字列を増やす) は使わない
        (
            "[msg.save.failed]\ntext = \"{path}{path}{path} {reason}\"\n",
            vec![(
                "msg.save.failed.text",
                locale(LocaleProblem::Placeholder {
                    expected: vec!["path".into(), "reason".into()],
                    issue: PlaceholderIssue::TooMany,
                }),
                2,
            )],
            vec![(
                "msg.save.failed.text",
                "{path} を保存できませんでした: {reason}",
            )],
        ),
        // `[meta]` は文言ではない。language はファイル名と同じ言語なら地域が付いてもよく、ほかのキーは読み飛ばす
        (
            "[meta]\nlanguage = \"JA-jp\"\nauthor = \"x\"\n",
            vec![],
            vec![],
        ),
        (
            "[meta]\nlanguage = \"fr\"\n[status]\nuntitled = \"無-{n}\"\n",
            vec![(
                "meta.language",
                locale(LocaleProblem::LanguageMismatch { lang: "ja".into() }),
                2,
            )],
            vec![("status.untitled", "無-{n}")],
        ),
        (
            "[meta]\nlanguage = 1\n",
            vec![(
                "meta.language",
                DiagKind::TypeMismatch {
                    expected: "string",
                    found: "integer",
                },
                2,
            )],
            vec![],
        ),
        (
            "meta = \"ja\"\n",
            vec![(
                "meta",
                DiagKind::TypeMismatch {
                    expected: "table",
                    found: "string",
                },
                1,
            )],
            vec![],
        ),
    ];
    for (src, want_diags, want_texts) in cases {
        let f = load(src);
        let want: Vec<(String, DiagKind, u32)> = want_diags
            .into_iter()
            .map(|(k, d, l)| (k.to_owned(), d, l))
            .collect();
        assert_eq!(kinds(&f), want, "{src}");
        // キーごとの問題は警告。検査 9 (ヒントの幅) だけは情報 (使って、画面の要約に出さない)
        assert!(
            f.diagnostics.iter().all(|d| d.severity
                == if matches!(d.kind, DiagKind::Locale(LocaleProblem::HintWidth { .. })) {
                    Severity::Info
                } else {
                    Severity::Warning
                }),
            "{src}"
        );
        let c = base().overlay(&f);
        for (k, v) in want_texts {
            assert_eq!(c.get(k), Some(v), "{src}: {k}");
        }
        assert_eq!(c.len(), base().len(), "同梱に無いキーは足さない: {src}");
    }
}

#[test]
fn language_mismatch_の値は書かれた値() {
    let f = load("[meta]\nlanguage = \"f\\u001br\"\n");
    assert_eq!(f.diagnostics.len(), 1);
    // 値はそのまま持ち、表示の前に無害化する (sanitize_untrusted)
    assert_eq!(f.diagnostics[0].value, "f\u{1B}r");
}

// verifies: AC-NFR-603-9
#[test]
fn ファイル全体の問題はエラー一件で全体を使わない() {
    let b = base();
    let at_limit = format!("# {}\n", "a".repeat(LOCALE_MAX_BYTES - 3));
    assert_eq!(at_limit.len(), LOCALE_MAX_BYTES);
    let over = format!("{at_limit}\n");
    // (本文, 期待する問題, 行)
    let cases: Vec<(Vec<u8>, Option<LocaleProblem>, u32)> = vec![
        (at_limit.into_bytes(), None, 0),
        (over.into_bytes(), Some(LocaleProblem::TooLarge), 0),
        (
            b"[status]\nuntitled = \"\xFF\"\n".to_vec(),
            Some(LocaleProblem::NotUtf8),
            2,
        ),
        (
            b"\xEF\xBB\xBF[status]\nuntitled = \"a{n}\"\n".to_vec(),
            None,
            0,
        ),
        (
            b"\xEF\xBB\xBFx = \n".to_vec(),
            Some(LocaleProblem::Syntax {
                message: String::new(),
            }),
            1,
        ),
        (
            b"[status\nuntitled = \"a\"\n".to_vec(),
            Some(LocaleProblem::Syntax {
                message: String::new(),
            }),
            1,
        ),
        (
            b"[status]\nuntitled = \"a\"\nuntitled = \"b\"\n".to_vec(),
            Some(LocaleProblem::Syntax {
                message: String::new(),
            }),
            3,
        ),
    ];
    for (bytes, want, line) in cases {
        let f = load_locale_file("ja", &bytes, &b, LOCALE_MAX_DIAGNOSTICS);
        match want {
            None => {
                assert!(f.diagnostics.is_empty(), "{:?}", f.diagnostics);
                assert!(!f.is_unusable());
            }
            Some(p) => {
                assert_eq!(f.diagnostics.len(), 1, "{:?}", f.diagnostics);
                let d = &f.diagnostics[0];
                assert_eq!(d.severity, Severity::Error);
                assert_eq!(d.line, line, "{d:?}");
                match (&d.kind, &p) {
                    (
                        DiagKind::Locale(LocaleProblem::Syntax { message }),
                        LocaleProblem::Syntax { .. },
                    ) => assert_ne!(message, ""),
                    (DiagKind::Locale(got), want) => assert_eq!(got, want),
                    other => panic!("{other:?}"),
                }
                assert!(f.texts.is_empty());
                assert!(f.is_unusable());
                assert_eq!(
                    b.overlay(&f).get("status.untitled"),
                    b.get("status.untitled")
                );
            }
        }
    }
}

// verifies: AC-NFR-603-9
#[test]
fn 診断は_100_件と_ほか_n_件() {
    let mut src = String::new();
    for i in 0..150 {
        src.push_str(&format!("k{i:03} = \"x\"\n"));
    }
    let f = load(&src);
    assert_eq!(f.diagnostics.len(), 101);
    for (i, d) in f.diagnostics[..100].iter().enumerate() {
        assert_eq!(d.kind, locale(LocaleProblem::UnknownKey));
        // 書かれた順 (行の順)
        assert_eq!(d.line as usize, i + 1);
        assert_eq!(d.key, format!("k{i:03}"));
    }
    let last = &f.diagnostics[100];
    assert_eq!(last.kind, locale(LocaleProblem::More { count: 50 }));
    assert_eq!((last.line, last.column), (0, 0));
    // ちょうど 100 件なら「ほか N 件」は付けない
    let src100: String = (0..100).map(|i| format!("k{i:03} = \"x\"\n")).collect();
    assert_eq!(load(&src100).diagnostics.len(), 100);
    // 上限を広げれば全件
    let all = load_locale_file("ja", src.as_bytes(), &base(), usize::MAX);
    assert_eq!(all.diagnostics.len(), 150);
}

#[test]
fn 診断の行と桁() {
    let f = load("[status]\n  untitled = \"\\u001b\"\n\n[x]\ny = \"1\"\n");
    let pos: Vec<(u32, u32)> = f.diagnostics.iter().map(|d| (d.line, d.column)).collect();
    assert_eq!(pos, [(2, 3), (5, 1)]);
}

// verifies: AC-NFR-603-9
#[test]
fn 深い入れ子は止まらずパニックしない() {
    let n = 5000;
    let inputs = [
        // 点で区切ったキー
        format!("{} = \"x\"\n", vec!["a"; n].join(".")),
        // 表の見出し
        format!("[{}]\nb = \"x\"\n", vec!["a"; n].join(".")),
        // インラインの表
        format!("a = {}\"x\"{}\n", "{ b = ".repeat(n), " }".repeat(n)),
        // 配列
        format!("a = {}{}\n", "[".repeat(n), "]".repeat(n)),
    ];
    for (i, src) in inputs.iter().enumerate() {
        let f = load(src);
        assert!(f.texts.is_empty());
        assert_eq!(f.diagnostics.len(), 1, "{:?}", f.diagnostics);
        let d = &f.diagnostics[0];
        assert_eq!(d.severity, Severity::Error);
        // 構文を読む前に拒む: 段数の多いキーは TooDeep、インラインの表と配列は InlineValue
        let want = if i < 2 {
            LocaleProblem::TooDeep
        } else {
            LocaleProblem::InlineValue
        };
        assert_eq!(d.kind, locale(want), "{d:?}");
        // 字句の走査を通らなくても、TOML パーサは既定の上限 (無制限にする機能 `unbounded` を有効にしていない) で止まる
        assert!(DeTable::parse(src).is_err(), "{i}");
    }
    // 8 段までは読める (同梱に無いキーとして知らせる)。9 段は拒む
    for (src, want) in [
        (
            format!("{} = \"x\"\n", ["a"; 8].join(".")),
            LocaleProblem::UnknownKey,
        ),
        (
            format!("[{}]\n{} = \"x\"\n", ["a"; 5].join("."), ["b"; 3].join(".")),
            LocaleProblem::UnknownKey,
        ),
        (
            format!("{} = \"x\"\n", ["a"; 9].join(".")),
            LocaleProblem::TooDeep,
        ),
        (
            format!("[{}]\n{} = \"x\"\n", ["a"; 5].join("."), ["b"; 4].join(".")),
            LocaleProblem::TooDeep,
        ),
        (
            format!("[{}]\n", ["a"; 9].join(".")),
            LocaleProblem::TooDeep,
        ),
        // 引用符の中の `.` は段に数えない
        (
            format!("\"{}\" = \"x\"\n", vec!["a"; 20].join(".")),
            LocaleProblem::UnknownKey,
        ),
    ] {
        let f = load(&src);
        assert_eq!(f.diagnostics.len(), 1, "{src}");
        assert_eq!(f.diagnostics[0].kind, locale(want), "{src}");
    }
}

#[test]
fn 長いキーは切って同梱に無いキーにする() {
    let long = "a".repeat(300_000);
    let mut src = format!("[\"{long}\"]\n");
    for i in 0..1000 {
        src.push_str(&format!("k{i} = \"x\"\n"));
    }
    let f = load_locale_file("ja", src.as_bytes(), &base(), usize::MAX);
    assert_eq!(f.diagnostics.len(), 1000);
    for d in &f.diagnostics {
        assert_eq!(d.kind, locale(LocaleProblem::UnknownKey));
        assert!(d.key.len() <= MAX_KEY_BYTES, "{}", d.key.len());
    }
    // 予約の接頭辞は、長くても予約として知らせる
    let reserved = format!("[\"stderr\".\"{long}\"]\nx = \"y\"\n");
    let f = load(&reserved);
    assert_eq!(f.diagnostics[0].kind, locale(LocaleProblem::Reserved));
}

#[test]
fn キーの長さの上限の境界() {
    let a = |n: usize| "a".repeat(n);
    // (本文, 期待するキー)。上限は 256 バイトで、超えたキーは 256 バイトまでで切る
    let cases: Vec<(String, String)> = vec![
        // 1 つ前の段でちょうど上限になり、`.` を足すと超える
        (format!("[{}]\nb = \"x\"\n", a(256)), a(256)),
        (
            format!("[{}.{}]\nz = \"x\"\n", a(205), "b".repeat(50)),
            format!("{}.{}", a(205), "b".repeat(50)),
        ),
        (format!("{}.b = \"x\"\n", a(256)), a(256)),
        // 葉のキーの合計が 255・256・257 バイト
        (
            format!("[{}]\nb = \"x\"\n", a(253)),
            format!("{}.b", a(253)),
        ),
        (
            format!("[{}]\nb = \"x\"\n", a(254)),
            format!("{}.b", a(254)),
        ),
        (
            format!("[{}]\nbc = \"x\"\n", a(254)),
            format!("{}.b", a(254)),
        ),
        // 上限を超えた段の下に、もう 1 段
        (
            format!("[{}.c]\nd = \"x\"\n", a(255)),
            format!("{}.", a(255)),
        ),
        // 多バイト文字の途中では切らない
        (
            format!("[\"{}\"]\nb = \"x\"\n", "あ".repeat(100)),
            "あ".repeat(85),
        ),
    ];
    for (src, want) in cases {
        let f = load_locale_file("ja", src.as_bytes(), &base(), usize::MAX);
        assert_eq!(f.diagnostics.len(), 1, "{:?}", f.diagnostics);
        let d = &f.diagnostics[0];
        assert_eq!(d.kind, locale(LocaleProblem::UnknownKey));
        assert!(d.key.len() <= MAX_KEY_BYTES, "{}", d.key.len());
        assert_eq!(d.key, want, "{}", src.len());
    }
}

// verifies: AC-NFR-603-9
#[test]
fn 補う言語() {
    assert!(std::ptr::eq(Catalog::base_for("ja"), Catalog::builtin_ja()));
    assert!(std::ptr::eq(Catalog::base_for("JA"), Catalog::builtin_ja()));
    assert!(std::ptr::eq(Catalog::base_for("en"), Catalog::builtin_en()));
    assert!(std::ptr::eq(Catalog::base_for("fr"), Catalog::builtin_en()));
    // 追加言語: 書いたキーは使い、書いていないキーは en の文言
    let en = Catalog::base_for("fr");
    let f = load_locale_file(
        "fr",
        "[meta]\nlanguage = \"fr\"\n[info.save]\ndone = \"{path} enregistré\"\n".as_bytes(),
        en,
        LOCALE_MAX_DIAGNOSTICS,
    );
    assert!(f.diagnostics.is_empty(), "{:?}", f.diagnostics);
    let c = en.overlay(&f);
    assert_eq!(
        c.render("info.save.done", &[("path", "a.txt")], &|_| None),
        "a.txt enregistré"
    );
    assert_eq!(c.get("status.untitled"), en.get("status.untitled"));
    assert_eq!(c.len(), en.len());
}

#[test]
fn 問題の文言のキーの種別() {
    let all = [
        LocaleProblem::Syntax {
            message: String::new(),
        },
        LocaleProblem::NotUtf8,
        LocaleProblem::TooLarge,
        LocaleProblem::LanguageMismatch {
            lang: String::new(),
        },
        LocaleProblem::UnknownKey,
        LocaleProblem::Reserved,
        LocaleProblem::DuplicateKey,
        LocaleProblem::TooLong { count: 0 },
        LocaleProblem::Empty,
        LocaleProblem::BadChar {
            class: CharClass::Control,
            code: 0,
        },
        LocaleProblem::Placeholder {
            expected: vec![],
            issue: PlaceholderIssue::Missing,
        },
        LocaleProblem::Accelerator,
        LocaleProblem::HintWidth { width: 0 },
        LocaleProblem::More { count: 0 },
    ];
    let names: Vec<&str> = all.iter().map(LocaleProblem::message_name).collect();
    assert_eq!(
        names,
        [
            "locale_syntax",
            "locale_not_utf8",
            "locale_too_large",
            "locale_language_mismatch",
            "locale_unknown_key",
            "locale_reserved",
            "locale_duplicate_key",
            "locale_too_long",
            "locale_empty",
            "locale_control_char",
            "locale_placeholder",
            "locale_accelerator",
            "locale_hint_width",
            "locale_more",
        ]
    );
}

// verifies: AC-NFR-603-9
#[test]
fn 同梱の言語ファイルは検査_2_から_9_に通る() {
    let (ja, en) = (Catalog::builtin_ja(), Catalog::builtin_en());
    for (lang, cat, other) in [("ja", ja, en), ("en", en, ja)] {
        for k in cat.keys() {
            let v = cat.get(k).unwrap_or_default();
            let theirs = other.get(k).unwrap_or_default();
            assert_eq!(check_text(k, v, v), TextCheck::Pass, "{lang}: {k} = {v:?}");
            assert_eq!(
                check_text(k, v, theirs),
                TextCheck::Pass,
                "{lang}: {k} = {v:?}"
            );
        }
    }
}

#[test]
fn 同梱の文言そのものを書いたファイルは診断なしで同じ表になる() {
    // 同梱と同じ書式の全訳のファイル (同梱の ja を写したもの)
    let src = include_str!("../../../../locales/ja.toml");
    let b = Catalog::builtin_ja();
    let f = load_locale_file("ja", src.as_bytes(), b, LOCALE_MAX_DIAGNOSTICS);
    assert!(f.diagnostics.is_empty(), "{:?}", f.diagnostics);
    assert_eq!(f.texts.len(), b.len());
    let c = b.overlay(&f);
    assert!(c.keys().eq(b.keys()));
    assert!(c.keys().all(|k| c.get(k) == b.get(k)));
}

#[test]
fn インラインの表と配列を字句で見つける() {
    // (本文, 見つける位置)
    let cases: &[(&str, Option<usize>)] = &[
        ("a = \"x\"\n", None),
        ("[a.b]\nc = \"x\"\n", None),
        ("  [ \"a[b]\" . 'c{d' ]\n", None),
        ("a = \"{ [ \\\" { \"\n", None),
        ("a = 'x\' # { \n", None),
        ("a = \"\"\"\n{ [\n\"\"\"\"\"\n", None),
        ("a = '''\n[x]\n'''''\nb = 1\n", None),
        ("# [ { \nb = 1 # {\n", None),
        ("a = { b = 1 }\n", Some(4)),
        ("a = [1]\n", Some(4)),
        ("[[a]]\n", Some(0)),
        ("[a[b]]\n", Some(2)),
        ("a.b = 1\nc = {}\n", Some(12)),
        // 閉じていない 1 行の文字列は改行で終わる (パーサが構文エラーにする)
        ("a = \"x\n[y]\n", None),
        ("a = \"x\nb = {\n", Some(11)),
    ];
    for &(src, want) in cases {
        assert_eq!(scan_structure(src).map(|(at, _)| at), want, "{src:?}");
    }
    // 見つけたらファイル全体の問題で、位置を示す
    let f = load("[status]\nuntitled = [\"x\"]\n");
    assert_eq!(
        kinds(&f),
        [(String::new(), locale(LocaleProblem::InlineValue), 2)]
    );
    assert_eq!(f.diagnostics[0].severity, Severity::Error);
    assert!(f.texts.is_empty());
}

#[test]
fn meta_language_の値の形() {
    for (v, ok) in [
        ("ja", true),
        ("JA-jp", true),
        ("ja_JP", true),
        ("jpn", false),
        ("j", false),
        ("jaxx", false),
        ("ja-", false),
        ("ja-toolongsub", false),
        ("ja.JP", false),
        ("en", false),
    ] {
        let f = load(&format!("[meta]\nlanguage = \"{v}\"\n"));
        assert_eq!(f.diagnostics.is_empty(), ok, "{v}: {:?}", f.diagnostics);
    }
}

#[test]
fn 文字列のキーの下の複数形は型の違い() {
    // 同梱が文字列で持つキーの下に `.other` を書くと、そのキーを表にしたことになる (07 §17.4)
    let f = load("[status.untitled]\nother = \"x {n}\"\n");
    assert_eq!(
        kinds(&f),
        [(
            "status.untitled".to_owned(),
            DiagKind::TypeMismatch {
                expected: "string",
                found: "table"
            },
            1
        )]
    );
}

#[test]
fn ヒントの幅は_zwnj_を可視化の記号の幅で数える() {
    // クロームは不可視文字を `<ZWNJ>` のように描く (core の cell_text と同じ設定)。その幅で数える
    assert_eq!(hint_width("a\u{200C}b"), 2 + "<ZWNJ>".len());
    assert_eq!(hint_width("→→"), 2, "曖昧幅は 1");
    assert_eq!(hint_width("保存"), 4);
}

// verifies: AC-NFR-603-9
#[test]
fn プレースホルダの問題は何が合わないかを分ける() {
    let issue = |text: &str, builtin: &str| match check_text("a.b", text, builtin) {
        TextCheck::Reject(LocaleProblem::Placeholder { issue, .. }) => Some(issue),
        _ => None,
    };
    use PlaceholderIssue as I;
    assert_eq!(issue("{path", "{path}"), Some(I::Unbalanced));
    assert_eq!(issue("x {other}", "{path} y"), Some(I::Unknown));
    assert_eq!(issue("x", "{path} y"), Some(I::Missing));
    assert_eq!(issue("{path}{path}", "{path} y"), Some(I::TooMany));
    assert_eq!(issue("{tmp} x", "x〔{tmp}〕"), Some(I::Conditional));
    assert_eq!(
        issue("x 〔{path}〕 y", "{path}〔 z〕"),
        Some(I::Conditional)
    );
    assert_eq!(issue("x {path}", "y {path}"), None);
}

// verifies: AC-NFR-603-9
#[test]
fn 訳の無いキーは書いていない同梱のキー() {
    let f = load("[status]\nuntitled = \"x-{n}\"\n[hint.app]\nquit = \"\\u001B\"\n");
    let b = base();
    let missing = f.untranslated(&b);
    assert!(!missing.contains(&"status.untitled"));
    assert!(
        !missing.contains(&"hint.app.quit"),
        "診断のあるキーは数えない"
    );
    assert!(missing.contains(&"msg.save.failed.text"));
    assert_eq!(missing.len(), b.len() - 2);
}
