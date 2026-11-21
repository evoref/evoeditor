//! 同じファイルの判定 (SAF-28、docs/03 §12.7.6) を実ファイルシステムで確かめる (L3: `Editor` + 実 FS + 手動のジョブ)。
//! 大文字小文字 (Windows)・シンボリックリンク・ハードリンクの別の書き方で同じファイルを開くと 1 つの文書になり、名前を
//! 付けて保存の保存先の照合 (03 §9.7 (a) の 1 / 1b) が同じファイルを見つける。
//!
//! 一時ファイルは `std::env::temp_dir()` に置く (Windows は NTFS で、識別 (`FileId`) を信頼する FS)。環境変数
//! `EVE_SAME_FILE_DIR` にディレクトリを渡すと、そこでも同じ試験をする (識別を信頼しない `exFAT`・ネットワークの FS で、
//! canonical のパスで判定することを手元で確かめる。例: `exFAT` のドライブ上のディレクトリ)。
//! `EVE_SAME_FILE_UNTRUSTED=1` を一緒に渡すと、そのディレクトリの識別を信頼しないことも確かめる。

#![allow(clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    Args, Clock, CommandInvocation, Editor, FileSystem, FsPath, InvocationSource, JobPorts, Ports,
    Tick,
};
use eve_platform::dirs::Dirs;
use eve_platform::path::PathCtx;
use eve_testkit::ManualJobSpawner;
use eve_view::{Capabilities, EditorEvent};

#[cfg(windows)]
use eve_platform::fs::WinFs as NativeFs;
#[cfg(unix)]
use eve_platform::unix_fs::UnixFs as NativeFs;

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Tick {
        Tick(0)
    }
}

/// 一時ディレクトリ (終わりに消す)。
struct Tmp(PathBuf);

impl Tmp {
    fn new(base: &Path, tag: &str) -> Tmp {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = base.join(format!("eve-same-file-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }

    fn file(&self, name: &str, data: &str) -> String {
        let p = self.0.join(name);
        std::fs::write(&p, data).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 試験を掛けるディレクトリ (一時ディレクトリと、`EVE_SAME_FILE_DIR` があればそこ)。
fn bases() -> Vec<PathBuf> {
    let mut v = vec![std::env::temp_dir()];
    if let Some(d) = std::env::var_os("EVE_SAME_FILE_DIR").filter(|d| !d.is_empty()) {
        v.push(PathBuf::from(d));
    }
    v
}

struct Env {
    editor: Editor,
    jobs: Rc<ManualJobSpawner>,
    events: Vec<EditorEvent>,
}

impl Env {
    fn new(cwd: &Path) -> Env {
        let ctx = PathCtx {
            cwd: cwd.to_string_lossy().into_owned(),
            home: None,
        };
        #[cfg(windows)]
        let paths: Arc<dyn eve_core::Paths> = Arc::new(eve_platform::dirs::WinPaths::new(
            Dirs::resolve(&|_| None),
            ctx,
        ));
        #[cfg(unix)]
        let paths: Arc<dyn eve_core::Paths> = Arc::new(eve_platform::dirs::UnixPaths::new(
            Dirs::resolve(&|_| None),
            ctx,
            None,
        ));
        let fs: Arc<dyn FileSystem> = Arc::new(NativeFs::default());
        let jobs = ManualJobSpawner::new(JobPorts {
            stdin: None,
            fs: fs.clone(),
            clock: Arc::new(FixedClock),
            paths: paths.clone(),
        });
        let ports = Ports {
            fs,
            clock: Arc::new(FixedClock),
            jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
            paths,
        };
        let mut editor = Editor::new(ports, Capabilities::default());
        editor.open_initial(None, false);
        Env {
            editor,
            jobs,
            events: Vec::new(),
        }
    }

    fn run_jobs(&mut self) {
        loop {
            self.jobs.run_jobs();
            let results = self.jobs.take_results();
            if results.is_empty() {
                break;
            }
            for r in results {
                self.editor.on_job_result(r);
            }
        }
        self.events.extend(self.editor.drain_events());
    }

    fn cmd(&mut self, id: &str, args: Args) {
        self.editor
            .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(args));
        self.run_jobs();
    }

    fn open(&mut self, path: &str) {
        self.cmd("file.open", Args::new().with_str("path", path));
    }

    fn save_as(&mut self, path: &str) {
        self.cmd("file.save_as", Args::new().with_str("path", path));
    }

    /// 開いているパスのある文書の数。
    fn file_docs(&self) -> usize {
        let Ok(eve_view::Answer::Documents(ds)) = self.editor.query(&eve_view::Query::Documents)
        else {
            panic!("文書の一覧");
        };
        ds.iter().filter(|d| d.path.is_some()).count()
    }

    fn active_path(&self) -> Option<String> {
        let st = self.editor.state();
        st.active_document()
            .and_then(|d| st.document(d))
            .and_then(|d| d.path.as_ref().map(FsPath::display))
    }

    fn has_msg(&self, key: &str) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e, EditorEvent::MessageShown { key: k, .. } if k == key))
    }
}

#[cfg(windows)]
fn symlink_file(target: &str, link: &str) -> bool {
    // 開発者モードか管理者でないと作れない。作れなければその試験を飛ばす
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[cfg(unix)]
fn symlink_file(target: &str, link: &str) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

// verifies: FR-103
#[cfg(windows)]
#[test]
fn 大文字小文字の違うパスで開くと_1_つの文書になる() {
    for base in bases() {
        let t = Tmp::new(&base, "case");
        let a = t.file("Case.txt", "alpha\n");
        let mut e = Env::new(&t.0);
        e.open(&a);
        assert_eq!(e.file_docs(), 1);
        e.open(&t.path("CASE.TXT"));
        assert_eq!(e.file_docs(), 1, "{}: 大文字小文字の違い", base.display());
        e.open(&t.path("case.txt"));
        assert_eq!(e.file_docs(), 1, "{}", base.display());
        assert!(e.has_msg("info.open.already_open"));
    }
}

#[test]
fn 識別を信頼するかは_fs_の種類で決まる() {
    // 一時ディレクトリ: Windows は NTFS で信頼する。`EVE_SAME_FILE_DIR` は、`EVE_SAME_FILE_UNTRUSTED=1` のとき信頼しない
    // FS (exFAT など) として確かめる
    let t = Tmp::new(&std::env::temp_dir(), "kind");
    let a = t.file("a.txt", "a");
    let m = NativeFs::default().metadata(&FsPath::new(&a)).unwrap();
    #[cfg(windows)]
    assert!(m.file_id_unique, "NTFS の一時ディレクトリ");
    #[cfg(unix)]
    assert_eq!(m.file_id_unique, m.fs_kind == eve_core::FsKind::Local);
    if let Some(dir) = std::env::var_os("EVE_SAME_FILE_DIR").filter(|d| !d.is_empty())
        && std::env::var("EVE_SAME_FILE_UNTRUSTED").as_deref() == Ok("1")
    {
        let t = Tmp::new(Path::new(&dir), "kind");
        let a = t.file("a.txt", "a");
        let m = NativeFs::default().metadata(&FsPath::new(&a)).unwrap();
        assert!(!m.file_id_unique, "{}: 識別を信頼しない FS", dir.display());
    }
}

// verifies: FR-103
#[test]
fn シンボリックリンクで開くと_1_つの文書になる() {
    for base in bases() {
        let t = Tmp::new(&base, "link");
        let a = t.file("a.txt", "alpha\n");
        let link = t.path("link.txt");
        if !symlink_file(&a, &link) {
            eprintln!("{}: シンボリックリンクを作れないので飛ばす", base.display());
            continue;
        }
        let mut e = Env::new(&t.0);
        e.open(&link);
        e.open(&a);
        assert_eq!(e.file_docs(), 1, "{}", base.display());
    }
}

// verifies: FR-103
#[test]
fn ハードリンクで開くと識別を信頼する_fs_では_1_つの文書になる() {
    for base in bases() {
        let t = Tmp::new(&base, "hard");
        let a = t.file("a.txt", "alpha\n");
        let hard = t.path("hard.txt");
        if std::fs::hard_link(&a, &hard).is_err() {
            eprintln!("{}: ハードリンクを作れないので飛ばす", base.display());
            continue;
        }
        let unique = NativeFs::default()
            .metadata(&FsPath::new(&a))
            .unwrap()
            .file_id_unique;
        let mut e = Env::new(&t.0);
        e.open(&a);
        e.open(&hard);
        let want = if unique { 1 } else { 2 };
        assert_eq!(
            e.file_docs(),
            want,
            "{}: file_id_unique = {unique}",
            base.display()
        );
    }
}

// verifies: FR-104
#[test]
fn 名前を付けて保存の保存先がリンクで今の文書なら上書き保存になる() {
    for base in bases() {
        let t = Tmp::new(&base, "saveas1");
        let a = t.file("a.txt", "alpha\n");
        let link = t.path("link.txt");
        if !symlink_file(&a, &link) {
            continue;
        }
        let mut e = Env::new(&t.0);
        e.open(&a);
        e.save_as(&link);
        assert_eq!(e.editor.state().overlay_kind(), "", "{}", base.display());
        assert_eq!(
            e.active_path().as_deref(),
            Some(a.as_str()),
            "パスを変えない"
        );
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "リンクは置き換えない"
        );
    }
}

// verifies: FR-104
#[cfg(windows)]
#[test]
fn 名前を付けて保存の保存先が大文字小文字違いのほかの文書なら拒む() {
    for base in bases() {
        let t = Tmp::new(&base, "saveas1b");
        let a = t.file("a.txt", "alpha\n");
        let b = t.file("b.txt", "beta\n");
        let mut e = Env::new(&t.0);
        e.open(&b);
        e.open(&a);
        e.save_as(&t.path("B.TXT"));
        assert!(e.has_msg("msg.save_as.open_in_other"), "{}", base.display());
        assert_eq!(e.editor.state().overlay_kind(), "prompt");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "beta\n", "書かない");
        // 今の文書と大文字小文字だけ違う名前は、今の文書への上書き保存 (03 §9.7 (a) の 1)
        e.cmd("ui.cancel", Args::new());
        e.save_as(&t.path("A.TXT"));
        assert_eq!(e.editor.state().overlay_kind(), "", "{}", base.display());
        assert_eq!(e.active_path().as_deref(), Some(a.as_str()));
    }
}

// verifies: FR-104
#[test]
fn 名前を付けて保存の保存先がハードリンクでほかの文書なら拒む() {
    for base in bases() {
        let t = Tmp::new(&base, "saveas1b-hard");
        let a = t.file("a.txt", "alpha\n");
        let b = t.file("b.txt", "beta\n");
        let hard = t.path("to-b.txt");
        if std::fs::hard_link(&b, &hard).is_err() {
            continue;
        }
        let unique = NativeFs::default()
            .metadata(&FsPath::new(&b))
            .unwrap()
            .file_id_unique;
        let mut e = Env::new(&t.0);
        e.open(&b);
        e.open(&a);
        e.save_as(&hard);
        if unique {
            assert!(e.has_msg("msg.save_as.open_in_other"), "{}", base.display());
            assert_eq!(std::fs::read_to_string(&b).unwrap(), "beta\n", "書かない");
        } else {
            // 識別を信頼しない FS では同じとは分からない: 既存のファイルとして上書きを確かめる (黙って書かない)
            assert_eq!(
                e.editor.state().overlay_kind(),
                "dialog",
                "{}",
                base.display()
            );
            assert_eq!(std::fs::read_to_string(&b).unwrap(), "beta\n");
        }
    }
}
