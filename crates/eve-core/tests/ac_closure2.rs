//! AC の棚卸し (M5) の穴埋め その 2: 不正バイトとコードポイントの表記、曖昧幅、往復コーパスの保存、チャンク境界の不正バイト。

use eve_config::Config;
use eve_core::Args;
use eve_testkit::Harness;

fn row0(h: &mut Harness) -> String {
    h.frame().panes[0].rows[0]
        .spans
        .iter()
        .map(|s| s.text.to_string())
        .collect()
}

fn opened(bytes: &[u8]) -> Harness {
    let mut h = Harness::new().with_size(60, 4).with_file("/w/a.txt", bytes);
    h.open("/w/a.txt");
    h.run_jobs();
    // 開くときの確認 (不正バイト・バイナリ) が出ていれば、読み取り専用で開く
    if h.overlay() == "dialog" {
        h.keys("r");
        h.run_jobs();
    }
    h
}

// verifies: AC-FR-507-3
#[test]
fn 不正バイトは_0x_形式で_c1_制御文字は_u_形式で書き分ける() {
    // 不正バイト 0x85 (UTF-8 として不正)
    let mut h = opened(b"a\x85b\n");
    let r = row0(&mut h);
    assert!(r.contains("<0x85>") && !r.contains("<U+0085>"), "{r}");
    // 文字 U+0085 (C1 制御文字)
    let mut h = Harness::new().with_size(60, 4);
    h.paste("a\u{85}b");
    let r = row0(&mut h);
    assert!(r.contains("<U+0085>") && !r.contains("<0x85>"), "{r}");
}

// verifies: AC-NFR-605-1
// verifies: AC-FR-701-1
#[test]
fn 曖昧幅の設定で_frame_のセル幅が切り替わる() {
    let width = |amb: u8| {
        let mut h = Harness::new().with_size(60, 4);
        h.editor.state_mut().set_config(Config {
            ambiguous_width: amb,
            ..Config::default()
        });
        h.paste("±±±");
        h.frame().panes[0].rows[0]
            .spans
            .iter()
            .map(|s| s.width)
            .sum::<u16>()
    };
    assert_eq!(width(1), 3, "曖昧幅 1: 1 文字 1 セル");
    assert_eq!(width(2), 6, "曖昧幅 2: 1 文字 2 セル");
}

// ---------- 往復コーパス (NFR-303) ----------

fn sjis(s: &str) -> Vec<u8> {
    encoding_rs::SHIFT_JIS.encode(s).0.into_owned()
}

/// 区分 A のコーパス (バイト列)。UTF-8/16 (BOM 有無)、Shift_JIS・EUC-JP、改行の各種、末尾改行の有無。
fn corpus_a() -> Vec<(&'static str, Vec<u8>)> {
    let utf16le: Vec<u8> = "日本語\r\nabc\r\n"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let utf16be: Vec<u8> = "日本語\nabc"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    vec![
        ("UTF-8", "日本語のテキストです。\nabc\n".as_bytes().to_vec()),
        (
            "UTF-8 BOM",
            [&b"\xEF\xBB\xBF"[..], "日本語\r\nabc\r\n".as_bytes()].concat(),
        ),
        ("UTF-16LE BOM", [&b"\xFF\xFE"[..], &utf16le].concat()),
        ("UTF-16BE BOM", [&b"\xFE\xFF"[..], &utf16be].concat()),
        (
            "Shift_JIS",
            sjis("日本語のテキストエディタです。これは文章の見本です。\nabc\n"),
        ),
        (
            "EUC-JP",
            encoding_rs::EUC_JP
                .encode("日本語のテキストエディタです。これは文章の見本です。\r\nabc\r\n")
                .0
                .into_owned(),
        ),
        ("LF", b"a\nb\nc\n".to_vec()),
        ("CRLF", b"a\r\nb\r\nc\r\n".to_vec()),
        ("CR", b"a\rb\rc\r".to_vec()),
        ("混在", b"a\nb\r\nc\rd\n".to_vec()),
        ("末尾改行なし", b"abc".to_vec()),
        ("末尾改行あり", b"abc\n".to_vec()),
        ("空", Vec::new()),
    ]
}

// verifies: AC-NFR-303-1, AC-NFR-303-2
#[test]
fn 区分_a_のコーパスは無編集の上書き保存でも名前を付けて保存でもバイト一致する() {
    for (name, bytes) in corpus_a() {
        let mut h = Harness::new().with_file("/w/c.txt", &bytes);
        h.open("/w/c.txt");
        h.run_jobs();
        if h.overlay() == "dialog" {
            h.keys("r");
            h.run_jobs();
        }
        // 無編集の上書き保存: 書き換わらない
        h.cmd("file.save");
        h.run_jobs();
        assert_eq!(h.fs_read("/w/c.txt").unwrap(), bytes, "{name}: 上書き");
        // 名前を付けて保存 (同じ文字コード): バイト一致
        h.cmd_with("file.save_as", Args::new().with_str("path", "/w/copy.txt"));
        h.run_jobs();
        assert_eq!(h.fs_read("/w/copy.txt").unwrap(), bytes, "{name}: 別名");
    }
}

// ---------- NFR-304 ----------

// verifies: AC-NFR-304-2
#[test]
fn チャンク境界をまたぐ不正バイトも元のバイト値として表示される() {
    // UTF-8 として不正なバイト (0x80 の単独・0xC3 の後に ASCII・途中で切れた 3 バイト列) を、チャンクの大きさを変えて読む
    let body = b"abc\x80def\xc3xyz\xe3\x81\x00ghi\xff\n";
    for chunk in [1usize, 2, 3, 5, 7, 64] {
        let mut h = Harness::new().with_size(80, 4).with_file("/w/a.txt", body);
        h.fs.set_chunk_size(chunk);
        h.open("/w/a.txt");
        h.run_jobs();
        if h.overlay() == "dialog" {
            h.keys("r");
            h.run_jobs();
        }
        let r = row0(&mut h);
        for want in ["<0x80>", "<0xC3>", "<0xE3>", "<0x81>", "<0xFF>"] {
            assert!(r.contains(want), "chunk={chunk}: {want} が {r:?} に無い");
        }
    }
}
