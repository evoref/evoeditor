//! 不正バイトのエスケープ文字のコピーと貼り付け (docs/03 §8.3.1 の「本文に入れる規則」、§12.6、§22.3 の「不正バイトを含む
//! 文書の編集」の (d)、v0.4 の M23 の 5 本目)。保存先は `MemFs`、システムのクリップボードは `RequestClipboard` への応答で
//! 偽装する。
//!
//! 不正バイトを含む文書は、今は読み取り専用 (「編集を許可」は M23 の後の PR)。貼り付け先として使うときは、試験の中で文書の
//! `read_only` から `InvalidBytes` を直に外して、解除した後の状態を作る (`escape_encoding.rs` の前例。製品の解除の経路は足さない)。

use eve_core::{
    Args, ClipboardFallbackReason, Document, Encoding, HostReport, OpenOptions, ReadOnlyReason,
};
use eve_encoding::PuaEscapeMode;
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_view::{ClipboardText, Effect};

fn env() -> Harness {
    Harness::builder().defer_start().build()
}

/// 不正バイトが多いファイルの確認 (03 §8.3) が出ていれば「読み取り専用で開く」を選ぶ。
fn accept_read_only(h: &mut Harness) {
    if h.overlay() == "dialog" {
        h.text("r");
        h.run_jobs();
    }
}

/// 文字コードを指定して開く (読み取り専用はそのまま)。
fn raw_open(h: &mut Harness, path: &str, enc: Encoding) {
    h.editor.state_mut().open_path(
        path,
        OpenOptions {
            forced: Some(enc),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    accept_read_only(h);
}

/// 文字コードを指定して開き、`InvalidBytes` があれば外す (「編集を許可」の後の状態。試験だけの近道)。
fn open_as(h: &mut Harness, path: &str, enc: Encoding) {
    h.editor.state_mut().open_path(
        path,
        OpenOptions {
            forced: Some(enc),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    accept_read_only(h);
    let id = h.doc_id();
    let d = h.editor.state_mut().document_mut(id).unwrap();
    d.read_only.remove(ReadOnlyReason::InvalidBytes);
    assert!(
        d.is_editable(),
        "{path}: {:?}",
        d.read_only.iter().collect::<Vec<_>>()
    );
}

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

fn message(h: &mut Harness) -> String {
    h.frame()
        .messages
        .current
        .map(|m| cell_text(&m.text))
        .unwrap_or_default()
}

/// これまでに出た `info.paste.*` のキー。
fn paste_notices(h: &Harness) -> Vec<String> {
    h.messages()
        .into_iter()
        .filter(|k| k.starts_with("info.paste."))
        .collect()
}

/// 最後の `SetClipboard` の中身。
fn last_set_clipboard(h: &Harness) -> ClipboardText {
    h.effects()
        .iter()
        .rev()
        .find_map(|e| match e {
            Effect::SetClipboard { content, .. } => Some(content.clone()),
            _ => None,
        })
        .expect("SetClipboard")
}

/// `edit.paste` を実行し、システムのクリップボードの読取の応答として `text` を返す。
fn paste_from_system(h: &mut Harness, text: Option<&str>) {
    h.keys("ctrl+v");
    let token = h
        .effects()
        .iter()
        .rev()
        .find_map(|e| match e {
            Effect::RequestClipboard { token, .. } => Some(*token),
            _ => None,
        })
        .expect("RequestClipboard");
    h.clipboard_content(token, text);
}

const ESC_A0: char = '\u{10FFA0}';

/// 不正バイト (0xA0) を含む `Shift_JIS` の文書 `/w/sjis.txt` (本文 `ab<0xA0>c\n`) を開いて解除した Harness。
fn sjis_env() -> Harness {
    let mut h = env();
    h.fs.add_file("/w/sjis.txt", b"ab\xA0c\n");
    open_as(&mut h, "/w/sjis.txt", Encoding::ShiftJis);
    assert_eq!(doc(&h).format.escape_encoding, Some(Encoding::ShiftJis));
    h
}

/// 3 文字目 (エスケープ文字) を選んでコピーする。
fn copy_escape(h: &mut Harness) {
    h.keys("ctrl+home right right shift+right ctrl+c");
}

// ---------- コピー ----------

// verifies: NFR-304
#[test]
fn コピーはシステムのクリップボードへ表記の文字列を渡し_文書は変えない() {
    let mut h = sjis_env();
    h.keys("ctrl+a ctrl+c");
    assert_eq!(
        last_set_clipboard(&h),
        ClipboardText::Inline("ab<0xA0>c\n".to_owned())
    );
    assert_eq!(h.doc_text(), format!("ab{ESC_A0}c\n"));
}

// verifies: NFR-304
#[test]
fn 大きい選択のハンドルも_to_text_で表記の文字列になる() {
    let mut h = env();
    let mut bytes = b"x\x80y\n".to_vec();
    bytes.extend(std::iter::repeat_n(b'a', 70 * 1024));
    h.fs.add_file("/w/big.txt", &bytes);
    open_as(&mut h, "/w/big.txt", Encoding::Utf8);
    h.keys("ctrl+a ctrl+c");
    let ClipboardText::Handle(id) = last_set_clipboard(&h) else {
        panic!("ハンドルで渡す");
    };
    let src = h.editor.take_clipboard(id).expect("取り出せる");
    assert_eq!(
        src.len_bytes(),
        bytes.len() + 3,
        "上限の判定は文字列化の前のバイト数"
    );
    let text = src.to_text();
    assert!(text.starts_with("x<0x80>y\n"), "{:?}", &text[..12]);
    assert_eq!(text.len(), bytes.len() + 5);
}

// verifies: NFR-304
#[test]
fn 衝突の文書のコピーは私用文字のままシステムのクリップボードへ渡す() {
    let mut h = env();
    h.fs.add_file("/w/c.txt", "x\u{10FF80}y".as_bytes());
    open_as(&mut h, "/w/c.txt", Encoding::Utf8);
    assert_eq!(doc(&h).format.pua_escape, PuaEscapeMode::DisabledCollision);
    h.keys("ctrl+a ctrl+c");
    assert_eq!(
        last_set_clipboard(&h),
        ClipboardText::Inline("x\u{10FF80}y".to_owned())
    );
}

// ---------- 貼り付け: コピー元 × 貼り付け先の表 ----------

// verifies: NFR-304
#[test]
fn 同じ文書で切り取り_貼り付けるとバイトのまま入り_保存で同じバイトに戻る() {
    let mut h = sjis_env();
    h.keys("ctrl+home right right shift+right ctrl+x");
    assert_eq!(h.doc_text(), "abc\n");
    // 行末へ動かして、OS のクリップボードの文字列 (表記) が届く
    h.keys("end");
    h.clear_log();
    paste_from_system(&mut h, Some("<0xA0>"));
    assert_eq!(h.doc_text(), format!("abc{ESC_A0}\n"));
    assert_eq!(paste_notices(&h), vec!["info.paste.as_bytes"]);
    assert_eq!(
        message(&mut h),
        "不正なバイト 1 個を、バイトのまま貼り付けました"
    );
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.fs.read("/w/sjis.txt").unwrap(), b"abc\xA0\n");
}

// verifies: NFR-304
#[test]
fn 端末からの貼り付けも内部レジスタと照合する() {
    let mut h = sjis_env();
    copy_escape(&mut h);
    h.keys("end");
    h.clear_log();
    h.paste("<0xA0>");
    assert_eq!(h.doc_text(), format!("ab{ESC_A0}c{ESC_A0}\n"));
    assert_eq!(paste_notices(&h), vec!["info.paste.as_bytes"]);
}

/// `Shift_JIS` の文書のエスケープ文字をコピーして、`path` (`enc` で開く) に貼り付けたときの本文と知らせ。
fn paste_into(path: &str, bytes: &[u8], enc: Encoding) -> (String, Vec<String>) {
    let mut h = sjis_env();
    copy_escape(&mut h);
    h.fs.add_file(path, bytes);
    open_as(&mut h, path, enc);
    h.keys("ctrl+end");
    h.clear_log();
    paste_from_system(&mut h, Some("<0xA0>"));
    (h.doc_text(), paste_notices(&h))
}

// verifies: NFR-304
#[test]
fn 別の文字コードのバイトの文書には表記の文字列で入れて知らせる() {
    let (text, notices) = paste_into("/w/euc.txt", b"q\xFF\n", Encoding::EucJp);
    assert_eq!(text, "q\u{10FFFF}\n<0xA0>");
    assert_eq!(notices, vec!["info.paste.escape_as_text"]);
}

// verifies: NFR-304
#[test]
fn 不正バイトの無い文書には表記の文字列で入れて知らせる() {
    let (text, notices) = paste_into("/w/u.txt", b"q\n", Encoding::Utf8);
    assert_eq!(text, "q\n<0xA0>");
    assert_eq!(notices, vec!["info.paste.escape_as_text"]);
    // 同じ Shift_JIS でも、不正バイトの無い文書はバイトの文書でない
    let (text, notices) = paste_into("/w/s2.txt", b"q\n", Encoding::ShiftJis);
    assert_eq!(text, "q\n<0xA0>");
    assert_eq!(notices, vec!["info.paste.escape_as_text"]);
}

// verifies: NFR-304
#[test]
fn 不正バイトの無い_iso_2022_jp_の文書には表記の文字列で入れて知らせる() {
    let (text, notices) = paste_into("/w/j.txt", b"q\n", Encoding::Iso2022Jp);
    assert_eq!(text, "q\n<0xA0>");
    assert_eq!(notices, vec!["info.paste.escape_as_text"]);
}

// verifies: NFR-304
#[test]
fn 衝突の文書_a_には表記の文字列で入れて知らせる() {
    let (text, notices) = paste_into("/w/c.txt", "q\u{10FF80}".as_bytes(), Encoding::Utf8);
    assert_eq!(text, "q\u{10FF80}<0xA0>");
    assert_eq!(notices, vec!["info.paste.escape_as_text"]);
}

// verifies: NFR-304
#[test]
fn 衝突の文書の私用文字は_バイトの文書へはコードポイントの表記で_衝突の文書へはそのまま() {
    let mut h = env();
    h.fs.add_file("/w/c.txt", "x\u{10FF80}y".as_bytes());
    open_as(&mut h, "/w/c.txt", Encoding::Utf8);
    h.keys("ctrl+home right shift+right ctrl+c");
    // 同じ衝突の文書へ: そのまま
    h.keys("ctrl+end");
    h.clear_log();
    paste_from_system(&mut h, Some("\u{10FF80}"));
    assert_eq!(h.doc_text(), "x\u{10FF80}y\u{10FF80}");
    assert_eq!(paste_notices(&h), Vec::<String>::new());
    // Shift_JIS のバイトの文書へ: `<U+10FF80>`
    h.fs.add_file("/w/sjis.txt", b"ab\xA0c\n");
    open_as(&mut h, "/w/sjis.txt", Encoding::ShiftJis);
    h.keys("ctrl+end");
    h.clear_log();
    paste_from_system(&mut h, Some("\u{10FF80}"));
    assert_eq!(h.doc_text(), format!("ab{ESC_A0}c\n<U+10FF80>"));
    assert_eq!(paste_notices(&h), vec!["info.paste.pua_as_text"]);
}

// ---------- 照合が外れる ----------

// verifies: NFR-304
#[test]
fn クリップボードが別の文字列に替わっていれば内部レジスタを使わず知らせる() {
    let mut h = sjis_env();
    copy_escape(&mut h);
    h.keys("end");
    h.clear_log();
    // 利用者が別のアプリで、文字どおりの `<0xA0>` を含む文字列をコピーした
    paste_from_system(&mut h, Some("z<0xA0>"));
    assert_eq!(h.doc_text(), format!("ab{ESC_A0}cz<0xA0>\n"));
    assert_eq!(paste_notices(&h), vec!["info.paste.notation_as_text"]);
    assert_eq!(
        message(&mut h),
        "クリップボードの内容がエディタ内のコピーと違うため、<0xNN> の形の文字列は文字列のまま貼り付けました"
    );
}

// verifies: NFR-304
#[test]
fn 空白が変わった文字列は一致しない() {
    let mut h = sjis_env();
    copy_escape(&mut h);
    h.keys("end");
    h.clear_log();
    paste_from_system(&mut h, Some("<0xA0> "));
    assert_eq!(h.doc_text(), format!("ab{ESC_A0}c<0xA0> \n"));
    assert_eq!(paste_notices(&h), vec!["info.paste.notation_as_text"]);
}

// verifies: NFR-304
#[test]
fn 表記の形の知らせは内部レジスタがエスケープ文字を含む表記のコピーのときだけ() {
    // エスケープ文字を含まないコピー (バイトの文書の "a") では知らせない
    let mut h = sjis_env();
    h.keys("ctrl+home shift+right ctrl+c end");
    h.clear_log();
    paste_from_system(&mut h, Some("<0xA0>"));
    assert_eq!(h.doc_text(), format!("ab{ESC_A0}c<0xA0>\n"));
    assert_eq!(paste_notices(&h), Vec::<String>::new());
    // 不正バイトの無い文書からのコピーでも知らせない
    let mut h = env();
    h.fs.add_file("/w/u.txt", b"plain\n");
    open_as(&mut h, "/w/u.txt", Encoding::Utf8);
    h.keys("ctrl+home shift+right ctrl+c end");
    h.clear_log();
    paste_from_system(&mut h, Some("<0xA0>"));
    assert_eq!(h.doc_text(), "plain<0xA0>\n");
    assert_eq!(paste_notices(&h), Vec::<String>::new());
}

// 不正バイトを含む ISO-2022-JP (状態付き・不正バイトあり。読み取り専用だがコピーはできる) のエスケープ文字は 1 バイトの
// 意味を持つが、どこに貼ってもバイトとしては運べない。文字列になったことを知らせる (M23 の 5 本目のレビューの再現 r2)
// verifies: NFR-304
#[test]
fn 状態付きで不正バイトのある文書からのコピーは表記の文字列で入れて知らせる() {
    let mut h = env();
    h.fs.add_file("/w/a.txt", b"\x1B$B$\"\n\x1B(Bx");
    raw_open(&mut h, "/w/a.txt", Encoding::Iso2022Jp);
    assert_eq!(doc(&h).format.pua_escape, PuaEscapeMode::DisabledStateful);
    h.keys("ctrl+a ctrl+c");
    let ClipboardText::Inline(sys) = last_set_clipboard(&h) else {
        panic!("Inline で渡す");
    };
    assert_eq!(sys, "あ<0x0A>x");
    h.fs.add_file("/w/s.txt", b"ab\xA0c\n");
    open_as(&mut h, "/w/s.txt", Encoding::ShiftJis);
    h.keys("ctrl+end");
    h.clear_log();
    paste_from_system(&mut h, Some(&sys));
    assert_eq!(h.doc_text(), format!("ab{ESC_A0}c\nあ<0x0A>x"));
    assert_eq!(paste_notices(&h), vec!["info.paste.escape_as_text"]);
}

// 衝突の文書 (b) (不正バイトと真の私用文字の両方。区別できない) からのコピーは、衝突の文書 (a) へも `<U+10FFNN>` の
// 文字列にする (私用文字のまま入れると、元は 1 バイトだった 0x9A が保存で 4 バイトになる。レビューの再現 r3)
// verifies: NFR-304
#[test]
fn 衝突の文書_b_からのコピーは衝突の文書_a_へもコードポイントの表記で入れて知らせる() {
    let mut h = env();
    let mut b = b"x".to_vec();
    b.extend_from_slice("\u{10FF80}".as_bytes());
    b.extend_from_slice(b"\x9A\n");
    h.fs.add_file("/w/b.txt", &b);
    raw_open(&mut h, "/w/b.txt", Encoding::Utf8);
    assert_eq!(doc(&h).format.pua_escape, PuaEscapeMode::DisabledCollision);
    assert!(doc(&h).format.invalid_bytes > 0);
    let src = h.doc_text();
    h.keys("ctrl+a ctrl+c");
    assert_eq!(
        last_set_clipboard(&h),
        ClipboardText::Inline(src.clone()),
        "他のアプリへは私用文字のまま"
    );
    h.fs.add_file("/w/a.txt", "q\u{10FF00}\n".as_bytes());
    open_as(&mut h, "/w/a.txt", Encoding::Utf8);
    assert_eq!(doc(&h).format.pua_escape, PuaEscapeMode::DisabledCollision);
    h.keys("ctrl+end");
    h.clear_log();
    paste_from_system(&mut h, Some(&src));
    assert_eq!(h.doc_text(), "q\u{10FF00}\nx<U+10FF80><U+10FF9A>\n");
    assert_eq!(paste_notices(&h), vec!["info.paste.pua_as_text"]);
    h.cmd("file.save");
    h.run_jobs();
    let out = h.fs.read("/w/a.txt").unwrap();
    assert!(
        !out.windows(4).any(|w| w == "\u{10FF9A}".as_bytes()),
        "0x9A を U+10FF9A の UTF-8 として書かない: {out:02X?}"
    );
}

// Windows のクリップボードの取得は最初の NUL で切れる (05 §8.1)。NUL で切れた文字列が届いても内部のコピーとみなし、
// 内部レジスタの全体を入れる (NUL より後ろを黙って失わない。レビューの再現 r4)
// verifies: NFR-304
#[test]
fn nul_を含む範囲の切り取りと貼り付けは_nul_で切れて届いても全体が戻る() {
    let mut h = sjis_env();
    h.keys("ctrl+home");
    h.cmd_with("edit.insert_text", Args::new().with_str("text", "z\u{0}"));
    let before = h.doc_text();
    h.keys("ctrl+a ctrl+x");
    assert_eq!(h.doc_text(), "");
    h.clear_log();
    paste_from_system(&mut h, Some("z"));
    assert_eq!(h.doc_text(), before);
    assert_eq!(paste_notices(&h), vec!["info.paste.as_bytes"]);
}

// ---------- 書き出し・取得の失敗の後も、届いた文字列が入る (レビューの再現 r1 / r1b / r1c の回帰の試験) ----------

/// Host は取得の失敗を `HostReport::ClipboardFallback` で知らせてから `None` を届ける。その順を再現する。
fn ctrl_v_read_fails(h: &mut Harness, reason: ClipboardFallbackReason) {
    h.keys("ctrl+v");
    let token = h
        .effects()
        .iter()
        .rev()
        .find_map(|e| match e {
            Effect::RequestClipboard { token, .. } => Some(*token),
            _ => None,
        })
        .expect("RequestClipboard");
    h.editor.report(HostReport::ClipboardFallback { reason });
    h.clipboard_content(token, None);
}

fn plain_env() -> Harness {
    let mut h = env();
    h.fs.add_file("/w/u.txt", b"line1\n");
    open_as(&mut h, "/w/u.txt", Encoding::Utf8);
    h.keys("ctrl+home shift+end ctrl+c ctrl+end");
    h
}

// SSH (取得は常に `Unsupported`) などでは、エディタ内の `edit.paste` は内部レジスタを入れ、端末からの貼り付けは
// 届いた文字列を入れる (05 §8.1 が案内している経路)
// verifies: NFR-304
#[test]
fn 取得できない環境でも端末からの貼り付けは届いた文字列が入る() {
    for reason in [
        ClipboardFallbackReason::Ssh,
        ClipboardFallbackReason::NoCommand,
        ClipboardFallbackReason::NoDisplay,
        ClipboardFallbackReason::TooLarge,
    ] {
        let mut h = plain_env();
        ctrl_v_read_fails(&mut h, reason);
        assert_eq!(h.doc_text(), "line1\nline1", "{reason:?}");
        h.paste(" https://example.com/");
        assert_eq!(
            h.doc_text(),
            "line1\nline1 https://example.com/",
            "{reason:?}"
        );
    }
}

// verifies: NFR-304
#[test]
fn 取得の上限超過の後に別のアプリでコピーした文字列は届いたとおりに入る() {
    let mut h = plain_env();
    ctrl_v_read_fails(&mut h, ClipboardFallbackReason::TooLarge);
    paste_from_system(&mut h, Some("NEW"));
    assert!(h.doc_text().ends_with("NEW"), "{:?}", h.doc_text());
}

// 書き出しの失敗は知らせるだけで、照合の規則は変えない (03 §8.3.1。前のコピーの失敗が遅れて届いても、次のコピーの
// 貼り付けを内部レジスタにすり替えない)
// verifies: NFR-304
#[test]
fn 書き出しの失敗の後も_届いた文字列が内部レジスタと違えば届いた文字列が入る() {
    for reason in [
        ClipboardFallbackReason::WriteFailed,
        ClipboardFallbackReason::TooLarge,
    ] {
        let mut h = env();
        h.fs.add_file("/w/u.txt", b"AAA\nBBB\n");
        open_as(&mut h, "/w/u.txt", Encoding::Utf8);
        h.keys("ctrl+home shift+end ctrl+c");
        h.keys("down home shift+end ctrl+c");
        h.editor.report(HostReport::ClipboardFallback { reason });
        h.keys("ctrl+end");
        paste_from_system(&mut h, Some("EXT"));
        assert!(
            h.messages().contains(&"msg.clipboard.fallback".to_owned()),
            "書き出しの失敗は知らせる"
        );
        assert!(
            h.doc_text().ends_with("EXT"),
            "{reason:?}: {:?}",
            h.doc_text()
        );
        h.paste("TERM");
        assert!(
            h.doc_text().ends_with("EXTTERM"),
            "{reason:?}: {:?}",
            h.doc_text()
        );
    }
}

// verifies: NFR-304
#[test]
fn 読めなかった貼り付けは内部レジスタを使う() {
    let mut h = sjis_env();
    copy_escape(&mut h);
    h.keys("end");
    h.clear_log();
    paste_from_system(&mut h, None);
    assert_eq!(h.doc_text(), format!("ab{ESC_A0}c{ESC_A0}\n"));
    assert_eq!(paste_notices(&h), vec!["info.paste.as_bytes"]);
}

// 64 KiB 以上の内部レジスタ (Handle で渡す) からも、経路 (1) でバイトのまま入る
// verifies: NFR-304
#[test]
fn 大きい内部レジスタからもバイトのまま貼り付ける() {
    let mut h = env();
    let mut bytes = b"x\x80y\n".to_vec();
    bytes.extend(std::iter::repeat_n(b'a', 70 * 1024));
    bytes.push(b'\n');
    h.fs.add_file("/w/big.txt", &bytes);
    open_as(&mut h, "/w/big.txt", Encoding::Utf8);
    h.keys("ctrl+a ctrl+c");
    let ClipboardText::Handle(id) = last_set_clipboard(&h) else {
        panic!("ハンドルで渡す");
    };
    let sys = h.editor.take_clipboard(id).expect("取り出せる").to_text();
    h.keys("ctrl+end");
    h.clear_log();
    paste_from_system(&mut h, Some(&sys));
    let text = h.doc_text();
    assert_eq!(text.matches('\u{10FF80}').count(), 2);
    assert!(!text.contains("<0x80>"));
    assert_eq!(paste_notices(&h), vec!["info.paste.as_bytes"]);
    h.cmd("file.save");
    h.run_jobs();
    let mut want = bytes.clone();
    want.extend_from_slice(&bytes);
    assert_eq!(h.fs.read("/w/big.txt").unwrap(), want, "バイトのまま");
}

// verifies: NFR-304
#[test]
fn crlf_の文書からのコピーは改行が_lf_で届いても一致する() {
    let mut h = env();
    h.fs.add_file("/w/crlf.txt", b"a\xA0\r\nb\r\n");
    open_as(&mut h, "/w/crlf.txt", Encoding::ShiftJis);
    h.keys("ctrl+a ctrl+c");
    assert_eq!(
        last_set_clipboard(&h),
        ClipboardText::Inline("a<0xA0>\r\nb\r\n".to_owned()),
        "改行は変えずに渡す"
    );
    h.keys("ctrl+end");
    h.clear_log();
    // 端末の貼り付けは改行が CR / LF になることが多い
    paste_from_system(&mut h, Some("a<0xA0>\nb\r"));
    assert_eq!(h.doc_text(), "a\u{10FFA0}\r\nb\r\na\u{10FFA0}\r\nb\r\n");
    assert_eq!(paste_notices(&h), vec!["info.paste.as_bytes"]);
}

// ---------- 外から来た私用文字の各入口 ----------

/// 不正バイトを含む `Shift_JIS` の文書と、不正バイトの無い UTF-8 の文書と、衝突の文書 (a)。
fn targets() -> Vec<(&'static str, &'static [u8], Encoding, bool)> {
    vec![
        ("/w/sjis.txt", b"ab\xA0c\n", Encoding::ShiftJis, true),
        ("/w/u.txt", b"plain\n", Encoding::Utf8, true),
        ("/w/j.txt", b"plain\n", Encoding::Iso2022Jp, true),
        (
            "/w/c.txt",
            "x\u{10FF00}\n".as_bytes(),
            Encoding::Utf8,
            false,
        ),
    ]
}

fn target_env(path: &str, bytes: &[u8], enc: Encoding) -> Harness {
    let mut h = env();
    h.fs.add_file(path, bytes);
    open_as(&mut h, path, enc);
    h.keys("ctrl+end");
    h.clear_log();
    h
}

/// 入れた後の本文の末尾。
fn tail(h: &Harness, bytes_text: usize) -> String {
    let t = h.doc_text();
    t.chars().skip(bytes_text).collect()
}

// verifies: NFR-304
#[test]
fn 外から貼った私用文字はコードポイントの表記にして知らせる() {
    for (path, bytes, enc, notation) in targets() {
        // OS のクリップボード (内部レジスタは空)
        let mut h = target_env(path, bytes, enc);
        let n = h.doc_text().chars().count();
        paste_from_system(&mut h, Some("p\u{10FF80}q"));
        let want = if notation {
            "p<U+10FF80>q"
        } else {
            "p\u{10FF80}q"
        };
        assert_eq!(tail(&h, n), want, "{path}");
        let notices = if notation {
            vec!["info.paste.pua_as_text"]
        } else {
            vec![]
        };
        assert_eq!(paste_notices(&h), notices, "{path}");
        if notation {
            assert_eq!(
                message(&mut h),
                "私用文字 (U+10FF00〜U+10FFFF) 1 個は、そのままでは入れられないため <U+XXXX> の文字列にしました"
            );
        }
        // 端末からの貼り付け
        let mut h = target_env(path, bytes, enc);
        h.paste("\u{10FF80}");
        let want = if notation { "<U+10FF80>" } else { "\u{10FF80}" };
        assert_eq!(tail(&h, n), want, "{path}");
        assert_eq!(paste_notices(&h), notices, "{path}");
    }
}

// verifies: NFR-304
#[test]
fn 打った私用文字と_edit_insert_text_の私用文字はコードポイントの表記にして知らせる() {
    for (path, bytes, enc, notation) in targets() {
        let mut h = target_env(path, bytes, enc);
        let n = h.doc_text().chars().count();
        h.text("\u{10FF80}");
        let want = if notation { "<U+10FF80>" } else { "\u{10FF80}" };
        assert_eq!(tail(&h, n), want, "{path}");
        let notices = if notation {
            vec!["info.paste.pua_as_text"]
        } else {
            vec![]
        };
        assert_eq!(paste_notices(&h), notices, "{path}");
        let mut h = target_env(path, bytes, enc);
        h.cmd_with(
            "edit.insert_text",
            Args::new().with_str("text", "a\u{10FFA0}\u{10FF01}"),
        );
        let want = if notation {
            "a<U+10FFA0><U+10FF01>"
        } else {
            "a\u{10FFA0}\u{10FF01}"
        };
        assert_eq!(tail(&h, n), want, "{path}");
        if notation {
            assert!(message(&mut h).contains(" 2 個は"), "{}", message(&mut h));
        }
    }
}

/// 検索語と置換文字列を入れた状態にする (検索欄 → Tab → 置換欄)。欄には打った文字のまま入る。
fn setup_replace(h: &mut Harness, find: &str, with: &str) {
    h.keys("ctrl+r");
    h.text(find);
    h.keys("tab");
    h.text(with);
}

/// 検索欄の文字列 (表示の Span をつないだもの) と書記素数。
fn search_field(h: &mut Harness) -> (String, usize) {
    let input = h.frame().search.expect("検索バー").input;
    (
        input.cells.iter().map(|c| c.text.as_str()).collect(),
        input.len,
    )
}

// verifies: NFR-304
#[test]
fn 置換の文字列の私用文字はコードポイントの表記にして知らせる() {
    for (path, bytes, enc, notation) in targets() {
        // 1 件の置換
        let mut h = target_env(path, bytes, enc);
        let before = h.doc_text();
        h.text("@@");
        setup_replace(&mut h, "@", "\u{10FF80}");
        h.cmd("search.replace");
        let want = if notation {
            "<U+10FF80>@"
        } else {
            "\u{10FF80}@"
        };
        assert_eq!(h.doc_text(), format!("{before}{want}"), "{path}");
        let notices = if notation {
            vec!["info.paste.pua_as_text"]
        } else {
            vec![]
        };
        assert_eq!(paste_notices(&h), notices, "{path}");
        // すべて置換: 件数は置換した数の分
        let mut h = target_env(path, bytes, enc);
        let before = h.doc_text();
        h.text("@@");
        setup_replace(&mut h, "@", "\u{10FF80}");
        h.keys("alt+a");
        let want = if notation {
            "<U+10FF80><U+10FF80>"
        } else {
            "\u{10FF80}\u{10FF80}"
        };
        assert_eq!(h.doc_text(), format!("{before}{want}"), "{path}");
        assert_eq!(paste_notices(&h), notices, "{path}");
        if notation {
            assert!(message(&mut h).contains(" 2 個は"), "{}", message(&mut h));
        }
    }
}

// verifies: NFR-304
#[test]
fn 正規表現の置換の参照が運ぶエスケープ文字はバイトのまま_文字どおりの私用文字は表記にする() {
    let mut h = target_env("/w/sjis.txt", b"ab\xA0c\n", Encoding::ShiftJis);
    // 検索欄は正規表現の `\x{10FFA0}` でエスケープ文字に一致する
    setup_replace(&mut h, "(\\x{10FFA0})", "[$1\u{10FF80}]");
    h.keys("alt+r");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), format!("ab[{ESC_A0}<U+10FF80>]c\n"));
    assert_eq!(paste_notices(&h), vec!["info.paste.pua_as_text"]);
    h.keys("esc");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(
        h.fs.read("/w/sjis.txt").unwrap(),
        b"ab[\xA0<U+10FF80>]c\n",
        "参照が運んだエスケープ文字は元のバイトに戻る"
    );
}

// ---------- 入力欄 ----------

// verifies: NFR-304
#[test]
fn 検索欄には内部レジスタ由来の表記を文字列のまま入れて知らせる() {
    let mut h = sjis_env();
    copy_escape(&mut h);
    h.keys("ctrl+f");
    h.clear_log();
    paste_from_system(&mut h, Some("<0xA0>"));
    assert_eq!(search_field(&mut h), ("<0xA0>".to_owned(), 6));
    assert_eq!(paste_notices(&h), vec!["info.paste.search_notation"]);
    assert_eq!(
        message(&mut h),
        "<0xNN> の文字列は不正なバイトに一致しません。正規表現の \\x{10FFNN} で探せます"
    );
    assert_eq!(h.doc_text(), format!("ab{ESC_A0}c\n"), "本文は変わらない");
    // 端末からの貼り付けも同じ
    h.keys("ctrl+a");
    h.clear_log();
    h.paste("<0xA0>");
    assert_eq!(search_field(&mut h), ("<0xA0>".to_owned(), 6));
    assert_eq!(paste_notices(&h), vec!["info.paste.search_notation"]);
}

// verifies: NFR-304
#[test]
fn 検索欄への別の文字列や私用文字は知らせずそのまま入れる() {
    let mut h = sjis_env();
    copy_escape(&mut h);
    h.keys("ctrl+f");
    h.clear_log();
    paste_from_system(&mut h, Some("z<0xA0>\u{10FF80}"));
    assert_eq!(
        search_field(&mut h).1,
        8,
        "私用文字は 1 文字のまま (<U+10FF80> にしない)"
    );
    assert_eq!(paste_notices(&h), Vec::<String>::new());
}

// verifies: NFR-304
#[test]
fn プロンプトの欄には文字列のまま入れて知らせない() {
    let mut h = sjis_env();
    copy_escape(&mut h);
    h.keys("ctrl+g");
    assert_eq!(h.overlay(), "prompt");
    h.clear_log();
    paste_from_system(&mut h, Some("<0xA0>"));
    assert_eq!(paste_notices(&h), Vec::<String>::new());
    assert_eq!(h.doc_text(), format!("ab{ESC_A0}c\n"));
}

// ---------- 文書の中の操作 ----------

// verifies: NFR-304
#[test]
fn 行の複製と移動はエスケープ文字をそのまま運ぶ() {
    let mut h = env();
    h.fs.add_file("/w/sjis.txt", b"x\nab\xA0c\n");
    open_as(&mut h, "/w/sjis.txt", Encoding::ShiftJis);
    h.keys("ctrl+home down");
    h.cmd("edit.duplicate_line");
    h.cmd("edit.move_line_up");
    let text = h.doc_text();
    assert_eq!(text.matches(ESC_A0).count(), 2, "{text:?}");
    assert_eq!(paste_notices(&h), Vec::<String>::new());
    h.cmd("file.save");
    h.run_jobs();
    let want: Vec<u8> = text
        .chars()
        .map(|c| {
            if c == ESC_A0 {
                0xA0
            } else {
                u8::try_from(c).expect("ASCII")
            }
        })
        .collect();
    assert_eq!(h.fs.read("/w/sjis.txt").unwrap(), want, "バイトのまま移る");
}

// ---------- 性質テスト: 挿入の入口のどれでも同じ結果になる (03 §22.1 の「貼り付けのエスケープ文字」) ----------

mod entrances {
    use super::*;
    use proptest::prelude::*;

    fn piece() -> impl Strategy<Value = &'static str> {
        prop::sample::select(vec![
            "a",
            "あ",
            "<0x9A>",
            " ",
            "$1",
            "\u{10FF9A}",
            "\u{10FF00}",
            "\u{10FFFF}",
            "\u{10FEFF}",
        ])
    }

    /// 入口ごとに、外からの文字列 `s` を入れた後の本文の末尾と `info.paste.*` の知らせ。
    fn via(entrance: usize, target: usize, s: &str) -> (String, Vec<String>) {
        let (path, bytes, enc, _) = targets()[target];
        let mut h = target_env(path, bytes, enc);
        let n = h.doc_text().chars().count();
        match entrance {
            0 => paste_from_system(&mut h, Some(s)),
            1 => h.paste(s),
            2 => {
                h.cmd_with("edit.insert_text", Args::new().with_str("text", s));
            }
            _ => {
                h.text("@");
                h.clear_log();
                setup_replace(&mut h, "@", s);
                h.cmd("search.replace");
            }
        }
        (tail(&h, n), paste_notices(&h))
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(64),
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        #[test]
        fn 外からの文字列は入口に依らず同じ本文と知らせになる(
            v in prop::collection::vec(piece(), 1..6),
            target in 0usize..4,
        ) {
            let s = v.concat();
            let notation = targets()[target].3;
            let want = if notation {
                s.chars()
                    .map(|c| {
                        if ('\u{10FF00}'..='\u{10FFFF}').contains(&c) {
                            format!("<U+{:X}>", c as u32)
                        } else {
                            c.to_string()
                        }
                    })
                    .collect()
            } else {
                s.clone()
            };
            let pua = s.chars().filter(|c| ('\u{10FF00}'..='\u{10FFFF}').contains(c)).count();
            let notices: Vec<String> = if notation && pua > 0 {
                vec!["info.paste.pua_as_text".to_owned()]
            } else {
                vec![]
            };
            for entrance in 0..4 {
                let (got, got_notices) = via(entrance, target, &s);
                prop_assert_eq!(&got, &want, "入口 {}", entrance);
                prop_assert_eq!(&got_notices, &notices, "入口 {}", entrance);
            }
        }
    }
}
