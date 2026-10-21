//! 緊急退避の書き手 (`WinRescue`) の実ファイルでの検査 (docs/02 §11.2.1、05 §7.5、01 NFR-302)。

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use eve_core::{DocSnapshot, Encoding, FsPath, RescueReason, RescueWriter, TextFormat};
use eve_platform::rescue::WinRescue;
use eve_text::{LineEnding, Text};
use eve_view::{DocumentId, Revision};

fn tmp(tag: &str) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let p = std::env::temp_dir().join(format!("eve-rescue-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}

fn doc(
    id: u64,
    text: &str,
    path: Option<&str>,
    f: impl FnOnce(&mut TextFormat),
) -> Arc<DocSnapshot> {
    let mut format = TextFormat::new_document();
    f(&mut format);
    Arc::new(DocSnapshot {
        doc: DocumentId(id),
        snapshot: Text::from(text).snapshot(),
        format,
        revision: Revision(id * 10),
        saved_node_exists: false,
        sensitive: false,
        path: path.map(FsPath::from),
        untitled_name: path.is_none().then(|| format!("無題-{id}")),
    })
}

/// manifest の `key = value` を 1 つ目のものから引く (テスト用の素朴な読み)。
fn manifest(dir: &Path) -> String {
    fs::read_to_string(dir.join("manifest.toml")).unwrap()
}

fn dirs_in(base: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(base)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

#[test]
fn 小さい文書から書いて_manifest_は完了になる() {
    let state = tmp("ok");
    let w = WinRescue::new(Some(state.join("state")));
    let big = doc(
        1,
        &"大きい文書\r\n".repeat(50),
        Some("C:\\w\\big.txt"),
        |f| {
            f.encoding = Encoding::ShiftJis;
            f.default_eol = LineEnding::Crlf;
        },
    );
    let small = doc(2, "small", None, |_| {});
    let out = w.write(&[big, small], RescueReason::Panic, Duration::from_secs(5));
    assert!(out.complete && !out.fallback, "{out:?}");
    assert_eq!((out.total, out.written), (2, 2));
    let dir = out.dir.unwrap();
    assert!(dir.starts_with(state.join("state").join("recovery")));
    // 小さい文書が 1 番、大きい文書が 2 番 (名前は `<n>-<basename>.txt`。basename は拡張子を含む)
    assert_eq!(fs::read(dir.join("1-無題-2.txt")).unwrap(), b"small");
    assert_eq!(
        fs::read_to_string(dir.join("2-big.txt.txt")).unwrap(),
        "大きい文書\r\n".repeat(50),
        "改行を正規化しない"
    );
    let m = manifest(&dir);
    for want in [
        "format_version = 1",
        "reason = \"panic\"",
        "complete = true",
        "file = \"1-無題-2.txt\"",
        "untitled_name = \"無題-2\"",
        "file = \"2-big.txt.txt\"",
        "path = \"C:\\\\w\\\\big.txt\"",
        "encoding = \"Shift_JIS\"",
        "default_eol = \"crlf\"",
        "revision = 10",
        "revision = 20",
    ] {
        assert!(m.contains(want), "{want}\n{m}");
    }
    assert_eq!(m.matches("written = true").count(), 2);
    assert!(!dir.join("manifest.toml.tmp").exists());
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 不正バイトのエスケープと_pua_を保つ() {
    let state = tmp("pua");
    let w = WinRescue::new(Some(state.join("s")));
    let text = format!("a{}b\u{FF9E}", eve_encoding_escape(0x9A));
    let d = doc(1, &text, None, |f| f.invalid_bytes = 1);
    let out = w.write(&[d], RescueReason::Fatal, Duration::from_secs(5));
    let dir = out.dir.unwrap();
    let body = fs::read_to_string(dir.join("1-無題-1.txt")).unwrap();
    assert_eq!(body, text, "内部の UTF-8 のまま (PUA を保つ)");
    assert!(manifest(&dir).contains("pua_escaped = true"));
    let _ = fs::remove_dir_all(state);
}

fn eve_encoding_escape(b: u8) -> char {
    char::from_u32(0x10FF00 + u32::from(b)).unwrap()
}

#[test]
fn 時間予算が尽きたら書けた分だけ残し_complete_は_false() {
    let state = tmp("budget");
    let w = WinRescue::new(Some(state.join("s")));
    let a = doc(1, "a", Some("C:\\a.txt"), |_| {});
    let out = w.write(&[a], RescueReason::Signal, Duration::ZERO);
    assert!(!out.complete);
    assert_eq!(out.written, 0);
    assert!(out.error.is_some());
    // 予算が尽きていても、ディレクトリと manifest は作る (complete = false)
    let dir = out.dir.unwrap();
    assert!(manifest(&dir).contains("complete = false"));
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 退避する文書が無ければ何も作らない() {
    let state = tmp("none");
    let w = WinRescue::new(Some(state.join("s")));
    let out = w.write(&[], RescueReason::Signal, Duration::from_secs(1));
    assert!(out.nothing_to_save() && out.complete && out.dir.is_none());
    assert!(!state.join("s").exists());
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 同じ秒の退避は連番で別のディレクトリになる() {
    let state = tmp("seq");
    let w = WinRescue::new(Some(state.join("s")));
    let mut dirs = Vec::new();
    for i in 0..3 {
        let d = doc(1, "x", None, |_| {});
        let out = w.write(&[d], RescueReason::Panic, Duration::from_secs(5));
        dirs.push(out.dir.unwrap());
        assert!(out.complete, "{i}");
    }
    dirs.sort();
    dirs.dedup();
    assert_eq!(dirs.len(), 3, "{dirs:?}");
    let names: Vec<String> = dirs
        .iter()
        .map(|d| d.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let pid = std::process::id().to_string();
    assert!(
        names.iter().all(|n| n.contains(&pid) && n.len() >= 16),
        "{names:?}"
    );
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 状態ディレクトリを使えなければ代替の退避先に書く() {
    let state = tmp("fallback");
    // 状態ディレクトリの場所にファイルを置いて、ディレクトリを作れなくする
    let blocked = state.join("blocked");
    fs::write(&blocked, b"file").unwrap();
    let w = WinRescue::new(Some(blocked.clone()));
    let d = doc(1, "rescued", Some("C:\\a.txt"), |_| {});
    let out = w.write(&[d], RescueReason::ConsoleClose, Duration::from_secs(5));
    let dir = out.dir.clone().expect("代替の退避先");
    assert!(out.fallback && out.complete, "{out:?}");
    assert!(
        dir.parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("eve-recovery-S-1-"),
        "{dir:?}"
    );
    assert_eq!(fs::read(dir.join("1-a.txt.txt")).unwrap(), b"rescued");
    // 後片付け (代替の退避先は一時ディレクトリの下)
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir(dir.parent().unwrap());
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 退避先は所有者のみの_dacl() {
    let state = tmp("acl");
    let w = WinRescue::new(Some(state.join("s")));
    let out = w.write(
        &[doc(1, "secret", None, |_| {})],
        RescueReason::Panic,
        Duration::from_secs(5),
    );
    let dir = out.dir.unwrap();
    for p in [
        dir.clone(),
        dir.join("1-無題-1.txt"),
        dir.join("manifest.toml"),
    ] {
        let o = Command::new("icacls").arg(&p).output().unwrap();
        let acl = String::from_utf8_lossy(&o.stdout).into_owned();
        assert!(acl.contains("SYSTEM"), "{p:?}\n{acl}");
        assert!(
            !acl.contains("BUILTIN\\Users")
                && !acl.contains("Everyone")
                && !acl.contains("Administrators"),
            "{p:?}\n{acl}"
        );
    }
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 長い名前と予約文字のファイル名() {
    let state = tmp("name");
    let w = WinRescue::new(Some(state.join("s")));
    let long = format!("C:\\w\\{}.txt", "あ".repeat(200));
    let out = w.write(
        &[doc(1, "x", Some(&long), |_| {})],
        RescueReason::Panic,
        Duration::from_secs(5),
    );
    assert!(out.complete, "{out:?}");
    let dir = out.dir.unwrap();
    let names: Vec<String> = dirs_in(&dir)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().any(|n| n.starts_with("1-あ")), "{names:?}");
    assert!(names.iter().all(|n| n.encode_utf16().count() < 255));
    let _ = fs::remove_dir_all(state);
}
