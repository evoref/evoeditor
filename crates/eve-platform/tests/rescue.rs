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

/// 時間予算を検査しない試験が渡す予算。
/// 予算 (5 秒) の判定は専用の 2 つの試験 (予算切れと 100 MB) が行う。ほかの試験が 5 秒を使うと、100 MB の試験とディスクを
/// 取り合う CI で書き終わらず `complete = false` になって落ちる (2026-10-28 の CI で `written: 1 / total: 2`)。
const AMPLE: Duration = Duration::from_secs(60);

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

// verifies: AC-NFR-302-1
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
    let out = w.write(&[big, small], RescueReason::Panic, AMPLE);
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
    let out = w.write(&[d], RescueReason::Fatal, AMPLE);
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
        let out = w.write(&[d], RescueReason::Panic, AMPLE);
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
    let out = w.write(&[d], RescueReason::ConsoleClose, AMPLE);
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

// verifies: AC-NFR-505-1
#[test]
fn 退避先は所有者のみの_dacl() {
    let state = tmp("acl");
    let w = WinRescue::new(Some(state.join("s")));
    let out = w.write(
        &[doc(1, "secret", None, |_| {})],
        RescueReason::Panic,
        AMPLE,
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
        AMPLE,
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

// ---- 起動時の走査 (list_recovery_dirs、05 §7.5「recovery/ の走査と削除の規則」) ----

use eve_core::FileSystem;
use eve_platform::fs::WinFs;

#[test]
fn 走査は名前の形に合うディレクトリだけを返し_inflight_と中身は見ない() {
    let state = tmp("scan");
    let rec = state.join("s").join("recovery");
    for d in [
        "20261021T120000Z-100",
        "20261021T120000Z-100-2",
        "rescue/20261022T000000Z-300",
        "inflight/0123456789abcdef", // inflight は見ない
        "not-a-recovery",
        "rescue/junk",
    ] {
        fs::create_dir_all(rec.join(d)).unwrap();
    }
    fs::write(rec.join("20261021T120000Z-999"), b"file").unwrap(); // ファイルは対象外
    let fsys = WinFs::new(state.join("s"));
    let mut got: Vec<(String, bool)> = fsys
        .list_recovery_dirs()
        .unwrap()
        .into_iter()
        .map(|e| {
            (
                e.path
                    .as_path()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                e.rescue,
            )
        })
        // 代替の退避先 (他のテスト・実行が残したもの) は除く
        .filter(|(n, _)| {
            [
                "20261021T120000Z-100",
                "20261021T120000Z-100-2",
                "20261022T000000Z-300",
            ]
            .contains(&n.as_str())
        })
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("20261021T120000Z-100".to_owned(), false),
            ("20261021T120000Z-100-2".to_owned(), false),
            ("20261022T000000Z-300".to_owned(), true),
        ]
    );
    // 状態ディレクトリを持たない実装は (代替の退避先を除いて) 何も返さない
    assert!(
        WinFs::default()
            .list_recovery_dirs()
            .unwrap()
            .iter()
            .all(|e| !e.path.as_path().starts_with(&state))
    );
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 書いた退避は次の走査で見つかる() {
    let state = tmp("scan2");
    let sd = state.join("s");
    let w = WinRescue::new(Some(sd.clone()));
    let out = w.write(&[doc(1, "x", None, |_| {})], RescueReason::Panic, AMPLE);
    let written = out.dir.unwrap();
    let found = WinFs::new(&sd).list_recovery_dirs().unwrap();
    assert!(
        found
            .iter()
            .any(|e| e.path.as_path() == written && !e.rescue),
        "{found:?}"
    );
    let _ = fs::remove_dir_all(state);
}

#[test]
fn 退避先に保存は_rescue_の下に_user_save_で書く() {
    use eve_core::FileSystem;
    let state = tmp("userSave");
    let fs_ = eve_platform::fs::WinFs::new(&state);
    let d = doc(1, "本文\nabc", Some("a.txt"), |_| {});
    let dir = fs_.write_recovery(&d).expect("書ける");
    let dir = dir.as_path().to_path_buf();
    assert_eq!(dir.parent().unwrap(), state.join("recovery").join("rescue"));
    let m = manifest(&dir);
    assert!(
        m.contains("reason = \"user_save\"") && m.contains("complete = true"),
        "{m}"
    );
    assert_eq!(
        fs::read_to_string(dir.join("1-a.txt.txt")).unwrap(),
        "本文\nabc"
    );
    // 起動時の走査は rescue として見つける
    let found = fs_.list_recovery_dirs().unwrap();
    assert!(found.iter().any(|e| e.rescue), "{found:?}");
    let _ = fs::remove_dir_all(state);
}

// verifies: AC-NFR-302-6
#[test]
fn 未知の_format_version_の退避は一覧に出るが_変更も移動も削除もされない() {
    use eve_core::FileSystem;
    let state = tmp("unknownVersion");
    let old = state.join("recovery").join("20200101T000000Z-1");
    fs::create_dir_all(&old).unwrap();
    // 将来の版の manifest (未知の版と未知のキー) と本文
    let manifest_text = "format_version = 99\nfuture_key = \"x\"\n[[document]]\nfile = \"1-a.txt\"\nnew_field = 7\n";
    fs::write(old.join("manifest.toml"), manifest_text).unwrap();
    fs::write(old.join("1-a.txt"), "将来の本文").unwrap();
    let snapshot = |dir: &Path| -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    };
    let before = snapshot(&old);

    let fs_ = eve_platform::fs::WinFs::new(&state);
    // 起動時の走査: 読めない版でも一覧に出る (案内の対象)
    let found = fs_.list_recovery_dirs().unwrap();
    assert!(
        found.iter().any(|e| e.path.as_path() == old),
        "未知の版でも見つかる: {found:?}"
    );
    // 新しい退避を書いても、古い退避には触れない
    let d = doc(1, "新しい", Some("n.txt"), |_| {});
    let out = WinRescue::new(Some(state.clone())).write(&[d], RescueReason::Panic, AMPLE);
    assert!(out.complete);
    assert_eq!(
        snapshot(&old),
        before,
        "未知の版の退避は変更・移動・削除されない"
    );
    let _ = fs::remove_dir_all(state);
}

// verifies: AC-NFR-302-5
#[test]
#[allow(clippy::print_stderr)] // 所要時間を記録に残す (--nocapture)
fn 合計_100_mb_の_dirty_文書の退避が_5_秒以内に完了する() {
    let state = tmp("big");
    let w = WinRescue::new(Some(state.join("s")));
    // 25 MB の文書を 4 つ (日本語と ASCII の混在、改行つき)
    let line = "0123456789 the quick brown fox 日本語の行です。0123456789\n";
    let body = line.repeat(25 * 1024 * 1024 / line.len());
    let docs: Vec<_> = (1..=4u64)
        .map(|i| doc(i, &body, Some(&format!(r"C:\big{i}.txt")), |_| {}))
        .collect();
    use eve_text::TextRead;
    let total: usize = docs.iter().map(|d| d.snapshot.len_bytes()).sum();
    assert!(total >= 100 * 1000 * 1000, "合計 {total} バイト");
    // 時間の判定は基準機 (EVE_PERF_STRICT=1) で行う。共有の CI ランナーは fsync が遅く、5 秒を超えて打ち切られることがある
    // (NFR-302 AC-5 の「時間切れの場合も書けた分は manifest に記録される」の側を、どの環境でも検査する)
    let strict = std::env::var_os("EVE_PERF_STRICT").is_some();
    let t = std::time::Instant::now();
    let out = w.write(&docs, RescueReason::Fatal, Duration::from_secs(5));
    let elapsed = t.elapsed();
    eprintln!(
        "100 MB の退避: {elapsed:?} (complete = {}, written = {})",
        out.complete, out.written
    );
    let dir = out.dir.clone().expect("退避先ができる");
    // どの環境でも: 打ち切られても書けた分が残り、manifest の complete が実際と一致する
    assert!(out.written >= 1, "少なくとも 1 文書は書ける");
    assert_eq!(
        manifest(&dir).contains("complete = true"),
        out.complete,
        "manifest の complete が実際と一致する"
    );
    let full_files = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
        .filter(|e| e.metadata().unwrap().len() == body.len() as u64)
        .count();
    assert_eq!(
        full_files, out.written,
        "書けたと記録した文書は全文が書かれている"
    );
    // 予算 (5 秒) を超えても、最後の fsync の分を除いて大きくは超えない
    assert!(elapsed < Duration::from_secs(5 + 5), "{elapsed:?}");
    if strict {
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
        assert!(out.complete, "時間内に全部書けた: {:?}", out.error);
        assert_eq!(out.written, 4);
        let written: u64 = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
            .map(|e| e.metadata().unwrap().len())
            .sum();
        assert_eq!(written, total as u64, "本文のバイト数が一致する");
    }
    let _ = fs::remove_dir_all(state);
}
