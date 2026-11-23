//! 対応括弧の強調と移動 (FR-308、docs/03 §5.7 の BRK-1〜BRK-6、07 §11.6 の「複数カーソルと対応括弧」、01 §5.5 の FR-308
//! AC-1 / AC-2) のシナリオ (03 §4.7.1 の L3。M32 の段 4)。文書と Range の印は `marked` (`multi_cursor.rs` と共有)。

mod marked;

use eve_testkit::Harness;
use eve_testkit::helpers::opened;
use eve_text::{ByteOff, Range};
use eve_view::{Frame, StyleMods};
use marked::{scene, selection, set_selection, shown};

/// 1 MiB (03 §5.7 の BRK-4)。
const MIB: usize = 1 << 20;

/// メッセージの記録を消してから `keys` を押し、出たメッセージのキー。
fn press(h: &mut Harness, keys: &str) -> Vec<String> {
    h.clear_log();
    h.keys(keys);
    h.messages()
}

/// Frame のペイン `pane` の、太字と下線 (`ui.bracket_match`) の付いたセルの (行, 文字) の並び。
fn marked_cells(f: &Frame, pane: usize) -> Vec<(usize, String)> {
    let want = StyleMods::BOLD | StyleMods::UNDERLINE;
    f.panes[pane]
        .rows
        .iter()
        .enumerate()
        .flat_map(|(r, row)| {
            row.spans
                .iter()
                .filter(move |s| s.style.mods.contains(want))
                .map(move |s| (r, s.text.clone()))
        })
        .collect()
}

/// フォーカスのペインの強調の文字だけ。
fn highlighted(h: &mut Harness) -> Vec<String> {
    let f = h.frame();
    let i = f
        .panes
        .iter()
        .position(|p| p.active)
        .expect("フォーカスのペイン");
    marked_cells(&f, i).into_iter().map(|(_, s)| s).collect()
}

// ---------- 強調 (BRK-2 / BRK-3 / BRK-5) ----------

// verifies: AC-FR-308-1
#[test]
fn カーソルの直後の括弧を優先し_括弧でなければ直前の括弧と対応を強調する() {
    let mut h = scene("f|(a, (b)) x");
    assert_eq!(highlighted(&mut h), ["(", ")"], "直後の ( と外側の )");
    let mut h = scene("f(a, (b)|) x");
    assert_eq!(highlighted(&mut h), ["(", ")"], "直後の ) と内側の (");
    let mut h = scene("f(a, (b))| x");
    assert_eq!(highlighted(&mut h), ["(", ")"], "直後が空白なので直前の )");
    // `)(` の間は直後の `(` を優先する (対応が無いので強調しない)
    let mut h = scene("(a)|(b");
    assert_eq!(highlighted(&mut h), Vec::<String>::new());
    let mut h = scene("a|bc");
    assert_eq!(highlighted(&mut h), Vec::<String>::new(), "括弧が無い");
}

// verifies: AC-FR-308-1
#[test]
fn 十組の括弧のうち同じ組だけを数え_交差するほかの組を無視する() {
    // `「(」)` の `(` の対応は `)`
    let mut h = scene("「|(」)");
    assert_eq!(highlighted(&mut h), ["(", ")"]);
    let mut h = scene("|「(」)");
    assert_eq!(highlighted(&mut h), ["「", "」"]);
    for (o, c) in [
        ("[", "]"),
        ("{", "}"),
        ("（", "）"),
        ("［", "］"),
        ("｛", "｝"),
        ("『", "』"),
        ("【", "】"),
        ("〔", "〕"),
    ] {
        let mut h = scene(&format!("|{o}あ{o}い{c}{c}"));
        assert_eq!(highlighted(&mut h), [o, c], "{o}{c} の入れ子");
    }
    // `<>` と引用符は括弧ではない
    let mut h = scene("|<a>");
    assert_eq!(highlighted(&mut h), Vec::<String>::new());
}

// verifies: AC-FR-308-1
#[test]
fn 結合文字の付いた括弧は_1_つと数え_書記素のセルを強調する() {
    let mut h = scene("|(\u{0301}a)");
    let f = h.frame();
    let cells = marked_cells(&f, 0);
    assert_eq!(cells.len(), 2, "{cells:?}");
    assert!(cells[0].1.starts_with('('), "{cells:?}");
    assert_eq!(cells[1].1, ")");
}

// verifies: AC-FR-308-1
#[test]
fn 主のカーソルだけを強調し_編集と移動で追いかける() {
    // 主は 2 つ目 (`*`)。1 つ目のカーソルの隣の `[` は強調しない
    let mut h = scene("|[x] *|(y)");
    assert_eq!(highlighted(&mut h), ["(", ")"]);
    // 移動で括弧から離れると消え、戻ると出る
    let mut h = scene("|(y) zz");
    h.keys("right right right right");
    assert_eq!(highlighted(&mut h), Vec::<String>::new(), "(y) z|z");
    h.keys("left");
    assert_eq!(highlighted(&mut h), ["(", ")"], "(y)| zz の直前の )");
    // 編集で対応が崩れると消え、戻すと出る
    h.keys("backspace");
    assert_eq!(h.doc_text(), "(y zz");
    assert_eq!(highlighted(&mut h), Vec::<String>::new());
    h.text(")");
    assert_eq!(h.doc_text(), "(y) zz");
    assert_eq!(highlighted(&mut h), ["(", ")"]);
}

// ---------- 移動 (BRK-6) ----------

// verifies: AC-FR-308-1
#[test]
fn ctrl_右角括弧で対応の直前へ移り_もう一度で元へ戻る() {
    let mut h = scene("f|(a, (b)) x");
    assert_eq!(press(&mut h, "ctrl+]"), Vec::<String>::new());
    assert_eq!(shown(&h), "f(a, (b)|) x", "対応の ) の直前");
    h.keys("ctrl+]");
    assert_eq!(shown(&h), "f|(a, (b)) x", "元の ( の直前");
    // 代替キー ctrl+k %
    h.keys("ctrl+k %");
    assert_eq!(shown(&h), "f(a, (b)|) x");
}

// verifies: AC-FR-308-1
#[test]
fn 閉じ括弧の直後から始めると_1_回目の後の_2_回で往復する() {
    let mut h = scene("x(a)| y");
    h.keys("ctrl+]");
    assert_eq!(shown(&h), "x|(a) y");
    h.keys("ctrl+]");
    assert_eq!(shown(&h), "x(a|) y");
    h.keys("ctrl+]");
    assert_eq!(shown(&h), "x|(a) y");
}

// verifies: AC-FR-308-1
#[test]
fn 選択を解き_目標の列を消す() {
    let mut h = scene("«ab»(cd)\nxy");
    h.keys("ctrl+]");
    assert_eq!(
        shown(&h),
        "ab(cd|)\nxy",
        "選択の head の直後の ( から移り、選択を解く"
    );
    let v = marked::view(&h);
    assert_eq!(h.editor.state().view(v).expect("View").goal_dcol, None);
}

// verifies: AC-FR-308-1
#[test]
fn 対応が無ければ文書を変えずに理由を出す() {
    let mut h = scene("ab|c");
    assert_eq!(press(&mut h, "ctrl+]"), ["info.bracket.none"]);
    assert_eq!(shown(&h), "ab|c");
    let mut h = scene("a|(bc");
    assert_eq!(press(&mut h, "ctrl+]"), ["info.bracket.no_match"]);
    assert_eq!(shown(&h), "a|(bc");
    let mut h = scene("|」");
    assert_eq!(press(&mut h, "ctrl+]"), ["info.bracket.no_match"]);
    assert_eq!(shown(&h), "|」", "文書を変えない");
}

// verifies: AC-FR-308-1
#[test]
fn 複数カーソルでは各_range_を動かし_主が動かなければ理由を出す() {
    let mut h = scene("|(a) |[b] |c *|{d}");
    assert_eq!(press(&mut h, "ctrl+]"), Vec::<String>::new());
    assert_eq!(
        shown(&h),
        "(a|) [b|] |c {d|}",
        "対応の無い Range (c) は動かない"
    );
    // 主が動かないときは、主の理由を出す (ほかは動く)
    let mut h = scene("|(a) *|c");
    assert_eq!(press(&mut h, "ctrl+]"), ["info.bracket.none"]);
    assert_eq!(shown(&h), "(a|) |c");
    // 2 つの Range が入れ替わる: ( の直前 (0) → ) の直前 (2)、主の ) の直前 (2) → ( の直前 (0)
    let mut h = scene("|(a*|)");
    h.keys("ctrl+]");
    let sel = selection(&h);
    let heads: Vec<usize> = sel.iter().map(|r| r.head.0).collect();
    assert_eq!(heads, [0, 2], "{}", shown(&h));
    assert!(sel.iter().all(|r| r.is_empty()), "選択を解く");
    assert_eq!(
        sel.primary().head,
        ByteOff(0),
        "主は ) の直前から ( の直前へ移る"
    );
    let mut h = scene("(|a|)");
    set_selection(
        &mut h,
        vec![Range::point(ByteOff(0)), Range::point(ByteOff(3))],
        0,
    );
    h.keys("ctrl+]");
    assert_eq!(shown(&h), "|(a|)");
}

// ---------- 探す範囲の上限 (BRK-4、AC-2) ----------

/// `(` + `a` × `n` + `)` の文書を開き、カーソルを先頭に置く。
fn big(n: usize) -> Harness {
    let body = format!("({})", "a".repeat(n));
    let mut h = opened("/big.txt", body.as_bytes());
    h.clear_log();
    h
}

// verifies: AC-FR-308-2
#[test]
fn 探す範囲は_1_mib_ちょうどまでで_1_バイト超は範囲を超えた() {
    // 開き括弧の終端から 1 MiB ちょうどで終わる `)` は対応
    let mut h = big(MIB - 1);
    assert_eq!(
        highlighted(&mut h),
        ["("],
        "先頭の ( を強調し、画面の外の ) は描かない"
    );
    assert_eq!(press(&mut h, "ctrl+]"), Vec::<String>::new());
    assert_eq!(h.selection_range(), (MIB, MIB));
    // 1 バイト超えると対応なし (範囲を超えた)
    let mut h = big(MIB);
    assert_eq!(highlighted(&mut h), Vec::<String>::new());
    assert_eq!(press(&mut h, "ctrl+]"), ["info.bracket.too_far"]);
    assert_eq!(h.selection_range(), (0, 0));
    let text: String = h
        .frame()
        .messages
        .current
        .expect("メッセージ")
        .text
        .spans
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert!(text.contains("1 MiB"), "{text}");
}

// verifies: AC-FR-308-2
#[test]
fn チャンクの境目にかかる全角の括弧も数える() {
    // 全角の括弧を多数並べ、rope のチャンクの境目に括弧がかかるようにする
    let inner = "【あ】".repeat(3000);
    let body = format!("【{inner}】");
    let mut h = opened("/fw.txt", body.as_bytes());
    h.keys("ctrl+]");
    assert_eq!(h.selection_range().0, body.len() - "】".len());
    h.keys("ctrl+]");
    assert_eq!(h.selection_range(), (0, 0));
}

// verifies: AC-FR-308-1
#[test]
fn フォーカスの無いペインは可視範囲の中だけで探す() {
    // 1 行目の `(` の対応は 200 行目。フォーカスのペインは対応を探して ( を強調し (対応は画面の外)、フォーカスの無い
    // ペインは可視範囲の外を探さないので強調しない
    let mut body = String::from("(\n");
    for _ in 0..198 {
        body.push_str("x\n");
    }
    body.push(')');
    let mut h = opened("/split.txt", body.as_bytes());
    h.cmd("pane.split_right");
    let f = h.frame();
    let (focused, other) = if f.panes[0].active { (0, 1) } else { (1, 0) };
    assert_eq!(marked_cells(&f, focused), [(0, "(".to_owned())]);
    assert_eq!(marked_cells(&f, other), []);
    // 対応が可視範囲の中なら、フォーカスの無いペインも強調する
    let mut h = opened("/split2.txt", b"(\nx\n)");
    h.cmd("pane.split_right");
    let f = h.frame();
    assert_eq!(marked_cells(&f, 0).len(), 2);
    assert_eq!(marked_cells(&f, 1).len(), 2);
}

// verifies: AC-FR-308-1
#[test]
fn フォーカスの無いペインは長い行でも可視範囲の程度だけを探す() {
    // 1 行目の `(` の対応が同じ行の 100 KB 先。フォーカスのペインは探して ( を強調し、フォーカスの無いペイン (行数 × 桁数 × 4
    // バイトまで) は強調しない
    let body = format!("({})", "a".repeat(100_000));
    let mut h = opened("/long.txt", body.as_bytes());
    h.cmd("pane.split_right");
    let f = h.frame();
    let (focused, other) = if f.panes[0].active { (0, 1) } else { (1, 0) };
    assert_eq!(marked_cells(&f, focused), [(0, "(".to_owned())]);
    assert_eq!(marked_cells(&f, other), []);
}

// verifies: AC-FR-308-1
#[test]
fn ほかのペインで編集すると_フォーカスの無いペインの強調が追う() {
    let mut h = opened("/edit.txt", b"(\nx\n)");
    h.cmd("pane.split_right");
    let f = h.frame();
    let other = usize::from(f.panes[0].active);
    assert_eq!(marked_cells(&f, other).len(), 2, "初めは両方を強調");
    // フォーカスのペインで `)` を消すと、フォーカスの無いペインの強調も消え、戻すと出る
    h.keys("ctrl+end backspace");
    assert_eq!(h.doc_text(), "(\nx\n");
    let f = h.frame();
    assert_eq!(marked_cells(&f, other), []);
    h.text(")");
    let f = h.frame();
    assert_eq!(marked_cells(&f, other).len(), 2);
}

// verifies: AC-FR-308-1
#[test]
fn 元に戻すとやり直しの後も強調が古くならない() {
    let mut h = scene("(a|");
    h.text(")");
    assert_eq!(highlighted(&mut h), ["(", ")"]);
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "(a");
    assert_eq!(highlighted(&mut h), Vec::<String>::new());
    h.keys("ctrl+y");
    assert_eq!(h.doc_text(), "(a)");
    assert_eq!(highlighted(&mut h), ["(", ")"]);
}
