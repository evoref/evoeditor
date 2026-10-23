//! Windows のパスのコーパス (01 NFR-404 AC-1): 260 文字超のパス・`\\?\`・UNC・ドライブレター・不正な UTF-16 のパスのファイルを、
//! 実ファイルシステムで開いて保存できる。`PathBuf` / `OsString` を `String` に変換していないことの検査でもある。
//! **環境が無いことを理由にスキップしない** (スキップは不合格)。UNC は管理共有 (`\\localhost\C$`) を使う。

#![cfg(windows)]

use std::ffi::OsString;
use std::fs;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eve_core::{
    CancelToken, ChunkError, Clock, Expect, FileStamp, FileSystem, FsPath, JobKind, JobPorts,
    JobRequest, JobResult, OpenOptions, ReadChunks, SaveOptions, Tick, run_job,
};
use eve_platform::dirs::{Dirs, WinPaths};
use eve_platform::fs::WinFs;
use eve_platform::path::PathCtx;
use eve_text::{ByteOff, ByteRange};
use eve_view::{JobId, Revision};

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Tick {
        Tick(0)
    }
}

struct One(Option<Vec<u8>>);

impl ReadChunks for One {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        Ok(self.0.take().map(|v| &*Box::leak(v.into_boxed_slice())))
    }
}

fn ports() -> JobPorts {
    JobPorts {
        fs: Arc::new(WinFs::default()),
        clock: Arc::new(FixedClock),
        paths: Arc::new(WinPaths::new(
            Dirs::resolve(&|_| None),
            PathCtx {
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                home: None,
            },
        )),
    }
}

/// 開く → 中身を確かめる。
fn open_text(path: &Path) -> String {
    let req = JobRequest {
        id: JobId(1),
        doc: None,
        base: Revision(0),
        kind: JobKind::Open {
            path: FsPath::new(path),
            opts: OpenOptions::default(),
        },
    };
    let mut out = Vec::new();
    run_job(&req, &ports(), &CancelToken::new(), &mut |r| out.push(r));
    match out.pop().expect("結果がある") {
        JobResult::Complete { text, .. } => {
            use eve_text::TextRead;
            text.slice_to_string(ByteRange::new(ByteOff(0), ByteOff(text.len_bytes())))
        }
        other => panic!("{}: 開けません: {other:?}", path.display()),
    }
}

/// 既存のファイルを原子的に保存する (無編集でなく、中身を変える)。
fn save_text(path: &Path, body: &str) {
    let fs_ = WinFs::default();
    let p = FsPath::new(path);
    let stamp = FileStamp::from_meta(&fs_.metadata(&p).expect("メタデータ"), None);
    fs_.write_atomic(
        &p,
        &mut One(Some(body.as_bytes().to_vec())),
        &SaveOptions::user_file(Expect::Stamp(stamp)),
    )
    .unwrap_or_else(|e| panic!("{}: 保存できません: {e:?}", path.display()));
}

/// 開いて保存して、実際にディスクの中身が変わっていることまで確かめる。
fn round_trip(path: &Path) {
    // 同じファイルを別の形のパスで続けて試せるよう、中身を元に戻す
    fs::write(path, "before\r\n").unwrap();
    assert_eq!(open_text(path), "before\r\n", "{}", path.display());
    save_text(path, "after 日本語\r\n");
    assert_eq!(fs::read(path).unwrap(), "after 日本語\r\n".as_bytes());
    assert_eq!(open_text(path), "after 日本語\r\n");
}

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!("eve-paths-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        // 長いパスは `\\?\` を付けないと消せないことがある
        let _ = fs::remove_dir_all(verbatim(&self.0));
    }
}

/// `\\?\` を付けた絶対パス (長いパスの作成・削除用)。`String` に直さない (不正な UTF-16 の名前を壊さない)。
fn verbatim(p: &Path) -> PathBuf {
    if p.as_os_str().to_string_lossy().starts_with(r"\\?\") {
        return p.to_path_buf();
    }
    let mut s = OsString::from(r"\\?\");
    s.push(p.as_os_str());
    PathBuf::from(s)
}

fn make(path: &Path) {
    fs::create_dir_all(verbatim(path).parent().unwrap()).unwrap();
    fs::write(verbatim(path), "before\r\n").unwrap();
}

// verifies: AC-NFR-404-1
#[test]
fn 二百六十文字を超えるパスのファイルを開いて保存できる() {
    let t = Tmp::new("long");
    let mut dir = t.0.clone();
    while dir.to_string_lossy().chars().count() < 300 {
        dir = dir.join("long-directory-name-for-path-corpus");
    }
    let file = dir.join("日本語のファイル.txt");
    assert!(file.to_string_lossy().chars().count() > 260);
    make(&file);
    // `\\?\` なしの通常の形で渡す (利用者が入力するのはこちら)
    round_trip(&file);
    // `\\?\` つきの形でも同じ
    round_trip(&verbatim(&file));
}

// verifies: AC-NFR-404-1
#[test]
fn ドライブレターは大文字でも小文字でも扱える() {
    let t = Tmp::new("drive");
    let file = t.0.join("a.txt");
    make(&file);
    round_trip(&file);
    let s = file.to_string_lossy().into_owned();
    let mut lower = s.clone();
    lower.replace_range(0..1, &s[0..1].to_lowercase());
    round_trip(Path::new(&lower));
}

// verifies: AC-NFR-404-1
#[test]
fn unc_のパスのファイルを開いて保存できる() {
    let t = Tmp::new("unc");
    let file = t.0.join("unc.txt");
    make(&file);
    // `C:\x\y` → `\\localhost\C$\x\y` (管理共有)。`\\?\UNC\localhost\C$\x\y` の形も試す
    let s = file.to_string_lossy().into_owned();
    let (drive, rest) = s.split_at(2);
    let unc = PathBuf::from(format!(r"\\localhost\{}${rest}", &drive[0..1]));
    assert!(
        unc.exists(),
        "管理共有 {} を使えません (UNC の検査はスキップしない。管理者権限と管理共有が要る)",
        unc.display()
    );
    round_trip(&unc);
    let verbatim_unc = PathBuf::from(format!(r"\\?\UNC\localhost\{}${rest}", &drive[0..1]));
    round_trip(&verbatim_unc);
}

// verifies: AC-NFR-404-1
#[test]
fn 不正な_utf16_のファイル名のファイルを開いて保存できる() {
    let t = Tmp::new("utf16");
    // 対になっていないサロゲート (U+D800) と、対になっているもの (U+1F600)
    let wide: Vec<u16> = "bad-"
        .encode_utf16()
        .chain([0xD800])
        .chain("-name-".encode_utf16())
        .chain([0xD83D, 0xDE00])
        .chain(".txt".encode_utf16())
        .collect();
    let name = OsString::from_wide(&wide);
    assert!(name.to_str().is_none(), "UTF-8 に直せない名前");
    let file = t.0.join(name);
    make(&file);
    round_trip(&file);
}
