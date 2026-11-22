//! 同じファイルの判定 (SAF-28、docs/03 §12.7.6): 別の書き方のパス (シンボリックリンク・ハードリンク・`..`) で同じファイルを
//! 開くと 1 つの文書になる (03 §9.2 手順 2b)。名前を付けて保存の保存先の照合 (`CheckTarget`、03 §9.7 (a)) の表の各行。
//! 識別 (`FileId`) を信頼しない FS (`exFAT`・ネットワーク。`MemFs::set_file_id_unique(false)`) では canonical のパスで比べる。
//! 実 FS (大文字小文字・NTFS / `exFAT`・Linux) の試験は eve-platform の `tests/same_file.rs`。

use eve_core::{Args, DocumentId};
use eve_testkit::Harness;

// ---------- レビューの指摘 (canonical の代わりのパス・閉じた文書の照合・Esc の順・古い識別の後始末) ----------

// verifies: FR-103
#[test]
fn canonicalize_できないパスでは識別で同じとしない() {
    // シャドウコピー・ドライブ文字の無いボリュームの先: canonicalize に失敗し、代わりのパスではボリュームの条件が効かない
    // (03 §12.7.6 の 0)。識別が一致しても同じとしない
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    h.fs.fail_next(
        eve_testkit::FsOp::Canonicalize,
        eve_core::FsErrorKind::Other,
    );
    h.open("/w/hard.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2, "{:?}", paths(&h));
}

// verifies: FR-104
#[test]
fn canonicalize_できない保存先は今の文書と同じとしない() {
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    h.fs.fail_next(
        eve_testkit::FsOp::Canonicalize,
        eve_core::FsErrorKind::Other,
    );
    save_as(&mut h, "/w/hard.txt");
    assert_eq!(
        h.editor.state().confirm_name(),
        Some("save.overwrite"),
        "既存のファイルの上書きの確認 (表の 6)。表の 1 ならハードリンクの確認 (save.hardlink) になる"
    );
    h.keys("r");
    assert_eq!(
        disk(&h, "/w/a.txt").as_deref(),
        Some("alpha\n"),
        "a は書かない"
    );
    assert_eq!(h.doc_id(), a);
    assert!(h.is_dirty());
}

// verifies: FR-103
#[test]
fn 古い識別の文書だけを下ろし_新しい文書の識別は使える() {
    // a の識別は古く、同じ番号を b が使う。b を開いた後、b のハードリンクを開くと b に切り替える (a ではない)
    let (mut h, a) = stale_id();
    h.open("/w/b.txt");
    h.run_jobs();
    let b = h.doc_id();
    assert_ne!(b, a);
    h.fs.add_hardlink("/w/b2.txt", "/w/b.txt");
    h.open("/w/b2.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2, "{:?}", paths(&h));
    assert_eq!(h.doc_id(), b, "b の識別は使える。古い a とは識別で比べない");
}

// verifies: FR-103
#[test]
fn 文書のファイルを調べ直せなければ古い識別として扱う() {
    // a.txt を外で消した (metadata に失敗する) 後、a の古い番号を b が使う
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    let old = h.fs.file_id_of("/w/a.txt").expect("a の識別");
    h.fs.remove(&eve_core::FsPath::from("/w/a.txt"));
    h.fs.add_file("/w/b.txt", b"beta\n");
    h.fs.set_file_id("/w/b.txt", old);
    h.open("/w/b.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2, "{:?}", paths(&h));
    assert_ne!(h.doc_id(), a);
}

// verifies: FR-104
#[test]
fn 照合中に文書を閉じても終了と開くは効く() {
    let (mut h, a, _) = a_dirty_and_b();
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    h.editor.state_mut().close_document(a);
    // 閉じた文書の名前の入力は開かない
    h.keys("esc");
    assert_eq!(h.overlay(), "");
    h.cmd_with("file.open", Args::new().with_str("path", "/w/b.txt"));
    assert!(!has_msg(&h, "info.save.in_progress"));
    h.run_jobs();
    assert!(disk(&h, "/w/c.txt").is_none(), "閉じた文書の照合は捨てる");
    h.cmd("app.quit");
    assert!(h.quit_code().is_some(), "dirty な文書は無いので終わる");
}

// verifies: FR-104
#[test]
fn 保留した照合の文書を閉じても終了は効く() {
    let (mut h, a, _) = a_dirty_and_b();
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    h.cmd("ui.palette_open");
    h.run_jobs();
    h.editor.state_mut().close_document(a);
    h.keys("esc");
    assert_eq!(h.overlay(), "", "閉じた文書の照合の結果は出さない");
    h.cmd("app.quit");
    assert!(h.quit_code().is_some());
    assert!(disk(&h, "/w/c.txt").is_none());
}

// verifies: FR-104
#[test]
fn esc_は今の文書の読込を照合より先に取り消す() {
    // 照合を待つ a と、読込中の d。d を表示しているときの Esc は d の読込を取り消し、次の Esc で a の照合を取り消す
    // (03 §12.7.6 の細目 (9) の順)
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_file("/w/d.txt", b"delta\n");
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    h.editor
        .state_mut()
        .open_path("/w/d.txt", eve_core::OpenOptions::default());
    for _ in 0..3 {
        if h.doc_path().as_deref() == Some("/w/d.txt") {
            break;
        }
        h.cmd("buffer.next");
    }
    assert_eq!(
        h.doc_path().as_deref(),
        Some("/w/d.txt"),
        "読込中の d を表示"
    );
    h.keys("esc");
    assert_eq!(
        h.overlay(),
        "",
        "今の文書 (d) の読込を取り消す (照合は取り消さないので名前の入力は出ない)"
    );
    // d の読込は取り消し済みで結果を待つだけ: 次の Esc は d に受け止められず、a の照合を取り消す
    h.keys("esc");
    assert_eq!(
        h.overlay(),
        "prompt",
        "次の Esc で a の照合を取り消し、名前の入力に戻る"
    );
    h.keys("esc");
    h.run_jobs();
    assert!(
        !paths(&h).iter().any(|p| p == "/w/d.txt"),
        "d の読込は取り消した"
    );
    assert!(disk(&h, "/w/c.txt").is_none(), "照合は取り消した");
    assert!(h.editor.state().document(a).is_some());
}

/// 保存先の照合・開くジョブの結果を、ジョブを走らせずに渡す (主スレッドの受け止めだけを確かめる)。
fn job_of(r: eve_core::CommandResult) -> eve_view::JobId {
    match r {
        eve_core::CommandResult::Pending(job) => job,
        other => panic!("ジョブを投入する: {other:?}"),
    }
}

fn file_id(h: &Harness, path: &str) -> eve_core::FileId {
    eve_core::FileId {
        volume: 1,
        index: h.fs.file_id_of(path).expect("識別"),
    }
}

// verifies: FR-103
#[test]
fn canonicalize_が_2_回とも失敗する開くは識別で同じとしない() {
    // `path` も親も canonicalize できない: canonical は無く、主スレッドが入力したパスで代える (03 §12.7.6 の 0)
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    for _ in 0..2 {
        h.fs.fail_next(
            eve_testkit::FsOp::Canonicalize,
            eve_core::FsErrorKind::Other,
        );
    }
    h.open("/w/hard.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2, "{:?}", paths(&h));
}

// verifies: FR-103
#[test]
fn canonical_の無い識別は主スレッドでも使わない() {
    // ジョブが canonical を返さず識別だけを信頼できると言っても、主スレッドは代わりのパスでは識別を使わない
    // (same_file.rs の `on_open_identity` の受け止め)
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    let id = file_id(&h, "/w/a.txt");
    let job = job_of(h.open("/w/hard.txt"));
    h.editor.on_job_result(eve_core::JobOutput {
        job,
        body: eve_core::JobResult::Identity {
            file_id: Some(id),
            file_id_unique: true,
            canonical: None,
            stale: Vec::new(),
        },
    });
    assert_eq!(docs(&h), 2, "新しい文書を閉じない: {:?}", paths(&h));
}

// verifies: FR-104
#[test]
fn canonicalize_が_2_回とも失敗する保存先は今の文書と同じとしない() {
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    for _ in 0..2 {
        h.fs.fail_next(
            eve_testkit::FsOp::Canonicalize,
            eve_core::FsErrorKind::Other,
        );
    }
    save_as(&mut h, "/w/hard.txt");
    assert_eq!(h.editor.state().confirm_name(), Some("save.overwrite"));
    h.keys("r");
    assert_eq!(disk(&h, "/w/a.txt").as_deref(), Some("alpha\n"));
    assert_eq!(h.doc_id(), a);
}

// verifies: FR-104
#[test]
fn canonical_の無い保存先の識別は主スレッドでも使わない() {
    // 照合のジョブが canonical を返さず識別だけを信頼できると言っても、主スレッドは入力したパスで代えるときは識別を
    // 使わない (save_as.rs の `apply_save_as_target` の受け止め)
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    let id = file_id(&h, "/w/hard.txt");
    let meta = eve_core::FileMeta {
        kind: eve_core::FileKind::File,
        len: 6,
        mtime: eve_core::FileTime::default(),
        readonly: false,
        file_id: Some(id),
        nlink: 2,
        link: None,
        fs_kind: eve_core::FsKind::Local,
        owner_is_me: Some(true),
        file_id_unique: true,
    };
    let stamp = eve_core::FileStamp::from_meta(&meta, None);
    let job = job_of(h.cmd_with("file.save_as", Args::new().with_str("path", "/w/hard.txt")));
    h.editor.on_job_result(eve_core::JobOutput {
        job,
        body: eve_core::JobResult::Meta {
            meta,
            stamp,
            canonical: None,
            stale: Vec::new(),
        },
    });
    assert_eq!(
        h.editor.state().confirm_name(),
        Some("save.overwrite"),
        "表の 1 (今の文書への上書き保存) にしない"
    );
    assert_eq!(h.doc_id(), a);
}

// verifies: FR-103
#[test]
fn 投入の後に同じ識別を得た文書は古い識別として下ろさない() {
    // a の識別 X は古い (a.txt は作り直された)。b.txt が X を使い、c.txt は b のハードリンク (X)。b と c を続けて開く:
    // c の結果の `stale` は a だけを指し、c のジョブの投入の後に同じ X を得た b は下ろさない (パスの条件)。c は b に
    // 切り替わる
    let (mut h, a) = stale_id();
    h.fs.add_hardlink("/w/c.txt", "/w/b.txt");
    h.open("/w/b.txt");
    h.open("/w/c.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2, "a と b だけ: {:?}", paths(&h));
    assert_eq!(h.doc_path().as_deref(), Some("/w/b.txt"));
    assert!(h.editor.state().document(a).is_some());
}

// verifies: FR-104
#[test]
fn 失敗した名前を付けて保存の識別の覚えは捨てる() {
    // 既存の c.txt への保存が Busy で失敗し、再試行で成功した: 照合の識別の覚えは失敗の時点で捨てたので、文書の識別は
    // 信頼しない (03 §12.7.6 の細目 (10))。c のハードリンクを開くと別の文書になる (canonical のパスは違う)
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_file("/w/c.txt", b"old\n");
    save_as(&mut h, "/w/c.txt");
    assert_eq!(h.overlay(), "dialog", "上書きの確認");
    h.fs.busy_times(eve_testkit::FsOp::WriteAtomic, 1);
    h.keys("o");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "保存の失敗の確認");
    h.keys("t");
    h.run_jobs();
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("xalpha\n"));
    assert_eq!(h.doc_path().as_deref(), Some("/w/c.txt"));
    assert_eq!(h.doc_id(), a);
    h.fs.add_hardlink("/w/c-hard.txt", "/w/c.txt");
    h.open("/w/c-hard.txt");
    h.run_jobs();
    assert_ne!(h.doc_id(), a, "識別では同じとしない");
}

fn docs(h: &Harness) -> usize {
    h.editor.state().documents_len()
}

/// 開いている文書のパス (表示の形、並びは文書の順)。
fn paths(h: &Harness) -> Vec<String> {
    let Ok(eve_view::Answer::Documents(ds)) = h.query(&eve_view::Query::Documents) else {
        panic!("文書の一覧");
    };
    ds.iter().filter_map(|d| d.path.clone()).collect()
}

fn opened(files: &[(&str, &[u8])]) -> Harness {
    let h = Harness::new();
    for (p, data) in files {
        h.fs.add_file(p, data);
    }
    h
}

fn save_as(h: &mut Harness, path: &str) {
    h.cmd_with("file.save_as", Args::new().with_str("path", path));
    h.run_jobs();
}

fn disk(h: &Harness, path: &str) -> Option<String> {
    h.fs_read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn has_msg(h: &Harness, key: &str) -> bool {
    h.messages().iter().any(|m| m == key)
}

// ---------- 開く (03 §9.2 手順 2b) ----------

// verifies: FR-103
#[test]
fn シンボリックリンクで同じファイルを開くと既存の文書に切り替える() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.add_link("/w/link.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    h.open("/w/link.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 1, "1 つの文書: {:?}", paths(&h));
    assert_eq!(h.doc_id(), a);
    // 打ったパスと既存の文書のパスが違うので、どの文書として開いているかを示す
    assert!(has_msg(&h, "info.open.same_file"), "{:?}", h.messages());
    // 同じパスをもう一度開くと「既に開いています」
    h.open("/w/a.txt");
    assert!(has_msg(&h, "info.open.already_open"));
}

// verifies: FR-103
#[test]
fn ハードリンクで同じファイルを開くと既存の文書を出す() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n"), ("/w/b.txt", b"beta\n")]);
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    h.open("/w/b.txt");
    h.run_jobs();
    assert_ne!(h.doc_id(), a);
    // 今の文書 (b) と違う既存の文書 (a) を、開いた時点のペインに出す
    h.open("/w/hard.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2, "a と b だけ: {:?}", paths(&h));
    assert_eq!(h.doc_id(), a, "既存の a を出す");
    assert!(!has_msg(&h, "info.open.already_open"));
    assert!(
        has_msg(&h, "info.open.same_file"),
        "名前の違う a に切り替えたことを示す"
    );
}

/// a.txt を開いた後に、外で a.txt を作り直し (新しい識別)、古い識別を b.txt が使い回した状態 (inode を使い回す FS)。
fn stale_id() -> (Harness, eve_core::DocumentId) {
    let mut h = opened(&[(
        "/w/a.txt", b"alpha
",
    )]);
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    let old = h.fs.file_id_of("/w/a.txt").expect("a の識別");
    h.fs.remove(&eve_core::FsPath::from("/w/a.txt"));
    h.fs.add_file(
        "/w/a.txt",
        b"recreated
",
    );
    h.fs.add_file(
        "/w/b.txt", b"beta
",
    );
    h.fs.set_file_id("/w/b.txt", old);
    assert_ne!(h.fs.file_id_of("/w/a.txt"), Some(old), "a は新しい識別");
    (h, a)
}

// verifies: FR-103
#[test]
fn 古い識別を使い回した別のファイルは別の文書として開く() {
    let (mut h, a) = stale_id();
    h.open("/w/b.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2, "{:?}", paths(&h));
    assert_ne!(h.doc_id(), a, "b を開く (a に切り替えない)");
    assert_eq!(
        h.doc_text(),
        "beta
"
    );
}

// verifies: FR-104
#[test]
fn 古い識別を使い回した別のファイルへの名前を付けて保存は上書き保存にしない() {
    let (mut h, a) = stale_id();
    assert!(
        !h.is_dirty(),
        "未編集 (表の 1 なら何も書かずに済ませてしまう)"
    );
    save_as(&mut h, "/w/b.txt");
    assert_eq!(h.overlay(), "dialog", "既存の b の上書きの確認 (表の 6)");
    h.keys("o");
    h.run_jobs();
    assert_eq!(
        disk(&h, "/w/b.txt").as_deref(),
        Some(
            "alpha
"
        ),
        "b に書く"
    );
    assert_eq!(h.doc_id(), a);
    assert_eq!(h.doc_path().as_deref(), Some("/w/b.txt"));
}

// verifies: FR-103
#[test]
fn 起動時の行の指定は読込中の既存の文書に引き継ぐ() {
    // `evoeditor a.txt link.txt:3` で、link.txt が a.txt と同じファイル: link の識別が届いた時点で a はまだ読込中なので、
    // 3 行目への移動を a に引き継ぐ (03 §12.7.6 の細目 (3))
    let data: Vec<u8> = (0..100)
        .flat_map(|i| {
            format!(
                "line {i:06}
"
            )
            .into_bytes()
        })
        .collect();
    let mut h = Harness::builder()
        .file("/w/a.txt", &data)
        .defer_start()
        .build();
    h.fs.add_link("/w/link.txt", "/w/a.txt");
    // 遅い読込 (64 バイトごとに 100 ms): a の読込は先行表示の先頭で止まる
    h.fs.set_chunk_size(64);
    h.fs.set_read_delay(Some((h.clock.clone(), eve_core::Millis(100))));
    let files = [
        eve_core::InitialFile::path("/w/a.txt"),
        eve_core::InitialFile {
            line: Some(3),
            ..eve_core::InitialFile::path("/w/link.txt")
        },
    ];
    h.editor.open_initial_files(&files, false, None);
    h.started();
    assert!(h.run_jobs_until_partial(), "a は読込の途中");
    // link の読込 (識別) を、a の読込が終わる前に渡す
    h.run_jobs_once();
    assert_eq!(docs(&h), 1, "{:?}", paths(&h));
    h.finish_held_jobs();
    let view = match h.query(&eve_view::Query::Windows).unwrap() {
        eve_view::Answer::Windows(ws) => ws[0].views[0],
        a => panic!("{a:?}"),
    };
    let head = match h.query(&eve_view::Query::Selection { view }).unwrap() {
        eve_view::Answer::Selection {
            ranges, primary, ..
        } => ranges[primary].head,
        a => panic!("{a:?}"),
    };
    assert_eq!(head, 24, "3 行目の先頭 (1 行 12 バイト) へ移る");
}

// verifies: FR-103
#[test]
fn 識別を信頼しない_fs_ではハードリンクは別の文書になる() {
    // exFAT・ネットワーク: 識別が使い回され・重なり得るので使わない (05 §5.4)。canonical のパスは違う
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.set_file_id_unique(false);
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    h.open("/w/hard.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2, "{:?}", paths(&h));
}

// verifies: FR-103
#[test]
fn 識別を信頼しない_fs_でも_canonical_のパスが同じなら_1_つの文書() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.set_file_id_unique(false);
    h.fs.add_link("/w/link.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    h.open("/w/link.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 1, "{:?}", paths(&h));
}

// verifies: FR-103
#[test]
fn 同時に開いた同じファイルは後から来たほうを閉じる() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.add_link("/w/link.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.open("/w/link.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 1, "{:?}", paths(&h));
    assert_eq!(paths(&h), ["/w/a.txt"]);
    assert_eq!(h.doc_text(), "alpha\n");
}

// verifies: FR-103, FR-127
#[test]
fn 確認を待つ文書も識別を持つ() {
    // 巨大ファイルの確認を待つ間 (読込中) に、別の書き方のパスで同じファイルを開いても 2 つ目の文書を作らない
    let mut h = opened(&[("/w/big.txt", b"x\n")]);
    h.fs.set_fake_len("/w/big.txt", Some(1 << 40));
    h.fs.add_link("/w/link.txt", "/w/big.txt");
    h.open("/w/big.txt");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "巨大ファイルの確認");
    h.keys("esc");
    // 確認を閉じた (中止) ので文書は閉じている。もう一度開いて確認を待つ間に、リンクを開く
    h.open("/w/big.txt");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    let before = docs(&h);
    h.editor
        .state_mut()
        .open_path("/w/link.txt", eve_core::OpenOptions::default());
    h.run_jobs();
    assert_eq!(
        docs(&h),
        before,
        "確認を待つ文書と同じファイル: {:?}",
        paths(&h)
    );
}

// verifies: FR-103
#[test]
fn 新規のファイルも親の_canonical_とファイル名で比べる() {
    // まだ無いファイルを 2 通りの書き方 (リンクのディレクトリ経由) で開く
    let mut h = Harness::new();
    h.fs.add_dir("/w/real");
    h.fs.add_link("/w/alias", "/w/real");
    h.open("/w/real/new.txt");
    h.run_jobs();
    h.open("/w/alias/new.txt");
    h.run_jobs();
    let news: Vec<_> = paths(&h)
        .into_iter()
        .filter(|p| p.ends_with("new.txt"))
        .collect();
    assert_eq!(news.len(), 1, "{news:?}");
}

// verifies: FR-103
#[test]
fn 別のファイルは別の文書のまま() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n"), ("/w/b.txt", b"beta\n")]);
    h.open("/w/a.txt");
    h.run_jobs();
    h.open("/w/b.txt");
    h.run_jobs();
    assert_eq!(docs(&h), 2);
}

// ---------- 名前を付けて保存 (03 §9.7 (a)) ----------

/// a.txt を開いて編集し (dirty)、b.txt も開いて表示を a に戻す。
fn a_dirty_and_b() -> (Harness, DocumentId, DocumentId) {
    let mut h = opened(&[("/w/a.txt", b"alpha\n"), ("/w/b.txt", b"beta\n")]);
    h.open("/w/b.txt");
    h.run_jobs();
    let b = h.doc_id();
    h.open("/w/a.txt");
    h.run_jobs();
    let a = h.doc_id();
    h.text("x");
    (h, a, b)
}

// verifies: FR-104
#[test]
fn 表の_1_今の文書と同じファイルへの名前を付けて保存は上書き保存になる() {
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_link("/w/link.txt", "/w/a.txt");
    save_as(&mut h, "/w/link.txt");
    assert_eq!(h.overlay(), "", "上書きの確認を出さない");
    assert_eq!(disk(&h, "/w/a.txt").as_deref(), Some("xalpha\n"));
    assert_eq!(h.doc_id(), a);
    assert_eq!(h.doc_path().as_deref(), Some("/w/a.txt"), "パスは変えない");
    assert!(!h.is_dirty());
}

// verifies: FR-104
#[test]
fn 表の_1_ハードリンクも今の文書と同じファイル() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.add_hardlink("/w/hard.txt", "/w/a.txt");
    save_as(&mut h, "/w/hard.txt");
    // 上書きの確認ではなく、今の文書 (a) の上書き保存のハードリンクの確認 (03 §9.3 手順 7)
    assert_eq!(h.overlay(), "dialog");
    h.answer(0); // リンクを保って直接上書き
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(disk(&h, "/w/a.txt").as_deref(), Some("xalpha\n"));
    assert_eq!(h.doc_path().as_deref(), Some("/w/a.txt"));
}

// verifies: FR-104, FR-116
#[test]
fn 表の_1_読み取り専用の文書は同じファイルに保存しない() {
    let mut h = opened(&[("/w/a.txt", b"alpha\n")]);
    h.fs.add_link("/w/link.txt", "/w/a.txt");
    h.open("/w/a.txt");
    h.run_jobs();
    h.cmd("file.toggle_read_only");
    save_as(&mut h, "/w/link.txt");
    assert!(has_msg(&h, "msg.save_as.same_file_read_only"));
    assert_eq!(h.overlay(), "prompt", "名前の入力に戻る");
    assert_eq!(disk(&h, "/w/a.txt").as_deref(), Some("alpha\n"));
}

// verifies: FR-104
#[test]
fn 表の_1b_ほかの開いている文書と同じファイルには保存しない() {
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_link("/w/to-b.txt", "/w/b.txt");
    save_as(&mut h, "/w/to-b.txt");
    assert!(has_msg(&h, "msg.save_as.open_in_other"));
    assert_eq!(h.overlay(), "prompt", "名前の入力に戻る");
    assert_eq!(disk(&h, "/w/b.txt").as_deref(), Some("beta\n"), "書かない");
    assert_eq!(h.doc_id(), a);
    assert!(h.is_dirty());
    // 構文的に同じパスでも同じ (照合のジョブを経る)
    h.keys("esc");
    save_as(&mut h, "/w/b.txt");
    assert!(has_msg(&h, "msg.save_as.open_in_other"));
    assert_eq!(disk(&h, "/w/b.txt").as_deref(), Some("beta\n"));
}

// verifies: FR-104
#[test]
fn 表の_1b_識別を信頼しない_fs_でもリンクは_canonical_で見つける() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.set_file_id_unique(false);
    h.fs.add_link("/w/to-b.txt", "/w/b.txt");
    save_as(&mut h, "/w/to-b.txt");
    assert!(has_msg(&h, "msg.save_as.open_in_other"));
    assert_eq!(disk(&h, "/w/b.txt").as_deref(), Some("beta\n"));
}

// verifies: FR-104
#[test]
fn 表の_1b_まだ無いファイルとして開いた文書とも比べる() {
    let (mut h, _, _) = a_dirty_and_b();
    h.open("/w/new.txt");
    h.run_jobs();
    h.cmd("buffer.prev");
    save_as(&mut h, "/w/new.txt");
    assert!(has_msg(&h, "msg.save_as.open_in_other"));
    assert!(disk(&h, "/w/new.txt").is_none(), "作らない");
}

// verifies: FR-104
#[test]
fn 表の_2_無いファイルには保存して文書のパスを替える() {
    let (mut h, a, _) = a_dirty_and_b();
    save_as(&mut h, "/w/c.txt");
    assert_eq!(h.overlay(), "");
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("xalpha\n"));
    assert_eq!(h.doc_id(), a);
    assert_eq!(h.doc_path().as_deref(), Some("/w/c.txt"));
    // 保存した後は新しいパスで同じファイルと判定する (canonical を替えた)。古いパスは別のファイル
    h.fs.add_link("/w/to-c.txt", "/w/c.txt");
    h.open("/w/to-c.txt");
    h.run_jobs();
    assert_eq!(h.doc_id(), a, "c の文書に切り替える");
    h.open("/w/a.txt");
    h.run_jobs();
    assert_ne!(h.doc_id(), a, "a.txt は別の文書として開く");
}

// verifies: FR-106
#[test]
fn 表の_3_ダングリングのリンクには保存しない() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.add_link("/w/dangling.txt", "/w/nowhere.txt");
    save_as(&mut h, "/w/dangling.txt");
    assert!(has_msg(&h, "msg.save.dangling_link"));
    assert_eq!(h.overlay(), "prompt");
    assert!(disk(&h, "/w/nowhere.txt").is_none());
}

// verifies: FR-104
#[test]
fn 表の_4_フォルダには保存しない() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.add_dir("/w/dir");
    save_as(&mut h, "/w/dir");
    assert!(has_msg(&h, "msg.save_as.not_a_file"));
    assert_eq!(h.overlay(), "prompt");
}

// verifies: FR-104, FR-116
#[test]
fn 表の_5_読み取り専用属性のファイルは上書きの確認の前に拒む() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.add_file("/w/ro.txt", b"keep\n");
    h.fs.set_readonly("/w/ro.txt", true);
    save_as(&mut h, "/w/ro.txt");
    assert!(has_msg(&h, "msg.save_as.target_read_only"));
    assert_eq!(h.overlay(), "prompt");
    assert_eq!(disk(&h, "/w/ro.txt").as_deref(), Some("keep\n"));
}

// verifies: FR-104
#[test]
fn 表の_6_既存のファイルは上書きを確かめる() {
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_file("/w/c.txt", b"old\n");
    save_as(&mut h, "/w/c.txt");
    assert_eq!(h.overlay(), "dialog", "上書きの確認");
    assert_eq!(
        disk(&h, "/w/c.txt").as_deref(),
        Some("old\n"),
        "確認の前は書かない"
    );
    // 既定は「別の名前を入力」: Enter で名前の入力に戻る (入力した名前は残す)
    h.keys("enter");
    assert_eq!(h.overlay(), "prompt");
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("old\n"));
    // Esc も名前の入力に戻る
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog");
    h.keys("esc");
    assert_eq!(h.overlay(), "prompt");
    // 上書き
    h.keys("enter");
    h.run_jobs();
    h.keys("o");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("xalpha\n"));
    assert_eq!(h.doc_id(), a);
    assert_eq!(h.doc_path().as_deref(), Some("/w/c.txt"));
}

// verifies: FR-104, FR-117
#[test]
fn 表の_6_確認の間に変わったファイルは上書きせずに尋ねる() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.add_file("/w/c.txt", b"old\n");
    save_as(&mut h, "/w/c.txt");
    assert_eq!(h.overlay(), "dialog");
    // 確認の間に他のプログラムが書き換えた
    h.advance_ms(10);
    h.fs.add_file("/w/c.txt", b"changed by other\n");
    h.keys("o");
    h.run_jobs();
    assert_eq!(
        disk(&h, "/w/c.txt").as_deref(),
        Some("changed by other\n"),
        "照合で止めて書かない"
    );
    assert_eq!(h.overlay(), "dialog", "外部変更の確認");
}

// verifies: FR-104
#[test]
fn 表の_7_調べられない保存先には保存しない() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.fail_next(
        eve_testkit::FsOp::Metadata,
        eve_core::FsErrorKind::PermissionDenied,
    );
    save_as(&mut h, "/w/c.txt");
    assert!(has_msg(&h, "msg.save.permission_denied"));
    assert_eq!(h.overlay(), "prompt");
    assert!(disk(&h, "/w/c.txt").is_none());
}

// verifies: FR-104
#[test]
fn 照合を待つ間の_esc_は名前の入力に戻る() {
    let (mut h, _, _) = a_dirty_and_b();
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    assert_eq!(
        h.overlay(),
        "",
        "照合のジョブを待つ間はオーバーレイを出さない"
    );
    h.keys("esc");
    assert_eq!(h.overlay(), "prompt");
    h.run_jobs();
    assert!(disk(&h, "/w/c.txt").is_none(), "照合の結果は捨てる");
    assert_eq!(h.overlay(), "prompt");
}

// verifies: FR-104
#[test]
fn 照合を待つ間の終了と開くは無視する() {
    let (mut h, a, _) = a_dirty_and_b();
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    h.cmd("app.quit");
    assert_eq!(h.overlay(), "", "終了の連鎖 (未保存の確認) を始めない");
    assert!(h.quit_code().is_none());
    h.cmd_with("file.open", Args::new().with_str("path", "/w/b.txt"));
    h.cmd("file.new");
    assert!(has_msg(&h, "info.save.in_progress"));
    h.run_jobs();
    assert_eq!(
        disk(&h, "/w/c.txt").as_deref(),
        Some(
            "xalpha
"
        )
    );
    assert_eq!(h.doc_id(), a, "開く・新規は無視したので a のまま");
    assert_eq!(docs(&h), 2);
}

// verifies: FR-104
#[test]
fn 照合の結果はほかのオーバーレイの間は保留する() {
    let (mut h, _, _) = a_dirty_and_b();
    h.fs.add_file(
        "/w/c.txt", b"old
",
    );
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    // 照合を待つ間にパレットを開く
    h.cmd("ui.palette_open");
    assert_eq!(h.overlay(), "palette");
    h.run_jobs();
    assert_eq!(
        h.overlay(),
        "palette",
        "上書きの確認でパレットを置き換えない"
    );
    h.keys("esc");
    assert_eq!(h.overlay(), "dialog", "閉じた後に上書きの確認を出す");
    h.keys("o");
    h.run_jobs();
    assert_eq!(
        disk(&h, "/w/c.txt").as_deref(),
        Some(
            "xalpha
"
        )
    );
}

// verifies: FR-104
#[test]
fn 照合を待つ間の_esc_は表示していない文書の読込より先に照合を取り消す() {
    let (mut h, a, _) = a_dirty_and_b();
    h.fs.add_file(
        "/w/d.txt", b"delta
",
    );
    // 表示していない文書の読込 (d) と照合を同時に待つ
    h.editor
        .state_mut()
        .open_path("/w/d.txt", eve_core::OpenOptions::default());
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    h.keys("esc");
    assert_eq!(h.overlay(), "prompt", "照合を取り消して名前の入力に戻る");
    h.keys("esc");
    h.run_jobs();
    assert!(disk(&h, "/w/c.txt").is_none());
    assert_eq!(docs(&h), 3, "d の読込は取り消していない: {:?}", paths(&h));
    assert!(h.editor.state().document(a).is_some());
}

// verifies: FR-104
#[test]
fn 照合を待つ間の上書き保存は無視する() {
    let (mut h, _, _) = a_dirty_and_b();
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    h.cmd("file.save");
    assert!(has_msg(&h, "info.save.in_progress"));
    h.run_jobs();
    assert_eq!(
        disk(&h, "/w/a.txt").as_deref(),
        Some("alpha\n"),
        "a は書かない"
    );
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("xalpha\n"));
}

// verifies: FR-104
#[test]
fn 照合から保存の完了までに同じファイルの文書が開かれたら知らせて両方残す() {
    let (mut h, a, _) = a_dirty_and_b();
    // 照合 (c は無い) の結果が届く前に c を開く: 開く読込の識別 (新規のファイル) は保存の前に届くので、その時点では
    // 別のファイル。保存が終わった時点で同じファイルになる
    h.cmd_with("file.save_as", Args::new().with_str("path", "/w/c.txt"));
    h.editor
        .state_mut()
        .open_path("/w/c.txt", eve_core::OpenOptions::default());
    h.run_jobs();
    assert_eq!(disk(&h, "/w/c.txt").as_deref(), Some("xalpha\n"));
    assert!(
        has_msg(&h, "msg.save_as.now_open_in_other"),
        "{:?}",
        h.messages()
    );
    assert!(
        !has_msg(&h, "msg.save_as.open_in_other"),
        "拒んだときの文言とは分ける"
    );
    assert!(
        h.editor.state().document(a).is_some(),
        "保存した文書は閉じない"
    );
}
