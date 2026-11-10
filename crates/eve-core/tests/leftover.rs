//! 置換途中の痕跡の起動時の走査 (docs/05 §4.6「起動時の提案の規則」、03 §10.3 手順 1b)。
//! `MemFs` に保存中フラグと痕跡のファイルを積み、起動時の `RecoveryScan` が作る提案を確かめる。

use eve_core::{FileTime, FsPath, InflightRecord, Leftover, LeftoverKind, SaveMethod};
use eve_testkit::Harness;

/// 起動していない Harness (走査は `start` で起こす)。
fn env() -> Harness {
    Harness::builder().defer_start().build()
}

/// 起動して走査の結果を受ける。
fn start(h: &mut Harness) -> Vec<Leftover> {
    h.editor.open_initial(None, false);
    h.run_jobs();
    h.editor.leftovers().to_vec()
}

/// 原子的保存の中断の記録を積む。`seq` はフラグの名前、`secs` は保存の開始時刻。
fn record(h: &Harness, method: SaveMethod, target: &str, seq: u32, secs: i64) -> InflightRecord {
    let r = make(method, target, seq, secs);
    h.fs.add_inflight(r.clone());
    r
}

fn make(method: SaveMethod, target: &str, seq: u32, secs: i64) -> InflightRecord {
    InflightRecord {
        target: FsPath::from(target),
        pid: 100,
        started_at: Some(FileTime { secs, nanos: 0 }),
        method,
        tmp: Some(FsPath::from(format!("{target}.eve-tmp-{seq}").as_str())),
        backup: Some(FsPath::from(format!("{target}.eve-bak-{seq}").as_str())),
        backup_recorded: false,
        backup_complete: false,
        flag: FsPath::from(format!("/s/inflight/{seq}.toml").as_str()),
        malformed: false,
    }
}

#[test]
fn 痕跡が無ければ提案しない() {
    let mut s = env();
    let actual = start(&mut s);
    assert!(actual.is_empty(), "{actual:?}");
}

// verifies: NFR-301
#[test]
fn t_が無く_bak_だけなら_bak_を戻す提案() {
    let mut s = env();
    record(&s, SaveMethod::Atomic, "/w/a.txt", 1, 10);
    s.fs.add_file("/w/a.txt.eve-bak-1", b"old");
    let l = start(&mut s);
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].kind, LeftoverKind::Bak);
    assert_eq!(l[0].target, FsPath::from("/w/a.txt"));
    assert_eq!(l[0].bak, Some(FsPath::from("/w/a.txt.eve-bak-1")));
    assert_eq!(l[0].tmp, None);
    assert_eq!(l[0].flags, vec![FsPath::from("/s/inflight/1.toml")]);
}

// verifies: NFR-301
#[test]
fn t_が無く_bak_と_tmp_の両方なら両方の提案() {
    let mut s = env();
    record(&s, SaveMethod::Atomic, "/w/a.txt", 1, 10);
    s.fs.add_file("/w/a.txt.eve-bak-1", b"old");
    s.fs.add_file("/w/a.txt.eve-tmp-1", b"new");
    let l = start(&mut s);
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].kind, LeftoverKind::BakAndTmp);
    assert!(l[0].bak.is_some() && l[0].tmp.is_some());
}

#[test]
fn t_が在って_bak_があれば削除漏れ_tmp_もあれば別の提案を続ける() {
    let mut s = env();
    record(&s, SaveMethod::Atomic, "/w/a.txt", 1, 10);
    s.fs.add_file("/w/a.txt", b"new");
    s.fs.add_file("/w/a.txt.eve-bak-1", b"old");
    let l = start(&mut s);
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].kind, LeftoverKind::StaleBak);

    let mut s = env();
    record(&s, SaveMethod::Atomic, "/w/a.txt", 1, 10);
    s.fs.add_file("/w/a.txt", b"new");
    s.fs.add_file("/w/a.txt.eve-bak-1", b"old");
    s.fs.add_file("/w/a.txt.eve-tmp-1", b"x");
    let l = start(&mut s);
    let kinds: Vec<_> = l.iter().map(|x| x.kind).collect();
    assert_eq!(kinds, vec![LeftoverKind::StaleBak, LeftoverKind::Tmp]);
}

#[test]
fn tmp_だけなら_t_の有無に依らず_tmp_の提案() {
    for t_exists in [false, true] {
        let mut s = env();
        record(&s, SaveMethod::NewFile, "/w/n.txt", 1, 10);
        s.fs.add_file("/w/n.txt.eve-tmp-1", b"x");
        if t_exists {
            s.fs.add_file("/w/n.txt", b"y");
        }
        let l = start(&mut s);
        assert_eq!(l.len(), 1, "t_exists={t_exists}: {l:?}");
        assert_eq!(l[0].kind, LeftoverKind::Tmp);
        assert_eq!(l[0].tmp, Some(FsPath::from("/w/n.txt.eve-tmp-1")));
    }
}

#[test]
fn 痕跡が何も残っていない記録はフラグだけを消す() {
    let mut s = env();
    record(&s, SaveMethod::Atomic, "/w/a.txt", 1, 10);
    let actual = start(&mut s);
    assert!(actual.is_empty(), "{actual:?}");
    assert!(s.fs.inflight_records().is_empty(), "フラグを掃除する");
}

#[test]
fn 提案したフラグは残す() {
    let mut s = env();
    record(&s, SaveMethod::Atomic, "/w/a.txt", 1, 10);
    s.fs.add_file("/w/a.txt.eve-bak-1", b"old");
    start(&mut s);
    assert_eq!(s.fs.inflight_records().len(), 1, "片付けるまで残す");
}

#[test]
fn 直接上書きと壊れたフラグは対象にせずフラグを残す() {
    let mut s = env();
    let mut inplace = make(SaveMethod::InPlace, "/w/a.txt", 1, 10);
    inplace.backup_recorded = true;
    inplace.backup_complete = true;
    s.fs.add_inflight(inplace);
    s.fs.add_file("/w/a.txt.eve-bak-1", b"old");
    let mut bad = make(SaveMethod::Atomic, "", 2, 10);
    bad.malformed = true;
    s.fs.add_inflight(bad);
    let l = start(&mut s);
    assert!(l.is_empty(), "{l:?}");
    assert_eq!(s.fs.inflight_records().len(), 2, "どちらのフラグも残す");
}

#[test]
fn 複数の提案は記録の新しい順() {
    let mut s = env();
    record(&s, SaveMethod::Atomic, "/w/old.txt", 1, 10);
    record(&s, SaveMethod::Atomic, "/w/new.txt", 2, 30);
    record(&s, SaveMethod::Atomic, "/w/mid.txt", 3, 20);
    for n in ["old", "new", "mid"] {
        let seq = match n {
            "old" => 1,
            "new" => 2,
            _ => 3,
        };
        s.fs.add_file(&format!("/w/{n}.txt.eve-bak-{seq}"), b"b");
    }
    let l = start(&mut s);
    let targets: Vec<String> = l.iter().map(|x| x.target.display()).collect();
    assert_eq!(targets, vec!["/w/new.txt", "/w/mid.txt", "/w/old.txt"]);
}

#[test]
fn 同じ_t_の記録は_1_件にまとめる() {
    let mut s = env();
    record(&s, SaveMethod::Atomic, "/w/a.txt", 1, 10);
    record(&s, SaveMethod::Atomic, "/w/a.txt", 2, 20);
    s.fs.add_file("/w/a.txt.eve-bak-1", b"old");
    s.fs.add_file("/w/a.txt.eve-tmp-2", b"x");
    let l = start(&mut s);
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].kind, LeftoverKind::BakAndTmp);
    assert_eq!(l[0].flags.len(), 2);
    assert_eq!(l[0].recorded_at, Some(FileTime { secs: 20, nanos: 0 }));
}

#[test]
fn 記録の無い書込途中の事前バックアップだけを片付ける() {
    let mut s = env();
    s.fs.add_orphan_backup("/b/k-1-100.bak.tmp", true, 3);
    s.fs.add_orphan_backup("/b/k-2-100.bak", false, 3);
    // フラグが記録している事前バックアップの書込途中の写しは残す
    let mut r = make(SaveMethod::InPlace, "/w/a.txt", 1, 10);
    r.backup = Some(FsPath::from("/b/k-3-100.bak"));
    s.fs.add_inflight(r);
    s.fs.add_orphan_backup("/b/k-3-100.bak.tmp", true, 1);
    start(&mut s);
    let left: Vec<String> =
        s.fs.orphan_backup_paths()
            .iter()
            .map(FsPath::display)
            .collect();
    assert_eq!(
        left,
        vec!["/b/k-2-100.bak", "/b/k-3-100.bak.tmp"],
        "完全な写しと、記録のある写しは残す"
    );
}
