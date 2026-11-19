//! 標準入力の公開 (docs/03 §9.2.2、01 FR-120 AC-1〜AC-5、OQ-219 の 4〜10。v0.4 の M25 の (9)): 起動の経路
//! (`InitialTarget::Stdin`)、変更ありの扱い、終わり方ごとのメッセージ、`Esc` と閉じる・終了の確認、緊急退避の
//! `sensitive` とスワップ、名前を付けて保存で `LoadIncomplete` を外すこと、ステータスバーの読んだ量。偽の
//! `StdinSource` (`eve_testkit::FakeStdin`) で確かめる。

use std::sync::Arc;

use eve_core::{
    CancelToken, CommandResult, Encoding, InitialFile, InitialTarget, JobError, JobKind, JobOutput,
    JobPorts, JobRequest, JobResult, LoadState, OpenOptions, ReadOnlyReason, StdinEnd, StdinSource,
    run_job,
};
use eve_testkit::{FakeStdin, FixedPaths, Harness, ManualClock, MemFs, StdinStep};
use eve_view::{DocumentId, Effect, JobId, Revision};

/// `harness` の標準入力の読込のジョブ (`defer_start` で最初に振られる番号)。
const STDIN_JOB: JobId = JobId(1);

/// 標準入力の文書を表示した Harness (読込のジョブは積んだだけ)。
fn harness_with(fake: &FakeStdin, opts: OpenOptions) -> (Harness, DocumentId) {
    let mut h = Harness::builder()
        .defer_start()
        .stdin(Arc::new(fake.clone()))
        .build();
    let doc = h.editor.state_mut().open_stdin(opts);
    h.started();
    (h, doc)
}

fn harness(fake: &FakeStdin) -> (Harness, DocumentId) {
    harness_with(fake, OpenOptions::default())
}

/// 起動 (`open_initial_files`) で開く Harness。
fn started(fake: &FakeStdin, files: &[InitialFile], read_only: bool) -> Harness {
    let mut h = Harness::builder()
        .defer_start()
        .stdin(Arc::new(fake.clone()))
        .file("/w/a.txt", b"aaa\n")
        .build();
    h.editor.open_initial_files(files, read_only, None);
    h.started();
    h
}

fn stdin_target(terminal: bool) -> InitialFile {
    InitialFile {
        target: InitialTarget::Stdin { terminal },
        line: None,
        col: None,
    }
}

/// 別に走らせた読込の結果 (`steps` を読んだ後の `Hang` で取り消す)。キューのジョブを走らせずに、読んでから止めた・
/// 失敗した終わり方を作る。
fn other_run(steps: Vec<StdinStep>, cancel_on_hang: bool) -> Vec<JobOutput> {
    other_run_as(STDIN_JOB, steps, cancel_on_hang)
}

/// `other_run` のジョブの番号を指定する版。
fn other_run_as(job: JobId, steps: Vec<StdinStep>, cancel_on_hang: bool) -> Vec<JobOutput> {
    let fake = FakeStdin::new(steps);
    let cancel = CancelToken::new();
    if cancel_on_hang {
        let (c, f) = (cancel.clone(), fake.clone());
        fake.on_hang(move || {
            c.cancel();
            f.wake();
        });
    }
    let clock = ManualClock::new();
    let p = JobPorts {
        stdin: Some(Arc::new(fake) as Arc<dyn StdinSource>),
        fs: Arc::new(MemFs::new(Arc::new(clock.clone()))),
        clock: Arc::new(clock),
        paths: Arc::new(FixedPaths::default()),
    };
    let req = JobRequest {
        id: job,
        doc: None,
        base: Revision(0),
        kind: JobKind::OpenStdin {
            opts: OpenOptions {
                read_only_requested: true,
                ..OpenOptions::default()
            },
        },
    };
    let mut out = Vec::new();
    run_job(&req, &p, &cancel, &mut |r| out.push(r));
    out
}

/// 結果を Editor に渡して、イベントと Effect を集める。
fn deliver(h: &mut Harness, out: Vec<JobOutput>) {
    for o in out {
        h.editor.on_job_result(o);
    }
    h.advance_ms(0);
}

/// 読込中に `bytes` まで読んだ進捗を届ける (キューのジョブは走らせない)。
fn progress(h: &mut Harness, bytes: u64) {
    deliver(
        h,
        vec![JobOutput {
            job: STDIN_JOB,
            body: JobResult::Progress { bytes, total: 0 },
        }],
    );
}

fn doc_of(h: &Harness, doc: DocumentId) -> &eve_core::Document {
    h.editor.state().document(doc).expect("文書")
}

fn ct(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.as_str()).collect()
}

/// 確認ダイアログの本文 (タイトル・メッセージ・選択肢を改行でつなげたもの)。
fn dialog_text(h: &mut Harness) -> String {
    let f = h.frame();
    let Some(eve_view::OverlayModel::Confirm {
        title,
        message,
        choices,
        ..
    }) = f.overlays.first()
    else {
        panic!("確認ダイアログではない");
    };
    let mut out = vec![ct(title), ct(message)];
    out.extend(choices.iter().map(|c| ct(&c.label)));
    out.join(
        "
",
    )
}

/// 入力プロンプトの見出し。
fn prompt_title(h: &mut Harness) -> String {
    let f = h.frame();
    let Some(eve_view::OverlayModel::Prompt { title, .. }) = f.overlays.first() else {
        panic!("入力プロンプトではない");
    };
    ct(title)
}

fn confirm_name(h: &Harness) -> Option<String> {
    h.editor.state().confirm_name().map(str::to_owned)
}

// ---- 起動の経路 (02 §5.4、03 §9.2.2 の「起動の経路」「標準入力が端末」) ----

// verifies: AC-FR-120-2
#[test]
fn 標準入力が端末なら読まずに空の無題の文書を開いて知らせる() {
    let fake = FakeStdin::chunks(b"never read", 4);
    let mut h = started(&fake, &[stdin_target(true)], false);
    h.run_jobs();
    assert!(!fake.taken(), "端末を読まない");
    let doc = h.doc_id();
    let d = doc_of(&h, doc);
    assert!(d.path.is_none());
    assert_eq!(d.load_state, LoadState::Ready);
    assert_eq!(h.doc_text(), "");
    assert!(d.stdin.is_none());
    assert!(!h.is_dirty());
    assert!(h.messages().contains(&"info.stdin.terminal".to_owned()));
}

// verifies: AC-FR-120-1
#[test]
fn 起動の標準入力は書いた順に足し_最初のものを表示する() {
    let fake = FakeStdin::chunks(b"from stdin\n", 3);
    let mut h = started(
        &fake,
        &[InitialFile::path("/w/a.txt"), stdin_target(false)],
        false,
    );
    h.run_jobs();
    assert_eq!(h.doc_text(), "aaa\n", "最初のファイルを表示する");
    h.cmd("buffer.next");
    assert_eq!(h.doc_text(), "from stdin\n");
    assert!(h.doc_path().is_none(), "無題の文書");
    assert!(h.is_dirty(), "読み込んだ内容は変更あり");
}

#[test]
fn 標準入力が先頭なら表示し_プラス_行は読み終えた時点で移る() {
    let fake = FakeStdin::chunks(b"1\n2\n3\n4\n", 2);
    let mut h = started(
        &fake,
        &[InitialFile {
            line: Some(3),
            ..stdin_target(false)
        }],
        false,
    );
    {
        let d = doc_of(&h, h.doc_id());
        assert_eq!(d.load_state, LoadState::Loading);
    }
    h.run_jobs();
    assert_eq!(h.doc_text(), "1\n2\n3\n4\n");
    assert!(h.frame_contains("行 3, 桁 1"), "{}", h.frame_text());
}

#[test]
fn ステータスバーは読込中の量を出す() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, _) = harness(&fake);
    progress(&mut h, 12 * 1024 * 1024 + 300 * 1024);
    assert!(h.frame_contains("[読込中 12.3 MiB]"), "{}", h.frame_text());
}

// ---- 変更あり (03 §9.2.2 の「変更あり」、OQ-219 の 4、01 FR-120 AC-3) ----

// verifies: AC-FR-120-3
#[test]
fn 読み込んだ内容は変更ありで_undo_で戻っても変更あり() {
    let fake = FakeStdin::chunks(b"log\n", 2);
    let (mut h, doc) = harness(&fake);
    h.run_jobs();
    assert!(h.is_dirty());
    h.text("x");
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), "log\n");
    assert!(doc_of(&h, doc).is_dirty(), "保存点の無い履歴");
    // 終了で FR-107 の確認が出る
    h.cmd("app.quit");
    assert_eq!(confirm_name(&h).as_deref(), Some("unsaved"));
    assert_eq!(h.quit_code(), None);
}

// verifies: AC-FR-120-3
#[test]
fn readonly_は変更なしで_解除すると変更ありになる() {
    let fake = FakeStdin::chunks(b"log\n", 2);
    let (mut h, doc) = harness_with(
        &fake,
        OpenOptions {
            user_read_only: true,
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    assert!(!h.is_dirty(), "--readonly は変更なし");
    assert!(
        h.editor.emergency_snapshot(doc).is_none(),
        "変更なしは退避しない"
    );
    h.cmd("app.quit");
    assert_eq!(h.quit_code(), Some(0), "確認なしで終わる");
    let (mut h, doc) = harness_with(
        &FakeStdin::chunks(b"log\n", 2),
        OpenOptions {
            user_read_only: true,
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    h.cmd("file.toggle_read_only");
    assert!(doc_of(&h, doc).read_only.is_empty());
    assert!(h.is_dirty(), "解除した時点で変更あり");
    assert!(h.editor.emergency_snapshot(doc).is_some());
}

#[test]
fn 何も読まなかった文書は理由の無い空の文書で_開くと置き換わる() {
    // 03 §9.2.2: `NUL` / `/dev/null` は読んで空 (EOF)。理由の無い空の文書で、変更なし。何も入力していない無題の
    // 文書と同じく、`file.open` で置き換わる (失うものが無い。03 §9.8 の `file.open` の行の「例外」)
    let fake = FakeStdin::new([]);
    let (mut h, doc) = harness(&fake);
    h.run_jobs();
    assert!(!h.is_dirty());
    assert!(doc_of(&h, doc).read_only.is_empty());
    assert!(
        !h.messages().iter().any(|m| m.contains("stdin")),
        "{:?}",
        h.messages()
    );
    h.fs.add_file("/w/b.txt", b"b\n");
    h.open("/w/b.txt");
    h.run_jobs();
    assert_eq!(h.doc_text(), "b\n");
    assert!(
        h.editor.state().document(doc).is_none(),
        "空の標準入力の文書は置き換わる"
    );
}

// ---- 終わり方ごとのメッセージ (03 §9.2.2、04 §15.2) ----

#[test]
fn 上限で読むのをやめたら途中までを読み取り専用で開き知らせる() {
    let fake = FakeStdin::chunks(&[b'a'; 300], 100);
    let (mut h, doc) = harness_with(
        &fake,
        OpenOptions {
            huge_file_warn_bytes: Some(200),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    let d = doc_of(&h, doc);
    assert!(d.read_only.contains(ReadOnlyReason::LoadIncomplete));
    assert!(d.is_dirty(), "途中までも変更あり");
    assert!(h.messages().contains(&"msg.stdin.truncated".to_owned()));
}

#[test]
fn 途中で読めなくなったら読めた分を開いて知らせる() {
    let fake = FakeStdin::new([
        StdinStep::Data(b"part\n".to_vec()),
        StdinStep::Error(eve_core::FsErrorKind::Other),
    ]);
    let (mut h, doc) = harness(&fake);
    h.run_jobs();
    assert_eq!(h.doc_text(), "part\n");
    assert!(
        doc_of(&h, doc)
            .read_only
            .contains(ReadOnlyReason::LoadIncomplete)
    );
    assert!(h.messages().contains(&"msg.stdin.read_failed".to_owned()));
}

#[test]
fn 最初から読めないときは理由の無い空の文書と読取の失敗() {
    // Host が `StdinSource` を作れなかった (移し替えの後にスレッドを作れなかったときを含む): Port が無い
    let mut h = Harness::builder().defer_start().build();
    let doc = h.editor.state_mut().open_stdin(OpenOptions::default());
    h.started();
    h.run_jobs();
    let d = doc_of(&h, doc);
    assert_eq!(d.load_state, LoadState::Ready);
    assert!(d.read_only.is_empty());
    assert!(!d.is_dirty());
    // 0 バイトのまま読めなかった: 「読み取り専用で開いた」と言わない別のキー
    assert!(h.messages().contains(&"msg.stdin.unreadable".to_owned()));
    assert!(!h.messages().contains(&"msg.stdin.read_failed".to_owned()));
}

#[test]
fn 確保の失敗は途中までを開いて知らせる() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    let text = eve_text::Text::from("abc");
    let mut detected = eve_core::ReadOnlyReasons::default();
    detected.insert(ReadOnlyReason::LoadIncomplete);
    let format = eve_core::TextFormat::new_document();
    deliver(
        &mut h,
        vec![JobOutput {
            job: STDIN_JOB,
            body: JobResult::StdinDone {
                text,
                format,
                detected,
                preserved: eve_core::PreservedMap::default(),
                bytes: 3,
                end: StdinEnd::OutOfMemory { requested: 1 << 30 },
            },
        }],
    );
    assert!(
        doc_of(&h, doc)
            .read_only
            .contains(ReadOnlyReason::LoadIncomplete)
    );
    assert!(h.messages().contains(&"msg.stdin.out_of_memory".to_owned()));
}

#[test]
fn 書く手段の無い内容は開いた時点で知らせる() {
    // 不正バイトを含む ISO-2022-JP (どの文字コードでも書けない。標準入力は開き直せない)
    let mut bytes = b"\x1b$B$\"\x1b(B ".to_vec();
    bytes.extend_from_slice(b"\x1b$B\xff\xfe\x1b(B\n");
    let fake = FakeStdin::chunks(&bytes, 4);
    let (mut h, _) = harness_with(
        &fake,
        OpenOptions {
            forced: Some(Encoding::Iso2022Jp),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    assert!(
        h.messages().contains(&"msg.stdin.unsaveable".to_owned()),
        "{:?}",
        h.messages()
    );
}

#[test]
fn パニックで終わった読込は知らせて文書を閉じる() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    deliver(
        &mut h,
        vec![JobOutput {
            job: STDIN_JOB,
            body: JobResult::Failed {
                err: JobError::Panic,
            },
        }],
    );
    assert!(h.editor.state().document(doc).is_none());
    assert!(h.messages().contains(&"msg.stdin.failed".to_owned()));
    assert!(!h.messages().contains(&"msg.open.failed".to_owned()));
}

// ---- `Esc` の確認 (03 §9.2.2 の「取消し (`Esc`)」、OQ-219 の 10 (i)、01 FR-120 AC-4) ----

// verifies: AC-FR-120-4
#[test]
fn 表示中の_esc_は確認を出し_ここまでで開くで読むのをやめる() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    progress(&mut h, 8);
    h.keys("esc");
    assert_eq!(confirm_name(&h).as_deref(), Some("stdin_stop"));
    let text = dialog_text(&mut h);
    assert!(
        text.contains("読み続ける") && text.contains("ここまでで開く"),
        "{text}"
    );
    assert!(
        !text.contains("キャンセル"),
        "読み続けると同じ意味のキャンセルは並べない: {text}"
    );
    // 読み続ける (Esc)
    h.keys("esc");
    assert_eq!(confirm_name(&h), None);
    assert_eq!(fake.wakes(), 0, "読み続ける");
    // もう一度 Esc → ここまでで開く
    h.keys("esc");
    h.keys("o");
    assert_eq!(fake.wakes(), 1, "取消しで待ちを起こす");
    deliver(
        &mut h,
        other_run(
            vec![StdinStep::Data(b"partial\n".to_vec()), StdinStep::Hang],
            true,
        ),
    );
    let d = doc_of(&h, doc);
    assert_eq!(d.load_state, LoadState::Ready);
    assert!(d.read_only.contains(ReadOnlyReason::LoadIncomplete));
    assert!(d.is_dirty());
    assert!(h.messages().contains(&"info.stdin.stopped".to_owned()));
}

#[test]
fn ほかの文書を表示しているときの_esc_は標準入力に触れない() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let mut h = Harness::builder()
        .defer_start()
        .stdin(Arc::new(fake.clone()))
        .file(
            "/w/a.txt", b"aaa
",
        )
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/a.txt")], false, None);
    h.started();
    h.run_jobs();
    // a.txt を表示したまま、標準入力の文書を足す (表示しない)
    let doc = h.editor.state_mut().open_stdin(OpenOptions::default());
    assert_ne!(h.doc_id(), doc);
    assert_eq!(
        h.doc_text(),
        "aaa
"
    );
    h.keys("esc");
    assert_eq!(confirm_name(&h), None);
    assert_eq!(fake.wakes(), 0);
    // 表示していない標準入力の読込はステータスバーに出さない
    assert!(!h.frame_contains("読込中"), "{}", h.frame_text());
}

#[test]
fn 確認の間に読み終えたら確認を閉じる() {
    let fake = FakeStdin::chunks(b"done\n", 2);
    let (mut h, _) = harness(&fake);
    h.keys("esc");
    assert_eq!(confirm_name(&h).as_deref(), Some("stdin_stop"));
    h.run_jobs();
    assert_eq!(confirm_name(&h), None);
    assert_eq!(h.doc_text(), "done\n");
    assert!(
        !h.editor
            .state()
            .document(h.doc_id())
            .unwrap()
            .read_only
            .contains(ReadOnlyReason::LoadIncomplete)
    );
}

// ---- 閉じる・終了の確認 (03 §9.2.2 の「閉じる・終了」、§9.8、OQ-219 の 10 (ii)) ----

#[test]
fn 読込中の文書を閉じると確認し_ここまでで開くと_fr_107_の確認に進む() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    progress(&mut h, 8);
    h.cmd("buffer.close");
    assert_eq!(confirm_name(&h).as_deref(), Some("stdin_loading"));
    h.answer(0); // ここまでで開く
    assert_eq!(fake.wakes(), 1);
    deliver(
        &mut h,
        other_run(
            vec![StdinStep::Data(b"partial\n".to_vec()), StdinStep::Hang],
            true,
        ),
    );
    assert_eq!(
        confirm_name(&h).as_deref(),
        Some("unsaved_close"),
        "開いた文書は変更ありなので FR-107 の確認"
    );
    let text = dialog_text(&mut h);
    assert!(text.contains("(途中まで 8 B)"), "{text}");
    assert!(h.editor.state().document(doc).is_some());
}

#[test]
fn 読込中の文書の閉じるの破棄は捨て_キャンセルは読み続ける() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    progress(&mut h, 8);
    h.cmd("buffer.close");
    h.keys("esc");
    assert!(h.editor.state().document(doc).is_some(), "キャンセル");
    assert_eq!(fake.wakes(), 0);
    h.cmd("buffer.close");
    h.answer(1); // 破棄
    assert!(h.editor.state().document(doc).is_none());
}

#[test]
fn 一バイトも読んでいなければ確認せずに閉じる() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    h.cmd("buffer.close");
    assert_eq!(confirm_name(&h), None);
    assert!(h.editor.state().document(doc).is_none());
}

#[test]
fn 終了は読込中の文書を連鎖で尋ね_ここまでで開くと_fr_107_に進む() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, _) = harness(&fake);
    progress(&mut h, 8);
    h.cmd("app.quit");
    assert_eq!(confirm_name(&h).as_deref(), Some("stdin_loading"));
    h.answer(0);
    deliver(
        &mut h,
        other_run(
            vec![StdinStep::Data(b"partial\n".to_vec()), StdinStep::Hang],
            true,
        ),
    );
    assert_eq!(confirm_name(&h).as_deref(), Some("unsaved"));
    h.answer(1); // 破棄
    assert_eq!(h.quit_code(), Some(0));
}

#[test]
fn 終了の連鎖で読込中の文書を破棄すると終わる() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, _) = harness(&fake);
    progress(&mut h, 8);
    h.cmd("app.quit");
    h.answer(1);
    assert!(
        h.effects()
            .iter()
            .any(|e| matches!(e, Effect::Quit { code: 0, .. }))
    );
}

#[test]
fn 終了の連鎖のキャンセルは連鎖を止めて読み続ける() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, _) = harness(&fake);
    progress(&mut h, 8);
    h.cmd("app.quit");
    h.keys("esc");
    assert_eq!(h.quit_code(), None);
    assert_eq!(fake.wakes(), 0);
}

// ---- 緊急退避とスワップ (03 §9.2.2 の「スワップと緊急退避」、OQ-219 の 5) ----

// verifies: AC-FR-120-3
#[test]
fn 読み込んだ時点から退避の対象で_sensitive_を立てる() {
    let fake = FakeStdin::chunks(b"secret\n", 3);
    let (mut h, doc) = harness(&fake);
    assert!(h.editor.emergency_snapshot(doc).is_none(), "読込中は対象外");
    h.run_jobs();
    let snap = h
        .editor
        .emergency_snapshot(doc)
        .expect("読み込んだ時点から");
    assert!(snap.sensitive);
    assert_eq!(snap.to_vec(), b"secret\n");
    // 名前を付けて保存した後も印は残す (標準入力から読んだ内容)
    h.cmd_with(
        "file.save_as",
        eve_core::Args::new().with_str("path", "/w/out.txt"),
    );
    h.run_jobs();
    h.text("x");
    assert!(h.editor.emergency_snapshot(doc).expect("dirty").sensitive);
}

// verifies: AC-FR-120-3
#[test]
fn スワップは最初の編集から書く() {
    let fake = FakeStdin::chunks(b"log\n", 2);
    let (mut h, doc) = harness(&fake);
    h.run_jobs();
    h.advance_ms(60_000);
    h.run_jobs();
    assert_eq!(h.fs.swap_writes(), 0, "読み込んだだけは書かない");
    h.text("x");
    h.advance_ms(0);
    h.advance_ms(2000);
    h.run_jobs();
    assert_eq!(h.fs.swap_writes(), 1);
    assert_eq!(h.fs.swap_of(doc).unwrap(), b"xlog\n");
}

// ---- `LoadIncomplete` の外し方 (03 §9.2.2、OQ-219 の 6) ----

// verifies: AC-FR-120-4
#[test]
fn 名前を付けて保存が成功したら途中までの印を外す() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    h.keys("esc");
    h.keys("o");
    deliver(
        &mut h,
        other_run(
            vec![StdinStep::Data(b"partial\n".to_vec()), StdinStep::Hang],
            true,
        ),
    );
    assert!(
        doc_of(&h, doc)
            .read_only
            .contains(ReadOnlyReason::LoadIncomplete)
    );
    // 名前を付けて保存の入力の案内に「途中まで ({size})」
    h.cmd("file.save_as");
    let title = prompt_title(&mut h);
    assert!(title.contains("途中まで (8 B)"), "{title}");
    h.keys("esc");
    h.cmd_with(
        "file.save_as",
        eve_core::Args::new().with_str("path", "/w/part.txt"),
    );
    h.run_jobs();
    assert_eq!(h.fs_read("/w/part.txt").unwrap(), b"partial\n");
    let d = doc_of(&h, doc);
    assert!(!d.read_only.contains(ReadOnlyReason::LoadIncomplete));
    assert!(d.read_only.is_empty(), "編集できる");
    assert!(!d.is_dirty());
    assert!(h.messages().contains(&"info.save.partial_saved".to_owned()));
    h.text("y");
    assert_eq!(h.doc_text(), "ypartial\n");
}

// verifies: FR-120
#[test]
fn 区分_b_を含む_shift_jis_を同じ文字コードで保存すると入力とバイトが一致する() {
    // 「日本語」+ NEC 選定 IBM 拡張の 0xED40 (読むと U+7E8A、そのまま書くと 0xFA5C になる文字)。保持方式の記録で
    // 元のバイトに戻す (03 §9.2.2 の「結果」、OQ-219 の 7)
    let line: &[u8] = b"\x93\xfa\x96\x7b\x8c\xea\xed\x40\r\n";
    let bytes = line.repeat(40);
    let fake = FakeStdin::chunks(&bytes, 1);
    let (mut h, doc) = harness_with(
        &fake,
        OpenOptions {
            forced: Some(Encoding::ShiftJis),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    assert_eq!(doc_of(&h, doc).format.encoding, Encoding::ShiftJis);
    h.cmd_with(
        "file.save_as",
        eve_core::Args::new().with_str("path", "/w/sjis.txt"),
    );
    h.run_jobs();
    assert_eq!(h.fs_read("/w/sjis.txt").unwrap(), bytes);
}

#[test]
fn 最初のチャンクの後に黙るパイプでも読んだ量が届き_閉じると確認する() {
    // 閉じないパイプ (`tail -f`): 最初のチャンクの後に書き手が黙ると、以後の進捗は来ない。最初のデータの進捗は間引かずに
    // 送るので、core は 1 バイト以上読んだことを知り、閉じる前に確認する (03 §9.2.2 の「閉じる・終了」)
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    let mut out = other_run(
        vec![
            StdinStep::Data(b"last 10 lines\n".to_vec()),
            StdinStep::Hang,
        ],
        true,
    );
    let done = out.pop().expect("終端");
    assert!(matches!(done.body, JobResult::StdinDone { .. }));
    assert!(
        out.iter()
            .any(|o| matches!(o.body, JobResult::Progress { bytes: 14, .. })),
        "最初のデータの進捗"
    );
    deliver(&mut h, out);
    assert!(h.frame_contains("[読込中 14 B]"), "{}", h.frame_text());
    h.cmd("buffer.close");
    assert_eq!(confirm_name(&h).as_deref(), Some("stdin_loading"));
    assert!(h.editor.state().document(doc).is_some());
}

#[test]
fn readonly_の読込中の文書でここまでで開くを選ぶと閉じずに残す() {
    // `--readonly` の文書は読み終えても変更なしなので、続きの FR-107 の確認が出ない。「ここまでで開く」が「破棄」と同じに
    // ならないよう、閉じる・終了は進めずに文書を残す
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness_with(
        &fake,
        OpenOptions {
            user_read_only: true,
            ..OpenOptions::default()
        },
    );
    progress(&mut h, 8);
    h.cmd("buffer.close");
    assert_eq!(confirm_name(&h).as_deref(), Some("stdin_loading"));
    h.answer(0);
    let mut out = other_run(
        vec![StdinStep::Data(b"partial\n".to_vec()), StdinStep::Hang],
        true,
    );
    // 別に走らせた読込は `--readonly` を付けていないので、利用者の理由を足す
    if let Some(JobOutput {
        body: JobResult::StdinDone { detected, .. },
        ..
    }) = out.last_mut()
    {
        detected.insert(ReadOnlyReason::UserRequested);
    }
    deliver(&mut h, out);
    assert!(h.editor.state().document(doc).is_some(), "閉じずに残す");
    assert_eq!(h.doc_text(), "partial\n");
    h.cmd("app.quit");
    assert_eq!(confirm_name(&h), None);
    assert_eq!(
        h.quit_code(),
        Some(0),
        "読み終えた後の --readonly は確認なしで終わる"
    );
}

#[test]
fn 受け取る前の確保の失敗は読めなかったと知らせる() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    deliver(
        &mut h,
        vec![JobOutput {
            job: STDIN_JOB,
            body: JobResult::StdinDone {
                text: eve_text::Text::from(""),
                format: eve_core::TextFormat::new_document(),
                detected: eve_core::ReadOnlyReasons::default(),
                preserved: eve_core::PreservedMap::default(),
                bytes: 0,
                end: StdinEnd::OutOfMemory { requested: 1 << 16 },
            },
        }],
    );
    assert!(doc_of(&h, doc).read_only.is_empty());
    assert!(h.messages().contains(&"msg.stdin.unreadable".to_owned()));
    assert!(!h.messages().contains(&"msg.stdin.out_of_memory".to_owned()));
}

#[test]
fn 受け取った後の失敗は開いた量が_0_でも読取の失敗() {
    // 切れた多バイト列だけを受け取った (開いた量は 0、`LoadIncomplete`)
    let fake = FakeStdin::new([
        StdinStep::Data(b"\xe3\x81".to_vec()),
        StdinStep::Error(eve_core::FsErrorKind::Other),
    ]);
    let (mut h, doc) = harness(&fake);
    h.run_jobs();
    assert_eq!(h.doc_text(), "");
    assert!(
        doc_of(&h, doc)
            .read_only
            .contains(ReadOnlyReason::LoadIncomplete)
    );
    assert!(h.messages().contains(&"msg.stdin.read_failed".to_owned()));
    assert!(!h.messages().contains(&"msg.stdin.unreadable".to_owned()));
}

/// a.txt の保存ジョブを投入した後に標準入力の文書を足し (表示しない)、`bytes` まで読んだ進捗を届けてから `app.quit` →
/// 5 秒 → `ui.confirm.quit_saving` を出す。(Harness, 標準入力の文書, 保存のジョブ, 標準入力の読込のジョブ)。
fn quit_saving_with_stdin(fake: &FakeStdin, bytes: u64) -> (Harness, DocumentId, JobId, JobId) {
    let mut h = Harness::builder()
        .defer_start()
        .stdin(Arc::new(fake.clone()))
        .file("/w/a.txt", b"abc")
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/a.txt")], false, None);
    h.started();
    h.run_jobs();
    h.text("x");
    let CommandResult::Pending(save) = h.cmd("file.save") else {
        panic!("保存ジョブを投入する");
    };
    let stdin_doc = h.editor.state_mut().open_stdin(OpenOptions::default());
    let job = JobId(save.0 + 1);
    deliver(
        &mut h,
        vec![JobOutput {
            job,
            body: JobResult::Progress { bytes, total: 0 },
        }],
    );
    assert_eq!(
        doc_of(&h, stdin_doc).stdin.map(|s| s.bytes_read),
        Some(bytes),
        "標準入力の読込のジョブの番号"
    );
    h.cmd("app.quit");
    h.advance_ms(5000);
    assert_eq!(confirm_name(&h).as_deref(), Some("quit_saving"));
    (h, stdin_doc, save, job)
}

fn quit_rescue(h: &Harness) -> Option<eve_view::QuitRescue> {
    h.effects().iter().find_map(|e| match e {
        Effect::Quit { rescue, .. } => Some(*rescue),
        _ => None,
    })
}

fn cancelled(job: JobId) -> JobOutput {
    JobOutput {
        job,
        body: JobResult::Failed {
            err: JobError::Cancelled,
        },
    }
}

#[test]
fn 保存を中止して終了は読込中の標準入力を止め_終端を待って退避してから終わる() {
    // 03 §9.7 (f) 3: 1 バイト以上受け取った読込中の標準入力の文書は黙って捨てない
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, stdin_doc, save, job) = quit_saving_with_stdin(&fake, 8);
    h.answer(1); // 保存を中止して終了
    assert_eq!(fake.wakes(), 1, "標準入力の読込も止める");
    // 取り消した保存の終端が先に届いても、標準入力の終端を待つ
    deliver(&mut h, vec![cancelled(save)]);
    assert_eq!(quit_rescue(&h), None, "標準入力の終端の前は終わらない");
    deliver(
        &mut h,
        other_run_as(
            job,
            vec![StdinStep::Data(b"partial\n".to_vec()), StdinStep::Hang],
            true,
        ),
    );
    let d = doc_of(&h, stdin_doc);
    assert!(d.is_dirty(), "読み終えた文書は変更あり");
    assert!(d.read_only.contains(ReadOnlyReason::LoadIncomplete));
    assert_eq!(
        quit_rescue(&h),
        Some(eve_view::QuitRescue::DirtyDocuments),
        "退避を伴って終了する"
    );
    let snap = h
        .editor
        .emergency_snapshot(stdin_doc)
        .expect("退避の対象に入る");
    assert!(snap.sensitive);
    assert_eq!(snap.to_vec(), b"partial\n");
}

#[test]
fn 保存を中止して終了で標準入力の終端が先でも保存の終端を待つ() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, stdin_doc, save, job) = quit_saving_with_stdin(&fake, 8);
    h.answer(1);
    deliver(
        &mut h,
        other_run_as(
            job,
            vec![StdinStep::Data(b"late\n".to_vec()), StdinStep::Hang],
            true,
        ),
    );
    assert_eq!(quit_rescue(&h), None, "保存の終端の前は終わらない");
    deliver(&mut h, vec![cancelled(save)]);
    assert_eq!(quit_rescue(&h), Some(eve_view::QuitRescue::DirtyDocuments));
    assert!(h.editor.emergency_snapshot(stdin_doc).is_some());
}

#[test]
fn 保存を中止して終了は一バイトも受け取っていない標準入力も止めて待つ() {
    // 受け取った量によらず止めて待つ (選んだ時点の進捗は遅れることがあり、受け渡し待ちの分も取り込むため)。0 バイトで
    // 終われば空の文書で、退避しない
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, stdin_doc, save, job) = quit_saving_with_stdin(&fake, 0);
    h.answer(1);
    assert_eq!(fake.wakes(), 1, "止める");
    deliver(&mut h, vec![cancelled(save)]);
    assert_eq!(quit_rescue(&h), None, "標準入力の終端を待つ");
    deliver(&mut h, other_run_as(job, vec![StdinStep::Hang], true));
    assert_eq!(quit_rescue(&h), Some(eve_view::QuitRescue::DirtyDocuments));
    assert!(h.editor.emergency_snapshot(stdin_doc).is_none());
}

#[test]
fn 保存を中止して終了の待ちの間に標準入力の文書を閉じても終わる() {
    // 保存が先に終わり、`StdinDone` の前に閉じる (「破棄」): 待ちの対象が外れたので終わる (03 §9.7 (f) 3)
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, stdin_doc, save, _) = quit_saving_with_stdin(&fake, 8);
    h.answer(1);
    deliver(&mut h, vec![cancelled(save)]);
    assert_eq!(quit_rescue(&h), None);
    h.editor.state_mut().close_document(stdin_doc);
    h.advance_ms(0);
    assert_eq!(quit_rescue(&h), Some(eve_view::QuitRescue::DirtyDocuments));
}

#[test]
fn 保存を中止して終了は_readonly_の標準入力も待つが退避しない() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, stdin_doc, save, job) = quit_saving_with_stdin(&fake, 8);
    h.answer(1);
    assert_eq!(fake.wakes(), 1);
    deliver(&mut h, vec![cancelled(save)]);
    let mut out = other_run_as(
        job,
        vec![StdinStep::Data(b"ro\n".to_vec()), StdinStep::Hang],
        true,
    );
    if let Some(JobOutput {
        body: JobResult::StdinDone { detected, .. },
        ..
    }) = out.last_mut()
    {
        detected.insert(ReadOnlyReason::UserRequested);
    }
    deliver(&mut h, out);
    assert_eq!(quit_rescue(&h), Some(eve_view::QuitRescue::DirtyDocuments));
    assert!(!doc_of(&h, stdin_doc).is_dirty());
    assert!(
        h.editor.emergency_snapshot(stdin_doc).is_none(),
        "--readonly は変更なしで退避しない"
    );
}

#[test]
fn 終了の連鎖で破棄の印を付けた標準入力の文書は保存を中止して終了で待たない() {
    // 連鎖: 表示中の標準入力の文書 (読込中) を「破棄」、次の a.txt を「保存」(ジョブはキューの中) → もう一度 `app.quit`
    // → 5 秒 → 「保存を中止して終了」。破棄の印の付いた標準入力の読込は止めず、待たずに終わる
    let fake = FakeStdin::new([StdinStep::Hang]);
    let mut h = Harness::builder()
        .defer_start()
        .stdin(Arc::new(fake.clone()))
        .file("/w/a.txt", b"abc")
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/a.txt")], false, None);
    h.started();
    h.run_jobs();
    h.text("x");
    let stdin_doc = h.editor.state_mut().open_stdin(OpenOptions::default());
    // 起動時の読込・版の記録・退避の走査の 3 つの後
    let job = JobId(4);
    deliver(
        &mut h,
        vec![JobOutput {
            job,
            body: JobResult::Progress { bytes: 8, total: 0 },
        }],
    );
    assert_eq!(doc_of(&h, stdin_doc).stdin.map(|s| s.bytes_read), Some(8));
    h.cmd("buffer.next");
    assert_eq!(h.doc_id(), stdin_doc);
    h.cmd("app.quit");
    assert_eq!(confirm_name(&h).as_deref(), Some("stdin_loading"));
    h.answer(1); // 破棄 (印を付ける)
    assert_eq!(confirm_name(&h).as_deref(), Some("unsaved"));
    let CommandResult::Pending(save) = h.answer(0) else {
        panic!("a.txt の保存ジョブを投入する");
    };
    // 保存の完了を待つ間に、もう一度 `app.quit` (待つ) → 5 秒で確認
    h.cmd("app.quit");
    h.advance_ms(5000);
    assert_eq!(confirm_name(&h).as_deref(), Some("quit_saving"));
    h.answer(1);
    assert_eq!(fake.wakes(), 0, "破棄の印の付いた標準入力の読込は止めない");
    deliver(&mut h, vec![cancelled(save)]);
    assert_eq!(
        quit_rescue(&h),
        Some(eve_view::QuitRescue::DirtyDocuments),
        "標準入力の終端を待たずに終わる"
    );
    assert!(h.editor.emergency_snapshot(stdin_doc).is_none());
}

// ---- 閉じる・終了の続き (3 回目のレビュー) ----

fn panicked() -> Vec<JobOutput> {
    vec![JobOutput {
        job: STDIN_JOB,
        body: JobResult::Failed {
            err: JobError::Panic,
        },
    }]
}

fn partial_done() -> Vec<JobOutput> {
    other_run(
        vec![StdinStep::Data(b"partial\n".to_vec()), StdinStep::Hang],
        true,
    )
}

#[test]
fn ここまでで開くの後に読込がパニックで終わっても終了の連鎖は進む() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    progress(&mut h, 8);
    h.cmd("app.quit");
    assert_eq!(confirm_name(&h).as_deref(), Some("stdin_loading"));
    h.answer(0); // ここまでで開く
    deliver(&mut h, panicked());
    assert!(h.editor.state().document(doc).is_none(), "読んだ内容は失う");
    assert_eq!(h.quit_code(), Some(0), "連鎖の続きが進んで終わる");
}

#[test]
fn ここまでで開くの後に読込がパニックで終わると閉じるは済む() {
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    progress(&mut h, 8);
    h.cmd("buffer.close");
    h.answer(0);
    deliver(&mut h, panicked());
    assert!(h.editor.state().document(doc).is_none());
    assert_eq!(confirm_name(&h), None);
    assert_eq!(h.quit_code(), None);
}

#[test]
fn 読み終えた後の続きはほかのオーバーレイの間は待ち_閉じた後に進む() {
    // 「ここまでで開く」の後、読み終える前にほかの入力 (ファイルを開く) を開いた: FR-107 の確認で置き換えない
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    progress(&mut h, 8);
    h.cmd("buffer.close");
    h.answer(0);
    h.cmd("file.open");
    assert_eq!(h.overlay(), "prompt");
    deliver(&mut h, partial_done());
    assert_eq!(h.overlay(), "prompt", "開いている入力を置き換えない");
    h.keys("esc");
    assert_eq!(
        confirm_name(&h).as_deref(),
        Some("unsaved_close"),
        "入力を閉じた後に続きを進める"
    );
    assert!(h.editor.state().document(doc).is_some());
}

#[test]
fn 閉じるの確認の表示中に読み終えた後の各選択肢() {
    // ここまでで開く: そのまま FR-107 の確認へ
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    progress(&mut h, 8);
    h.cmd("buffer.close");
    deliver(&mut h, partial_done());
    assert_eq!(confirm_name(&h).as_deref(), Some("stdin_loading"));
    h.answer(0);
    assert_eq!(confirm_name(&h).as_deref(), Some("unsaved_close"));
    assert!(h.editor.state().document(doc).is_some());
    // 破棄: 読み終えた文書を閉じる
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    progress(&mut h, 8);
    h.cmd("buffer.close");
    deliver(&mut h, partial_done());
    h.answer(1);
    assert!(h.editor.state().document(doc).is_none());
    // キャンセル: 文書は残り、確認は出ない
    let fake = FakeStdin::new([StdinStep::Hang]);
    let (mut h, doc) = harness(&fake);
    progress(&mut h, 8);
    h.cmd("buffer.close");
    deliver(&mut h, partial_done());
    h.keys("esc");
    assert_eq!(confirm_name(&h), None);
    assert!(h.editor.state().document(doc).is_some());
    assert!(doc_of(&h, doc).is_dirty());
}

#[test]
fn 終了の連鎖で破棄した読込中の文書が読み終わるともう一度尋ねる() {
    // 03 §9.8: 破棄の印は Revision と組。読み終えると Revision が進むので、安全側としてもう一度 FR-107 で尋ねる
    let fake = FakeStdin::new([StdinStep::Hang]);
    let mut h = Harness::builder()
        .defer_start()
        .stdin(Arc::new(fake.clone()))
        .file("/w/a.txt", b"abc")
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/a.txt")], false, None);
    h.started();
    h.run_jobs();
    h.text("x");
    let stdin_doc = h.editor.state_mut().open_stdin(OpenOptions::default());
    let job = JobId(4);
    deliver(
        &mut h,
        vec![JobOutput {
            job,
            body: JobResult::Progress { bytes: 8, total: 0 },
        }],
    );
    h.cmd("buffer.next");
    h.cmd("app.quit");
    assert_eq!(confirm_name(&h).as_deref(), Some("stdin_loading"));
    h.answer(1); // 破棄 (印を付ける)
    assert_eq!(confirm_name(&h).as_deref(), Some("unsaved"));
    // a.txt を尋ねている間に、標準入力の読込が終わる (EOF)
    let mut out = other_run_as(job, vec![StdinStep::Data(b"late\n".to_vec())], false);
    out.retain(|o| matches!(o.body, JobResult::StdinDone { .. }));
    deliver(&mut h, out);
    assert!(doc_of(&h, stdin_doc).is_dirty());
    h.answer(1); // a.txt を破棄
    assert_eq!(
        confirm_name(&h).as_deref(),
        Some("unsaved"),
        "読み終えた標準入力の文書をもう一度尋ねる"
    );
    assert_eq!(h.doc_id(), stdin_doc);
    assert_eq!(h.quit_code(), None);
}

#[test]
fn 名前を付けて保存の後の読み取り専用の切替は変更ありにせず_sensitive_は残る() {
    let fake = FakeStdin::chunks(b"log\n", 2);
    let (mut h, doc) = harness_with(
        &fake,
        OpenOptions {
            user_read_only: true,
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    h.cmd_with(
        "file.save_as",
        eve_core::Args::new().with_str("path", "/w/log.txt"),
    );
    h.run_jobs();
    assert_eq!(h.doc_path().as_deref(), Some("/w/log.txt"));
    // 名前を付けて保存しても `--readonly` は残る。解除しても保存したファイルと一致するので変更ありにしない
    h.cmd("file.toggle_read_only");
    assert!(doc_of(&h, doc).read_only.is_empty());
    assert!(!h.is_dirty(), "保存した後の解除では変更ありにしない");
    h.text("y");
    let snap = h.editor.emergency_snapshot(doc).expect("編集で dirty");
    assert!(snap.sensitive, "標準入力から読んだ文書の印は残る");
}
