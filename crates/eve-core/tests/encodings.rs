//! 文字コードごとの読み書きのシナリオ (docs/03 §8、01 の FR-108 / 109 / 111 / 112 / 115、NFR-303 / 304)。
//! 保存先は `MemFs`、ジョブは `ManualJobSpawner`。

use eve_core::{
    Args, CommandResult, Document, Encoding, FsPath, OpenOptions, Origin, ReadOnlyReason, Tick,
    Transaction,
};
use eve_testkit::Harness;
use eve_text::{ByteOff, ByteRange, ChangeSet, Edit};

/// 起動していない Harness (文書は `open` / `open_as` で開く)。
fn env() -> Harness {
    Harness::builder().defer_start().build()
}

/// 文書の先頭に `text` をそのまま入れる (挿入の入口の規則 (03 §8.3.1) を通さない試験だけの近道)。貼り付け・入力では
/// 外から来た `U+10FF00..` が `<U+10FFNN>` になるので (v0.4 の M23)、エスケープ文字が不正バイトの無い文書の本文に残る
/// 状況 (違う文字コードで保存した後の Undo・文字コードの違う復旧など) を短く作るのに使う。
fn inject_start(h: &mut Harness, text: &str) {
    let id = h.doc_id();
    let st = h.editor.state_mut();
    let d = st.document(id).unwrap();
    let start = ByteOff(0);
    let cs = ChangeSet::from_sorted_edits(
        &d.snapshot(),
        [Edit {
            range: ByteRange::new(start, start),
            insert: text.into(),
        }],
    )
    .unwrap();
    st.apply_transaction(id, Transaction::new(cs, Origin::Paste, Tick(0)))
        .unwrap();
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

/// これまでに出たメッセージの `MessageKey` (読むと捨てる)。
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

// ---------- 連鎖の開始時の拒否 (03 §9.7、NFR-304 AC-3): 保存先を尋ねる前にメッセージを出す ----------

/// JIS X 0208 状態で改行を含む ISO-2022-JP (不正バイトになる。03 §8.3)。
const ISO_INVALID: &[u8] = b"\x1B$B$\"\n\x1B(Bx";
const ISO_REJECTED: &str = "msg.save.encoding_unsupported_iso2022";

/// 不正バイトを含む ISO-2022-JP の文書 (読み取り専用、理由 `StatefulEncodingWithInvalidBytes`) を開いた状態。
fn iso_invalid() -> Harness {
    let mut s = env();
    s.fs.add_file("/w/a.txt", ISO_INVALID);
    open_as(&mut s, "/w/a.txt", Encoding::Iso2022Jp);
    assert!(doc(&s).format.invalid_bytes > 0);
    assert!(
        doc(&s)
            .read_only
            .contains(ReadOnlyReason::StatefulEncodingWithInvalidBytes)
    );
    messages(&mut s);
    s
}

/// ディスク上の全ファイル (パスと中身)。
fn disk(s: &Harness) -> Vec<(String, Option<Vec<u8>>)> {
    s.fs.paths()
        .into_iter()
        .map(|p| {
            let b = s.fs.read(&p);
            (p, b)
        })
        .collect()
}

/// 拒否された: 名前の入力・確認を出さずに拒否のメッセージを出し、どのファイルも書いていない (元のファイル・保存先とも)。
fn assert_rejected(s: &mut Harness, before: &[(String, Option<Vec<u8>>)], what: &str) {
    assert_eq!(s.overlay(), "", "{what}: 名前の入力も確認も出さない");
    let m = messages(s);
    assert!(m.contains(&ISO_REJECTED.to_owned()), "{what}: {m:?}");
    assert_eq!(disk(s), before, "{what}: 何も書かない");
    assert_eq!(s.pending_jobs(), 0, "{what}: ジョブを投入しない");
}

/// 後続を伴う保存の入口を試すための、dirty で不正バイトを含む ISO-2022-JP の文書。`path` が無ければ無題、
/// `read_only` なら読み取り専用。不正バイトを含む ISO-2022-JP の文書は読込で解除できない読み取り専用になり編集できないので、
/// 正しい ISO-2022-JP の文書を編集してから不正バイトの数だけを直に足す (拒否の条件は書式だけで決まる、03 §9.7)。
fn dirty_iso_invalid(path: Option<&str>, read_only: bool) -> Harness {
    let mut s = match path {
        Some(p) => {
            let mut s = env();
            s.fs.add_file(p, &iso2022("あ\n"));
            open_as(&mut s, p, Encoding::Iso2022Jp);
            s
        }
        // 起動した Harness の無題の文書 (起動時の書込を済ませておく)
        None => {
            let mut s = Harness::new();
            s.run_jobs();
            s
        }
    };
    s.text("い");
    let id = s.doc_id();
    let d = s.editor.state_mut().document_mut(id).unwrap();
    d.format.encoding = Encoding::Iso2022Jp;
    d.format.invalid_bytes = 1;
    if read_only {
        d.read_only
            .insert(ReadOnlyReason::StatefulEncodingWithInvalidBytes);
    }
    assert!(s.is_dirty());
    messages(&mut s);
    s
}

// verifies: AC-NFR-304-3
#[test]
fn 不正バイトを含む_iso_2022_jp_の名前を付けて保存と文字コードの指定は名前を尋ねずに拒否する() {
    // 名前を付けて保存 (パスなし)
    let mut s = iso_invalid();
    let before = disk(&s);
    run(&mut s, "file.save_as", Args::new());
    assert_rejected(&mut s, &before, "file.save_as (パスなし)");

    // 名前を付けて保存 (今の文書と同じパス): 読み取り専用の同じファイルの拒否より先に拒否する
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/a.txt"),
    );
    assert_rejected(&mut s, &before, "file.save_as (同じパス)");

    // 文字コードを指定して保存 (読み取り専用の文書は名前を付けて保存の流れへ進むところ)
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "iso-2022-jp"),
    );
    assert_rejected(&mut s, &before, "file.set_encoding");

    // 文字コードを尋ねてから指定する経路も同じ
    run(&mut s, "file.set_encoding", Args::new());
    assert_eq!(s.overlay(), "prompt", "文字コードを尋ねる");
    s.text("iso-2022-jp");
    s.keys("enter");
    s.run_jobs();
    assert_rejected(&mut s, &before, "file.set_encoding (文字コードの入力)");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), ISO_INVALID);
}

// verifies: AC-NFR-304-3
#[test]
fn 不正バイトを含む_utf8_の文書を_iso_2022_jp_に指定して保存すると名前を尋ねずに拒否する() {
    // 不正バイトの割合が小さい (多いときの確認を出さない) UTF-8
    const BYTES: &[u8] = b"abcdefghijklmnopqrstuvwxyz\x80\n";
    let mut s = env();
    s.fs.add_file("/w/a.txt", BYTES);
    open_as(&mut s, "/w/a.txt", Encoding::Utf8);
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
    assert!(doc(&s).format.invalid_bytes > 0);
    assert!(!doc(&s).is_editable());
    messages(&mut s);
    let before = disk(&s);
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "iso-2022-jp"),
    );
    assert_rejected(&mut s, &before, "読み取り専用");

    // 「編集を許可」で解除した (編集できる) 文書でも同じ
    assert_eq!(s.cmd("file.toggle_read_only"), CommandResult::Done);
    assert_eq!(s.overlay(), "dialog", "編集を許可の確認");
    s.text("a");
    assert!(
        doc(&s).is_editable(),
        "{:?}",
        doc(&s).read_only.iter().collect::<Vec<_>>()
    );
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "iso-2022-jp"),
    );
    assert_rejected(&mut s, &before, "編集できる");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), BYTES);
}

// verifies: AC-NFR-304-3
#[test]
fn 不正バイトを含む_iso_2022_jp_の無題の文書は上書き保存で名前を尋ねずに拒否する() {
    let mut s = dirty_iso_invalid(None, false);
    let before = disk(&s);
    run(&mut s, "file.save", Args::new());
    assert_rejected(&mut s, &before, "file.save");
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "iso-2022-jp"),
    );
    assert_rejected(&mut s, &before, "file.set_encoding");
    assert!(s.is_dirty(), "文書は変えない");
    assert_eq!(doc(&s).format.encoding, Encoding::Iso2022Jp);
}

// 終了・閉じるの確認で「保存」を選んだとき、名前の入力 (無題・読み取り専用) や書込の前に拒否し、後続 (終了・閉じる) も行わない
// verifies: AC-NFR-304-3
#[test]
fn 未保存の確認の保存は名前を尋ねずに拒否し_後続を行わない() {
    for (path, read_only) in [
        (Some("/w/a.txt"), false),
        (Some("/w/a.txt"), true),
        (None, false),
    ] {
        let what = format!("{path:?} 読み取り専用={read_only}");
        // 終了
        let mut s = dirty_iso_invalid(path, read_only);
        let before = disk(&s);
        let id = s.doc_id();
        run(&mut s, "app.quit", Args::new());
        assert_eq!(s.overlay(), "dialog", "{what}: 未保存の確認");
        s.answer(0);
        s.run_jobs();
        assert_rejected(&mut s, &before, &format!("{what}: 終了"));
        assert_eq!(s.quit_code(), None, "{what}: 終了しない");
        assert!(s.is_dirty());

        // 保存して終了 (`app.save_and_quit` は確認を経ずに同じ「保存」へ進む)
        run(&mut s, "app.save_and_quit", Args::new());
        assert_rejected(&mut s, &before, &format!("{what}: 保存して終了"));
        assert_eq!(s.quit_code(), None, "{what}: 終了しない");

        // 閉じる
        run(&mut s, "buffer.close", Args::new());
        assert_eq!(s.overlay(), "dialog", "{what}: 未保存の確認");
        s.answer(0);
        s.run_jobs();
        assert_rejected(&mut s, &before, &format!("{what}: 閉じる"));
        assert!(s.editor.state().document(id).is_some(), "{what}: 閉じない");
        assert!(s.is_dirty());
    }
}

// 全保存は確認を出さないので、拒否する文書は保存せず、理由を添えて失敗の一覧に載せる (03 §9.8)
// verifies: AC-NFR-304-3
#[test]
fn 全保存は不正バイトを含む_iso_2022_jp_の文書を書かずに失敗の一覧に載せる() {
    use eve_testkit::helpers::cell_text;
    use eve_view::OverlayModel;
    let mut s = dirty_iso_invalid(Some("/w/a.txt"), false);
    let before = disk(&s);
    run(&mut s, "file.save_all", Args::new());
    assert_eq!(disk(&s), before, "何も書かない");
    assert!(s.is_dirty());
    assert_eq!(s.overlay(), "dialog", "失敗の一覧");
    let f = s.frame();
    let OverlayModel::Confirm { details, .. } = &f.overlays[0] else {
        panic!("確認が出ていない")
    };
    let details: Vec<String> = details.iter().flatten().map(cell_text).collect();
    assert_eq!(details.len(), 1, "{details:?}");
    assert!(
        details[0].starts_with("/w/a.txt: ") && details[0].contains("ISO-2022-JP"),
        "理由は拒否のメッセージ: {details:?}"
    );
}

// 回帰: 不正バイトの無い文書は、ISO-2022-JP でも今までどおり保存できる (無題なら名前を尋ねる)
#[test]
fn 不正バイトの無い文書は_iso_2022_jp_で保存できる() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", "あ\n".as_bytes());
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
    messages(&mut s);
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "iso-2022-jp"),
    );
    assert!(!messages(&mut s).contains(&ISO_REJECTED.to_owned()));
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), iso2022("あ\n"));
    assert_eq!(doc(&s).format.encoding, Encoding::Iso2022Jp);

    // 無題の ISO-2022-JP の文書の上書き保存は名前を尋ねる
    let mut s = Harness::new();
    s.text("い");
    let id = s.doc_id();
    s.editor
        .state_mut()
        .document_mut(id)
        .unwrap()
        .format
        .encoding = Encoding::Iso2022Jp;
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.overlay(), "prompt", "名前を尋ねる");
    s.text("/w/u.txt");
    s.keys("enter");
    s.run_jobs();
    assert_eq!(s.fs.read("/w/u.txt").unwrap(), iso2022("い"));
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

// 読込時の検査は区分 B の行を数える (v0.1)。v0.4 では、保持が効いている文書 (元のバイト列を覚えた) には読込の完了時の
// 通知を出さず、保持できなかった箇所がある文書 (ここでは ISO-2022-JP の、ローマ字状態のまま改行する行) に数を言って出す
// (03 §8.7.1。v0.3 までは区分 B の行があれば `info.open.lossy_roundtrip` を出していた)
// verifies: AC-NFR-303-3
#[test]
fn 重複符号化の行を読込時に数え_保持できなかった箇所がある文書だけ通知する() {
    let mut s = env();
    let bytes = [sjis(JP), b"\xFA\x54\n".to_vec()].concat();
    s.fs.add_file("/w/a.txt", &bytes);
    messages(&mut s);
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).format.roundtrip.lossy_lines, 1);
    let m = messages(&mut s);
    assert!(
        !m.contains(&"info.open.lossy_roundtrip".to_owned()),
        "{m:?}"
    );
    assert!(
        !m.iter().any(|k| k.starts_with("info.open.preserve")),
        "保持が効いている文書には出さない: {m:?}"
    );
    // 保持できなかった箇所がある文書
    s.fs.add_file("/w/j.txt", b"\x1B$@$\"\x1B(B\nabc\n\x1B(J\\\n");
    open_as(&mut s, "/w/j.txt", Encoding::Iso2022Jp);
    assert!(doc(&s).format.roundtrip.lossy_lines > 0);
    let m = messages(&mut s);
    assert!(
        m.contains(&"info.open.preserve_partial".to_owned()),
        "{m:?}"
    );
}

// 往復の検査を打ち切った文書 (4 MiB を超える行がある) は、打ち切りより後ろの区分 B / D を数えていない。
// 数えていない範囲に区分 B / D があり得るので、編集して保存するときは確認を出す (03 §8.7、Issue #495)。
// v0.4 で検査を打ち切るのは ISO-2022-JP だけ (Shift_JIS / EUC-JP は長い行を小分けにして調べる、03 §8.7.1)。
// verifies: AC-NFR-303-3
#[test]
fn 検査を打ち切った文書は編集して保存するとき確認する() {
    let mut s = env();
    // 1 行目が 4 MiB を超える ASCII、2 行目にエスケープの選び方の差 (旧 JIS の `ESC $ @`。再符号化すると `ESC $ B`)
    let mut bytes = vec![b'a'; 4 * 1024 * 1024 + 1];
    bytes.extend_from_slice(b"\n\x1B$@$\"\x1B(B\n");
    s.fs.add_file("/w/a.txt", &bytes);
    open_as(&mut s, "/w/a.txt", Encoding::Iso2022Jp);
    assert!(doc(&s).format.roundtrip.incomplete, "検査を打ち切った");
    s.text("x");
    save(&mut s);
    assert_eq!(s.overlay(), "dialog", "書く前に確認する");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes, "まだ書いていない");
    // 全保存は、確認が要る文書を保存しない (閉じると元の確認に戻る)
    s.keys("esc");
    run(&mut s, "file.save_all", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes, "全保存でも書かない");
    if s.overlay() == "dialog" {
        s.keys("esc");
    }
    // 確認で「保存」を選ぶと書く。承認した Revision では繰り返さない
    save(&mut s);
    assert_eq!(s.overlay(), "dialog");
    s.text("s");
    s.run_jobs();
    assert!(!doc(&s).is_dirty(), "承認すると保存する");
    assert_ne!(s.fs.read("/w/a.txt").unwrap(), bytes);
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

/// 区分 B の文書 (CP932 の重複符号化) を開いて、区分 B でない所 (先頭) を編集した状態。
fn lossy_doc() -> (Harness, Vec<u8>) {
    let mut s = env();
    let bytes = [sjis(JP), b"\xFA\x54\n".to_vec()].concat();
    s.fs.add_file("/w/a.txt", &bytes);
    open(&mut s, "/w/a.txt");
    s.text("x");
    (s, bytes)
}

/// 「保持なし」の区分 B の文書: 重複符号化 (IBM 拡張の ￢ 0xFA54) を含む `Shift_JIS` の文書を開き、スワップから復旧した
/// (元のバイト列を覚えていないので、元の文字コードで書く前に確認する。03 §8.6 E2'、§8.7.1)。復旧で編集した状態。
fn unpreserved_doc() -> (Harness, Vec<u8>) {
    let bytes = b"a\xFA\x54b\n\xFA\x54\xFA\x54c\n".to_vec();
    let mut s = Harness::new()
        .with_size(80, 12)
        .with_file("/w/a.txt", &bytes);
    s.fs.add_swap_entry(
        eve_core::SwapEntry {
            dir: FsPath::from("/s/swap/aaaa-4242.swp"),
            path: Some(FsPath::from("/w/a.txt")),
            untitled_name: None,
            revision: 7,
            len: 12,
            pid: 4242,
            owner: eve_core::SwapOwner::Stale,
            sensitive: false,
            modified: Some(eve_core::FileTime {
                secs: 4_000_000_000,
                nanos: 0,
            }),
            readable: true,
        },
        Some(eve_core::SwapBody {
            text: "a￢b\n復旧\n".to_owned(),
            encoding: Encoding::ShiftJis,
            has_bom: false,
            default_eol: eve_text::LineEnding::Lf,
            pua_escaped: false,
        }),
    );
    s.open("/w/a.txt");
    s.run_jobs();
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
    s.advance_ms(0);
    assert_eq!(s.overlay(), "dialog", "復旧の提案");
    s.answer(0);
    s.run_jobs();
    assert_eq!(
        doc(&s).format.roundtrip.preserve,
        eve_encoding::PreserveStatus::Unavailable(eve_encoding::NoPreserveReason::Recovered)
    );
    assert!(doc(&s).is_dirty());
    (s, bytes)
}

// v0.4 は「保持なし」の文書だけが対象 (03 §8.7.1)。保持が効いている文書は下の試験
// verifies: AC-FR-112-4, AC-NFR-303-3
#[test]
fn 保持なしの区分_b_の文書を編集して保存するときは書き込む前に確認し_保存を選ぶと代表バイトで書く()
{
    let (mut s, bytes) = unpreserved_doc();
    save(&mut s);
    assert_eq!(s.overlay(), "dialog", "確認が出る");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes, "まだ書いていない");
    // 保存 (S)
    s.text("s");
    s.run_jobs();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        [b"a\x81\xCAb\n".to_vec(), sjis("復旧\n")].concat(),
        "代表バイト列 (0x81CA) で書く"
    );
    assert!(!doc(&s).is_dirty());
}

// verifies: AC-FR-112-4
#[test]
fn 保持なしの文書の確認で別名で保存を選ぶと名前の入力へ_esc_では何もしない() {
    let (mut s, bytes) = unpreserved_doc();
    save(&mut s);
    s.text("a");
    s.run_jobs();
    assert_eq!(s.overlay(), "prompt");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes);
    let (mut s, bytes) = unpreserved_doc();
    save(&mut s);
    s.keys("esc");
    assert_eq!(s.overlay(), "");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes);
    assert!(doc(&s).is_dirty());
}

// v0.4: 保持が効いている区分 B の文書は、編集して元の文字コードで保存しても確認せず、編集していない範囲を元のバイト列で
// 書く (03 §8.6 E2 / E2'、§8.7.1。v0.3 までは確認を出し、承認すると全体を代表バイト列で書いていた)。01 §5.5 の NFR-303 AC-5
// verifies: NFR-303
#[test]
fn 区分_b_の文書を編集して保存しても確認せず_編集していない範囲は元のバイト列で書く() {
    let (mut s, bytes) = lossy_doc();
    save(&mut s);
    assert_eq!(s.overlay(), "", "確認は出ない");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        [b"x".to_vec(), bytes].concat(),
        "重複符号化 0xFA54 はそのまま"
    );
    assert!(!doc(&s).is_dirty());
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

// v0.4 は、保持できなかった箇所がある文書 (ここでは、ローマ字状態のまま改行する最後の行) だけが対象 (03 §8.7.1)。
// 確認の既定は「保存」(文字は変わらない)
// verifies: AC-NFR-303-4
#[test]
fn iso_2022_jp_の保持できなかった行がある文書は編集して保存するとき確認する() {
    let mut s = env();
    // JIS X 0208 を `ESC $ @` (旧い書き方) で書いた行 + 通常の行 + ローマ字状態のまま改行する行
    let mut bytes = b"\x1b$@\x46\x7c\x4b\x5c\x1b(B\n".to_vec();
    bytes.extend_from_slice(&iso2022(JP));
    bytes.extend_from_slice(b"\x1B(J\\\n");
    s.fs.add_file("/w/a.txt", &bytes);
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).format.encoding, Encoding::Iso2022Jp);
    assert!(
        doc(&s).format.roundtrip.lossy_lines > 0,
        "区分 D の差を数える"
    );
    s.text("x");
    save(&mut s);
    assert_eq!(s.overlay(), "dialog");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes, "まだ書いていない");
    s.keys("enter");
    s.run_jobs();
    assert!(!doc(&s).is_dirty());
}

// v0.4: エスケープの選び方が再エンコードと違う行 (区分 D) は、編集しなければ元のバイト列で書き、確認しない (03 §8.7.1。
// v0.3 までは編集して保存するときに確認を出し、正規形で書いていた)。編集した行は正規形。01 §5.5 の NFR-303 AC-6
// verifies: NFR-303
#[test]
fn iso_2022_jp_のエスケープが変わる行は編集しなければ元のバイト列で書き確認しない() {
    let mut s = env();
    // JIS X 0208 を `ESC $ @` (旧い書き方) で書いた行 + 通常の行。再エンコードは `ESC $ B` に揃う
    let first = b"\x1b$@\x46\x7c\x4b\x5c\x1b(B\n".to_vec();
    let rest = iso2022(JP);
    let bytes = [first.clone(), rest.clone()].concat();
    s.fs.add_file("/w/a.txt", &bytes);
    open(&mut s, "/w/a.txt");
    assert!(
        doc(&s).format.roundtrip.lossy_lines > 0,
        "区分 D の差を数える"
    );
    // 2 行目の行頭に入力する (1 行目は編集しない)
    s.keys("down");
    s.text("x");
    save(&mut s);
    assert_eq!(s.overlay(), "", "確認は出ない");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        [first.clone(), b"x".to_vec(), rest.clone()].concat()
    );
    // 1 行目を編集すると、その行は正規形 (`ESC $ B`)
    s.keys("ctrl+home");
    s.text("y");
    save(&mut s);
    assert_eq!(s.overlay(), "");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        [b"y\x1b$BF|K\\\x1b(B\n".to_vec(), b"x".to_vec(), rest].concat()
    );
    assert!(!doc(&s).is_dirty());
}

/// 補助漢字 (JIS X 0212) の行を含む EUC-JP の文書。
fn euc_0212_doc() -> (Harness, Vec<u8>) {
    let mut s = env();
    // JIS X 0212 の 0x8F 0xB0 0xA1 (丂 U+4E02) は復号できるが、エンコーダは出力しない
    let mut bytes = encoding_rs::EUC_JP.encode(JP).0.into_owned();
    bytes.extend_from_slice(b"\x8f\xb0\xa1\n");
    s.fs.add_file("/w/a.txt", &bytes);
    open_as(&mut s, "/w/a.txt", Encoding::EucJp);
    (s, bytes)
}

// v0.4: 編集で入れた補助漢字は、これまでどおり表現不能として何も書かずに E3 の選択肢を出す (FR-112 AC-4 (b))。編集して
// いない補助漢字は元のバイト列で書く (下の試験)
// verifies: AC-FR-112-4
#[test]
fn euc_jp_の文書に補助漢字を入力して保存すると何も書かず表現不能の選択肢を出す() {
    let (mut s, bytes) = euc_0212_doc();
    s.text("丂");
    save(&mut s);
    assert_eq!(s.overlay(), "dialog", "E3 の選択肢が出る");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes, "何も書かれない");
}

// v0.4: 編集していない補助漢字は元のバイト列で書く (確認も表現不能も出ない、03 §8.7.1)。01 §5.5 の NFR-303 AC-5
// verifies: NFR-303
#[test]
fn euc_jp_の補助漢字は編集していなければ元のバイト列で書く() {
    let (mut s, bytes) = euc_0212_doc();
    s.text("x");
    save(&mut s);
    assert_eq!(s.overlay(), "", "確認も表現不能の選択肢も出ない");
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        [b"x".to_vec(), bytes].concat()
    );
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
fn 不正バイトの無い文書の本文にあるエスケープ文字は_元のバイトに戻さず表現不能として止める() {
    let mut s = env();
    s.fs.add_file("/w/b.txt", b"xyz\n");
    open(&mut s, "/w/b.txt");
    assert_eq!(doc(&s).format.invalid_bytes, 0);
    // エスケープ文字 (U+10FF80 = 0x80 の印) が本文にある。v0.4 の M23 から、貼り付け・入力では入らない
    // (`<U+10FF80>` の文字列になる) ので、規則を通さずに入れる
    inject_start(&mut s, "\u{10FF80}");
    save(&mut s);
    assert_eq!(s.overlay(), "dialog", "表現不能の確認が出る");
    assert_eq!(
        s.fs.read("/w/b.txt").unwrap(),
        b"xyz\n",
        "生のバイト 0x80 を書かない"
    );
}

// ---------- E3 の選択肢の条件 (03 §8.6「E3 の選択肢」、OQ-207 (3)、NFR-304) ----------

/// 出ている確認の選択肢のアクセラレータ (表示順。E3 は UTF-8 で保存が `u`、置換して保存が `r`)。
fn choice_keys(s: &mut Harness) -> Vec<Option<char>> {
    s.frame()
        .overlays
        .iter()
        .find_map(|o| match o {
            eve_view::OverlayModel::Confirm { choices, .. } => {
                Some(choices.iter().map(|c| c.key).collect())
            }
            _ => None,
        })
        .expect("確認が出ている")
}

/// 空の UTF-8 の文書の本文に `text` を (挿入の入口の規則を通さずに) 入れ、`file.set_encoding` で `Shift_JIS` にして
/// 保存する。
fn inject_and_save_as_sjis(text: &str) -> Harness {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"\n");
    open_as(&mut s, "/w/a.txt", Encoding::Utf8);
    inject_start(&mut s, text);
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    s
}

// 記録する表現不能は先頭 100 件だけ。エスケープ文字の有無は全件で判定する (101 件目より後ろにあっても選択肢 1・2 を
// 出さない。出すと「置換して保存」がエスケープ文字を `?` にして、不正バイトが黙って失われる)
// verifies: NFR-304
#[test]
fn 表現不能の_101_件目より後ろにエスケープ文字があっても_選択肢はキャンセルだけ() {
    let mut s = inject_and_save_as_sjis(&format!("{}\u{10FF80}", "😀".repeat(101)));
    assert_eq!(s.overlay(), "dialog", "E3 の選択肢が出る");
    let keys = choice_keys(&mut s);
    assert_eq!(keys.len(), 1, "キャンセルだけ: {keys:?}");
    assert!(!keys.contains(&Some('u')) && !keys.contains(&Some('r')));
    s.answer(0);
    s.run_jobs();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"\n",
        "ファイルは変わらない"
    );
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
    assert!(doc(&s).is_dirty());
    assert!(doc(&s).to_string_lossy().contains('\u{10FF80}'));
}

#[test]
fn 表現不能の先頭にエスケープ文字があれば_選択肢はキャンセルだけ() {
    let mut s = inject_and_save_as_sjis(&format!("\u{10FF80}{}", "😀".repeat(101)));
    assert_eq!(s.overlay(), "dialog");
    assert_eq!(choice_keys(&mut s).len(), 1);
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"\n");
}

#[test]
fn エスケープ文字が無ければ_表現不能が_101_件以上でも_置換して保存を出し_全件を置換する() {
    let mut s = inject_and_save_as_sjis(&"😀".repeat(101));
    let keys = choice_keys(&mut s);
    assert_eq!(
        keys.len(),
        3,
        "UTF-8 で保存 / 置換して保存 / キャンセル: {keys:?}"
    );
    assert_eq!(keys[..2], [Some('u'), Some('r')]);
    // 置換して保存 → 一覧で承認
    s.answer(1);
    s.run_jobs();
    assert_eq!(s.overlay(), "dialog", "置換の一覧");
    s.answer(0);
    s.run_jobs();
    let want = [b"?".repeat(101), b"\n".to_vec()].concat();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), want);
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
}
