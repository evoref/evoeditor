//! エスケープ文字の由来の文字コード (`TextFormat.escape_encoding`) と保存の扱い (docs/03 §8.3.1 の 2 つの表、§22.3 の
//! 「不正バイトを含む文書の編集」の (f)(g)、v0.4 の M23 の 2 本目)。保存先は `MemFs`、ジョブは `ManualJobSpawner`。
//!
//! 不正バイトを含む文書は読み取り専用で開く。編集が要る場合は、製品の「編集を許可」(`file.toggle_read_only` と確認の
//! `[A]`、M23 の 6 本目) で解除する。

use eve_core::{
    Args, CommandResult, Document, Encoding, FileTime, FsPath, OpenOptions, ReadOnlyReason,
    SwapBody, SwapEntry, SwapOwner,
};
use eve_encoding::PuaEscapeMode;
use eve_testkit::Harness;
use eve_text::LineEnding;

fn env() -> Harness {
    Harness::builder().defer_start().build()
}

fn run(h: &mut Harness, id: &str, args: Args) -> CommandResult {
    let r = h.cmd_with(id, args);
    h.run_jobs();
    r
}

/// 不正バイトが多いファイルの確認 (03 §8.3) が出ていれば「読み取り専用で開く」を選ぶ。
fn accept_read_only(h: &mut Harness) {
    if h.overlay() == "dialog" {
        h.text("r");
        h.run_jobs();
    }
}

fn open(h: &mut Harness, path: &str) {
    run(h, "file.open", Args::new().with_str("path", path));
    accept_read_only(h);
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

/// 「編集を許可」(`file.toggle_read_only` → 確認 `ui.confirm.read_only.invalid_bytes` の `[A]`、03 §8.3.1)。
fn release_invalid_bytes(h: &mut Harness) {
    assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
    assert_eq!(
        h.overlay(),
        "dialog",
        "編集を許可の確認: {:?}",
        h.messages()
    );
    h.text("a");
    let id = h.doc_id();
    let d = h.editor.state().document(id).unwrap();
    assert!(
        d.is_editable() && d.invalid_bytes_released,
        "{:?} {:?}",
        d.read_only.iter().collect::<Vec<_>>(),
        d.load_state
    );
}

/// 出ている確認の選択肢のアクセラレータ (表示順。E3 は UTF-8 で保存が `u`、置換して保存が `r`、最後がキャンセル)。
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

/// E3 (表現不能) の確認が出ていて、選べるのがキャンセルだけ。
fn assert_e3_cancel_only(s: &mut Harness) {
    assert_eq!(
        s.overlay(),
        "dialog",
        "表現不能の確認が出る: {:?}",
        s.messages()
    );
    let keys = choice_keys(s);
    assert_eq!(keys.len(), 1, "キャンセルだけ: {keys:?}");
}

fn sjis(s: &str) -> Vec<u8> {
    encoding_rs::SHIFT_JIS.encode(s).0.into_owned()
}

/// 本文の `U+10FF00..=U+10FFFF` の数。
fn escapes(d: &Document) -> u64 {
    d.to_string_lossy()
        .chars()
        .filter(|c| (0x10FF00..=0x10FFFF).contains(&u32::from(*c)))
        .count() as u64
}

// ---------- (e) 読込で決まる値 (03 §8.3.1 の「エスケープ文字の由来の文字コード」の表の 1 行目、INV-D7) ----------

/// `bytes` を `enc` を指定して開き、`escape_encoding` と、INV-D7 (読込の直後は、衝突でなければエスケープ文字の数が
/// 不正バイトの数と同じ) を確かめる。
fn check_load(bytes: &[u8], enc: Encoding, want: Option<Encoding>) {
    let mut s = env();
    s.fs.add_file("/w/a.txt", bytes);
    open_as(&mut s, "/w/a.txt", enc);
    let d = doc(&s);
    assert_eq!(d.format.encoding, enc, "{bytes:02X?}");
    assert_eq!(d.format.escape_encoding, want, "{enc:?} {bytes:02X?}");
    if d.format.pua_escape != PuaEscapeMode::DisabledCollision {
        assert_eq!(escapes(d), u64::from(d.format.invalid_bytes), "INV-D7");
    }
}

#[test]
fn 読込で_enabled_で不正バイトがあれば文書の文字コード_なければ_none() {
    // UTF-8
    check_load(b"a\x80b\n", Encoding::Utf8, Some(Encoding::Utf8));
    check_load("あいう\n".as_bytes(), Encoding::Utf8, None);
    // UTF-8 + BOM
    check_load(b"\xEF\xBB\xBFa\xFFb", Encoding::Utf8, Some(Encoding::Utf8));
    // Shift_JIS
    check_load(
        &[sjis("日本"), b"\xA0x\n".to_vec()].concat(),
        Encoding::ShiftJis,
        Some(Encoding::ShiftJis),
    );
    check_load(&sjis("日本語\n"), Encoding::ShiftJis, None);
    // EUC-JP
    check_load(b"a\xFFb\n", Encoding::EucJp, Some(Encoding::EucJp));
    check_load(
        &encoding_rs::EUC_JP.encode("日本語\n").0,
        Encoding::EucJp,
        None,
    );
    // UTF-16 (奇数長の末尾の 1 バイト)
    check_load(
        b"\xFF\xFEa\x00b\x00c",
        Encoding::Utf16Le,
        Some(Encoding::Utf16Le),
    );
    check_load(b"\xFF\xFEa\x00b\x00", Encoding::Utf16Le, None);
    check_load(
        b"\xFE\xFF\x00a\x00b\x00",
        Encoding::Utf16Be,
        Some(Encoding::Utf16Be),
    );
    check_load(b"\xFE\xFF\x00a\x00b", Encoding::Utf16Be, None);
}

#[test]
fn 読込で状態付きの文書は不正バイトがあっても_none() {
    // ISO-2022-JP: JIS X 0208 の状態のままの改行は不正バイト (03 §8.3)
    check_load(b"\x1B$B$\"\n\x1B(Bx", Encoding::Iso2022Jp, None);
    check_load(b"abc\x1B$B$\"$$\x1B(B def\n", Encoding::Iso2022Jp, None);
}

#[test]
fn 読込で衝突の文書は_none() {
    // 衝突の文書 (a): 元のファイルが真に U+10FF41 を含む (不正バイトなし)
    check_load(b"a\xF4\x8F\xBD\x81b\n", Encoding::Utf8, None);
    // 衝突の文書 (b): 真の私用文字と不正バイトの両方
    check_load(b"a\xF4\x8F\xBD\x81b\x80\n", Encoding::Utf8, None);
}

// ---------- (f) 違う文字コードで保存した後の Undo (表の 2 行目「違う文字コードでの保存の成功」) ----------

/// 不正バイト (0xA0) を含む `Shift_JIS` の文書を開いて「編集を許可」した後の状態。本文は `ab<0xA0>c\n`。
/// 不正バイトの比率の確認 (1% 超) には「読み取り専用で開く」で答える (理由は `InvalidBytes` だけ、03 §9.2 の表)。
fn sjis_released() -> Harness {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"ab\xA0c\n");
    open_as(&mut s, "/w/a.txt", Encoding::ShiftJis);
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::ShiftJis));
    release_invalid_bytes(&mut s);
    s
}

/// カーソルを 3 文字目 (エスケープ文字) に置いて消す。
fn delete_escape(s: &mut Harness) {
    s.keys("ctrl+home");
    s.keys("right");
    s.keys("right");
    s.keys("delete");
    assert_eq!(s.doc_text(), "abc\n");
}

// verifies: NFR-304
#[test]
fn エスケープ文字を消して別の文字コードで保存し_undo_で戻したエスケープ文字は元のバイトに戻さない()
{
    let mut s = sjis_released();
    delete_escape(&mut s);
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    let utf8 = b"abc\n".to_vec();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), utf8);
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
    assert_eq!(
        doc(&s).format.escape_encoding,
        None,
        "違う文字コードの保存の成功"
    );
    // Undo でエスケープ文字が戻る。保存しようとすると表現不能 (キャンセルだけ) で、何も書かない
    s.cmd("edit.undo");
    assert_eq!(s.doc_text(), "ab\u{10FFA0}c\n");
    assert_eq!(doc(&s).format.escape_encoding, None);
    run(&mut s, "file.save", Args::new());
    assert_e3_cancel_only(&mut s);
    s.answer(0);
    s.run_jobs();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        utf8,
        "元の Shift_JIS のバイト (0xA0) を UTF-8 のファイルに書かない"
    );
    assert!(doc(&s).is_dirty());
}

// E3 の「UTF-8 で保存」も違う文字コードの保存の成功 (03 §8.3.1)
#[test]
fn e3_の_utf8_で保存の後も_undo_で戻したエスケープ文字は元のバイトに戻さない() {
    let mut s = sjis_released();
    delete_escape(&mut s);
    // Shift_JIS で表せない文字を入れて保存 → E3 で「UTF-8 で保存」
    s.text("😀");
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.overlay(), "dialog");
    assert_eq!(choice_keys(&mut s)[0], Some('u'));
    s.answer(0);
    s.run_jobs();
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
    assert_eq!(doc(&s).format.escape_encoding, None);
    let saved = s.fs.read("/w/a.txt").unwrap();
    s.cmd("edit.undo");
    s.cmd("edit.undo");
    assert!(s.doc_text().contains('\u{10FFA0}'), "{:?}", s.doc_text());
    run(&mut s, "file.save", Args::new());
    assert_e3_cancel_only(&mut s);
    s.answer(0);
    s.run_jobs();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), saved, "何も書かない");
}

// 表の「同じ文字コードでの保存の成功」の行: 変えない
#[test]
fn 同じ文字コードで保存しても由来の文字コードは変わらず_元のバイトに戻して書く() {
    let mut s = sjis_released();
    s.keys("ctrl+end");
    s.text("z");
    run(&mut s, "file.save", Args::new());
    let want = b"ab\xA0c\nz".to_vec();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), want);
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::ShiftJis));
    // 同じ文字コードの `file.set_encoding` (BOM の有無だけを変える経路と同じ) でも変えない
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::ShiftJis));
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), want);
}

// ---------- (g) 衝突の文書 (b) (表の 4 行目) ----------

/// 真の私用文字 U+10FF41 と不正バイト 0x80 の両方を含む UTF-8 の文書。
const COLLISION_B: &[u8] = b"a\xF4\x8F\xBD\x81b\x80\n";

// verifies: NFR-304
#[test]
fn 衝突の文書_b_を同じ文字コードで名前を付けて保存すると_キャンセルだけで何も書かない() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", COLLISION_B);
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).format.pua_escape, PuaEscapeMode::DisabledCollision);
    assert!(doc(&s).format.invalid_bytes > 0);
    assert!(!doc(&s).is_editable());
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/copy.txt"),
    );
    assert_e3_cancel_only(&mut s);
    s.answer(0);
    s.run_jobs();
    assert!(
        s.fs.read("/w/copy.txt").is_none(),
        "不正バイトを私用文字の UTF-8 として書かない"
    );
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), COLLISION_B);
}

#[test]
fn 衝突の文書_b_は別の文字コードでも保存できない() {
    for enc in ["shift_jis", "utf-16le", "euc-jp"] {
        let mut s = env();
        s.fs.add_file("/w/a.txt", COLLISION_B);
        open(&mut s, "/w/a.txt");
        run(
            &mut s,
            "file.set_encoding",
            Args::new().with_str("encoding", enc),
        );
        assert_eq!(s.overlay(), "prompt", "読み取り専用は名前を付けて保存へ");
        for _ in 0..256 {
            s.keys("backspace");
        }
        s.text("/w/out.txt");
        s.keys("enter");
        s.run_jobs();
        assert_e3_cancel_only(&mut s);
        assert!(s.fs.read("/w/out.txt").is_none(), "{enc}");
    }
}

#[test]
fn 衝突の文書_a_の私用文字は通常の文字として書く() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"a\xF4\x8F\xBD\x81b\n");
    open(&mut s, "/w/a.txt");
    assert!(doc(&s).is_editable(), "不正バイトが無ければ編集できる");
    s.keys("ctrl+end");
    s.text("z");
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"a\xF4\x8F\xBD\x81b\nz");
}

// ---------- (c) 復旧の適用 (表の「復旧の適用」の行) ----------

fn swap_entry(path: Option<&str>) -> SwapEntry {
    SwapEntry {
        dir: FsPath::from(if path.is_some() {
            "/s/swap/aaaa-4242.swp"
        } else {
            "/s/swap/untitled-1-4242.swp"
        }),
        path: path.map(FsPath::from),
        untitled_name: path.is_none().then(|| "無題".to_owned()),
        revision: 7,
        len: 3,
        pid: 4242,
        owner: SwapOwner::Stale,
        sensitive: false,
        modified: Some(FileTime {
            secs: 4_000_000_000,
            nanos: 0,
        }),
        readable: true,
        encoding: None,
    }
}

fn swap_body(text: &str, encoding: Encoding, pua_escaped: bool) -> SwapBody {
    SwapBody {
        text: text.to_owned(),
        encoding,
        has_bom: false,
        default_eol: LineEnding::Lf,
        pua_escaped,
    }
}

/// `bytes` のファイル `/w/a.txt` を開き、スワップ `bodies` (先頭から順に提案される) の 1 つ目の復旧を適用した状態。
fn recovered_from(bytes: &[u8], body: SwapBody) -> Harness {
    let mut s = Harness::new()
        .with_size(80, 12)
        .with_file("/w/a.txt", bytes);
    s.fs.add_swap_entry(swap_entry(Some("/w/a.txt")), Some(body));
    s.open("/w/a.txt");
    s.run_jobs();
    s.advance_ms(0);
    assert_eq!(s.overlay(), "dialog", "復旧の提案 {:?}", s.messages());
    s.answer(0);
    s.run_jobs();
    // 適用の後も読み取り専用 (不正バイトを含む) なら、切替で編集を許可すると続けられると示す (03 §10.3、M23 の 6 本目)
    let key = if doc(&s).is_editable() {
        "info.swap.recovered"
    } else {
        "info.swap.applied_read_only"
    };
    assert!(s.messages().contains(&key.to_owned()), "{:?}", s.messages());
    s
}

/// 不正バイトの無い UTF-8 のファイル `/w/a.txt` (`old`) を開き、スワップの復旧を適用した状態。
fn recovered(body: SwapBody) -> Harness {
    recovered_from(b"old", body)
}

fn invalid_bytes_reason(s: &Harness) -> bool {
    doc(s).read_only.contains(ReadOnlyReason::InvalidBytes)
}

// verifies: NFR-304
#[test]
fn 同じ文字コードの復旧は元のバイトに戻し_読込と同じく読み取り専用にし_復旧の_undo_で前の値に戻る()
{
    let mut s = recovered(swap_body("x\u{10FF80}y", Encoding::Utf8, true));
    assert_eq!(s.doc_text(), "x\u{10FF80}y");
    assert_eq!(
        doc(&s).format.escape_encoding,
        Some(Encoding::Utf8),
        "ディスクのファイルが不正バイトを含まなくても"
    );
    // 読込と同じく、内容に由来する読み取り専用の理由が付く (「本文に入れる規則」と解除が入るまで編集させない)
    assert!(invalid_bytes_reason(&s));
    assert!(!doc(&s).is_editable());
    // 名前を付けて保存は、元のバイトに戻して書く
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/b.txt"),
    );
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(s.fs.read("/w/b.txt").unwrap(), b"x\x80y");
    // 復旧の Undo (「編集を許可」で解除した後) で、由来の文字コードが前に戻り、Redo で後の値になる。解除は Undo /
    // Redo で戻らないので、Redo でも読み取り専用には戻らない (03 §7.2、§8.3.1 の「解除の持ち越し」の細目 (b))
    release_invalid_bytes(&mut s);
    s.cmd("edit.undo");
    assert_eq!(s.doc_text(), "old");
    assert_eq!(doc(&s).format.escape_encoding, None, "復旧の前の値");
    assert!(!invalid_bytes_reason(&s));
    s.cmd("edit.redo");
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::Utf8));
    assert!(!invalid_bytes_reason(&s), "解除の記録があるので付けない");
}

// データの損失のレビューの再現 (1): 復旧で Some になった文書が編集できると、外から入れた私用文字 (Shift_JIS の文書の
// エスケープ文字のコピー・他のアプリの U+10FFA0) が、保存で生の 1 バイト (0xA0) になっていた
// verifies: NFR-304
#[test]
fn 復旧で由来の文字コードが付いた文書には外から私用文字を入れられず_生のバイトで書かれない() {
    let mut s = recovered(swap_body("x\u{10FF80}y", Encoding::Utf8, true));
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::Utf8));
    assert!(!doc(&s).is_editable(), "復旧した文書は読み取り専用");
    s.keys("ctrl+end");
    s.text("\u{10FFA0}");
    assert_eq!(s.doc_text(), "x\u{10FF80}y", "入力は拒否される");
    run(&mut s, "file.save", Args::new());
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"old",
        "読み取り専用の文書は同じファイルへ書かない (名前の入力へ進む)"
    );
}

// verifies: NFR-304
#[test]
fn 文字コードの違う復旧のエスケープ文字は元のバイトに戻さない() {
    let mut s = recovered(swap_body("x\u{10FFA0}y", Encoding::ShiftJis, true));
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8, "開いた形式のまま");
    assert_eq!(doc(&s).format.escape_encoding, None);
    assert!(!invalid_bytes_reason(&s), "None なら読み取り専用にしない");
    run(&mut s, "file.save", Args::new());
    assert_e3_cancel_only(&mut s);
    s.answer(0);
    s.run_jobs();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"old",
        "Shift_JIS のバイトを UTF-8 のファイルに書かない"
    );
}

#[test]
fn pua_escaped_が偽の復旧のエスケープ文字は元のバイトに戻さない() {
    let mut s = recovered(swap_body("x\u{10FF80}y", Encoding::Utf8, false));
    assert_eq!(doc(&s).format.escape_encoding, None);
    run(&mut s, "file.save", Args::new());
    assert_e3_cancel_only(&mut s);
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"old");
}

// データの損失のレビューの再現 (2): 同じパスのスワップが 2 つある (2 つのプロセスが同じファイルを開いたまま落ちた)。
// 1 つ目 (pua_escaped = true) を復旧して保存すると Some(UTF-8) のまま clean になり、2 つ目 (pua_escaped = false) が
// 提案される。本文を置き換える復旧で「偽なら変えない」だと Some(UTF-8) が残り、2 つ目の本文のエスケープ文字
// (Shift_JIS のバイト) が UTF-8 のファイルに生のバイトで書かれていた
// verifies: NFR-304
#[test]
fn 由来の文字コードがある文書に_pua_escaped_が偽のスワップを適用すると_none_になる() {
    let mut s = Harness::new()
        .with_size(80, 12)
        .with_file("/w/a.txt", b"old");
    s.fs.add_swap_entry(
        swap_entry(Some("/w/a.txt")),
        Some(swap_body("x\u{10FF80}y", Encoding::Utf8, true)),
    );
    let mut second = swap_entry(Some("/w/a.txt"));
    second.dir = FsPath::from("/s/swap/aaaa-4343.swp");
    second.modified = Some(FileTime {
        secs: 3_999_999_000,
        nanos: 0,
    });
    s.fs.add_swap_entry(
        second,
        Some(swap_body("p\u{10FFA0}q", Encoding::ShiftJis, false)),
    );
    s.open("/w/a.txt");
    s.run_jobs();
    s.advance_ms(0);
    assert_eq!(s.overlay(), "dialog", "1 つ目の復旧の提案");
    s.answer(0);
    s.run_jobs();
    assert_eq!(s.doc_text(), "x\u{10FF80}y");
    // 1 つ目の読込の間に、古い Revision で 2 つ目の提案が出ている。適用を選んでも「提案の後で文書が変わった」で
    // 候補に戻る (安全側)
    assert_eq!(
        s.overlay(),
        "dialog",
        "古い Revision で 2 つ目の提案が出ている"
    );
    s.answer(0);
    s.run_jobs();
    assert!(
        s.messages()
            .contains(&"msg.swap.changed_since_offer".to_owned())
    );
    // 解除した後の状態で保存する (元のバイトに戻して書く)
    release_invalid_bytes(&mut s);
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"x\x80y");
    assert!(!doc(&s).is_dirty());
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::Utf8));
    // 2 つ目の提案 (pua_escaped = false)
    s.advance_ms(0);
    assert_eq!(
        s.overlay(),
        "dialog",
        "2 つ目の復旧の提案 {:?}",
        s.messages()
    );
    s.answer(0);
    s.run_jobs();
    assert_eq!(s.doc_text(), "p\u{10FFA0}q");
    assert_eq!(doc(&s).format.escape_encoding, None);
    run(&mut s, "file.save", Args::new());
    assert_e3_cancel_only(&mut s);
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"x\x80y",
        "pua_escaped = false のスワップのエスケープ文字を生のバイトで書かない"
    );
}

// データの損失のレビューの再現 (3、変更前から): 衝突の文書 (a) (元のファイルが真に U+10FF41 を含み、不正バイトは
// 無い。編集できる) に、不正バイトを持つ文書のスワップ (pua_escaped = true) を復旧すると、保存の扱いが衝突 (a) の
// AsText のままで、スワップの不正バイト 0x80 が私用文字 U+10FF80 の UTF-8 (F4 8F BE 80) で書かれていた
// verifies: NFR-304
#[test]
fn 衝突の文書_a_への_pua_escaped_が真の復旧は_スワップに従ってバイトの文書になる() {
    let mut s = recovered_from(
        b"a\xF4\x8F\xBD\x81b",
        swap_body("x\u{10FF80}y", Encoding::Utf8, true),
    );
    assert_eq!(s.doc_text(), "x\u{10FF80}y");
    assert_eq!(doc(&s).format.pua_escape, PuaEscapeMode::Enabled);
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::Utf8));
    assert!(invalid_bytes_reason(&s));
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/b.txt"),
    );
    assert_eq!(
        s.fs.read("/w/b.txt").unwrap(),
        b"x\x80y",
        "私用文字の UTF-8 ではなく、元のバイトで書く"
    );
    // 復旧の Undo で衝突の文書に戻る
    release_invalid_bytes(&mut s);
    s.cmd("edit.undo");
    assert_eq!(doc(&s).format.pua_escape, PuaEscapeMode::DisabledCollision);
    assert_eq!(doc(&s).format.escape_encoding, None);
    assert!(!invalid_bytes_reason(&s));
}

#[test]
fn 衝突の文書_a_への文字コードの違う復旧はどの文字コードでも元のバイトに戻さない() {
    let mut s = recovered_from(
        b"a\xF4\x8F\xBD\x81b",
        swap_body("x\u{10FFA0}y", Encoding::ShiftJis, true),
    );
    assert_eq!(doc(&s).format.pua_escape, PuaEscapeMode::Enabled);
    assert_eq!(doc(&s).format.escape_encoding, None);
    run(&mut s, "file.save", Args::new());
    assert_e3_cancel_only(&mut s);
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"a\xF4\x8F\xBD\x81b");
}

#[test]
fn 状態付きの文書への復旧は元のバイトに戻さない() {
    const ISO: &[u8] = b"abc\x1B$B$\"$$\x1B(B def\n";
    for (enc, text) in [
        (Encoding::Iso2022Jp, "x\u{10FF0A}y"),
        (Encoding::Utf8, "x\u{10FF80}y"),
    ] {
        let mut s = recovered_from(ISO, swap_body(text, enc, true));
        assert_eq!(doc(&s).format.encoding, Encoding::Iso2022Jp);
        assert_eq!(
            doc(&s).format.pua_escape,
            PuaEscapeMode::DisabledStateful,
            "{enc:?}"
        );
        assert_eq!(doc(&s).format.escape_encoding, None);
        run(&mut s, "file.save", Args::new());
        assert_e3_cancel_only(&mut s);
        assert_eq!(s.fs.read("/w/a.txt").unwrap(), ISO, "{enc:?}");
    }
}

// 本文が同じ文字列のスワップでも、本文の全体を置き換える復旧なので、表のとおりに決める (ここでは文字コードが違うので None)
#[test]
fn 本文が同じ文字列のスワップも本文を置き換える復旧として表のとおり決める() {
    let s = recovered(swap_body("old", Encoding::ShiftJis, true));
    assert_eq!(s.doc_text(), "old");
    assert_eq!(doc(&s).format.escape_encoding, None);
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
    assert!(doc(&s).is_editable());
}

// 本文が同じ復旧 (全置換が恒等。空の無題の文書に空のスワップ) は書式を変えない (エスケープ文字の意味も今の値のまま)。
// 恒等でなければ pua_escaped = false で None になるところ
#[test]
fn 全置換が恒等の復旧は由来の文字コードを変えない() {
    let s = recovered_untitled(swap_body("", Encoding::Utf8, false), |d| {
        d.format.escape_encoding = Some(Encoding::Utf8);
    });
    assert_eq!(s.doc_text(), "");
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::Utf8));
    assert!(!doc(&s).is_dirty());
}

/// 起動時の無題で空の文書に、無題のスワップ `body` を復旧した状態。`before` で復旧の前の文書を整える。
fn recovered_untitled(body: SwapBody, before: impl FnOnce(&mut Document)) -> Harness {
    let mut s = Harness::new();
    let id = s.doc_id();
    before(s.editor.state_mut().document_mut(id).unwrap());
    s.fs.add_swap_entry(swap_entry(None), Some(body));
    s.run_jobs();
    s.advance_ms(0);
    assert_eq!(s.overlay(), "dialog", "{:?}", s.messages());
    s.answer(0);
    s.run_jobs();
    s
}

#[test]
fn 無題の文書はスワップの文字コードで元のバイトに戻す() {
    let mut s = recovered_untitled(swap_body("a\u{10FFA0}b", Encoding::ShiftJis, true), |_| {});
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::ShiftJis));
    assert!(invalid_bytes_reason(&s));
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/u.txt"),
    );
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(s.fs.read("/w/u.txt").unwrap(), b"a\xA0b");
    // 復旧の Undo で前の値 (新規文書の None) に戻る
    release_invalid_bytes(&mut s);
    s.cmd("edit.undo");
    assert_eq!(s.doc_text(), "");
    assert_eq!(doc(&s).format.escape_encoding, None);
}

// 無題の文書の Some(X) に、違う文字コードのスワップを適用する: 文字コードを写す前に None にするので、復旧の Undo で
// 戻るのも None (Some(X) が別の文字コードの文書に戻らない。不変条件 None か Some(encoding))
#[test]
fn 無題の文書の由来の文字コードは違う文字コードのスワップを写すと前の値が_none_になる() {
    for pua_escaped in [true, false] {
        let mut s = recovered_untitled(
            swap_body("a\u{10FFA0}b", Encoding::ShiftJis, pua_escaped),
            |d| d.format.escape_encoding = Some(Encoding::Utf8),
        );
        assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
        let want = pua_escaped.then_some(Encoding::ShiftJis);
        assert_eq!(doc(&s).format.escape_encoding, want);
        // 由来の文字コードが付いた (読み取り専用になった) ときだけ「編集を許可」する
        assert_eq!(doc(&s).is_editable(), !pua_escaped);
        if pua_escaped {
            release_invalid_bytes(&mut s);
        }
        s.cmd("edit.undo");
        assert_eq!(s.doc_text(), "");
        assert_eq!(
            doc(&s).format.encoding,
            Encoding::ShiftJis,
            "形式は戻さない"
        );
        assert_eq!(doc(&s).format.escape_encoding, None, "{pua_escaped}");
    }
}

// 状態付きの文字コードのスワップは、無題の文書でも元のバイトに戻さない (03 §8.3.1。生のバイトを状態付きの出力に入れない)
#[test]
fn 無題の文書でも状態付きのスワップのエスケープ文字は元のバイトに戻さない() {
    let s = recovered_untitled(swap_body("a\u{10FF0A}b", Encoding::Iso2022Jp, true), |_| {});
    assert_eq!(doc(&s).format.encoding, Encoding::Iso2022Jp);
    assert_eq!(doc(&s).format.escape_encoding, None);
    assert!(!invalid_bytes_reason(&s));
}

// 違う文字コードで保存した後は、復旧の Redo でも元の文字コードの値に戻らない (不変条件: None か Some(encoding))
#[test]
fn 違う文字コードで保存した後は復旧の_redo_でも由来の文字コードが戻らない() {
    let mut s = recovered(swap_body("x\u{10FF80}y", Encoding::Utf8, true));
    release_invalid_bytes(&mut s);
    s.keys("ctrl+home");
    s.keys("right");
    s.keys("delete");
    assert_eq!(s.doc_text(), "xy");
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
    assert_eq!(doc(&s).format.escape_encoding, None);
    s.cmd("edit.undo"); // 削除
    s.cmd("edit.undo"); // 復旧
    assert_eq!(s.doc_text(), "old");
    s.cmd("edit.redo"); // 復旧
    assert_eq!(s.doc_text(), "x\u{10FF80}y");
    assert_eq!(doc(&s).format.escape_encoding, None);
    assert!(
        invalid_bytes_reason(&s),
        "読み取り専用の理由は復旧の後の値 (安全側)"
    );
    run(
        &mut s,
        "file.save_as",
        Args::new().with_str("path", "/w/b.txt"),
    );
    assert_e3_cancel_only(&mut s);
    assert!(s.fs.read("/w/b.txt").is_none());
}

// ---------- (d) 再読込・開き直し・保存の失敗 ----------

#[test]
fn 再読込の_undo_redo_で由来の文字コードも書式ごと戻る() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc\n");
    open(&mut s, "/w/a.txt");
    assert_eq!(doc(&s).format.escape_encoding, None);
    // 外部で不正バイトを含む内容に変わったので読み直す
    s.fs.add_file("/w/a.txt", b"a\x80bc\n");
    s.cmd("file.reload");
    s.run_jobs();
    // 不正バイトの比率の確認 (1% 超) には「読み取り専用で開く」で答える
    accept_read_only(&mut s);
    assert_eq!(s.doc_text(), "a\u{10FF80}bc\n");
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::Utf8));
    // 再読込の Undo は読み取り専用では行えない (03 §7.2)。「編集を許可」で解除してから戻す
    release_invalid_bytes(&mut s);
    s.cmd("edit.undo");
    assert_eq!(s.doc_text(), "abc\n");
    assert_eq!(doc(&s).format.escape_encoding, None);
    s.cmd("edit.redo");
    assert_eq!(s.doc_text(), "a\u{10FF80}bc\n");
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::Utf8));
    // 解除は Undo / Redo で戻らない (03 §7.2、§8.3.1 の「解除の持ち越し」の細目 (b)): 再読込のノードの後の理由に
    // `InvalidBytes` があっても、解除の記録があるので付けない
    assert!(
        !doc(&s).read_only.contains(ReadOnlyReason::InvalidBytes),
        "解除の記録があるので Redo でも読み取り専用に戻らない"
    );
    assert!(doc(&s).is_editable());
}

// 違う文字コードで保存した後に再読込のノードを Undo / Redo すると、文字コードと由来の文字コードが組で戻る
#[test]
fn 違う文字コードの保存の後の再読込の_undo_redo_は文字コードと組で戻る() {
    let mut s = env();
    s.fs.add_file("/w/a.txt", b"abc\n");
    open(&mut s, "/w/a.txt");
    s.fs.add_file("/w/a.txt", b"a\x80bc\n");
    s.cmd("file.reload");
    s.run_jobs();
    // 不正バイトの比率の確認 (1% 超) には「読み取り専用で開く」で答える
    accept_read_only(&mut s);
    release_invalid_bytes(&mut s);
    // エスケープ文字を消して Shift_JIS で保存する
    s.keys("ctrl+home");
    s.keys("right");
    s.keys("delete");
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
    assert_eq!(doc(&s).format.escape_encoding, None);
    s.cmd("edit.undo"); // 削除: 文字コードは Shift_JIS のまま、エスケープ文字は意味を持たない
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
    assert_eq!(doc(&s).format.escape_encoding, None);
    s.cmd("edit.undo"); // 再読込: 再読込の前の書式 (UTF-8、None) に戻る
    assert_eq!(s.doc_text(), "abc\n");
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
    assert_eq!(doc(&s).format.escape_encoding, None);
    s.cmd("edit.redo"); // 再読込: 再読込の後の書式 (UTF-8、Some(UTF-8)) に組で戻る
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::Utf8));
}

#[test]
fn 文字コードを指定して開き直すと由来の文字コードが入れ替わる() {
    let mut s = env();
    // Shift_JIS では「あ」(82 A0)、UTF-8 では不正バイト 2 つ
    s.fs.add_file("/w/a.txt", b"a\x82\xA0\n");
    open_as(&mut s, "/w/a.txt", Encoding::Utf8);
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::Utf8));
    run(
        &mut s,
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    accept_read_only(&mut s);
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
    assert_eq!(doc(&s).format.escape_encoding, None);
    run(
        &mut s,
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    accept_read_only(&mut s);
    assert_eq!(doc(&s).format.encoding, Encoding::Utf8);
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::Utf8));
}

// 違う文字コードの保存が表現不能 (E3) で止まった・キャンセルしたときは変えない (成功したときだけ None にする)
#[test]
fn 違う文字コードの保存がキャンセルで終わったら由来の文字コードは変わらない() {
    let mut s = sjis_released();
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    assert_e3_cancel_only(&mut s);
    s.keys("esc");
    s.run_jobs();
    assert_eq!(s.overlay(), "");
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::ShiftJis));
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), b"ab\xA0c\n");
}

// バイトの文書 (Shift_JIS、エスケープ文字が残る) に Shift_JIS で表せない文字を入れて保存すると E3 が出る。03 §8.6 の
// E3 の表のとおり、この保存はエスケープ文字を元のバイトに戻すので (escapes_restored > 0)「UTF-8 で保存」は出さない
// (選ぶと UTF-8 ではエスケープ文字が表現不能になり、2 回目の E3 になっていた。M23 の 3 本目で直した)。
// 「置換して保存」は出る (表現不能の文字にエスケープ文字を含まない)。何も書かない
#[test]
fn e3_の_utf8_で保存はエスケープ文字を戻す保存では出さない() {
    let mut s = sjis_released();
    s.keys("ctrl+end");
    s.text("😀");
    let before = s.fs.read("/w/a.txt").unwrap();
    run(&mut s, "file.save", Args::new());
    assert_eq!(s.overlay(), "dialog");
    let keys = choice_keys(&mut s);
    assert!(!keys.contains(&Some('u')), "{keys:?}");
    assert!(keys.contains(&Some('r')), "{keys:?}");
    s.keys("esc");
    s.run_jobs();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before, "何も書かない");
    assert_eq!(doc(&s).format.encoding, Encoding::ShiftJis);
}
