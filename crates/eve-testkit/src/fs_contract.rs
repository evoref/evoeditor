//! `FileSystem` Port の契約試験 (docs/02 §4.5 の「既定の実装」の注、docs/review/v0.3-findings.md の STR-D3 / STR-T4)。
//!
//! MemFs と実 FS (`WinFs` / `UnixFs`) に同じ検査を掛け、仕様 (02 §4.5、05 §2.1 / §4.1 / §4.5 / §5.4) が定める観察できる振る舞いを
//! 揃える。trait の既定の実装 (空の一覧・`Unsupported`・`Other`) を上書きし忘れた実装は、書く操作の検査で落ちる。
//!
//! 使い方: `suite(|| Subject::new(fs, root).caps(..).lister(..).keep(guard))`。検査ごとに `make` を呼んで新しい対象を作る。
//! `root` は空の既存ディレクトリ。検査は trait の操作だけで準備と観察をする (保存先のファイルも `write_atomic` で作る)。
//! 実装が意図して仕様と違う点は `Caps` の名前の付いたフラグで飛ばす (黙って飛ばさない)。

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use eve_core::{
    CancelToken, ChunkError, DocSnapshot, Expect, FileKind, FileStamp, FileSystem, FsError,
    FsErrorDetail, FsErrorKind, FsPath, ReadChunks, SaveMethod, SaveOptions, SaveReport,
    TextFormat,
};

use crate::memfs::BytesSource;

/// 実装の能力と、既知の仕様との差。
#[derive(Clone, Copy, Debug)]
pub struct Caps {
    /// 状態ディレクトリを持つ (状態ファイル・スワップ・直接上書き・退避先に保存を確かめる)。`false` なら、状態ディレクトリを
    /// 使う操作が `Unsupported` を返すことを確かめる (02 §4.5、`FileSystem::read_state_file` などの注)。
    pub state_dir: bool,
    /// `write_recovery` で書いた退避が `list_recovery_dirs` に `rescue = true` で現れる (02 §4.5)。MemFs はテストが積んだ
    /// ものだけを返すので `false` (既知の差)。
    pub recovery_listing_reflects_writes: bool,
}

impl Default for Caps {
    fn default() -> Caps {
        Caps {
            state_dir: true,
            recovery_listing_reflects_writes: true,
        }
    }
}

/// ディレクトリの直下の名前を返す関数。
type Lister = Box<dyn Fn(&FsPath) -> Vec<String>>;

/// 検査の対象 1 つ (検査ごとに新しく作る)。
pub struct Subject {
    pub fs: Arc<dyn FileSystem>,
    /// 空の既存ディレクトリ。
    pub root: FsPath,
    pub caps: Caps,
    /// ディレクトリの直下の名前 (一時ファイルの残りの検査に使う)。無ければ残りを確かめない。
    lister: Option<Lister>,
    /// 検査の後に落とすもの (一時ディレクトリの後片付けなど)。
    _keep: Option<Box<dyn Any>>,
}

impl Subject {
    pub fn new(fs: Arc<dyn FileSystem>, root: FsPath) -> Subject {
        Subject {
            fs,
            root,
            caps: Caps::default(),
            lister: None,
            _keep: None,
        }
    }

    #[must_use]
    pub fn caps(mut self, caps: Caps) -> Subject {
        self.caps = caps;
        self
    }

    #[must_use]
    pub fn lister(mut self, f: impl Fn(&FsPath) -> Vec<String> + 'static) -> Subject {
        self.lister = Some(Box::new(f));
        self
    }

    #[must_use]
    pub fn keep(mut self, guard: impl Any) -> Subject {
        self._keep = Some(Box::new(guard));
        self
    }

    fn p(&self, name: &str) -> FsPath {
        self.root.join(name)
    }

    fn names(&self) -> Option<Vec<String>> {
        self.lister.as_ref().map(|f| {
            let mut v = f(&self.root);
            v.sort();
            v
        })
    }
}

type Check = fn(&Subject);

/// 検査の一覧 (名前と本体)。
pub const CHECKS: &[(&str, Check)] = &[
    (
        "metadata_はファイルとディレクトリを区別する",
        metadata_kinds,
    ),
    (
        "無いパスは_notfound_で親の有無を区別する",
        not_found_and_parent_missing,
    ),
    ("書いた内容をチャンクで読み返せる", read_after_write),
    ("ディレクトリは読めない", open_read_dir),
    ("canonicalize_は同じファイルを指す", canonicalize_same_file),
    ("absent_で新規に作る", absent_creates_new_file),
    (
        "absent_で保存先が在れば_conflict",
        absent_conflicts_with_existing,
    ),
    ("stamp_が合えば置換する", stamp_replaces),
    ("stamp_が合わなければ_conflict", stamp_mismatch_conflicts),
    (
        "stamp_で保存先が無ければ_conflict_none",
        stamp_without_target_conflicts,
    ),
    ("any_は照合しない", any_skips_check),
    (
        "親フォルダが無ければ作らずに_parent_missing",
        save_parent_missing,
    ),
    (
        "供給側の失敗は_sourcefailed_で元のまま",
        source_failure_keeps_original,
    ),
    (
        "保存先がディレクトリなら_invalidpath",
        save_to_dir_is_invalid,
    ),
    ("rename_no_replace_は置き換えない", rename_no_replace),
    ("remove_file_は消し_無ければ_notfound", remove_file),
    ("保存の後に保存中フラグが残らない", no_inflight_after_save),
    ("直接上書きは同じファイルを書き換える", write_in_place),
    ("状態ファイルを読み書きし退ける", state_files),
    ("スワップを書いて消す", swap),
    ("退避先に保存はディレクトリを返す", write_recovery),
];

/// すべての検査を掛ける。落ちた検査の名前をまとめて panic する。
pub fn suite(make: impl Fn() -> Subject) {
    let failed = failures(make);
    assert!(failed.is_empty(), "契約試験で落ちた検査: {failed:?}");
}

/// すべての検査を掛け、落ちた検査の名前を返す (検査が既定の実装を見逃さないことの試験用)。
pub fn failures(make: impl Fn() -> Subject) -> Vec<&'static str> {
    let mut failed = Vec::new();
    for (name, check) in CHECKS {
        let s = make();
        if catch_unwind(AssertUnwindSafe(|| check(&s))).is_err() {
            failed.push(*name);
        }
    }
    failed
}

// ---------- 補助 ----------

fn opts(expected: Expect) -> SaveOptions {
    SaveOptions::user_file(expected)
}

fn save(s: &Subject, path: &FsPath, body: &[u8], expected: Expect) -> Result<SaveReport, FsError> {
    let mut src = BytesSource::new(body, 4096);
    s.fs.write_atomic(path, &mut src, &opts(expected))
}

fn create(s: &Subject, name: &str, body: &[u8]) -> FsPath {
    let p = s.p(name);
    save(s, &p, body, Expect::Absent).expect("準備のファイルを作れない");
    p
}

fn read_all(s: &Subject, path: &FsPath) -> Vec<u8> {
    let mut r = s.fs.open_read(path).expect("open_read");
    let mut out = Vec::new();
    while let Some(c) = r.next_chunk().expect("next_chunk") {
        out.extend_from_slice(c);
    }
    out
}

fn stamp(s: &Subject, path: &FsPath) -> FileStamp {
    FileStamp::from_meta(&s.fs.metadata(path).expect("metadata"), None)
}

fn kind_detail(e: &FsError) -> (FsErrorKind, Option<FsErrorDetail>) {
    (e.kind.clone(), e.detail.clone())
}

/// 書込の前後で root の直下の名前が同じ (一時ファイルを残さない、05 §4.1 手順 8)。
fn assert_no_leftovers(s: &Subject, before: Option<Vec<String>>) {
    if let (Some(before), Some(after)) = (before, s.names()) {
        assert_eq!(after, before, "一時ファイルを残さない");
    }
}

fn doc(path: Option<FsPath>) -> DocSnapshot {
    DocSnapshot {
        doc: eve_view::DocumentId(9),
        snapshot: eve_text::Text::from("契約\n").snapshot(),
        format: TextFormat::new_document(),
        revision: eve_view::Revision(1),
        saved_node_exists: false,
        sensitive: false,
        untitled_name: path.is_none().then(|| "無題-9".to_owned()),
        path,
    }
}

// ---------- 読み取り (02 §4.5 の metadata / open_read / canonicalize) ----------

// 02 §4.5 FileMeta.kind、05 §2.1 の metadata
fn metadata_kinds(s: &Subject) {
    let p = create(s, "a.txt", b"abc");
    let m = s.fs.metadata(&p).unwrap();
    assert_eq!((m.kind, m.len), (FileKind::File, 3));
    assert_eq!(s.fs.metadata(&s.root).unwrap().kind, FileKind::Dir);
    // 05 §5.4: 同じファイルは同じ識別
    assert_eq!(s.fs.metadata(&p).unwrap().file_id, m.file_id);
}

// 02 §4.5 metadata / open_read の注: 親が在れば NotFound (detail なし)、親も無ければ NotFound + ParentMissing { dir }
fn not_found_and_parent_missing(s: &Subject) {
    let none = s.p("none.txt");
    let deep = s.root.join("nodir").join("x.txt");
    let missing = Some(FsErrorDetail::ParentMissing { dir: s.p("nodir") });
    let e = s.fs.metadata(&none).unwrap_err();
    assert_eq!(kind_detail(&e), (FsErrorKind::NotFound, None), "metadata");
    let e = s.fs.metadata(&deep).unwrap_err();
    assert_eq!(kind_detail(&e), (FsErrorKind::NotFound, missing.clone()));
    let e = s.fs.open_read(&none).map(|_| ()).unwrap_err();
    assert_eq!(kind_detail(&e), (FsErrorKind::NotFound, None), "open_read");
    let e = s.fs.open_read(&deep).map(|_| ()).unwrap_err();
    assert_eq!(kind_detail(&e), (FsErrorKind::NotFound, missing));
}

// 02 §4.5 open_read (チャンク読込、既定 1 MiB)、05 §4.5
fn read_after_write(s: &Subject) {
    let empty = create(s, "empty.txt", b"");
    assert!(
        s.fs.open_read(&empty)
            .unwrap()
            .next_chunk()
            .unwrap()
            .is_none()
    );
    assert_eq!(s.fs.metadata(&empty).unwrap().len, 0);

    let text = "こんにちは\r\n世界\n".as_bytes();
    let p = create(s, "t.txt", text);
    assert_eq!(read_all(s, &p), text);

    let big: Vec<u8> = (0..(5 << 19) + 7).map(|i| (i % 251) as u8).collect();
    let p = create(s, "big.bin", &big);
    let mut r = s.fs.open_read(&p).unwrap();
    let mut out = Vec::new();
    while let Some(c) = r.next_chunk().unwrap() {
        assert!(!c.is_empty() && c.len() <= 1 << 20, "チャンクは 1 MiB 以下");
        out.extend_from_slice(c);
    }
    assert_eq!(out, big);
    assert_eq!(s.fs.metadata(&p).unwrap().len, big.len() as u64);
}

// 05 §2.1 の写像の表 (ディレクトリは InvalidPath)、§4.5
fn open_read_dir(s: &Subject) {
    let e = s.fs.open_read(&s.root).map(|_| ()).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::InvalidPath);
}

// 05 §3.3 (canonicalize)、§5.4 (同一性の判定に使う)。無いパスは NotFound (05 §2.1)
fn canonicalize_same_file(s: &Subject) {
    let p = create(s, "c.txt", b"x");
    let c = s.fs.canonicalize(&p).unwrap();
    assert_eq!(c.file_name().as_deref(), Some("c.txt"));
    assert_eq!(
        s.fs.metadata(&c).unwrap().file_id,
        s.fs.metadata(&p).unwrap().file_id
    );
    let e = s.fs.canonicalize(&s.p("none.txt")).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::NotFound);
}

// ---------- 原子的保存 (02 §4.5 write_atomic、05 §4.1) ----------

// 02 §4.5 SaveMethod::NewFile、05 §4.1 手順 7 (new_stamp は事後の metadata)
fn absent_creates_new_file(s: &Subject) {
    let p = s.p("new.txt");
    let r = save(s, &p, b"hello", Expect::Absent).unwrap();
    assert_eq!(r.method, SaveMethod::NewFile);
    assert_eq!(read_all(s, &p), b"hello");
    assert_eq!(r.new_stamp, stamp(s, &p), "報告の Stamp は実際と一致");
}

// 05 §4.1 手順 0b: Absent で T が在れば Conflict { Some(T の Stamp) }。何も書かない
fn absent_conflicts_with_existing(s: &Subject) {
    let p = create(s, "a.txt", b"orig");
    let before = stamp(s, &p);
    let e = save(s, &p, b"mine", Expect::Absent).unwrap_err();
    assert_eq!(
        e.kind,
        FsErrorKind::Conflict {
            current: Some(before)
        }
    );
    assert_eq!(read_all(s, &p), b"orig");
    assert_eq!(stamp(s, &p), before);
}

// 02 §4.5 SaveMethod::Atomic、05 §4.1 手順 7、§5.4 (置換で FileId は変わる)
fn stamp_replaces(s: &Subject) {
    let p = create(s, "a.txt", b"v1");
    let before = stamp(s, &p);
    let r = save(s, &p, b"version 2", Expect::Stamp(before)).unwrap();
    assert_eq!(r.method, SaveMethod::Atomic);
    assert_eq!(read_all(s, &p), b"version 2");
    assert_eq!(r.new_stamp, stamp(s, &p));
    if let (Some(a), Some(b)) = (before.file_id, r.new_stamp.file_id) {
        assert_ne!(a, b, "置換の後は新しいファイル");
    }
    // 報告の Stamp で続けて保存できる
    save(s, &p, b"v3", Expect::Stamp(r.new_stamp)).unwrap();
    assert_eq!(read_all(s, &p), b"v3");
}

// 05 §4.1 手順 0b / §5.5: 不一致なら Conflict { Some(T の Stamp) }。何も書かない
fn stamp_mismatch_conflicts(s: &Subject) {
    let p = create(s, "a.txt", b"orig");
    let have = stamp(s, &p);
    let mut want = have;
    want.len += 1;
    let e = save(s, &p, b"mine", Expect::Stamp(want)).unwrap_err();
    assert_eq!(
        e.kind,
        FsErrorKind::Conflict {
            current: Some(have)
        }
    );
    assert_eq!(read_all(s, &p), b"orig");
    assert_eq!(stamp(s, &p), have);
}

// 05 §4.1 手順 0b: Stamp を期待していて T が無ければ NotFound ではなく Conflict { None }
fn stamp_without_target_conflicts(s: &Subject) {
    let p = create(s, "a.txt", b"orig");
    let st = stamp(s, &p);
    s.fs.remove_file(&p).unwrap();
    let e = save(s, &p, b"mine", Expect::Stamp(st)).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::Conflict { current: None });
    assert_eq!(s.fs.metadata(&p).unwrap_err().kind, FsErrorKind::NotFound);
}

// 02 §4.5 Expect::Any (照合しない)、05 §2.1 SaveReport.method
fn any_skips_check(s: &Subject) {
    let p = create(s, "a.txt", b"orig");
    let r = save(s, &p, b"any", Expect::Any).unwrap();
    assert_eq!(r.method, SaveMethod::Atomic);
    assert_eq!(read_all(s, &p), b"any");
    let q = s.p("b.txt");
    let r = save(s, &q, b"new", Expect::Any).unwrap();
    assert_eq!(r.method, SaveMethod::NewFile);
    assert_eq!(read_all(s, &q), b"new");
}

// 05 §4.1 手順 0b の末尾と「新規ファイル」: T も親も無ければ Expect に依らず NotFound + ParentMissing。フォルダは作らない
fn save_parent_missing(s: &Subject) {
    let dir = s.p("nodir");
    let p = dir.join("x.txt");
    let fake = FileStamp {
        mtime: eve_core::FileTime::default(),
        len: 1,
        file_id: None,
        hash: None,
    };
    for expected in [Expect::Absent, Expect::Any, Expect::Stamp(fake)] {
        let e = save(s, &p, b"x", expected).unwrap_err();
        assert_eq!(
            kind_detail(&e),
            (
                FsErrorKind::NotFound,
                Some(FsErrorDetail::ParentMissing { dir: dir.clone() })
            ),
            "{expected:?}"
        );
        assert_eq!(s.fs.metadata(&dir).unwrap_err().kind, FsErrorKind::NotFound);
    }
}

// 02 §4.5 write_atomic の注 (data の失敗で止め、一時ファイルを消して SourceFailed)、05 §4.1 手順 8
fn source_failure_keeps_original(s: &Subject) {
    let p = create(s, "a.txt", b"orig");
    let before = stamp(s, &p);
    let names = s.names();
    let q = s.p("n.txt");
    for how in [How::Unmappable, How::Aborted, How::OutOfMemory, How::Io] {
        // `Io` (読込側の変種) を保存の供給側が返したら、中の `FsError` をそのまま返す (02 §4.5 の write_atomic の注)
        let want = if how == How::Io {
            FsErrorKind::Other
        } else {
            FsErrorKind::SourceFailed
        };
        let e =
            s.fs.write_atomic(
                &p,
                &mut FailingSource::new(how),
                &opts(Expect::Stamp(before)),
            )
            .unwrap_err();
        assert_eq!(e.kind, want, "{how:?}");
        assert_eq!(read_all(s, &p), b"orig");
        assert_eq!(stamp(s, &p), before);
        // 新規ファイルは作らない
        let e =
            s.fs.write_atomic(&q, &mut FailingSource::new(how), &opts(Expect::Absent))
                .unwrap_err();
        assert_eq!(e.kind, want, "{how:?}");
        assert_eq!(s.fs.metadata(&q).unwrap_err().kind, FsErrorKind::NotFound);
    }
    assert_no_leftovers(s, names);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum How {
    Unmappable,
    Aborted,
    OutOfMemory,
    Io,
}

/// 1 チャンクを返した後に `how` で失敗する供給側。
struct FailingSource {
    how: How,
    sent: bool,
}

impl FailingSource {
    fn new(how: How) -> FailingSource {
        FailingSource { how, sent: false }
    }
}

impl ReadChunks for FailingSource {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        if !self.sent {
            self.sent = true;
            return Ok(Some(b"0123"));
        }
        Err(match self.how {
            How::Unmappable => ChunkError::Unmappable,
            How::Aborted => ChunkError::Aborted,
            How::OutOfMemory => ChunkError::OutOfMemory { requested: 1 << 40 },
            How::Io => ChunkError::Io(FsError::new(FsErrorKind::Other)),
        })
    }
}

// 05 §2.1 の写像の表 (ディレクトリは InvalidPath)
fn save_to_dir_is_invalid(s: &Subject) {
    let e = save(s, &s.root, b"x", Expect::Any).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::InvalidPath);
    assert_eq!(s.fs.metadata(&s.root).unwrap().kind, FileKind::Dir);
}

// ---------- 名前の操作 (02 §4.5 rename_no_replace / remove_file、05 §4.6) ----------

// 02 §4.5: to が在れば失敗する (置き換えない)
fn rename_no_replace(s: &Subject) {
    let a = create(s, "a.txt", b"A");
    let b = create(s, "b.txt", b"B");
    assert!(s.fs.rename_no_replace(&a, &b).is_err(), "在れば失敗");
    assert_eq!(
        (read_all(s, &a), read_all(s, &b)),
        (b"A".to_vec(), b"B".to_vec())
    );
    let c = s.p("c.txt");
    s.fs.rename_no_replace(&a, &c).unwrap();
    assert_eq!(read_all(s, &c), b"A");
    assert_eq!(s.fs.metadata(&a).unwrap_err().kind, FsErrorKind::NotFound);
    // 05 §2.1: 無い from は NotFound
    let e = s.fs.rename_no_replace(&a, &s.p("d.txt")).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::NotFound);
}

// eve-core の `FileSystem::remove_file` の注: 存在しなければ NotFound
fn remove_file(s: &Subject) {
    let p = create(s, "a.txt", b"x");
    s.fs.remove_file(&p).unwrap();
    assert_eq!(s.fs.metadata(&p).unwrap_err().kind, FsErrorKind::NotFound);
    assert_eq!(
        s.fs.remove_file(&p).unwrap_err().kind,
        FsErrorKind::NotFound
    );
}

// 05 §4.1 手順 7 (成功の後にフラグを消す)、02 §4.5 inflight_for / list_inflight (自分のプロセスのものは除く)
fn no_inflight_after_save(s: &Subject) {
    let p = create(s, "a.txt", b"v1");
    save(s, &p, b"v2", Expect::Stamp(stamp(s, &p))).unwrap();
    assert!(s.fs.inflight_for(&p).unwrap().is_empty());
    assert!(!s.fs.list_inflight().unwrap().iter().any(|r| r.target == p));
}

// ---------- 状態ディレクトリを使う操作 ----------

// 02 §4.5 write_in_place、05 §4.1 手順 7 / §4.4 (直接上書きでは FileId は変わらない)
fn write_in_place(s: &Subject) {
    if !s.caps.state_dir {
        return;
    }
    let p = create(s, "a.txt", b"orig");
    let before = stamp(s, &p);
    let names = s.names();
    let body = b"in place body";
    let mut src = BytesSource::new(body, 4);
    let r =
        s.fs.write_in_place(
            &p,
            &mut src,
            body.len() as u64,
            &opts(Expect::Stamp(before)),
            &CancelToken::new(),
        )
        .unwrap();
    assert_eq!(r.method, SaveMethod::InPlace);
    assert_eq!(read_all(s, &p), body);
    assert_eq!(r.new_stamp, stamp(s, &p));
    assert_eq!(r.new_stamp.file_id, before.file_id, "同じファイル");
    assert_no_leftovers(s, names);

    // 照合が合わなければ Conflict で何も書かない (02 §4.5 の「直接上書き」(1))
    let mut src = BytesSource::new(b"x", 4);
    let e =
        s.fs.write_in_place(
            &p,
            &mut src,
            1,
            &opts(Expect::Stamp(before)),
            &CancelToken::new(),
        )
        .unwrap_err();
    assert!(
        matches!(e.kind, FsErrorKind::Conflict { .. }),
        "{:?}",
        e.kind
    );
    assert_eq!(read_all(s, &p), body);
}

// 02 §4.5 状態ファイル、eve-core の注 (無ければ NotFound、state_dir を使えなければ Unsupported)、05 §7.1 / §7.5 (a)
fn state_files(s: &Subject) {
    let name = "contract_state.toml";
    if !s.caps.state_dir {
        for e in [
            s.fs.read_state_file(name).unwrap_err(),
            s.fs.write_state_file(name, b"x").unwrap_err(),
            s.fs.set_aside_state_file(name).unwrap_err(),
        ] {
            assert_eq!(e.kind, FsErrorKind::Unsupported);
        }
        return;
    }
    assert_eq!(
        s.fs.read_state_file(name).unwrap_err().kind,
        FsErrorKind::NotFound
    );
    s.fs.write_state_file(name, b"v = 1\n").unwrap();
    assert_eq!(s.fs.read_state_file(name).unwrap(), b"v = 1\n");
    s.fs.write_state_file(name, b"v = 2\n").unwrap();
    assert_eq!(
        s.fs.read_state_file(name).unwrap(),
        b"v = 2\n",
        "最終書込勝ち"
    );
    // 退けたら元の名前では読めない (消さずに別名へ移す)
    s.fs.set_aside_state_file(name).unwrap();
    assert_eq!(
        s.fs.read_state_file(name).unwrap_err().kind,
        FsErrorKind::NotFound
    );
}

// 02 §4.5 スワップ (実装していない Port は Unsupported)、eve-core の注 (discard は無ければ成功、list は自分のものを含めない)
fn swap(s: &Subject) {
    let d = doc(Some(s.p("doc.txt")));
    if !s.caps.state_dir {
        assert_eq!(
            s.fs.write_swap(&d).unwrap_err().kind,
            FsErrorKind::Unsupported
        );
        return;
    }
    s.fs.write_swap(&d).unwrap();
    s.fs.write_swap(&d).unwrap();
    let me = std::process::id();
    assert!(!s.fs.list_swaps().unwrap().iter().any(|e| e.pid == me));
    s.fs.discard_swap(d.doc, d.path.as_ref()).unwrap();
    s.fs.discard_swap(d.doc, d.path.as_ref()).unwrap();
    let u = doc(None);
    s.fs.write_swap(&u).unwrap();
    s.fs.discard_swap(u.doc, None).unwrap();
}

// 02 §4.5 write_recovery (書いたディレクトリを返す) / list_recovery_dirs (rescue/ 直下を rescue = true で返す)
fn write_recovery(s: &Subject) {
    if !s.caps.state_dir {
        return;
    }
    let dir = s.fs.write_recovery(&doc(Some(s.p("doc.txt")))).unwrap();
    assert_eq!(s.fs.metadata(&dir).unwrap().kind, FileKind::Dir);
    if s.caps.recovery_listing_reflects_writes {
        let list = s.fs.list_recovery_dirs().unwrap();
        assert!(
            list.iter().any(|e| e.path == dir && e.rescue),
            "書いた退避が一覧に現れる"
        );
    }
}
