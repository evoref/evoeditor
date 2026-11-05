//! 文字コードごとの読み書きのシナリオ (docs/03 §8、01 の FR-108 / 109 / 111 / 112 / 115、NFR-303 / 304)。
//! 保存先は `MemFs`、ジョブは `ManualJobSpawner`。

use eve_core::{Args, CommandResult, Document, Encoding, FsPath, OpenOptions, ReadOnlyReason};
use eve_testkit::Harness;

/// 起動していない Harness (文書は `open` / `open_as` で開く)。
fn env() -> Harness {
    Harness::builder().defer_start().build()
}

fn run(h: &mut Harness, id: &str, args: Args) -> CommandResult {
    let r = h.cmd_with(id, args);
    h.run_jobs();
    r
}

fn open(h: &mut Harness, path: &str) {
    run(h, "file.open", Args::new().with_str("path", path));
}

/// 不正バイトが多いファイルの確認(03 §8.3)が出ていれば「読み取り専用で開く」を選ぶ。
fn accept_read_only(h: &mut Harness) {
    if h.overlay() == "dialog" {
        h.text("r");
        h.run_jobs();
    }
}

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
}

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

fn save(h: &mut Harness) {
    run(h, "file.save", Args::new());
}

/// これまでに出たメッセージの MessageKey (読むと捨てる)。
fn messages(h: &mut Harness) -> Vec<String> {
    let m = h.messages();
    h.clear_log();
    m
}

fn iso2022(s: &str) -> Vec<u8> {
    let (b, _, _) = encoding_rs::ISO_2022_JP.encode(s);
    b.into_owned()
}

fn sjis(s: &str) -> Vec<u8> {
    encoding_rs::SHIFT_JIS.encode(s).0.into_owned()
}

const JP: &str = "日本語のテキストエディタです。これは文章の見本で、文字コードの判定に使います。\n";

// verifies: AC-FR-109-1, AC-FR-109-2
#[test]
fn shift_jis_を判定して開き_編集して同じ文字コードで保存する() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", &sjis(JP));
    open(&mut s, "/w/a.txt");
    assert_eq!(s.doc_text(), JP);
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
    assert!(doc(&s).is_editable());
    s.text("追記");
    save(&mut s);
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), sjis(&format!("追記{JP}")));
}

// verifies: AC-FR-109-1
#[test]
fn euc_jp_を開いて保存する() {
    let mut s = env();
    let bytes = encoding_rs::EUC_JP.encode(JP).0.into_owned();
    s.fs.add_file("/w/a.txt", &bytes);
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).format.encoding, Encoding::EucJp);
    assert_eq!(s.doc_text(), JP);
    s.text("x");
    save(&mut s);
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        encoding_rs::EUC_JP.encode(&format!("x{JP}")).0.into_owned()
    );
}

// verifies: AC-FR-108-1
#[test]
fn utf16_の_bom_を保って保存する() {
    let mut s = env();
    let mut bytes = b"\xFF\xFE".to_vec();
    bytes.extend("あa😀\n".encode_utf16().flat_map(u16::to_le_bytes));
    s.fs.add_file("/w/a.txt", &bytes);
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).format.encoding, Encoding::Utf16Le);
    assert!(doc(&s).format.has_bom);
    assert_eq!(s.doc_text(), "あa😀\n");
    s.text("b");
    save(&mut s);
    let mut want = b"\xFF\xFE".to_vec();
    want.extend("bあa😀\n".encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), want);
}

// verifies: AC-FR-109-1, AC-FR-109-3
#[test]
fn iso_2022_jp_を判定して行ごとに閉じて保存する() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc\x1B$B$\"$$\x1B(B def\nx\n");
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).format.encoding, Encoding::Iso2022Jp);
    assert_eq!(s.doc_text(), "abcあい def\nx\n");
    s.text("日");
    save(&mut s);
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"\x1B$BF|\x1B(Babc\x1B$B$\"$$\x1B(B def\nx\n".to_vec()
    );
}

#[test]
fn 文字コードを指定して開く() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", &sjis("日本語"));
    // UTF-8 と指定すると不正バイトを含む (読み取り専用)
    open_as(&mut s, "/w/a.txt", Encoding::Utf8);
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
    assert!(doc(&s).format.invalid_bytes > 0);
    assert!(doc(&s).read_only.contains(ReadOnlyReason::InvalidBytes));
}

// verifies: AC-NFR-304-1
#[test]
fn 不正バイトを含む_shift_jis_は読み取り専用で_保存しなければバイトは変わらない() {
    let mut s = env();
    let bytes = [sjis("日本語テキスト"), b"\xA0\xFD".to_vec(), sjis("おわり")].concat();
    s.fs.add_file("/w/a.txt", &bytes);
    open_as(&mut s, "/w/a.txt", Encoding::ShiftJis); // 判定は chardetng 次第なので指定して開く
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
    assert_eq!(doc(&s).format.invalid_bytes, 2);
    assert!(!doc(&s).is_editable());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes);
}

// verifies: AC-NFR-304-3
#[test]
fn 不正バイトを含む_iso_2022_jp_は専用の理由で読み取り専用_保存を拒否する() {
    let mut s = env();
    // JIS X 0208 状態で改行を含む (03 §8.3): 7 bit なので自動判定は ASCII になる → 指定して開く
    s.fs.add_file("/w/a.txt", b"\x1B$B$\"\n\x1B(Bx");
    open_as(&mut s, "/w/a.txt", Encoding::Iso2022Jp);
    assert_eq!(doc(&s).format.encoding, Encoding::Iso2022Jp);
    assert!(doc(&s).format.invalid_bytes > 0);
    assert!(
        doc(&s)
            .read_only
            .contains(ReadOnlyReason::StatefulEncodingWithInvalidBytes)
    );
    messages(&mut s);
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/b.txt"),
    );
    assert!(messages(&mut s).contains(&"msg.save.encoding_unsupported_iso2022".to_owned()));
    assert!(!s.fs.exists("/w/b.txt"), "何も書かない");
}

#[test]
fn 先頭が_ascii_で後半に_shift_jis_が現れたら判定し直して読み直す() {
    let mut s = env();
    let mut bytes = b"plain ascii line\n".repeat(5000); // 85 KB の ASCII
    bytes.extend(sjis(&JP.repeat(20)));
    s.fs.add_file("/w/a.txt", &bytes);
    messages(&mut s);
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
    assert!(s.doc_text().ends_with(JP));
    assert!(doc(&s).is_editable());
    assert!(messages(&mut s).contains(&"info.open.redetected".to_owned()));
}

#[test]
fn 先頭が_ascii_で後半に_iso_2022_jp_が現れたら判定し直す() {
    let mut s = env();
    let mut bytes = b"plain ascii line\n".repeat(5000);
    bytes.extend(b"\x1B$B$\"$$\x1B(B\n");
    s.fs.add_file("/w/a.txt", &bytes);
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).format.encoding, Encoding::Iso2022Jp);
    assert!(s.doc_text().ends_with("あい\n"));
}

#[test]
fn 対応外の文字コードの推定は既定で開いて案内する() {
    let mut s = env();
    s.fs.add_file(
        "/w/a.txt",
        b"caf\xE9 au lait, na\xEFve r\xE9sum\xE9 and more text here for the detector.\n",
    );
    messages(&mut s);
    open(&mut s, "/w/a.txt");
    accept_read_only(&mut s);
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
    assert!(messages(&mut s).contains(&"info.open.guess_unsupported".to_owned()));
}

// verifies: AC-FR-112-1
#[test]
fn 表現できない文字は何も書かずに保存を止める() {
    let mut s = env();
    let original = sjis(JP);
    s.fs.add_file("/w/a.txt", &original);
    open(&mut s, "/w/a.txt");
    s.text("😀"); // Shift_JIS では表せない
    messages(&mut s);
    save(&mut s);
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        original,
        "元のファイルは変わらない"
    );
    assert!(doc(&s).is_dirty(), "保存していない");
    assert_eq!(s.overlay(), "dialog", "E3 の選択肢が出る");
}

#[test]
fn 黙示の置換も表現不能として止める() {
    let mut s = env();
    let original = sjis(JP);
    s.fs.add_file("/w/a.txt", &original);
    open(&mut s, "/w/a.txt");
    s.text("¥"); // encoding_rs は 0x5C に黙って写す
    save(&mut s);
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), original);
    assert!(doc(&s).is_dirty());
}

// verifies: AC-NFR-303-3
#[test]
fn 重複符号化の行を読込時に数えて通知する() {
    let mut s = env();
    let bytes = [sjis(JP), b"\xFA\x54\n".to_vec()].concat();
    s.fs.add_file("/w/a.txt", &bytes);
    messages(&mut s);
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).format.roundtrip.lossy_lines, 1);
    assert!(messages(&mut s).contains(&"info.open.lossy_roundtrip".to_owned()));
}

#[test]
fn 無編集の_shift_jis_は書き込まない() {
    let mut s = env();
    let bytes = [sjis(JP), b"\xFA\x54\n".to_vec()].concat();
    s.fs.add_file("/w/a.txt", &bytes);
    open(&mut s, "/w/a.txt");
    save(&mut s);
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        bytes,
        "区分 B も無編集ならバイト一致 (03 §9.3 手順 0)"
    );
}

#[test]
fn ステータスバーの文字コードは名前と_bom_を出す() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", &sjis(JP));
    open(&mut s, "/w/a.txt");
    let f = s.frame();
    let enc = f
        .status
        .items
        .iter()
        .find(|i| i.kind == eve_view::StatusKind::Encoding)
        .unwrap();
    let text: String = enc.long.spans.iter().map(|sp| sp.text.as_str()).collect();
    assert_eq!(text, "Shift_JIS");
    let short: String = enc
        .short
        .as_ref()
        .unwrap()
        .spans
        .iter()
        .map(|sp| sp.text.as_str())
        .collect();
    assert_eq!(short, "SJIS");
    let _ = FsPath::display;
}

/// E3: 表現不能の文字は、確認が出ている間とキャンセルの後も強調し、次の編集で消える (03 §8.6)。
#[test]
fn 表現不能の文字を強調し_次の編集まで残す() {
    use eve_view::StyleMods;
    let mut s = env();
    s.fs.add_file("/w/a.txt", &sjis(JP));
    open(&mut s, "/w/a.txt");
    s.text("😀");
    save(&mut s);
    assert_eq!(s.overlay(), "dialog", "E3 の選択肢が出る");
    let marked = |s: &mut Harness| -> String {
        let f = s.frame();
        f.panes[0]
            .rows
            .iter()
            .flat_map(|r| r.spans.iter())
            .filter(|sp| {
                sp.style
                    .mods
                    .contains(StyleMods::REVERSE | StyleMods::UNDERLINE)
            })
            .map(|sp| sp.text.to_string())
            .collect()
    };
    assert_eq!(marked(&mut s), "😀", "確認が出ている間");
    s.keys("esc");
    assert_eq!(s.overlay(), "");
    assert_eq!(marked(&mut s), "😀", "キャンセルの後も残る");
    s.text("y");
    assert_eq!(marked(&mut s), "", "次の編集で消える");
}

/// 編集した区分 B の文書を元の文字コードで保存する前に、書き込まず確認する (03 §8.6 E2'、FR-112 AC-4 (a))。
fn lossy_doc() -> (Harness, Vec<u8>) {
    let mut s = env();
    let bytes = [sjis(JP), b"\xFA\x54\n".to_vec()].concat();
    s.fs.add_file("/w/a.txt", &bytes);
    open(&mut s, "/w/a.txt");
    s.text("x");
    (s, bytes)
}

// verifies: AC-FR-112-4, AC-NFR-303-3
#[test]
fn 区分_b_の文書を編集して保存するときは書き込む前に確認し_保存を選ぶと代表バイトで書く() {
    let (mut s, bytes) = lossy_doc();
    save(&mut s);
    assert_eq!(s.overlay(), "dialog", "確認が出る");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes, "まだ書いていない");
    // 保存 (S)
    s.text("s");
    s.run_jobs();
    let saved = s.fs.read("/w/a.txt").unwrap();
    assert_ne!(saved, bytes);
    assert!(saved.starts_with(b"x"));
    assert!(!doc(&s).is_dirty());
}

// verifies: AC-FR-112-4
#[test]
fn 区分_b_の確認で別名で保存を選ぶと名前の入力へ_esc_では何もしない() {
    let (mut s, bytes) = lossy_doc();
    save(&mut s);
    s.text("a");
    s.run_jobs();
    assert_eq!(s.overlay(), "prompt");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes);
    let (mut s, bytes) = lossy_doc();
    save(&mut s);
    s.keys("esc");
    assert_eq!(s.overlay(), "");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes);
    assert!(doc(&s).is_dirty());
}

// verifies: AC-NFR-303-3
#[test]
fn 区分_b_でも_utf8_で保存し直すなら確認しない() {
    let (mut s, _) = lossy_doc();
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    s.run_jobs();
    assert_ne!(s.overlay(), "dialog");
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
}

// verifies: AC-NFR-303-4
#[test]
fn iso_2022_jp_のエスケープが変わる文書は編集して保存するとき確認する() {
    let mut s = env();
    // JIS X 0208 を `ESC $ @` (旧い書き方) で書いた行 + 通常の行。再エンコードは `ESC $ B` に揃う
    let mut bytes = b"\x1b$@\x46\x7c\x4b\x5c\x1b(B\n".to_vec();
    bytes.extend_from_slice(&iso2022(JP));
    s.fs.add_file("/w/a.txt", &bytes);
    open(&mut s, "/w/a.txt");
    assert!(
        doc(&s).format.roundtrip.lossy_lines > 0,
        "区分 D の差を数える"
    );
    s.text("x");
    save(&mut s);
    assert_eq!(s.overlay(), "dialog");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes, "まだ書いていない");
    s.text("s");
    s.run_jobs();
    assert!(!doc(&s).is_dirty());
}

// verifies: AC-FR-112-4
#[test]
fn euc_jp_の補助漢字を含む文書は編集して保存すると何も書かず表現不能の選択肢を出す() {
    let mut s = env();
    // JIS X 0212 の 0x8F 0xB0 0xA1 (丂 U+4E02) は復号できるが、エンコーダは出力しない
    let mut bytes = encoding_rs::EUC_JP.encode(JP).0.into_owned();
    bytes.extend_from_slice(b"\x8f\xb0\xa1\n");
    s.fs.add_file("/w/a.txt", &bytes);
    open_as(&mut s, "/w/a.txt", Encoding::EucJp);
    s.text("x");
    save(&mut s);
    assert_eq!(s.overlay(), "dialog", "E3 の選択肢が出る");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes, "何も書かれない");
}

// ---------- 不正バイトのエスケープ文字の書き方 (03 §8.3、E4。docs/review/v0.3-findings.md の SAF-02 / 02b) ----------

/// 入力欄を空にしてから `text` を打ち、Enter で確定する (名前を付けて保存のプロンプト)。
fn submit_prompt(s: &mut Harness, text: &str) {
    for _ in 0..256 {
        s.keys("backspace");
    }
    s.text(text);
    s.keys("enter");
    s.run_jobs();
}

#[test]
fn 不正バイトを含む文書を別の文字コードで別名保存すると_元のバイトを書かずに表現不能として止める() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"a\x80bc\n");
    open(&mut s, "/w/a.txt");
    accept_read_only(&mut s);
    assert!(doc(&s).format.invalid_bytes > 0);
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-16le"),
    );
    assert_eq!(
        s.overlay(),
        "prompt",
        "読み取り専用の文書は名前を付けて保存の流れ"
    );
    submit_prompt(&mut s, "/w/u16.txt");
    // UTF-16LE の中に元の 1 バイト (0x80) を書くと、以降のすべての文字が 1 バイトずれる
    assert!(s.fs.read("/w/u16.txt").is_none(), "何も書かれない");
    assert_eq!(s.overlay(), "dialog", "表現不能の確認が出る");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"a\x80bc\n",
        "元のファイルは変わらない"
    );
}

#[test]
fn 不正バイトを含む文書を同じ文字コードで別名保存すると_元のバイトに戻して書く() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"a\x80bc\n");
    open(&mut s, "/w/a.txt");
    accept_read_only(&mut s);
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/copy.txt"),
    );
    s.run_jobs();
    assert_eq!(
        s.fs.read("/w/copy.txt").unwrap(),
        b"a\x80bc\n",
        "バイト一致 (NFR-303)"
    );
}

#[test]
fn 不正バイトの無い文書に貼ったエスケープ文字は_元のバイトに戻さず表現不能として止める() {
    let mut s = env();
    s.fs.add_file("/w/b.txt", b"xyz\n");
    open(&mut s, "/w/b.txt");
    assert_eq!(doc(&s).format.invalid_bytes, 0);
    // 他の文書からエスケープ文字 (U+10FF80 = 0x80 の印) を貼り付けた
    s.text("\u{10FF80}");
    save(&mut s);
    assert_eq!(s.overlay(), "dialog", "表現不能の確認が出る");
    assert_eq!(
        s.fs.read("/w/b.txt").unwrap(),
        b"xyz\n",
        "生のバイト 0x80 を書かない"
    );
}
