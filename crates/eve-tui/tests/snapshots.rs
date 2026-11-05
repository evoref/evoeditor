//! 主要画面のスナップショット (01 NFR-603 AC-6、docs/09 §3、docs/review/v0.3-findings.md の STR-T2)。端末に描く画面
//! (本文・ステータスバー・ヒントバー・メッセージ行・オーバーレイ) を行の文字列にした `<name>.txt` と、セルのスタイル
//! (前景・背景・属性) の連続を行ごとに並べた `<name>.styles.txt` の 2 層で固定し、比べる。
//! 比べ方と書き換え方は `eve_testkit::golden` (`EVE_UPDATE_GOLDEN=1` で作る・書き換える)。既にあるファイルの差分の
//! 書き換えは人間が判断する (docs/11 §6.2)。

use std::fmt::Write as _;
use std::path::PathBuf;

use eve_testkit::{Harness, Locale, golden};
use eve_tui::buffer::{chrome_rows_for_surface, row_text};
use eve_tui::chrome::render_screen;
use eve_tui::glyphs::Glyphs;
use eve_view::{Capabilities, CellSize, ColorDepth};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

/// 描き方 (色の段と記号)。
#[derive(Clone, Copy)]
struct Look {
    depth: ColorDepth,
    glyphs: Glyphs,
}

const DEFAULT_LOOK: Look = Look {
    depth: ColorDepth::TrueColor,
    glyphs: Glyphs::unicode(),
};

/// (文字の層, スタイルの層)。
fn screen(h: &mut Harness, look: Look) -> (String, String) {
    let f = h.frame();
    let base = f.size.rows + eve_tui::buffer::bar_rows(&f);
    let window = CellSize {
        cols: f.size.cols,
        rows: base + chrome_rows_for_surface(base),
    };
    let r = render_screen(&f, window, look.depth, &look.glyphs);
    let mut text: Vec<String> = (0..r.buffer.area.height)
        .map(|y| row_text(&r.buffer, y).trim_end().to_owned())
        .collect();
    text.push(format!("cursor: {:?}", r.cursor));

    // スタイルの層: 行ごとに、同じスタイルが続く列の範囲を `開始-終了 fg bg 属性` で並べる (既定のスタイルの範囲は書かない)
    let mut styles = String::new();
    for y in 0..r.buffer.area.height {
        let mut runs: Vec<(u16, u16, String)> = Vec::new();
        for x in 0..r.buffer.area.width {
            let c = &r.buffer[(x, y)];
            let s = format!("{:?} {:?} {:?}", c.fg, c.bg, c.modifier);
            match runs.last_mut() {
                Some(last) if last.2 == s && last.1 + 1 == x => last.1 = x,
                _ => runs.push((x, x, s)),
            }
        }
        let shown: Vec<String> = runs
            .into_iter()
            .filter(|(_, _, s)| s != "Reset Reset NONE")
            .map(|(a, b, s)| format!("{a}-{b} {s}"))
            .collect();
        if !shown.is_empty() {
            let _ = writeln!(styles, "{y}: {}", shown.join(" | "));
        }
    }
    (text.join("\n") + "\n", styles)
}

fn check_look(name: &str, h: &mut Harness, look: Look) {
    let (text, styles) = screen(h, look);
    golden::check(&dir().join(format!("{name}.txt")), &text);
    golden::check(&dir().join(format!("{name}.styles.txt")), &styles);
}

fn check(name: &str, h: &mut Harness) {
    check_look(name, h, DEFAULT_LOOK);
}

const MEMO: &str = "こんにちは、世界\nhello fox\n\tタブと　全角スペース\n";

fn with_file() -> Harness {
    opened(Harness::new().with_size(80, 14))
}

fn opened(h: Harness) -> Harness {
    let mut h = h.with_file("/memo.txt", MEMO.as_bytes());
    h.open("/memo.txt");
    h.run_jobs();
    h
}

fn with_depth(depth: ColorDepth) -> Harness {
    opened(Harness::new().with_size(80, 14).with_caps(Capabilities {
        color_depth: depth,
        ..Capabilities::default()
    }))
}

// verifies: AC-NFR-603-6
#[test]
fn 起動直後の無題の画面() {
    let mut h = Harness::new().with_size(80, 14);
    check("untitled", &mut h);
}

// verifies: AC-NFR-603-6
#[test]
fn ファイルを開いた画面() {
    let mut h = with_file();
    check("opened", &mut h);
}

// verifies: AC-NFR-603-6
#[test]
fn 検索バーの画面() {
    let mut h = with_file();
    h.keys("ctrl+f");
    h.text("fox");
    h.run_jobs();
    check("search", &mut h);
}

// verifies: AC-NFR-603-6
#[test]
fn コマンドパレットの画面() {
    let mut h = with_file();
    h.keys("ctrl+p");
    check("palette", &mut h);
}

// verifies: AC-NFR-603-6
#[test]
fn 入力プロンプトの画面() {
    let mut h = with_file();
    h.keys("ctrl+g");
    check("prompt", &mut h);
}

// verifies: AC-NFR-603-6
#[test]
fn 未保存で終了する確認ダイアログの画面() {
    let mut h = with_file();
    h.keys("ctrl+end");
    h.text("x");
    h.keys("ctrl+q");
    check("quit-dialog", &mut h);
}

// verifies: AC-NFR-603-6
#[test]
fn 警告メッセージの画面() {
    let mut h = Harness::new().with_size(100, 14);
    h.open("/nodir/a.txt");
    h.run_jobs();
    check("warning", &mut h);
}

// verifies: AC-NFR-603-6
#[test]
fn ヘルプ画面の先頭() {
    let mut h = with_file();
    h.keys("ctrl+k h");
    check("help", &mut h);
}

// verifies: AC-NFR-603-6
#[test]
fn ヘルプ画面のキー一覧() {
    let mut h = with_file();
    // パレットから「キー一覧を開く」を実行する
    h.keys("ctrl+p");
    h.text("help.keys");
    h.keys("enter");
    check("help-keys", &mut h);
}

// ---- 色の段・記号 (04 §6・§9) ----

#[test]
fn 色の段_mono() {
    let mut h = with_depth(ColorDepth::Mono);
    h.keys("shift+right shift+right shift+right");
    let look = Look {
        depth: ColorDepth::Mono,
        ..DEFAULT_LOOK
    };
    check_look("depth-mono", &mut h, look);
}

#[test]
fn 色の段_16_色() {
    let mut h = with_depth(ColorDepth::Ansi16);
    h.keys("shift+right shift+right shift+right");
    let look = Look {
        depth: ColorDepth::Ansi16,
        ..DEFAULT_LOOK
    };
    check_look("depth-ansi16", &mut h, look);
}

#[test]
fn 色の段_256_色() {
    let mut h = with_depth(ColorDepth::Ansi256);
    h.keys("shift+right shift+right shift+right");
    let look = Look {
        depth: ColorDepth::Ansi256,
        ..DEFAULT_LOOK
    };
    check_look("depth-ansi256", &mut h, look);
}

#[test]
fn ascii_の記号に縮退した画面() {
    let mut h = opened(Harness::new().with_size(80, 14).with_caps(Capabilities {
        ascii_only: true,
        ..Capabilities::default()
    }));
    let look = Look {
        glyphs: Glyphs::ascii(),
        ..DEFAULT_LOOK
    };
    check_look("ascii-glyphs", &mut h, look);
}

#[test]
fn 選択した範囲() {
    let mut h = with_file();
    h.keys("down shift+right shift+right shift+right shift+right shift+right");
    check("selection", &mut h);
}

// ---- 狭い幅・長い行 ----

#[test]
fn 狭い幅の画面() {
    let mut h = opened(Harness::new().with_size(40, 12));
    check("narrow-opened", &mut h);
}

#[test]
fn 狭い幅の検索バー() {
    let mut h = opened(Harness::new().with_size(40, 12));
    h.keys("ctrl+f");
    h.text("fox");
    h.run_jobs();
    check("narrow-search", &mut h);
}

#[test]
fn 画面の幅より長い行() {
    let long = format!("{}終わり\n2 行目\n", "0123456789".repeat(10));
    let mut h = Harness::new()
        .with_size(60, 10)
        .with_file("/long.txt", long.as_bytes());
    h.open("/long.txt");
    h.run_jobs();
    h.keys("end");
    check("long-line", &mut h);
}

// ---- 編集・検索・置換・プロンプト ----

#[test]
fn 未保存の変更がある画面() {
    let mut h = with_file();
    h.keys("ctrl+end");
    h.text("追記");
    check("modified", &mut h);
}

#[test]
fn 検索で見つからない() {
    let mut h = with_file();
    h.keys("ctrl+f");
    h.text("zzz");
    h.run_jobs();
    check("search-no-match", &mut h);
}

#[test]
fn 置換欄の画面() {
    let mut h = with_file();
    h.keys("ctrl+r");
    h.text("fox");
    h.keys("tab");
    h.text("dog");
    h.run_jobs();
    check("replace", &mut h);
}

#[test]
fn 行へ移動のプロンプトに入力した画面() {
    let mut h = with_file();
    h.keys("ctrl+g");
    h.text("2");
    check("prompt-input", &mut h);
}

// ---- オーバーレイ ----

#[test]
fn 続けて押すキーの候補() {
    let mut h = with_file();
    h.keys("ctrl+k");
    h.advance_ms(2000);
    check("which-key", &mut h);
}

#[test]
fn パレットの候補が溢れる画面() {
    let mut h = opened(Harness::new().with_size(80, 10));
    h.keys("ctrl+p");
    h.text("e");
    check("palette-overflow", &mut h);
}

#[test]
fn パレットの絞り込み() {
    let mut h = with_file();
    h.keys("ctrl+p");
    h.text("保存");
    check("palette-filtered", &mut h);
}

#[test]
fn 文書の一覧() {
    let mut h = with_file().with_file("/b.txt", b"b\n");
    h.open("/b.txt");
    h.run_jobs();
    h.keys("ctrl+e");
    check("buffer-list", &mut h);
}

#[test]
fn ヒントバーを隠した画面() {
    let mut h = with_file();
    h.cmd("ui.toggle_hint_bar");
    check("hint-bar-hidden", &mut h);
}

// ---- 英語 (04 §15、NFR-603) ----

fn english() -> Harness {
    opened(Harness::new().with_size(80, 14).with_locale(Locale::En))
}

#[test]
fn 英語_起動直後の無題の画面() {
    let mut h = Harness::new().with_size(80, 14).with_locale(Locale::En);
    check("en-untitled", &mut h);
}

#[test]
fn 英語_ファイルを開いた画面() {
    let mut h = english();
    check("en-opened", &mut h);
}

#[test]
fn 英語_未保存で終了する確認ダイアログ() {
    let mut h = english();
    h.keys("ctrl+end");
    h.text("x");
    h.keys("ctrl+q");
    check("en-quit-dialog", &mut h);
}

#[test]
fn 英語_コマンドパレット() {
    let mut h = english();
    h.keys("ctrl+p");
    check("en-palette", &mut h);
}

#[test]
fn 英語_検索バー() {
    let mut h = english();
    h.keys("ctrl+f");
    h.text("fox");
    h.run_jobs();
    check("en-search", &mut h);
}

#[test]
fn 英語_警告メッセージ() {
    let mut h = Harness::new().with_size(100, 14).with_locale(Locale::En);
    h.open("/nodir/a.txt");
    h.run_jobs();
    check("en-warning", &mut h);
}
