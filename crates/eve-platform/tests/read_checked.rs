//! `FileSystem::read_checked` の OS ごとの試験 (docs/07 §12.6.1 の「L0 (所有者の検査)」、07 §12.1 の OWN-1〜OWN-6、05 §5.7)。
//! OS に依らない振る舞いは `tests/fs_contract.rs` (eve-testkit の契約試験) で確かめる。ここは OS の ACL・mode・FS の種類で
//! 作る場合だけを確かめる。一時ディレクトリは所有者を判定できる FS (Windows は `%TEMP%` の NTFS、Unix は tmpfs・ext4 など)
//! にあること。

#![allow(clippy::print_stderr)]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use eve_core::{CheckedObject, CheckedRead, FileSystem, FsPath, OwnerCheck, OwnerVerdict};
#[cfg(windows)]
use eve_platform::fs::WinFs as NativeFs;
#[cfg(unix)]
use eve_platform::unix_fs::UnixFs as NativeFs;

const CFG: &str = "config.toml";

/// 一時ディレクトリ (終わりに消す。ACL を付けたものも消せるよう、Windows は先に継承を戻す)。
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("eve-owner-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }

    fn dir(&self, name: &str) -> PathBuf {
        let d = self.0.join(name);
        fs::create_dir_all(&d).unwrap();
        d
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for e in walk(&self.0) {
                let _ = fs::set_permissions(&e, fs::Permissions::from_mode(0o755));
            }
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn walk(p: &Path) -> Vec<PathBuf> {
    let mut out = vec![p.to_path_buf()];
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                out.extend(walk(&e.path()));
            }
        }
    }
    out
}

fn rc(dir: &Path, name: &str, check: OwnerCheck) -> CheckedRead {
    NativeFs::default()
        .read_checked(&FsPath::new(dir), OsStr::new(name), 1 << 16, check)
        .unwrap_or_else(|e| panic!("read_checked({dir:?}, {name}): {e:?}"))
}

fn read(bytes: &[u8]) -> CheckedRead {
    CheckedRead::Read {
        bytes: bytes.to_vec(),
    }
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

#[test]
fn 自分で作ったディレクトリのファイルは読む() {
    let t = Tmp::new("own");
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"a = 1\n").unwrap();
    assert_eq!(rc(&d, CFG, OwnerCheck::Check), read(b"a = 1\n"));
}

/// ファイル・ディレクトリが無い所の `.editorconfig` の祖先探索で、`/tmp`・`C:\` のような他人が書けるディレクトリに
/// 無いだけでは診断を出さない (OWN-6 の「まず有無を確かめる」)。
#[test]
fn 他人が書けるシステムのディレクトリに無いファイルは_missing() {
    let sys = if cfg!(windows) {
        PathBuf::from(format!(
            "{}\\",
            std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned())
        ))
    } else {
        PathBuf::from("/tmp")
    };
    assert_eq!(
        rc(&sys, ".editorconfig-eve-none", OwnerCheck::Check),
        CheckedRead::Missing
    );
}

// ---------- Windows ----------

#[cfg(windows)]
fn icacls(p: &Path, args: &[&str]) {
    run(Command::new("icacls").arg(p).args(args)).expect("icacls");
}

#[cfg(windows)]
fn writers(r: &CheckedRead, what: CheckedObject) -> Vec<String> {
    match r {
        CheckedRead::Rejected {
            what: w,
            verdict: OwnerVerdict::OthersCanWrite { who },
        } if *w == what => who.clone(),
        other => panic!("{what:?} の OthersCanWrite でない: {other:?}"),
    }
}

/// ドライブの直下から継承する `Authenticated Users` の書込も他人の書込に数える (S-25 の W7、OWN-2 の (2))。
#[cfg(windows)]
#[test]
fn authenticated_users_の書込は読まない() {
    let t = Tmp::new("authusers");
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    icacls(&d, &["/grant", "*S-1-5-11:(OI)(CI)M"]);
    let who = writers(&rc(&d, CFG, OwnerCheck::Check), CheckedObject::Dir);
    assert_eq!(who.len(), 1, "{who:?}");
    assert!(
        who[0].contains("Authenticated Users") || who[0] == "S-1-5-11",
        "{who:?}"
    );
    assert_eq!(rc(&d, CFG, OwnerCheck::Skip), read(b"x"));
}

/// ファイルだけに `Everyone:M` (S-25 の W3d): ディレクトリは通り、ファイルで落ちる。
#[cfg(windows)]
#[test]
fn ファイルだけの_everyone_の書込は読まない() {
    let t = Tmp::new("filew");
    let d = t.dir("ws");
    fs::write(d.join(".editorconfig"), b"root = true\n").unwrap();
    icacls(&d.join(".editorconfig"), &["/grant", "*S-1-1-0:M"]);
    let who = writers(
        &rc(&d, ".editorconfig", OwnerCheck::Check),
        CheckedObject::File,
    );
    assert_eq!(who.len(), 1, "{who:?}");
}

/// 拒否の ACE だけ (`OneDrive` の `Everyone:(CI)(DENY)(DC)` の形、S-25 の W3e) は数えない。読むだけの許可も数えない。
#[cfg(windows)]
#[test]
fn 拒否と読むだけの_ace_は読む() {
    let t = Tmp::new("deny");
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    icacls(&d, &["/deny", "*S-1-1-0:(CI)(DC)"]);
    icacls(&d, &["/grant", "*S-1-5-11:(OI)(CI)RX"]);
    assert_eq!(rc(&d, CFG, OwnerCheck::Check), read(b"x"));
    icacls(&d, &["/remove:d", "*S-1-1-0"]);
}

/// 継承専用の ACE はそのディレクトリに効かないので数えない。継承したファイルの側で落ちる。
#[cfg(windows)]
#[test]
fn 継承専用の_ace_はディレクトリに数えずファイルで落ちる() {
    let t = Tmp::new("inherit");
    let d = t.dir("ws");
    icacls(&d, &["/grant", "*S-1-1-0:(OI)(IO)M"]);
    fs::write(d.join(CFG), b"x").unwrap();
    let who = writers(&rc(&d, CFG, OwnerCheck::Check), CheckedObject::File);
    assert_eq!(who.len(), 1, "{who:?}");
}

/// 祖先のジャンクションは辿る (開いたファイルの親と固定したディレクトリの最終のパスが同じになる)。
#[cfg(windows)]
#[test]
fn 祖先のジャンクションを通ったパスは読む() {
    let t = Tmp::new("junction");
    let real = t.dir("real");
    let sub = real.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join(CFG), b"x").unwrap();
    let link = t.0.join("link");
    run(Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&real))
    .expect("mklink /J");
    assert_eq!(rc(&link.join("sub"), CFG, OwnerCheck::Check), read(b"x"));
    // 置き場所そのものがジャンクション (`.eve` でない) でも辿る
    assert_eq!(
        rc(&link, "none.toml", OwnerCheck::Check),
        CheckedRead::Missing
    );
}

/// UNC を指すシンボリックリンクは、開く前に UNC へ出ずに `NotRegular` で読まない (OWN-6。辿ると認証が走り、届かない
/// アドレスでは数十秒止まる)。シンボリックリンクを作れない開発機では確かめない (CI では作れる)。
#[cfg(windows)]
#[test]
fn unc_を指すシンボリックリンクは辿らない() {
    let t = Tmp::new("unc");
    let d = t.dir("ws");
    // 文書用の IP アドレス (RFC 5737)。辿れば接続を待って止まる
    let target = Path::new(r"\\192.0.2.1\share\config.toml");
    let eve_target = Path::new(r"\\192.0.2.1\share\.eve");
    if let Err(e) = std::os::windows::fs::symlink_file(target, d.join(CFG)) {
        assert!(
            std::env::var_os("GITHUB_ACTIONS").is_none(),
            "CI でシンボリックリンクを作れない: {e}"
        );
        eprintln!("シンボリックリンクを作れない環境 ({e}): 確かめない");
        return;
    }
    std::os::windows::fs::symlink_dir(eve_target, d.join(".eve")).unwrap();
    let start = std::time::Instant::now();
    for check in [OwnerCheck::Check, OwnerCheck::Skip] {
        assert_eq!(
            rc(&d, CFG, check),
            CheckedRead::Rejected {
                what: CheckedObject::File,
                verdict: OwnerVerdict::NotRegular
            }
        );
        assert_eq!(
            rc(&d.join(".eve"), CFG, check),
            CheckedRead::Rejected {
                what: CheckedObject::Dir,
                verdict: OwnerVerdict::NotRegular
            }
        );
    }
    assert!(
        start.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
}

/// ほかのエディタが書込のために開いていても読める (共有違反にしない)。共有を許さずに開かれていれば `Busy` (一時の失敗)。
#[cfg(windows)]
#[test]
fn 開かれているファイルの共有() {
    use std::os::windows::fs::OpenOptionsExt;
    let t = Tmp::new("share");
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    {
        // FILE_SHARE_READ | FILE_SHARE_WRITE で書込のために開いている
        let _w = fs::OpenOptions::new()
            .write(true)
            .share_mode(0x1 | 0x2)
            .open(d.join(CFG))
            .unwrap();
        assert_eq!(rc(&d, CFG, OwnerCheck::Check), read(b"x"));
    }
    let _x = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(d.join(CFG))
        .unwrap();
    let e = NativeFs::default()
        .read_checked(&FsPath::new(&d), OsStr::new(CFG), 1024, OwnerCheck::Check)
        .unwrap_err();
    assert_eq!(e.kind, eve_core::FsErrorKind::Busy);
}

/// 所有者を記録しない FS (exFAT・FAT32) は判定できない (OWN-3)。`EVE_TEST_NOACL_DIR` にそういうボリュームの空の
/// ディレクトリを指したときだけ確かめる (CI には無い。手元の exFAT のドライブで確かめる)。
#[cfg(windows)]
#[test]
fn acl_の無い_fs_は判定できない() {
    let Some(base) = std::env::var_os("EVE_TEST_NOACL_DIR") else {
        eprintln!("EVE_TEST_NOACL_DIR が無い: 確かめない");
        return;
    };
    let d = PathBuf::from(base).join(format!("eve-noacl-{}", std::process::id()));
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join(CFG), b"x").unwrap();
    let r = rc(&d, CFG, OwnerCheck::Check);
    let skip = rc(&d, CFG, OwnerCheck::Skip);
    let _ = fs::remove_dir_all(&d);
    match r {
        CheckedRead::Rejected {
            what: CheckedObject::Dir,
            verdict: OwnerVerdict::Unknown { fs },
        } => assert!(fs.to_ascii_uppercase().contains("FAT"), "{fs}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(skip, read(b"x"), "workspace.safe_directories の下なら読む");
}

// ---------- Unix ----------

#[cfg(unix)]
fn chmod(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
}

/// 実効 uid (作ったディレクトリの所有者)。
#[cfg(unix)]
fn euid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    let t = Tmp::new("uid");
    fs::metadata(&t.0).unwrap().uid()
}

/// sticky でも `o+w` なら読まない (`/tmp` の形。S-25 の U3b)。`g+w` は見ない。
#[cfg(unix)]
#[test]
fn sticky_でも_o_w_は読まず_g_w_は読む() {
    let t = Tmp::new("sticky");
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    chmod(&d, 0o1777);
    assert_eq!(
        rc(&d, CFG, OwnerCheck::Check),
        CheckedRead::Rejected {
            what: CheckedObject::Dir,
            verdict: OwnerVerdict::OthersCanWrite {
                who: vec!["others".to_owned()]
            }
        }
    );
    chmod(&d, 0o775);
    chmod(&d.join(CFG), 0o664);
    assert_eq!(rc(&d, CFG, OwnerCheck::Check), read(b"x"));
}

/// 拡張 ACL (`setfacl -m u:nobody:rwx`) は `st_mode` に `o+w` が出ないまま他人が書ける。中身を見ずに読まない (OWN-2 の (2))。
/// `setfacl` が無い・FS が ACL を持たない環境では確かめない。
#[cfg(unix)]
#[test]
fn 拡張_acl_があれば読まない() {
    let t = Tmp::new("acl");
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    // setfacl が無い環境 (acl のパッケージを入れていない WSL など) は、python3 で同じ ACL (u:nobody:rwx と mask) の
    // 拡張属性を書く (system.posix_acl_access の形: 版 2 と、(tag u16, perm u16, id u32) の並び)
    const PY: &str = "import os,struct,sys\n\
        e=lambda t,p,i: struct.pack('<HHI',t,p,i)\n\
        b=struct.pack('<I',2)+e(1,7,0xFFFFFFFF)+e(2,7,65534)+e(4,5,0xFFFFFFFF)+e(0x10,7,0xFFFFFFFF)+e(0x20,5,0xFFFFFFFF)\n\
        os.setxattr(sys.argv[1],'system.posix_acl_access',b)";
    let set = run(Command::new("setfacl").args(["-m", "u:nobody:rwx"]).arg(&d))
        .or_else(|_| run(Command::new("python3").args(["-c", PY]).arg(&d)));
    if let Err(e) = set {
        eprintln!("拡張 ACL を付けられない ({e}): 確かめない");
        return;
    }
    assert_eq!(
        rc(&d, CFG, OwnerCheck::Check),
        CheckedRead::Rejected {
            what: CheckedObject::Dir,
            verdict: OwnerVerdict::OthersCanWrite {
                who: vec!["acl".to_owned()]
            }
        }
    );
    assert_eq!(rc(&d, CFG, OwnerCheck::Skip), read(b"x"));
}

/// 開けないディレクトリは `PermissionDenied` (root は権限を見ないので確かめない)。
#[cfg(unix)]
#[test]
fn 開けないディレクトリは_permission_denied() {
    if euid() == 0 {
        return;
    }
    let t = Tmp::new("denied");
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    chmod(&d, 0o000);
    let e = NativeFs::default()
        .read_checked(&FsPath::new(&d), OsStr::new(CFG), 1024, OwnerCheck::Check)
        .unwrap_err();
    chmod(&d, 0o755);
    assert_eq!(e.kind, eve_core::FsErrorKind::PermissionDenied);
}

/// 別の所有者 (root で走らせたときだけ。CI の Linux を root で走らせる構成で確かめる): root で動かしたときの「自分」は
/// uid 0 だけ (OWN-4)。
#[cfg(unix)]
#[test]
fn root_で動かすとほかの_uid_の所有は読まない() {
    if euid() != 0 {
        eprintln!("root でない: 確かめない");
        return;
    }
    let t = Tmp::new("root");
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    std::os::unix::fs::chown(&d, Some(65534), None).unwrap();
    assert!(matches!(
        rc(&d, CFG, OwnerCheck::Check),
        CheckedRead::Rejected {
            what: CheckedObject::Dir,
            verdict: OwnerVerdict::NotOwner { .. }
        }
    ));
}

/// WSL2 の `/mnt/c` (DrvFs、9p) は所有者も mode も写像で、判定できない (OWN-3。S-25 の U6)。WSL でだけ確かめる。
#[cfg(unix)]
#[test]
fn wsl_の_drvfs_は判定できない() {
    let win = Path::new("/mnt/c/Windows");
    if !win.join("win.ini").exists() {
        eprintln!("/mnt/c/Windows/win.ini が無い (WSL でない): 確かめない");
        return;
    }
    match rc(win, "win.ini", OwnerCheck::Check) {
        CheckedRead::Rejected {
            what: CheckedObject::Dir,
            verdict: OwnerVerdict::Unknown { fs },
        } => assert!(fs == "9p" || fs == "fuse", "{fs}"),
        other => panic!("{other:?}"),
    }
}
