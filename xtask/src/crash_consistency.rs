//! `cargo xtask crash-consistency`: 保存のクラッシュ一貫性の検査 (NFR-301 の「電源断」、docs/09 §6.3、OQ-143、Linux)。
//! **LazyFS** (FUSE。fsync されていない書込を自前のキャッシュに持ち、`lazyfs::clear-cache` で落とせる) の上で、
//! `eve-test-helpers save` を保存手順の各切断点で強制終了させ (`EVE_TEST_HOOK`)、キャッシュを落として電源断を模擬し、
//! 「元ファイルまたは新ファイルが完全に残る」ことを確かめる。直接上書きは、途中で落ちても完全なバックアップと保存中フラグが
//! 残っていれば合格 (起動時の走査が復旧を提案できる、05 §4.4)。
//!
//! LazyFS が落とすのはデータだけで、メタデータ (作成・rename) は下の FS にすぐ反映される。そこでこの検査が見つけられるのは
//! 「rename / 完了の前に fsync していない」誤りで、ディレクトリの fsync の欠落は見つけられない (09 §6.3)。
//! 道具が効いていることは、最初に **対照** (fsync せずに rename したファイルの内容が落ちること) で確かめる。
//!
//! ```text
//! cargo xtask crash-consistency --lazyfs <lazyfs の実行ファイル> [--root <下の FS のディレクトリ>]...
//! cargo xtask crash-consistency --no-lazyfs     # 空回し (電源断を模擬しない。切断点と判定の組み立ての確認)
//! ```
//!
//! `--lazyfs` を省くと環境変数 `LAZYFS_BIN`、`--root` を省くと環境変数 `EVE_CRASH_ROOTS` (`:` 区切り。夜間ジョブが
//! ext4 / xfs / btrfs のループバックを渡す)、それも無ければ `target/crash-consistency/root` を使う。

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::util::{run, workspace_root};

const ORIG: &[u8] = b"original content of the file\n";
const NEW: &[u8] = b"NEW content that is somewhat longer than the original one\n";

/// 直接上書きの元と新しい内容 (バックアップの途中 (1 MiB のチャンクの間) を作れる大きさ)。
fn big(seed: u8) -> Vec<u8> {
    (0..2 * 1024 * 1024u32)
        .map(|i| seed + (i % 26) as u8)
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// 既存のファイルの原子的保存
    Atomic,
    /// 新規のファイルの保存 (rename の先が無い)
    NewFile,
    /// 直接上書き (事前バックアップあり)
    InPlace,
}

struct Case {
    mode: Mode,
    /// 切断点 (`None` は最後まで保存する)
    hook: Option<&'static str>,
}

const CASES: &[Case] = &[
    Case {
        mode: Mode::Atomic,
        hook: Some("kill_after_tmp_write"),
    },
    Case {
        mode: Mode::Atomic,
        hook: Some("kill_after_flush"),
    },
    Case {
        mode: Mode::Atomic,
        hook: Some("kill_before_rename"),
    },
    Case {
        mode: Mode::Atomic,
        hook: Some("kill_after_rename"),
    },
    Case {
        mode: Mode::Atomic,
        hook: None,
    },
    Case {
        mode: Mode::NewFile,
        hook: Some("kill_before_rename"),
    },
    Case {
        mode: Mode::NewFile,
        hook: Some("kill_after_rename"),
    },
    Case {
        mode: Mode::NewFile,
        hook: None,
    },
    Case {
        mode: Mode::InPlace,
        hook: Some("kill_after_inflight_flag"),
    },
    Case {
        mode: Mode::InPlace,
        hook: Some("kill_mid_backup"),
    },
    Case {
        mode: Mode::InPlace,
        hook: Some("kill_after_backup"),
    },
    Case {
        mode: Mode::InPlace,
        hook: Some("kill_mid_inplace_write=100000"),
    },
    Case {
        mode: Mode::InPlace,
        hook: None,
    },
];

impl Case {
    fn name(&self) -> String {
        let mode = match self.mode {
            Mode::Atomic => "atomic",
            Mode::NewFile => "newfile",
            Mode::InPlace => "inplace",
        };
        let hook = self
            .hook
            .map_or("complete", |h| h.split('=').next().unwrap_or(h));
        format!("{mode}-{hook}")
    }

    fn contents(&self) -> (Vec<u8>, Vec<u8>) {
        match self.mode {
            Mode::InPlace => (big(b'a'), big(b'A')),
            _ => (ORIG.to_vec(), NEW.to_vec()),
        }
    }
}

/// 電源断の後に残ったもの。
#[derive(Debug, Default)]
struct After {
    /// 対象のファイルの内容 (無ければ `None`)
    target: Option<Vec<u8>>,
    /// 状態ディレクトリの完了したバックアップ (`backup/*.bak`) のどれかが元の内容とバイト一致する
    backup_complete: bool,
    /// 保存中フラグ (`recovery/inflight/*/*`) が残っている
    flag: bool,
}

/// 電源断の後の状態の判定 (NFR-301)。合格なら `Ok`、不合格なら理由。
fn judge(mode: Mode, completed: bool, orig: &[u8], new: &[u8], a: &After) -> Result<(), String> {
    let t = a.target.as_deref();
    let what = || match t {
        None => "無い".to_owned(),
        Some(b) if b == orig => "元の内容".to_owned(),
        Some(b) if b == new => "新しい内容".to_owned(),
        Some(b) => format!("どちらでもない {} バイト", b.len()),
    };
    if completed {
        // 保存が成功を返した後の電源断: 新しい内容が残っていなければならない
        return if t == Some(new) {
            Ok(())
        } else {
            Err(format!("保存が成功した後なのに、対象は {}", what()))
        };
    }
    let ok = match mode {
        Mode::Atomic => t == Some(orig) || t == Some(new),
        Mode::NewFile => t.is_none() || t == Some(new),
        Mode::InPlace => t == Some(orig) || t == Some(new) || (a.backup_complete && a.flag),
    };
    if ok {
        Ok(())
    } else if mode == Mode::InPlace {
        Err(format!(
            "対象は {} で、完全なバックアップ {} ・保存中フラグ {}",
            what(),
            if a.backup_complete {
                "あり"
            } else {
                "なし"
            },
            if a.flag { "あり" } else { "なし" }
        ))
    } else {
        Err(format!("対象は {}", what()))
    }
}

/// 引数の値 (`--name 値`) を集める。
fn values<'a>(args: &'a [String], name: &str) -> Vec<&'a str> {
    args.windows(2)
        .filter(|w| w[0] == name)
        .map(|w| w[1].as_str())
        .collect()
}

pub fn run_cmd(args: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("crash-consistency は Linux だけで動きます (LazyFS は FUSE)".into());
    }
    let lazyfs: Option<PathBuf> = if args.iter().any(|a| a == "--no-lazyfs") {
        None
    } else {
        let bin = values(args, "--lazyfs")
            .first()
            .map(|s| (*s).to_owned())
            .or_else(|| std::env::var("LAZYFS_BIN").ok())
            .ok_or("LazyFS の実行ファイルを --lazyfs か環境変数 LAZYFS_BIN で指してください (空回しは --no-lazyfs)")?;
        Some(PathBuf::from(bin))
    };
    let work = workspace_root().join("target").join("crash-consistency");
    let mut roots: Vec<PathBuf> = values(args, "--root")
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if roots.is_empty() {
        roots = std::env::var("EVE_CRASH_ROOTS")
            .ok()
            .map(|v| {
                v.split(':')
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
    }
    if roots.is_empty() {
        roots.push(work.join("root"));
    }

    println!("▶ crash-consistency (NFR-301)");
    run(
        "eve-test-helpers のビルド (test-hooks)",
        Command::new("cargo").args(["build", "-q", "-p", "eve-test-helpers"]),
    )?;
    let helper = workspace_root()
        .join("target")
        .join("debug")
        .join("eve-test-helpers");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;

    let mut failures: Vec<String> = Vec::new();
    for (i, root) in roots.iter().enumerate() {
        let label = root.display().to_string();
        let cases_dir = root.join(format!("eve-crash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&cases_dir);
        fs::create_dir_all(&cases_dir).map_err(|e| format!("{}: {e}", cases_dir.display()))?;
        let r = match &lazyfs {
            Some(bin) => {
                let mnt = work.join(format!("mnt-{i}"));
                Lazy::mount(bin, &cases_dir, &mnt, &work.join(format!("fs-{i}"))).and_then(|lz| {
                    let r = run_root(&helper, &work, &lz.mnt, Some(&lz));
                    lz.unmount();
                    r
                })
            }
            None => run_root(&helper, &work, &cases_dir, None),
        };
        let _ = fs::remove_dir_all(&cases_dir);
        match r {
            Ok(n) => println!("  {label}: {n} 件すべて合格"),
            Err(e) => failures.push(format!("{label}:\n{e}")),
        }
    }
    if lazyfs.is_none() {
        println!("  (空回し: 電源断は模擬していない)");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// 1 つの FS の上で、対照と全ての切断点を試す。合格した件数を返す。
fn run_root(helper: &Path, work: &Path, base: &Path, lz: Option<&Lazy>) -> Result<usize, String> {
    if let Some(lz) = lz {
        control(base, lz)?;
    }
    let mut errors: Vec<String> = Vec::new();
    for case in CASES {
        let name = case.name();
        if let Err(e) = run_case(helper, work, base, lz, case) {
            errors.push(format!("  - {name}: {e}"));
        }
    }
    if errors.is_empty() {
        Ok(CASES.len())
    } else {
        Err(errors.join("\n"))
    }
}

/// 対照: fsync せずに rename したファイルの内容が、キャッシュを落とすと失われること (LazyFS が効いている証拠)。
fn control(base: &Path, lz: &Lazy) -> Result<(), String> {
    let dir = base.join("control");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let target = dir.join("c.txt");
    write_synced(&target, ORIG)?;
    let tmp = dir.join(".c.txt.tmp");
    fs::write(&tmp, NEW).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    lz.clear_cache()?;
    let after = fs::read(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    if after == NEW {
        return Err(
            "対照: fsync していない内容がキャッシュを落としても残った (LazyFS が電源断を模擬していない)".into(),
        );
    }
    println!(
        "  対照: fsync していない内容は落ちた ({} バイト)。OK",
        after.len()
    );
    Ok(())
}

fn run_case(
    helper: &Path,
    work: &Path,
    base: &Path,
    lz: Option<&Lazy>,
    case: &Case,
) -> Result<(), String> {
    let dir = base.join(case.name());
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (orig, new) = case.contents();
    let target = dir.join("t.txt");
    if case.mode != Mode::NewFile {
        write_synced(&target, &orig)?;
    }
    // 保存する内容は LazyFS の外に置く (キャッシュを落とす対象にしない)
    let content = work.join(format!("{}.bin", case.name()));
    fs::write(&content, &new).map_err(|e| format!("{}: {e}", content.display()))?;
    let state = dir.join("state");
    let mode = if case.mode == Mode::InPlace {
        "inplace"
    } else {
        "atomic"
    };
    let mut cmd = Command::new(helper);
    cmd.arg("save")
        .arg(&state)
        .arg(&target)
        .arg(mode)
        .arg(&content);
    match case.hook {
        Some(h) => cmd.env("EVE_TEST_HOOK", h),
        None => cmd.env_remove("EVE_TEST_HOOK"),
    };
    let st = cmd
        .stdout(Stdio::null())
        .status()
        .map_err(|e| format!("補助プロセスを起動できません: {e}"))?;
    let completed = match (case.hook, killed_by_sigkill(st)) {
        (None, _) if st.success() => true,
        (Some(_), true) => false,
        (None, _) => return Err(format!("切断点なしの保存が失敗した ({st})")),
        (Some(h), false) => return Err(format!("切断点 {h} に届かなかった ({st})")),
    };
    if let Some(lz) = lz {
        lz.clear_cache()?;
    }
    let after = After {
        target: fs::read(&target).ok(),
        backup_complete: files_in(&state.join("backup"))
            .iter()
            .filter(|p| p.extension().is_some_and(|x| x == "bak"))
            .any(|p| fs::read(p).is_ok_and(|b| b == orig)),
        flag: files_in(&state.join("recovery").join("inflight"))
            .iter()
            .any(|d| !files_in(d).is_empty()),
    };
    judge(case.mode, completed, &orig, &new, &after)
}

#[cfg(unix)]
fn killed_by_sigkill(st: ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    st.signal() == Some(9)
}

#[cfg(not(unix))]
fn killed_by_sigkill(_: ExitStatus) -> bool {
    false
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// 内容を書いて fsync し、親のディレクトリも fsync する (切断の前の状態を確実に残す)。
fn write_synced(path: &Path, data: &[u8]) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut f = fs::File::create(path).map_err(err)?;
    f.write_all(data).map_err(err)?;
    f.sync_all().map_err(err)?;
    if let Some(parent) = path.parent() {
        // ディレクトリの fsync を受け付けない FS もある (LazyFS)。メタデータはすぐ下の FS に反映されるので無視してよい
        let _ = fs::File::open(parent).and_then(|d| d.sync_all());
    }
    Ok(())
}

/// マウントした LazyFS。
struct Lazy {
    child: Child,
    mnt: PathBuf,
    fifo: PathBuf,
    done: PathBuf,
    /// 完了の FIFO の読み口 (マウントの間ずっと開いておく。LazyFS は書き口を開いたまま使い回すので、
    /// 1 回ごとに閉じると次の通知の書込が失敗する)
    notice: Option<fs::File>,
    log: PathBuf,
}

impl Lazy {
    /// `root` の上に LazyFS を `mnt` へマウントする (前面で動かし、マウントが現れるまで待つ)。
    fn mount(bin: &Path, root: &Path, mnt: &Path, dir: &Path) -> Result<Lazy, String> {
        fs::create_dir_all(mnt).map_err(|e| format!("{}: {e}", mnt.display()))?;
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let fifo = dir.join("faults.fifo");
        let done = dir.join("faults-done.fifo");
        let log = dir.join("lazyfs.log");
        let config = dir.join("config.toml");
        fs::write(&config, config_toml(&fifo, &done, &log))
            .map_err(|e| format!("{}: {e}", config.display()))?;
        let out =
            fs::File::create(dir.join("lazyfs.out")).map_err(|e| format!("lazyfs.out: {e}"))?;
        let child = Command::new(bin)
            .arg(mnt)
            .arg("--config-path")
            .arg(&config)
            .args(["-o", "modules=subdir", "-o"])
            .arg(format!("subdir={}", root.display()))
            .args(["-f", "-s"])
            .stdout(out.try_clone().map_err(|e| e.to_string())?)
            .stderr(out)
            .spawn()
            .map_err(|e| format!("LazyFS を起動できません ({}): {e}", bin.display()))?;
        let mut lz = Lazy {
            child,
            mnt: mnt.to_owned(),
            fifo,
            done,
            notice: None,
            log,
        };
        let end = Instant::now() + Duration::from_secs(30);
        while !(is_mounted(mnt) && lz.fifo.exists() && lz.done.exists()) {
            if let Ok(Some(st)) = lz.child.try_wait() {
                return Err(format!("LazyFS が起動直後に終わった ({st})\n{}", lz.tail()));
            }
            if Instant::now() >= end {
                lz.unmount();
                return Err("LazyFS のマウントが 30 秒で現れない".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        match open_nonblocking(&lz.done) {
            Ok(f) => lz.notice = Some(f),
            Err(e) => {
                let msg = format!("{}: {e}", lz.done.display());
                lz.unmount();
                return Err(msg);
            }
        }
        Ok(lz)
    }

    /// fsync されていない書込を落とす (電源断の模擬)。完了の通知 (`fifo_path_completed`) を待つ。
    fn clear_cache(&self) -> Result<(), String> {
        // 送る側の FIFO の open は LazyFS が開くまで止まるので、`timeout` の下の外部コマンドで行う (LazyFS が止まっても戻る)
        let send = Command::new("timeout")
            .args([
                "30",
                "sh",
                "-c",
                "printf 'lazyfs::clear-cache\\n' > \"$1\"",
                "sh",
            ])
            .arg(&self.fifo)
            .status()
            .map_err(|e| format!("timeout を実行できません: {e}"))?;
        if !send.success() {
            return Err(format!("clear-cache を送れない ({send})\n{}", self.tail()));
        }
        let Some(notice) = &self.notice else {
            return Err("完了の FIFO を開いていない".into());
        };
        // 非ブロッキングの読み口を、通知の 1 行が届くまで (最大 30 秒) 読む
        let mut got: Vec<u8> = Vec::new();
        let end = Instant::now() + Duration::from_secs(30);
        let mut buf = [0u8; 256];
        while !got.contains(&b'\n') {
            match (&*notice).read(&mut buf) {
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(format!("{}: {e}", self.done.display())),
            }
            if Instant::now() >= end {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let line = String::from_utf8_lossy(&got);
        if line.contains("clear-cache") {
            Ok(())
        } else {
            Err(format!(
                "clear-cache の完了の通知が 30 秒で来ない ({line:?})\n{}",
                self.tail()
            ))
        }
    }

    fn unmount(mut self) {
        let _ = Command::new("fusermount3")
            .arg("-u")
            .arg(&self.mnt)
            .status();
        let end = Instant::now() + Duration::from_secs(10);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// LazyFS の出力の末尾 (失敗の手掛かり)。
    fn tail(&self) -> String {
        // ログのファイル (無ければ標準出力の写し) の最後の 10 行
        let out = self.log.with_file_name("lazyfs.out");
        let text = [&self.log, &out]
            .iter()
            .filter_map(|p| fs::read_to_string(p).ok())
            .find(|t| !t.is_empty())
            .unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(10)..].join("\n")
    }
}

fn config_toml(fifo: &Path, done: &Path, log: &Path) -> String {
    format!(
        "[faults]\nfifo_path=\"{}\"\nfifo_path_completed=\"{}\"\n[cache]\napply_eviction=false\n[cache.simple]\ncustom_size=\"256mb\"\nblocks_per_page=1\n[filesystem]\nlog_all_operations=false\nlogfile=\"{}\"\n",
        fifo.display(),
        done.display(),
        log.display()
    )
}

/// 読み書き両用・非ブロッキングで開く (FIFO を読み書き両用で開くと、相手を待たずに開ける。Linux)。
#[cfg(unix)]
fn open_nonblocking(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    // O_NONBLOCK (Linux の x86_64 / aarch64 は 0o4000。xtask は libc に依存しない)
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(0o4000)
        .open(path)
}

#[cfg(not(unix))]
fn open_nonblocking(_: &Path) -> std::io::Result<fs::File> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

/// `/proc/self/mountinfo` に `mnt` がマウント点として載っているか。
fn is_mounted(mnt: &Path) -> bool {
    let Ok(text) = fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    let want = mnt.to_string_lossy();
    text.lines().any(|l| l.split(' ').nth(4) == Some(&*want))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after(target: Option<&[u8]>, backup_complete: bool, flag: bool) -> After {
        After {
            target: target.map(<[u8]>::to_vec),
            backup_complete,
            flag,
        }
    }

    #[test]
    fn 原子的保存は元か新しい内容なら合格() {
        assert!(
            judge(
                Mode::Atomic,
                false,
                ORIG,
                NEW,
                &after(Some(ORIG), false, false)
            )
            .is_ok()
        );
        assert!(
            judge(
                Mode::Atomic,
                false,
                ORIG,
                NEW,
                &after(Some(NEW), false, false)
            )
            .is_ok()
        );
        assert!(
            judge(
                Mode::Atomic,
                false,
                ORIG,
                NEW,
                &after(Some(b""), false, false)
            )
            .is_err()
        );
        assert!(judge(Mode::Atomic, false, ORIG, NEW, &after(None, false, false)).is_err());
    }

    #[test]
    fn 保存が成功した後は新しい内容だけが合格() {
        assert!(
            judge(
                Mode::Atomic,
                true,
                ORIG,
                NEW,
                &after(Some(NEW), false, false)
            )
            .is_ok()
        );
        assert!(
            judge(
                Mode::Atomic,
                true,
                ORIG,
                NEW,
                &after(Some(ORIG), false, false)
            )
            .is_err()
        );
        assert!(
            judge(
                Mode::InPlace,
                true,
                ORIG,
                NEW,
                &after(Some(b"x"), true, true)
            )
            .is_err()
        );
    }

    #[test]
    fn 新規は無いか新しい内容なら合格() {
        assert!(judge(Mode::NewFile, false, ORIG, NEW, &after(None, false, false)).is_ok());
        assert!(
            judge(
                Mode::NewFile,
                false,
                ORIG,
                NEW,
                &after(Some(NEW), false, false)
            )
            .is_ok()
        );
        assert!(
            judge(
                Mode::NewFile,
                false,
                ORIG,
                NEW,
                &after(Some(b""), false, false)
            )
            .is_err()
        );
    }

    #[test]
    fn 直接上書きは途中の内容でも完全なバックアップと保存中フラグがあれば合格() {
        assert!(
            judge(
                Mode::InPlace,
                false,
                ORIG,
                NEW,
                &after(Some(b"NEWx"), true, true)
            )
            .is_ok()
        );
        assert!(
            judge(
                Mode::InPlace,
                false,
                ORIG,
                NEW,
                &after(Some(b"NEWx"), true, false)
            )
            .is_err()
        );
        assert!(
            judge(
                Mode::InPlace,
                false,
                ORIG,
                NEW,
                &after(Some(b"NEWx"), false, true)
            )
            .is_err()
        );
    }

    #[test]
    fn 事例の名前は重ならない() {
        let mut names: Vec<String> = CASES.iter().map(Case::name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), CASES.len());
    }

    #[test]
    fn 引数の値を集める() {
        let a: Vec<String> = ["--root", "/a", "--x", "--root", "/b"]
            .map(String::from)
            .to_vec();
        assert_eq!(values(&a, "--root"), ["/a", "/b"]);
    }
}
