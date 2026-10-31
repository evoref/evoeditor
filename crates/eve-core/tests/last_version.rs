//! 更新後の初回起動の要点表示 (FR-518、07 §18.8 の (2))。前回起動した版の記録 (`state_dir/last_version.toml`) と
//! 今の版を比べ、新しい版で初めて起動したときだけ `info.app.updated` を 1 回出す。

use eve_core::LAST_VERSION_FILE;
use eve_testkit::{Harness, MEMFS_STATE_DIR};

const CURRENT: &str = env!("CARGO_PKG_VERSION");

fn record_path() -> String {
    format!("{MEMFS_STATE_DIR}/{LAST_VERSION_FILE}")
}

fn record(version: &str) -> Vec<u8> {
    format!("format_version = 1\nlast_run_version = \"{version}\"\n").into_bytes()
}

/// 記録 `rec` (無ければ `None`) を置いて起動し、起動時のジョブを終えた Harness。
fn started(rec: Option<&[u8]>) -> Harness {
    let h = Harness::new();
    let h = match rec {
        Some(r) => h.with_file(&record_path(), r),
        None => h,
    };
    let mut h = h;
    h.run_jobs();
    h
}

fn recorded(h: &Harness) -> String {
    String::from_utf8(h.fs.read(&record_path()).expect("記録がある")).unwrap()
}

fn updated_shown(h: &Harness) -> bool {
    h.messages().contains(&"info.app.updated".to_owned())
}

// verifies: FR-518
#[test]
fn 前回より新しい版で起動すると要点を_1_回だけ出し_記録を更新する() {
    let h = started(Some(&record("0.0.1")));
    assert!(updated_shown(&h));
    assert!(recorded(&h).contains(&format!("last_run_version = \"{CURRENT}\"")));
    // 同じ版の 2 回目の起動では出さない
    let fs_after = h.fs.read(&record_path()).unwrap();
    let h2 = started(Some(&fs_after));
    assert!(!updated_shown(&h2));
}

// verifies: FR-518
#[test]
fn 記録が無ければ表示せず記録だけを作る() {
    let h = started(None);
    assert!(!updated_shown(&h));
    assert!(recorded(&h).contains("format_version = 1"));
    assert!(recorded(&h).contains(&format!("last_run_version = \"{CURRENT}\"")));
}

// verifies: FR-518
#[test]
fn ダウングレードと壊れた記録は表示せず今の版で書き直す() {
    for rec in [
        record("99.0.0"),
        b"format_version = 1\nlast_run_version = \"x.y\"\n".to_vec(),
        b"\xff\xfe".to_vec(),
    ] {
        let h = started(Some(&rec));
        assert!(!updated_shown(&h));
        assert!(recorded(&h).contains(&format!("last_run_version = \"{CURRENT}\"")));
    }
}

// verifies: FR-518
#[test]
fn 未知の書式の版は退避してから作り直し_表示しない() {
    let unknown = b"format_version = 2\nlast_run_version = \"0.0.1\"\n";
    let h = started(Some(unknown));
    assert!(!updated_shown(&h));
    assert!(recorded(&h).contains("format_version = 1"));
    // 元の内容は `<name>.unsupported-<ts>` に残る (消さない、05 §7.5 (a))
    let prefix = format!("{}.unsupported-", record_path());
    let kept =
        h.fs.paths()
            .into_iter()
            .find(|p| p.starts_with(&prefix))
            .expect("退避したファイル");
    assert_eq!(h.fs.read(&kept).unwrap(), unknown);
}
