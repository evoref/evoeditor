//! スワップの復旧の探索の実ファイルでの検査 (docs/03 §10.3、05 §5.3 / §7.5)。

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use eve_core::{DocSnapshot, DocumentId, FileSystem, FsErrorKind, FsPath, SwapOwner, TextFormat};
use eve_platform::fs::WinFs;
use eve_text::Text;
use eve_view::Revision;

fn tmp(tag: &str) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let p = std::env::temp_dir().join(format!("eve-swscan-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}

fn doc(id: u64, rev: u64, text: &str, path: Option<&Path>) -> DocSnapshot {
    let mut format = TextFormat::new_document();
    format.default_eol = eve_text::LineEnding::Crlf;
    DocSnapshot {
        doc: DocumentId(id),
        snapshot: Text::from(text).snapshot(),
        format,
        revision: Revision(rev),
        saved_node_exists: false,
        sensitive: false,
        path: path.map(FsPath::new),
        untitled_name: path.is_none().then(|| format!("無題-{id}")),
    }
}

/// 自分が書いたスワップを、別のプロセス (pid) が書いたものに見せかける。ディレクトリ名と manifest の pid を書き換える。
fn as_other_process(state: &Path, pid: u32, started: bool) -> PathBuf {
    let base = state.join("swap");
    let dir = fs::read_dir(&base).unwrap().next().unwrap().unwrap().path();
    let me = std::process::id();
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let new_dir = base.join(name.replace(&format!("-{me}.swp"), &format!("-{pid}.swp")));
    fs::rename(&dir, &new_dir).unwrap();
    let m = fs::read_to_string(new_dir.join("manifest.toml")).unwrap();
    let mut out = String::new();
    for l in m.lines() {
        if l.starts_with("pid = ") {
            out.push_str(&format!("pid = {pid}\n"));
        } else if l.starts_with("started_at_") && !started {
            // 起動時刻を消して pid だけで判定させる
        } else {
            out.push_str(l);
            out.push('\n');
        }
    }
    fs::write(new_dir.join("manifest.toml"), out).unwrap();
    new_dir
}

/// 終了済みのプロセスの pid。
fn dead_pid() -> u32 {
    let mut c = Command::new("cmd")
        .args(["/c", "exit", "0"])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let pid = c.id();
    c.wait().unwrap();
    pid
}

#[test]
fn 自分のスワップは列挙しない() {
    let state = tmp("mine");
    let w = WinFs::new(&state);
    w.write_swap(&doc(1, 1, "a", None)).unwrap();
    assert!(w.list_swaps().unwrap().is_empty());
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 終了したプロセスのスワップは_stale_で_要約と本文を読める() {
    let state = tmp("stale");
    let work = tmp("work");
    let file = work.join("a.txt");
    fs::write(&file, "x").unwrap();
    let w = WinFs::new(&state);
    w.write_swap(&doc(3, 9, "こんにちは\r\n世界", Some(&file)))
        .unwrap();
    let pid = dead_pid();
    let dir = as_other_process(&state, pid, false);

    let v = w.list_swaps().unwrap();
    assert_eq!(v.len(), 1);
    let e = &v[0];
    assert_eq!(e.dir, FsPath::new(&dir));
    assert_eq!(e.owner, SwapOwner::Stale);
    assert_eq!(e.pid, pid);
    assert_eq!(e.revision, 9);
    assert!(e.readable && !e.sensitive);
    assert_eq!(e.len, "こんにちは\r\n世界".len() as u64);
    assert_eq!(
        e.path.as_ref().unwrap().display(),
        file.display().to_string()
    );

    let body = w.read_swap(e).unwrap();
    assert_eq!(body.text, "こんにちは\r\n世界", "改行を正規化しない");
    assert_eq!(body.encoding, eve_core::Encoding::Utf8);
    assert!(!body.has_bom);
    assert_eq!(body.default_eol, eve_text::LineEnding::Crlf);
    let _ = fs::remove_dir_all(state);
    let _ = fs::remove_dir_all(work);
}

#[test]
fn 動いているプロセスのスワップは_alive() {
    let state = tmp("alive");
    let w = WinFs::new(&state);
    w.write_swap(&doc(1, 1, "a", None)).unwrap();
    let mut child = Command::new("cmd")
        .args(["/c", "ping", "-n", "30", "127.0.0.1"])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    as_other_process(&state, child.id(), false);
    let v = w.list_swaps().unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].owner, SwapOwner::Alive);
    let _ = fs::remove_dir_all(state);
}

#[test]
fn pid_が再利用されて起動時刻が違えば_stale() {
    let state = tmp("reuse");
    let w = WinFs::new(&state);
    w.write_swap(&doc(1, 1, "a", None)).unwrap();
    let mut child = Command::new("cmd")
        .args(["/c", "ping", "-n", "30", "127.0.0.1"])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    // manifest の起動時刻は自分のもの (数秒以上前ではない) なので、子の pid に付けても「別のプロセス」になるとは限らない。
    // 起動時刻を古い値に書き換えて、再利用された pid を再現する
    let dir = as_other_process(&state, child.id(), true);
    let m = fs::read_to_string(dir.join("manifest.toml")).unwrap();
    let m: String = m
        .lines()
        .map(|l| {
            if l.starts_with("started_at_secs = ") {
                "started_at_secs = 1000000000".to_owned()
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(dir.join("manifest.toml"), m).unwrap();
    let v = w.list_swaps().unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(v[0].owner, SwapOwner::Stale);
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 未知の版と壊れた_manifest_は読めないものとして出す() {
    let state = tmp("unreadable");
    let base = state.join("swap");
    let a = base.join("untitled-1-4000000001.swp");
    let b = base.join("untitled-2-4000000002.swp");
    let c = base.join("untitled-3-4000000003.swp");
    for d in [&a, &b, &c] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(
        a.join("manifest.toml"),
        "format_version = 99\npid = 4000000001\n",
    )
    .unwrap();
    fs::write(b.join("manifest.toml"), "壊れている").unwrap();
    // c は manifest が無い
    // 名前の形に合わないものは無視する
    fs::create_dir_all(base.join("note")).unwrap();
    fs::write(base.join("file.swp"), "x").unwrap();
    let w = WinFs::new(&state);
    let v = w.list_swaps().unwrap();
    assert_eq!(v.len(), 3, "{v:?}");
    assert!(v.iter().all(|e| !e.readable && e.owner == SwapOwner::Stale));
    assert!(v.iter().all(|e| e.path.is_none()));
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 書きかけの_manifest_written_false_は読めないものとして出す() {
    let state = tmp("unwritten");
    let w = WinFs::new(&state);
    w.write_swap(&doc(1, 1, "a", None)).unwrap();
    let dir = as_other_process(&state, dead_pid(), false);
    let m = fs::read_to_string(dir.join("manifest.toml")).unwrap();
    fs::write(
        dir.join("manifest.toml"),
        m.replace("written = true", "written = false"),
    )
    .unwrap();
    let v = w.list_swaps().unwrap();
    assert!(!v[0].readable);
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 消せるのは_swap_の下の_swp_だけ() {
    let state = tmp("remove");
    let w = WinFs::new(&state);
    w.write_swap(&doc(1, 1, "a", None)).unwrap();
    let dir = as_other_process(&state, dead_pid(), false);
    let mut e = w.list_swaps().unwrap().remove(0);
    // swap の外のパスは消さない
    let outside = tmp("outside");
    let mut bad = e.clone();
    bad.dir = FsPath::new(&outside);
    assert_eq!(
        w.remove_swap(&bad).unwrap_err().kind,
        FsErrorKind::InvalidPath
    );
    assert!(outside.exists());
    // .swp でないディレクトリも消さない
    let notswp = state.join("swap").join("keep");
    fs::create_dir_all(&notswp).unwrap();
    e.dir = FsPath::new(&notswp);
    assert_eq!(
        w.remove_swap(&e).unwrap_err().kind,
        FsErrorKind::InvalidPath
    );
    assert!(notswp.exists());
    // 本物は消える。もう一度は成功 (無ければ何もしない)
    let real = w.list_swaps().unwrap().remove(0);
    w.remove_swap(&real).unwrap();
    assert!(!dir.exists());
    w.remove_swap(&real).unwrap();
    let _ = fs::remove_dir_all(state);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn 状態ディレクトリが無ければ空_消すのは未対応() {
    let w = WinFs::default();
    assert!(w.list_swaps().unwrap().is_empty());
    let e = eve_core::SwapEntry {
        dir: FsPath::from("C:\\x\\swap\\a.swp"),
        path: None,
        untitled_name: None,
        revision: 0,
        len: 0,
        pid: 1,
        owner: SwapOwner::Stale,
        sensitive: false,
        readable: false,
    };
    assert_eq!(
        w.remove_swap(&e).unwrap_err().kind,
        FsErrorKind::Unsupported
    );
}
