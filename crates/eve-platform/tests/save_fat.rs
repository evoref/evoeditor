//! 置換をまたいで識別が保たれない FS (FAT・exFAT、後ろがそれらの SMB の共有・WSL の `DrvFs`) での保存の事後の確かめ
//! (docs/05 §4.1 手順 7 の (b)、§5.4。Issue #597)。`EVE_TEST_FAT_DIR` にそういう場所のディレクトリを指したときだけ確かめる
//! (CI には無い。09 §8.2 の手の確認)。無ければ何もしない。

#![cfg(any(windows, target_os = "linux"))]
#![allow(clippy::print_stderr)]

use std::fs;
use std::path::PathBuf;

use eve_core::{
    CancelToken, ChunkError, Expect, FileStamp, FileSystem, FsPath, ReadChunks, SaveMethod,
    SaveOptions, SaveReport, SaveWarning,
};
#[cfg(windows)]
use eve_platform::fs::WinFs as NativeFs;
#[cfg(unix)]
use eve_platform::unix_fs::UnixFs as NativeFs;

/// `EVE_TEST_FAT_DIR` の下の作業ディレクトリ (終わりに消す)。
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Option<Dir> {
        let Some(base) = std::env::var_os("EVE_TEST_FAT_DIR") else {
            eprintln!("EVE_TEST_FAT_DIR が無い: 確かめない");
            return None;
        };
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let d = PathBuf::from(base).join(format!("eve-fat-{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&d).expect("作業ディレクトリ");
        Some(Dir(d))
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Bytes<'a>(Option<&'a [u8]>);

impl ReadChunks for Bytes<'_> {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        Ok(self.0.take())
    }
}

/// 保存の結果を確かめる: `ChangedAfterSave` が無く、`new_stamp` がディスクの T と同じ (次の保存が Conflict にならない)。
fn check(fsys: &NativeFs, p: &FsPath, r: &SaveReport, what: &str) -> FileStamp {
    assert!(
        !r.warnings.contains(&SaveWarning::ChangedAfterSave),
        "{what}: {:?}",
        r.warnings
    );
    let disk = FileStamp::from_meta(&fsys.metadata(p).expect("metadata"), None);
    assert_eq!(r.new_stamp, disk, "{what}: new_stamp はディスクの T と同じ");
    r.new_stamp
}

#[test]
fn 識別が保たれない_fs_でも保存を続けて_conflict_にならず警告も出ない() {
    let Some(d) = Dir::new("save") else { return };
    let fsys = NativeFs::new(d.0.join("state"));
    let p = FsPath::new(d.0.join("a.txt"));
    let atomic = |body: &[u8], expected: Expect| {
        fsys.write_atomic(
            &p,
            &mut Bytes(Some(body)),
            &SaveOptions::user_file(expected),
        )
        .expect("原子的保存")
    };
    let in_place = |body: &[u8], expected: Expect| {
        fsys.write_in_place(
            &p,
            &mut Bytes(Some(body)),
            body.len() as u64,
            &SaveOptions::user_file(expected),
            &CancelToken::new(),
        )
        .expect("直接上書き")
    };

    // 新規ファイル (MoveFileExW / renameat2・link の退路)。WSL の DrvFs (後ろが exFAT) は renameat2 も link も使えず
    // (EINVAL・EPERM) 新規ファイルを保存できないので、そのときは std で作って続ける (この試験の対象の手順 7 とは別の問題)
    let new_file = fsys.write_atomic(
        &p,
        &mut Bytes(Some(b"one\n")),
        &SaveOptions::user_file(Expect::Absent),
    );
    let mut s = match new_file {
        Ok(r) => {
            assert_eq!(r.method, SaveMethod::NewFile);
            check(&fsys, &p, &r, "新規")
        }
        Err(e) if cfg!(unix) && e.kind == eve_core::FsErrorKind::PermissionDenied => {
            eprintln!("新規ファイルを保存できない FS (link の退路が EPERM): {e:?}");
            fs::write(p.as_path(), b"one\n").expect("作る");
            FileStamp::from_meta(&fsys.metadata(&p).expect("metadata"), None)
        }
        Err(e) => panic!("新規: {e:?}"),
    };
    // 置換を 2 回続ける (ReplaceFileW / rename。2 回目が 1 回目の new_stamp で照合に通る)。同じ長さの内容も含める
    for (i, body) in [&b"two!\n"[..], b"thr!\n", b"four, longer\n"]
        .iter()
        .enumerate()
    {
        let r = atomic(body, Expect::Stamp(s));
        assert_eq!(r.method, SaveMethod::Atomic);
        s = check(&fsys, &p, &r, &format!("置換 {i}"));
        assert_eq!(fs::read(p.as_path()).expect("読む"), *body);
    }
    // 直接上書きを 2 回続ける (改名しないので識別は変わらない)
    for (i, body) in [&b"five\n"[..], b"six, much longer\n"].iter().enumerate() {
        let r = in_place(body, Expect::Stamp(s));
        assert_eq!(r.method, SaveMethod::InPlace);
        s = check(&fsys, &p, &r, &format!("直接上書き {i}"));
        assert_eq!(fs::read(p.as_path()).expect("読む"), *body);
    }
    // 直接上書きの後の置換も通る
    let r = atomic(b"seven\n", Expect::Stamp(s));
    check(&fsys, &p, &r, "直接上書きの後の置換");
}
