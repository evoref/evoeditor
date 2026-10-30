//! ソフトラップの Frame (docs/03 §14.3 / §14.3.1、01 FR-506): 表示行への分割、継続行のガター、カーソルと affinity、
//! 選択と検索の強調、スクロール位置の表示行。

use eve_core::{Affinity, ScrollPos, WrapMode, WrapSettings};
use eve_testkit::Harness;
use eve_view::{Frame, RenderRow, RowSource, StyleMods};

fn text(r: &RenderRow) -> String {
    r.spans.iter().map(|s| s.text.as_str()).collect()
}

fn gutter(r: &RenderRow) -> String {
    r.gutter.iter().map(|s| s.text.as_str()).collect()
}

fn wrap_on(h: &mut Harness, mode: WrapMode) {
    let w = h.editor.state().first_window().unwrap();
    let v = h.editor.state().window_view(w).unwrap();
    h.editor.state_mut().set_wrap(
        v,
        WrapSettings {
            mode,
            ..WrapSettings::default()
        },
    );
}

fn rows(f: &Frame) -> Vec<String> {
    f.panes[0]
        .rows
        .iter()
        .map(|r| text(r).trim_end().to_owned())
        .collect()
}

/// 幅 20 (ガター 3 + 本文 17、折り返し幅 16) の文書。
fn doc(body: &str, mode: WrapMode) -> Harness {
    let mut h = Harness::new().with_size(20, 8);
    h.paste(body);
    h.keys("ctrl+home");
    wrap_on(&mut h, mode);
    h
}

#[test]
fn 長い行は表示行に分かれ_ガターの行番号は最初の行だけ() {
    let mut h = doc("aaaa bbbb cccc dddd eeee ffff\nnext", WrapMode::Word);
    let f = h.frame();
    let r = rows(&f);
    assert_eq!(&r[..3], ["aaaa bbbb cccc", "dddd eeee ffff", "next"]);
    let p = &f.panes[0].rows;
    assert_eq!(gutter(&p[0]).trim(), "1");
    assert_eq!(gutter(&p[1]).trim(), "", "継続行に行番号は出さない");
    assert_eq!(gutter(&p[2]).trim(), "2");
    assert_eq!(
        (p[0].source, p[1].source, p[2].source),
        (
            RowSource::Line {
                line: 0,
                wrap_idx: 0
            },
            RowSource::Line {
                line: 0,
                wrap_idx: 1
            },
            RowSource::Line {
                line: 1,
                wrap_idx: 0
            },
        )
    );
}

#[test]
fn 折り返しなしのときは従来どおり水平スクロール() {
    let mut h = doc("aaaa bbbb cccc dddd eeee ffff", WrapMode::Off);
    let f = h.frame();
    assert_eq!(f.panes[0].rows[1].source, RowSource::Virtual);
    assert!(f.panes[0].rows[0].clipped_right, "右に続きがある");
    assert_eq!(
        rows(&f)[0],
        "aaaa bbbb cccc dd",
        "切れた分は水平スクロールで見る"
    );
}

#[test]
fn 継続行は論理行のインデントでずらす() {
    let mut h = doc("    aaaa bbbb cccc dddd", WrapMode::Word);
    let r = rows(&h.frame());
    assert_eq!(r[0], "    aaaa bbbb");
    assert_eq!(r[1], "    cccc dddd", "{r:?}");
}

#[test]
fn 全角の文章は禁則を守って折る() {
    // 幅 16 に全角 8 文字: 8 文字目の次が「、」なら「、」を行頭に出さず、7 文字目から次の行へ送る
    let mut h = doc("あいうえおかきく、けこ", WrapMode::Word);
    let r = rows(&h.frame());
    assert_eq!(r[0], "あいうえおかき");
    assert_eq!(r[1], "く、けこ");
}

#[test]
fn カーソルは折り返し位置で_affinity_により上下どちらかに描かれる() {
    let mut h = doc("aaaa bbbb cccc dddd eeee", WrapMode::Soft);
    // soft: 16 桁で折れる。カーソルを 16 文字目の次 (折り返し位置) に置く
    h.keys("right right right right right right right right right right right right right right right right");
    let f = h.frame();
    let pos = f.panes[0].cursors[0];
    assert_eq!(
        (pos.row, pos.col),
        (1, 3),
        "After: 下の行の先頭 (ガター 3 の次)"
    );
    let w = h.editor.state().first_window().unwrap();
    let v = h.editor.state().window_view(w).unwrap();
    h.editor
        .state_mut()
        .set_cursor_affinity(v, Affinity::Before);
    let f = h.frame();
    let pos = f.panes[0].cursors[0];
    assert_eq!((pos.row, pos.col), (0, 19), "Before: 上の行の末尾");
}

#[test]
fn 選択と検索の強調は表示行をまたいで付く() {
    let mut h = doc("aaaa bbbb cccc dddd eeee ffff", WrapMode::Word);
    h.keys("shift+end");
    let f = h.frame();
    for i in 0..2 {
        let r = &f.panes[0].rows[i];
        assert!(
            r.spans
                .iter()
                .any(|s| s.style.mods.contains(StyleMods::REVERSE)),
            "行 {i} に選択の反転: {r:?}"
        );
    }
}

#[test]
fn スクロール位置の表示行番号から始める() {
    let mut h = doc("aaaa bbbb cccc dddd eeee ffff\nnext", WrapMode::Word);
    let w = h.editor.state().first_window().unwrap();
    let v = h.editor.state().window_view(w).unwrap();
    h.editor.state_mut().set_scroll(
        v,
        ScrollPos {
            wrap_idx: 1,
            ..Default::default()
        },
    );
    let f = h.frame();
    assert_eq!(rows(&f)[0], "dddd eeee ffff");
    assert_eq!(
        f.panes[0].rows[0].source,
        RowSource::Line {
            line: 0,
            wrap_idx: 1
        }
    );
}

#[test]
fn 文書の最後より後ろは空の行で埋める() {
    let mut h = doc("aaaa bbbb cccc dddd eeee ffff", WrapMode::Word);
    let f = h.frame();
    assert_eq!(f.panes[0].rows.len(), 8);
    assert_eq!(f.panes[0].rows[2].source, RowSource::Virtual);
}
