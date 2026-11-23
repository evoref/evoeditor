//! ファイル監視 (FR-117 の監視の部分、docs/03 §9.5.1 の WATCH-1〜WATCH-11。v0.5 の M33 の段 8)。
//!
//! 偽の監視 (`FakeWatcher`) で知らせを入れ、手動の時計 (`ManualClock`) で待ち合わせ (100 ms / 最長 1 s) を決定的に越える。

use eve_config::{Config, ExternalChangePolicy};
use eve_core::{
    Clock as _, DocumentId, ExternalPending, FsEventKind, WATCH_RELOAD_MIN_INTERVAL, WATCH_SETTLE,
    WATCH_SETTLE_MAX, WatchError, WatchId,
};
use eve_testkit::{FakeWatcher, FsOp, Harness, WatchRequest};

/// 監視 (登録にすぐ `Started` を返す) と、`/w` のファイル。起動の無題の文書が出ている。
fn watched(files: &[(&str, &[u8])]) -> (Harness, FakeWatcher) {
    watched_with(FakeWatcher::new().auto_started(), files)
}

fn watched_with(fake: FakeWatcher, files: &[(&str, &[u8])]) -> (Harness, FakeWatcher) {
    let mut b = Harness::builder().watcher(fake.clone());
    for (p, d) in files {
        b = b.file(p, d);
    }
    (b.build(), fake)
}

/// ファイルを開いて読込を終え、登録の結果を渡す。開いた文書を返す。
fn open(h: &mut Harness, path: &str) -> DocumentId {
    h.open(path);
    h.run_jobs();
    h.pump_fs();
    h.doc_id()
}

fn id_of(fake: &FakeWatcher, dir: &str) -> WatchId {
    fake.id_of(dir).expect("監視している")
}

/// 外部のプロセスが書いた (ファイルを替えて、名前の知らせを入れる)。
fn write_outside(h: &mut Harness, fake: &FakeWatcher, path: &str, data: &[u8]) {
    h.fs.add_file(path, data);
    let (dir, name) = path.rsplit_once('/').expect("パス");
    fake.changed(id_of(fake, dir), name);
    h.pump_fs();
}

/// 待ち合わせを越えて照合を走らせる。
fn settle(h: &mut Harness) {
    h.advance_ms(WATCH_SETTLE.0);
    h.run_jobs();
}

fn pending_of(h: &Harness, doc: DocumentId) -> Option<ExternalPending> {
    h.editor
        .state()
        .document(doc)
        .expect("文書")
        .external_pending
}

fn text_of(h: &Harness, doc: DocumentId) -> String {
    h.editor
        .state()
        .document(doc)
        .expect("文書")
        .to_string_lossy()
}

fn watch_messages(h: &Harness) -> Vec<String> {
    h.messages()
        .into_iter()
        .filter(|k| k.starts_with("info.external.") || k.starts_with("info.watch."))
        .collect()
}

fn set_policy(h: &mut Harness, watch: bool, policy: ExternalChangePolicy) {
    h.editor.state_mut().set_config(Config {
        watch_enabled: watch,
        reload_on_external_change: policy,
        ..Config::default()
    });
}

// ---------- AC-7: 印とメッセージ・表示中の未編集の文書の再読込・確認を出さない ----------

// verifies: AC-FR-117-7
#[test]
fn 表示していない文書の外部の変更は印とメッセージで知らせ_確認を出さない() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    let b = open(&mut h, "/w/b.txt");
    assert_ne!(a, b);
    assert_eq!(
        fake.active().len(),
        1,
        "同じディレクトリの文書は 1 つの監視"
    );
    h.clear_log();
    write_outside(&mut h, &fake, "/w/a.txt", b"changed\n");
    settle(&mut h);
    assert_eq!(h.overlay(), "", "監視のイベントでは確認を出さない");
    assert_eq!(watch_messages(&h), vec!["info.external.changed"]);
    assert!(matches!(
        pending_of(&h, a),
        Some(ExternalPending::Modified(_))
    ));
    assert_eq!(text_of(&h, a), "a\n", "表示していない文書は読み直さない");
    assert_eq!(pending_of(&h, b), None);
    // 同じ変更の知らせがもう一度来ても、印もメッセージも出し直さない (WATCH-7)
    h.clear_log();
    fake.changed(id_of(&fake, "/w"), "a.txt");
    h.pump_fs();
    settle(&mut h);
    assert!(watch_messages(&h).is_empty(), "{:?}", h.messages());
    // バッファ一覧の行に印が出る (文字コードの欄の前、04 §8.5)
    h.cmd("buffer.list");
    let details = list_details(&mut h);
    assert!(
        details.iter().any(|d| d.ends_with("外部で変更  UTF-8")),
        "{details:?}"
    );
    assert!(
        details.iter().any(|d| d.ends_with("/w  UTF-8")),
        "印の無い行は今のまま: {details:?}"
    );
}

/// バッファ一覧の行の右の欄 (ディレクトリ・印・文字コード)。
fn list_details(h: &mut Harness) -> Vec<String> {
    let f = h.frame();
    let Some(eve_view::OverlayModel::List { items, .. }) = f.overlays.first() else {
        panic!("一覧が出ていない");
    };
    items
        .iter()
        .map(|i| {
            i.detail
                .as_ref()
                .map(|d| d.spans.iter().map(|s| s.text.as_str()).collect())
                .unwrap_or_default()
        })
        .collect()
}

// verifies: AC-FR-117-7
#[test]
fn 表示中の未編集の文書は確認なしで読み直し_undo_で戻せる() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    h.clear_log();
    write_outside(&mut h, &fake, "/w/a.txt", b"new\n");
    settle(&mut h);
    assert_eq!(h.overlay(), "");
    assert_eq!(text_of(&h, a), "new\n");
    assert!(!h.is_dirty());
    assert_eq!(pending_of(&h, a), None, "読み直したら印は付けない");
    assert!(
        h.messages().contains(&"info.external.reloaded".to_owned()),
        "{:?}",
        h.messages()
    );
    h.keys("ctrl+z");
    assert_eq!(text_of(&h, a), "a\n", "Undo で読み直す前に戻る");
}

// verifies: AC-FR-117-7
#[test]
fn 表示中でも編集中の文書は読み直さず印を付ける() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    h.text("x");
    h.clear_log();
    write_outside(&mut h, &fake, "/w/a.txt", b"new\n");
    settle(&mut h);
    assert_eq!(h.overlay(), "");
    assert_eq!(text_of(&h, a), "xa\n");
    assert!(matches!(
        pending_of(&h, a),
        Some(ExternalPending::Modified(_))
    ));
    assert_eq!(watch_messages(&h), vec!["info.external.changed"]);
    assert!(h.frame_contains("[外部で変更]"), "ステータスバーの印");
    // 保存時の照合の確認は今のまま出る (WATCH-8 の (d))
    h.keys("ctrl+s");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
}

// verifies: AC-FR-117-7
#[test]
fn 外部で削除されたら削除の印とメッセージ_作り直されたら変更として付け直す() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    h.clear_log();
    h.fs.remove(&"/w/a.txt".into());
    fake.changed(id_of(&fake, "/w"), "a.txt");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(pending_of(&h, a), Some(ExternalPending::Deleted));
    assert_eq!(watch_messages(&h), vec!["info.external.deleted"]);
    assert_eq!(h.overlay(), "");
    h.clear_log();
    write_outside(&mut h, &fake, "/w/a.txt", b"again\n");
    settle(&mut h);
    assert!(matches!(
        pending_of(&h, a),
        Some(ExternalPending::Modified(_))
    ));
    assert_eq!(watch_messages(&h), vec!["info.external.changed"]);
}

// verifies: AC-FR-117-7
#[test]
fn 同じ回の照合で_2_つ以上の文書に印が付いたら_1_件にまとめる() {
    let (mut h, fake) = watched(&[
        ("/w/a.txt", b"a\n"),
        ("/w/b.txt", b"b\n"),
        ("/w/c.txt", b"c\n"),
    ]);
    let a = open(&mut h, "/w/a.txt");
    let b = open(&mut h, "/w/b.txt");
    let c = open(&mut h, "/w/c.txt");
    h.fs.add_file("/w/a.txt", b"A\n");
    h.fs.add_file("/w/b.txt", b"B\n");
    h.clear_log();
    // あふれ (git checkout など): ディレクトリの全文書を照合する
    fake.push(id_of(&fake, "/w"), FsEventKind::Rescan);
    h.pump_fs();
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 3, "文書ごとに 1 回の照合");
    h.run_jobs();
    assert_eq!(watch_messages(&h), vec!["info.external.changed_many"]);
    assert!(pending_of(&h, a).is_some() && pending_of(&h, b).is_some());
    assert_eq!(pending_of(&h, c), None, "変わっていない表示中の文書");
}

// verifies: AC-FR-117-7
#[test]
fn 切り替えたら照合し_未編集なら読み直し_編集中なら確認を出さずに知らせる() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    let b = open(&mut h, "/w/b.txt");
    write_outside(&mut h, &fake, "/w/a.txt", b"A1\n");
    settle(&mut h);
    assert!(pending_of(&h, a).is_some());
    // 未編集の文書へ切り替える: 照合して読み直す
    h.clear_log();
    h.cmd("buffer.prev");
    assert_eq!(h.doc_id(), a);
    h.advance_ms(1);
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(text_of(&h, a), "A1\n");
    assert_eq!(pending_of(&h, a), None);
    assert!(h.messages().contains(&"info.external.reloaded".to_owned()));
    // 編集中の文書 (表示していない) の変更: 切り替えても確認のダイアログを出さず、メッセージで知らせて印を残す
    h.cmd("buffer.prev");
    assert_eq!(h.doc_id(), b);
    h.text("x");
    h.cmd("buffer.prev");
    assert_eq!(h.doc_id(), a);
    h.advance_ms(1);
    h.run_jobs();
    write_outside(&mut h, &fake, "/w/b.txt", b"B1\n");
    settle(&mut h);
    assert!(pending_of(&h, b).is_some());
    h.clear_log();
    h.cmd("buffer.prev");
    assert_eq!(h.doc_id(), b);
    h.advance_ms(1);
    h.run_jobs();
    assert_eq!(
        h.overlay(),
        "",
        "切り替えた直後に確認を出さない (07 §12.6.3)"
    );
    assert_eq!(watch_messages(&h), vec!["info.external.changed"]);
    assert_eq!(text_of(&h, b), "xb\n");
    assert!(pending_of(&h, b).is_some());
}

// verifies: AC-FR-117-7
#[test]
fn 無視を選んだ変更は監視で知らせ直さず_保存と再読込で印が外れる() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    h.text("x");
    write_outside(&mut h, &fake, "/w/a.txt", b"new\n");
    settle(&mut h);
    assert!(pending_of(&h, a).is_some());
    // フォーカス復帰の確認 (今の流れ) で「無視」を選ぶ: 印は `external_seen` に移る
    h.focus(false);
    h.focus(true);
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    h.keys("enter");
    assert_eq!(pending_of(&h, a), None);
    h.clear_log();
    fake.changed(id_of(&fake, "/w"), "a.txt");
    h.pump_fs();
    settle(&mut h);
    assert!(watch_messages(&h).is_empty(), "無視した変更は知らせない");
    assert!(h.frame_contains("[外部で変更]"));
    // 上書き保存の確認で上書きを選ぶ: 印が外れる
    h.keys("ctrl+s");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    h.answer(0);
    h.run_jobs();
    assert!(!h.frame_contains("[外部で変更]"), "{}", h.frame_text());
}

// ---------- WATCH-6 の上限 ----------

// verifies: AC-FR-117-7
#[test]
fn 同じ文書を監視で読み直すのは_5_秒に_1_回で_間の変更は印にして後で読み直す() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    write_outside(&mut h, &fake, "/w/a.txt", b"1\n");
    settle(&mut h);
    assert_eq!(text_of(&h, a), "1\n");
    h.advance_ms(1000);
    write_outside(&mut h, &fake, "/w/a.txt", b"2\n");
    settle(&mut h);
    assert_eq!(text_of(&h, a), "1\n", "5 s の内は読み直さない");
    assert!(pending_of(&h, a).is_some());
    // 最後に読み直してから 5 s を過ぎたら、照合し直して読み直す
    let left = WATCH_RELOAD_MIN_INTERVAL.0 - 1000 - WATCH_SETTLE.0;
    h.advance_ms(left - 1);
    assert_eq!(h.pending_jobs(), 0);
    h.advance_ms(1);
    h.run_jobs();
    assert_eq!(text_of(&h, a), "2\n");
    assert_eq!(pending_of(&h, a), None);
}

// verifies: AC-FR-117-7
#[test]
fn redo_の履歴がある文書と_16_mib_を超えるファイルは読み直さず印を付ける() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/big.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    h.text("x");
    h.keys("ctrl+z");
    assert!(!h.is_dirty());
    write_outside(&mut h, &fake, "/w/a.txt", b"new\n");
    settle(&mut h);
    assert_eq!(text_of(&h, a), "a\n", "Redo を消さない");
    assert!(pending_of(&h, a).is_some());
    let big = open(&mut h, "/w/big.txt");
    h.fs.add_file("/w/big.txt", b"B\n");
    h.fs.set_fake_len("/w/big.txt", Some(16 * 1024 * 1024 + 1));
    fake.changed(id_of(&fake, "/w"), "big.txt");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(text_of(&h, big), "b\n");
    assert!(pending_of(&h, big).is_some());
}

// verifies: AC-FR-117-7
#[test]
fn 開く前の確認が要る内容になったら確認を出さずに印にする() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    h.editor.state_mut().set_config(Config {
        huge_file_warn_bytes: 1_048_576,
        ..Config::default()
    });
    let a = open(&mut h, "/w/a.txt");
    h.clear_log();
    write_outside(&mut h, &fake, "/w/a.txt", &vec![b'x'; 1_200_000]);
    settle(&mut h);
    assert_eq!(h.overlay(), "", "監視のイベントでモーダルの確認を出さない");
    assert_eq!(text_of(&h, a), "a\n");
    assert!(pending_of(&h, a).is_some());
    assert_eq!(watch_messages(&h), vec!["info.external.changed"]);
    h.text("y");
    assert_eq!(text_of(&h, a), "ya\n", "読込中のままにしない");
}

// verifies: AC-FR-117-7
#[test]
fn 監視で読み直している間に打鍵したら読込を取り消して印にし_打った文字を残す() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    write_outside(&mut h, &fake, "/w/a.txt", b"new\n");
    h.advance_ms(WATCH_SETTLE.0);
    // 照合だけを走らせる (読込のジョブは積まれたまま)
    assert!(h.run_one_job());
    assert_eq!(h.pending_jobs(), 1, "再読込のジョブ");
    h.text("z");
    h.run_jobs();
    assert_eq!(text_of(&h, a), "za\n");
    assert!(pending_of(&h, a).is_some());
    assert!(h.is_dirty());
}

// ---------- AC-8: 自分の保存・待ち合わせ ----------

// verifies: AC-FR-117-8
#[test]
fn 自分の保存の知らせでは印もメッセージも出ない() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    h.text("x");
    h.clear_log();
    h.keys("ctrl+s");
    // 保存の途中に、一時ファイルの作成と置換の知らせが届く (待ち合わせの期限が保存の間に来る)
    let id = id_of(&fake, "/w");
    fake.changed(id, ".a.txt.eve-tmp");
    fake.changed(id, "a.txt");
    h.pump_fs();
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 1, "保存の間は照合を延ばす");
    h.run_jobs();
    assert_eq!(h.fs_read("/w/a.txt").as_deref(), Some(&b"xa\n"[..]));
    assert_eq!(pending_of(&h, a), None);
    assert!(watch_messages(&h).is_empty(), "{:?}", h.messages());
    // 保存の後に届いた自分の書込の知らせ (直接上書きなど) も同じ
    fake.changed(id, "a.txt");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(pending_of(&h, a), None);
    assert_eq!(watch_messages(&h), Vec::<String>::new());
}

// verifies: AC-FR-117-8
#[test]
fn 続けて届いた知らせは最後から_100_ms_待って_1_回の照合にまとめる() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    let id = id_of(&fake, "/w");
    h.fs.add_file("/w/a.txt", b"A\n");
    for _ in 0..3 {
        fake.changed(id, "a.txt");
        h.pump_fs();
        h.advance_ms(30);
    }
    // 最後の知らせは 60 ms の時点。その 100 ms 後 (160 ms) まで照合しない
    h.advance_ms(69);
    assert_eq!(h.pending_jobs(), 0);
    h.advance_ms(1);
    assert_eq!(h.pending_jobs(), 1);
    h.run_jobs();
    assert!(pending_of(&h, a).is_some());
}

// verifies: AC-FR-117-8
#[test]
fn 書き続けるファイルは最初の知らせから_1_秒で打ち切って照合する() {
    let (mut h, fake) = watched(&[("/w/log.txt", b"0\n"), ("/w/b.txt", b"b\n")]);
    let log = open(&mut h, "/w/log.txt");
    open(&mut h, "/w/b.txt");
    let id = id_of(&fake, "/w");
    let mut checks = 0;
    for i in 0..30u32 {
        h.fs.add_file("/w/log.txt", format!("{i}\n").as_bytes());
        fake.changed(id, "log.txt");
        h.pump_fs();
        h.advance_ms(50);
        checks += h.pending_jobs();
        h.run_jobs();
    }
    // 1.5 s の間、50 ms ごとに書いた: 1 s の打ち切りで 1 回 (待ち合わせの 100 ms には一度も達しない)
    assert_eq!(checks, 1);
    assert!(pending_of(&h, log).is_some());
}

// verifies: AC-FR-117-8
#[test]
fn 打ち切りの照合の直前の知らせの後の書込は_待ち合わせの期限の照合が取り込む() {
    // 07 §12.6.2 の段 8 の行の場面: 打ち切りの照合の 10 ms 前に知らせが届き、その後の書込の知らせは eve-platform の
    // まとめ (50 ms) で捨てられて届かない。待ち合わせの期限 (最後の知らせ + 100 ms) の照合が最後の内容を取り込む
    let (mut h, fake) = watched(&[("/w/log.txt", b"0\n"), ("/w/b.txt", b"b\n")]);
    let log = open(&mut h, "/w/log.txt");
    open(&mut h, "/w/b.txt");
    let id = id_of(&fake, "/w");
    let t0 = h.clock.now().0;
    // 0, 50, …, 900 ms に書いて知らせる
    for i in 0..19u32 {
        h.fs.add_file("/w/log.txt", format!("{i}\n").as_bytes());
        fake.changed(id, "log.txt");
        h.pump_fs();
        h.advance_ms(50);
        assert_eq!(h.pending_jobs(), 0, "{i}");
    }
    // 990 ms: 打ち切りの 10 ms 前の知らせ
    h.advance_ms(40);
    h.fs.add_file("/w/log.txt", b"v990\n");
    fake.changed(id, "log.txt");
    h.pump_fs();
    h.advance_ms(9);
    assert_eq!(h.pending_jobs(), 0);
    // 1000 ms: 打ち切りの照合 (990 ms の内容)
    h.advance_ms(1);
    assert_eq!(h.clock.now().0 - t0, WATCH_SETTLE_MAX.0);
    assert_eq!(h.pending_jobs(), 1, "打ち切りの照合");
    h.run_jobs();
    let first = match pending_of(&h, log) {
        Some(ExternalPending::Modified(s)) => s,
        other => panic!("{other:?}"),
    };
    assert_eq!(first.len, 5);
    // 1010 ms: 書き込むが、知らせは eve-platform のまとめで捨てられて届かない
    h.advance_ms(10);
    h.fs.add_file("/w/log.txt", b"final-content\n");
    // 1089 ms: まだ照合しない
    h.advance_ms(79);
    assert_eq!(h.pending_jobs(), 0);
    // 1090 ms (990 + 100): 待ち合わせの期限の照合が最後の内容を取り込む
    h.advance_ms(1);
    assert_eq!(h.pending_jobs(), 1, "期限の照合");
    h.run_jobs();
    match pending_of(&h, log) {
        Some(ExternalPending::Modified(s)) => {
            assert_eq!(s.len, "final-content\n".len() as u64);
        }
        other => panic!("{other:?}"),
    }
    // その後は照合しない (一定の間隔で起きない)
    assert_eq!(h.editor.next_deadline(), None);
}

// verifies: AC-FR-117-8
#[test]
fn 照合の間に届いた知らせは結果の後にもう一度照合する() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    let id = id_of(&fake, "/w");
    h.fs.add_file("/w/a.txt", b"1\n");
    fake.changed(id, "a.txt");
    h.pump_fs();
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 1);
    // 照合のジョブが走る前に、次の書込と知らせ
    let held = h.run_one_job_held();
    h.fs.add_file("/w/a.txt", b"22\n");
    fake.changed(id, "a.txt");
    h.pump_fs();
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 0, "照合の結果を待つ間は延ばす");
    h.deliver(held);
    assert_eq!(h.pending_jobs(), 1, "結果の後に待たずに照合する");
    h.run_jobs();
    match pending_of(&h, a) {
        Some(ExternalPending::Modified(s)) => assert_eq!(s.len, 3),
        other => panic!("{other:?}"),
    }
}

// verifies: AC-FR-117-8
#[test]
fn 照合の失敗は_0_5_秒と_2_秒の後にやり直す() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    h.fs.busy_times(FsOp::Metadata, 2);
    write_outside(&mut h, &fake, "/w/a.txt", b"A\n");
    settle(&mut h);
    assert_eq!(pending_of(&h, a), None, "読めない");
    h.advance_ms(499);
    assert_eq!(h.pending_jobs(), 0);
    h.advance_ms(1);
    assert_eq!(h.pending_jobs(), 1, "0.5 s 後");
    h.run_jobs();
    h.advance_ms(1999);
    assert_eq!(h.pending_jobs(), 0);
    h.advance_ms(1);
    assert_eq!(h.pending_jobs(), 1, "2 s 後");
    h.run_jobs();
    assert!(pending_of(&h, a).is_some());
    assert_eq!(h.editor.next_deadline(), None);
}

// verifies: AC-FR-117-8
#[test]
fn 呼び戻しの起きるファイルは内容ハッシュを使わずに照合する() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"aa\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    let id = id_of(&fake, "/w");
    // 同じ長さ・同じ更新時刻で中身だけが違う: 内容ハッシュを使えば変更と分かる
    h.fs.set_data_keep_mtime("/w/a.txt", b"bb\n");
    h.fs.set_offline("/w/a.txt", true);
    fake.changed(id, "a.txt");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(
        pending_of(&h, a),
        None,
        "オンライン専用のファイルを照合で呼び戻さない"
    );
    h.fs.set_offline("/w/a.txt", false);
    fake.changed(id, "a.txt");
    h.pump_fs();
    settle(&mut h);
    assert!(
        pending_of(&h, a).is_some(),
        "ふつうのファイルは内容ハッシュで比べる"
    );
}

// ---------- AC-9: 縮退 ----------

// verifies: AC-FR-117-9
#[test]
fn 監視できないときは理由の種類ごとに_1_回知らせて_v0_2_の確かめ方に縮退する() {
    let fake = FakeWatcher::new().auto_failed(WatchError::Unsupported {
        fs: "9p".to_owned(),
    });
    let (mut h, fake) = watched_with(fake, &[("/w/a.txt", b"a\n"), ("/v/b.txt", b"b\n")]);
    h.clear_log();
    let a = open(&mut h, "/w/a.txt");
    assert_eq!(watch_messages(&h), vec!["info.watch.unavailable"]);
    assert!(h.frame_contains("9p"), "{}", h.frame_text());
    h.clear_log();
    open(&mut h, "/v/b.txt");
    assert!(watch_messages(&h).is_empty(), "同じ種類の理由は 1 回");
    // 監視しない: 知らせが来てもポーリングもせず、期限も無い (NFR-204)
    fake.changed(fake.id_of("/w").expect("要求"), "a.txt");
    h.pump_fs();
    assert_eq!(h.editor.next_deadline(), None);
    // フォーカス復帰の確かめ (v0.2) は効く
    h.cmd("buffer.prev");
    assert_eq!(h.doc_id(), a);
    h.fs.add_file("/w/a.txt", b"new\n");
    h.focus(false);
    h.focus(true);
    h.run_jobs();
    assert_eq!(text_of(&h, a), "new\n");
}

// verifies: AC-FR-117-9
#[test]
fn ネットワークの_fs_の文書は監視を頼まずに縮退を知らせる() {
    let (mut h, fake) = watched(&[("/n/a.txt", b"a\n")]);
    h.fs.set_network_dir("/n");
    h.clear_log();
    open(&mut h, "/n/a.txt");
    assert_eq!(
        fake.id_of("/n"),
        None,
        "FileWatcher に頼まない (io を止めない)"
    );
    assert_eq!(watch_messages(&h), vec!["info.watch.unavailable"]);
    assert!(
        h.frame_contains("ネットワークや仮想の FS"),
        "{}",
        h.frame_text()
    );
}

// verifies: AC-FR-117-9
#[test]
fn 違う種類の理由はそれぞれ_1_回知らせ_failed_の後の知らせは捨てる() {
    let (mut h, fake) = watched_with(
        FakeWatcher::new(),
        &[
            ("/w/a.txt", b"a\n"),
            ("/v/b.txt", b"b\n"),
            ("/u/c.txt", b"c\n"),
        ],
    );
    open(&mut h, "/w/a.txt");
    open(&mut h, "/v/b.txt");
    open(&mut h, "/u/c.txt");
    let (w, v, u) = (id_of(&fake, "/w"), id_of(&fake, "/v"), id_of(&fake, "/u"));
    h.clear_log();
    // Failed より先に届いた知らせ (OS の監視を一瞬張ってから失敗した) は捨てる
    fake.changed(w, "a.txt");
    fake.push(
        w,
        FsEventKind::Failed {
            err: WatchError::Limit,
        },
    );
    fake.push(
        v,
        FsEventKind::Failed {
            err: WatchError::Limit,
        },
    );
    fake.push(
        u,
        FsEventKind::Failed {
            err: WatchError::Denied,
        },
    );
    fake.changed(u, "c.txt");
    h.pump_fs();
    assert_eq!(
        watch_messages(&h),
        vec!["info.watch.unavailable", "info.watch.unavailable"],
        "上限と権限で 1 回ずつ"
    );
    assert_eq!(h.editor.next_deadline(), None);
}

// ---------- AC-10 (core の部分): 一定の間隔で起きない ----------

// verifies: AC-FR-117-10
#[test]
fn 監視している間も知らせが無ければ期限を持たない() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/v/b.txt", b"b\n")]);
    open(&mut h, "/w/a.txt");
    open(&mut h, "/v/b.txt");
    assert_eq!(fake.active().len(), 2);
    assert_eq!(h.editor.next_deadline(), None);
    h.advance_ms(60_000);
    assert_eq!(h.pending_jobs(), 0, "ポーリングしない");
}

// ---------- WATCH-1: 張り外し ----------

#[test]
fn 最後の文書を閉じたら監視を外し_閉じた文書の知らせは使わない() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    let id = id_of(&fake, "/w");
    fake.take_requests();
    // b を閉じる: a が残るので外さない。閉じた b の名前の知らせは捨てる
    h.cmd("buffer.close");
    h.run_jobs();
    assert_eq!(fake.take_requests(), Vec::<WatchRequest>::new());
    fake.changed(id, "b.txt");
    h.pump_fs();
    assert_eq!(h.editor.next_deadline(), None);
    // 待ち合わせの間に閉じた文書の照合もしない
    fake.changed(id, "a.txt");
    h.pump_fs();
    h.cmd("buffer.close");
    h.run_jobs();
    assert_eq!(fake.take_requests(), vec![WatchRequest::Unwatch(id)]);
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 0);
    assert_eq!(h.editor.next_deadline(), None);
}

#[test]
fn 失効した監視は保存のときに張り直し_古い要求の知らせは捨てる() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    let old = id_of(&fake, "/w");
    fake.take_requests();
    // ディレクトリが消えて作り直された (失効): 全文書を照合する
    fake.push(old, FsEventKind::Lost);
    h.pump_fs();
    settle(&mut h);
    assert_eq!(pending_of(&h, a), None, "ファイルは残っている");
    // 次の保存で張り直す (新しい要求を積んでから古い要求を外す)
    h.text("x");
    h.keys("ctrl+s");
    h.run_jobs();
    h.pump_fs();
    let reqs = fake.take_requests();
    let new = fake.id_of("/w").expect("張り直した");
    assert_ne!(new, old);
    assert_eq!(
        reqs,
        vec![
            WatchRequest::Watch {
                id: new,
                dir: "/w".into()
            },
            WatchRequest::Unwatch(old),
        ]
    );
    // 古い要求の知らせは捨てる
    h.fs.add_file("/w/a.txt", b"zzz\n");
    fake.changed(old, "a.txt");
    h.pump_fs();
    assert_eq!(h.editor.next_deadline(), None);
    // 新しい要求の知らせは効く
    fake.changed(new, "a.txt");
    h.pump_fs();
    settle(&mut h);
    assert!(pending_of(&h, a).is_some());
}

#[test]
fn 登録の結果より先に届いた知らせも受ける() {
    let (mut h, fake) = watched_with(
        FakeWatcher::new(),
        &[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")],
    );
    let a = open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    let id = id_of(&fake, "/w");
    h.fs.add_file("/w/a.txt", b"A\n");
    fake.changed(id, "a.txt");
    fake.push(id, FsEventKind::Started);
    h.pump_fs();
    settle(&mut h);
    assert!(pending_of(&h, a).is_some());
}

#[test]
fn 設定で監視を止めると外し_戻すと張り直す() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/v/b.txt", b"b\n")]);
    open(&mut h, "/w/a.txt");
    open(&mut h, "/v/b.txt");
    assert_eq!(fake.active().len(), 2);
    set_policy(&mut h, false, ExternalChangePolicy::AutoIfUnmodified);
    assert!(fake.active().is_empty(), "file.watch.enabled = false");
    set_policy(&mut h, true, ExternalChangePolicy::AutoIfUnmodified);
    assert_eq!(fake.active().len(), 2);
    set_policy(&mut h, true, ExternalChangePolicy::Never);
    assert!(
        fake.active().is_empty(),
        "reload_on_external_change = never"
    );
}

#[test]
fn 監視を止めた設定で起動すると監視を頼まない() {
    let fake = FakeWatcher::new().auto_started();
    let mut h = Harness::builder()
        .watcher(fake.clone())
        .config(Config {
            watch_enabled: false,
            ..Config::default()
        })
        .file("/w/a.txt", b"a\n")
        .build();
    open(&mut h, "/w/a.txt");
    assert_eq!(fake.requests(), Vec::<WatchRequest>::new());
}

#[test]
fn 名前を付けて保存でパスが変わったら新しいディレクトリを監視し_古いものを外す() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    h.fs.add_dir("/v");
    let a = open(&mut h, "/w/a.txt");
    let old = id_of(&fake, "/w");
    fake.take_requests();
    h.cmd_with(
        "file.save_as",
        eve_core::Args::new().with_str("path", "/v/b.txt"),
    );
    h.run_jobs();
    h.pump_fs();
    let new = id_of(&fake, "/v");
    assert_eq!(
        fake.take_requests(),
        vec![
            WatchRequest::Unwatch(old),
            WatchRequest::Watch {
                id: new,
                dir: "/v".into()
            },
        ]
    );
    write_outside(&mut h, &fake, "/v/b.txt", b"B\n");
    settle(&mut h);
    assert_eq!(text_of(&h, a), "B\n");
}

#[cfg(windows)]
#[test]
fn windows_では名前の大文字小文字を区別せず_短い名前はディレクトリを照合する() {
    let (mut h, fake) = watched(&[("/w/LongName.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/LongName.txt");
    open(&mut h, "/w/b.txt");
    let id = id_of(&fake, "/w");
    h.fs.add_file("/w/LongName.txt", b"1\n");
    fake.changed(id, "LONGNAME.TXT");
    h.pump_fs();
    settle(&mut h);
    assert!(pending_of(&h, a).is_some());
    h.fs.add_file("/w/LongName.txt", b"22\n");
    fake.changed(id, "LONGNA~1.TXT");
    h.pump_fs();
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(
        h.pending_jobs(),
        2,
        "短い名前はディレクトリの全文書を照合する"
    );
    h.run_jobs();
    match pending_of(&h, a) {
        Some(ExternalPending::Modified(s)) => assert_eq!(s.len, 3),
        other => panic!("{other:?}"),
    }
}

// ---------- PR #631 のレビューの直し ----------

fn request_token(h: &Harness) -> Option<eve_view::ClipboardToken> {
    h.effects().iter().rev().find_map(|e| match e {
        eve_view::Effect::RequestClipboard { token, .. } => Some(*token),
        _ => None,
    })
}

// verifies: AC-FR-117-7
#[test]
fn 監視で読み直している間に届いた貼り付けの応答は捨てずに貼り付ける() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    h.keys("ctrl+v");
    let token = request_token(&h).expect("RequestClipboard");
    write_outside(&mut h, &fake, "/w/a.txt", b"new\n");
    h.advance_ms(WATCH_SETTLE.0);
    // 照合だけを走らせる (監視の読み直しのジョブが積まれたまま)
    assert!(h.run_one_job());
    assert_eq!(h.pending_jobs(), 1, "監視の読み直しのジョブ");
    h.clipboard_content(token, Some("P"));
    h.run_jobs();
    assert_eq!(text_of(&h, a), "Pa\n", "貼り付けを読込で捨てない");
    assert!(pending_of(&h, a).is_some(), "読み直しを取り消して印にする");
}

// verifies: AC-FR-117-7
#[test]
fn 数_ms_ずれて届いた知らせの印は_1_件のメッセージにまとめる() {
    let (mut h, fake) = watched(&[
        ("/w/a.txt", b"a\n"),
        ("/w/b.txt", b"b\n"),
        ("/w/c.txt", b"c\n"),
    ]);
    let a = open(&mut h, "/w/a.txt");
    let b = open(&mut h, "/w/b.txt");
    open(&mut h, "/w/c.txt");
    h.clear_log();
    write_outside(&mut h, &fake, "/w/a.txt", b"A\n");
    h.advance_ms(3);
    write_outside(&mut h, &fake, "/w/b.txt", b"B\n");
    // a の待ち合わせの期限 (100 ms): a の照合の回が終わっても、b の期限が近いので知らせを保留する
    h.advance_ms(WATCH_SETTLE.0 - 3);
    h.run_jobs();
    assert!(pending_of(&h, a).is_some());
    assert_eq!(watch_messages(&h), Vec::<String>::new(), "保留する");
    // b の期限 (103 ms): 2 つの回の印を 1 件にまとめる
    h.advance_ms(3);
    h.run_jobs();
    assert!(pending_of(&h, b).is_some());
    assert_eq!(watch_messages(&h), vec!["info.external.changed_many"]);
    assert_eq!(h.editor.next_deadline(), None);
}

// verifies: AC-FR-117-7
#[test]
fn 保留した印は次の回が来なくても_100_ms_で知らせる() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    let b = open(&mut h, "/w/b.txt");
    h.clear_log();
    write_outside(&mut h, &fake, "/w/a.txt", b"A\n");
    h.advance_ms(3);
    // b の知らせ (期限は 103 ms) の後に b を閉じる: 回は来ない
    fake.changed(id_of(&fake, "/w"), "b.txt");
    h.pump_fs();
    h.advance_ms(WATCH_SETTLE.0 - 3);
    h.run_jobs();
    assert_eq!(watch_messages(&h), Vec::<String>::new());
    assert_eq!(h.doc_id(), b);
    h.cmd("buffer.close");
    h.run_jobs();
    h.advance_ms(WATCH_SETTLE.0);
    assert!(pending_of(&h, a).is_some());
    assert_eq!(watch_messages(&h), vec!["info.external.changed"]);
}

// verifies: AC-FR-117-7
#[test]
fn 書き続けるファイルは印の付いたときに_1_回だけ知らせ_印の_stamp_を新しくする() {
    let (mut h, fake) = watched(&[("/w/log.txt", b"0\n"), ("/w/b.txt", b"b\n")]);
    let log = open(&mut h, "/w/log.txt");
    open(&mut h, "/w/b.txt");
    h.clear_log();
    for i in 0..10u32 {
        let body = format!("{}\n", "x".repeat(i as usize + 1));
        write_outside(&mut h, &fake, "/w/log.txt", body.as_bytes());
        h.advance_ms(300);
        h.run_jobs();
    }
    assert_eq!(watch_messages(&h), vec!["info.external.changed"]);
    match pending_of(&h, log) {
        Some(ExternalPending::Modified(s)) => assert_eq!(s.len, 11, "最後の書込の stamp"),
        other => panic!("{other:?}"),
    }
    // 削除に替わったら知らせる
    h.fs.remove(&"/w/log.txt".into());
    fake.changed(id_of(&fake, "/w"), "log.txt");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(
        watch_messages(&h),
        vec!["info.external.changed", "info.external.deleted"]
    );
}

// verifies: AC-FR-117-7
#[test]
fn 表示中の読み直しの最短の間隔の内の変更も知らせは_1_回() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    write_outside(&mut h, &fake, "/w/a.txt", b"1\n");
    settle(&mut h);
    assert_eq!(text_of(&h, a), "1\n");
    h.clear_log();
    for body in [&b"22\n"[..], b"333\n", b"4444\n"] {
        h.advance_ms(500);
        write_outside(&mut h, &fake, "/w/a.txt", body);
        settle(&mut h);
    }
    assert_eq!(watch_messages(&h), vec!["info.external.changed"]);
}

// verifies: AC-FR-117-7
#[test]
fn 監視の知らせは表示中の警告を置き換えずに履歴に積む() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    h.editor
        .state_mut()
        .show_message(eve_view::MessageLevel::Warn, "msg.edit.read_only", &[]);
    h.clear_log();
    write_outside(&mut h, &fake, "/w/a.txt", b"A\n");
    settle(&mut h);
    assert!(pending_of(&h, a).is_some());
    assert_eq!(
        watch_messages(&h),
        Vec::<String>::new(),
        "警告を置き換えない"
    );
    let current = h
        .frame()
        .messages
        .current
        .map(|m| m.level)
        .expect("表示中のメッセージ");
    assert_eq!(current, eve_view::MessageLevel::Warn);
}

// verifies: AC-FR-117-7
#[test]
fn 実行中に監視を止めると印を外し_照合の途中の結果も使わない() {
    for policy in [
        (false, ExternalChangePolicy::AutoIfUnmodified),
        (true, ExternalChangePolicy::Never),
    ] {
        let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
        let a = open(&mut h, "/w/a.txt");
        let b = open(&mut h, "/w/b.txt");
        write_outside(&mut h, &fake, "/w/a.txt", b"A\n");
        settle(&mut h);
        assert!(pending_of(&h, a).is_some());
        // b の照合の途中で止める
        write_outside(&mut h, &fake, "/w/b.txt", b"B\n");
        h.advance_ms(WATCH_SETTLE.0);
        assert_eq!(h.pending_jobs(), 1);
        h.clear_log();
        set_policy(&mut h, policy.0, policy.1);
        assert_eq!(pending_of(&h, a), None, "{policy:?}");
        assert!(!h.frame_contains("[外部で変更]"));
        h.run_jobs();
        assert_eq!(pending_of(&h, b), None, "{policy:?}");
        assert_eq!(text_of(&h, b), "b\n", "{policy:?}");
        assert_eq!(watch_messages(&h), Vec::<String>::new(), "{policy:?}");
        // 切り替えても照合しない
        h.cmd("buffer.prev");
        h.advance_ms(1);
        assert_eq!(h.pending_jobs(), 0, "{policy:?}");
        assert_eq!(h.editor.next_deadline(), None, "{policy:?}");
    }
}

// verifies: AC-FR-117-8
#[test]
fn ジョブの結果なしに_ready_に戻った文書の延ばした照合も再開する() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    h.editor.state_mut().set_config(Config {
        huge_file_warn_bytes: 1_048_576,
        ..Config::default()
    });
    let a = open(&mut h, "/w/a.txt");
    // 利用者の再読込で、開く前の確認 (巨大ファイル) が出る: 文書は読込中のまま、ジョブは無い
    h.fs.add_file("/w/a.txt", &vec![b'x'; 1_200_000]);
    h.cmd("file.reload");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    fake.changed(id_of(&fake, "/w"), "a.txt");
    h.pump_fs();
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 0, "読込中は延ばす");
    // 確認を Esc で閉じる (ジョブの結果は届かない): 次の tick で延ばした照合を積む
    h.keys("esc");
    assert_eq!(h.overlay(), "");
    h.advance_ms(1);
    assert_eq!(h.pending_jobs(), 1);
    h.run_jobs();
    assert!(pending_of(&h, a).is_some());
    assert_eq!(h.overlay(), "");
}

// verifies: AC-FR-117-7
#[test]
fn 確認で無視を選んでも_確認の後の新しい変更の印は残す() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    h.text("x");
    write_outside(&mut h, &fake, "/w/a.txt", b"v1\n");
    settle(&mut h);
    h.focus(false);
    h.focus(true);
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    // 確認を出している間に、もう一度外部で変わる
    write_outside(&mut h, &fake, "/w/a.txt", b"v22\n");
    settle(&mut h);
    h.keys("enter");
    assert_eq!(h.overlay(), "");
    match pending_of(&h, a) {
        Some(ExternalPending::Modified(s)) => assert_eq!(s.len, 4, "新しい変更の印"),
        other => panic!("{other:?}"),
    }
}

// verifies: AC-FR-117-8
#[test]
fn 非同期の編集のジョブの間は照合を延ばし_利用者の操作を捨てない() {
    let big = "line of text\r\n".repeat(80_000);
    let (mut h, fake) = watched(&[("/w/big.txt", big.as_bytes())]);
    let d = open(&mut h, "/w/big.txt");
    h.cmd_with(
        "file.set_line_ending",
        eve_core::Args::new().with_str("eol", "lf"),
    );
    assert_eq!(h.pending_jobs(), 1, "改行の変換のジョブ");
    let mut changed = big.clone();
    changed.push_str("tail\r\n");
    write_outside(&mut h, &fake, "/w/big.txt", changed.as_bytes());
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 1, "変換の間は照合を延ばす");
    h.run_jobs();
    assert!(!text_of(&h, d).contains("\r\n"), "変換を捨てない");
    assert!(pending_of(&h, d).is_some(), "変換の後に照合して印を付ける");
}

// verifies: AC-FR-117-7
#[test]
fn 監視の照合の間のフォーカス復帰は照合の後に確かめ直す() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    h.editor.state_mut().set_config(Config {
        reload_on_external_change: ExternalChangePolicy::Ask,
        ..Config::default()
    });
    open(&mut h, "/w/a.txt");
    write_outside(&mut h, &fake, "/w/a.txt", b"new\n");
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 1, "監視の照合");
    h.focus(false);
    h.focus(true);
    assert_eq!(h.pending_jobs(), 1, "照合の間は延ばす");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "照合の後にフォーカス復帰の確認");
}

#[test]
fn フォルダの無い新規ファイルは監視を入れ直しても頼まない() {
    let (mut h, fake) = watched(&[]);
    h.clear_log();
    h.open("/nodir/new.txt");
    h.run_jobs();
    h.pump_fs();
    assert_eq!(fake.requests(), Vec::<WatchRequest>::new());
    set_policy(&mut h, false, ExternalChangePolicy::AutoIfUnmodified);
    set_policy(&mut h, true, ExternalChangePolicy::AutoIfUnmodified);
    h.pump_fs();
    assert_eq!(fake.requests(), Vec::<WatchRequest>::new());
    assert!(
        !h.messages().contains(&"info.watch.unavailable".to_owned()),
        "{:?}",
        h.messages()
    );
}

#[cfg(windows)]
#[test]
fn windows_でも_8_3_の形でない_チルダの名前ではディレクトリを照合しない() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    let id = id_of(&fake, "/w");
    for name in ["~$report.docx", "a.txt~", "~WRL0001.tmp"] {
        fake.changed(id, name);
        h.pump_fs();
        assert_eq!(h.editor.next_deadline(), None, "{name}");
    }
}

// ---------- PR #631 の再レビューの直し ----------

// verifies: AC-FR-117-10
#[test]
fn 照合の回が残る間は保留の期限を過去のまま返さない() {
    let (mut h, fake) = watched(&[
        ("/w/a.txt", b"a\n"),
        ("/w/b.txt", b"b\n"),
        ("/w/c.txt", b"c\n"),
    ]);
    let a = open(&mut h, "/w/a.txt");
    let b = open(&mut h, "/w/b.txt");
    open(&mut h, "/w/c.txt");
    h.clear_log();
    write_outside(&mut h, &fake, "/w/a.txt", b"A\n");
    h.advance_ms(50);
    write_outside(&mut h, &fake, "/w/b.txt", b"B\n");
    // a の照合 (100 ms) だけを走らせる: b の期限が近いので知らせを保留する
    h.advance_ms(50);
    assert!(h.run_one_job());
    assert!(pending_of(&h, a).is_some());
    // b の照合 (150 ms) は io の長いジョブの後ろで待つ
    h.advance_ms(50);
    assert_eq!(h.pending_jobs(), 1);
    h.advance_ms(300);
    let now = h.clock.now();
    assert!(
        h.editor.next_deadline().is_none_or(|d| d > now),
        "過去の期限で Host を空回りさせない: {:?} (今 {now:?})",
        h.editor.next_deadline()
    );
    h.run_jobs();
    assert!(pending_of(&h, b).is_some());
    assert_eq!(watch_messages(&h), vec!["info.external.changed_many"]);
}

// verifies: AC-FR-117-7
#[test]
fn オンライン専用のファイルでも無視を選ぶと印が外れ_同じ変更では付け直さない() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    h.text("x");
    h.fs.set_offline("/w/a.txt", true);
    write_outside(&mut h, &fake, "/w/a.txt", b"v1\n");
    h.fs.set_offline("/w/a.txt", true);
    settle(&mut h);
    assert!(pending_of(&h, a).is_some());
    h.focus(false);
    h.focus(true);
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    h.keys("enter");
    assert_eq!(pending_of(&h, a), None, "内容ハッシュの無い印も外す");
    fake.changed(id_of(&fake, "/w"), "a.txt");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(pending_of(&h, a), None, "無視した変更は付け直さない");
}

// verifies: AC-FR-117-7
#[test]
fn 監視の読み直しの知らせも表示中の警告を置き換えない() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    h.editor
        .state_mut()
        .show_message(eve_view::MessageLevel::Warn, "msg.edit.read_only", &[]);
    write_outside(&mut h, &fake, "/w/a.txt", b"new\n");
    settle(&mut h);
    assert_eq!(text_of(&h, a), "new\n");
    let current = h.frame().messages.current.map(|m| m.level);
    assert_eq!(current, Some(eve_view::MessageLevel::Warn));
}

// verifies: AC-FR-117-7
#[test]
fn 読み直しの最中に監視を止めると読込を取り消し_打鍵を捨てない() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    let a = open(&mut h, "/w/a.txt");
    write_outside(&mut h, &fake, "/w/a.txt", b"new\n");
    h.advance_ms(WATCH_SETTLE.0);
    assert!(h.run_one_job());
    assert_eq!(h.pending_jobs(), 1, "監視の読み直しのジョブ");
    set_policy(&mut h, false, ExternalChangePolicy::AutoIfUnmodified);
    h.text("z");
    h.run_jobs();
    assert_eq!(text_of(&h, a), "za\n", "止めた後に読み直しを当てない");
    assert_eq!(pending_of(&h, a), None);
}

// verifies: AC-FR-117-8
#[test]
fn 打ち切りの照合の結果はその後に知らせが届いても捨てない() {
    let (mut h, fake) = watched(&[("/w/log.txt", b"0\n"), ("/w/b.txt", b"b\n")]);
    let log = open(&mut h, "/w/log.txt");
    open(&mut h, "/w/b.txt");
    let id = id_of(&fake, "/w");
    h.clear_log();
    for i in 0..20u32 {
        h.fs.add_file("/w/log.txt", format!("{i}\n").as_bytes());
        fake.changed(id, "log.txt");
        h.pump_fs();
        h.advance_ms(50);
        if h.pending_jobs() > 0 {
            break;
        }
    }
    // 1 s の打ち切りの照合が積まれた。結果が届く前に次の書込の知らせが届く
    assert_eq!(h.pending_jobs(), 1);
    h.advance_ms(10);
    h.fs.add_file("/w/log.txt", b"more\n");
    fake.changed(id, "log.txt");
    h.pump_fs();
    h.run_jobs();
    assert!(
        pending_of(&h, log).is_some(),
        "最初の知らせを最長 1 s で出す"
    );
    assert_eq!(watch_messages(&h), vec!["info.external.changed"]);
}

// verifies: AC-FR-117-7
#[test]
fn 監視を入れ直したら止めていた間の変更を照合する() {
    let (mut h, _fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    set_policy(&mut h, false, ExternalChangePolicy::AutoIfUnmodified);
    h.fs.add_file("/w/a.txt", b"changed while off\n");
    h.clear_log();
    set_policy(&mut h, true, ExternalChangePolicy::AutoIfUnmodified);
    h.pump_fs();
    h.run_jobs();
    assert!(pending_of(&h, a).is_some());
    assert_eq!(watch_messages(&h), vec!["info.external.changed"]);
}

// verifies: AC-FR-117-7
#[test]
fn 保留の間に印が外れた文書は保留の知らせで数えない() {
    let (mut h, fake) = watched(&[
        ("/w/a.txt", b"a\n"),
        ("/w/b.txt", b"b\n"),
        ("/w/c.txt", b"c\n"),
    ]);
    let a = open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    open(&mut h, "/w/c.txt");
    h.clear_log();
    write_outside(&mut h, &fake, "/w/a.txt", b"A\n");
    h.advance_ms(3);
    write_outside(&mut h, &fake, "/w/b.txt", b"B\n");
    h.advance_ms(WATCH_SETTLE.0 - 3);
    h.run_jobs();
    assert_eq!(watch_messages(&h), Vec::<String>::new(), "保留する");
    // 保留の間に a の印が外れた (保存・読み直しなど)
    h.editor
        .state_mut()
        .document_mut(a)
        .expect("a")
        .external_pending = None;
    h.advance_ms(3);
    h.run_jobs();
    assert_eq!(
        watch_messages(&h),
        vec!["info.external.changed"],
        "今の印で数える"
    );
}

// verifies: AC-FR-117-7
#[test]
fn 延ばしたフォーカス復帰の確かめはフォーカスを失ったら捨てる() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n")]);
    h.editor.state_mut().set_config(Config {
        reload_on_external_change: ExternalChangePolicy::Ask,
        ..Config::default()
    });
    open(&mut h, "/w/a.txt");
    write_outside(&mut h, &fake, "/w/a.txt", b"new\n");
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 1, "監視の照合");
    h.focus(false);
    h.focus(true);
    h.focus(false);
    h.run_jobs();
    assert_eq!(h.overlay(), "", "フォーカスの無い間に確認を出さない");
}

// ---------- 自己点検で足した試験 ----------

// verifies: AC-FR-117-9
#[test]
fn 監視を入れ直しても監視できないフォルダの文書は照合しない() {
    let (mut h, fake) = watched(&[("/n/a.txt", b"a\n")]);
    h.fs.set_network_dir("/n");
    open(&mut h, "/n/a.txt");
    set_policy(&mut h, false, ExternalChangePolicy::AutoIfUnmodified);
    set_policy(&mut h, true, ExternalChangePolicy::AutoIfUnmodified);
    h.pump_fs();
    assert_eq!(h.pending_jobs(), 0, "監視していない文書と同じく照合しない");
    assert_eq!(fake.id_of("/n"), None);
}

// verifies: AC-FR-117-7
#[test]
fn 照合の途中で監視を止めて入れ直しても知らせは_1_回() {
    let (mut h, fake) = watched(&[("/w/a.txt", b"a\n"), ("/w/b.txt", b"b\n")]);
    let a = open(&mut h, "/w/a.txt");
    open(&mut h, "/w/b.txt");
    write_outside(&mut h, &fake, "/w/a.txt", b"A\n");
    h.advance_ms(WATCH_SETTLE.0);
    assert_eq!(h.pending_jobs(), 1, "a の照合");
    h.clear_log();
    set_policy(&mut h, false, ExternalChangePolicy::AutoIfUnmodified);
    set_policy(&mut h, true, ExternalChangePolicy::AutoIfUnmodified);
    h.pump_fs();
    h.run_jobs();
    h.advance_ms(WATCH_SETTLE.0);
    h.run_jobs();
    assert!(pending_of(&h, a).is_some());
    assert_eq!(watch_messages(&h), vec!["info.external.changed"]);
    assert_eq!(h.editor.next_deadline(), None);
}

// ---------- 最後の確認の直し ----------

/// 行番号の欄の表記 (フォーカスのペインのステータスバー)。
fn status_line_no(h: &mut Harness) -> String {
    let t = h.frame_text_with(eve_testkit::AsciiOptions {
        show_chrome: true,
        ..eve_testkit::AsciiOptions::default()
    });
    let at = t.find("行 ").expect("ステータスバーの位置");
    t[at..].split(',').next().unwrap_or_default().to_owned()
}

// verifies: AC-FR-117-7
#[test]
fn 取り消した監視の読み直しの後に横断検索のジャンプを残さない() {
    let fake = FakeWatcher::new().auto_started();
    let mut h = Harness::builder()
        .watcher(fake.clone())
        .defer_start()
        .dir("/w/proj/.git")
        .file("/w/proj/a.txt", b"one\ntwo hit\nthree\nfour\n")
        .file("/w/proj/b.txt", b"b\n")
        .build();
    h.editor
        .open_initial_files(&[eve_core::InitialFile::path("/w/proj/a.txt")], false, None);
    h.started();
    h.run_jobs();
    h.pump_fs();
    // 左に a (表示中・未編集)、右に b (フォーカス)
    h.cmd("pane.split_right");
    h.cmd_with(
        "file.open",
        eve_core::Args::new().with_str("path", "/w/proj/b.txt"),
    );
    h.run_jobs();
    h.pump_fs();
    h.keys("ctrl+shift+f");
    h.text("hit");
    h.run_jobs();
    h.keys("esc");
    // a を外で書き換え、照合だけを走らせる (監視の読み直しのジョブが積まれたまま)
    h.fs.add_file("/w/proj/a.txt", b"one\ntwo hit\nthree\nfour\n\n");
    fake.changed(id_of(&fake, "/w/proj"), "a.txt");
    h.pump_fs();
    h.advance_ms(WATCH_SETTLE.0);
    assert!(h.run_one_job());
    assert_eq!(h.pending_jobs(), 1, "監視の読み直しのジョブ");
    // a の一致を選ぶ (読込中なので移るのを保留する)
    h.keys("ctrl+shift+f enter");
    // 打鍵で読み直しを取り消す: 旧本文に戻ったので保留のジャンプが済み、↓ はその後で動く
    h.keys("down");
    h.run_jobs();
    assert_eq!(status_line_no(&mut h), "行 3", "ジャンプの後の ↓");
    // 5 s の後にもう一度書き換えて読み直しが完了しても、カーソルは一致の位置へ飛ばない
    h.advance_ms(WATCH_RELOAD_MIN_INTERVAL.0);
    h.fs.add_file("/w/proj/a.txt", b"one\ntwo hit\nthree\nfour\n\n\n");
    fake.changed(id_of(&fake, "/w/proj"), "a.txt");
    h.pump_fs();
    settle(&mut h);
    assert_eq!(
        h.doc_text(),
        "one\ntwo hit\nthree\nfour\n\n\n",
        "読み直した"
    );
    assert_eq!(
        status_line_no(&mut h),
        "行 3",
        "保留のジャンプが残っていない"
    );
}
