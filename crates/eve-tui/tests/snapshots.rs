//! 主要画面のスナップショット (01 NFR-603 AC-6、docs/09 §3、docs/review/v0.3-findings.md の STR-T2)。端末に描く画面
//! (本文・ステータスバー・ヒントバー・メッセージ行・オーバーレイ) を行の文字列にした `<name>.txt` と、セルのスタイル
//! (前景・背景・属性) の連続を行ごとに並べた `<name>.styles.txt` の 2 層で固定し、比べる。
//! 比べ方と書き換え方は `eve_testkit::golden` (`EVE_UPDATE_GOLDEN=1` で作る・書き換える)。既にあるファイルの差分の
//! 書き換えは人間が判断する (docs/11 §6.2)。

use std::fmt::Write as _;
use std::path::PathBuf;

use eve_testkit::{Harness, Locale, golden};
use eve_tui::buffer::row_text;
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
    // 端末の編集面 = Frame の編集面 + 検索バー + タブライン (core が引いた行)
    let base = f.size.rows + eve_tui::buffer::bar_rows(&f) + eve_tui::buffer::tab_rows(&f);
    let window = CellSize {
        cols: f.size.cols,
        // ヒントバーの表示に合わせた端末の大きさ (本物の frontend なら食い違いで Resize を送る)
        rows: base + eve_tui::buffer::chrome_rows(base, f.hint_bar),
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

// ---- 複数カーソル (v0.5 の M32、FR-211。04 §9・03 §4.5.5) ----

/// 3 行の行頭にカーソルを置き (下へ 2 回足す。主は最後に足した 3 行目)、それぞれ 2 文字ずつ選ぶ。
fn multi_cursor_scene(depth: ColorDepth) -> Harness {
    let mut h = with_depth(depth);
    h.keys("ctrl+alt+down ctrl+alt+down shift+right shift+right");
    h
}

// verifies: AC-FR-211-3
#[test]
fn 複数カーソル_主でないカーソルは反転と下線_主は端末のカーソル() {
    let mut h = multi_cursor_scene(ColorDepth::TrueColor);
    check("multi-cursor", &mut h);
    let (text, styles) = screen(&mut h, DEFAULT_LOOK);
    // 主のカーソル (3 行目) だけが端末のカーソル。1・2 行目の選択の直後のセルが反転と下線
    assert!(
        text.ends_with("cursor: Some(CellPos { col: 9, row: 2 })\n"),
        "{text}"
    );
    for row in ["0:", "1:"] {
        let line = styles
            .lines()
            .find(|l| l.starts_with(row))
            .unwrap_or_else(|| panic!("{row} のスタイル: {styles}"));
        assert!(
            line.contains("UNDERLINED | REVERSED"),
            "{row} の主でないカーソル: {line}"
        );
    }
}

// verifies: AC-FR-211-3
#[test]
fn 複数カーソル_無色の段でも主でないカーソルを見分けられる() {
    let mut h = multi_cursor_scene(ColorDepth::Mono);
    let look = Look {
        depth: ColorDepth::Mono,
        ..DEFAULT_LOOK
    };
    check_look("multi-cursor-mono", &mut h, look);
    let (_, styles) = screen(&mut h, look);
    // 選択 (反転) と、その直後の主でないカーソル (反転 + 下線) は別のスタイルの連続になる
    for row in ["0:", "1:"] {
        let line = styles
            .lines()
            .find(|l| l.starts_with(row))
            .unwrap_or_else(|| panic!("{row} のスタイル: {styles}"));
        assert!(line.contains("UNDERLINED | REVERSED"), "{row}: {line}");
        assert!(
            line.contains("Reset Reset REVERSED"),
            "{row} の選択: {line}"
        );
    }
}

// ---- 対応括弧 (v0.5 の M32、FR-308。04 §9・03 §5.7 の BRK-5) ----

/// 括弧を含む文書を開き、カーソルを `(` の直前に置く (`(` と対応の `)` を強調する。内側の `[]` と全角の `「」` は強調しない)。
fn bracket_scene(depth: ColorDepth) -> Harness {
    let h = Harness::new().with_size(80, 14).with_caps(Capabilities {
        color_depth: depth,
        ..Capabilities::default()
    });
    let mut h = h.with_file("/br.txt", "f(a, [b]) 「x」\n".as_bytes());
    h.open("/br.txt");
    h.run_jobs();
    h.keys("right");
    h
}

/// スタイルの層の 0 行目。
fn row0(styles: &str) -> String {
    styles
        .lines()
        .find(|l| l.starts_with("0:"))
        .unwrap_or_else(|| panic!("0 行目のスタイル: {styles}"))
        .to_owned()
}

// verifies: AC-FR-308-1
#[test]
fn 対応括弧は太字と下線() {
    let mut h = bracket_scene(ColorDepth::TrueColor);
    check("bracket-match", &mut h);
    let (_, styles) = screen(&mut h, DEFAULT_LOOK);
    // ガター (行番号) の後ろの `(` (列 4) と `)` (列 11) だけが太字と下線
    let line = row0(&styles);
    assert!(line.contains("4-4 Reset Reset BOLD | UNDERLINED"), "{line}");
    assert!(
        line.contains("11-11 Reset Reset BOLD | UNDERLINED"),
        "{line}"
    );
}

// verifies: AC-FR-308-1
#[test]
fn 対応括弧は無色の段でも太字と下線() {
    let mut h = bracket_scene(ColorDepth::Mono);
    let look = Look {
        depth: ColorDepth::Mono,
        ..DEFAULT_LOOK
    };
    check_look("bracket-match-mono", &mut h, look);
    let (_, styles) = screen(&mut h, look);
    let line = row0(&styles);
    assert!(line.contains("4-4 Reset Reset BOLD | UNDERLINED"), "{line}");
    assert!(
        line.contains("11-11 Reset Reset BOLD | UNDERLINED"),
        "{line}"
    );
}

// verifies: AC-FR-308-1
#[test]
fn 対応括弧は選択の反転に太字と下線を重ねる() {
    // `(` を選ぶと head は `(` の直後。直後の `a` は括弧でないので、直前の `(` とその対応を強調する
    let mut h = bracket_scene(ColorDepth::Mono);
    h.keys("shift+right");
    let look = Look {
        depth: ColorDepth::Mono,
        ..DEFAULT_LOOK
    };
    check_look("bracket-match-selection-mono", &mut h, look);
    let (_, styles) = screen(&mut h, look);
    let line = row0(&styles);
    assert!(
        line.contains("4-4 Reset Reset BOLD | UNDERLINED | REVERSED"),
        "選択の上の ( : {line}"
    );
    assert!(
        line.contains("11-11 Reset Reset BOLD | UNDERLINED"),
        "{line}"
    );
}

// verifies: AC-FR-308-1
#[test]
fn 対応括弧は検索の一致の色に太字を重ねる() {
    // `(a) (a)` の行末 (後ろの `)` の直後) で `(a` を検索する。現在の一致は先頭の `(a` で、後ろの `(a` (ふつうの一致) の中の
    // `(` が対応括弧 (一致の色と下線に太字が足される)
    let h = Harness::new().with_size(80, 14);
    let mut h = h.with_file("/m.txt", "(a) (a)\n".as_bytes());
    h.open("/m.txt");
    h.run_jobs();
    h.keys("end ctrl+f");
    h.text("(a");
    check("bracket-match-search", &mut h);
    let (_, styles) = screen(&mut h, DEFAULT_LOOK);
    let line = row0(&styles);
    // 後ろの `(` (列 7): ふつうの一致の色 + 下線 + 太字。対応の `)` (列 9) は太字と下線だけ
    assert!(
        line.contains("7-7 Rgb(26, 26, 26) Rgb(230, 200, 74) BOLD | UNDERLINED"),
        "{line}"
    );
    assert!(line.contains("9-9 Reset Reset BOLD | UNDERLINED"), "{line}");
}

// ---- 高コントラストのテーマ (07 §8.7、09 §3.5 の (g)) ----

/// 制御文字と、大文字を含む検索語 (`Fo`) に何度も一致する語を含む文書。
const HC_MEMO: &str = "こんにちは、世界\nhello Fox \u{1b} Foo\n\tタブと　全角スペース\n";

/// テーマ `theme` (設定ファイルから。eve-tui は設定の型を知らない) と色の段 `depth` で、`HC_MEMO` を開いた Harness。
fn high_contrast_opened(theme: &str, depth: ColorDepth) -> Harness {
    let mut h = Harness::new()
        .with_size(80, 14)
        .with_caps(Capabilities {
            color_depth: depth,
            ..Capabilities::default()
        })
        .with_file("/memo.txt", HC_MEMO.as_bytes());
    h.fs.add_file(
        "/evo/config/config.toml",
        format!("[theme]\nname = \"{theme}\"\n").as_bytes(),
    );
    h.cmd("config.reload");
    h.run_jobs();
    h.open("/memo.txt");
    h.run_jobs();
    h
}

fn depth_label(depth: ColorDepth) -> &'static str {
    match depth {
        ColorDepth::Mono => "mono",
        ColorDepth::Ansi16 => "ansi16",
        ColorDepth::Ansi256 => "ansi256",
        _ => "truecolor",
    }
}

/// 1 行目の先頭の 3 文字を選んでから、検索バーで `Fo` を探した画面。選択 (1 行目)・現在の一致 (`Fox`)・検索の一致
/// (`Foo`)・大文字小文字のトグルの ON (`[Aa~]`。大文字を含むので区別している)・ステータスバー・ヒントバー・行番号
/// (現在の行番号を含む)・空白と制御文字の記号・`~` が 1 枚に出る。
fn high_contrast(theme: &str, depth: ColorDepth) {
    let mut h = high_contrast_opened(theme, depth);
    h.keys("shift+right shift+right shift+right");
    h.keys("ctrl+f");
    h.text("Fo");
    h.run_jobs();
    let look = Look {
        depth,
        ..DEFAULT_LOOK
    };
    check_look(&format!("{theme}-{}", depth_label(depth)), &mut h, look);
}

/// `Ctrl+K` を押してチョードを保留した画面 (ステータスバーの上に `ui.status_bar.chord` を重ねる。07 §8.6 の
/// 「重ねるロール」はステータスバーの色を通す)。
fn high_contrast_chord(theme: &str, depth: ColorDepth) {
    let mut h = high_contrast_opened(theme, depth);
    h.keys("ctrl+k");
    let look = Look {
        depth,
        ..DEFAULT_LOOK
    };
    check_look(
        &format!("{theme}-{}-chord", depth_label(depth)),
        &mut h,
        look,
    );
}

// verifies: AC-NFR-604-2
#[test]
fn 高コントラスト_チョードの保留はステータスバーの色に重ねる() {
    for depth in [ColorDepth::TrueColor, ColorDepth::Ansi16] {
        high_contrast_chord("high-contrast-dark", depth);
        high_contrast_chord("high-contrast-light", depth);
    }
}

// verifies: AC-NFR-604-2
#[test]
fn 高コントラスト_暗い_4_段() {
    for depth in [
        ColorDepth::TrueColor,
        ColorDepth::Ansi256,
        ColorDepth::Ansi16,
        ColorDepth::Mono,
    ] {
        high_contrast("high-contrast-dark", depth);
    }
}

// verifies: AC-NFR-604-2
#[test]
fn 高コントラスト_明るい_4_段() {
    for depth in [
        ColorDepth::TrueColor,
        ColorDepth::Ansi256,
        ColorDepth::Ansi16,
        ColorDepth::Mono,
    ] {
        high_contrast("high-contrast-light", depth);
    }
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

// ---- 複数ペイン (v0.5 の M30、FR-509。09 §3.4 の複数ペインの計画の (4)) ----

/// `MEMO` を開き、右に分割して右のペインで 1 文字入れ、下に分割した画面 (左 1・右上 1・右下 1 の 3 つ。フォーカスは右下)。
fn split_panes(h: Harness) -> Harness {
    let mut h = opened(h);
    h.keys("ctrl+\\");
    h.keys("down");
    h.text("x");
    h.keys("ctrl+k -");
    h
}

/// 見出しの行 (編集面の行 `y`) の文字列。
fn line(h: &mut Harness, y: u16) -> String {
    let (text, _) = screen(h, DEFAULT_LOOK);
    text.lines()
        .nth(usize::from(y))
        .unwrap_or_default()
        .to_owned()
}

// verifies: AC-FR-509-1
#[test]
fn 複数ペイン_見出しと区切りとフォーカスの強調() {
    let mut h = split_panes(Harness::new().with_size(80, 14));
    check("panes", &mut h);
    // ハードウェアカーソルは、フォーカスのペインの rect の位置を足した所 (03 §9.9 の PANE-10 (a))
    let f = h.frame();
    let p = f
        .panes
        .iter()
        .find(|p| p.active)
        .expect("フォーカスのペイン");
    let c = f.primary_cursor.expect("カーソル").pos;
    assert!(
        c.col >= p.rect.col && c.row >= p.rect.row,
        "{c:?} {:?}",
        p.rect
    );
    let r = render_screen(
        &f,
        CellSize {
            cols: 80,
            rows: f.size.rows + eve_tui::buffer::chrome_rows(f.size.rows, f.hint_bar),
        },
        ColorDepth::TrueColor,
        &Glyphs::unicode(),
    );
    assert_eq!(r.cursor, Some(c));
    // フォーカスのペインの見出しは `▶` で始まり、ほかの見出しは `─` で埋める
    let bar_y = p.bar.as_ref().expect("見出し").rect.row;
    assert!(
        line(&mut h, bar_y).contains("│▶ memo.txt"),
        "{}",
        line(&mut h, bar_y)
    );
}

// verifies: AC-FR-509-1
#[test]
fn 複数ペイン_無色の段と_ascii_の縮退を重ねてもフォーカスが分かる() {
    let mut h = split_panes(Harness::new().with_size(80, 14).with_caps(Capabilities {
        color_depth: ColorDepth::Mono,
        ascii_only: true,
        ..Capabilities::default()
    }));
    let look = Look {
        depth: ColorDepth::Mono,
        glyphs: Glyphs::ascii(),
    };
    check_look("panes-mono-ascii", &mut h, look);
}

// verifies: AC-FR-509-1
#[test]
fn 複数ペイン_狭い幅では見出しの項目を省略する() {
    // 41 列 (左右に分けた最小) と長いファイル名: 名前の中間を省略し、変更の印は残す (04 §8.7)
    let mut h = Harness::new()
        .with_size(41, 14)
        .with_file("/a-rather-long-file-name.txt", MEMO.as_bytes());
    h.open("/a-rather-long-file-name.txt");
    h.run_jobs();
    h.keys("ctrl+\\");
    h.text("x");
    check("panes-narrow", &mut h);
}

/// ファインダ (04 §8.11): 長いルートの見出しと長い相対パスの行は、中間を `…` で詰める (`CutPolicy::Middle`)。一致した文字の
/// 強調は、詰めた後も元の文字に付く (スタイルの層で確かめる)。
#[test]
fn ファインダの長いルートと長いパスは中間を詰める() {
    let root = "/w/a-very-long-workspace-directory-name-for-the-finder-title/and-it-goes-on-and-on";
    let file = format!(
        "{root}/src/components/deeply/nested/directory/structure/for/testing/the/cut/main_component.rs"
    );
    let mut h = Harness::builder()
        .size(80, 14)
        .defer_start()
        .dir(&format!("{root}/.git"))
        .file(&file, b"\n")
        .build();
    h.editor
        .open_initial_files(&[eve_core::InitialFile::path(&file)], false, None);
    h.started();
    h.run_jobs();
    h.keys("ctrl+o");
    h.text("main");
    h.run_jobs();
    // ルートの表示の区切りは OS で違う (Windows は `\`)。文字の層は `/` に揃えて比べる (幅は同じ)
    let (text, styles) = screen(&mut h, DEFAULT_LOOK);
    golden::check(&dir().join("finder-long.txt"), &text.replace('\\', "/"));
    golden::check(&dir().join("finder-long.styles.txt"), &styles);
}

/// 横断検索の結果 (03 §12.7.5、04 §8.12): 見出しのトグル、`パス:行: 抜粋` の行と一致の強調、件数の欄。
#[test]
fn 横断検索の結果() {
    let mut h = Harness::builder()
        .size(80, 14)
        .defer_start()
        .dir("/w/proj/.git")
        .file(
            "/w/proj/src/a.rs",
            b"fn main() {\n    let needle = 1; // needle\n}\n",
        )
        .file("/w/proj/b.txt", b"needle\n")
        .build();
    h.editor
        .open_initial_files(&[eve_core::InitialFile::path("/w/proj/b.txt")], false, None);
    h.started();
    h.run_jobs();
    h.keys("ctrl+shift+f");
    h.text("needle");
    h.run_jobs();
    let (text, styles) = screen(&mut h, DEFAULT_LOOK);
    golden::check(&dir().join("workspace-search.txt"), &text);
    golden::check(&dir().join("workspace-search.styles.txt"), &styles);
}

// ---- タブライン (v0.5 の M32、FR-510。03 §4.7.1 の L4) ----

/// `files` を順に開いた Harness (最後に開いた文書を表示する)。
fn opened_files(h: Harness, files: &[(&str, &[u8])]) -> Harness {
    let mut h = h;
    for (p, d) in files {
        h.fs.add_file(p, d);
    }
    for (p, _) in files {
        h.open(p);
        h.run_jobs();
    }
    h
}

/// 4 つの文書 (同じ名前の 2 つを含む) を開き、先頭の文書をアクティブにして 1 文字入れた画面。
fn tab_line_docs(h: Harness) -> Harness {
    let mut h = opened_files(
        h,
        &[
            ("/w/main.rs", MEMO.as_bytes()),
            ("/w/lib.rs", b"lib\n"),
            ("/p/a/mod.rs", b"a\n"),
            ("/p/b/mod.rs", b"b\n"),
        ],
    );
    h.cmd("buffer.next");
    h.text("x");
    h
}

/// 画面の最上行 (タブライン) の文字列。
fn top_line(h: &mut Harness, look: Look) -> String {
    let (text, _) = screen(h, look);
    text.lines().next().unwrap_or_default().to_owned()
}

// verifies: AC-FR-510-1
#[test]
fn タブライン_全部入る幅ではアクティブなタブを反転と印で示し_編集面を_1_行下げる() {
    let mut h = tab_line_docs(Harness::new().with_size(80, 14));
    check("tab-line", &mut h);
    let line = top_line(&mut h, DEFAULT_LOOK);
    assert!(
        line.starts_with("[▶main.rs [+] ] [ lib.rs ] [ mod.rs (a) ] [ mod.rs (b) ]"),
        "{line}"
    );
    // ハードウェアカーソルは編集面の座標に、タブラインの 1 行を足した所
    let f = h.frame();
    let c = f.primary_cursor.expect("カーソル").pos;
    let base = f.size.rows + eve_tui::buffer::tab_rows(&f);
    let r = render_screen(
        &f,
        CellSize {
            cols: 80,
            rows: base + eve_tui::buffer::chrome_rows(base, f.hint_bar),
        },
        ColorDepth::TrueColor,
        &Glyphs::unicode(),
    );
    assert_eq!(
        r.cursor,
        Some(eve_view::CellPos {
            col: c.col,
            row: c.row + 1
        })
    );
}

// verifies: AC-FR-510-1
#[test]
fn タブライン_無色の段と_ascii_の縮退でも_アクティブと省いた側が分かる() {
    let mut h = tab_line_docs(Harness::new().with_size(40, 8).with_caps(Capabilities {
        color_depth: ColorDepth::Mono,
        ascii_only: true,
        ..Capabilities::default()
    }));
    // 2 つ目のタブ (lib.rs) をアクティブにする: 40 桁には入らない。lib.rs から右の mod.rs (a) を足し、左の main.rs
    // (15 セル) は入らないので左右に `<` `>`
    h.cmd("buffer.next");
    let look = Look {
        depth: ColorDepth::Mono,
        glyphs: Glyphs::ascii(),
    };
    check_look("tab-line-mono-ascii", &mut h, look);
    let line = top_line(&mut h, look);
    assert!(line.starts_with("<[>lib.rs ] [ mod.rs (a) ]"), "{line}");
    assert!(line.ends_with('>'), "{line}");
}

/// 描いたタブライン (ASCII の縮退) は、ヘッドレスの ASCII の Frame の最上行 (`eve_testkit::tab_line_text`) と同じ並びに
/// なる (03 §9.9 の PANE-12 (h)。並べ方を 2 か所に書いているので、食い違いをここで捕まえる)。マウスの幾何のタブの矩形は、
/// 描いた `[` から `]` までと一致する (PANE-12 (g))。
// verifies: AC-FR-510-1, AC-FR-510-2
#[test]
fn タブライン_描いた並びはヘッドレスの_ascii_の_frame_と同じで_マウスの矩形はタブと一致する() {
    let files: Vec<(String, Vec<u8>)> = [
        "/w/a.txt",
        "/w/全角の名前.txt",
        "/w/a-quite-long-file-name-that-is-cut-in-the-middle.txt",
        "/w/b.txt",
        "/x/b.txt",
        "/w/\u{202e}evil.txt",
        "/w/c.txt",
    ]
    .iter()
    .map(|p| ((*p).to_owned(), b"x\n".to_vec()))
    .collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_slice()))
        .collect();
    for cols in [20u16, 23, 31, 40, 57, 80, 120, 200] {
        let mut h = opened_files(Harness::new().with_size(cols, 6), &refs);
        for step in 0..files.len() {
            if step % 2 == 1 {
                h.text("x");
            }
            let f = h.frame();
            let model = f.tab_line.as_ref().expect("タブライン");
            let want = eve_testkit::tab_line_text(model, cols);
            let look = Look {
                depth: ColorDepth::Mono,
                glyphs: Glyphs::ascii(),
            };
            let got = top_line(&mut h, look);
            assert_eq!(got, want, "{cols} 桁、アクティブ {:?}", model.active);
            // マウスの幾何のタブ: 矩形の両端が `[` と `]` (行の幅で切ったものを除く)
            let r = render_screen(
                &f,
                CellSize {
                    cols,
                    rows: f.size.rows + 1 + eve_tui::buffer::chrome_rows(f.size.rows + 1, true),
                },
                ColorDepth::Mono,
                &Glyphs::ascii(),
            );
            let g = eve_tui::mouse::Geometry::from_frame(&f, &Glyphs::ascii());
            let active_doc = model.active.map(|i| model.tabs[i].doc);
            assert!(
                g.tabs.iter().any(|t| Some(t.doc) == active_doc),
                "アクティブなタブは並べた範囲にある"
            );
            for t in &g.tabs {
                let first = r.buffer[(t.rect.col, 0)].symbol().to_owned();
                assert_eq!(first, "[", "{cols} 桁 {t:?}");
                let end = t.rect.col + t.rect.cols - 1;
                if end + 1 < cols {
                    assert_eq!(r.buffer[(end, 0)].symbol(), "]", "{cols} 桁 {t:?}");
                }
            }
            h.cmd("buffer.next");
        }
    }
}
