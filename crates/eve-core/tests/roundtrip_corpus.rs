//! 往復コーパス (docs/09 §4 の「文字コード」「改行」。01 NFR-303 / NFR-304、03 §8.3 / §8.7)。
//! バイト列 → 開く → 先頭に 1 文字入力 → 保存 で、入力した 1 文字を除いて元のバイト列に戻ること (区分 A: バイト一致)、
//! 区分 B は検出されること、不正バイトは読み取り専用で、名前を付けて保存でバイト一致することを、表の全件で確かめる。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    Args, CommandInvocation, Document, Editor, Encoding, InvocationSource, OpenOptions, Ports,
    ReadOnlyReason,
};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_view::{Capabilities, InputEvent};

struct Env {
    e: Editor,
    fs: Arc<MemFs>,
    jobs: Rc<ManualJobSpawner>,
}

impl Env {
    fn new() -> Env {
        let clock = ManualClock::new();
        let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
        fs.add_dir("/w");
        let paths = Arc::new(FixedPaths::default());
        let jp = eve_core::JobPorts {
            fs: fs.clone(),
            clock: Arc::new(clock.clone()),
            paths: paths.clone(),
        };
        let jobs = ManualJobSpawner::new(jp);
        let ports = Ports {
            fs: fs.clone(),
            clock: Arc::new(clock),
            jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
            paths,
        };
        Env {
            e: Editor::new(ports, Capabilities::default()),
            fs,
            jobs,
        }
    }

    fn pump(&mut self) {
        loop {
            self.jobs.run_jobs();
            let rs = self.jobs.take_results();
            if rs.is_empty() {
                break;
            }
            for r in rs {
                self.e.on_job_result(r);
            }
        }
    }

    fn run(&mut self, id: &str, args: Args) {
        self.e
            .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(args));
        self.pump();
    }

    /// 文字コードを指定して開く。確認ダイアログ (不正バイトが多い等) が出たら読み取り専用で開く。
    fn open_as(&mut self, path: &str, enc: Option<Encoding>) {
        self.e.state_mut().open_path(
            path,
            OpenOptions {
                forced: enc,
                ..OpenOptions::default()
            },
        );
        self.pump();
        if self.e.state().overlay_kind() == "dialog" {
            let w = self.e.state().first_window().unwrap();
            self.e.handle_input(w, InputEvent::Text("r".into()));
            self.pump();
        }
    }

    fn doc(&self) -> &Document {
        let id = self.e.state().active_document().expect("文書がある");
        self.e.state().document(id).unwrap()
    }

    fn type_text(&mut self, s: &str) {
        let w = self.e.state().first_window().unwrap();
        self.e.handle_input(w, InputEvent::Text(s.into()));
    }
}

/// 区分 A: 開いて 1 文字足して保存すると、足した 1 文字以外は元のバイト列のまま。
fn assert_lossless(enc: Encoding, orig: &[u8], label: &str) {
    let mut s = Env::new();
    let mut bytes = b"a".to_vec();
    bytes.extend_from_slice(orig);
    bytes.extend_from_slice(b"b");
    s.fs.add_file("/w/c.txt", &bytes);
    s.open_as("/w/c.txt", Some(enc));
    assert_eq!(s.doc().format.encoding, enc, "{label}");
    assert_eq!(
        s.doc().format.invalid_bytes,
        0,
        "{label}: 不正バイトにならない"
    );
    assert_eq!(
        s.doc().format.roundtrip.lossy_lines,
        0,
        "{label}: 区分 B に数えない"
    );
    assert!(s.doc().is_editable(), "{label}");
    s.type_text("x");
    s.run("file.save", Args::new());
    let mut want = b"x".to_vec();
    want.extend_from_slice(&bytes);
    assert_eq!(s.fs.read("/w/c.txt").unwrap(), want, "{label}");
}

#[test]
fn shift_jis_の_cp932_互換文字は区分_a_でバイト一致する() {
    for (b, name) in [
        (&[0x81u8, 0x60][..], "0x8160"),
        (&[0x81, 0x61], "0x8161"),
        (&[0x81, 0x7C], "0x817C"),
        (&[0x81, 0x91], "0x8191"),
        (&[0x81, 0x92], "0x8192"),
        (&[0x81, 0xCA], "0x81CA"),
        (&[0x81, 0x5C], "0x815C"),
        (&[0xFA, 0x55], "0xFA55"),
    ] {
        assert_lossless(Encoding::ShiftJis, b, name);
    }
}

// verifies: AC-FR-109-2
#[test]
fn shift_jis_の_cp932_外字は往復し_外字以外の編集でも不変() {
    for lead_trail in [
        [0xF0u8, 0x40],
        [0xF0, 0x7E],
        [0xF0, 0x80],
        [0xF0, 0x9F],
        [0xF0, 0xFC],
        [0xF1, 0x40],
        [0xF5, 0x80],
        [0xF9, 0xFC],
    ] {
        assert_lossless(
            Encoding::ShiftJis,
            &lead_trail,
            &format!("外字 {:02X}{:02X}", lead_trail[0], lead_trail[1]),
        );
    }
}

#[test]
fn shift_jis_の重複符号化は区分_b_として検出される() {
    // NEC/IBM 拡張の重複 (¬ の 0xEEF9 / 0xFA54): 再エンコードで別のバイトになる
    for dup in [[0xEEu8, 0xF9], [0xFA, 0x54]] {
        let mut s = Env::new();
        let mut bytes = b"a".to_vec();
        bytes.extend_from_slice(&dup);
        s.fs.add_file("/w/c.txt", &bytes);
        s.open_as("/w/c.txt", Some(Encoding::ShiftJis));
        assert!(
            s.doc().format.roundtrip.lossy_lines > 0,
            "{dup:02X?} は区分 B"
        );
    }
}

#[test]
fn 単独バイトは不正バイトとして読み取り専用で_名前を付けて保存でバイト一致する() {
    // 0xA0 / 0xFD / 0xFE / 0xFF は Shift_JIS の単独バイトとして不正
    for b in [0xA0u8, 0xFD, 0xFE, 0xFF] {
        let mut s = Env::new();
        let bytes = vec![b'a', b, b'b'];
        s.fs.add_file("/w/c.txt", &bytes);
        s.open_as("/w/c.txt", Some(Encoding::ShiftJis));
        assert!(
            s.doc().read_only.contains(ReadOnlyReason::InvalidBytes),
            "{b:#04X}: 読み取り専用"
        );
        s.run("file.save_as", Args::new().with_str("path", "/w/copy.txt"));
        assert_eq!(s.fs.read("/w/copy.txt").unwrap(), bytes, "{b:#04X}");
    }
    // 0x80 は U+0080 で往復する
    assert_lossless(Encoding::ShiftJis, &[0x80], "0x80");
}

#[test]
fn euc_jp_の互換文字は区分_a_でバイト一致する() {
    for (b, name) in [
        (&[0xA1u8, 0xC1][..], "0xA1C1"),
        (&[0xA1, 0xC2], "0xA1C2"),
        (&[0xA1, 0xDD], "0xA1DD"),
        (&[0xA1, 0xF1], "0xA1F1"),
        (&[0xA1, 0xF2], "0xA1F2"),
        (&[0xA2, 0xCC], "0xA2CC"),
        (&[0xA1, 0xBD], "0xA1BD"),
    ] {
        assert_lossless(Encoding::EucJp, b, name);
    }
}

#[test]
fn iso_2022_jp_の正規形は往復する() {
    let (orig, _, _) = encoding_rs::ISO_2022_JP.encode("日本語abc\n二行目\n");
    let mut s = Env::new();
    s.fs.add_file("/w/c.txt", &orig);
    s.open_as("/w/c.txt", Some(Encoding::Iso2022Jp));
    assert_eq!(s.doc().format.invalid_bytes, 0);
    s.type_text("x");
    s.run("file.save", Args::new());
    let (want, _, _) = encoding_rs::ISO_2022_JP.encode("x日本語abc\n二行目\n");
    assert_eq!(s.fs.read("/w/c.txt").unwrap(), want.as_ref());
}

#[test]
fn bom_付きの_utf8_と_utf16_は_bom_ごと往復する() {
    let le: Vec<u8> = "ab\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let be: Vec<u8> = "ab\n".encode_utf16().flat_map(u16::to_be_bytes).collect();
    let cases: [(Encoding, Vec<u8>); 3] = [
        (Encoding::Utf8, b"\xEF\xBB\xBFab\n".to_vec()),
        (Encoding::Utf16Le, [&[0xFF, 0xFE][..], &le].concat()),
        (Encoding::Utf16Be, [&[0xFE, 0xFF][..], &be].concat()),
    ];
    for (enc, orig) in cases {
        let mut s = Env::new();
        s.fs.add_file("/w/c.txt", &orig);
        s.open_as("/w/c.txt", None);
        assert_eq!(s.doc().format.encoding, enc);
        assert!(s.doc().format.has_bom);
        s.type_text("x");
        s.run("file.save", Args::new());
        let saved = s.fs.read("/w/c.txt").unwrap();
        let bom_len = if enc == Encoding::Utf8 { 3 } else { 2 };
        assert_eq!(&saved[..bom_len], &orig[..bom_len], "{enc:?}: BOM が残る");
    }
}

// verifies: AC-FR-113-1, AC-FR-114-1
#[test]
fn 改行の種類はそのまま往復する() {
    let cases: [(&str, &[u8]); 9] = [
        ("LF", b"a\nb\n"),
        ("CRLF", b"a\r\nb\r\n"),
        ("CR", b"a\rb\r"),
        ("混在", b"a\nb\r\nc\rd"),
        ("末尾改行なし", b"abc"),
        ("空", b""),
        ("改行のみ", b"\n\n\n"),
        ("CR CR LF", b"a\r\r\nb"),
        ("CRLF 混在の末尾なし", b"x\r\ny\nz"),
    ];
    for (name, orig) in cases {
        let mut s = Env::new();
        s.fs.add_file("/w/c.txt", orig);
        s.open_as("/w/c.txt", None);
        s.type_text("x");
        s.run("file.save", Args::new());
        let mut want = b"x".to_vec();
        want.extend_from_slice(orig);
        assert_eq!(s.fs.read("/w/c.txt").unwrap(), want, "{name}");
    }
}
