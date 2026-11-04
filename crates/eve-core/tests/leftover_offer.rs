//! 置換途中の痕跡の提案と適用 (docs/05 §4.6「起動時の提案の規則」)。`MemFs` に保存中フラグと痕跡のファイルを積み、
//! 起動時の走査 → 確認ダイアログ → 戻す / 削除 / 別バッファで開く、を確かめる。

use eve_core::{FileTime, FsPath, InflightRecord, SaveMethod};
use eve_testkit::Harness;
use eve_view::OverlayModel;

fn ct(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.as_str()).collect()
}

/// 確認ダイアログのタイトル・メッセージ・選択肢をつなげた文字列。
fn dialog_text(h: &mut Harness) -> String {
    assert_eq!(h.overlay(), "dialog");
    let f = h.frame();
    let OverlayModel::Confirm {
        title,
        message,
        choices,
        ..
    } = &f.overlays[0]
    else {
        panic!("確認ダイアログではない");
    };
    let mut out = vec![ct(title), ct(message)];
    out.extend(choices.iter().map(|c| ct(&c.label)));
    out.join("\n")
}

fn record(target: &str, seq: u32, secs: i64) -> InflightRecord {
    InflightRecord {
        target: FsPath::from(target),
        pid: 100,
        started_at: Some(FileTime { secs, nanos: 0 }),
        method: SaveMethod::Atomic,
        tmp: Some(FsPath::from(format!("{target}.eve-tmp-{seq}").as_str())),
        backup: Some(FsPath::from(format!("{target}.eve-bak-{seq}").as_str())),
        backup_recorded: false,
        backup_complete: false,
        flag: FsPath::from(format!("/s/inflight/{seq}.toml").as_str()),
        malformed: false,
    }
}

/// 痕跡を積んで起動し、走査を終えて tick まで進める。
fn started(setup: impl FnOnce(&Harness)) -> Harness {
    let mut h = Harness::new();
    setup(&h);
    h.run_jobs();
    h.advance_ms(0);
    h
}

#[test]
fn 痕跡が無ければダイアログを出さない() {
    let h = started(|_| {});
    assert_eq!(h.overlay(), "");
}

#[test]
fn bak_だけなら元に戻す提案で_戻すと_t_が復活しフラグが消える() {
    let mut h = started(|h| {
        h.fs.add_inflight(record("/w/a.txt", 1, 10));
        h.fs.add_file("/w/a.txt.eve-bak-1", b"original");
    });
    let t = dialog_text(&mut h);
    assert!(t.contains("保存の途中で中断された痕跡"), "{t}");
    assert!(
        t.contains("[R] 元に戻す") && t.contains("[D] 退避名を削除"),
        "{t}"
    );
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.fs_read("/w/a.txt").as_deref(), Some(&b"original"[..]));
    assert!(!h.fs.exists("/w/a.txt.eve-bak-1"));
    assert!(
        h.fs.inflight_records().is_empty(),
        "痕跡が無くなったのでフラグも消す"
    );
    assert!(h.messages().contains(&"info.leftover.restored".to_owned()));
    assert_eq!(h.overlay(), "");
}

#[test]
fn bak_を削除する選択() {
    let mut h = started(|h| {
        h.fs.add_inflight(record("/w/a.txt", 1, 10));
        h.fs.add_file("/w/a.txt.eve-bak-1", b"original");
    });
    h.answer(1);
    h.run_jobs();
    assert!(!h.fs.exists("/w/a.txt.eve-bak-1"));
    assert!(!h.fs.exists("/w/a.txt"), "戻していない");
    assert!(h.fs.inflight_records().is_empty());
    assert!(h.messages().contains(&"info.leftover.deleted".to_owned()));
}

#[test]
fn 内容が同一の退避名は確認なしで消す() {
    let h = started(|h| {
        h.fs.add_inflight(record("/w/a.txt", 1, 10));
        h.fs.add_file("/w/a.txt", b"same");
        h.fs.add_file("/w/a.txt.eve-bak-1", b"same");
    });
    assert_eq!(h.overlay(), "", "同一なので提案しない");
    assert!(!h.fs.exists("/w/a.txt.eve-bak-1"));
    assert!(h.fs.inflight_records().is_empty());
}

#[test]
fn 内容が違う退避名は削除漏れとして提案する() {
    let mut h = started(|h| {
        h.fs.add_inflight(record("/w/a.txt", 1, 10));
        h.fs.add_file("/w/a.txt", b"new");
        h.fs.add_file("/w/a.txt.eve-bak-1", b"old");
    });
    let t = dialog_text(&mut h);
    assert!(t.contains("退避名が残っています"), "{t}");
    h.answer(0);
    h.run_jobs();
    assert!(!h.fs.exists("/w/a.txt.eve-bak-1"));
    assert_eq!(h.fs_read("/w/a.txt").as_deref(), Some(&b"new"[..]));
}

#[test]
fn tmp_は読み取り専用の別バッファで開き_削除は別の選択() {
    let mut h = started(|h| {
        h.fs.add_inflight(record("/w/n.txt", 1, 10));
        h.fs.add_file("/w/n.txt.eve-tmp-1", b"partial");
    });
    let t = dialog_text(&mut h);
    assert!(t.contains("[O] 別バッファで開いて確認"), "{t}");
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), "partial");
    assert_eq!(h.doc_path().as_deref(), Some("/w/n.txt.eve-tmp-1"));
    assert!(h.messages().contains(&"msg.leftover.tmp_opened".to_owned()));
    assert!(h.fs.exists("/w/n.txt.eve-tmp-1"), "開くだけで消さない");
    assert_eq!(
        h.fs.inflight_records().len(),
        1,
        "削除を選ぶまでフラグは残す"
    );
    // 読み取り専用: 編集できない
    h.text("x");
    assert_eq!(h.doc_text(), "partial");
}

#[test]
fn tmp_を削除する選択() {
    let mut h = started(|h| {
        h.fs.add_inflight(record("/w/n.txt", 1, 10));
        h.fs.add_file("/w/n.txt.eve-tmp-1", b"partial");
    });
    h.answer(1);
    h.run_jobs();
    assert!(!h.fs.exists("/w/n.txt.eve-tmp-1"));
    assert!(h.fs.inflight_records().is_empty());
}

#[test]
fn 両方なら戻してから_tmp_を開き_tmp_のフラグは残す() {
    let mut h = started(|h| {
        h.fs.add_inflight(record("/w/a.txt", 1, 10));
        h.fs.add_file("/w/a.txt.eve-bak-1", b"original");
        h.fs.add_file("/w/a.txt.eve-tmp-1", b"new-partial");
    });
    let t = dialog_text(&mut h);
    assert!(
        t.contains("[B] 元に戻して、新しい内容を別バッファで開く"),
        "{t}"
    );
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.fs_read("/w/a.txt").as_deref(), Some(&b"original"[..]));
    assert_eq!(h.doc_text(), "new-partial", "tmp を開いている");
    assert_eq!(
        h.fs.inflight_records().len(),
        1,
        "tmp が残るのでフラグも残す"
    );
}

#[test]
fn 戻せなければ_tmp_は開かず失敗を知らせる() {
    let mut h = started(|h| {
        h.fs.add_inflight(record("/w/a.txt", 1, 10));
        h.fs.add_file("/w/a.txt.eve-bak-1", b"original");
        h.fs.add_file("/w/a.txt.eve-tmp-1", b"new-partial");
    });
    // 提案の後に T が作られた: 置き換えずに失敗する
    h.fs.add_file("/w/a.txt", b"created-meanwhile");
    h.answer(0);
    h.run_jobs();
    assert!(h.messages().contains(&"msg.leftover.failed".to_owned()));
    assert_eq!(
        h.fs_read("/w/a.txt").as_deref(),
        Some(&b"created-meanwhile"[..])
    );
    assert!(h.fs.exists("/w/a.txt.eve-bak-1"), "退避名は残す");
    assert_ne!(h.doc_text(), "new-partial");
}

#[test]
fn esc_は何もせず痕跡を残す() {
    let mut h = started(|h| {
        h.fs.add_inflight(record("/w/a.txt", 1, 10));
        h.fs.add_file("/w/a.txt.eve-bak-1", b"original");
    });
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert!(h.fs.exists("/w/a.txt.eve-bak-1"));
    assert_eq!(h.fs.inflight_records().len(), 1);
    // 同じ起動の間は再び出さない
    h.advance_ms(5000);
    assert_eq!(h.overlay(), "");
}

#[test]
fn 複数の提案は新しい順に_1_件ずつ() {
    let mut h = started(|h| {
        h.fs.add_inflight(record("/w/old.txt", 1, 10));
        h.fs.add_file("/w/old.txt.eve-bak-1", b"o");
        h.fs.add_inflight(record("/w/new.txt", 2, 30));
        h.fs.add_file("/w/new.txt.eve-bak-2", b"n");
    });
    let t = dialog_text(&mut h);
    assert!(t.contains("/w/new.txt"), "新しい方が先: {t}");
    h.answer(0);
    h.run_jobs();
    h.advance_ms(0);
    let t = dialog_text(&mut h);
    assert!(t.contains("/w/old.txt"), "{t}");
}
