//! 複数カーソルの編集の性質試験 (docs/03 §4.7.1 の L0 / L1 の (a)〜(d)、M32 の段 2)。
//!
//! 任意の文書 (LF・CRLF・CR・全角・結合文字・タブを含む) と任意の複数の Range (重なり・接する・空と空でないの混在・CRLF の
//! 間を含む。View に置くときに正規化される) について、1 回の編集コマンドの結果を参照モデルと突き合わせる:
//!
//! - (a) 本文が参照モデル「(i) 元の文書の座標で Range ごとの局所の編集を求め、(ii) 重なりは先行の Range を優先して後続を
//!   切り詰め、行のコマンドは触れる行をブロックにまとめて同じ行を 1 回にし、(iii) 末尾から順に適用する」と一致する。
//!   局所の編集の位置 (書記素・単語の境界) は eve-text の関数で求める (組み合わせ方を試す)。
//! - (b) 編集の後の Selection が INV-S1〜INV-S3 を満たす (正規化済み・`start` の順・重ならない・上限以下・主が範囲内)。
//! - (c) 1 回の Undo で本文と複数の Range が編集の前に戻り、Redo で後に戻る。続けて打った文字は全 Range で 1 つにまとまる。
//! - (d) 貼り付けの分配 (内部レジスタの片 → 行 → 全文) が 3 段の参照モデルと一致し、同じ選択でのコピー → 貼り付け
//!   (改行を CR にした端末の貼り付けを含む) は各 Range の文字列を (改行の正規化だけをかけて) 戻す。
//!
//! (e) のほかの View からの写像は `fuzz_editor` (全 View の選択の正規化の検査) と L3 の `multi_cursor.rs` で見る。

use eve_testkit::Harness;
use eve_testkit::helpers::opened;
use eve_text::{
    ByteOff, MAX_CURSORS, Range, Selection, Text, next_grapheme_boundary, next_word_stop,
    prev_grapheme_boundary, prev_word_stop,
};
use eve_view::{ClipboardText, Effect};
use proptest::prelude::*;

const PIECES: &[&str] = &[
    "a", "b", " ", "\t", "あ", "e\u{301}", "\n", "\r\n", "\r", "  ",
];

/// 編集コマンド。
#[derive(Clone, Debug)]
enum Cmd {
    Insert(&'static str),
    Newline,
    Backspace,
    Delete,
    WordBack,
    WordFwd,
    Indent,
    Outdent,
    Duplicate,
    DeleteLine,
    MoveUp,
    MoveDown,
    /// 端末からの貼り付け (`InputEvent::Paste`。内部レジスタは空)。
    Paste(String),
    /// 同じ選択でコピーし、貼り付ける (`cr` なら改行を CR にした文字列が届く)。
    CopyPaste {
        cr: bool,
    },
}

fn cmd() -> impl Strategy<Value = Cmd> {
    let paste = proptest::collection::vec(proptest::sample::select(PIECES.to_vec()), 0..8)
        .prop_map(|v| Cmd::Paste(v.concat()));
    prop_oneof![
        3 => proptest::sample::select(vec!["x", "あ", "\n", "  ", "\t"]).prop_map(Cmd::Insert),
        2 => Just(Cmd::Newline),
        2 => Just(Cmd::Backspace),
        2 => Just(Cmd::Delete),
        1 => Just(Cmd::WordBack),
        1 => Just(Cmd::WordFwd),
        2 => Just(Cmd::Indent),
        2 => Just(Cmd::Outdent),
        1 => Just(Cmd::Duplicate),
        1 => Just(Cmd::DeleteLine),
        1 => Just(Cmd::MoveUp),
        1 => Just(Cmd::MoveDown),
        2 => paste,
        2 => any::<bool>().prop_map(|cr| Cmd::CopyPaste { cr }),
    ]
}

fn doc_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(PIECES.to_vec()), 0..24)
        .prop_map(|v| v.concat())
}

fn ranges() -> impl Strategy<Value = (Vec<(u16, u16)>, usize)> {
    (
        proptest::collection::vec((any::<u16>(), any::<u16>()), 1..7),
        any::<usize>(),
    )
}

// ---------- 参照モデル ----------

/// 局所の編集 (元の文書の座標の削除範囲と挿入)。
type Local = (usize, usize, String);

/// (ii) 先行を優先して後続を切り詰め、(iii) 末尾から順に当てる。
fn combine(text: &str, locals: &[Option<Local>]) -> String {
    let mut trimmed: Vec<Local> = Vec::new();
    let mut covered = 0usize;
    for (s, e, ins) in locals.iter().flatten() {
        let (s, e) = ((*s).max(covered), (*e).max(covered));
        if s == e && ins.is_empty() {
            continue;
        }
        covered = e;
        trimmed.push((s, e, ins.clone()));
    }
    let mut out = text.to_owned();
    for (s, e, ins) in trimmed.iter().rev() {
        out.replace_range(*s..*e, ins);
    }
    out
}

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

fn join(lines: &[(String, String)]) -> String {
    lines.iter().map(|(c, e)| format!("{c}{e}")).collect()
}

/// 行の先頭のバイト位置。
fn starts(lines: &[(String, String)]) -> Vec<usize> {
    let mut at = 0;
    lines
        .iter()
        .map(|(c, e)| {
            let s = at;
            at += c.len() + e.len();
            s
        })
        .collect()
}

fn line_of(starts: &[usize], p: usize) -> usize {
    starts.partition_point(|&s| s <= p) - 1
}

/// 触れる行のブロック (03 §15.4: 行頭で終わる選択は最後の行を含めない。連続する行はまとめる)。
fn blocks(text: &str, sel: &[Range]) -> Vec<(usize, usize)> {
    let lines = split(text);
    let st = starts(&lines);
    let mut v: Vec<(usize, usize)> = sel
        .iter()
        .map(|r| {
            let first = line_of(&st, r.start().0);
            let mut last = line_of(&st, r.end().0);
            if !r.is_empty() && last > first && st[last] == r.end().0 {
                last -= 1;
            }
            (first, last)
        })
        .collect();
    v.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (a, b) in v {
        match out.last_mut() {
            Some(l) if a <= l.1 + 1 => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// 行頭からの表示列 (タブは 4 の倍数まで、全角は 2、結合文字は 0)。
fn col(s: &str) -> usize {
    s.chars().fold(0, |c, ch| match ch {
        '\t' => c + 4 - c % 4,
        'あ' => c + 2,
        '\u{301}' => c,
        _ => c + 1,
    })
}

/// 行頭空白のうち、表示幅 4 以内に収まる部分のバイト数 (03 §15.3 の Shift+Tab)。
fn outdent_bytes(line: &str) -> usize {
    let (mut c, mut bytes) = (0usize, 0usize);
    for ch in line.chars().take_while(|&ch| ch == ' ' || ch == '\t') {
        let next = if ch == '\t' { c + 4 - c % 4 } else { c + 1 };
        if next > 4 {
            break;
        }
        c = next;
        bytes += 1;
    }
    bytes
}

fn line_bounds(text: &str, p: usize) -> (usize, usize) {
    let lines = split(text);
    let st = starts(&lines);
    let l = line_of(&st, p);
    (st[l], st[l] + lines[l].0.len())
}

fn normalize_eol(s: &str, eol: &str) -> String {
    let lines = split(s);
    let n = lines.len();
    lines
        .into_iter()
        .enumerate()
        .map(|(i, (c, _))| if i + 1 < n { format!("{c}{eol}") } else { c })
        .collect()
}

/// 貼り付けの 3 段 (03 §4.5.3 の MCUR-10)。`register` は内部レジスタの片 (改行の正規化の前)。
fn paste_dist(arrived: &str, register: Option<&[String]>, n: usize, eol: &str) -> Vec<String> {
    if n == 1 {
        return vec![normalize_eol(arrived, eol)];
    }
    if let Some(pieces) = register
        && pieces.len() == n
    {
        return pieces.iter().map(|p| normalize_eol(p, eol)).collect();
    }
    let lines: Vec<String> = {
        let mut v: Vec<String> = split(arrived).into_iter().map(|(c, _)| c).collect();
        if v.len() > 1 && v.last().is_some_and(String::is_empty) {
            v.pop();
        }
        v
    };
    if lines.len() == n {
        lines
    } else {
        vec![normalize_eol(arrived, eol); n]
    }
}

/// 参照モデルの本文。`register` は `CopyPaste` のときの片。
fn model(text: &str, sel: &[Range], cmd: &Cmd, eol: &str, arrived: Option<&str>) -> String {
    let t = Text::from(text);
    let any_selection = sel.iter().any(|r| !r.is_empty());
    let span = |r: &Range| (r.start().0, r.end().0);
    let per_range = |f: &dyn Fn(&Range) -> Option<Local>| -> String {
        let locals: Vec<Option<Local>> = sel.iter().map(f).collect();
        combine(text, &locals)
    };
    let delete = |f: &dyn Fn(usize) -> (usize, usize)| {
        per_range(&|r| {
            let (s, e) = if r.is_empty() { f(r.head.0) } else { span(r) };
            (s != e).then(|| (s, e, String::new()))
        })
    };
    match cmd {
        Cmd::Insert(s) => per_range(&|r| Some((r.start().0, r.end().0, (*s).to_owned()))),
        Cmd::Newline => per_range(&|r| {
            let p = r.start().0;
            let (ls, le) = line_bounds(text, p);
            let ie = ls
                + text[ls..le]
                    .bytes()
                    .take_while(|b| *b == b' ' || *b == b'\t')
                    .count();
            Some((p, r.end().0, format!("{eol}{}", &text[ls..ie.min(p)])))
        }),
        Cmd::Backspace => delete(&|p| (prev_grapheme_boundary(&t, ByteOff(p)).0, p)),
        Cmd::Delete => delete(&|p| (p, next_grapheme_boundary(&t, ByteOff(p)).0)),
        Cmd::WordBack => delete(&|p| (prev_word_stop(&t, ByteOff(p)).0, p)),
        Cmd::WordFwd => delete(&|p| (p, next_word_stop(&t, ByteOff(p)).0)),
        Cmd::Indent if !any_selection => per_range(&|r| {
            let (ls, _) = line_bounds(text, r.head.0);
            let c = col(&text[ls..r.head.0]);
            Some((r.head.0, r.head.0, " ".repeat(4 - c % 4)))
        }),
        Cmd::Outdent if !any_selection => per_range(&|r| {
            let (ls, le) = line_bounds(text, r.head.0);
            let n = outdent_bytes(&text[ls..le]);
            (n > 0).then(|| (ls, ls + n, String::new()))
        }),
        Cmd::Indent | Cmd::Outdent => {
            let mut lines = split(text);
            for (a, b) in blocks(text, sel) {
                for (c, _) in &mut lines[a..=b] {
                    if matches!(cmd, Cmd::Indent) {
                        if !c.is_empty() {
                            c.insert_str(0, "    ");
                        }
                    } else {
                        let n = outdent_bytes(c);
                        c.replace_range(..n, "");
                    }
                }
            }
            join(&lines)
        }
        Cmd::Duplicate => {
            let mut lines = split(text);
            for (a, b) in blocks(text, sel).into_iter().rev() {
                let mut copy: Vec<(String, String)> = lines[a..=b].to_vec();
                if lines[b].1.is_empty() {
                    lines[b].1 = eol.to_owned();
                    copy[b - a].1 = String::new();
                }
                lines.splice(b + 1..b + 1, copy);
            }
            join(&lines)
        }
        Cmd::DeleteLine => {
            let mut lines = split(text);
            for (a, b) in blocks(text, sel).into_iter().rev() {
                let last = b + 1 == lines.len() && lines[b].1.is_empty();
                lines.drain(a..=b);
                if last && a > 0 {
                    lines[a - 1].1 = String::new();
                }
            }
            join(&lines)
        }
        Cmd::MoveUp | Cmd::MoveDown => {
            let up = matches!(cmd, Cmd::MoveUp);
            let mut lines = split(text);
            let bl = blocks(text, sel);
            let total = lines.len();
            if bl
                .iter()
                .any(|&(a, b)| if up { a == 0 } else { b + 1 >= total })
            {
                return text.to_owned();
            }
            for (a, b) in bl {
                if up {
                    let prev = lines[a - 1].clone();
                    let block_eol = lines[b].1.clone();
                    let mut moved: Vec<(String, String)> = lines[a..=b].to_vec();
                    moved[b - a].1 = prev.1;
                    moved.push((prev.0, block_eol));
                    lines.splice(a - 1..=b, moved);
                } else {
                    let next = lines[b + 1].clone();
                    let mut moved: Vec<(String, String)> = vec![(next.0, lines[b].1.clone())];
                    let mut block: Vec<(String, String)> = lines[a..=b].to_vec();
                    block[b - a].1 = next.1;
                    moved.extend(block);
                    lines.splice(a..=b + 1, moved);
                }
            }
            join(&lines)
        }
        Cmd::Paste(_) | Cmd::CopyPaste { .. } => {
            let Some(arrived) = arrived else {
                return text.to_owned();
            };
            let register: Option<Vec<String>> = matches!(cmd, Cmd::CopyPaste { .. }).then(|| {
                sel.iter()
                    .filter(|r| !r.is_empty())
                    .map(|r| text[r.start().0..r.end().0].to_owned())
                    .collect()
            });
            let dist = paste_dist(arrived, register.as_deref(), sel.len(), eol);
            per_range(&|r| {
                let i = sel.iter().position(|x| x == r).expect("Range");
                Some((r.start().0, r.end().0, dist[i].clone()))
            })
        }
    }
}

// ---------- 実行と検査 ----------

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

fn default_eol(h: &Harness) -> &'static str {
    h.editor
        .state()
        .document(h.doc_id())
        .expect("文書がある")
        .format
        .default_eol
        .as_str()
}

/// INV-S1〜INV-S3 (03 §4.4、§21 の表): 正規化済み、`start` の順で重ならない、数が上限以下、主が範囲内。
fn check_invariants(h: &Harness, what: &str) {
    let sel = selection(h);
    let doc = h.editor.state().document(h.doc_id()).expect("文書がある");
    let normalized = sel.clone().normalized(doc.text());
    assert_eq!(normalized, sel, "{what}: 正規化されていない");
    assert!(
        (1..=MAX_CURSORS).contains(&sel.len()),
        "{what}: Range の数 {}",
        sel.len()
    );
    assert!(sel.primary_index() < sel.len(), "{what}: 主が範囲外");
    for w in sel.ranges().windows(2) {
        assert!(
            w[0].end() <= w[1].start(),
            "{what}: Range が重なる {:?} {:?}",
            w[0],
            w[1]
        );
    }
}

fn run(h: &mut Harness, cmd: &Cmd) -> Option<String> {
    let id = match cmd {
        Cmd::Insert(s) => {
            h.text(s);
            return None;
        }
        Cmd::Paste(s) => {
            h.paste(s);
            return Some(s.clone());
        }
        Cmd::CopyPaste { cr } => {
            h.clear_log();
            h.cmd("clipboard.copy");
            let copied = h.effects().iter().find_map(|e| match e {
                Effect::SetClipboard {
                    content: ClipboardText::Inline(s),
                    ..
                } => Some(s.clone()),
                _ => None,
            })?;
            let arrived = if *cr {
                copied.replace("\r\n", "\r").replace('\n', "\r")
            } else {
                copied
            };
            h.paste(&arrived);
            return Some(arrived);
        }
        Cmd::Newline => "edit.newline",
        Cmd::Backspace => "edit.delete_backward",
        Cmd::Delete => "edit.delete_forward",
        Cmd::WordBack => "edit.delete_word_backward",
        Cmd::WordFwd => "edit.delete_word_forward",
        Cmd::Indent => "edit.indent",
        Cmd::Outdent => "edit.outdent",
        Cmd::Duplicate => "edit.duplicate_line",
        Cmd::DeleteLine => "edit.delete_line",
        Cmd::MoveUp => "edit.move_line_up",
        Cmd::MoveDown => "edit.move_line_down",
    };
    h.cmd(id);
    None
}

/// 文書を開き、Range を置いた Harness と、置いた後の (正規化された) Selection。
fn setup(text: &str, raw: &[(u16, u16)], primary: usize) -> (Harness, Selection) {
    let mut h = opened("/w/p.txt", text.as_bytes());
    assert_eq!(h.doc_text(), text, "本文がそのまま開ける");
    let len = text.len() + 1;
    let fix = |p: u16| {
        let mut p = usize::from(p) % len;
        while !text.is_char_boundary(p) {
            p -= 1;
        }
        ByteOff(p)
    };
    let rs: Vec<Range> = raw
        .iter()
        .map(|&(a, b)| Range::new(fix(a), fix(b)))
        .collect();
    let n = rs.len();
    let sel = Selection::new(rs, primary % n).expect("Selection");
    let v = view(&h);
    h.editor.state_mut().set_selection(v, sel);
    let sel = selection(&h);
    (h, sel)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    /// (a)〜(d): 1 回の編集が参照モデルと一致し、選択が不変条件を満たし、Undo / Redo で本文と複数の Range が戻る。
    #[test]
    fn 複数の_range_への編集は参照モデルと一致し_undo_で戻る(
        text in doc_text(),
        (raw, primary) in ranges(),
        c in cmd(),
    ) {
        let (mut h, before) = setup(&text, &raw, primary);
        let eol = default_eol(&h);
        let rev = h.editor.state().document(h.doc_id()).unwrap().revision();
        let arrived = run(&mut h, &c);
        let want = model(&text, before.ranges(), &c, eol, arrived.as_deref());
        prop_assert_eq!(h.doc_text(), want, "{:?} {:?}", c, before);
        check_invariants(&h, "編集の後");
        let changed = h.editor.state().document(h.doc_id()).unwrap().revision() != rev;
        if changed {
            let after = selection(&h);
            let after_text = h.doc_text();
            h.cmd("edit.undo");
            prop_assert_eq!(h.doc_text(), text.clone(), "Undo で本文が戻る");
            prop_assert_eq!(selection(&h), before.clone(), "Undo で複数の Range が戻る");
            h.cmd("edit.redo");
            prop_assert_eq!(h.doc_text(), after_text, "Redo で本文が戻る");
            prop_assert_eq!(selection(&h), after, "Redo で編集の後の Range に戻る");
        }
    }

    /// (a): 入力と貼り付けの後は、各 Range が自分の入れた文字列の直後の空のカーソルになる (同じ位置は 1 つ)。
    #[test]
    fn 入力と貼り付けのカーソルは入れた文字列の直後(
        text in doc_text(),
        (raw, primary) in ranges(),
        paste in proptest::collection::vec(proptest::sample::select(PIECES.to_vec()), 0..6),
        typed in any::<bool>(),
    ) {
        let (mut h, before) = setup(&text, &raw, primary);
        let eol = default_eol(&h);
        let paste = paste.concat();
        let dist: Vec<String> = if typed {
            h.text("xy");
            vec!["xy".to_owned(); before.len()]
        } else {
            h.paste(&paste);
            paste_dist(&paste, None, before.len(), eol)
        };
        let mut want: Vec<usize> = Vec::new();
        let mut delta = 0isize;
        for (r, ins) in before.iter().zip(&dist) {
            let start = isize::try_from(r.start().0).unwrap() + delta;
            let at = usize::try_from(start).unwrap() + ins.len();
            want.push(at);
            delta += isize::try_from(ins.len()).unwrap() - isize::try_from(r.end().0 - r.start().0).unwrap();
        }
        want.dedup();
        let got: Vec<usize> = selection(&h).iter().map(|r| {
            assert!(r.is_empty(), "空のカーソル");
            r.head.0
        }).collect();
        prop_assert_eq!(got, want);
    }

    /// (c): 続けて打った文字は、全 Range で R7 の条件を満たすとき (Range が離れていて、編集が 1 つのまとまりに
    /// つながらない) 1 つの Undo にまとまる。主の空のカーソルが選択の始まりに残る (N4) と、最初の入力の 2 つの編集が
    /// つながってまとまりの数が Range の数と変わるので、新しいノードになる (MCUR-7)。
    #[test]
    fn 続けて打った文字は全_range_で_1_つの_undo_になる(
        text in doc_text(),
        (raw, primary) in ranges(),
        s in proptest::sample::select(vec!["x", "あ", "  "]),
    ) {
        let (mut h, before) = setup(&text, &raw, primary);
        let apart = before.ranges().windows(2).all(|w| w[0].end() < w[1].start());
        h.text(s);
        h.text(s);
        h.text(s);
        h.cmd("edit.undo");
        if apart {
            prop_assert_eq!(h.doc_text(), text);
            prop_assert_eq!(selection(&h), before);
        } else {
            prop_assert_ne!(h.doc_text(), text, "つながった編集の後の入力は別のノード");
        }
    }
}

#[test]
fn 参照モデルの行の扱い() {
    assert_eq!(
        split("a\r\nb\rc\n"),
        vec![
            ("a".to_owned(), "\r\n".to_owned()),
            ("b".to_owned(), "\r".to_owned()),
            ("c".to_owned(), "\n".to_owned()),
            (String::new(), String::new()),
        ]
    );
    assert_eq!(paste_dist("a\nb\n", None, 2, "\n"), vec!["a", "b"]);
    assert_eq!(paste_dist("a\n\nb", None, 3, "\n"), vec!["a", "", "b"]);
    assert_eq!(
        paste_dist("a\n\nb", None, 2, "\n"),
        vec!["a\n\nb", "a\n\nb"]
    );
    assert_eq!(
        combine(
            "abcdef",
            &[Some((1, 3, "X".into())), Some((2, 5, String::new()))]
        ),
        "aXf",
        "後続は先行の削除の後ろから"
    );
}
