//! AC の棚卸し (M5) で、実装はあるのに検査するテストが無かったものの穴埋め
//! (改行コード・読み取り専用・文字コードの開き直し・貼り付け・ガター幅)。

use eve_core::{Args, Encoding, ReadOnlyReason};
use eve_testkit::Harness;
use eve_text::LineEnding;
use eve_view::{ClipboardToken, Effect, StatusKind};

fn status_text(h: &mut Harness, kind: StatusKind) -> String {
    h.frame()
        .status
        .items
        .iter()
        .find(|i| i.kind == kind)
        .map(|i| i.long.spans.iter().map(|s| s.text.to_string()).collect())
        .unwrap_or_default()
}

fn default_eol(h: &Harness) -> LineEnding {
    let id = h.doc_id();
    h.editor.state().document(id).unwrap().format.default_eol
}

fn opened(path: &str, bytes: &[u8]) -> Harness {
    let mut h = Harness::new().with_file(path, bytes);
    h.open(path);
    h.run_jobs();
    h
}

fn request_token(h: &Harness) -> ClipboardToken {
    h.effects()
        .iter()
        .rev()
        .find_map(|e| match e {
            Effect::RequestClipboard { token, .. } => Some(*token),
            _ => None,
        })
        .expect("RequestClipboard")
}

// ---------- FR-113 ----------

// verifies: AC-FR-113-2
#[test]
fn 新規文書で_enter_を押すと_lf_が入る() {
    let mut h = Harness::new();
    h.text("a");
    h.keys("enter");
    h.text("b");
    assert_eq!(h.doc_text(), "a\nb");
    assert_eq!(default_eol(&h), LineEnding::Lf);
}

// verifies: AC-FR-113-3
#[test]
fn 貼り付けの改行は経路によらず文書の既定の改行になる() {
    for (eol, want) in [("crlf", "a\r\nb"), ("lf", "a\nb")] {
        // ブラケットペースト (LF / CRLF / CR の入力)
        for src in ["a\nb", "a\r\nb", "a\rb"] {
            let mut h = Harness::new();
            h.cmd_with("file.set_line_ending", Args::new().with_str("eol", eol));
            h.paste(src);
            assert_eq!(h.doc_text(), want, "{eol} <- {src:?}");
        }
        // システムクリップボード
        let mut h = Harness::new();
        h.cmd_with("file.set_line_ending", Args::new().with_str("eol", eol));
        h.keys("ctrl+v");
        let t = request_token(&h);
        h.clipboard_content(t, Some("a\r\nb"));
        assert_eq!(h.doc_text(), want, "{eol} クリップボード");
        // 内部レジスタ (システムクリップボードを読めなかった)
        let mut h = Harness::new();
        h.text("a");
        h.keys("enter");
        h.text("b");
        h.keys("ctrl+a ctrl+c");
        h.cmd_with("file.set_line_ending", Args::new().with_str("eol", eol));
        h.keys("ctrl+end ctrl+v");
        let t = request_token(&h);
        h.clipboard_content(t, None);
        assert!(
            h.doc_text().ends_with(want),
            "{eol} レジスタ: {:?}",
            h.doc_text()
        );
    }
}

// verifies: AC-FR-113-4
#[test]
fn 改行コードの変換は_lf_crlf_cr_のどれでも全部そろい_状態表示と次の_enter_も従う() {
    for (eol, name, unit) in [
        ("lf", LineEnding::Lf, "\n"),
        ("crlf", LineEnding::Crlf, "\r\n"),
        ("cr", LineEnding::Cr, "\r"),
    ] {
        let mut h = opened("/w/m.txt", b"a\nb\r\nc\rd");
        assert!(status_text(&mut h, StatusKind::Eol).contains("混在"));
        h.cmd_with("file.set_line_ending", Args::new().with_str("eol", eol));
        let id = h.doc_id();
        let d = h.editor.state().document(id).unwrap();
        assert!(d.is_dirty(), "{eol}");
        assert!(!d.format.eol_stats.is_mixed(), "{eol}");
        assert_eq!(d.format.default_eol, name);
        assert!(
            !status_text(&mut h, StatusKind::Eol).contains("混在"),
            "{eol}"
        );
        h.keys("ctrl+end enter");
        h.text("z");
        assert_eq!(
            h.doc_text(),
            format!("a{unit}b{unit}c{unit}d{unit}z"),
            "{eol}"
        );
        // 1 回の Undo で変換の前に戻る (Enter と入力は別の Undo 単位)
        h.keys("ctrl+z ctrl+z ctrl+z");
        assert!(
            status_text(&mut h, StatusKind::Eol).contains("混在"),
            "{eol}"
        );
        assert_eq!(default_eol(&h), LineEnding::Lf, "{eol}");
    }
}

// verifies: AC-FR-113-4
#[test]
fn 改行がすでにそろっていれば文書も履歴も変わらず_読み取り専用では拒否する() {
    let mut h = opened("/w/l.txt", b"a\nb\n");
    h.cmd_with("file.set_line_ending", Args::new().with_str("eol", "lf"));
    let id = h.doc_id();
    assert!(!h.editor.state().document(id).unwrap().is_dirty());
    assert!(h.messages().contains(&"info.eol.unchanged".to_owned()));
    let mut ro = Harness::new().with_file("/w/r.txt", b"a\r\nb\n");
    ro.editor.state_mut().open_path(
        "/w/r.txt",
        eve_core::OpenOptions {
            user_read_only: true,
            ..Default::default()
        },
    );
    ro.run_jobs();
    ro.cmd_with("file.set_line_ending", Args::new().with_str("eol", "lf"));
    assert_eq!(ro.doc_text(), "a\r\nb\n");
}

// ---------- FR-502 ----------

// verifies: AC-FR-502-1
#[test]
fn ガター幅は行数の桁数に応じて変わる() {
    let width = |lines: usize| {
        let mut h = Harness::new().with_size(80, 10);
        h.paste(&"x\n".repeat(lines - 1));
        h.frame().panes[0].gutter_width
    };
    // 行数 = 区切り + 1。最小 2 桁 + 区切りの 1 セル
    assert_eq!(width(9), 3);
    assert_eq!(width(99), 3);
    assert_eq!(width(100), 4);
    assert_eq!(width(999), 4);
    assert_eq!(width(1000), 5);
}

// ---------- FR-111 / FR-115 ----------

fn utf16le_no_bom(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

const JP: &str = "日本語のテキストエディタです。これは文章の見本で、文字コードの判定に使います。\n二行目の文章です。\n";

fn reopen_as(h: &mut Harness, enc: &str) {
    // 開くときの確認 (バイナリ・不正バイトなど) が出ていれば、読み取り専用で開く
    if h.overlay() == "dialog" {
        h.keys("r");
        h.run_jobs();
    }
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", enc),
    );
    h.run_jobs();
}

// verifies: AC-FR-111-1, AC-FR-115-2
#[test]
fn 誤判定された_utf16le_は_文字コードを指定して開き直すと正しく表示できる() {
    let mut h = Harness::new().with_file("/w/u.txt", &utf16le_no_bom(JP));
    h.open("/w/u.txt");
    h.run_jobs();
    reopen_as(&mut h, "utf-16le");
    assert_eq!(h.doc_text(), JP);
    let id = h.doc_id();
    assert_eq!(
        h.editor.state().document(id).unwrap().format.encoding,
        Encoding::Utf16Le
    );
}

// verifies: AC-FR-111-1
#[test]
fn shift_jis_を誤判定しても_指定して開き直すと正しく表示できる() {
    let (sj, _, _) = encoding_rs::SHIFT_JIS.encode("ｱｲｳｴｵ 半角カナと漢字 髙﨑 ｶﾞｷﾞ\n");
    let mut h = Harness::new().with_file("/w/s.txt", &sj);
    h.open("/w/s.txt");
    h.run_jobs();
    reopen_as(&mut h, "euc-jp");
    reopen_as(&mut h, "shift_jis");
    assert_eq!(h.doc_text(), "ｱｲｳｴｵ 半角カナと漢字 髙﨑 ｶﾞｷﾞ\n");
}

// verifies: AC-FR-111-4
#[test]
fn 開き直すと表示は内容に合い_履歴は消えて通知は_1_回() {
    let (sj, _, _) = encoding_rs::SHIFT_JIS.encode(JP);
    let mut h = opened("/w/s.txt", &sj);
    h.text("x");
    // 未保存を破棄して開き直す
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    if h.overlay() == "dialog" {
        h.keys("d");
    }
    h.run_jobs();
    let id = h.doc_id();
    let d = h.editor.state().document(id).unwrap();
    assert_eq!(d.format.encoding, Encoding::ShiftJis);
    assert!(!d.is_dirty(), "未変更");
    assert!(d.read_only.is_empty(), "読み取り専用でない");
    assert!(
        status_text(&mut h, StatusKind::Encoding).contains("Shift_JIS")
            || status_text(&mut h, StatusKind::Encoding).contains("SJIS")
    );
    assert_eq!(
        h.messages()
            .iter()
            .filter(|m| *m == "info.reopen.history_cleared")
            .count(),
        1
    );
    let before = h.doc_text();
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), before, "開き直しは Undo できない");
}

// ---------- FR-116 ----------

// verifies: AC-FR-116-1, AC-FR-106-1
#[test]
fn 読み取り専用属性と_readonly_のファイルは編集操作を理由付きで拒否する() {
    for via_attr in [true, false] {
        let mut h = Harness::new().with_file("/w/ro.txt", b"abc\n");
        if via_attr {
            h.fs.set_readonly("/w/ro.txt", true);
            h.open("/w/ro.txt");
        } else {
            h.editor.state_mut().open_path(
                "/w/ro.txt",
                eve_core::OpenOptions {
                    user_read_only: true,
                    ..Default::default()
                },
            );
        }
        h.run_jobs();
        let id = h.doc_id();
        let d = h.editor.state().document(id).unwrap();
        assert!(d.read_only.contains(if via_attr {
            ReadOnlyReason::FileAttribute
        } else {
            ReadOnlyReason::UserRequested
        }));
        assert!(status_text(&mut h, StatusKind::ReadOnly).contains("RO"));
        for op in ["x", "backspace", "delete", "enter", "tab"] {
            h.clear_log();
            if op.len() == 1 {
                h.text(op);
            } else {
                h.keys(op);
            }
            assert_eq!(h.doc_text(), "abc\n", "{op}");
            assert!(
                h.messages().contains(&"msg.edit.read_only".to_owned()),
                "{op}: 理由付き"
            );
        }
        h.paste("zzz");
        h.keys("ctrl+z ctrl+y");
        assert_eq!(h.doc_text(), "abc\n");
        // 名前を付けて保存で別のパスに書いても、元のファイルの内容は変わらない
        h.cmd_with("file.save_as", Args::new().with_str("path", "/w/copy.txt"));
        assert_eq!(h.fs_read("/w/ro.txt").unwrap(), b"abc\n");
    }
}

// ---------- FR-202 / FR-206 ----------

// verifies: AC-FR-202-3
#[test]
fn 選択して_enter_は選択を改行に置き換え_1_回の_undo_で戻る() {
    let mut h = Harness::new();
    h.paste("    foo bar baz\n");
    h.keys("ctrl+home");
    h.keys("right right right right right right right right right"); // "    foo b" の後ろ
    h.keys("shift+right shift+right shift+right");
    let before = h.doc_text();
    h.keys("enter");
    assert_eq!(
        h.doc_text(),
        "    foo b\n     baz\n".replace("     baz", "    baz")
    );
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), before);
}

// verifies: AC-FR-206-1
#[test]
fn インデント付きの_100_行を貼ると元のインデントのまま_1_回の_undo_で消える() {
    let mut h = Harness::new();
    let src: String = (0..100)
        .map(|i| format!("{}line{i}\n", "  ".repeat(i % 5)))
        .collect();
    h.paste(&src);
    assert_eq!(h.doc_text(), src, "自動インデントは適用しない");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "");
}
