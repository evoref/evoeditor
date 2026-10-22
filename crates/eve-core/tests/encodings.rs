//! 文字コードごとの読み書きのシナリオ (docs/03 §8、01 の FR-108 / 109 / 111 / 112 / 115、NFR-303 / 304)。
//! 保存先は `MemFs`、ジョブは `ManualJobSpawner`。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    Args, CommandInvocation, CommandResult, Document, Editor, Encoding, FsPath, InvocationSource,
    OpenOptions, Ports, ReadOnlyReason,
};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_view::{Capabilities, EditorEvent, InputEvent, WindowId};

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

    fn run(&mut self, id: &str, args: Args) -> CommandResult {
        let r = self
            .e
            .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(args));
        self.pump();
        r
    }

    fn open(&mut self, path: &str) {
        self.run("file.open", Args::new().with_str("path", path));
    }

    /// 不正バイトが多いファイルの確認(03 §8.3)が出ていれば「読み取り専用で開く」を選ぶ。
    fn accept_read_only(&mut self) {
        if self.e.state().overlay_kind() == "dialog" {
            self.type_text("r");
            self.pump();
        }
    }

    fn open_as(&mut self, path: &str, enc: Encoding) {
        self.e.state_mut().open_path(
            path,
            OpenOptions {
                forced: Some(enc),
                ..OpenOptions::default()
            },
        );
        self.pump();
        self.accept_read_only();
    }

    fn doc(&self) -> &Document {
        let id = self.e.state().active_document().expect("文書がある");
        self.e.state().document(id).unwrap()
    }

    fn text(&self) -> String {
        self.doc().to_string_lossy()
    }

    fn win(&self) -> WindowId {
        self.e.state().first_window().unwrap()
    }

    fn type_text(&mut self, s: &str) {
        self.e.handle_input(self.win(), InputEvent::Text(s.into()));
    }

    fn save(&mut self) {
        self.run("file.save", Args::new());
    }

    fn messages(&mut self) -> Vec<String> {
        self.e
            .drain_events()
            .into_iter()
            .filter_map(|ev| match ev {
                EditorEvent::MessageShown { key, .. } => Some(key),
                _ => None,
            })
            .collect()
    }
}

fn sjis(s: &str) -> Vec<u8> {
    encoding_rs::SHIFT_JIS.encode(s).0.into_owned()
}

const JP: &str = "日本語のテキストエディタです。これは文章の見本で、文字コードの判定に使います。\n";

// verifies: AC-FR-109-1, AC-FR-109-2
#[test]
fn shift_jis_を判定して開き_編集して同じ文字コードで保存する() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", &sjis(JP));
    s.open("/w/a.txt");
    assert_eq!(s.text(), JP);
    assert_eq!(s.doc().format.encoding, Encoding::ShiftJis);
    assert!(s.doc().is_editable());
    s.type_text("追記");
    s.save();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), sjis(&format!("追記{JP}")));
}

// verifies: AC-FR-109-1
#[test]
fn euc_jp_を開いて保存する() {
    let mut s = Env::new();
    let bytes = encoding_rs::EUC_JP.encode(JP).0.into_owned();
    s.fs.add_file("/w/a.txt", &bytes);
    s.open("/w/a.txt");
    assert_eq!(s.doc().format.encoding, Encoding::EucJp);
    assert_eq!(s.text(), JP);
    s.type_text("x");
    s.save();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        encoding_rs::EUC_JP.encode(&format!("x{JP}")).0.into_owned()
    );
}

// verifies: AC-FR-108-1
#[test]
fn utf16_の_bom_を保って保存する() {
    let mut s = Env::new();
    let mut bytes = b"\xFF\xFE".to_vec();
    bytes.extend("あa😀\n".encode_utf16().flat_map(u16::to_le_bytes));
    s.fs.add_file("/w/a.txt", &bytes);
    s.open("/w/a.txt");
    assert_eq!(s.doc().format.encoding, Encoding::Utf16Le);
    assert!(s.doc().format.has_bom);
    assert_eq!(s.text(), "あa😀\n");
    s.type_text("b");
    s.save();
    let mut want = b"\xFF\xFE".to_vec();
    want.extend("bあa😀\n".encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), want);
}

// verifies: AC-FR-109-1, AC-FR-109-3
#[test]
fn iso_2022_jp_を判定して行ごとに閉じて保存する() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", b"abc\x1B$B$\"$$\x1B(B def\nx\n");
    s.open("/w/a.txt");
    assert_eq!(s.doc().format.encoding, Encoding::Iso2022Jp);
    assert_eq!(s.text(), "abcあい def\nx\n");
    s.type_text("日");
    s.save();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        b"\x1B$BF|\x1B(Babc\x1B$B$\"$$\x1B(B def\nx\n".to_vec()
    );
}

#[test]
fn 文字コードを指定して開く() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", &sjis("日本語"));
    // UTF-8 と指定すると不正バイトを含む (読み取り専用)
    s.open_as("/w/a.txt", Encoding::Utf8);
    assert_eq!(s.doc().format.encoding, Encoding::Utf8);
    assert!(s.doc().format.invalid_bytes > 0);
    assert!(s.doc().read_only.contains(ReadOnlyReason::InvalidBytes));
}

// verifies: AC-NFR-304-1
#[test]
fn 不正バイトを含む_shift_jis_は読み取り専用で_保存しなければバイトは変わらない() {
    let mut s = Env::new();
    let bytes = [sjis("日本語テキスト"), b"\xA0\xFD".to_vec(), sjis("おわり")].concat();
    s.fs.add_file("/w/a.txt", &bytes);
    s.open_as("/w/a.txt", Encoding::ShiftJis); // 判定は chardetng 次第なので指定して開く
    assert_eq!(s.doc().format.encoding, Encoding::ShiftJis);
    assert_eq!(s.doc().format.invalid_bytes, 2);
    assert!(!s.doc().is_editable());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), bytes);
}

// verifies: AC-NFR-304-3
#[test]
fn 不正バイトを含む_iso_2022_jp_は専用の理由で読み取り専用_保存を拒否する() {
    let mut s = Env::new();
    // JIS X 0208 状態で改行を含む (03 §8.3): 7 bit なので自動判定は ASCII になる → 指定して開く
    s.fs.add_file("/w/a.txt", b"\x1B$B$\"\n\x1B(Bx");
    s.open_as("/w/a.txt", Encoding::Iso2022Jp);
    assert_eq!(s.doc().format.encoding, Encoding::Iso2022Jp);
    assert!(s.doc().format.invalid_bytes > 0);
    assert!(
        s.doc()
            .read_only
            .contains(ReadOnlyReason::StatefulEncodingWithInvalidBytes)
    );
    s.messages();
    s.run("file.save_as", Args::new().with_str("path", "/w/b.txt"));
    assert!(
        s.messages()
            .contains(&"msg.save.encoding_unsupported_iso2022".to_owned())
    );
    assert!(!s.fs.exists("/w/b.txt"), "何も書かない");
}

#[test]
fn 先頭が_ascii_で後半に_shift_jis_が現れたら判定し直して読み直す() {
    let mut s = Env::new();
    let mut bytes = b"plain ascii line\n".repeat(5000); // 85 KB の ASCII
    bytes.extend(sjis(&JP.repeat(20)));
    s.fs.add_file("/w/a.txt", &bytes);
    s.messages();
    s.open("/w/a.txt");
    assert_eq!(s.doc().format.encoding, Encoding::ShiftJis);
    assert!(s.text().ends_with(JP));
    assert!(s.doc().is_editable());
    assert!(s.messages().contains(&"info.open.redetected".to_owned()));
}

#[test]
fn 先頭が_ascii_で後半に_iso_2022_jp_が現れたら判定し直す() {
    let mut s = Env::new();
    let mut bytes = b"plain ascii line\n".repeat(5000);
    bytes.extend(b"\x1B$B$\"$$\x1B(B\n");
    s.fs.add_file("/w/a.txt", &bytes);
    s.open("/w/a.txt");
    assert_eq!(s.doc().format.encoding, Encoding::Iso2022Jp);
    assert!(s.text().ends_with("あい\n"));
}

#[test]
fn 対応外の文字コードの推定は既定で開いて案内する() {
    let mut s = Env::new();
    s.fs.add_file(
        "/w/a.txt",
        b"caf\xE9 au lait, na\xEFve r\xE9sum\xE9 and more text here for the detector.\n",
    );
    s.messages();
    s.open("/w/a.txt");
    s.accept_read_only();
    assert_eq!(s.doc().format.encoding, Encoding::Utf8);
    assert!(
        s.messages()
            .contains(&"info.open.guess_unsupported".to_owned())
    );
}

// verifies: AC-FR-112-1
#[test]
fn 表現できない文字は何も書かずに保存を止める() {
    let mut s = Env::new();
    let original = sjis(JP);
    s.fs.add_file("/w/a.txt", &original);
    s.open("/w/a.txt");
    s.type_text("😀"); // Shift_JIS では表せない
    s.messages();
    s.save();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        original,
        "元のファイルは変わらない"
    );
    assert!(s.doc().is_dirty(), "保存していない");
    assert_eq!(s.e.state().overlay_kind(), "dialog", "E3 の選択肢が出る");
}

#[test]
fn 黙示の置換も表現不能として止める() {
    let mut s = Env::new();
    let original = sjis(JP);
    s.fs.add_file("/w/a.txt", &original);
    s.open("/w/a.txt");
    s.type_text("¥"); // encoding_rs は 0x5C に黙って写す
    s.save();
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), original);
    assert!(s.doc().is_dirty());
}

#[test]
fn 重複符号化の行を読込時に数えて通知する() {
    let mut s = Env::new();
    let bytes = [sjis(JP), b"\xFA\x54\n".to_vec()].concat();
    s.fs.add_file("/w/a.txt", &bytes);
    s.messages();
    s.open("/w/a.txt");
    assert_eq!(s.doc().format.roundtrip.lossy_lines, 1);
    assert!(
        s.messages()
            .contains(&"info.open.lossy_roundtrip".to_owned())
    );
}

#[test]
fn 無編集の_shift_jis_は書き込まない() {
    let mut s = Env::new();
    let bytes = [sjis(JP), b"\xFA\x54\n".to_vec()].concat();
    s.fs.add_file("/w/a.txt", &bytes);
    s.open("/w/a.txt");
    s.save();
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        bytes,
        "区分 B も無編集ならバイト一致 (03 §9.3 手順 0)"
    );
}

#[test]
fn ステータスバーの文字コードは名前と_bom_を出す() {
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", &sjis(JP));
    s.open("/w/a.txt");
    let f = s.e.frame(s.win()).unwrap();
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
    use eve_view::{InputEvent, StyleMods};
    let mut s = Env::new();
    s.fs.add_file("/w/a.txt", &sjis(JP));
    s.open("/w/a.txt");
    s.type_text("😀");
    s.save();
    assert_eq!(s.e.state().overlay_kind(), "dialog", "E3 の選択肢が出る");
    let marked = |s: &mut Env| -> String {
        let w = s.win();
        let f = s.e.frame(w).unwrap();
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
    let w = s.win();
    s.e.handle_input(
        w,
        InputEvent::Key(eve_view::KeyEvent::press(
            eve_view::KeyCode::Esc,
            eve_view::Modifiers::NONE,
        )),
    );
    assert_eq!(s.e.state().overlay_kind(), "");
    assert_eq!(marked(&mut s), "😀", "キャンセルの後も残る");
    s.type_text("y");
    assert_eq!(marked(&mut s), "", "次の編集で消える");
}
