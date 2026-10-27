//! `WinFs::write_atomic` の実ファイルでの検査 (docs/05 §4.1 / §4.2 / §5.6、09 §3.6 の保存の項)。
//! 一時ファイル + `ReplaceFileW` で、元のファイルを無傷に保ち、ACL・ADS・属性を保つこと。

#![cfg(windows)]
#![allow(clippy::result_large_err, clippy::print_stderr)]

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use eve_core::{
    ChunkError, Expect, FileKind, FileStamp, FileSystem, FsErrorDetail, FsErrorKind, FsPath,
    HardlinkPolicy, ReadChunks, SaveMethod, SaveOptions, SaveWarning,
};
use eve_platform::fs::WinFs;

/// 一時ディレクトリ (終わりに消す)。
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!("eve-save-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }

    fn p(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn path(&self, name: &str) -> FsPath {
        FsPath::new(self.p(name))
    }

    /// ディレクトリの中のファイル名 (一時ファイル・退避名の残りを見つける)。
    fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// チャンクの列を返し、`fail_at` 番目のチャンクで失敗する。
struct Chunks {
    chunks: Vec<Vec<u8>>,
    i: usize,
    fail_at: Option<usize>,
}

impl Chunks {
    fn of(data: &[u8], size: usize) -> Chunks {
        Chunks {
            chunks: data.chunks(size.max(1)).map(<[u8]>::to_vec).collect(),
            i: 0,
            fail_at: None,
        }
    }

    fn failing(mut self, at: usize) -> Chunks {
        self.fail_at = Some(at);
        self
    }
}

impl ReadChunks for Chunks {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        if self.fail_at == Some(self.i) {
            return Err(ChunkError::Aborted);
        }
        let c = self.chunks.get(self.i);
        self.i += 1;
        Ok(c.map(Vec::as_slice))
    }
}

fn save(
    path: &FsPath,
    data: &[u8],
    expect: Expect,
) -> Result<eve_core::SaveReport, eve_core::FsError> {
    WinFs::default().write_atomic(
        path,
        &mut Chunks::of(data, 4096),
        &SaveOptions::user_file(expect),
    )
}

fn stamp(path: &FsPath) -> FileStamp {
    FileStamp::from_meta(&WinFs::default().metadata(path).unwrap(), None)
}

#[test]
fn 新規ファイルを作る() {
    let t = Tmp::new("new");
    let p = t.path("a.txt");
    let r = save(&p, b"hello", Expect::Absent).unwrap();
    assert_eq!(r.method, SaveMethod::NewFile);
    assert_eq!(fs::read(t.p("a.txt")).unwrap(), b"hello");
    assert_eq!(r.new_stamp, stamp(&p), "new_stamp は保存後の T と一致する");
    assert_eq!(t.names(), ["a.txt"], "一時ファイルは残らない");
}

#[test]
fn 親フォルダが無いときは作らずに_not_found() {
    let t = Tmp::new("parent");
    let e = save(&t.path("no/such/a.txt"), b"x", Expect::Absent).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::NotFound);
    assert!(matches!(
        e.detail,
        Some(FsErrorDetail::ParentMissing { .. })
    ));
    assert!(!t.p("no").exists());
}

#[test]
fn 既存のファイルを置換する() {
    let t = Tmp::new("replace");
    let p = t.path("a.txt");
    fs::write(t.p("a.txt"), b"old").unwrap();
    let s0 = stamp(&p);
    let r = save(&p, "新しい内容".as_bytes(), Expect::Stamp(s0)).unwrap();
    assert_eq!(r.method, SaveMethod::Atomic);
    assert_eq!(fs::read(t.p("a.txt")).unwrap(), "新しい内容".as_bytes());
    assert_eq!(r.new_stamp, stamp(&p));
    assert_ne!(
        r.new_stamp.file_id, s0.file_id,
        "ReplaceFileW では tmp の FileId になる (05 §5.4)"
    );
    assert_eq!(t.names(), ["a.txt"], "一時ファイルも退避名も残らない");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn 大きな内容を複数のチャンクで書く() {
    let t = Tmp::new("big");
    let p = t.path("big.bin");
    let data: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    save(&p, &data, Expect::Absent).unwrap();
    assert_eq!(fs::read(t.p("big.bin")).unwrap(), data);
}

#[test]
fn 空の内容も書ける() {
    let t = Tmp::new("empty");
    let p = t.path("e.txt");
    save(&p, b"", Expect::Absent).unwrap();
    assert_eq!(fs::read(t.p("e.txt")).unwrap(), b"");
}

#[test]
fn 照合の不一致は_conflict_で何も書かない() {
    let t = Tmp::new("conflict");
    let p = t.path("a.txt");
    fs::write(t.p("a.txt"), b"mine").unwrap();
    let s0 = stamp(&p);
    // 他のプログラムが変更した
    std::thread::sleep(std::time::Duration::from_millis(30));
    fs::write(t.p("a.txt"), b"theirs!").unwrap();
    let e = save(&p, b"overwrite", Expect::Stamp(s0)).unwrap_err();
    assert!(
        matches!(e.kind, FsErrorKind::Conflict { current: Some(_) }),
        "{e:?}"
    );
    assert_eq!(fs::read(t.p("a.txt")).unwrap(), b"theirs!");
    assert_eq!(t.names(), ["a.txt"]);
    // 新規を期待したのに在る
    let e = save(&p, b"x", Expect::Absent).unwrap_err();
    assert!(matches!(e.kind, FsErrorKind::Conflict { current: Some(_) }));
    // 在ると思ったのに消えた
    fs::remove_file(t.p("a.txt")).unwrap();
    let e = save(&p, b"x", Expect::Stamp(s0)).unwrap_err();
    assert!(matches!(e.kind, FsErrorKind::Conflict { current: None }));
    assert!(!t.p("a.txt").exists());
}

// verifies: AC-FR-105-2
#[test]
fn 供給側の失敗は一時ファイルを消し元のファイルは無傷() {
    let t = Tmp::new("source");
    let p = t.path("a.txt");
    fs::write(t.p("a.txt"), b"original").unwrap();
    let s0 = stamp(&p);
    let mut src = Chunks::of(&vec![b'x'; 20_000], 4096).failing(2);
    let e = WinFs::default()
        .write_atomic(&p, &mut src, &SaveOptions::user_file(Expect::Stamp(s0)))
        .unwrap_err();
    assert_eq!(e.kind, FsErrorKind::SourceFailed);
    assert_eq!(fs::read(t.p("a.txt")).unwrap(), b"original");
    assert_eq!(t.names(), ["a.txt"]);
    // 新規ファイルでも部分書込のファイルを残さない
    let mut src = Chunks::of(b"abcdef", 2).failing(1);
    let e = WinFs::default()
        .write_atomic(
            &t.path("n.txt"),
            &mut src,
            &SaveOptions::user_file(Expect::Absent),
        )
        .unwrap_err();
    assert_eq!(e.kind, FsErrorKind::SourceFailed);
    assert_eq!(t.names(), ["a.txt"]);
}

#[test]
fn ads_zone_identifier_と_hidden_属性を保つ() {
    let t = Tmp::new("ads");
    let p = t.path("a.txt");
    fs::write(t.p("a.txt"), b"old").unwrap();
    // 名前付きストリーム (ダウンロードの印)
    let ads = format!("{}:Zone.Identifier", t.p("a.txt").display());
    fs::write(&ads, b"[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
    let st = Command::new("attrib")
        .arg("+H")
        .arg(t.p("a.txt"))
        .status()
        .unwrap();
    assert!(st.success());
    let s0 = stamp(&p);
    save(&p, b"new", Expect::Stamp(s0)).unwrap();
    assert_eq!(fs::read(t.p("a.txt")).unwrap(), b"new");
    assert_eq!(
        fs::read(&ads).unwrap(),
        b"[ZoneTransfer]\r\nZoneId=3\r\n",
        "ReplaceFileW は ADS を保つ (05 §4.2)"
    );
    let out = Command::new("attrib").arg(t.p("a.txt")).output().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains('H'),
        "HIDDEN を保つ"
    );
}

#[test]
fn 読み取り専用の保存先は置換できず_atomic_unsupported_で無傷() {
    let t = Tmp::new("readonly");
    let p = t.path("a.txt");
    fs::write(t.p("a.txt"), b"keep").unwrap();
    let mut perm = fs::metadata(t.p("a.txt")).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    {
        perm.set_readonly(true);
        fs::set_permissions(t.p("a.txt"), perm.clone()).unwrap();
    }
    let s0 = stamp(&p);
    let e = save(&p, b"new", Expect::Stamp(s0)).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::AtomicUnsupported, "{e:?}");
    assert_eq!(fs::read(t.p("a.txt")).unwrap(), b"keep");
    assert_eq!(t.names(), ["a.txt"], "一時ファイルを消す");
    #[allow(clippy::permissions_set_readonly_false)]
    {
        perm.set_readonly(false);
        fs::set_permissions(t.p("a.txt"), perm).unwrap();
    }
}

#[test]
fn ハードリンクは_ask_で_atomic_unsupported_break_link_なら置換する() {
    let t = Tmp::new("hardlink");
    let p = t.path("a.txt");
    fs::write(t.p("a.txt"), b"shared").unwrap();
    fs::hard_link(t.p("a.txt"), t.p("b.txt")).unwrap();
    let s0 = stamp(&p);
    let e = save(&p, b"new", Expect::Stamp(s0)).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::AtomicUnsupported);
    assert!(matches!(
        e.detail,
        Some(FsErrorDetail::Hardlink { nlink: 2 })
    ));
    assert_eq!(fs::read(t.p("b.txt")).unwrap(), b"shared");
    // リンクを切って置換する
    let mut o = SaveOptions::user_file(Expect::Stamp(s0));
    o.hardlink = HardlinkPolicy::BreakLink;
    WinFs::default()
        .write_atomic(&p, &mut Chunks::of(b"new", 8), &o)
        .unwrap();
    assert_eq!(fs::read(t.p("a.txt")).unwrap(), b"new");
    assert_eq!(
        fs::read(t.p("b.txt")).unwrap(),
        b"shared",
        "もう一方は変わらない"
    );
}

/// シンボリックリンクを作れたか。権限が無い開発機では飛ばす。CI (`GITHUB_ACTIONS`) では作れるはずなので、作れなければ失敗にする。
fn symlink_made(r: std::io::Result<()>) -> bool {
    match r {
        Ok(()) => true,
        Err(e) if std::env::var_os("GITHUB_ACTIONS").is_some() => {
            panic!("CI でシンボリックリンクを作れません: {e}")
        }
        Err(_) => {
            eprintln!("シンボリックリンクを作れない環境 (権限なし): この検査を飛ばす");
            false
        }
    }
}

// verifies: AC-FR-106-2
#[test]
fn ジャンクション経由でもリンク先のファイルを置換しジャンクションは残る() {
    // ジャンクションは権限なしで作れるので、飛ばさない
    let t = Tmp::new("junction");
    fs::create_dir(t.p("real")).unwrap();
    fs::write(t.p("real").join("a.txt"), b"old").unwrap();
    let out = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(t.p("junc"))
        .arg(t.p("real"))
        .output()
        .expect("cmd");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let via = FsPath::new(t.p("junc").join("a.txt"));
    let s0 = stamp(&via);
    save(&via, b"new", Expect::Stamp(s0)).unwrap();
    assert_eq!(fs::read(t.p("real").join("a.txt")).unwrap(), b"new");
    assert!(
        t.p("junc").join("a.txt").exists() && t.p("junc").is_dir(),
        "ジャンクションは残る"
    );
    // 後始末: ジャンクションは中身ごと消さないよう、先にリンクだけを外す
    fs::remove_dir(t.p("junc")).unwrap();
}

// verifies: AC-FR-106-2
#[test]
fn シンボリックリンクはリンク先を置換しリンクは残る() {
    let t = Tmp::new("symlink");
    fs::write(t.p("real.txt"), b"old").unwrap();
    if !symlink_made(std::os::windows::fs::symlink_file(
        t.p("real.txt"),
        t.p("link.txt"),
    )) {
        return;
    }
    let link = t.path("link.txt");
    let s0 = stamp(&link);
    save(&link, b"new", Expect::Stamp(s0)).unwrap();
    assert_eq!(fs::read(t.p("real.txt")).unwrap(), b"new");
    assert!(
        fs::symlink_metadata(t.p("link.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

// verifies: AC-FR-106-2
#[test]
fn ダングリングリンクは保存しない() {
    let t = Tmp::new("dangling");
    if !symlink_made(std::os::windows::fs::symlink_file(
        t.p("missing.txt"),
        t.p("link.txt"),
    )) {
        return;
    }
    let e = save(&t.path("link.txt"), b"x", Expect::Absent).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::NotFound);
    assert!(matches!(e.detail, Some(FsErrorDetail::DanglingLink { .. })));
    assert!(!t.p("missing.txt").exists(), "リンク先を作らない");
}

#[test]
fn 他のプロセスが_share_delete_なしで開いていると置換できず_atomic_unsupported() {
    use std::os::windows::fs::OpenOptionsExt;
    let t = Tmp::new("sharing");
    let p = t.path("a.txt");
    fs::write(t.p("a.txt"), b"busy").unwrap();
    let s0 = stamp(&p);
    // FILE_SHARE_READ | FILE_SHARE_WRITE (DELETE なし)
    let _hold = fs::OpenOptions::new()
        .read(true)
        .share_mode(1 | 2)
        .open(t.p("a.txt"))
        .unwrap();
    let t0 = std::time::Instant::now();
    let e = save(&p, b"new", Expect::Stamp(s0)).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::AtomicUnsupported, "{e:?}");
    assert!(
        t0.elapsed().as_millis() >= 2000,
        "指数バックオフで再試行する (合計 2.55 s)"
    );
    drop(_hold);
    assert_eq!(fs::read(t.p("a.txt")).unwrap(), b"busy");
    assert_eq!(t.names(), ["a.txt"]);
}

/// `icacls` の出力から、アクセス許可を持つ主体の一覧を取る。
fn acl_principals(path: &std::path::Path) -> Vec<String> {
    let out = Command::new("icacls").arg(path).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.lines()
        .filter(|l| l.contains(":(") || l.contains(":F"))
        .map(|l| l.trim().to_owned())
        .collect()
}

/// 継承の印 `(I)` とパスの前置きを除いた、許可のエントリの集合 (`NT AUTHORITY\SYSTEM:(F)` の形)。
fn acl_entries(path: &std::path::Path) -> std::collections::BTreeSet<String> {
    let prefix = path.to_string_lossy().into_owned();
    acl_principals(path)
        .into_iter()
        .map(|l| {
            l.strip_prefix(&prefix)
                .unwrap_or(&l)
                .trim()
                .replace("(I)", "")
        })
        .collect()
}

// verifies: AC-NFR-505-1
#[test]
fn private_で作った新規ファイルは所有者と_system_だけ() {
    let t = Tmp::new("private");
    let p = t.path("secret.txt");
    let mut o = SaveOptions::user_file(Expect::Absent);
    o.private = true;
    WinFs::default()
        .write_atomic(&p, &mut Chunks::of(b"secret", 8), &o)
        .unwrap();
    let acl = acl_principals(&t.p("secret.txt")).join("\n");
    assert!(acl.contains("SYSTEM"), "{acl}");
    assert!(
        !acl.contains("BUILTIN\\Users") && !acl.contains("Everyone"),
        "{acl}"
    );
    assert!(
        !acl.contains("Administrators"),
        "管理者も含めない (05 §5.6): {acl}"
    );
    // 継承は遮断されている ((I) の付いた行がない)
    assert!(!acl.contains("(I)"), "{acl}");
}

#[test]
fn 置換後の_acl_は元の_t_のものを保ち_tmp_の所有者のみにならない() {
    let t = Tmp::new("acl");
    let p = t.path("a.txt");
    fs::write(t.p("a.txt"), b"old").unwrap();
    let before = acl_entries(&t.p("a.txt"));
    let s0 = stamp(&p);
    save(&p, b"new", Expect::Stamp(s0)).unwrap();
    let after = acl_entries(&t.p("a.txt"));
    // T の許可は 1 つも失われない (tmp の所有者のみの DACL に狭まらない)。継承の印 `(I)` は比べない:
    // `ReplaceFileW` が T の明示のエントリを適用した結果、親からの継承のエントリが加わることがある (CI のランナーで確認)。
    // 広がるのは許容、狭まるのは不可 (05 §4.2 の S-09 の確認項目 (2))
    let lost: Vec<&String> = before.difference(&after).collect();
    assert!(
        lost.is_empty(),
        "失われた許可: {lost:?}
before={before:?}
after={after:?}"
    );
    assert!(!after.is_empty());
}

#[test]
fn 一時ファイルの書込中も_t_は旧内容のまま() {
    // 書込の途中 (チャンクの間) で T を読むと旧内容。置換は最後の 1 回だけ
    let t = Tmp::new("window");
    let p = t.path("a.txt");
    fs::write(t.p("a.txt"), b"OLD").unwrap();
    let s0 = stamp(&p);
    struct Peek<'a> {
        target: PathBuf,
        n: usize,
        seen: &'a std::cell::RefCell<Vec<Vec<u8>>>,
    }
    impl ReadChunks for Peek<'_> {
        fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
            self.n += 1;
            self.seen.borrow_mut().push(fs::read(&self.target).unwrap());
            Ok((self.n <= 3).then_some(&b"chunk"[..]))
        }
    }
    let seen = std::cell::RefCell::new(Vec::new());
    let mut src = Peek {
        target: t.p("a.txt"),
        n: 0,
        seen: &seen,
    };
    WinFs::default()
        .write_atomic(&p, &mut src, &SaveOptions::user_file(Expect::Stamp(s0)))
        .unwrap();
    assert!(seen.borrow().iter().all(|b| b == b"OLD"));
    assert_eq!(fs::read(t.p("a.txt")).unwrap(), b"chunkchunkchunk");
    let _ = (
        FileKind::File,
        SaveWarning::FsyncFailed,
        std::io::stdout().flush(),
    );
}
