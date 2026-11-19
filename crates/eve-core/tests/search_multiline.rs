//! 行をまたぐ検索を利用者に開いた後の振る舞い (docs/03 §12.3.1、01 §5.5 の FR-402 AC-5〜AC-9、08 §2.5 の M24 の (4))。
//!
//! 試験の口 (`SearchTuning`) を使わず、既定の設定・既定の上限 (16 MiB) のままエディタを通して確かめる。シナリオ DSL
//! (`tests/scenarios/search/`) で書けない、16 MiB を超える文書・メッセージ行の hint・ヘルプの節・設定の値をここで見る。

use eve_core::SearchQuery;
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_view::{HelpLineKind, OverlayModel};

/// 検索バーの件数の位置の文言 (件数か誤り)。
fn bar_text(h: &mut Harness) -> String {
    let s = h.frame().search.expect("検索バー");
    s.error
        .or(s.count_text)
        .map(|t| cell_text(&t))
        .unwrap_or_default()
}

/// メッセージ行の (text, hint)。
fn message(h: &mut Harness) -> (String, String) {
    let m = h.frame().messages.current.expect("メッセージ");
    (
        cell_text(&m.text),
        m.hint.map(|t| cell_text(&t)).unwrap_or_default(),
    )
}

/// `doc` を開いて、検索バー (置換欄つき) を正規表現で開く。
fn open_bar(doc: &[u8]) -> Harness {
    let mut h = Harness::new().with_size(100, 12).with_file("/w/a.txt", doc);
    h.open("/w/a.txt");
    h.run_jobs();
    h.keys("ctrl+home");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h
}

/// 16 MiB (`SEARCH_CONCAT_MAX_BYTES`) を少し超える文書。
fn big_doc() -> Vec<u8> {
    let unit = b"a\nb\n";
    let n = (16 << 20) / unit.len() + 16;
    unit.repeat(n)
}

// verifies: AC-FR-402-9
#[test]
fn 上限の無いパターンは_16_mib_を超える文書では探さず_理由と回数を限る書き方を示す() {
    let doc = big_doc();
    assert!(doc.len() > eve_core::SEARCH_CONCAT_MAX_BYTES);
    let mut h = open_bar(&doc);
    h.text(r"(?s)a.*b");
    assert_eq!(h.pending_jobs(), 0, "ジョブを投げない (入力を止めない)");
    assert_eq!(
        bar_text(&mut h),
        "✖ 探せません: 何行でもまたげるパターン (16 MiB を超える文書)"
    );
    // 次へ: メッセージ行に text と hint を出して何もしない
    h.keys("enter");
    assert_eq!(
        message(&mut h),
        (
            "✖ 探せません: 何行でもまたげるパターン (16 MiB を超える文書)".to_owned(),
            "回数を限ると探せます (例: (?s).{0,1000})".to_owned()
        )
    );
    assert_eq!(h.selection_range(), (0, 0));
    // すべて置換も何もしない
    h.keys("tab");
    h.text("x");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(
        message(&mut h).0,
        "✖ 探せません: 何行でもまたげるパターン (16 MiB を超える文書)"
    );
    assert_eq!(h.doc_text().len(), doc.len(), "文書は変わらない");
}

// verifies: AC-FR-402-9
#[test]
fn 案内の書き方は重くなく_16_mib_を超える文書でも探せる() {
    // hint の例 `(?s).{0,1000}` は、書き換えた後も重い検索語に当たらず、行の数に上限があり、最大の長さが有限
    // (03 §12.3.1 の「探さない場合」。例を決める前に確かめること)
    let q = SearchQuery::literal(r"(?s).{0,1000}").with_regex(true);
    let plan = q.analyze().unwrap().unwrap();
    assert!(!plan.heavy());
    assert!(plan.max_len().is_some());
    let m = q.matcher().unwrap();
    assert!(!m.too_heavy());
    assert!(!m.too_large_for(1 << 40), "文書の大きさに依らず探せる");
    // 文書でも: 探さない理由を出さず、ジョブを投げる
    let mut h = open_bar(&big_doc());
    h.text(r"(?s)a.{0,1000}b");
    assert!(h.frame().search.unwrap().error.is_none());
    assert!(h.pending_jobs() > 0);
}

// verifies: AC-FR-402-9
#[test]
fn 上限の無いパターンは_16_mib_以下の文書では探す() {
    let mut h = open_bar(b"x a\nb\nc b y");
    h.text(r"(?s)a.*b");
    h.run_jobs();
    assert_eq!(bar_text(&mut h), "1/1");
    h.keys("enter");
    assert_eq!(h.selection_range(), (2, 9), "最後の b まで (貪欲)");
}

// verifies: AC-FR-402-9
#[test]
fn 重すぎる検索語は打ち切って理由を示す() {
    // 上限の無い重い検索語は、全文の連結に入れずに始める前に打ち切る (03 §12.3.1 の「取り消しの遅れ」)
    let mut h = open_bar(b"a\nb");
    h.text(r"(?s)a.*[^\n]{0,2000}b");
    assert_eq!(h.pending_jobs(), 0);
    assert_eq!(
        bar_text(&mut h),
        "✖ 検索語が重すぎるため、検索を打ち切りました"
    );
    h.keys("enter");
    assert_eq!(
        message(&mut h),
        (
            "✖ 検索語が重すぎるため、検索を打ち切りました".to_owned(),
            "繰り返しの回数を小さくするか、.* の範囲を狭めてください".to_owned()
        )
    );
}

// verifies: AC-FR-402-8
#[test]
fn crlf_を_cr_と_lf_に分けて一致しない() {
    for (doc, p, want) in [
        ("a\r\nb", r"\n\n", "0 件"),
        ("a\r\nb", r"\n{2,}", "0 件"),
        ("a\r\nb", r"\n\s*\n", "0 件"),
        ("a\r\nb", r"[\s\n]{2}", "0 件"),
        ("a\r\n\r\nb", r"\n\n", "1/1"),
    ] {
        let mut h = open_bar(doc.as_bytes());
        h.text(p);
        h.run_jobs();
        assert_eq!(bar_text(&mut h), want, "{p} / {doc:?}");
        if want == "1/1" {
            h.keys("enter");
            assert_eq!(h.selection_range(), (1, 5), "{p}");
        }
    }
}

// verifies: AC-FR-402-5
#[test]
fn 前へも行をまたぐ一致へ移り_画面で強調する() {
    let mut h = open_bar(b"a\nb x a\r\nb y a\rb");
    h.text(r"a\nb");
    h.run_jobs();
    assert_eq!(bar_text(&mut h), "1/3");
    // 前へ (先頭から折り返して末尾の一致へ)
    h.keys("up");
    h.run_jobs();
    assert_eq!(h.selection_range(), (13, 16));
    h.keys("up");
    h.run_jobs();
    assert_eq!(h.selection_range(), (6, 10));
    // 画面の強調: 2 行目の先頭の `b` (一致の後ろ半分) も強調する
    let f = h.frame();
    let marked: String = f.panes[0]
        .rows
        .iter()
        .flat_map(|r| r.spans.iter())
        .filter(|s| s.style.mods.contains(eve_view::StyleMods::UNDERLINE))
        .map(|s| s.text.as_str())
        .collect();
    assert!(marked.contains('b'), "{marked:?}");
}

#[test]
fn 設定_search_newline_sugar_は検索バーを開いたときの値で効く() {
    let doc = b"a\r\nb\nc";
    let count = |sugar: Option<bool>| {
        let mut h = Harness::new().with_size(60, 10).with_file("/w/a.txt", doc);
        if let Some(v) = sugar {
            let loaded = eve_config::load_str(&format!("[search]\nnewline_sugar = {v}\n"));
            assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
            assert_eq!(loaded.config.newline_sugar, v);
            h.editor.configure(eve_core::StartupConfig {
                config: Some(loaded.config),
                ..Default::default()
            });
        }
        h.open("/w/a.txt");
        h.run_jobs();
        h.keys("ctrl+home");
        h.keys("ctrl+f");
        h.keys("alt+r");
        h.text(r"\n");
        h.run_jobs();
        bar_text(&mut h)
    };
    assert_eq!(count(None), "1/2", "既定は ON (CRLF と LF)");
    assert_eq!(count(Some(true)), "1/2");
    assert_eq!(count(Some(false)), "1/1", "OFF は LF の改行だけ");
}

#[test]
fn ヘルプに検索の書き方の節がある() {
    let mut h = Harness::new().with_size(100, 20);
    h.keys("ctrl+k h");
    let f = h.frame();
    let Some(OverlayModel::Help { lines, .. }) = f.overlays.into_iter().next() else {
        panic!("ヘルプが出ている")
    };
    let text = |i: usize| cell_text(&lines[i].text);
    let at = |kind: HelpLineKind, t: &str| {
        lines
            .iter()
            .position(|l| l.kind == kind && cell_text(&l.text) == t)
    };
    let search = at(HelpLineKind::Heading, "検索の書き方").expect("節の見出し");
    let terminal = at(HelpLineKind::Heading, "端末の設定").expect("端末の設定");
    let commands = at(HelpLineKind::Heading, "コマンド一覧").expect("コマンド一覧");
    assert!(
        commands < search && search < terminal,
        "コマンド一覧と端末の設定の間"
    );
    let body: Vec<(String, String)> = lines[search + 1..terminal]
        .iter()
        .map(|l| {
            (
                l.key.as_ref().map(cell_text).unwrap_or_default(),
                cell_text(&l.text),
            )
        })
        .collect();
    let find = |key: &str| {
        body.iter()
            .find(|(k, _)| k == key)
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| panic!("{key}: {body:?}"))
    };
    assert_eq!(find(r"\n"), "改行 1 つ (LF・CRLF・CR のどれでも)");
    assert!(find(r"(?:\r\n)+  \n+").contains(r"\r\n+ は CRLF の文書で一致しない"));
    assert!(find(r"\s  [^x]  .").contains("行をまたがない"));
    // 導入の行は正規表現のトグルの表示 (`[.*]`) で案内する。上限と設定の名前も案内する
    let intro = text(search + 1);
    assert!(intro.contains("[.*]"), "{intro}");
    assert!(body.iter().any(|(_, t)| t.contains("16 MiB")));
    assert!(body.iter().any(|(_, t)| t.contains("search.newline_sugar")));
}
