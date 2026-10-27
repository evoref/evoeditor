//! 端末インジェクション対策 (docs/02 §8.2 INV-F1、01 NFR-503): 外部由来の文字列 (ファイル内容・ファイル名・メッセージ) に
//! 制御シーケンスが含まれていても、端末への出力バイト列に生の制御文字が現れない。

use eve_testkit::Harness;
use eve_tui::ansi::Emitter;
use eve_tui::buffer::chrome_rows_for_surface;
use eve_tui::chrome::render_screen;
use eve_tui::glyphs::Glyphs;
use eve_view::{CellSize, ColorDepth};

/// 外部由来の文字列に入れる攻撃用のコーパス。`EVIL` は検出用の目印。
const CORPUS: &[&str] = &[
    "\x1b]0;EVIL\x07",
    "\x1b]8;;http://EVIL\x1b\\",
    "\x1b[2JEVIL",
    "\x1b[31mEVIL\x1b[0m",
    "\x1bPEVIL\x1b\\",
    "\x1b[?1049lEVIL",
    "\u{9b}2JEVIL",
    "\u{9d}0;EVIL\u{9c}",
    "\u{90}EVIL\u{9c}",
    "EVIL\x07\x08\x0b\x0c\x7f\x00",
];

/// 出力のうち、製品自身が出す正規のシーケンスを取り除いたもの。
/// 正規のものは `ESC [` (CSI) と、タイトルの `ESC ] 0 ; … ESC \` だけ。
fn without_legit(out: &str) -> String {
    let mut s = String::new();
    let mut it = out.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            s.push(c);
            continue;
        }
        match it.peek() {
            Some('[') => {
                it.next();
                // CSI: 終端バイト (0x40..=0x7e) まで
                for d in it.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&d) {
                        break;
                    }
                }
            }
            Some(']') => {
                it.next();
                // OSC: ST (ESC \) まで。内容は検査のため残す
                let mut body = String::new();
                while let Some(d) = it.next() {
                    if d == '\x1b' && it.peek() == Some(&'\\') {
                        it.next();
                        break;
                    }
                    body.push(d);
                }
                s.push_str(&format!("<OSC:{body}>"));
            }
            _ => s.push_str("<ESC>"),
        }
    }
    s
}

fn output(h: &mut Harness) -> String {
    let f = h.frame();
    let win = CellSize {
        cols: f.size.cols,
        rows: f.size.rows + chrome_rows_for_surface(f.size.rows),
    };
    let r = render_screen(&f, win, ColorDepth::TrueColor, &Glyphs::unicode());
    Emitter::new().frame(&r, &f, true)
}

fn assert_clean(out: &str, what: &str) {
    let rest = without_legit(out);
    assert!(!rest.contains("<ESC>"), "{what}: 生の ESC が残る: {rest:?}");
    // 正規の OSC はタイトルだけ。内容に制御文字・攻撃の目印は無い
    for osc in rest.split("<OSC:").skip(1) {
        let body = osc.split('>').next().unwrap();
        assert!(body.starts_with("0;"), "{what}: 想定外の OSC {body:?}");
        assert!(
            !body.chars().any(char::is_control),
            "{what}: タイトルに制御文字 {body:?}"
        );
    }
    let outside: String = rest
        .split("<OSC:")
        .enumerate()
        .map(|(i, p)| {
            if i == 0 {
                p
            } else {
                p.split_once('>').map_or("", |x| x.1)
            }
        })
        .collect();
    let bad = |c: char| {
        (c.is_control() && !matches!(c, '\r' | '\n')) || ('\u{80}'..='\u{9f}').contains(&c)
    };
    assert!(
        !outside.chars().any(bad),
        "{what}: 生の制御文字が出力に出る: {:?}",
        outside.chars().filter(|c| bad(*c)).collect::<String>()
    );
}

// verifies: AC-NFR-503-1
#[test]
fn ファイル内容の制御シーケンスは端末への出力に現れない() {
    for (i, evil) in CORPUS.iter().enumerate() {
        let body = format!("abc {evil} def\nline2 {evil}\n");
        let mut h = Harness::new()
            .with_size(80, 6)
            .with_file("/w/a.txt", body.as_bytes());
        h.open("/w/a.txt");
        h.run_jobs();
        if h.overlay() == "dialog" {
            h.keys("r");
            h.run_jobs();
        }
        let out = output(&mut h);
        assert!(
            out.contains("EVIL"),
            "目印の文字は無害化されて表示される: {out:?}"
        );
        assert_clean(&output(&mut h), &format!("内容 #{i}"));
    }
}

// verifies: AC-NFR-503-1
#[test]
fn ファイル名とメッセージの制御シーケンスは端末への出力に現れない() {
    for (i, evil) in CORPUS.iter().enumerate() {
        let path = format!("/w/x{evil}.txt");
        let mut h = Harness::new().with_size(80, 6).with_file(&path, b"hello\n");
        h.open(&path);
        h.run_jobs();
        assert_clean(&output(&mut h), &format!("名前 (開く) #{i}"));
        // 保存のメッセージ (パスを含む) とタイトル
        h.text("z");
        h.cmd("file.save");
        h.run_jobs();
        assert_clean(&output(&mut h), &format!("名前 (保存) #{i}"));
    }
}

// verifies: AC-NFR-503-1
#[test]
fn 貼り付けと入力の制御シーケンスは端末への出力に現れない() {
    for (i, evil) in CORPUS.iter().enumerate() {
        let mut h = Harness::new().with_size(80, 6);
        h.paste(&format!("p {evil} q"));
        assert_clean(&output(&mut h), &format!("貼り付け #{i}"));
    }
}
