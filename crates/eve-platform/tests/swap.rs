//! スワップ (swap-lite) の実ファイルでの検査 (docs/03 §10.2、05 §7.5、02 §11.2.1)。

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};

use eve_core::{DocSnapshot, DocumentId, FileSystem, FsErrorKind, FsPath, TextFormat};
use eve_platform::fs::WinFs;
use eve_text::Text;
use eve_view::Revision;

fn tmp(tag: &str) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let p = std::env::temp_dir().join(format!("eve-swap-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}

fn doc(id: u64, rev: u64, text: &str, path: Option<&Path>) -> DocSnapshot {
    DocSnapshot {
        doc: DocumentId(id),
        snapshot: Text::from(text).snapshot(),
        format: TextFormat::new_document(),
        revision: Revision(rev),
        saved_node_exists: false,
        sensitive: false,
        path: path.map(FsPath::new),
        untitled_name: path.is_none().then(|| format!("無題-{id}")),
    }
}

fn swap_dirs(state: &Path) -> Vec<PathBuf> {
    let base = state.join("swap");
    let mut v: Vec<PathBuf> = fs::read_dir(base)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn 無題の文書のスワップは_untitled_で始まる名前の_swp_に本文と_manifest_を書く() {
    let state = tmp("untitled");
    let w = WinFs::new(&state);
    let d = doc(7, 3, "こんにちは\r\n", None);
    w.write_swap(&d).unwrap();
    let dirs = swap_dirs(&state);
    assert_eq!(dirs.len(), 1);
    let pid = std::process::id();
    assert_eq!(
        dirs[0].file_name().unwrap().to_string_lossy(),
        format!("untitled-7-{pid}.swp")
    );
    assert_eq!(names(&dirs[0]), ["1-無題-7.r3.txt", "manifest.toml"]);
    assert_eq!(
        fs::read_to_string(dirs[0].join("1-無題-7.r3.txt")).unwrap(),
        "こんにちは\r\n",
        "改行を正規化しない"
    );
    let m = fs::read_to_string(dirs[0].join("manifest.toml")).unwrap();
    for want in [
        "format_version = 1",
        "reason = \"swap\"",
        "complete = true",
        &format!("pid = {pid}"),
        "started_at_secs = ",
        "file = \"1-無題-7.r3.txt\"",
        "untitled_name = \"無題-7\"",
        "revision = 3",
        "written = true",
        "final_newline = true",
    ] {
        assert!(m.contains(want), "{want}\n{m}");
    }
    let _ = fs::remove_dir_all(state);
}

#[test]
fn パスのある文書は_ハッシュと_pid_の名前で_大文字小文字の違いは同じ名前() {
    let state = tmp("path");
    let work = tmp("work");
    let a = work.join("Notes.txt");
    fs::write(&a, "x").unwrap();
    let w = WinFs::new(&state);
    w.write_swap(&doc(1, 1, "abc", Some(&a))).unwrap();
    let dirs = swap_dirs(&state);
    assert_eq!(dirs.len(), 1);
    let name = dirs[0].file_name().unwrap().to_string_lossy().into_owned();
    let pid = std::process::id();
    assert!(name.ends_with(&format!("-{pid}.swp")), "{name}");
    let hash = name.trim_end_matches(&format!("-{pid}.swp"));
    assert_eq!(hash.len(), 16);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "{hash}");
    let m = fs::read_to_string(dirs[0].join("manifest.toml")).unwrap();
    assert!(m.contains("1-Notes.txt.r1.txt"), "{m}");
    assert!(m.contains("Notes.txt"), "元パスを表示用に残す: {m}");

    // 別の綴り (大文字小文字だけ違う) でも同じスワップ
    let b = work.join("NOTES.TXT");
    w.write_swap(&doc(2, 5, "abcd", Some(&b))).unwrap();
    assert_eq!(swap_dirs(&state).len(), 1, "同じファイルは同じスワップ");
    let _ = fs::remove_dir_all(state);
    let _ = fs::remove_dir_all(work);
}

#[test]
fn 書き直すと新しい本文を指し_古い本文と一時ファイルは残らない() {
    let state = tmp("rewrite");
    let w = WinFs::new(&state);
    w.write_swap(&doc(1, 1, "one", None)).unwrap();
    let dir = swap_dirs(&state).remove(0);
    // 前の書込が途中で止まった名残の一時ファイル
    fs::write(dir.join("1-無題-1.r0.txt.tmp"), "半端").unwrap();
    w.write_swap(&doc(1, 2, "two!", None)).unwrap();
    assert_eq!(names(&dir), ["1-無題-1.r2.txt", "manifest.toml"]);
    assert_eq!(
        fs::read_to_string(dir.join("1-無題-1.r2.txt")).unwrap(),
        "two!"
    );
    let m = fs::read_to_string(dir.join("manifest.toml")).unwrap();
    assert!(m.contains("revision = 2") && m.contains("len = 4"), "{m}");
    assert!(!m.contains("r1.txt"));
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 空の文書でも書ける() {
    let state = tmp("empty");
    let w = WinFs::new(&state);
    w.write_swap(&doc(1, 1, "", None)).unwrap();
    let dir = swap_dirs(&state).remove(0);
    assert_eq!(fs::read(dir.join("1-無題-1.r1.txt")).unwrap(), b"");
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 秘密ファイルの文書は_sensitive_を立てる() {
    let state = tmp("sensitive");
    let w = WinFs::new(&state);
    let mut d = doc(1, 1, "K=1", None);
    d.sensitive = true;
    w.write_swap(&d).unwrap();
    let dir = swap_dirs(&state).remove(0);
    assert!(
        fs::read_to_string(dir.join("manifest.toml"))
            .unwrap()
            .contains("sensitive = true")
    );
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 消すのは自分の_pid_のスワップだけで_無ければ成功() {
    let state = tmp("discard");
    let w = WinFs::new(&state);
    w.write_swap(&doc(1, 1, "mine", None)).unwrap();
    // 他のインスタンス (別の pid) のスワップ
    let other = state.join("swap").join("untitled-1-999999.swp");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("manifest.toml"), "x").unwrap();
    w.discard_swap(DocumentId(1), None).unwrap();
    assert_eq!(
        swap_dirs(&state),
        std::slice::from_ref(&other),
        "自分のだけ消える"
    );
    w.discard_swap(DocumentId(1), None)
        .expect("無ければ何もしない (成功)");
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 状態ディレクトリが無ければ未対応を返す() {
    let w = WinFs::default();
    let e = w.write_swap(&doc(1, 1, "a", None)).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::Unsupported);
    let e = w.discard_swap(DocumentId(1), None).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::Unsupported);
}

#[test]
fn ディレクトリは所有者のみの_dacl_で作る() {
    let state = tmp("dacl");
    let w = WinFs::new(&state);
    w.write_swap(&doc(1, 1, "a", None)).unwrap();
    let dir = swap_dirs(&state).remove(0);
    // 保護付きの DACL (継承を遮断) で作るので、親の継承による ACE が付かない。icacls の出力に「継承」の印 (I) が無いことで確かめる
    let out = std::process::Command::new("icacls")
        .arg(&dir)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{text}");
    assert!(!text.contains("(I)"), "継承された ACE がある: {text}");
    let _ = fs::remove_dir_all(state);
}
