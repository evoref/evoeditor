//! 行コメントの切替 `edit.toggle_comment` (docs/03 §15.6 の CMT-1〜CMT-7、§4.5.2 の「全カーソル」、01 FR-209)。
//!
//! 表の試験で CMT-1〜CMT-7 を、proptest で 07 §12.6.1 の (a)〜(d) を確かめる:
//!
//! - (a) 記号の無い行を含む対象を切り替えて (付けて) からもう一度切り替えると本文が元に戻る。外す向きから始めたときは、
//!   外した後に付けると各行が「最も浅い字下げの位置 + 記号 + 空白」の形になる。
//! - (b) 1 回目で記号の無い行があったときは、2 回目の前に空白でない対象の行がすべて記号で始まる。
//! - (c) 空行と空白だけの行 (と対象でない行) は変わらない。
//! - (d) 1 回の Undo で本文と選択が戻る。

use eve_config::Conditions;
use eve_core::{CommandResult, InitialFile, StartupConfig};
use eve_testkit::Harness;
use eve_text::{ByteOff, Range, Selection};
use proptest::prelude::*;

/// 利用者の言語定義 (組込みに `--` の記号の言語が無いので足す)。
const CONFIG: &str =
    "[[language]]\nname = \"lua\"\nfile_types = [\"lua\"]\ncomment = { line = \"--\" }\n";

/// `path` に `text` を置いて開いた Harness (タブの幅は `tab_width`)。言語は拡張子で決まる。
fn open_with(path: &str, text: &str, tab_width: u8, read_only: bool) -> Harness {
    let mut h = Harness::builder()
        .defer_start()
        .file(path, text.as_bytes())
        .build();
    let resolved = eve_config::resolve(
        &[eve_config::Source {
            name: "config.toml".to_owned(),
            text: Ok(Some(CONFIG.to_owned())),
        }],
        &[format!("editor.tab_width={tab_width}")],
        Conditions::default(),
    );
    h.editor.configure(StartupConfig {
        config: Some(resolved.config),
        config_origins: Some(resolved.origins),
        config_languages: Some(resolved.languages),
        ..Default::default()
    });
    h.editor
        .open_initial_files(&[InitialFile::path(path)], read_only, None);
    h.started();
    h.run_jobs();
    assert_eq!(h.doc_text(), text, "本文がそのまま開ける");
    h
}

fn open(path: &str, text: &str) -> Harness {
    open_with(path, text, 4, false)
}

fn view(h: &Harness) -> eve_view::ViewId {
    h.editor
        .state()
        .window_view(h.window())
        .expect("View がある")
}

fn selection(h: &Harness) -> Selection {
    h.editor
        .state()
        .view(view(h))
        .expect("View がある")
        .selection
        .clone()
}

/// Range (`(anchor, head)` のバイト位置) を置く。最初の Range が主。
fn select(h: &mut Harness, ranges: &[(usize, usize)]) {
    let rs = ranges
        .iter()
        .map(|&(a, b)| Range::new(ByteOff(a), ByteOff(b)))
        .collect();
    let v = view(h);
    h.editor
        .state_mut()
        .set_selection(v, Selection::new(rs, 0).expect("Selection"));
}

fn ranges(h: &Harness) -> Vec<(usize, usize)> {
    selection(h)
        .iter()
        .map(|r| (r.anchor.0, r.head.0))
        .collect()
}

fn toggle(h: &mut Harness) -> CommandResult {
    h.cmd("edit.toggle_comment")
}

/// 文書の全体を選んで切り替えた結果。
fn toggled_all(path: &str, text: &str) -> String {
    let mut h = open(path, text);
    h.keys("ctrl+a");
    assert_eq!(toggle(&mut h), CommandResult::Done);
    h.doc_text()
}

// ---------- AC-1: 付ける・外す ----------

// verifies: AC-FR-209-1
#[test]
fn 対象の行の最も浅い字下げの位置に記号と空白を付け_もう一度で外す() {
    let text = "    if x:\n        y = 1\n    z\n";
    let mut h = open("/w/a.py", text);
    h.keys("ctrl+a");
    toggle(&mut h);
    assert_eq!(h.doc_text(), "    # if x:\n    #     y = 1\n    # z\n");
    toggle(&mut h);
    assert_eq!(h.doc_text(), text, "付けた分だけが外れて元に戻る");
}

// verifies: AC-FR-209-1
#[test]
fn 字下げの浅い行が列_0_なら全部の行の列_0_に付ける() {
    assert_eq!(toggled_all("/w/a.rs", "a\n  b\n"), "// a\n//   b\n");
}

// verifies: AC-FR-209-1
#[test]
fn 選択が無ければカーソルの行だけ() {
    let mut h = open("/w/a.rs", "a\nb\nc\n");
    h.keys("down");
    toggle(&mut h);
    assert_eq!(h.doc_text(), "a\n// b\nc\n");
}

// verifies: AC-FR-209-1
#[test]
fn 一部の行だけがコメントなら全部に付ける() {
    let text = "# a\nb\n";
    let mut h = open("/w/a.py", text);
    h.keys("ctrl+a");
    toggle(&mut h);
    assert_eq!(h.doc_text(), "# # a\n# b\n");
    toggle(&mut h);
    assert_eq!(h.doc_text(), text);
}

// verifies: AC-FR-209-1
#[test]
fn 空行と空白だけの行は飛ばし_字下げの幅の判定にも使わない() {
    assert_eq!(
        toggled_all("/w/a.py", "  a\n\n \n\t\n  b\n"),
        "  # a\n\n \n\t\n  # b\n"
    );
}

// verifies: AC-FR-209-1
#[test]
fn 一回の_undo_で本文と選択が戻る() {
    let text = "a\nb\n";
    let mut h = open("/w/a.py", text);
    select(&mut h, &[(1, 3)]);
    let before = selection(&h);
    toggle(&mut h);
    assert_eq!(h.doc_text(), "# a\n# b\n");
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), text);
    assert_eq!(selection(&h), before);
    assert!(!h.is_dirty());
}

// verifies: AC-FR-209-1
#[test]
fn 既定キーは_ctrl_slash_と_ctrl_k_slash() {
    let mut h = open("/w/a.rs", "a\n");
    h.keys("ctrl+/");
    assert_eq!(h.doc_text(), "// a\n");
    h.keys("ctrl+k /");
    assert_eq!(h.doc_text(), "a\n");
}

// ---------- CMT-1: 対象の行 ----------

#[test]
fn cmt1_行頭で終わる選択はその行を含めない() {
    let mut h = open("/w/a.py", "a\nb\nc\n");
    h.keys("shift+down shift+down");
    toggle(&mut h);
    assert_eq!(h.doc_text(), "# a\n# b\nc\n");
}

#[test]
fn cmt1_空のカーソルはその行() {
    let mut h = open("/w/a.py", "a\nb\n");
    select(&mut h, &[(2, 2)]);
    toggle(&mut h);
    assert_eq!(h.doc_text(), "a\n# b\n");
}

// verifies: AC-FR-209-1
#[test]
fn cmt1_複数カーソルは同じ行を_1_回だけ扱い_飛んだ行は変えない() {
    let mut h = open("/w/a.py", "ab\ncd\nef\n");
    // 1 行目に 2 つ、3 行目に 1 つ
    select(&mut h, &[(0, 0), (1, 1), (6, 7)]);
    toggle(&mut h);
    assert_eq!(h.doc_text(), "# ab\ncd\n# ef\n");
}

// verifies: AC-FR-209-1
#[test]
fn cmt2_複数カーソルでは付けるか外すかを対象の全体で_1_回決める() {
    let mut h = open("/w/a.py", "# a\nb\n# c\n");
    select(&mut h, &[(0, 0), (8, 8)]);
    toggle(&mut h);
    assert_eq!(h.doc_text(), "a\nb\nc\n", "全部がコメントなので外す");
    let mut h = open("/w/a.py", "# a\nb\n# c\n");
    select(&mut h, &[(0, 0), (4, 4), (8, 8)]);
    toggle(&mut h);
    assert_eq!(
        h.doc_text(),
        "# # a\n# b\n# # c\n",
        "コメントでない行があるので全部に付ける"
    );
}

#[test]
fn cmt1_対象が空行だけなら文書を変えずに知らせる() {
    let mut h = open("/w/a.py", "a\n\n  \nb\n");
    select(&mut h, &[(2, 2), (3, 5)]);
    h.clear_log();
    assert_eq!(toggle(&mut h), CommandResult::Done);
    assert_eq!(h.doc_text(), "a\n\n  \nb\n");
    assert!(!h.is_dirty());
    assert_eq!(h.messages(), ["info.comment.no_lines"]);
}

// ---------- CMT-3: 付ける位置 ----------

#[test]
fn cmt3_字下げは表示の列で比べ_タブの途中に落ちる行はタブの後ろに付ける() {
    // タブの幅 4: "\tA" は幅 4、"  B" は幅 2 なので m = 2。"\tA" は列 2 がタブの途中なのでタブの後ろ
    assert_eq!(toggled_all("/w/a.py", "\tA\n  B\n"), "\t# A\n  # B\n");
    // 空白とタブの混ざった行: "  \tA" は幅 4、"    B" も幅 4
    assert_eq!(
        toggled_all("/w/a.py", "  \tA\n    B\n"),
        "  \t# A\n    # B\n"
    );
}

#[test]
fn cmt3_タブの幅は文書の値で数える() {
    // タブの幅 8: "\tA" は幅 8、"    B" は幅 4 なので m = 4 で、"\tA" はタブの後ろ
    let mut h = open_with("/w/a.py", "\tA\n    B\n\t  C\n", 8, false);
    h.keys("ctrl+a");
    toggle(&mut h);
    assert_eq!(h.doc_text(), "\t# A\n    # B\n\t#   C\n");
    // タブの幅 2: "\t\tA" は幅 4、"   B" は幅 3 なので m = 3。"\t\tA" の 2 つ目のタブ (列 2〜4) の途中に落ちる
    let mut h = open_with("/w/a.py", "\t\tA\n   B\n", 2, false);
    h.keys("ctrl+a");
    toggle(&mut h);
    assert_eq!(h.doc_text(), "\t\t# A\n   # B\n");
}

#[test]
fn cmt3_字下げの単位には依らない() {
    // Go は字下げの単位がタブ。空白の字下げの行にも空白のまま付ける
    assert_eq!(toggled_all("/w/a.go", "  a\n  b\n"), "  // a\n  // b\n");
}

// ---------- CMT-4: 外し方 ----------

#[test]
fn cmt4_記号の後の空白は_1_つだけ消し_タブと_2_つ目の空白は残す() {
    assert_eq!(
        toggled_all("/w/a.py", "#a\n# b\n#  c\n#\td\n  # e\n"),
        "a\nb\n c\n\td\n  e\n"
    );
}

#[test]
fn cmt4_記号に続く文字が記号でも記号の分だけを消す() {
    assert_eq!(toggled_all("/w/a.rs", "/// a\n//b\n"), "/ a\nb\n");
}

#[test]
fn 記号は言語ごと() {
    assert_eq!(toggled_all("/w/a.lua", "a\n"), "-- a\n");
    assert_eq!(toggled_all("/w/a.ini", "a\n"), "; a\n");
    assert_eq!(toggled_all("/w/a.lua", "-- a\n--b\n"), "a\nb\n");
}

// ---------- CMT-5: Transaction と選択 ----------

#[test]
fn cmt5_改行コードは変えない() {
    let text = "a\r\n  b\r\nc";
    let mut h = open("/w/a.py", text);
    h.keys("ctrl+a");
    toggle(&mut h);
    assert_eq!(h.doc_text(), "# a\r\n#   b\r\n# c");
    toggle(&mut h);
    assert_eq!(h.doc_text(), text);
    // 混在
    let text = "a\nb\r\nc\r";
    assert_eq!(toggled_all("/w/a.py", text), "# a\n# b\r\n# c\r");
}

#[test]
fn cmt5_行全体の選択は行全体のまま() {
    let mut h = open("/w/a.py", "a\nb\nc\n");
    select(&mut h, &[(0, 4)]);
    toggle(&mut h);
    assert_eq!(h.doc_text(), "# a\n# b\nc\n");
    assert_eq!(ranges(&h), [(0, 8)]);
    // 逆向きの選択も向きを保つ
    let mut h = open("/w/a.py", "a\nb\nc\n");
    select(&mut h, &[(4, 0)]);
    toggle(&mut h);
    assert_eq!(ranges(&h), [(8, 0)]);
}

#[test]
fn cmt5_行頭の空のカーソルは行頭のまま_ほかは自分の編集の後ろへ() {
    let mut h = open("/w/a.py", "    ab\n    cd\n");
    // 1 行目の行頭、1 行目の字下げの末尾 (付ける位置)、2 行目の `c` の後ろ
    select(&mut h, &[(0, 0), (4, 4), (12, 12)]);
    toggle(&mut h);
    assert_eq!(h.doc_text(), "    # ab\n    # cd\n");
    assert_eq!(ranges(&h), [(0, 0), (6, 6), (16, 16)]);
    // 付ける位置が行頭でも、行頭のカーソルは行頭のまま
    let mut h = open("/w/a.py", "ab\n");
    select(&mut h, &[(0, 0)]);
    toggle(&mut h);
    assert_eq!(h.doc_text(), "# ab\n");
    assert_eq!(ranges(&h), [(0, 0)]);
}

#[test]
fn cmt5_外すときのカーソルは消した分だけ前へ() {
    let mut h = open("/w/a.py", "# ab\n");
    select(&mut h, &[(3, 3)]);
    toggle(&mut h);
    assert_eq!(h.doc_text(), "ab\n");
    assert_eq!(ranges(&h), [(1, 1)]);
}

// ---------- CMT-6: 断る場合 (AC-2) ----------

// verifies: AC-FR-209-2
#[test]
fn 記号の無い言語と_plain_では文書を変えずに理由を示す() {
    for (path, shown) in [("/w/a.txt", "plain"), ("/w/a.md", "Markdown")] {
        let mut h = open(path, "a\n");
        h.keys("ctrl+a");
        h.clear_log();
        let rev = h.editor.state().document(h.doc_id()).unwrap().revision();
        assert_eq!(toggle(&mut h), CommandResult::Done);
        assert_eq!(h.doc_text(), "a\n");
        let d = h.editor.state().document(h.doc_id()).unwrap();
        assert_eq!(d.revision(), rev, "{path}");
        assert_eq!(d.history().undo_len(), 0, "{path}");
        assert_eq!(h.messages(), ["info.comment.unsupported"], "{path}");
        assert!(
            h.frame_contains(&format!("{shown} には行コメントの記号がありません")),
            "{path}: {}",
            h.frame_text()
        );
    }
}

// verifies: AC-FR-209-2
#[test]
fn 読み取り専用の文書では実行できず_キーでは編集と同じ知らせ() {
    let mut h = open_with("/w/a.py", "a\n", 4, true);
    let d = h.editor.state().document(h.doc_id()).unwrap();
    assert!(!d.is_editable());
    let rev = d.revision();
    assert!(matches!(toggle(&mut h), CommandResult::Rejected(_)));
    assert_eq!(h.doc_text(), "a\n");
    // キーで押したときの知らせは、ほかの編集のコマンド (Tab) と同じ
    h.clear_log();
    h.keys("tab");
    let edit = h.messages();
    h.clear_log();
    h.keys("ctrl+/");
    assert_eq!(h.messages(), edit);
    assert!(!edit.is_empty(), "読み取り専用の知らせが出る");
    assert_eq!(h.doc_text(), "a\n");
    let d = h.editor.state().document(h.doc_id()).unwrap();
    assert_eq!(d.revision(), rev);
}

// ---------- CMT-7: 費用 ----------

#[test]
fn cmt7_長大行でも行頭の空白だけを見る() {
    let long = format!("  {}\n  b\n", "x".repeat(2 * 1024 * 1024));
    let mut h = open("/w/a.py", &long);
    h.keys("ctrl+a");
    toggle(&mut h);
    let got = h.doc_text();
    assert!(got.starts_with("  # xxx"));
    assert!(got.ends_with("\n  # b\n"));
    toggle(&mut h);
    assert_eq!(h.doc_text(), long);
}

// ---------- 性質 (07 §12.6.1 の (a)〜(d)) ----------

const MARKERS: &[(&str, &str)] = &[("py", "#"), ("rs", "//"), ("lua", "--"), ("ini", ";")];

/// 文書の部品 (`#` は記号に置き換える)。CRLF・CR・LF の混在、タブと空白の字下げ、空白だけの行を作る。
const PIECES: &[&str] = &[
    "x", "y", "あ", " ", "  ", "\t", "\n", "\n", "\r\n", "\r", "#", "# ", "#\t",
];

/// 行 (本文, 改行)。改行は LF・CRLF・CR。最後の行の改行は空。
fn split(text: &str) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    let b = text.as_bytes();
    let (mut start, mut i) = (0, 0);
    while i < b.len() {
        let eol = match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => 2,
            b'\r' | b'\n' => 1,
            _ => {
                i += 1;
                continue;
            }
        };
        lines.push((text[start..i].to_owned(), text[i..i + eol].to_owned()));
        i += eol;
        start = i;
    }
    lines.push((text[start..].to_owned(), String::new()));
    lines
}

/// 行の先頭のバイト位置。
fn line_starts(lines: &[(String, String)]) -> Vec<usize> {
    let mut out = Vec::with_capacity(lines.len());
    let mut at = 0;
    for (body, eol) in lines {
        out.push(at);
        at += body.len() + eol.len();
    }
    out
}

/// 参照モデルの対象の行 (CMT-1。空白だけの行を含む行の番号)。
fn model_targets(text: &str, sel: &Selection) -> Vec<usize> {
    let starts = line_starts(&split(text));
    let line_of = |p: usize| starts.iter().rposition(|&s| s <= p).unwrap_or(0);
    let mut out = Vec::new();
    for r in sel.iter() {
        let first = line_of(r.start().0);
        let mut last = line_of(r.end().0);
        if !r.is_empty() && last > first && starts[last] == r.end().0 {
            last -= 1;
        }
        out.extend(first..=last);
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn is_blank(body: &str) -> bool {
    body.bytes().all(|b| b == b' ' || b == b'\t')
}

fn lead(body: &str) -> &str {
    let n = body
        .bytes()
        .take_while(|&b| b == b' ' || b == b'\t')
        .count();
    &body[..n]
}

fn width(ws: &str, tab: usize) -> usize {
    ws.bytes()
        .fold(0, |c, b| if b == b'\t' { c + tab - c % tab } else { c + 1 })
}

/// 各行が「最も浅い字下げの位置 + 記号 + 空白」の形か (付けた後の形)。
fn added_form(after: &str, targets: &[usize], token: &str, tab: usize) -> Result<(), String> {
    let lines = split(after);
    let bodies: Vec<&str> = targets
        .iter()
        .map(|&l| lines[l].0.as_str())
        .filter(|b| !is_blank(b))
        .collect();
    let Some(m) = bodies.iter().map(|b| width(lead(b), tab)).min() else {
        return Ok(());
    };
    for b in bodies {
        let ws = lead(b);
        let rest = &b[ws.len()..];
        if !rest.starts_with(&format!("{token} ")) {
            return Err(format!("記号と空白で始まらない: {b:?}"));
        }
        // 付ける位置は、幅が初めて m 以上になる位置 (その手前の幅は m 未満)
        let before = width(&ws[..ws.len().saturating_sub(1)], tab);
        if width(ws, tab) < m || (!ws.is_empty() && before >= m) {
            return Err(format!("最も浅い字下げの位置でない: {b:?} (m = {m})"));
        }
    }
    Ok(())
}

fn doc_text() -> impl Strategy<Value = Vec<&'static str>> {
    proptest::collection::vec(proptest::sample::select(PIECES.to_vec()), 0..32)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// (a)〜(d)。
    #[test]
    fn 切り替えを_2_回すると付けた分が戻り_空白だけの行は変わらず_undo_は_1_回(
        pieces in doc_text(),
        marker in 0..MARKERS.len(),
        tab in 1u8..=8,
        raw in proptest::collection::vec((any::<u16>(), any::<u16>()), 1..5),
        primary in any::<usize>(),
    ) {
        let (ext, token) = MARKERS[marker];
        let text: String = pieces.iter().map(|p| p.replace('#', token)).collect();
        let tab_width = usize::from(tab);
        let mut h = open_with(&format!("/w/p.{ext}"), &text, tab, false);
        // Range を置く (文字の境界に寄せる。CRLF の間は正規化で寄る)
        let len = text.len() + 1;
        let fix = |p: u16| {
            let mut p = usize::from(p) % len;
            while !text.is_char_boundary(p) {
                p -= 1;
            }
            ByteOff(p)
        };
        let rs: Vec<Range> = raw.iter().map(|&(a, b)| Range::new(fix(a), fix(b))).collect();
        let n = rs.len();
        let v = view(&h);
        h.editor.state_mut().set_selection(v, Selection::new(rs, primary % n).expect("Selection"));
        let before = selection(&h);
        let targets = model_targets(&text, &before);
        let orig = split(&text);
        // CR の行の次の、記号 (と空白) だけで LF で終わる行は、外すと CR と LF が隣り合って CRLF の 1 つの改行になる
        // (行の数が変わる。手で消したときと同じ)。本文と Undo の性質は保つが、行ごとの比べ方ができないので除く
        prop_assume!(!orig.windows(2).any(|w| {
            w[0].1 == "\r" && w[1].1 == "\n" && (w[1].0 == token || w[1].0 == format!("{token} "))
        }));
        let content: Vec<usize> = targets.iter().copied().filter(|&l| !is_blank(&orig[l].0)).collect();
        h.clear_log();
        prop_assert_eq!(toggle(&mut h), CommandResult::Done);
        if content.is_empty() {
            prop_assert_eq!(h.doc_text(), text.clone());
            prop_assert_eq!(h.messages(), vec!["info.comment.no_lines".to_owned()]);
            return Ok(());
        }
        let adding = content.iter().any(|&l| !lead_rest(&orig[l].0).starts_with(token));
        let once = h.doc_text();
        let after = split(&once);
        prop_assert_eq!(after.len(), orig.len(), "行の数は変わらない");
        // (c) 空行・空白だけの行・対象でない行と、改行は変わらない
        for (i, ((ob, oe), (ab, ae))) in orig.iter().zip(&after).enumerate() {
            prop_assert_eq!(oe, ae, "改行 (行 {})", i);
            if is_blank(ob) || !targets.contains(&i) {
                prop_assert_eq!(ob, ab, "行 {} は変わらない", i);
            }
        }
        // (d) 1 回の Undo で本文と選択が戻り、Redo で後に戻る
        let sel_after = selection(&h);
        h.cmd("edit.undo");
        prop_assert_eq!(h.doc_text(), text.clone(), "Undo で本文が戻る");
        prop_assert_eq!(selection(&h), before.clone(), "Undo で選択が戻る");
        h.cmd("edit.redo");
        prop_assert_eq!(h.doc_text(), once.clone());
        prop_assert_eq!(selection(&h), sel_after);
        if adding {
            // (b) 2 回目の前に、空白でない対象の行がすべて記号で始まる (付けた形)
            prop_assert_eq!(added_form(&once, &targets, token, tab_width), Ok(()));
            // (a) もう一度で元に戻る
            toggle(&mut h);
            prop_assert_eq!(h.doc_text(), text, "付けてから外すと元に戻る");
        } else {
            // 外す向き: 外した後に付けると、各行が「最も浅い字下げの位置 + 記号 + 空白」の形になる (外した後もすべての行が
            // 記号で始まっていれば (`###`)、2 回目も外す向きなので調べない)。2 回目の対象は、外した後の選択から決める
            // (選択の端が外した記号の中にあると、消した範囲の始まりへ寄るので、行頭へ寄った終わりの行は対象から外れる)
            let second = model_targets(&once, &selection(&h));
            let adds = second
                .iter()
                .any(|&l| !is_blank(&after[l].0) && !lead_rest(&after[l].0).starts_with(token));
            toggle(&mut h);
            if adds {
                prop_assert_eq!(added_form(&h.doc_text(), &second, token, tab_width), Ok(()));
            }
        }
    }
}

/// 行頭の空白の後ろ。
fn lead_rest(body: &str) -> &str {
    &body[lead(body).len()..]
}
