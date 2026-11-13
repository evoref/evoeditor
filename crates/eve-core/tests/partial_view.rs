//! 先行表示の core と表示 (docs/03 §9.2.1、v0.4 の M25 の (2)、01 FR-122 AC-3 / AC-4): 読込が 200 ms を過ぎても終わらない
//! ときに届く固定の先頭を、読み取り専用の `Partial` の文書として表示し、完了で全文へ差し替える。遅い読込は `MemFs` の
//! チャンクごとに `ManualClock` を進めて作り、`Harness::run_jobs_until_partial` で先頭を受けた所で止める。

use eve_config::Config;
use eve_core::{
    Args, CommandInvocation, CommandResult, ConfirmAsk, DocumentId, InitialFile, InvocationSource,
    JobOutput, JobResult, LoadState, Millis, RejectReason, TextFormat,
};
use eve_testkit::Harness;
use eve_text::{Text, TextRead, is_grapheme_boundary};
use eve_view::{
    Answer, ClipboardText, Effect, LoadStateView, Modifiers, MouseKind, Query, QueryError,
    StatusKind,
};

/// 12 バイトの行を `n` 行 (`line 000000\n` …)。
fn lines(n: usize) -> Vec<u8> {
    (0..n)
        .flat_map(|i| format!("line {i:06}\n").into_bytes())
        .collect()
}

/// 遅い読込 (64 バイトのチャンクごとに 100 ms)。2 つ目のチャンクの区切りで 200 ms を過ぎ、先頭が届く。
fn slow(h: &Harness) {
    h.fs.set_chunk_size(64);
    h.fs.set_read_delay(Some((h.clock.clone(), Millis(100))));
}

/// 無題のバッファで起動し、`path` に `data` を置いた遅い読込の Harness。
fn slow_harness(path: &str, data: &[u8]) -> Harness {
    let h = Harness::new().with_file(path, data);
    slow(&h);
    h
}

fn doc_of(h: &Harness, path: &str) -> DocumentId {
    let Ok(Answer::Documents(ds)) = h.query(&Query::Documents) else {
        panic!("文書の一覧");
    };
    ds.iter()
        .find(|d| d.path.as_deref() == Some(path))
        .unwrap_or_else(|| panic!("{path} の文書"))
        .doc
}

fn load_state(h: &Harness, doc: DocumentId) -> LoadState {
    h.editor.state().document(doc).expect("文書").load_state
}

fn text_of(h: &Harness, doc: DocumentId) -> String {
    h.editor
        .state()
        .document(doc)
        .expect("文書")
        .to_string_lossy()
}

fn revision(h: &Harness, doc: DocumentId) -> eve_view::Revision {
    h.editor.state().document(doc).expect("文書").revision()
}

fn last_message(h: &Harness) -> String {
    h.messages().last().cloned().unwrap_or_default()
}

fn status_item(h: &mut Harness, kind: StatusKind) -> Option<String> {
    h.frame()
        .status
        .items
        .iter()
        .find(|i| i.kind == kind)
        .map(|i| i.long.spans.iter().map(|s| s.text.as_str()).collect())
}

/// 主カーソルの位置 (バイト)。
fn head(h: &Harness) -> usize {
    h.selection_range().1
}

/// 先頭を表示した状態まで進めた Harness と、読み込んでいる文書。
fn partial_shown(path: &str, data: &[u8]) -> (Harness, DocumentId) {
    let mut h = slow_harness(path, data);
    h.open(path);
    assert!(h.run_jobs_until_partial(), "先頭が届く");
    let doc = doc_of(&h, path);
    (h, doc)
}

// ---------- 表示とステータスバー ----------

// 01 §5.5 の FR-122 AC-3
#[test]
fn 先頭が読み取り専用で表示され_ステータスバーに先頭のみと出る() {
    let data = lines(100);
    let (mut h, doc) = partial_shown("/w/a.txt", &data);
    assert_eq!(h.doc_id(), doc, "先行表示の時点で切り替わる");
    assert_eq!(load_state(&h, doc), LoadState::Partial);
    let shown = text_of(&h, doc);
    assert!(!shown.is_empty() && shown.len() < data.len());
    assert!(data.starts_with(shown.as_bytes()), "全文の接頭辞");
    assert!(shown.ends_with('\n'), "行の区切りで切る");
    assert!(h.frame_contains("line 000000"));
    let loading = status_item(&mut h, StatusKind::Loading).expect("読込中の項目");
    assert!(loading.starts_with("[先頭のみ "), "{loading}");
    // 問い合わせ: 状態は Partial、本文と選択は NotLoaded
    let Ok(Answer::Documents(ds)) = h.query(&Query::Documents) else {
        panic!()
    };
    let info = ds.iter().find(|d| d.doc == doc).unwrap();
    assert_eq!(info.load_state, LoadStateView::Partial);
    assert!(!info.dirty);
    assert_eq!(info.read_only, Vec::new(), "読込中は理由に入れない");
    assert_eq!(
        h.query(&Query::DocumentText { doc, range: None }),
        Err(QueryError::NotLoaded(doc))
    );
    let view = info.views[0];
    assert_eq!(
        h.query(&Query::Selection { view }),
        Err(QueryError::NotLoaded(doc))
    );
    // 完了で全文になり、読込中の項目は消える
    h.finish_held_jobs();
    assert_eq!(load_state(&h, doc), LoadState::Ready);
    assert_eq!(h.doc_text().as_bytes(), data);
    assert_eq!(status_item(&mut h, StatusKind::Loading), None);
}

#[test]
fn 英語の文言でも先頭のみの項目が出る() {
    let mut h = Harness::builder()
        .locale(eve_config::Locale::En)
        .file("/w/a.txt", &lines(100))
        .build();
    slow(&h);
    h.open("/w/a.txt");
    assert!(h.run_jobs_until_partial());
    let loading = status_item(&mut h, StatusKind::Loading).unwrap();
    assert!(loading.starts_with("[First part "), "{loading}");
}

#[test]
fn 行番号の欄の幅はファイルの大きさから推定し_完了で確定する() {
    // 1 万行 (12 万バイト)。先頭は 10 行ほどなので、先頭の行数だけなら 2 桁 + 1
    let data = lines(10_000);
    let (mut h, _) = partial_shown("/w/a.txt", &data);
    let w = h.frame().panes[0].gutter_width;
    assert_eq!(w, 6, "推定の行数 (約 1 万) の 5 桁 + 1");
    h.finish_held_jobs();
    assert_eq!(
        h.frame().panes[0].gutter_width,
        6,
        "完了: 10001 行 (末尾の空行を含む)"
    );
}

// ---------- できない操作 ----------

// 01 §5.5 の FR-122 AC-3
#[test]
fn 編集と_undo_は読込中として拒否する() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    let before = text_of(&h, doc);
    h.clear_log();
    h.text("x");
    assert_eq!(last_message(&h), "msg.edit.read_only_loading");
    assert!(h.frame_contains("読込中のため編集できません"));
    assert!(
        !h.frame_contains("名前を付けて保存"),
        "使えない案内 (名前を付けて保存・開き直す) を出さない"
    );
    h.keys("enter");
    h.cmd("edit.undo");
    h.paste("y");
    assert_eq!(text_of(&h, doc), before);
    assert!(!h.is_dirty());
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 検索_置換_行へ移動_すべて選択_末尾まで選択は_partial_unavailable_で拒否する() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    let sel = h.selection_range();
    for (i, id) in [
        "search.open",
        "search.replace_open",
        "search.next",
        "search.prev",
        "search.replace",
        "search.replace_all",
        "cursor.goto_line",
        "select.all",
        "select.extend_to_doc_end",
    ]
    .into_iter()
    .enumerate()
    {
        // 前の拒否の警告を Esc で消してから (消す Esc は読込を取り消さない、04 §7.5)
        if i > 0 {
            h.keys("esc");
        }
        h.clear_log();
        assert_eq!(
            h.cmd(id),
            CommandResult::Rejected(RejectReason::NotAvailable),
            "{id}"
        );
        assert_eq!(h.messages(), vec!["msg.open.partial_unavailable"], "{id}");
        assert_eq!(h.overlay(), "", "{id}: 入力を開かない");
        assert!(
            !h.editor
                .state()
                .context_value("search.active")
                .is_some_and(|v| matches!(v, eve_core::CtxValue::Bool(true))),
            "{id}: 検索バーを開かない"
        );
    }
    assert_eq!(h.selection_range(), sel);
    assert!(h.frame_contains("読込中のため使えません"));
    // キー (Ctrl+F・Ctrl+A・Ctrl+Shift+End) とマクロ・外部クライアントの呼び出しでも同じ
    h.keys("esc");
    h.clear_log();
    h.keys("ctrl+f ctrl+a ctrl+shift+end");
    assert_eq!(
        h.messages(),
        vec!["msg.open.partial_unavailable"],
        "続く拒否は 1 回にまとめる"
    );
    for src in [InvocationSource::Macro, InvocationSource::External] {
        let r = h.editor.execute(CommandInvocation::new("select.all", src));
        assert_eq!(r, CommandResult::Rejected(RejectReason::NotAvailable));
    }
    assert_eq!(load_state(&h, doc), LoadState::Partial);
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 本文の無い新しく開く読込の_loading_でも拒否する() {
    // 起動時の最初のパス: 読込中の文書を最初から表示している
    let mut h = Harness::builder()
        .file("/w/a.txt", &lines(10))
        .defer_start()
        .build();
    h.editor.open_initial(Some("/w/a.txt"), false);
    h.started();
    assert_eq!(load_state(&h, h.doc_id()), LoadState::Loading);
    h.clear_log();
    assert_eq!(
        h.cmd("search.open"),
        CommandResult::Rejected(RejectReason::NotAvailable)
    );
    assert_eq!(h.messages(), vec!["msg.open.partial_unavailable"]);
    h.run_jobs();
    assert_eq!(
        h.cmd("search.open"),
        CommandResult::Done,
        "完了の後は使える"
    );
}

// 01 §5.5 の FR-122 AC-3 / AC-4
#[test]
fn 再読込の間は旧本文で検索できる() {
    let mut h = Harness::new().with_file("/w/a.txt", b"hello\nworld\n");
    h.open("/w/a.txt");
    h.run_jobs();
    h.cmd("file.reload");
    assert_eq!(load_state(&h, h.doc_id()), LoadState::Loading);
    h.clear_log();
    assert_eq!(h.cmd("search.open"), CommandResult::Done);
    assert!(
        !h.messages()
            .contains(&"msg.open.partial_unavailable".to_owned())
    );
    assert_ne!(
        h.cmd("select.all"),
        CommandResult::Rejected(RejectReason::NotAvailable)
    );
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 名前を付けて保存は名前を尋ねずに拒否し_ほかの保存の入口も書かない() {
    let data = lines(100);
    let (mut h, doc) = partial_shown("/w/a.txt", &data);
    let files = h.fs.paths();
    h.clear_log();
    h.cmd("file.save_as");
    assert_eq!(h.overlay(), "", "名前の入力を開かない");
    assert_eq!(h.messages(), vec!["info.reload.busy"]);
    assert_eq!(
        h.cmd_with("file.save_as", Args::new().with_str("path", "/w/b.txt")),
        CommandResult::Rejected(RejectReason::NotAvailable)
    );
    h.cmd("file.save");
    h.cmd("file.save_all");
    h.cmd_with(
        "file.set_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    h.cmd_with("file.set_line_ending", Args::new().with_str("eol", "crlf"));
    h.cmd("file.reload");
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    // 駆動 API の挿入も拒否する
    let r = h.editor.execute(
        CommandInvocation::new("edit.insert_text", InvocationSource::External)
            .with_args(Args::new().with_str("text", "z")),
    );
    assert!(matches!(r, CommandResult::Rejected(_)));
    h.run_jobs_until_partial();
    assert_eq!(h.fs.paths(), files, "何も書かない");
    assert_eq!(h.fs_read("/w/b.txt"), None);
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), data);
    assert_eq!(load_state(&h, doc), LoadState::Partial);
}

// 01 §5.5 の FR-122 AC-4
#[test]
fn 先行表示の文書はスワップと緊急退避の対象にならない() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(100));
    // 無題のバッファを編集してから開く (切り替え元は dirty なので、そのまま残る)
    h.text("dirty");
    let untitled = h.doc_id();
    slow(&h);
    h.open("/w/a.txt");
    assert!(h.run_jobs_until_partial());
    let doc = doc_of(&h, "/w/a.txt");
    assert_eq!(h.doc_id(), doc);
    assert!(
        h.editor.emergency_snapshot(doc).is_none(),
        "緊急退避の対象外"
    );
    assert!(
        h.editor.emergency_snapshot(untitled).is_some(),
        "編集した文書は対象"
    );
    h.advance_ms(30_000);
    h.run_jobs_until_partial();
    assert_eq!(h.fs.swap_of(doc), None, "先頭はスワップに書かない");
    assert!(h.fs.swap_of(untitled).is_some());
    // 終了時の保存 (`app.save_and_quit`): 表示中の先頭は書かずに終了の連鎖へ (dirty な無題の確認が出る)
    h.cmd("app.save_and_quit");
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), lines(100), "先頭を書かない");
    assert!(h.quit_code().is_none(), "dirty な無題の確認が出る");
}

// ---------- できる操作 ----------

// 01 §5.5 の FR-122 AC-3
#[test]
fn スクロール_カーソルの移動_選択とコピー_バッファの切替ができる() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    h.keys("down down right");
    assert_eq!(head(&h), 12 * 2 + 1);
    h.keys("shift+down");
    assert_eq!(h.selection_range(), (25, 37));
    h.keys("ctrl+c");
    let copied: Vec<_> = h
        .effects()
        .iter()
        .filter_map(|e| match e {
            Effect::SetClipboard { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        copied,
        vec![ClipboardText::Inline("ine 000002\nl".to_owned())]
    );
    // ホイールのスクロール (画面を小さくして、先頭の中で動かす)
    h.resize(40, 6);
    h.mouse(MouseKind::WheelDown, Modifiers::NONE, 0, 0);
    assert!(
        h.frame().panes[0].rows[0]
            .spans
            .iter()
            .any(|s| !s.text.contains("line 000000"))
    );
    // バッファの切替: 元の無題へ戻して、また先頭へ
    h.cmd("buffer.next");
    assert_ne!(h.doc_id(), doc);
    h.cmd("buffer.next");
    assert_eq!(h.doc_id(), doc);
    // パレットとヘルプは開いて閉じられる
    h.cmd("ui.palette_open");
    assert_eq!(h.overlay(), "palette");
    h.keys("esc");
    h.cmd("help.open");
    assert_eq!(h.overlay(), "help");
    h.keys("esc");
    assert_eq!(load_state(&h, doc), LoadState::Partial);
}

#[test]
fn バッファ一覧に先行表示の文書が載る() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(100));
    h.text("x");
    slow(&h);
    h.open("/w/a.txt");
    assert!(h.run_jobs_until_partial());
    // 元の文書へ戻しても、一覧から先頭の文書を選べる
    h.cmd("buffer.prev");
    h.cmd("buffer.list");
    assert_eq!(h.overlay(), "buffer_list");
    let list = format!("{:?}", h.frame().overlays);
    assert_eq!(
        list.matches("ListItem {").count(),
        2,
        "無題と先頭の文書の 2 件"
    );
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 先頭の末尾より先へは動かず_partial_end_を_1_回だけ出す() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    let len = text_of(&h, doc).len();
    h.clear_log();
    h.keys("ctrl+end");
    assert_eq!(head(&h), len, "先頭の末尾に止まる");
    assert_eq!(h.messages(), vec!["info.open.partial_end"]);
    assert!(h.frame_contains("だけを表示しています"));
    h.keys("down pagedown right");
    assert_eq!(head(&h), len);
    assert_eq!(h.messages().len(), 1, "同じ文書の先行表示の間は 1 回だけ");
}

#[test]
fn 末尾の行での下と_pagedown_でも知らせる() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    let last_line = text_of(&h, doc).lines().count();
    h.clear_log();
    // 末尾の行の手前までは知らせない
    for _ in 0..last_line {
        h.keys("down");
    }
    assert_eq!(h.messages(), Vec::<String>::new());
    h.keys("down");
    assert_eq!(h.messages(), vec!["info.open.partial_end"]);
    let (mut h, _) = partial_shown("/w/a.txt", &lines(100));
    h.clear_log();
    h.keys("pagedown pagedown pagedown");
    assert_eq!(h.messages(), vec!["info.open.partial_end"]);
}

// ---------- 全文への差し替え ----------

// 01 §5.5 の FR-122 AC-3
#[test]
fn 完了でカーソル_選択_スクロールを引き継ぎ_revision_は続き() {
    let data = lines(100);
    let mut h = Harness::builder()
        .size(40, 6)
        .file("/w/a.txt", &data)
        .build();
    slow(&h);
    let CommandResult::Pending(_) = h.open("/w/a.txt") else {
        panic!()
    };
    let doc = doc_of(&h, "/w/a.txt");
    let r0 = revision(&h, doc);
    assert!(h.run_jobs_until_partial());
    let r1 = revision(&h, doc);
    assert!(r1 > r0, "先頭は Loading の文書の続き");
    h.keys("down down down down down down right shift+right shift+right");
    let sel = h.selection_range();
    let top_before = h.frame().panes[0].rows[0].spans.clone();
    let view = h.editor.state().window_view(h.window()).unwrap();
    let scroll = h.editor.state().view(view).unwrap().scroll;
    h.finish_held_jobs();
    assert_eq!(load_state(&h, doc), LoadState::Ready);
    assert!(revision(&h, doc) > r1, "全文は先頭の続き");
    assert_eq!(h.selection_range(), sel, "選択を引き継ぐ");
    assert_eq!(h.editor.state().view(view).unwrap().scroll, scroll);
    assert_eq!(
        h.frame().panes[0].rows[0].spans,
        top_before,
        "スクロールを引き継ぐ"
    );
    // 上下の移動の目標の列も引き継ぐ
    h.keys("down");
    assert_eq!(head(&h), 7 * 12 + 3);
}

#[test]
fn crlf_の文書でも位置を引き継ぐ() {
    let data = b"abc\r\n".repeat(200);
    let (mut h, doc) = partial_shown("/w/c.txt", &data);
    let shown = text_of(&h, doc);
    assert!(shown.ends_with("\r\n"), "CRLF を分けない");
    h.keys("ctrl+end");
    let at = head(&h);
    assert_eq!(at, shown.len());
    h.finish_held_jobs();
    assert_eq!(head(&h), at, "次の行の行頭のまま");
    let t = Text::from(h.doc_text().as_str());
    assert_eq!(t.line_to_byte(t.byte_to_line(eve_text::ByteOff(at))).0, at);
}

/// `job` に先頭 `head` を渡す (形式は先頭から数える)。
fn send_head(h: &mut Harness, job: eve_view::JobId, head: &str) {
    let text = Text::from(head);
    let mut format = TextFormat::new_document();
    format.eol_stats = eve_core::EolStats::count(&text);
    h.editor.on_job_result(JobOutput {
        job,
        body: JobResult::Partial { head: text, format },
    });
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 完了で書記素の途中に来た位置は境界へ寄る() {
    // 全文は「e + 結合アキュート」。先頭は e だけ (末尾の書記素に、全文では結合する文字が続く)
    let full = "xe\u{301}y\n";
    let mut h = Harness::new().with_file("/w/g.txt", full.as_bytes());
    let CommandResult::Pending(job) = h.open("/w/g.txt") else {
        panic!()
    };
    send_head(&mut h, job, "xe");
    let doc = doc_of(&h, "/w/g.txt");
    assert_eq!(h.doc_id(), doc);
    h.keys("end");
    assert_eq!(head(&h), 2);
    h.run_jobs();
    assert_eq!(h.doc_text(), full);
    let at = head(&h);
    assert!(
        is_grapheme_boundary(&Text::from(full), eve_text::ByteOff(at)),
        "{at}"
    );
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 判定のやり直しの先頭は差し替えて位置を先頭に戻す() {
    let mut h = Harness::new().with_file("/w/a.txt", b"bbb\nbbb\nbbb\n");
    let CommandResult::Pending(job) = h.open("/w/a.txt") else {
        panic!()
    };
    send_head(&mut h, job, "aaa\naaa\n");
    let doc = doc_of(&h, "/w/a.txt");
    let r1 = revision(&h, doc);
    h.keys("down right");
    assert_eq!(head(&h), 5);
    send_head(&mut h, job, "bbb\nbbb\n");
    assert_eq!(text_of(&h, doc), "bbb\nbbb\n");
    assert!(revision(&h, doc) > r1, "続きの Revision");
    assert_eq!(h.selection_range(), (0, 0), "先頭に戻す");
    assert_eq!(h.doc_id(), doc, "表示は変えない");
    h.run_jobs();
    assert_eq!(h.doc_text(), "bbb\nbbb\nbbb\n");
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 長大行の覚えを全文に持ち越さない() {
    // 閾値 256 バイトの長大行。先頭 (行の途中で切る) で桁の索引を作った後、完了の全文で行末の桁が全文の値になる
    let config = Config {
        long_line_threshold: 256,
        ..Config::default()
    };
    let line = "あいうえおかきくけこ".repeat(40); // 1200 バイト、400 桁 (幅 2 で 800 列)
    let mut h = Harness::builder()
        .config(config)
        .file("/w/l.txt", line.as_bytes())
        .build();
    let CommandResult::Pending(job) = h.open("/w/l.txt") else {
        panic!()
    };
    let head_text = &line[..300];
    let text = Text::from(head_text);
    let mut format = TextFormat::new_document();
    format.long_line_threshold = 256;
    format.long_lines = 1;
    h.editor.on_job_result(JobOutput {
        job,
        body: JobResult::Partial { head: text, format },
    });
    h.keys("end");
    assert!(h.frame_contains("101"), "先頭の行末 (100 文字の後ろ)");
    h.run_jobs();
    h.keys("end");
    assert_eq!(head(&h), line.len());
    let pos = status_item(&mut h, StatusKind::Position).unwrap();
    assert!(pos.contains("401"), "全文の行末の桁: {pos}");
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 起動時の_行_は完了の時点で移る() {
    let mut h = Harness::builder()
        .file("/w/a.txt", &lines(100))
        .defer_start()
        .build();
    slow(&h);
    h.editor.open_initial_files(
        &[InitialFile {
            path: "/w/a.txt".into(),
            line: Some(50),
            col: None,
        }],
        false,
        None,
    );
    h.started();
    assert!(h.run_jobs_until_partial());
    assert_eq!(head(&h), 0, "先行表示の間は移らない");
    h.keys("down");
    h.finish_held_jobs();
    assert_eq!(
        head(&h),
        49 * 12,
        "完了で 50 行目へ (先行表示の間の移動より優先)"
    );
}

// ---------- 確認・失敗・取消し・閉じる ----------

#[test]
fn 先行表示の後の確認は先頭を捨てて_loading_に戻す() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(10));
    let CommandResult::Pending(job) = h.open("/w/a.txt") else {
        panic!()
    };
    send_head(&mut h, job, "line 000000\nline 000001\n");
    let doc = doc_of(&h, "/w/a.txt");
    h.keys("down");
    let r1 = revision(&h, doc);
    h.editor.on_job_result(JobOutput {
        job,
        body: JobResult::NeedsConfirm {
            ask: ConfirmAsk::HugeFile { len: 1 << 30 },
        },
    });
    assert_eq!(load_state(&h, doc), LoadState::Loading);
    assert_eq!(text_of(&h, doc), "");
    assert!(revision(&h, doc) > r1);
    assert_eq!(h.selection_range(), (0, 0));
    assert_eq!(h.overlay(), "dialog");
    assert_eq!(
        h.editor.query(&Query::DocumentText { doc, range: None }),
        Err(QueryError::NotLoaded(doc))
    );
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 失敗では先頭を閉じ_無題のバッファを同じ番号のまま表示に戻す() {
    let data = lines(100);
    let mut h = slow_harness("/w/a.txt", &data);
    let untitled = h.doc_id();
    h.fs.read_fail_after(Some(600));
    h.open("/w/a.txt");
    h.run_jobs();
    assert_eq!(h.doc_id(), untitled, "開く前の無題のバッファ");
    assert_eq!(h.editor.state().documents_len(), 1, "新しい無題を作らない");
    assert!(h.frame_contains("無題-1"));
    assert!(h.messages().iter().any(|m| m.starts_with("msg.open")));
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 取消しでは先頭を閉じ_開く前の文書を表示する() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(100));
    h.text("keep");
    let before = h.doc_id();
    slow(&h);
    h.open("/w/a.txt");
    assert!(h.run_jobs_until_partial());
    let doc = doc_of(&h, "/w/a.txt");
    assert_eq!(h.doc_id(), doc);
    h.clear_log();
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.doc_id(), before);
    assert_eq!(h.doc_text(), "keep");
    assert!(h.editor.state().document(doc).is_none(), "先頭も捨てる");
    assert!(h.messages().contains(&"info.open.cancelled".to_owned()));
    h.finish_held_jobs();
    assert_eq!(h.editor.state().documents_len(), 1);
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 閉じると確認なしで先頭を捨てる() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    h.cmd("buffer.close");
    assert_eq!(h.overlay(), "", "未保存の確認を出さない");
    assert!(h.editor.state().document(doc).is_none());
    assert_eq!(h.editor.state().documents_len(), 1);
    assert!(
        h.frame_contains("無題-1"),
        "表示から外した無題のバッファへ戻る"
    );
    h.finish_held_jobs();
    assert_eq!(
        h.editor.state().documents_len(),
        1,
        "取り消した読込の結果は捨てる"
    );
}

// ---------- 何も入力していない無題のバッファ ----------

// 01 §5.5 の FR-122 AC-4
#[test]
fn 無題のバッファは先行表示では閉じず_完了で閉じる() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    assert_eq!(h.editor.state().documents_len(), 2, "表示から外すだけ");
    h.finish_held_jobs();
    assert_eq!(h.editor.state().documents_len(), 1, "完了で閉じる");
    assert_eq!(h.doc_id(), doc);
    h.cmd("file.new");
    assert!(h.frame_contains("無題-2"), "番号は再利用しない");
}

#[test]
fn 無題のバッファへ戻していたら完了でも切り替えず閉じない() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    h.cmd("buffer.next");
    let untitled = h.doc_id();
    assert_ne!(untitled, doc);
    h.finish_held_jobs();
    assert_eq!(h.doc_id(), untitled, "切り替え直さない");
    assert_eq!(h.editor.state().documents_len(), 2);
}

#[test]
fn 先行表示の後に無題のバッファへ入力していたら完了でも閉じない() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    h.cmd("buffer.next");
    h.text("typed");
    let untitled = h.doc_id();
    h.cmd("buffer.next");
    assert_eq!(h.doc_id(), doc);
    h.finish_held_jobs();
    assert_eq!(h.editor.state().documents_len(), 2);
    assert_eq!(text_of(&h, untitled), "typed");
}

// ---------- 表示の切り替え ----------

// 01 §5.5 の FR-122 AC-4
#[test]
fn 先行表示で切り替えた後に元の文書へ戻すと完了でも切り替わらない() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(100));
    h.text("x");
    let from = h.doc_id();
    slow(&h);
    h.open("/w/a.txt");
    assert!(h.run_jobs_until_partial());
    let doc = doc_of(&h, "/w/a.txt");
    assert_eq!(h.doc_id(), doc, "先行表示の時点で切り替える");
    h.cmd("buffer.prev");
    assert_eq!(h.doc_id(), from);
    h.finish_held_jobs();
    assert_eq!(h.doc_id(), from, "完了では切り替え直さない");
    assert_eq!(load_state(&h, doc), LoadState::Ready);
}

#[test]
fn 先行表示の前に別の文書へ切り替えていたら切り替えない() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(100));
    h.text("x");
    slow(&h);
    h.open("/w/a.txt");
    h.cmd("file.new");
    let other = h.doc_id();
    assert!(h.run_jobs_until_partial());
    assert_eq!(h.doc_id(), other);
    h.finish_held_jobs();
    assert_eq!(h.doc_id(), other);
}

// 01 §5.5 の FR-122 AC-4
#[test]
fn 名前を付けて保存の入力の間は切り替えを保留し_入力は開いた時点の文書を書く() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(100));
    h.text("mine");
    let from = h.doc_id();
    slow(&h);
    h.open("/w/a.txt");
    h.cmd("file.save_as");
    assert_eq!(h.overlay(), "prompt");
    assert!(h.run_jobs_until_partial());
    let doc = doc_of(&h, "/w/a.txt");
    assert_eq!(h.doc_id(), from, "入力の間は切り替えない");
    assert_eq!(load_state(&h, doc), LoadState::Partial);
    h.text("/w/mine.txt");
    h.keys("enter");
    // 入力を閉じた後に切り替わる。保存したのは入力を開いた時点の文書
    assert_eq!(h.doc_id(), doc, "閉じた後に切り替える");
    h.run_jobs_until_partial();
    assert_eq!(h.fs_read("/w/mine.txt").unwrap(), b"mine");
    h.finish_held_jobs();
    assert_eq!(h.doc_id(), doc);
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), lines(100));
}

#[test]
fn 完了の切り替えもオーバーレイの間は保留する() {
    let mut h = Harness::new().with_file("/w/a.txt", b"small\n");
    h.text("x");
    let from = h.doc_id();
    h.open("/w/a.txt");
    h.cmd("ui.palette_open");
    h.run_jobs();
    assert_eq!(h.doc_id(), from, "パレットの間は切り替えない");
    h.keys("esc");
    assert_eq!(h.doc_text(), "small\n", "閉じた後に切り替える");
}

#[test]
fn 保留の間に別の文書へ切り替えたら閉じた後も切り替えない() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(100));
    h.text("x");
    let from = h.doc_id();
    h.cmd("file.new");
    let other = h.doc_id();
    h.cmd("buffer.prev");
    assert_eq!(h.doc_id(), from);
    slow(&h);
    h.open("/w/a.txt");
    h.cmd("ui.palette_open");
    assert!(h.run_jobs_until_partial());
    assert_eq!(h.doc_id(), from, "パレットの間は保留");
    // パレットから `buffer.next` を実行する (パレットを閉じてから実行され、その後に保留を見直す)
    h.text("buffer.next");
    h.keys("enter");
    assert_eq!(h.doc_id(), other, "利用者が切り替えた");
    h.finish_held_jobs();
    assert_eq!(
        h.doc_id(),
        other,
        "切り替え元を表示していないので、完了でも切り替えない"
    );
}

#[test]
fn 保留はオーバーレイを閉じた時点で条件を満たせば切り替える() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(100));
    h.text("x");
    slow(&h);
    h.open("/w/a.txt");
    h.cmd("help.open");
    assert!(h.run_jobs_until_partial());
    assert_ne!(h.doc_id(), doc_of(&h, "/w/a.txt"));
    h.keys("esc");
    assert_eq!(h.doc_id(), doc_of(&h, "/w/a.txt"));
}

#[test]
fn 起動時の_2_つ目以降のパスは先行表示でも表示を変えない() {
    let mut h = Harness::builder()
        .file("/w/a.txt", &lines(100))
        .file("/w/b.txt", &lines(100))
        .defer_start()
        .build();
    slow(&h);
    h.editor.open_initial_files(
        &[
            InitialFile {
                path: "/w/a.txt".into(),
                line: None,
                col: None,
            },
            InitialFile {
                path: "/w/b.txt".into(),
                line: None,
                col: None,
            },
        ],
        false,
        None,
    );
    h.started();
    let a = doc_of(&h, "/w/a.txt");
    assert!(h.run_jobs_until_partial());
    assert_eq!(h.doc_id(), a, "最初のパスはそのまま先頭を出す");
    assert_eq!(load_state(&h, a), LoadState::Partial);
    // a の残り (完了) を渡し、b の読込の先頭まで進める
    h.finish_held_jobs_until_partial();
    let b = doc_of(&h, "/w/b.txt");
    assert_eq!(load_state(&h, b), LoadState::Partial);
    assert_eq!(h.doc_id(), a, "2 つ目のパスでは切り替えない");
    h.finish_held_jobs();
    assert_eq!(h.doc_id(), a);
}

// ---------- Esc の範囲 ----------

// 01 §5.5 の FR-122 AC-3
#[test]
fn esc_は表示中の文書の読込だけを取り消し_表示していない文書の読込は残す() {
    let mut h = Harness::new()
        .with_file("/w/a.txt", &lines(100))
        .with_file("/w/b.txt", b"bee\n");
    h.text("x");
    slow(&h);
    h.open("/w/a.txt");
    h.open("/w/b.txt"); // 読込は直列: a の後に読む (表示していない文書)
    assert!(h.run_jobs_until_partial());
    let a = doc_of(&h, "/w/a.txt");
    let b = doc_of(&h, "/w/b.txt");
    assert_eq!(h.doc_id(), a);
    h.keys("esc");
    h.fs.set_read_delay(None);
    h.run_jobs();
    assert!(
        h.editor.state().document(a).is_none(),
        "表示中の a は取り消す"
    );
    assert_eq!(text_of(&h, b), "bee\n", "表示していない b は取り消さない");
}

#[test]
fn 表示中の文書が読込中でなければ表示していない文書の新しく開く読込を取り消す() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(10));
    h.text("x");
    h.open("/w/a.txt");
    h.keys("esc");
    h.run_jobs();
    assert!(h.messages().contains(&"info.open.cancelled".to_owned()));
    assert_eq!(h.editor.state().documents_len(), 1);
}

#[test]
fn esc_はほかの文書の再読込を取り消さない() {
    let mut h = Harness::new().with_file("/w/r.txt", b"old\n");
    h.open("/w/r.txt");
    h.run_jobs();
    let r = h.doc_id();
    h.fs.add_file("/w/r.txt", b"new\n");
    h.cmd("file.reload");
    h.cmd("file.new");
    assert_ne!(h.doc_id(), r);
    h.clear_log();
    h.keys("esc");
    h.run_jobs();
    assert_eq!(text_of(&h, r), "new\n", "再読込は続く");
    assert!(!h.messages().contains(&"info.open.cancelled".to_owned()));
}

// ---------- 状態の不変条件 ----------

#[test]
fn 先行表示の間も完了の後も_dirty_にならない() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    assert!(!h.editor.state().document(doc).unwrap().is_dirty());
    h.finish_held_jobs();
    assert!(!h.is_dirty());
    // 完了の後は編集できる
    h.text("z");
    assert!(h.is_dirty());
}

// ---------- 自動の切り替えの連鎖と保留 (レビューの指摘。03 §9.2.1 / §9.8) ----------

fn search_active(h: &Harness) -> bool {
    h.editor
        .state()
        .context_value("search.active")
        .is_some_and(|v| matches!(v, eve_core::CtxValue::Bool(true)))
}

fn dirty(h: &Harness, doc: DocumentId) -> bool {
    h.editor.state().document(doc).expect("文書").is_dirty()
}

// 01 §5.5 の FR-122 AC-3 / AC-4
#[test]
fn 先頭を取り消して元の文書へ戻った後に_続けて開いた読込は表示を奪わない() {
    let mut h = Harness::new()
        .with_file("/w/a.txt", &lines(100))
        .with_file("/w/b.txt", b"bee\n");
    h.text("x");
    let from = h.doc_id();
    slow(&h);
    h.open("/w/a.txt");
    h.open("/w/b.txt"); // 切り替え元は from
    assert!(h.run_jobs_until_partial());
    let a = doc_of(&h, "/w/a.txt");
    assert_eq!(
        h.doc_id(),
        a,
        "先行表示で a へ (b の切り替え元は a に書き換わる)"
    );
    h.keys("esc");
    h.fs.set_read_delay(None);
    h.run_jobs();
    let b = doc_of(&h, "/w/b.txt");
    assert_eq!(h.doc_id(), from, "取り消して元の文書へ戻る");
    assert_eq!(text_of(&h, b), "bee\n", "b は読み終えるが、表示は奪わない");
    h.text("y");
    assert_eq!(text_of(&h, from), "xy", "打鍵は元の文書に入る");
    assert!(!dirty(&h, b));
}

#[test]
fn 開く入力の間に保留した切り替えの後も_入力で開いた文書へ切り替わる() {
    let mut h = Harness::new()
        .with_file("/w/b.txt", b"bee\n")
        .with_file("/w/c.txt", b"sea\n");
    h.text("x");
    let a = h.doc_id();
    h.open("/w/b.txt");
    h.cmd("file.open");
    assert_eq!(h.overlay(), "prompt");
    h.run_jobs(); // b の完了 (入力の間は保留)
    assert_eq!(h.doc_id(), a);
    h.keys("ctrl+a");
    h.text("/w/c.txt");
    h.keys("enter");
    assert_eq!(
        h.doc_text(),
        "bee\n",
        "入力を閉じた後に、保留した b へ切り替える"
    );
    h.run_jobs(); // c の完了: 切り替え元は b に書き換わっている
    assert_eq!(
        h.doc_id(),
        doc_of(&h, "/w/c.txt"),
        "最後に開いた c を表示する"
    );
}

#[test]
fn 検索バーに入力している間は完了の切り替えを保留し_検索語は本文に入らない() {
    let mut h = Harness::new().with_file("/w/b.txt", b"bee\n");
    h.text("x");
    let from = h.doc_id();
    h.open("/w/b.txt");
    h.cmd("search.open");
    h.text("fo");
    h.run_jobs();
    let b = doc_of(&h, "/w/b.txt");
    assert_eq!(h.doc_id(), from, "検索バーの間は切り替えない");
    assert!(search_active(&h));
    h.text("o");
    assert!(!dirty(&h, b), "検索語は本文に入らない");
    assert_eq!(text_of(&h, from), "x");
    h.keys("esc");
    assert!(!search_active(&h));
    assert_eq!(h.doc_id(), b, "検索バーを閉じた後に切り替える");
}

// 01 §5.5 の FR-122 AC-4
#[test]
fn 検索バーに入力している間は先行表示の切り替えも保留する() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(100));
    h.text("x");
    let from = h.doc_id();
    slow(&h);
    h.open("/w/a.txt");
    h.cmd("search.open");
    h.text("lin");
    assert!(h.run_jobs_until_partial());
    let a = doc_of(&h, "/w/a.txt");
    assert_eq!(h.doc_id(), from, "入力の途中で切り替えない");
    assert!(
        search_active(&h),
        "検索バーは開いたまま (入力した語を失わない)"
    );
    h.clear_log();
    h.text("e");
    assert!(
        !h.messages()
            .contains(&"msg.edit.read_only_loading".to_owned()),
        "続きの文字は検索バーに入る"
    );
    h.keys("esc");
    assert_eq!(h.doc_id(), a, "検索バーを閉じた後に先頭へ切り替える");
    assert_eq!(load_state(&h, a), LoadState::Partial);
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 警告を消す_esc_は読込を取り消さず_もう一度の_esc_で取り消す() {
    let (mut h, a) = partial_shown("/w/a.txt", &lines(100));
    h.text("z");
    assert_eq!(last_message(&h), "msg.edit.read_only_loading");
    h.clear_log();
    h.keys("esc");
    assert_eq!(h.messages(), Vec::<String>::new());
    assert!(
        !h.frame_contains("読込中のため編集できません"),
        "警告は消える"
    );
    h.run_jobs();
    assert_eq!(load_state(&h, a), LoadState::Partial, "読込は続く");
    h.keys("esc");
    h.run_jobs();
    assert!(
        h.editor.state().document(a).is_none(),
        "2 回目の Esc で取り消す"
    );
    assert!(h.messages().contains(&"info.open.cancelled".to_owned()));
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn パレットの間の読込の失敗は表示を移さず_選んだコマンドは開く前の文書に効かない() {
    let mut h = Harness::new()
        .with_file("/w/c.txt", b"disk\n")
        .with_file("/w/a.txt", &lines(100));
    h.open("/w/c.txt");
    h.run_jobs();
    let c = h.doc_id();
    h.text("EDIT ");
    slow(&h);
    h.fs.read_fail_after(Some(600));
    h.open("/w/a.txt");
    assert!(h.run_jobs_until_partial());
    let a = doc_of(&h, "/w/a.txt");
    assert_eq!(h.doc_id(), a);
    h.cmd("ui.palette_open");
    h.finish_held_jobs();
    assert_eq!(h.overlay(), "palette");
    assert_eq!(h.doc_id(), a, "パレットの間は表示を移さない");
    assert_eq!(load_state(&h, a), LoadState::Failed);
    assert_eq!(text_of(&h, a), "", "先頭は捨てる");
    h.text("file.save");
    h.keys("enter");
    // 開いたときの文書 (先頭の文書) では上書き保存は使えないので、パレットは閉じない (c を書かない)
    assert_eq!(h.overlay(), "palette");
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.fs_read("/w/c.txt").unwrap(), b"disk\n", "c は書かない");
    assert!(dirty(&h, c));
    assert_eq!(h.doc_id(), c, "パレットを閉じた後に開く前の文書へ戻す");
    assert!(h.editor.state().document(a).is_none());
}

#[test]
fn 表示から外した無題をオーバーレイの間は閉じない() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    h.cmd("buffer.list");
    h.finish_held_jobs();
    assert_eq!(h.editor.state().documents_len(), 2, "一覧の間は閉じない");
    h.keys("esc");
    assert_eq!(h.editor.state().documents_len(), 1, "閉じた後に閉じる");
    assert_eq!(h.doc_id(), doc);
}

// 01 §5.5 の FR-122 AC-3
#[test]
fn 先頭の文書の上書き保存は読込中であることを示す() {
    let (mut h, _) = partial_shown("/w/a.txt", &lines(100));
    h.clear_log();
    assert_eq!(
        h.cmd("file.save"),
        CommandResult::Rejected(RejectReason::NotAvailable)
    );
    assert_eq!(h.messages(), vec!["info.reload.busy"]);
    h.clear_log();
    h.keys("ctrl+s");
    assert_eq!(h.messages(), vec!["info.reload.busy"]);
}

#[test]
fn 保留した先行表示の後の確認では_本文の無い文書へ切り替えない() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(10));
    h.text("x");
    let from = h.doc_id();
    let CommandResult::Pending(job) = h.open("/w/a.txt") else {
        panic!()
    };
    h.cmd("help.open");
    send_head(&mut h, job, "line 000000\n");
    assert_eq!(h.doc_id(), from, "ヘルプの間は保留");
    h.editor.on_job_result(JobOutput {
        job,
        body: JobResult::NeedsConfirm {
            ask: ConfirmAsk::HugeFile { len: 1 << 30 },
        },
    });
    assert_eq!(h.overlay(), "dialog");
    h.answer(0);
    assert_eq!(h.overlay(), "", "確認はヘルプに替わって出ていた");
    assert_eq!(
        h.doc_id(),
        from,
        "本文の無い Loading の文書へは切り替えない"
    );
}

// ---------- 試験の抜け ----------

#[test]
fn 表示していない文書の先行表示はステータスバーに読込中と出す() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(100));
    h.text("x");
    let from = h.doc_id();
    slow(&h);
    h.open("/w/a.txt");
    assert!(h.run_jobs_until_partial());
    h.cmd("buffer.prev");
    assert_eq!(h.doc_id(), from);
    let loading = status_item(&mut h, StatusKind::Loading).expect("読込中の項目");
    assert!(loading.starts_with("[読込中 "), "{loading}");
}

#[test]
fn 何も入力していない無題からの完了の切り替えを保留しても_置き換えで番号を増やさない() {
    let mut h = Harness::new().with_file("/w/a.txt", b"small\n");
    h.open("/w/a.txt");
    h.cmd("ui.palette_open");
    h.run_jobs();
    assert_eq!(h.editor.state().documents_len(), 2);
    assert_eq!(h.doc_text(), "", "パレットの間は無題のまま");
    h.keys("esc");
    assert_eq!(h.doc_text(), "small\n");
    assert_eq!(h.editor.state().documents_len(), 1, "無題を置き換える");
}

#[test]
fn 行の区切りで切った先頭の_end_では知らせない() {
    let (mut h, doc) = partial_shown("/w/a.txt", &lines(100));
    let last_line = text_of(&h, doc).lines().count();
    h.clear_log();
    h.keys("end");
    for _ in 0..last_line {
        h.keys("down");
    }
    h.keys("end shift+end");
    assert_eq!(h.messages(), Vec::<String>::new());
}

#[test]
fn 再読込のジョブから来た先頭は捨てる() {
    let mut h = Harness::new().with_file("/w/a.txt", b"old\n");
    h.open("/w/a.txt");
    h.run_jobs();
    let doc = h.doc_id();
    h.fs.add_file("/w/a.txt", b"new\n");
    let CommandResult::Pending(job) = h.cmd("file.reload") else {
        panic!("再読込のジョブ")
    };
    send_head(&mut h, job, "XXX\n");
    assert_eq!(load_state(&h, doc), LoadState::Loading);
    assert_eq!(text_of(&h, doc), "old\n", "旧本文のまま");
    h.run_jobs();
    assert_eq!(text_of(&h, doc), "new\n");
}

#[test]
fn バッファ一覧で読込中の文書に印を付ける() {
    let mut h = Harness::new().with_file("/w/a.txt", &lines(100));
    h.text("x");
    slow(&h);
    h.open("/w/a.txt");
    assert!(h.run_jobs_until_partial());
    h.cmd("buffer.list");
    let details: Vec<String> = h
        .frame()
        .overlays
        .iter()
        .flat_map(|o| match o {
            eve_view::OverlayModel::List { items, .. } => items
                .iter()
                .map(|i| {
                    i.detail
                        .as_ref()
                        .map(|d| d.spans.iter().map(|s| s.text.as_str()).collect())
                        .unwrap_or_default()
                })
                .collect::<Vec<String>>(),
            _ => Vec::new(),
        })
        .collect();
    assert!(details.iter().any(|d| d.ends_with("読込中")), "{details:?}");
    assert!(details.iter().any(|d| d.ends_with("UTF-8")), "{details:?}");
}
