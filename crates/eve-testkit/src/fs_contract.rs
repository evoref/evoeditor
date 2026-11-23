//! `FileSystem` Port の契約試験 (docs/02 §4.5 の「既定の実装」の注、docs/review/v0.3-findings.md の STR-D3 / STR-T4)。
//!
//! `MemFs` と実 FS (`WinFs` / `UnixFs`) に同じ検査を掛け、仕様 (02 §4.5、05 §2.1 / §4.1 / §4.5 / §5.4) が定める観察できる振る舞いを
//! 揃える。trait の既定の実装 (空の一覧・`Unsupported`・`Other`) を上書きし忘れた実装は、書く操作の検査で落ちる。
//!
//! 使い方: `suite(|| Subject::new(fs, root).caps(..).lister(..).fixture(..).keep(guard))`。検査ごとに `make` を呼んで新しい対象を
//! 作る。`root` は空の既存ディレクトリ。検査は trait の操作だけで準備と観察をする (保存先のファイルも `write_atomic` で作る)。
//! ただし `read_dir` の検査 (v0.5 の M31、03 §12.7.8) は、trait に無い準備 (ディレクトリ・リンク・属性・FIFO) を `Fixture` で行う。
//! 実装が意図して仕様と違う点は `Caps` の名前の付いたフラグで飛ばす (黙って飛ばさない)。OS や環境に無いもの (Unix のオフラインの
//! 属性、Windows の FIFO、権限の無い開発機のシンボリックリンク) は `Fixture` が `false` を返し、その項目だけを確かめない。

use std::any::Any;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use eve_core::{
    CancelToken, CheckedObject, CheckedRead, ChunkError, DirEntry, DirLinks, DocSnapshot,
    EntryKind, Expect, FileKind, FileStamp, FileSystem, FsError, FsErrorDetail, FsErrorKind,
    FsPath, OwnerVerdict, ReadChunks, SaveMethod, SaveOptions, SaveReport, TextFormat,
    WORKSPACE_CONFIG_DIR,
};

use crate::memfs::{BytesSource, MemFs};

/// 実装の能力と、既知の仕様との差。
#[derive(Clone, Copy, Debug)]
pub struct Caps {
    /// 状態ディレクトリを持つ (状態ファイル・スワップ・直接上書き・退避先に保存を確かめる)。`false` なら、状態ディレクトリを
    /// 使う操作が `Unsupported` を返すことを確かめる (02 §4.5、`FileSystem::read_state_file` などの注)。
    pub state_dir: bool,
    /// `write_recovery` で書いた退避が `list_recovery_dirs` に `rescue = true` で現れる (02 §4.5)。MemFs はテストが積んだ
    /// ものだけを返すので `false` (既知の差)。
    pub recovery_listing_reflects_writes: bool,
    /// `read_dir` がファイルの大きさ (`DirEntry::len`) を返す (05 §2.1。Windows の実 FS と `MemFs` は `true`、Unix は
    /// `stat` をしないので `false`)。`true` なら正しい値を、`false` なら `None` を求める。
    pub read_dir_len: bool,
    /// `read_checked` が所有者を判定する (Windows・Linux の実 FS と `MemFs`)。`false` (Linux 以外の Unix。どれも
    /// `Unknown { unsupported }`) なら、読めること・他人の書込を判定することを求める `read_checked` の検査を飛ばす。
    pub owner_check: bool,
}

impl Default for Caps {
    fn default() -> Caps {
        Caps {
            state_dir: true,
            recovery_listing_reflects_writes: true,
            read_dir_len: false,
            owner_check: true,
        }
    }
}

/// ディレクトリの直下の名前を返す関数。
type Lister = Box<dyn Fn(&FsPath) -> Vec<String>>;

/// `read_dir` の検査の準備に使う、trait に無い操作。実 FS は OS の API で、`MemFs` は準備の口で作る。
/// 作れないもの (OS に無い・権限が無い) は `false` / `None` を返し、検査はその項目だけを確かめない。
/// 後片付け (列挙を拒んだ権限を戻すなど) は実装が `Drop` で行う (対象の一時ディレクトリより先に落ちる)。
pub trait Fixture {
    /// ディレクトリを作る (祖先も作る)。
    fn dir(&self, path: &FsPath);
    /// ファイルを作る。名前は変えない (Windows の末尾のドット・空白、予約名 `con.txt` もそのまま作る)。
    fn file(&self, path: &FsPath, data: &[u8]);
    /// 隠しファイル (Windows は隠し属性を付ける。ほかは `file` と同じ)。
    fn hidden_file(&self, path: &FsPath) {
        self.file(path, b"");
    }
    /// 呼び戻しの属性 (オフライン) を持つファイルを作る。属性の無い FS は `false`。
    fn offline_file(&self, _path: &FsPath) -> bool {
        false
    }
    /// `link` に `target` を指すシンボリックリンクを作る (`kind` はリンク先の種類。Windows はファイルとディレクトリで作り方が違う)。
    /// 作れない環境 (権限の無い Windows の開発機) は `false`。
    fn symlink(&self, link: &FsPath, target: &FsPath, kind: FileKind) -> bool;
    /// ジャンクション (Windows)。無い OS は `false`。
    fn junction(&self, _link: &FsPath, _target: &FsPath) -> bool {
        false
    }
    /// FIFO などの、通常のファイルでもディレクトリでもない項目を作る。作れない OS は `false`。
    fn special(&self, _path: &FsPath) -> bool {
        false
    }
    /// `dir` の中で、開くと止まり得る通常でないファイル (FIFO・デバイス) のパス。無ければ `None`。
    fn device_in(&self, _dir: &FsPath) -> Option<FsPath> {
        None
    }
    /// `dir` を列挙できなくする。できない (Unix の root など) なら `false`。
    fn deny_list(&self, _dir: &FsPath) -> bool {
        false
    }
    /// `dir` の列挙を、`after` 個の項目を返した後に失敗させる (SMB の切断などの再現)。実 FS では作れないので `false`。
    fn fail_listing_after(&self, _dir: &FsPath, _after: usize) -> bool {
        false
    }
    /// `path` (ディレクトリかファイル) に他人の書込を許す (`read_checked` の検査の準備。Unix は `o+w`、Windows は
    /// `Everyone` に変更の許可。`MemFs` は偽の所有者の印)。できなければ `false`。
    fn others_can_write(&self, _path: &FsPath) -> bool {
        false
    }
}

/// `MemFs` の準備の口で作る `Fixture`。どの項目も作れる (リンク・FIFO の代わり・オフライン・列挙の拒否)。
pub struct MemFixture(pub Arc<MemFs>);

fn mem_str(path: &FsPath) -> &str {
    path.as_path()
        .to_str()
        .expect("MemFs の準備のパスは UTF-8 (UTF-8 でない名前はファイルだけ)")
}

impl Fixture for MemFixture {
    fn dir(&self, path: &FsPath) {
        self.0.add_dir(mem_str(path));
    }
    fn file(&self, path: &FsPath, data: &[u8]) {
        self.0.set_file(path, data);
    }
    fn offline_file(&self, path: &FsPath) -> bool {
        self.0.set_file(path, b"");
        self.0.set_offline(mem_str(path), true);
        true
    }
    fn symlink(&self, link: &FsPath, target: &FsPath, _kind: FileKind) -> bool {
        self.0.add_link(mem_str(link), mem_str(target));
        true
    }
    fn special(&self, path: &FsPath) -> bool {
        self.0.add_special(mem_str(path));
        true
    }
    fn device_in(&self, dir: &FsPath) -> Option<FsPath> {
        let p = dir.join("device");
        self.0.add_special(mem_str(&p));
        Some(p)
    }
    fn deny_list(&self, dir: &FsPath) -> bool {
        self.0.set_unreadable(mem_str(dir), true);
        true
    }
    fn fail_listing_after(&self, dir: &FsPath, after: usize) -> bool {
        self.0.set_read_dir_fail_after(mem_str(dir), Some(after));
        true
    }
    fn others_can_write(&self, path: &FsPath) -> bool {
        self.0.set_owner_verdict(
            mem_str(path),
            Some(OwnerVerdict::OthersCanWrite {
                who: vec!["others".to_owned()],
            }),
        );
        true
    }
}

/// 検査の対象 1 つ (検査ごとに新しく作る)。
pub struct Subject {
    /// 検査する `FileSystem` の実装。
    pub fs: Arc<dyn FileSystem>,
    /// 空の既存ディレクトリ。
    pub root: FsPath,
    /// 実装の能力と既知の仕様との差。
    pub caps: Caps,
    /// ディレクトリの直下の名前 (一時ファイルの残りの検査に使う)。無ければ残りを確かめない。
    lister: Option<Lister>,
    /// `read_dir` の検査の準備。`_keep` より先に落とす (権限を戻してから一時ディレクトリを消す)。
    fixture: Option<Box<dyn Fixture>>,
    /// 検査の後に落とすもの (一時ディレクトリの後片付けなど)。
    _keep: Option<Box<dyn Any>>,
}

impl Subject {
    /// `root` (空の既存ディレクトリ) の下で `fs` を検査する対象を作る。能力は `Caps::default()`。
    pub fn new(fs: Arc<dyn FileSystem>, root: FsPath) -> Subject {
        Subject {
            fs,
            root,
            caps: Caps::default(),
            lister: None,
            fixture: None,
            _keep: None,
        }
    }

    /// 能力と既知の差を決める。
    #[must_use]
    pub fn caps(mut self, caps: Caps) -> Subject {
        self.caps = caps;
        self
    }

    /// ディレクトリの直下の名前を返す関数を渡す (一時ファイルが残らないことの検査に使う。無ければ確かめない)。
    #[must_use]
    pub fn lister(mut self, f: impl Fn(&FsPath) -> Vec<String> + 'static) -> Subject {
        self.lister = Some(Box::new(f));
        self
    }

    /// `read_dir` の検査の準備を渡す (無ければ `read_dir` の検査は落ちる)。
    #[must_use]
    pub fn fixture(mut self, f: impl Fixture + 'static) -> Subject {
        self.fixture = Some(Box::new(f));
        self
    }

    /// 検査の後まで持っておくもの (一時ディレクトリの後片付けなど)。対象と一緒に落とす。
    #[must_use]
    pub fn keep(mut self, guard: impl Any) -> Subject {
        self._keep = Some(Box::new(guard));
        self
    }

    fn p(&self, name: &str) -> FsPath {
        self.root.join(name)
    }

    fn fx(&self) -> &dyn Fixture {
        self.fixture
            .as_deref()
            .expect("read_dir の検査には Subject::fixture が要る")
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
    ("read_dir_は空のディレクトリで空", read_dir_empty),
    ("read_dir_は種類を返しリンクを辿らない", read_dir_kinds),
    ("read_dir_は名前を変えない", read_dir_names),
    ("read_dir_は長いパスを読む", read_dir_long_path),
    ("read_dir_はオフラインの属性を返す", read_dir_offline),
    ("read_dir_の失敗の種類", read_dir_errors),
    ("read_dir_は_max_entries_で打ち切る", read_dir_max_entries),
    (
        "read_dir_は列挙の途中の失敗を_incomplete_で返す",
        read_dir_incomplete,
    ),
    (
        "open_read_は通常でないファイルで止まらずに_invalidpath",
        open_read_not_regular,
    ),
    (
        "read_checked_は判定を通ったファイルを読む",
        read_checked_reads,
    ),
    (
        "read_checked_は無いファイルを_missing_で返す",
        read_checked_missing,
    ),
    (
        "read_checked_はリンクとふつうでないファイルを読まない",
        read_checked_not_regular,
    ),
    (
        "read_checked_は他人が書けるディレクトリとファイルを読まない",
        read_checked_others_write,
    ),
    (
        "read_checked_は_1_つの名前だけを受け付ける",
        read_checked_invalid_name,
    ),
];

/// すべての検査を掛ける。落ちた検査の名前をまとめて panic する。
///
/// # Panics
///
/// 1 つでも検査が落ちたとき。
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

    let big: Vec<u8> = (0..(5 << 19) + 7)
        .map(|i| u8::try_from(i % 251).expect("251 未満"))
        .collect();
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
            current: Some(Box::new(before))
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
            current: Some(Box::new(have))
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
    let actual = s.fs.inflight_for(&p).unwrap();
    assert!(actual.is_empty(), "{actual:?}");
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

// ---------- ディレクトリの列挙 (02 §4.5 read_dir、05 §2.1、03 §12.7.8。v0.5 の M31) ----------

/// UTF-8 で表せない名前 (Unix) / 対にならないサロゲートを含む名前 (Windows)。
fn odd_name() -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(b"odd-\xff-name.txt".to_vec())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut w: Vec<u16> = "odd-".encode_utf16().collect();
        w.push(0xD800);
        w.extend(".txt".encode_utf16());
        OsString::from_wide(&w)
    }
}

/// 打ち切らずに全部を列挙し、名前で引ける形にする。名前の重複・`.`・`..` が無いことも確かめる。
fn listing(s: &Subject, dir: &FsPath) -> BTreeMap<OsString, DirEntry> {
    let l = s.fs.read_dir(dir, usize::MAX).expect("read_dir");
    assert!(!l.truncated, "上限に達していないのに打ち切った");
    assert!(!l.incomplete, "列挙は途中で失敗していない");
    let n = l.entries.len();
    let map: BTreeMap<OsString, DirEntry> =
        l.entries.into_iter().map(|e| (e.name.clone(), e)).collect();
    assert_eq!(map.len(), n, "同じ名前を 2 度返した");
    for dot in [".", ".."] {
        assert!(!map.contains_key(&OsString::from(dot)), "{dot} を返した");
    }
    map
}

fn kind_of(map: &BTreeMap<OsString, DirEntry>, name: &str) -> EntryKind {
    map.get(&OsString::from(name))
        .unwrap_or_else(|| panic!("{name} が無い: {:?}", map.keys().collect::<Vec<_>>()))
        .kind
}

// 02 §4.5: 空のディレクトリは空の一覧 (既定の Unsupported と区別する)
fn read_dir_empty(s: &Subject) {
    let l = s.fs.read_dir(&s.root, 10).expect("read_dir");
    assert!(l.entries.is_empty() && !l.truncated, "{l:?}");
}

// 02 §4.5 EntryKind・DirEntry.len、05 §2.1 (リンクは辿らない。ジャンクションも Symlink。FIFO は Other)
fn read_dir_kinds(s: &Subject) {
    let fx = s.fx();
    fx.file(&s.p("a.txt"), b"abc");
    fx.dir(&s.p("sub"));
    fx.file(&s.p("sub").join("inner.txt"), b"x");
    let mut want = vec![("a.txt", EntryKind::File), ("sub", EntryKind::Dir)];
    if fx.symlink(&s.p("link-file"), &s.p("a.txt"), FileKind::File) {
        want.push(("link-file", EntryKind::Symlink));
    }
    if fx.symlink(&s.p("link-dir"), &s.p("sub"), FileKind::Dir) {
        want.push(("link-dir", EntryKind::Symlink));
    }
    if fx.symlink(&s.p("dangling"), &s.p("none.txt"), FileKind::File) {
        want.push(("dangling", EntryKind::Symlink));
    }
    if fx.junction(&s.p("junction"), &s.p("sub")) {
        want.push(("junction", EntryKind::Symlink));
    }
    if fx.special(&s.p("fifo")) {
        want.push(("fifo", EntryKind::Other));
    }
    let map = listing(s, &s.root);
    assert_eq!(
        map.len(),
        want.len(),
        "{:?}",
        map.keys().collect::<Vec<_>>()
    );
    for (name, kind) in &want {
        assert_eq!(kind_of(&map, name), *kind, "{name}");
    }
    for e in map.values() {
        if e.kind == EntryKind::File {
            let want = s.caps.read_dir_len.then_some(3);
            assert_eq!(
                e.len, want,
                "大きさは列挙で分かるときだけ、正しい値で: {e:?}"
            );
        } else {
            assert_eq!(e.len, None, "ファイル以外の大きさは返さない: {e:?}");
        }
        assert!(!e.offline, "{e:?}");
    }
    // 直下だけを返す (降りない)
    assert!(!map.contains_key(&OsString::from("inner.txt")));
}

// 03 §12.7.8: 隠しファイル・日本語と空白・UTF-8 で表せない名前 (Unix) と対にならないサロゲート (Windows)・末尾のドットと空白・
// 予約名 (`con.txt`)。列挙で名前が変わらない (02 §4.5 の DirEntry.name、05 §2.1)
fn read_dir_names(s: &Subject) {
    let fx = s.fx();
    let plain = [
        "日本語 の 名前.txt",
        "末尾のドット.",
        "末尾の空白 ",
        "con.txt",
        " 先頭の空白.txt",
    ];
    for n in plain {
        fx.file(&s.p(n), b"x");
    }
    fx.file(&s.p(".hidden"), b"x");
    fx.hidden_file(&s.p("隠し属性.txt"));
    let odd = odd_name();
    fx.file(&s.root.join(&odd), b"x");
    let map = listing(s, &s.root);
    let mut want: Vec<OsString> = plain.iter().map(OsString::from).collect();
    want.extend([
        OsString::from(".hidden"),
        OsString::from("隠し属性.txt"),
        odd,
    ]);
    want.sort();
    assert_eq!(map.keys().cloned().collect::<Vec<_>>(), want);
    assert!(map.values().all(|e| e.kind == EntryKind::File), "{map:?}");
}

// 03 §12.7.8 / 05 §3.1: 260 文字を超えるパス (Windows は std が verbatim の接頭辞を付けて読む)
fn read_dir_long_path(s: &Subject) {
    let mut dir = s.root.clone();
    while dir.as_path().as_os_str().len() < 320 {
        dir = dir.join("長いパスの途中のディレクトリ-0123456789");
    }
    s.fx().dir(&dir);
    s.fx().file(&dir.join("深いファイル.txt"), b"deep");
    let map = listing(s, &dir);
    assert_eq!(kind_of(&map, "深いファイル.txt"), EntryKind::File);
    assert_eq!(map.len(), 1);
}

// 02 §4.5 DirEntry.offline、05 §2.1 (RECALL_ON_DATA_ACCESS / RECALL_ON_OPEN / OFFLINE の属性。Unix は常に false)
fn read_dir_offline(s: &Subject) {
    let fx = s.fx();
    fx.file(&s.p("local.txt"), b"x");
    let has = fx.offline_file(&s.p("online-only.txt"));
    let map = listing(s, &s.root);
    assert!(!map[&OsString::from("local.txt")].offline);
    if has {
        let e = &map[&OsString::from("online-only.txt")];
        assert!(e.offline && e.kind == EntryKind::File, "{e:?}");
    }
}

// 02 §4.5 read_dir の注: 無い → NotFound (detail なし。ダングリングのリンクだけ DanglingLink)、ディレクトリでない
// (ファイルを指すリンクを含む) → InvalidPath (detail なし)、読めない → PermissionDenied
fn read_dir_errors(s: &Subject) {
    let e = s.fs.read_dir(&s.p("none"), 10).unwrap_err();
    assert_eq!(
        kind_detail(&e),
        (FsErrorKind::NotFound, None),
        "無いディレクトリ"
    );
    let e = s.fs.read_dir(&s.p("none").join("deeper"), 10).unwrap_err();
    assert_eq!(kind_detail(&e), (FsErrorKind::NotFound, None), "親も無い");
    s.fx().file(&s.p("f.txt"), b"x");
    let e = s.fs.read_dir(&s.p("f.txt"), 10).unwrap_err();
    assert_eq!(
        kind_detail(&e),
        (FsErrorKind::InvalidPath, None),
        "ファイル"
    );
    if s.fx()
        .symlink(&s.p("link-f"), &s.p("f.txt"), FileKind::File)
    {
        let e = s.fs.read_dir(&s.p("link-f"), 10).unwrap_err();
        assert_eq!(
            kind_detail(&e),
            (FsErrorKind::InvalidPath, None),
            "ファイルを指すリンク"
        );
    }
    if s.fx()
        .symlink(&s.p("dangling"), &s.p("none-dir"), FileKind::Dir)
    {
        let e = s.fs.read_dir(&s.p("dangling"), 10).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::NotFound, "ダングリングのリンク");
        assert!(
            matches!(e.detail, Some(FsErrorDetail::DanglingLink { .. })),
            "{:?}",
            e.detail
        );
    }
    let locked = s.p("locked");
    s.fx().dir(&locked);
    if s.fx().deny_list(&locked) {
        let e = s.fs.read_dir(&locked, 10).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::PermissionDenied, "読めない");
    }
}

// 02 §4.5 DirListing.truncated、05 §2.1 (max_entries 個を返した後にまだ項目があれば打ち切る)
fn read_dir_max_entries(s: &Subject) {
    let all: Vec<OsString> = (0..5)
        .map(|i| OsString::from(format!("f{i}.txt")))
        .collect();
    for n in &all {
        s.fx().file(&s.root.join(n), b"x");
    }
    for (max, truncated) in [(0, true), (1, true), (4, true), (5, false), (6, false)] {
        let l = s.fs.read_dir(&s.root, max).expect("read_dir");
        assert_eq!(l.truncated, truncated, "max_entries = {max}");
        assert_eq!(l.entries.len(), max.min(all.len()), "max_entries = {max}");
        for e in &l.entries {
            assert!(all.contains(&e.name), "{e:?}");
        }
    }
}

// 02 §4.5 DirListing.incomplete、05 §2.1 (列挙そのものの途中の失敗は、そこまでの項目で incomplete。打ち切りとは別)
fn read_dir_incomplete(s: &Subject) {
    for i in 0..5 {
        s.fx().file(&s.p(&format!("f{i}.txt")), b"x");
    }
    if !s.fx().fail_listing_after(&s.root, 2) {
        // 実 FS では途中の失敗を作れない。失敗しない列挙は incomplete を立てない
        let l = s.fs.read_dir(&s.root, usize::MAX).expect("read_dir");
        assert!(
            !l.incomplete && !l.truncated && l.entries.len() == 5,
            "{l:?}"
        );
        return;
    }
    let l =
        s.fs.read_dir(&s.root, usize::MAX)
            .expect("途中の失敗は Err にしない");
    assert!(l.incomplete && !l.truncated, "{l:?}");
    assert_eq!(l.entries.len(), 2, "失敗までの項目を返す");
    // 失敗の前に上限に達したら、打ち切りだけ
    let l = s.fs.read_dir(&s.root, 1).expect("read_dir");
    assert!(
        l.truncated && !l.incomplete && l.entries.len() == 1,
        "{l:?}"
    );
}

/// 通常でないファイルを開く操作が戻るのを待つ時間。FIFO を開くのが退行して止まっても、試験を失敗で終わらせる。
const OPEN_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(10);

// 05 §2.1 の open_read() の行 (v0.5 の M31): 開いた後のハンドルで通常のファイルと確かめ、FIFO・デバイスで止まらずに
// InvalidPath (detail = NotAFile)
fn open_read_not_regular(s: &Subject) {
    let Some(p) = s.fx().device_in(&s.root) else {
        return;
    };
    // 別のスレッドで開き、戻らなければ失敗にする (止まったスレッドはプロセスの終わりまで残る)
    let (tx, rx) = std::sync::mpsc::channel();
    let fs = Arc::clone(&s.fs);
    let target = p.clone();
    // 試験だけの見張りのスレッド (deps-policy.toml の allowed_files)
    #[allow(clippy::disallowed_methods)]
    std::thread::spawn(move || {
        let r = fs
            .open_read(&target)
            .map(|_| ())
            .map_err(|e| kind_detail(&e));
        let _ = tx.send(r);
    });
    let r = rx.recv_timeout(OPEN_WATCHDOG).unwrap_or_else(|_| {
        panic!("open_read が {OPEN_WATCHDOG:?} 戻らない (FIFO で止まった): {p:?}")
    });
    assert_eq!(
        r,
        Err((FsErrorKind::InvalidPath, Some(FsErrorDetail::NotAFile))),
        "{p:?}"
    );
}

// ---------- read_checked (v0.5 の M33。02 §4.5、07 §12.1 の OWN-1〜OWN-6) ----------
// 実 FS では一時ディレクトリ (Windows は NTFS、Unix は ext4 / tmpfs など所有者を判定できる FS) の下で行う。

const CFG: &str = "config.toml";

/// 安全な場所 (`workspace.safe_directories`) に何を渡すか。
#[derive(Clone, Copy, Debug)]
enum Safe {
    /// 渡さない (判定する)。
    None,
    /// 対象のディレクトリ (`root`) を渡す (その下は判定を省く)。
    Root,
}

fn rc_links(
    s: &Subject,
    dir: &FsPath,
    name: &str,
    max_len: u64,
    safe: Safe,
    links: DirLinks,
) -> CheckedRead {
    let safe_dirs = match safe {
        Safe::None => Vec::new(),
        Safe::Root => vec![s.root.clone()],
    };
    s.fs.read_checked(dir, std::ffi::OsStr::new(name), max_len, &safe_dirs, links)
        .unwrap_or_else(|e| panic!("read_checked({dir:?}, {name}) が失敗した: {e:?}"))
}

/// `dir` の最後の要素が `.eve` なら `NoFollowFromEve`、ほかは `Follow` で読む (段 5・段 6 の呼び方)。
fn rc(s: &Subject, dir: &FsPath, name: &str, max_len: u64, safe: Safe) -> CheckedRead {
    let links = if dir
        .as_path()
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case(WORKSPACE_CONFIG_DIR))
    {
        DirLinks::NoFollowFromEve
    } else {
        DirLinks::Follow
    };
    rc_links(s, dir, name, max_len, safe, links)
}

fn not_regular(what: CheckedObject) -> CheckedRead {
    CheckedRead::Rejected {
        what,
        verdict: OwnerVerdict::NotRegular,
    }
}

/// 判定を通ると同じ中身を読む。`max_len` ちょうどは読み、超えたら `TooLarge`。`.eve` とその下 (`.eve/themes`) も同じ。
fn read_checked_reads(s: &Subject) {
    if !s.caps.owner_check {
        return;
    }
    let d = s.p("ws");
    s.fx().dir(&d);
    s.fx().file(&d.join(CFG), b"a = 1\n");
    let want = CheckedRead::Read {
        bytes: b"a = 1\n".to_vec(),
    };
    for safe in [Safe::None, Safe::Root] {
        assert_eq!(rc(s, &d, CFG, 1024, safe), want, "{safe:?}");
        assert_eq!(rc(s, &d, CFG, 6, safe), want, "max_len ちょうど");
        assert_eq!(rc(s, &d, CFG, 5, safe), CheckedRead::TooLarge);
    }
    assert_eq!(rc(s, &d, CFG, 0, Safe::None), CheckedRead::TooLarge);
    let eve = d.join(".eve");
    s.fx().dir(&eve.join("themes"));
    s.fx().file(&eve.join("keymap.toml"), b"");
    s.fx().file(&eve.join("themes").join("t.toml"), b"t");
    assert_eq!(
        rc(s, &eve, "keymap.toml", 1024, Safe::None),
        CheckedRead::Read { bytes: Vec::new() },
        "空のファイル"
    );
    assert_eq!(
        rc(s, &eve.join("themes"), "t.toml", 1024, Safe::None),
        CheckedRead::Read {
            bytes: b"t".to_vec()
        },
        ".eve の下のディレクトリ"
    );
}

/// ファイル・ディレクトリ・親が無ければ `Missing` (エラーにしない)。`.eve` がファイルなら、その中は無い。
fn read_checked_missing(s: &Subject) {
    let d = s.p("ws");
    s.fx().dir(&d);
    for (dir, why) in [
        (d.clone(), "ファイルが無い"),
        (s.p("none"), "ディレクトリが無い"),
        (s.p("none").join(".eve"), "親も無い"),
        (d.join(".eve").join("themes"), ".eve が無い"),
    ] {
        assert_eq!(
            rc(s, &dir, CFG, 1024, Safe::None),
            CheckedRead::Missing,
            "{why}"
        );
    }
    let w = s.p("w2");
    s.fx().dir(&w);
    s.fx().file(&w.join(".eve"), b"");
    for dir in [w.join(".eve"), w.join(".eve").join("themes")] {
        assert_eq!(
            rc(s, &dir, CFG, 1024, Safe::None),
            CheckedRead::Missing,
            ".eve がファイル ({dir:?})"
        );
    }
}

/// 読むファイルがディレクトリ・シンボリックリンク・FIFO なら `NotRegular` で読まない。`.eve` とその下のディレクトリが
/// リンク・ジャンクションならディレクトリの側で `NotRegular`。安全な場所の下でも同じ (OWN-5 でもリンクは辿らない)。
/// `.eve` より上と `.editorconfig` の置き場所のディレクトリのリンクは辿る。
fn read_checked_not_regular(s: &Subject) {
    if !s.caps.owner_check {
        return;
    }
    let fx = s.fx();
    let d = s.p("ws");
    fx.dir(&d);
    fx.dir(&d.join(CFG));
    fx.file(&d.join("target.toml"), b"x");
    let mut cases = vec![(CFG, "ディレクトリ")];
    if fx.symlink(&d.join("link.toml"), &d.join("target.toml"), FileKind::File) {
        cases.push(("link.toml", "シンボリックリンク"));
    }
    if fx.special(&d.join("fifo.toml")) {
        cases.push(("fifo.toml", "FIFO"));
    }
    for safe in [Safe::None, Safe::Root] {
        for (name, why) in &cases {
            assert_eq!(
                rc(s, &d, name, 1024, safe),
                not_regular(CheckedObject::File),
                "{why} ({safe:?})"
            );
        }
    }
    let real = s.p("real");
    fx.dir(&real);
    fx.file(&real.join(CFG), b"x");
    // ディレクトリのリンクを作る (シンボリックリンクを作れなければジャンクション)
    let dir_link =
        |link: &FsPath| fx.symlink(link, &real, FileKind::Dir) || fx.junction(link, &real);
    let mut eves = Vec::new();
    let sym = s.p("w-sym");
    fx.dir(&sym);
    if fx.symlink(&sym.join(".eve"), &real, FileKind::Dir) {
        eves.push((sym.join(".eve"), "シンボリックリンク"));
    }
    let jun = s.p("w-jun");
    fx.dir(&jun);
    if fx.junction(&jun.join(".eve"), &real) {
        eves.push((jun.join(".eve"), "ジャンクション"));
    }
    // `.eve` の下のディレクトリのリンク (`.eve/themes`)
    let th = s.p("w-themes");
    fx.dir(&th.join(".eve"));
    if dir_link(&th.join(".eve").join("themes")) {
        eves.push((th.join(".eve").join("themes"), ".eve の下のリンク"));
    }
    // `.eve` がリンクなら、その下のディレクトリを読むときも辿らない
    if let Some((e, _)) = eves.first().cloned() {
        fx.dir(&real.join("themes"));
        eves.push((e.join("themes"), "リンクの .eve の下"));
    }
    for safe in [Safe::None, Safe::Root] {
        for (eve, why) in &eves {
            assert_eq!(
                rc(s, eve, CFG, 1024, safe),
                not_regular(CheckedObject::Dir),
                "{why} ({safe:?})"
            );
        }
    }
    // `.eve` より上と `.editorconfig` の置き場所のリンクは辿る (判定するのは辿った先のディレクトリ)
    let up = s.p("up");
    if dir_link(&up) {
        assert_eq!(
            rc(s, &up, CFG, 1024, Safe::None),
            CheckedRead::Read {
                bytes: b"x".to_vec()
            },
            "ディレクトリのリンク"
        );
        fx.dir(&real.join(".eve"));
        fx.file(&real.join(".eve").join(CFG), b"e");
        assert_eq!(
            rc(s, &up.join(".eve"), CFG, 1024, Safe::None),
            CheckedRead::Read {
                bytes: b"e".to_vec()
            },
            ".eve より上のリンク"
        );
    }
}

fn others_can_write(r: &CheckedRead, what: CheckedObject) -> bool {
    matches!(r, CheckedRead::Rejected { what: w, verdict: OwnerVerdict::OthersCanWrite { who } }
        if *w == what && !who.is_empty())
}

/// 他人が書けるディレクトリ・ファイルは読まない (OWN-1 の (a)(b))。他人が書けるディレクトリに無いファイルは `Missing`
/// (診断を出さない)。安全な場所の下なら読む。安全な場所の下のリンクは、辿った先 (最終のパス) で比べるので、安全な場所の
/// 外を指していれば判定する (OWN-5)。
fn read_checked_others_write(s: &Subject) {
    if !s.caps.owner_check {
        return;
    }
    let fx = s.fx();
    let d = s.p("open-dir");
    fx.dir(&d);
    fx.file(&d.join(CFG), b"x");
    let read = CheckedRead::Read {
        bytes: b"x".to_vec(),
    };
    if fx.others_can_write(&d) {
        let r = rc(s, &d, CFG, 1024, Safe::None);
        assert!(others_can_write(&r, CheckedObject::Dir), "{r:?}");
        assert_eq!(
            rc(s, &d, ".editorconfig", 1024, Safe::None),
            CheckedRead::Missing
        );
        assert_eq!(rc(s, &d, CFG, 1024, Safe::Root), read);
        // 相対のパス・空のパスの安全な場所は比べない (判定する)
        let r =
            s.fs.read_checked(
                &d,
                std::ffi::OsStr::new(CFG),
                1024,
                &[FsPath::new(""), FsPath::new("open-dir")],
                DirLinks::Follow,
            )
            .expect("read_checked");
        assert!(
            others_can_write(&r, CheckedObject::Dir),
            "相対のパス: {r:?}"
        );
        // 安全な場所 `safe` の下のリンク `safe/alias` が、安全な場所の外の `open-dir` を指す
        let safe = s.p("safe");
        fx.dir(&safe);
        let alias = safe.join("alias");
        if fx.symlink(&alias, &d, FileKind::Dir) || fx.junction(&alias, &d) {
            let r =
                s.fs.read_checked(
                    &alias,
                    std::ffi::OsStr::new(CFG),
                    1024,
                    std::slice::from_ref(&safe),
                    DirLinks::Follow,
                )
                .expect("read_checked");
            assert!(
                others_can_write(&r, CheckedObject::Dir),
                "安全な場所の下のリンクの先は判定する: {r:?}"
            );
        }
    }
    let f = s.p("open-file");
    fx.dir(&f);
    fx.file(&f.join(CFG), b"x");
    if fx.others_can_write(&f.join(CFG)) {
        let r = rc(s, &f, CFG, 1024, Safe::None);
        assert!(others_can_write(&r, CheckedObject::File), "{r:?}");
        assert_eq!(rc(s, &f, CFG, 1024, Safe::Root), read);
    }
}

/// `name` は 1 つの名前だけ (空・`.`・`..`・区切りを含むものは `InvalidPath`)。`dir` に `..` があるか、`NoFollowFromEve` で
/// `.eve` が無ければ `InvalidPath`。
fn read_checked_invalid_name(s: &Subject) {
    let d = s.p("ws");
    s.fx().dir(&d);
    s.fx().dir(&d.join("sub"));
    let kind = |dir: &FsPath, name: &str, links| {
        s.fs.read_checked(dir, std::ffi::OsStr::new(name), 1024, &[], links)
            .map_err(|e| e.kind)
    };
    for name in ["", ".", "..", "sub/config.toml"] {
        assert_eq!(
            kind(&d, name, DirLinks::Follow),
            Err(FsErrorKind::InvalidPath),
            "{name:?}"
        );
    }
    assert_eq!(
        kind(&d, CFG, DirLinks::NoFollowFromEve),
        Err(FsErrorKind::InvalidPath),
        ".eve が無い"
    );
    assert_eq!(
        kind(
            &d.join("sub").join("..").join(".eve"),
            CFG,
            DirLinks::NoFollowFromEve
        ),
        Err(FsErrorKind::InvalidPath),
        ".. がある"
    );
}
