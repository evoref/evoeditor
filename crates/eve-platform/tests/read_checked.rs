//! `FileSystem::read_checked` の OS ごとの試験 (docs/07 §12.6.1 の「L0 (所有者の検査)」、07 §12.1 の OWN-1〜OWN-6、05 §5.7)。
//! OS に依らない振る舞いは `tests/fs_contract.rs` (eve-testkit の契約試験) で確かめる。ここは OS の ACL・mode・FS の種類で
//! 作る場合だけを確かめる。
//!
//! 前提: 一時ディレクトリが所有者を判定できる FS (Windows は `%TEMP%` の NTFS、Unix は tmpfs・ext4 など) にあり、私的
//! (自分の所有で他人が書けない) であること。各試験の最初に一時ディレクトリそのものを判定し、満たさなければ理由を示して
//! 止める (CI では失敗にする)。
//! root で動かしたとき・exFAT の試験は CI では走らない (09 §8.2 の手の確認で確かめる。07 §12.6.3)。

#![allow(clippy::print_stderr)]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use eve_core::{
    CheckedObject, CheckedRead, DirLinks, FileSystem, FsErrorKind, FsPath, OwnerVerdict,
    WORKSPACE_CONFIG_DIR,
};
#[cfg(windows)]
use eve_platform::fs::WinFs as NativeFs;
#[cfg(unix)]
use eve_platform::unix_fs::UnixFs as NativeFs;

const CFG: &str = "config.toml";
const NO_SAFE: &[PathBuf] = &[];

fn in_ci() -> bool {
    std::env::var_os("GITHUB_ACTIONS").is_some()
}

/// 一時ディレクトリ (終わりに消す。Unix は mode を戻してから消す。Windows の拒否の ACE は付けた試験が外す)。
struct Tmp(PathBuf);

impl Tmp {
    /// 作って、前提 (所有者を判定できる FS で、私的) を確かめる。満たさなければ `None` (CI では panic)。
    fn new(tag: &str) -> Option<Tmp> {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("eve-owner-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        let t = Tmp(p);
        fs::write(t.0.join("probe.toml"), b"p").unwrap();
        let r = rc(&t.0, "probe.toml", NO_SAFE);
        if r != read(b"p") {
            let msg = format!(
                "一時ディレクトリ {:?} が前提を満たさない (所有者を判定できる FS で、自分の所有で他人が書けないこと): {r:?}",
                t.0
            );
            assert!(!in_ci(), "{msg}");
            eprintln!("{msg}: 確かめない");
            return None;
        }
        Some(t)
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

/// `dir` に `.eve` の要素があれば `NoFollowFromEve`、無ければ `Follow` で読む。`safe` は安全な場所の一覧。
fn rc(dir: &Path, name: &str, safe: &[PathBuf]) -> CheckedRead {
    try_rc(dir, name, safe).unwrap_or_else(|e| panic!("read_checked({dir:?}, {name}): {e:?}"))
}

fn try_rc(dir: &Path, name: &str, safe: &[PathBuf]) -> Result<CheckedRead, FsErrorKind> {
    let links = if dir
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case(WORKSPACE_CONFIG_DIR))
    {
        DirLinks::NoFollowFromEve
    } else {
        DirLinks::Follow
    };
    let safe: Vec<FsPath> = safe.iter().map(FsPath::new).collect();
    NativeFs::default()
        .read_checked(&FsPath::new(dir), OsStr::new(name), 1 << 16, &safe, links)
        .map_err(|e| e.kind)
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

fn writers(r: &CheckedRead, what: CheckedObject) -> Vec<String> {
    match r {
        CheckedRead::Rejected {
            what: w,
            verdict: OwnerVerdict::OthersCanWrite { who },
        } if *w == what => who.clone(),
        other => panic!("{what:?} の OthersCanWrite でない: {other:?}"),
    }
}

/// ディレクトリのリンク (Windows はジャンクション、Unix はシンボリックリンク) を作る。
fn dir_link(link: &Path, target: &Path) {
    #[cfg(windows)]
    run(Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target))
    .expect("mklink /J");
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("symlink");
}

/// 他人に書かせる (Windows は `Everyone:(OI)(CI)M`、Unix は `o+w`)。
fn open_to_others(d: &Path) {
    #[cfg(windows)]
    run(Command::new("icacls")
        .arg(d)
        .args(["/grant", "*S-1-1-0:(OI)(CI)M"]))
    .expect("icacls");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(d, fs::Permissions::from_mode(0o777)).unwrap();
    }
}

#[test]
fn 自分で作ったディレクトリのファイルは読む() {
    let Some(t) = Tmp::new("own") else { return };
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"a = 1\n").unwrap();
    assert_eq!(rc(&d, CFG, NO_SAFE), read(b"a = 1\n"));
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
        rc(&sys, ".editorconfig-eve-none", NO_SAFE),
        CheckedRead::Missing
    );
}

/// 安全な場所の下でも、リンクの先が安全な場所の外なら判定する (開いたディレクトリの最終のパスで比べる。OWN-5)。
/// 安全な場所はリンクを解いた形で比べるので、リンクを含む形で書いても、その中の本当の場所は判定を省く。
#[test]
fn 安全な場所の下のリンクは辿った先で比べる() {
    let Some(t) = Tmp::new("safe") else { return };
    let outside = t.dir("outside");
    fs::write(outside.join(CFG), b"x").unwrap();
    open_to_others(&outside);
    let safe = t.dir("safe");
    let alias = safe.join("alias");
    dir_link(&alias, &outside);
    let r = rc(&alias, CFG, std::slice::from_ref(&safe));
    assert_ne!(writers(&r, CheckedObject::Dir), Vec::<String>::new());
    // 安全な場所そのものと、その下のふつうのディレクトリは判定を省く
    assert_eq!(
        rc(&outside, CFG, std::slice::from_ref(&outside)),
        read(b"x")
    );
    // 安全な場所をリンクの形で書いても、リンクを解いた形 (outside) で比べる
    assert_eq!(rc(&outside, CFG, std::slice::from_ref(&alias)), read(b"x"));
    assert_eq!(rc(&alias, CFG, std::slice::from_ref(&alias)), read(b"x"));
}

/// 判定と開く間に読むファイルを差し替えても、リンクを辿らない (Unix はディレクトリの記述子から `openat(O_NOFOLLOW)`、
/// Windows は `FILE_FLAG_OPEN_REPARSE_POINT`)。差し替えは別のスレッドで繰り返し、競争の窓に当たるかは確かめない
/// (同じプロセスの中で窓に当てる試験は `src/read_checked.rs` の単体試験のフック)。
#[test]
fn 名前を差し替え続けてもリンクの先を読まない() {
    let Some(t) = Tmp::new("race") else { return };
    let d = t.dir("ws");
    let secret = t.dir("secret");
    fs::write(secret.join(CFG), b"secret").unwrap();
    fs::write(d.join(CFG), b"x").unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker = {
        let (d, secret, stop) = (d.clone(), secret.clone(), stop.clone());
        // 試験だけの差し替えのスレッド
        #[allow(clippy::disallowed_methods)]
        std::thread::spawn(move || {
            let mut made_link = false;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = fs::remove_file(d.join(CFG));
                #[cfg(windows)]
                {
                    made_link |=
                        std::os::windows::fs::symlink_file(secret.join(CFG), d.join(CFG)).is_ok();
                }
                #[cfg(unix)]
                {
                    made_link |= std::os::unix::fs::symlink(secret.join(CFG), d.join(CFG)).is_ok();
                }
                let _ = fs::remove_file(d.join(CFG));
                let _ = fs::write(d.join(CFG), b"x");
            }
            made_link
        })
    };
    for _ in 0..300 {
        if let Ok(CheckedRead::Read { bytes }) = try_rc(&d, CFG, NO_SAFE) {
            assert_ne!(bytes, b"secret", "リンクの先を読んだ");
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    if !worker.join().unwrap() {
        eprintln!("シンボリックリンクを作れない環境: 差し替えの試験にならない");
        assert!(!in_ci() || cfg!(windows), "CI の Unix でリンクを作れない");
    }
}

// ---------- Windows ----------

#[cfg(windows)]
fn icacls(p: &Path, args: &[&str]) {
    run(Command::new("icacls").arg(p).args(args)).expect("icacls");
}

/// ドライブの直下から継承する `Authenticated Users` の書込も他人の書込に数える (S-25 の W7、OWN-2 の (2))。
#[cfg(windows)]
#[test]
fn authenticated_users_の書込は読まない() {
    let Some(t) = Tmp::new("authusers") else {
        return;
    };
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    icacls(&d, &["/grant", "*S-1-5-11:(OI)(CI)M"]);
    let who = writers(&rc(&d, CFG, NO_SAFE), CheckedObject::Dir);
    assert_eq!(who.len(), 1, "{who:?}");
    assert!(
        who[0].contains("Authenticated Users") || who[0] == "S-1-5-11",
        "{who:?}"
    );
    assert_eq!(rc(&d, CFG, std::slice::from_ref(&d)), read(b"x"));
}

/// ファイルだけに `Everyone:M` (S-25 の W3d): ディレクトリは通り、ファイルで落ちる。属性の書込 (`WA`) だけでも落ちる。
#[cfg(windows)]
#[test]
fn ファイルだけの_everyone_の書込は読まない() {
    let Some(t) = Tmp::new("filew") else { return };
    let d = t.dir("ws");
    fs::write(d.join(".editorconfig"), b"root = true\n").unwrap();
    icacls(&d.join(".editorconfig"), &["/grant", "*S-1-1-0:M"]);
    let who = writers(&rc(&d, ".editorconfig", NO_SAFE), CheckedObject::File);
    assert_eq!(who.len(), 1, "{who:?}");
    fs::write(d.join(CFG), b"x").unwrap();
    icacls(&d.join(CFG), &["/grant", "*S-1-1-0:(WA)"]);
    assert_eq!(writers(&rc(&d, CFG, NO_SAFE), CheckedObject::File).len(), 1);
}

/// 拒否の ACE だけ (`OneDrive` の `Everyone:(CI)(DENY)(DC)` の形、S-25 の W3e) は数えない。読むだけの許可も数えない。
#[cfg(windows)]
#[test]
fn 拒否と読むだけの_ace_は読む() {
    let Some(t) = Tmp::new("deny") else { return };
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    icacls(&d, &["/deny", "*S-1-1-0:(CI)(DC)"]);
    icacls(&d, &["/grant", "*S-1-5-11:(OI)(CI)RX"]);
    assert_eq!(rc(&d, CFG, NO_SAFE), read(b"x"));
    icacls(&d, &["/remove:d", "*S-1-1-0"]);
}

/// 継承専用の ACE はそのディレクトリに効かないので数えない。継承したファイルの側で落ちる。
#[cfg(windows)]
#[test]
fn 継承専用の_ace_はディレクトリに数えずファイルで落ちる() {
    let Some(t) = Tmp::new("inherit") else { return };
    let d = t.dir("ws");
    icacls(&d, &["/grant", "*S-1-1-0:(OI)(IO)M"]);
    fs::write(d.join(CFG), b"x").unwrap();
    let who = writers(&rc(&d, CFG, NO_SAFE), CheckedObject::File);
    assert_eq!(who.len(), 1, "{who:?}");
}

/// 祖先のジャンクションは辿る (開いたファイルの親と固定したディレクトリの最終のパスが同じになる)。
#[cfg(windows)]
#[test]
fn 祖先のジャンクションを通ったパスは読む() {
    let Some(t) = Tmp::new("junction") else {
        return;
    };
    let real = t.dir("real");
    let sub = real.join("sub");
    fs::create_dir_all(sub.join(".eve")).unwrap();
    fs::write(sub.join(CFG), b"x").unwrap();
    fs::write(sub.join(".eve").join(CFG), b"e").unwrap();
    let link = t.0.join("link");
    dir_link(&link, &real);
    assert_eq!(rc(&link.join("sub"), CFG, NO_SAFE), read(b"x"));
    assert_eq!(rc(&link.join("sub").join(".eve"), CFG, NO_SAFE), read(b"e"));
    // 置き場所そのものがジャンクション (`.eve` でない) でも辿る
    assert_eq!(rc(&link, "none.toml", NO_SAFE), CheckedRead::Missing);
}

/// Win32 の正規化が落とす形 (末尾の `.`・空白) と `:` の要素は `InvalidPath` (`.eve.` が `.eve` を開いて、リンクを辿らない
/// 要素の見分けを外せる)。
#[cfg(windows)]
#[test]
fn 末尾の_点と空白とコロンの要素は受け付けない() {
    let Some(t) = Tmp::new("trail") else { return };
    let w = t.dir("w");
    fs::create_dir_all(w.join(".eve")).unwrap();
    for dir in [
        w.join(".eve."),
        w.join(".eve "),
        w.join(".eve").join("x."),
        // `join("a:b")` はドライブの `a:` として前を捨てるので、文字列で繋ぐ
        PathBuf::from(format!(r"{}\a:b", w.display())),
        PathBuf::from(format!("{}.", w.display())),
    ] {
        assert_eq!(
            try_rc(&dir, CFG, NO_SAFE),
            Err(FsErrorKind::InvalidPath),
            "{dir:?}"
        );
    }
    for name in ["config.toml.", "config.toml ", "config.toml:s"] {
        assert_eq!(
            try_rc(&w.join(".eve"), name, NO_SAFE),
            Err(FsErrorKind::InvalidPath),
            "{name:?}"
        );
    }
    // 大文字小文字を変えた `.EVE` も `.eve` として辿らない
    let jun = t.dir("j");
    dir_link(&jun.join(".EVE"), &w);
    assert_eq!(
        rc(&jun.join(".EVE"), CFG, NO_SAFE),
        CheckedRead::Rejected {
            what: CheckedObject::Dir,
            verdict: OwnerVerdict::NotRegular
        }
    );
}

/// UNC を指すシンボリックリンクは、開く前に UNC へ出ずに `NotRegular` で読まない (OWN-6。辿ると認証が走り、届かない
/// アドレスでは数十秒止まる)。シンボリックリンクを作れない開発機では確かめない (CI では作れる)。
#[cfg(windows)]
#[test]
fn unc_を指すシンボリックリンクは辿らない() {
    let Some(t) = Tmp::new("unc") else { return };
    let d = t.dir("ws");
    // 文書用の IP アドレス (RFC 5737)。辿れば接続を待って止まる
    let target = Path::new(r"\\192.0.2.1\share\config.toml");
    let eve_target = Path::new(r"\\192.0.2.1\share\.eve");
    if let Err(e) = std::os::windows::fs::symlink_file(target, d.join(CFG)) {
        assert!(!in_ci(), "CI でシンボリックリンクを作れない: {e}");
        eprintln!("シンボリックリンクを作れない環境 ({e}): 確かめない");
        return;
    }
    std::os::windows::fs::symlink_dir(eve_target, d.join(".eve")).unwrap();
    let start = std::time::Instant::now();
    for safe in [NO_SAFE, std::slice::from_ref(&d)] {
        assert_eq!(
            rc(&d, CFG, safe),
            CheckedRead::Rejected {
                what: CheckedObject::File,
                verdict: OwnerVerdict::NotRegular
            }
        );
        assert_eq!(
            rc(&d.join(".eve"), CFG, safe),
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
    let Some(t) = Tmp::new("share") else { return };
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    {
        // FILE_SHARE_READ | FILE_SHARE_WRITE で書込のために開いている
        let _w = fs::OpenOptions::new()
            .write(true)
            .share_mode(0x1 | 0x2)
            .open(d.join(CFG))
            .unwrap();
        assert_eq!(rc(&d, CFG, NO_SAFE), read(b"x"));
    }
    let _x = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(d.join(CFG))
        .unwrap();
    assert_eq!(try_rc(&d, CFG, NO_SAFE), Err(FsErrorKind::Busy));
}

/// 所有者を記録しない FS (exFAT・FAT32) は判定できない (OWN-3)。`EVE_TEST_NOACL_DIR` にそういうボリュームの空の
/// ディレクトリを指したときだけ確かめる (CI には無い。09 §8.2 の手の確認)。
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
    let r = rc(&d, CFG, NO_SAFE);
    let skip = rc(&d, CFG, std::slice::from_ref(&d));
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

/// クラウドのファイル (Cloud Files の再解析点) を、識別で開き直して読める (PO の決定 2026-11-23 の B)。
/// `EVE_TEST_CLOUD_FILE` に、同期済みの小さなファイル (`OneDrive` の下) を指したときだけ確かめる (CI には無い)。
/// 中身は表示しない。
#[cfg(windows)]
#[test]
fn クラウドのファイルを識別で開き直して読む() {
    let Some(file) = std::env::var_os("EVE_TEST_CLOUD_FILE").map(PathBuf::from) else {
        eprintln!("EVE_TEST_CLOUD_FILE が無い: 確かめない");
        return;
    };
    let dir = file.parent().expect("親");
    let name = file
        .file_name()
        .expect("名前")
        .to_string_lossy()
        .into_owned();
    let want = fs::read(&file).expect("std で読める");
    match rc(dir, &name, &[dir.to_path_buf()]) {
        CheckedRead::Read { bytes } => assert!(bytes == want, "中身が std で読んだものと違う"),
        CheckedRead::TooLarge => eprintln!("大きすぎる (読まなかった)"),
        other => panic!("{other:?}"),
    }
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
    let d = std::env::temp_dir().join(format!("eve-owner-uid-{}", std::process::id()));
    fs::create_dir_all(&d).unwrap();
    let uid = fs::metadata(&d).unwrap().uid();
    let _ = fs::remove_dir_all(&d);
    uid
}

/// sticky でも `o+w` なら読まない (`/tmp` の形。S-25 の U3b)。`g+w` は見ない。
#[cfg(unix)]
#[test]
fn sticky_でも_o_w_は読まず_g_w_は読む() {
    let Some(t) = Tmp::new("sticky") else { return };
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    chmod(&d, 0o1777);
    assert_eq!(
        writers(&rc(&d, CFG, NO_SAFE), CheckedObject::Dir),
        ["others"]
    );
    chmod(&d, 0o775);
    chmod(&d.join(CFG), 0o664);
    assert_eq!(rc(&d, CFG, NO_SAFE), read(b"x"));
}

/// 拡張 ACL (`setfacl -m u:nobody:rwx`) は `st_mode` に `o+w` が出ないまま他人が書ける。中身を見ずに読まない (OWN-2 の (2))。
/// `setfacl` も python3 も無い・FS が ACL を持たない環境では確かめない (CI では失敗にする)。
#[cfg(unix)]
#[test]
fn 拡張_acl_があれば読まない() {
    let Some(t) = Tmp::new("acl") else { return };
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
        assert!(!in_ci(), "CI で拡張 ACL を付けられない: {e}");
        eprintln!("拡張 ACL を付けられない ({e}): 確かめない");
        return;
    }
    assert_eq!(writers(&rc(&d, CFG, NO_SAFE), CheckedObject::Dir), ["acl"]);
    assert_eq!(rc(&d, CFG, std::slice::from_ref(&d)), read(b"x"));
}

/// 開けないディレクトリは `PermissionDenied` (root は権限を見ないので確かめない)。
#[cfg(unix)]
#[test]
fn 開けないディレクトリは_permission_denied() {
    if euid() == 0 {
        return;
    }
    let Some(t) = Tmp::new("denied") else { return };
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    chmod(&d, 0o000);
    let r = try_rc(&d, CFG, NO_SAFE);
    chmod(&d, 0o755);
    assert_eq!(r, Err(FsErrorKind::PermissionDenied));
}

/// 別の所有者 (root で走らせたときだけ。CI では走らない。09 §8.2 の手の確認): root で動かしたときの「自分」は uid 0 だけ
/// (OWN-4)。
#[cfg(unix)]
#[test]
fn root_で動かすとほかの_uid_の所有は読まない() {
    if euid() != 0 {
        eprintln!("root でない: 確かめない");
        return;
    }
    let Some(t) = Tmp::new("root") else { return };
    let d = t.dir("ws");
    fs::write(d.join(CFG), b"x").unwrap();
    std::os::unix::fs::chown(&d, Some(12_345), None).unwrap();
    assert!(matches!(
        rc(&d, CFG, NO_SAFE),
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
    match rc(win, "win.ini", NO_SAFE) {
        CheckedRead::Rejected {
            what: CheckedObject::Dir,
            verdict: OwnerVerdict::Unknown { fs },
        } => assert!(fs == "9p" || fs == "fuse", "{fs}"),
        other => panic!("{other:?}"),
    }
}
