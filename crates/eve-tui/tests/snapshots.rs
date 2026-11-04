//! 主要画面のスナップショット (01 NFR-603 AC-6、docs/09 §3)。`ja` の文言で、端末に描く画面 (本文・ステータスバー・ヒントバー・
//! メッセージ行・オーバーレイ) を行の文字列にして固定し、比べる。
//! 画面を意図して変えたときは `EVE_UPDATE_SNAPSHOTS=1 cargo nextest run -p eve-tui --test snapshots` で作り直し、差分を見て commit する。

use std::path::PathBuf;

use eve_testkit::Harness;
use eve_tui::buffer::{chrome_rows_for_surface, row_text};
use eve_tui::chrome::render_screen;
use eve_tui::glyphs::Glyphs;
use eve_view::{CellSize, ColorDepth};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

fn screen(h: &mut Harness) -> String {
    let f = h.frame();
    let base = f.size.rows + eve_tui::buffer::bar_rows(&f);
    let window = CellSize {
        cols: f.size.cols,
        rows: base + chrome_rows_for_surface(base),
    };
    let r = render_screen(&f, window, ColorDepth::TrueColor, &Glyphs::unicode());
    let mut out: Vec<String> = (0..r.buffer.area.height)
        .map(|y| row_text(&r.buffer, y).trim_end().to_owned())
        .collect();
    out.push(format!("cursor: {:?}", r.cursor));
    out.join("\n") + "\n"
}

fn check(name: &str, h: &mut Harness) {
    let got = screen(h);
    let path = dir().join(format!("{name}.txt"));
    if std::env::var_os("EVE_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &got).expect("スナップショットを書ける");
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| {
            panic!(
                "{} がありません (EVE_UPDATE_SNAPSHOTS=1 で作る)",
                path.display()
            )
        })
        .replace("\r\n", "\n");
    assert_eq!(got, want, "スナップショット {name} が違います");
}

fn with_file() -> Harness {
    let mut h = Harness::new().with_size(80, 14).with_file(
        "/memo.txt",
        "こんにちは、世界\nhello fox\n\tタブと　全角スペース\n".as_bytes(),
    );
    h.open("/memo.txt");
    h.run_jobs();
    h
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
