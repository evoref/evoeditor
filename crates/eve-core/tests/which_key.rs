//! which-key (docs/04 §7.4): チョードの保留が 500 ms を超えたら、続けて押せるキーと動作の一覧を出す。

use eve_testkit::Harness;
use eve_view::{KeyCode, WhichKeyModel};

fn text_of(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.to_string()).collect()
}

fn which(h: &mut Harness) -> Option<WhichKeyModel> {
    h.frame().which_key
}

#[test]
fn 保留が五百ミリ秒を超えたら続きの一覧を出す() {
    let mut h = Harness::new().with_size(80, 20);
    h.keys("ctrl+k");
    assert!(which(&mut h).is_none(), "すぐには出さない");
    h.advance_ms(499);
    assert!(which(&mut h).is_none());
    h.advance_ms(1);
    let m = which(&mut h).expect("500 ms で出る");
    assert_eq!(m.prefix.len(), 1);
    let find = |c: char| {
        m.items
            .iter()
            .find(|i| i.key.key == KeyCode::Char(c))
            .map(|i| text_of(&i.text))
    };
    assert_eq!(find('s').as_deref(), Some("名前を付けて保存"));
    assert_eq!(find('m').as_deref(), Some("メッセージ履歴"));
    assert!(m.items.iter().all(|i| !i.more));
}

#[test]
fn 次のキーで保留が終われば消え_打つたびに五百ミリ秒を数え直す() {
    let mut h = Harness::new().with_size(80, 20);
    h.keys("ctrl+k");
    h.advance_ms(600);
    assert!(which(&mut h).is_some());
    h.keys("esc");
    assert!(which(&mut h).is_none(), "Esc で保留が消えれば一覧も消える");
    h.keys("ctrl+k");
    h.advance_ms(300);
    assert!(which(&mut h).is_none(), "数え直し");
}

#[test]
fn 出す時刻は_next_deadline_で知らせ_描き直しを要求する() {
    let mut h = Harness::new().with_size(80, 20);
    assert!(h.editor.next_deadline().is_none());
    h.keys("ctrl+k");
    let d = h.editor.next_deadline().expect("which-key の期限");
    h.frame();
    assert!(!h.editor.needs_render());
    h.advance_ms(500);
    assert!(h.editor.needs_render(), "出す時刻に描き直しを求める");
    h.frame();
    assert!(
        h.editor.next_deadline().is_none_or(|n| n > d),
        "出した後は期限を残さない"
    );
}

#[test]
fn チョードの保留がなければ出さない() {
    let mut h = Harness::new().with_size(80, 20);
    h.advance_ms(2000);
    assert!(which(&mut h).is_none());
}
