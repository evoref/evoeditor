//! クリップボード (docs/03 §15.3、02 §6.3、05 §8、01 FR-205): コピー・切り取り・貼り付けと内部レジスタ。

use eve_testkit::Harness;
use eve_view::{ClipboardText, Effect};

fn set_clipboard(h: &Harness) -> Vec<ClipboardText> {
    h.effects()
        .iter()
        .filter_map(|e| match e {
            Effect::SetClipboard { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

fn request_token(h: &Harness) -> Option<eve_view::ClipboardToken> {
    h.effects().iter().rev().find_map(|e| match e {
        Effect::RequestClipboard { token, .. } => Some(*token),
        _ => None,
    })
}

#[test]
fn コピーは選択を_inline_で渡し_文書を変えない() {
    let mut h = Harness::new();
    h.text("hello");
    h.keys("ctrl+a ctrl+c");
    assert_eq!(
        set_clipboard(&h),
        vec![ClipboardText::Inline("hello".to_owned())]
    );
    assert_eq!(h.doc_text(), "hello");
}

#[test]
fn 選択が空ならコピーも切り取りも何もしない() {
    let mut h = Harness::new();
    h.text("hello");
    h.clear_log();
    h.keys("ctrl+c ctrl+x");
    assert!(
        set_clipboard(&h).is_empty(),
        "通知もクリップボードも変わらない"
    );
    assert_eq!(h.doc_text(), "hello");
}

#[test]
fn 切り取りは渡してから削除し_undo_で戻る() {
    let mut h = Harness::new();
    h.text("hello");
    h.keys("ctrl+a ctrl+x");
    assert_eq!(
        set_clipboard(&h),
        vec![ClipboardText::Inline("hello".to_owned())]
    );
    assert_eq!(h.doc_text(), "");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "hello");
}

#[test]
fn 貼り付けは応答の到着時の選択に入り_改行を正規化する() {
    let mut h = Harness::new();
    h.text("ab");
    h.keys("ctrl+v");
    let token = request_token(&h).expect("RequestClipboard");
    // 待つ間にカーソルを動かす: 到着時の位置に入る
    h.keys("home");
    h.clipboard_content(token, Some("X\r\nY"));
    assert_eq!(h.doc_text(), "X\nYab");
}

#[test]
fn 古い_token_の応答は捨てる() {
    let mut h = Harness::new();
    h.text("ab");
    h.keys("ctrl+v");
    let old = request_token(&h).unwrap();
    h.keys("ctrl+v");
    let new = request_token(&h).unwrap();
    assert_ne!(old, new);
    h.clipboard_content(old, Some("OLD"));
    assert_eq!(h.doc_text(), "ab");
    h.clipboard_content(new, Some("NEW"));
    assert_eq!(h.doc_text(), "abNEW");
}

#[test]
fn 読み取れなければ内部レジスタを貼り付け_案内は最初の1回だけ() {
    let mut h = Harness::new();
    h.text("abc");
    h.keys("ctrl+a ctrl+c end ctrl+v");
    let t = request_token(&h).unwrap();
    h.clipboard_content(t, None);
    assert_eq!(h.doc_text(), "abcabc");
    assert_eq!(
        h.messages()
            .iter()
            .filter(|k| *k == "msg.clipboard.fallback")
            .count(),
        1
    );
    h.keys("ctrl+v");
    let t = request_token(&h).unwrap();
    h.clipboard_content(t, None);
    assert_eq!(h.doc_text(), "abcabcabc");
    assert_eq!(
        h.messages()
            .iter()
            .filter(|k| *k == "msg.clipboard.fallback")
            .count(),
        1,
        "同じ起動の中で最初の 1 回だけ"
    );
}

#[test]
fn 大きい選択はハンドルで渡し_回収できる() {
    let mut h = Harness::new();
    let big = "a".repeat(70 * 1024);
    h.paste(&big);
    h.clear_log();
    h.keys("ctrl+a ctrl+c");
    let id = match set_clipboard(&h).as_slice() {
        [ClipboardText::Handle(id)] => *id,
        other => panic!("{other:?}"),
    };
    let src = h.editor.take_clipboard(id).expect("取り出せる");
    assert_eq!(src.len_bytes(), big.len());
    assert!(src.to_text() == big);
    assert!(h.editor.take_clipboard(id).is_none(), "一度だけ");
}
