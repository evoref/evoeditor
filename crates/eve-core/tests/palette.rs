//! コマンドパレット (docs/04 §8.1、07 §11.4、01 FR-602・FR-511・FR-514)。

use eve_testkit::Harness;
use eve_view::{Frame, ListItem, OverlayModel};

fn list(f: &Frame) -> (&Vec<ListItem>, usize, usize) {
    let OverlayModel::List {
        items,
        first,
        selected,
        ..
    } = &f.overlays[0]
    else {
        panic!("List ではありません: {:?}", f.overlays);
    };
    (items, *first, *selected)
}

fn cells(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.to_string()).collect()
}

fn ids(f: &Frame) -> Vec<String> {
    list(f)
        .0
        .iter()
        .map(|i| cells(i.detail.as_ref().unwrap()))
        .collect()
}

// verifies: AC-FR-514-2
// verifies: AC-FR-603-1
#[test]
fn ctrl_p_と_f1_で開き_esc_で閉じる() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    assert_eq!(h.overlay(), "palette");
    h.keys("esc");
    assert_eq!(h.overlay(), "");
    h.keys("f1");
    assert_eq!(h.overlay(), "palette");
}

// verifies: AC-FR-514-2
// verifies: AC-FR-601-1
// verifies: AC-FR-606-2
// verifies: AC-FR-606-3
#[test]
fn 空の入力は_hidden_を除く全コマンドを_id_順に出す() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    let f = h.frame();
    let OverlayModel::List { footer, .. } = &f.overlays[0] else {
        panic!()
    };
    let footer: String = footer.spans.iter().map(|s| s.text.to_string()).collect();
    assert!(footer.starts_with(&format!("{0}/{0}", footer.split('/').next().unwrap())));
    let all = ids(&f);
    assert!(!all.is_empty());
    let mut sorted = all.clone();
    sorted.sort();
    assert_eq!(all, sorted);
    for hidden in ["ui.confirm", "ui.list_next", "ui.dialog_choose"] {
        assert!(!all.iter().any(|i| i == hidden), "{hidden} は出さない");
    }
}

// verifies: AC-FR-602-1
#[test]
fn 入力で絞り込み_前方一致を部分一致より先に出す() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.text("save");
    let all = ids(&h.frame());
    assert!(all.contains(&"file.save".to_owned()), "{all:?}");
    assert!(all.contains(&"file.save_as".to_owned()), "{all:?}");
    // 日本語の title でも探せる
    h.keys("ctrl+a");
}

#[test]
fn 日本語の_title_でも探せて_一致箇所を返す() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.text("名前を付けて");
    let f = h.frame();
    let (items, _, _) = list(&f);
    assert!(ids(&f).contains(&"file.save_as".to_owned()));
    assert!(items.iter().any(|i| !i.match_ranges.is_empty()));
}

#[test]
fn 一致が無ければ_0_件() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.text("zzzzzz");
    let f = h.frame();
    assert!(list(&f).0.is_empty());
    h.keys("enter");
    assert_eq!(h.overlay(), "palette", "候補が無ければ閉じない");
}

#[test]
fn 上下で選択が動き_端で折り返す() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    assert_eq!(list(&h.frame()).2, 0);
    h.keys("down");
    assert_eq!(list(&h.frame()).2, 1);
    h.keys("up up");
    let f = h.frame();
    let total = ids(&f).len().min(12);
    assert!(
        list(&f).2 > 1,
        "先頭から上へは末尾へ折り返す (表示窓 {total})"
    );
}

#[test]
fn enter_で選んだコマンドを実行する() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.text("file.save_as");
    h.keys("enter");
    assert_eq!(h.overlay(), "prompt", "名前を付けて保存の入力が開く");
}

#[test]
fn 文書を編集するコマンドを実行できる() {
    let mut h = Harness::new();
    h.text("abc");
    h.keys("ctrl+p");
    h.text("select.all");
    h.keys("enter");
    h.keys("ctrl+x");
    assert_eq!(h.doc_text(), "");
}

// verifies: AC-FR-602-2
// verifies: AC-FR-601-1
#[test]
fn 実行できないコマンドは理由つきで出し_enter_では実行しない() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.text("file.");
    let f = h.frame();
    let (items, _, _) = list(&f);
    let d = items
        .iter()
        .find(|i| i.disabled_reason.is_some())
        .expect("無題の文書では実行できないコマンドがある");
    let reason = cells(d.disabled_reason.as_ref().unwrap());
    assert!(reason.starts_with("現在は実行できません: "), "{reason}");
    // その 1 件だけに絞って Enter: 閉じない
    let id = cells(d.detail.as_ref().unwrap());
    h.keys("ctrl+a backspace");
    h.text(&id);
    h.keys("enter");
    assert_eq!(h.overlay(), "palette");
}

// verifies: AC-FR-514-2
#[test]
fn 割当キーを逆引きして添える() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.text("file.save_as");
    let f = h.frame();
    let (items, _, _) = list(&f);
    let item = items
        .iter()
        .find(|i| cells(i.detail.as_ref().unwrap()) == "file.save_as")
        .unwrap();
    assert!(!item.keys.is_empty());
}

#[test]
fn プロンプトが出ていてもパレットを開ける() {
    let mut h = Harness::new();
    h.keys("ctrl+o");
    assert_eq!(h.overlay(), "prompt");
    h.keys("ctrl+p");
    assert_eq!(h.overlay(), "palette");
}

#[test]
fn 行は_カテゴリ_タイトル_で_照合箇所はタイトルに合う() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.text("名前を付けて");
    let f = h.frame();
    let (items, _, _) = list(&f);
    let item = items
        .iter()
        .find(|i| cells(i.detail.as_ref().unwrap()) == "file.save_as")
        .unwrap();
    let label = cells(&item.label);
    assert!(label.starts_with("ファイル: "), "{label}");
    // 照合箇所 (書記素番号) が、ラベルの「名前を付けて」の位置に当たる
    let r = item.match_ranges[0].clone();
    let g: Vec<String> = item
        .label
        .spans
        .iter()
        .map(|s| s.text.to_string())
        .collect();
    assert_eq!(g[r].concat(), "名前を付けて");
}

#[test]
fn id_は単語に分けた形でも探せる() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.text("save as");
    assert!(ids(&h.frame()).contains(&"file.save_as".to_owned()));
}

#[test]
fn ページ送りは窓の行数ぶん動き_端で止まる() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.keys("pagedown");
    assert_eq!(list(&h.frame()).2, 12);
    h.keys("pagedown pagedown pagedown pagedown pagedown pagedown");
    let f = h.frame();
    let OverlayModel::List { footer, .. } = &f.overlays[0] else {
        panic!()
    };
    let total: usize = cells(footer).split('/').nth(1).unwrap().parse().unwrap();
    assert_eq!(list(&f).2, total - 1, "末尾で止まる");
    h.keys("pageup");
    assert_eq!(list(&h.frame()).2, total - 13);
}

fn title(f: &Frame) -> String {
    let OverlayModel::List { title, .. } = &f.overlays[0] else {
        panic!()
    };
    cells(title)
}

fn labels(f: &Frame) -> Vec<String> {
    list(f).0.iter().map(|i| cells(&i.label)).collect()
}

// verifies: AC-FR-601-1
#[test]
fn 必須の列挙引数があるコマンドは_同じオーバーレイで値を尋ねる() {
    let mut h = Harness::new();
    h.text("a");
    h.keys("enter");
    h.text("b");
    h.keys("ctrl+p");
    h.text("file.set_line_ending");
    h.keys("enter");
    assert_eq!(h.overlay(), "palette", "閉じずに値の一覧になる");
    let f = h.frame();
    assert_eq!(title(&f), "改行コードを変換: 改行コード");
    assert_eq!(labels(&f), ["LF", "CRLF", "CR"]);
    h.text("crlf");
    assert_eq!(labels(&h.frame()), ["CRLF"]);
    h.keys("enter");
    assert_eq!(h.overlay(), "");
    let eol = h
        .editor
        .state()
        .document(h.doc_id())
        .unwrap()
        .format
        .default_eol;
    assert_eq!(eol, eve_text::LineEnding::Crlf);
}

#[test]
fn 引数の一覧で_esc_は_1_段戻り_もう一度で閉じる() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.text("file.set_line_ending");
    h.keys("enter");
    h.keys("esc");
    assert_eq!(h.overlay(), "palette");
    assert_eq!(title(&h.frame()), "コマンド");
    h.keys("esc");
    assert_eq!(h.overlay(), "");
}

#[test]
fn 文字コードを一覧から選べる() {
    let mut h = Harness::new();
    h.text("日本語");
    h.keys("ctrl+p");
    h.text("file.set_encoding");
    h.keys("enter");
    let f = h.frame();
    assert_eq!(
        labels(&f),
        [
            "UTF-8",
            "UTF-16LE",
            "UTF-16BE",
            "Shift_JIS",
            "EUC-JP",
            "ISO-2022-JP"
        ]
    );
    h.text("shift");
    h.keys("enter");
    // パスのない文書では、新しい書式で保存する名前を尋ねる (03 §8.8)
    assert_eq!(h.overlay(), "prompt");
}

#[test]
fn 引数の無いコマンドはこれまでどおり実行する() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.text("file.save_as");
    h.keys("enter");
    assert_eq!(h.overlay(), "prompt");
}

#[test]
fn 引数の一覧でもページ送りが動く() {
    let mut h = Harness::new();
    h.keys("ctrl+p");
    h.text("file.set_encoding");
    h.keys("enter pagedown");
    assert_eq!(list(&h.frame()).2, 5, "6 項目の末尾で止まる");
    h.keys("pageup");
    assert_eq!(list(&h.frame()).2, 0);
}

#[test]
fn ui_redraw_は_ctrl_k_r_とパレットから全画面の再描画を依頼する() {
    let mut h = Harness::new();
    h.clear_log();
    h.keys("ctrl+k r");
    assert!(
        h.effects().contains(&eve_view::Effect::Redraw),
        "{:?}",
        h.effects()
    );
    h.clear_log();
    h.cmd("ui.redraw");
    assert!(h.effects().contains(&eve_view::Effect::Redraw));
}
