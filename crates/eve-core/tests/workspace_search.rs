//! 横断検索 (docs/03 §12.7.5 / §12.7.6 の 3、04 §8.12、01 FR-405。v0.5 の M31 の段 7)。
//!
//! 偽 FS の木で、開く・探す・トグル・ジャンプ・結果を保つ・文字コード・開いている文書・飛ばすファイル・取消しと世代・
//! 上限・失敗の知らせを確かめる。偽 FS のカレントディレクトリは `/w`。ジョブの区切りと 1 ファイルの扱いは、ジョブを
//! 直に走らせて確かめる。

use std::sync::Arc;

use eve_core::Clock as _;
use eve_core::{
    Args, CancelToken, Case, ExcludeConfig, FileId, FsPath, InitialFile, JobError, JobKind,
    JobOutput, JobPorts, JobRequest, JobResult, OpenDocBody, OpenDocRef, SearchQuery,
    WS_FILES_PER_STEP, WS_MAX_FILE_SIZE, WS_MAX_RESULT_LINES, WsHit, WsProgress, run_job,
};
use eve_testkit::{FixedPaths, Harness, ManualClock, MemFs};
use eve_text::{ByteOff, ByteRange, TextRead};
use eve_view::{Answer, OverlayModel, Query, Span};

const JP: &str = "日本語のテキストエディタです。これは文章の見本で、文字コードの判定に使います。\n";

/// `/w/proj` (`.git` のあるリポジトリ) に `files` を置き、`/w/proj/README.md` を開いて起動する (ルートは目印で `/w/proj`)。
fn repo(files: &[(&str, &[u8])]) -> Harness {
    let mut b = Harness::builder().defer_start().dir("/w/proj/.git");
    b = b.file("/w/proj/README.md", b"readme\n");
    for (p, data) in files {
        b = b.file(p, data);
    }
    let mut h = b.build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/proj/README.md")], false, None);
    h.started();
    h.run_jobs();
    h
}

/// 横断検索を開いて `text` を打ち、終わるまで走らせる。
fn search(h: &mut Harness, text: &str) {
    h.keys("ctrl+shift+f");
    assert_eq!(h.overlay(), "workspace_search");
    h.text(text);
    h.run_jobs();
}

/// 出している行 (`相対パス:行`、行は 1 始まり)。
fn rows(h: &Harness) -> Vec<String> {
    h.editor
        .state()
        .ws_search_rows()
        .expect("横断検索が開いている")
        .1
}

fn progress(h: &Harness) -> WsProgress {
    h.editor.state().ws_search_rows().expect("開いている").3
}

/// 一覧のオーバーレイの (見出し, 行の文字列, 強調した Span の文字列, 件数の欄, 0 件の文言)。
fn model(h: &mut Harness) -> (String, Vec<String>, Vec<String>, String, String) {
    let f = h.frame();
    let Some(OverlayModel::List {
        title,
        items,
        footer,
        no_match,
        ..
    }) = f.overlays.first()
    else {
        panic!("一覧のオーバーレイが無い: {:?}", f.overlays);
    };
    let text = |s: &[Span]| s.iter().map(|x| x.text.as_str()).collect::<String>();
    let labels = items.iter().map(|i| text(&i.label.spans)).collect();
    let marked = items
        .iter()
        .map(|i| {
            i.label
                .spans
                .iter()
                .enumerate()
                .filter(|(k, _)| i.match_ranges.iter().any(|r| r.contains(k)))
                .map(|(_, s)| s.text.as_str())
                .collect::<String>()
        })
        .collect();
    (
        text(&title.spans),
        labels,
        marked,
        text(&footer.spans),
        text(&no_match.spans),
    )
}

/// 主の Range の (anchor, head) (バイト)。
fn sel(h: &Harness) -> (u64, u64) {
    let view = match h.query(&Query::Windows).unwrap() {
        Answer::Windows(ws) => ws[0].views[0],
        a => panic!("{a:?}"),
    };
    match h.query(&Query::Selection { view }).unwrap() {
        Answer::Selection {
            ranges, primary, ..
        } => (ranges[primary].anchor, ranges[primary].head),
        a => panic!("{a:?}"),
    }
}

fn shown_path(h: &Harness) -> Option<String> {
    h.doc_path().map(|p| p.replace('\\', "/"))
}

fn count(h: &Harness, key: &str) -> usize {
    h.messages().iter().filter(|k| *k == key).count()
}

fn sjis(s: &str) -> Vec<u8> {
    encoding_rs::SHIFT_JIS.encode(s).0.into_owned()
}

fn utf16le_bom(s: &str) -> Vec<u8> {
    let mut v = vec![0xFF, 0xFE];
    v.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
    v
}

// ---- 開く・探す・ジャンプ (AC-1) ----

// verifies: AC-FR-405-1
#[test]
fn ctrl_shift_f_で開き_一致を行ごとに出し_選ぶと一致の位置へ移る() {
    let mut h = repo(&[
        (
            "/w/proj/src/a.rs",
            b"fn main() {\n    let needle = 1; // needle\n}\n",
        ),
        ("/w/proj/src/b.rs", b"needle\n"),
        ("/w/proj/docs/c.md", b"nothing here\n"),
    ]);
    search(&mut h, "needle");
    assert_eq!(
        rows(&h),
        ["src/a.rs:2", "src/b.rs:1"],
        "同じ行の一致は 1 件にまとめる"
    );
    let (title, labels, marked, footer, no_match) = model(&mut h);
    assert_eq!(title, "ワークスペースを検索 [Aa~] [.*] [ab]");
    assert_eq!(
        labels[0], "src/a.rs:2: let needle = 1; // needle",
        "先頭の空白を詰めた抜粋"
    );
    assert_eq!(marked[0], "needleneedle", "行の一致をすべて強調する");
    assert_eq!(
        footer, "2 件 (2/4 ファイル)",
        "探したのは README.md・a.rs・b.rs・c.md"
    );
    assert_eq!(no_match, "");
    h.keys("down enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "", "選んだら閉じる");
    assert_eq!(shown_path(&h).as_deref(), Some("/w/proj/src/b.rs"));
    assert_eq!(sel(&h), (0, 6), "一致の範囲を主の選択にする");
}

// verifies: AC-FR-405-1
#[test]
fn ctrl_k_f_でも開け_esc_で閉じても結果と選択を保ち_探し直さない() {
    let mut h = repo(&[("/w/proj/a.txt", b"x\nx\nx\n")]);
    h.keys("ctrl+k f");
    assert_eq!(h.overlay(), "workspace_search");
    h.text("x");
    h.run_jobs();
    assert_eq!(rows(&h), ["a.txt:1", "a.txt:2", "a.txt:3"]);
    h.keys("down down esc");
    assert_eq!(h.overlay(), "");
    h.keys("ctrl+shift+f");
    assert_eq!(h.pending_jobs(), 0, "開き直しても探し直さない");
    let (running, r, selected, _) = h.editor.state().ws_search_rows().unwrap();
    assert!(!running);
    assert_eq!(r, ["a.txt:1", "a.txt:2", "a.txt:3"]);
    assert_eq!(selected, 2, "選択も保つ");
    let (_, _, _, footer, _) = model(&mut h);
    assert_eq!(footer, "3 件 (1/2 ファイル)");
}

// verifies: AC-FR-405-1
#[test]
fn 除外に当たるファイルは探さない() {
    let mut h = repo(&[
        ("/w/proj/.gitignore", b"*.log\n"),
        ("/w/proj/debug.log", b"hit\n"),
        ("/w/proj/node_modules/x/index.js", b"hit\n"),
        ("/w/proj/target/out.txt", b"hit\n"),
        ("/w/proj/.env", b"hit\n"),
    ]);
    search(&mut h, "hit");
    assert_eq!(rows(&h), [".env:1"]);
}

// verifies: AC-FR-405-1
#[test]
fn トグルで正規表現と大小と単語単位を切り替えて探し直す() {
    let mut h = repo(&[("/w/proj/a.txt", b"foo\nFOO\nfoooo\nfoobar\n")]);
    search(&mut h, "fo+");
    assert!(rows(&h).is_empty(), "リテラルでは当たらない");
    let (_, _, _, footer, no_match) = model(&mut h);
    assert_eq!(footer, "0 件 (0/2 ファイル)");
    assert_eq!(no_match, "一致はありません");
    h.keys("alt+r");
    h.run_jobs();
    assert_eq!(rows(&h), ["a.txt:1", "a.txt:2", "a.txt:3", "a.txt:4"]);
    assert_eq!(
        model(&mut h).0,
        "ワークスペースを検索 [Aa~] [.*\u{2713}] [ab]"
    );
    h.keys("alt+c");
    h.run_jobs();
    assert_eq!(
        rows(&h),
        ["a.txt:1", "a.txt:3", "a.txt:4"],
        "大小を区別する"
    );
    h.keys("ctrl+k b");
    h.run_jobs();
    assert_eq!(rows(&h), ["a.txt:1", "a.txt:3"], "単語単位");
    assert_eq!(
        model(&mut h).0,
        "ワークスペースを検索 [Aa\u{2713}] [.*\u{2713}] [ab\u{2713}]"
    );
}

// verifies: AC-FR-405-1
#[test]
fn トグルは検索バーが開いていてもオーバーレイの検索語に効き_ファインダでは効かない() {
    let mut h = repo(&[("/w/proj/a.txt", b"x\n")]);
    h.keys("ctrl+f");
    h.text("x");
    h.keys("ctrl+shift+f");
    assert_eq!(h.overlay(), "workspace_search");
    h.keys("alt+r");
    assert_eq!(
        model(&mut h).0,
        "ワークスペースを検索 [Aa~] [.*\u{2713}] [ab]",
        "オーバーレイの正規表現のトグルが切り替わる"
    );
    h.keys("esc");
    assert_eq!(h.overlay(), "");
    let bar = h.frame().search.expect("検索バーが開いている");
    assert!(!bar.regex, "検索バーのトグルは変えない");
    // 検索バーを開いたまま (コマンドの `when` の search.active は真) ファインダを開く: ファインダの中の alt+r は、割当の
    // `when` (overlay.active == "workspace_search") で外れ、検索バーのトグルも変えない
    h.keys("ctrl+o");
    assert_eq!(h.overlay(), "finder");
    h.keys("alt+r");
    assert_eq!(h.overlay(), "finder");
    h.keys("esc");
    let bar = h.frame().search.expect("検索バーが開いている");
    assert!(!bar.regex, "ファインダの alt+r は何もしない");
}

// verifies: AC-FR-405-1
#[test]
fn 検索バーから開くと検索語とトグルを引き継ぎ_選択からも始められる() {
    let mut h = repo(&[("/w/proj/a.txt", b"Abc abc\n")]);
    h.cmd_with("file.open", Args::new().with_str("path", "/w/proj/a.txt"));
    h.run_jobs();
    h.keys("ctrl+f");
    h.text("abc");
    h.keys("alt+c");
    h.keys("ctrl+shift+f");
    h.run_jobs();
    assert_eq!(rows(&h), ["a.txt:1"]);
    let (title, _, marked, _, _) = model(&mut h);
    assert_eq!(title, "ワークスペースを検索 [Aa\u{2713}] [.*] [ab]");
    assert_eq!(marked[0], "abc", "大小を区別して abc だけ");
    h.keys("esc");
    h.keys("esc");
    // 編集面の選択 (1 行の中) から始める
    h.keys("ctrl+home shift+right shift+right shift+right");
    h.keys("ctrl+shift+f");
    h.run_jobs();
    let (title, _, marked, _, _) = model(&mut h);
    assert_eq!(
        title, "ワークスペースを検索 [Aa~] [.*] [ab]",
        "選択から始めるときは設定の値"
    );
    assert_eq!(marked[0], "Abc", "Smart: 大文字を含むので区別する");
}

#[test]
fn 正規表現の誤りは件数の位置に出して探さない() {
    let mut h = repo(&[("/w/proj/a.txt", b"x\n")]);
    h.keys("ctrl+shift+f alt+r");
    h.text("(");
    assert_eq!(h.pending_jobs(), 0, "誤りのある検索語では探さない");
    let (_, _, _, footer, _) = model(&mut h);
    assert!(footer.starts_with("✖ 正規表現が不正"), "{footer}");
}

#[test]
fn 双方向の制御文字と全角は可視の記法にし_強調は記法の後の位置() {
    let mut h = repo(&[(
        "/w/proj/a\u{202E}b.txt",
        "ｘ\u{202E}全角needle\n".as_bytes(),
    )]);
    search(&mut h, "needle");
    let (_, labels, marked, _, _) = model(&mut h);
    assert_eq!(labels.len(), 1);
    assert!(
        !labels[0].contains('\u{202E}'),
        "制御文字をそのまま出さない: {labels:?}"
    );
    assert_eq!(
        marked,
        ["needle"],
        "記法で幅が変わっても一致の文字を強調する"
    );
}

// 名前が UTF-8 で表せないファイル (Unix の実 FS) は eve-host の tests/workspace_search_unix.rs で確かめる (偽 FS の名前は UTF-8)

// ---- 1 つのファイルの扱い (AC-2) ----

/// 横断検索の結果 (行, 最初の一致の始まり) と、同じファイルをエディタで開いて同じ検索語で探した一致の (行, 行の中の始まり)
/// の各行の最初のもの。
fn same_as_editor(files: &[(&str, Vec<u8>)], query: SearchQuery) {
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(p, d)| (*p, d.as_slice())).collect();
    let mut h = repo(&refs);
    h.keys("ctrl+shift+f");
    if query.is_regex() {
        h.keys("alt+r");
    }
    h.text(query.pattern());
    h.run_jobs();
    let hits: Vec<WsHit> = h.editor.state().ws_search_hits().unwrap().to_vec();
    h.keys("esc");
    for (path, _) in files {
        let rel = path.trim_start_matches("/w/proj/");
        let got: Vec<(u32, Vec<std::ops::Range<u32>>)> = hits
            .iter()
            .filter(|x| &*x.rel == rel)
            .map(|x| (x.line, x.ranges.clone()))
            .collect();
        h.cmd_with("file.open", Args::new().with_str("path", path));
        h.run_jobs();
        if h.overlay() == "dialog" {
            h.text("r");
            h.run_jobs();
        }
        let d = h.editor.state().document(h.doc_id()).unwrap();
        let snap = d.snapshot();
        let m = query.matcher().expect("照合器");
        // 行ごとの一致の範囲 (行の中の位置。行をまたぐ一致は行末まで)。試験のファイルの行は短く、1 行の一致は 16 個以下
        // なので、横断検索の結果は抜粋の窓と個数で削られず、すべての一致を持つ
        let mut want: Vec<(u32, Vec<std::ops::Range<u32>>)> = Vec::new();
        eve_core::scan_bytes(
            &snap,
            &m,
            ByteRange::new(ByteOff(0), ByteOff(snap.len_bytes())),
            &CancelToken::new(),
            &mut |r| {
                let line = snap.byte_to_line(r.start);
                let ls = snap.line_to_byte(line).0;
                let le = snap.line_end_byte(line).0;
                let l = u32::try_from(line.0).unwrap();
                let s = u32::try_from(r.start.0 - ls).unwrap();
                let e = u32::try_from(r.end.0.min(le).max(r.start.0) - ls).unwrap();
                match want.last_mut() {
                    Some(w) if w.0 == l => w.1.push(s..e),
                    _ => want.push((l, std::iter::once(s..e).collect())),
                }
                true
            },
        );
        assert!(!want.is_empty(), "{path}: エディタで一致する");
        assert_eq!(got, want, "{path}: エディタで開いて探した結果と同じ");
    }
}

// verifies: AC-FR-405-2
#[test]
fn shift_jis_euc_jp_utf16_のファイルでもエディタと同じに一致する() {
    let euc = encoding_rs::EUC_JP.encode(JP).0.into_owned();
    same_as_editor(
        &[
            ("/w/proj/sjis.txt", sjis(&format!("{JP}{JP}"))),
            ("/w/proj/euc.txt", euc),
            ("/w/proj/u16.txt", utf16le_bom(&format!("abc\r\n{JP}"))),
        ],
        SearchQuery::literal("文字コード"),
    );
}

// verifies: AC-FR-405-2
#[test]
fn cp932_互換文字と行の終端の扱いはエディタ内の検索と同じ() {
    same_as_editor(
        &[(
            "/w/proj/wave.txt",
            "a\u{FF5E}b\r\nc\u{301C}d\n".as_bytes().to_vec(),
        )],
        SearchQuery::literal("\u{301C}"),
    );
    same_as_editor(
        &[("/w/proj/crlf.txt", b"x\r\nend\r\nx\rend\n".to_vec())],
        SearchQuery::literal("x\\nend").with_regex(true),
    );
}

// verifies: AC-FR-405-2
#[test]
fn 不正バイトの多いファイルも確認を出さずに復号したまま探す() {
    let mut data = b"hit ".to_vec();
    data.extend([0xFF, 0xFE, 0xFD, 0x80, 0x81].repeat(20));
    data.extend(b"\nhit\n");
    same_as_editor(&[("/w/proj/bad.txt", data)], SearchQuery::literal("hit"));
}

// verifies: AC-FR-405-2
#[test]
fn 開いている文書は保存していない編集を探し_ディスクは探さない() {
    let mut h = repo(&[("/w/proj/a.txt", b"disk\n")]);
    h.cmd_with("file.open", Args::new().with_str("path", "/w/proj/a.txt"));
    h.run_jobs();
    h.keys("ctrl+end");
    h.text("buffer side");
    assert!(h.is_dirty());
    // 開いた後にディスクの側だけが変わった (文書には無い語)
    h.fs.set_file(&FsPath::from("/w/proj/a.txt"), b"outside\n");
    search(&mut h, "side");
    assert_eq!(
        rows(&h),
        ["a.txt:2"],
        "ディスクの outside ではなく文書の 2 行目の buffer side"
    );
    let hits = h.editor.state().ws_search_hits().unwrap();
    assert!(hits[0].from_open_doc);
    assert_eq!(&*hits[0].excerpt, "buffer side");
}

// verifies: AC-FR-405-2
#[test]
fn 大きすぎるファイルとバイナリは飛ばして数える() {
    let big = vec![b'a'; usize::try_from(WS_MAX_FILE_SIZE).unwrap() + 1];
    let mut h = repo(&[
        ("/w/proj/big.txt", &big),
        ("/w/proj/bin.dat", b"hit\0\0\0\x01\x02\x03hit"),
        ("/w/proj/ok.txt", b"hit\n"),
    ]);
    // 大きさが列挙で分からない FS (Unix) でも、上限 + 1 バイトを読んだ時点で飛ばす
    h.fs.set_fake_len("/w/proj/big.txt", None);
    search(&mut h, "a");
    let p = progress(&h);
    assert_eq!(p.skipped_large, 1, "{p:?}");
    search_again(&mut h, "hit");
    let p = progress(&h);
    assert_eq!(rows(&h), ["ok.txt:1"]);
    assert_eq!((p.skipped_large, p.skipped_binary), (1, 1), "{p:?}");
    let (_, _, _, footer, _) = model(&mut h);
    assert_eq!(footer, "1 件 (1/2 ファイル)・飛ばした 2");
}

/// 開いている横断検索の入力欄を `text` にして探し直す。
fn search_again(h: &mut Harness, text: &str) {
    h.keys("ctrl+a backspace");
    h.text(text);
    h.run_jobs();
}

// verifies: AC-FR-405-2
#[test]
fn 読めないファイルとオフラインのファイルは飛ばし_辿らないリンクは数えない() {
    let mut h = repo(&[
        ("/w/proj/a.txt", b"hit\n"),
        ("/w/proj/cloud.txt", b"hit\n"),
        ("/w/outside/secret.txt", b"hit\n"),
    ]);
    h.fs.set_offline("/w/proj/cloud.txt", true);
    h.fs.add_link("/w/proj/link.txt", "/w/outside/secret.txt");
    h.fs.read_fail_after(Some(0));
    search(&mut h, "hit");
    h.fs.read_fail_after(None);
    let p = progress(&h);
    assert_eq!(rows(&h), Vec::<String>::new());
    assert_eq!(
        p.skipped_other, 2,
        "読めない a.txt とオフラインの cloud.txt: {p:?}"
    );
    assert_eq!(p.files, 1, "探したのは開いている README.md だけ: {p:?}");
    search_again(&mut h, "hi");
    assert_eq!(
        rows(&h),
        ["a.txt:1"],
        "リンク先 (ワークスペースの外) は読まない"
    );
}

// ---- 取消し・上限・失敗 (AC-3) ----

// verifies: AC-FR-405-3
#[test]
fn 入力を変えると前の検索を取り消し_古い世代の結果は捨てて_bg_で手放す() {
    let files: Vec<(String, Vec<u8>)> = (0..200)
        .map(|i| (format!("/w/proj/f{i:03}.txt"), b"abc\n".to_vec()))
        .collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_slice()))
        .collect();
    let mut h = repo(&refs);
    h.keys("ctrl+shift+f");
    h.text("ab");
    // 1 区切りだけ走らせ、結果は渡さずに持っておく
    let stale = h.run_one_job_held();
    assert!(
        stale
            .iter()
            .any(|o| matches!(&o.body, JobResult::WorkspaceHits { hits, .. } if !hits.is_empty())),
        "最初の区切りで結果が届く"
    );
    let (running, r, _, _) = h.editor.state().ws_search_rows().unwrap();
    assert!(running && r.is_empty());
    let (_, _, _, footer, _) = model(&mut h);
    assert_eq!(footer, "0 件・0 ファイルを検索中…");
    h.text("c");
    // 古い世代の結果は採らない
    h.deliver(stale);
    assert!(rows(&h).is_empty(), "古い世代の結果を出さない");
    let mut cancelled = 0;
    let mut released = 0;
    h.run_jobs_with(|o| match &o.body {
        JobResult::Failed {
            err: JobError::Cancelled,
        } => cancelled += 1,
        JobResult::Released => released += 1,
        _ => {}
    });
    assert_eq!(cancelled, 1, "前の検索を取り消した");
    assert_eq!(released, 1, "古い結果を主スレッドで手放さず bg に渡した");
    assert_eq!(rows(&h).len(), 200);
    // 出している結果がある状態で入力を変える: 出していた結果も bg で手放す
    h.keys("backspace");
    let mut released = 0;
    h.run_jobs_with(|o| {
        if matches!(o.body, JobResult::Released) {
            released += 1;
        }
    });
    assert_eq!(released, 1, "出していた 200 行を主スレッドで手放さない");
    assert_eq!(rows(&h).len(), 200);
}

// verifies: AC-FR-405-3
#[test]
fn 検索の間は探したファイルの数を件数の欄に出す() {
    let files: Vec<(String, Vec<u8>)> = (0..200)
        .map(|i| (format!("/w/proj/f{i:03}.txt"), b"nothing\n".to_vec()))
        .collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_slice()))
        .collect();
    let mut h = repo(&refs);
    h.keys("ctrl+shift+f");
    h.text("zz");
    // 最初の区切り (一致の無い 64 ファイル): 結果の無い区切りでも進捗を送る
    assert!(h.run_one_job());
    let (running, _, _, p) = h.editor.state().ws_search_rows().unwrap();
    assert!(running);
    assert_eq!(p.files, 64, "{p:?}");
    let (_, _, _, footer, no_match) = model(&mut h);
    assert_eq!(footer, "0 件・64 ファイルを検索中…");
    assert_eq!(no_match, "", "検索中は 0 件の文言を出さない");
    h.run_jobs();
    let (_, _, _, footer, _) = model(&mut h);
    assert_eq!(footer, "0 件 (0/201 ファイル)");
}

#[test]
fn 結果の無い区切りの進捗は_100_ms_ごとに送る() {
    let (fs, clock) = fs();
    for i in 0..(WS_FILES_PER_STEP * 4) {
        fs.add_file(&format!("/r/f{i:03}.txt"), b"nothing\n");
    }
    let ports = JobPorts {
        stdin: None,
        fs: fs.clone(),
        clock: Arc::new(clock.clone()),
        paths: Arc::new(FixedPaths::default()),
    };
    let cancel = CancelToken::new();
    let mut kind = job("/r", "zz", Vec::new(), 10);
    let mut sent = Vec::new();
    for step in 0..3 {
        if step == 2 {
            clock.advance(eve_core::WS_PROGRESS_INTERVAL);
        }
        let mut out = Vec::new();
        let s = run_job(
            JobRequest {
                id: eve_view::JobId(1),
                doc: None,
                base: eve_view::Revision(0),
                kind,
            },
            &ports,
            &cancel,
            &mut |o| out.push(o),
        );
        sent.push(
            out.iter()
                .filter_map(|o| match &o.body {
                    JobResult::WorkspaceHits { hits, progress, .. } => {
                        assert_eq!(hits, &Vec::<WsHit>::new());
                        Some(progress.files)
                    }
                    _ => None,
                })
                .collect::<Vec<u64>>(),
        );
        match s {
            eve_core::JobStep::Continue(k) => kind = k,
            eve_core::JobStep::Done => break,
        }
    }
    assert_eq!(
        sent,
        [vec![64], vec![], vec![192]],
        "最初の区切りは送り、100 ms 経つまでは送らない"
    );
}

// verifies: AC-FR-405-3
#[test]
fn 閉じると検索を取り消し_開き直すと同じ検索語で探し直す() {
    let files: Vec<(String, Vec<u8>)> = (0..200)
        .map(|i| (format!("/w/proj/f{i:03}.txt"), b"abc\n".to_vec()))
        .collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_slice()))
        .collect();
    let mut h = repo(&refs);
    h.keys("ctrl+shift+f");
    h.text("abc");
    assert!(h.run_one_job());
    assert_ne!(rows(&h), Vec::<String>::new());
    h.keys("esc");
    let mut cancelled = 0;
    h.run_jobs_with(|o| {
        if matches!(
            o.body,
            JobResult::Failed {
                err: JobError::Cancelled
            }
        ) {
            cancelled += 1;
        }
    });
    assert_eq!(cancelled, 1, "閉じると取り消す");
    h.keys("ctrl+shift+f");
    assert!(
        h.editor.state().ws_search_rows().unwrap().0,
        "途中で止めた検索は探し直す"
    );
    h.run_jobs();
    assert_eq!(rows(&h).len(), 200);
}

// verifies: AC-FR-405-3
#[test]
fn 結果は_10000_行で打ち切りその旨を示す() {
    let body = "hit\n".repeat(WS_MAX_RESULT_LINES + 5);
    let mut h = repo(&[
        ("/w/proj/a.txt", body.as_bytes()),
        ("/w/proj/z.txt", b"hit\n"),
    ]);
    search(&mut h, "hit");
    assert_eq!(rows(&h).len(), WS_MAX_RESULT_LINES);
    let (_, _, _, footer, _) = model(&mut h);
    assert_eq!(footer, "10000 件で打ち切りました (1 ファイル)");
}

#[test]
fn ちょうど上限の行数で終わる検索は打ち切りの印を立てない() {
    let (fs, clock) = fs();
    fs.add_file("/r/a.txt", b"hit\nhit\n");
    let out = run_to_end(&fs, &clock, job("/r", "hit", Vec::new(), 2));
    let (hits, truncated) = collect(&out);
    assert_eq!(hits.len(), 2);
    assert!(!truncated, "上限を超える行を見つけていない");
    fs.add_file("/r/b.txt", b"hit\n");
    let out = run_to_end(&fs, &clock, job("/r", "hit", Vec::new(), 2));
    let (hits, truncated) = collect(&out);
    assert_eq!(hits.len(), 2);
    assert!(truncated);
}

// verifies: AC-FR-405-3
#[test]
fn 検索のジョブが失敗したら知らせ_件数の欄を失敗にして抜けられる() {
    let mut h = repo(&[("/w/proj/a.txt", b"hit\n")]);
    h.keys("ctrl+shift+f");
    h.text("hit");
    let held = h.run_one_job_held();
    let job = held.first().expect("結果").job;
    h.deliver(vec![JobOutput {
        job,
        body: JobResult::Failed {
            err: JobError::Panic,
        },
    }]);
    assert_eq!(count(&h, "msg.workspace.search_failed"), 1);
    let (running, _, _, _) = h.editor.state().ws_search_rows().unwrap();
    assert!(!running, "検索中のままにしない");
    let (_, _, _, footer, no_match) = model(&mut h);
    assert_eq!(footer, "0 件 (検索に失敗しました)");
    assert_eq!(no_match, "一致はありません");
    // 検索語を変えると探し直す
    h.text("x");
    h.keys("backspace");
    h.run_jobs();
    assert_eq!(rows(&h), ["a.txt:1"]);
}

#[test]
fn ルートを読めなければ知らせる() {
    let mut h = repo(&[("/w/proj/a.txt", b"hit\n")]);
    h.fs.set_unreadable("/w/proj", true);
    search(&mut h, "hit");
    assert_eq!(count(&h, "msg.workspace.scan_failed"), 1);
    let (_, _, _, footer, _) = model(&mut h);
    assert_eq!(footer, "0 件 (検索に失敗しました)");
}

#[test]
fn ルートを待つ間は調べていますを出し_届いたら探し_決まらなければそう示す() {
    let mut h = Harness::builder()
        .defer_start()
        .dir("/w/proj/.git")
        .file("/w/proj/a.txt", b"hit\n")
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/proj/a.txt")], false, None);
    h.started();
    // 開くジョブだけを走らせる (ルートの検出はまだ)
    h.run_one_job();
    h.keys("ctrl+shift+f");
    h.text("hit");
    let (_, _, _, footer, _) = model(&mut h);
    assert_eq!(footer, "ワークスペースを調べています…");
    h.run_jobs();
    assert_eq!(rows(&h), ["a.txt:1"]);

    // ルートの検出のジョブが失敗した: 調べていますのまま待たない (ファインダも同じ)
    let mut h = Harness::builder()
        .defer_start()
        .dir("/w/proj/.git")
        .file("/w/proj/a.txt", b"hit\n")
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/proj/a.txt")], false, None);
    h.started();
    h.run_one_job();
    h.keys("ctrl+shift+f");
    h.text("hit");
    let root = h.run_one_job_held();
    let job = root.first().expect("ルートの検出").job;
    h.deliver(vec![JobOutput {
        job,
        body: JobResult::Failed {
            err: JobError::Panic,
        },
    }]);
    let (_, _, _, footer, _) = model(&mut h);
    assert_eq!(footer, "ワークスペースを決められませんでした");
    h.keys("esc ctrl+o");
    let (_, _, _, footer, _) = model(&mut h);
    assert_eq!(footer, "ワークスペースを決められませんでした");
}

#[test]
fn ファインダの照合が失敗したら知らせる() {
    let mut h = repo(&[("/w/proj/abc.txt", b"\n")]);
    h.keys("ctrl+o");
    h.run_jobs();
    h.text("abc");
    let held = h.run_one_job_held();
    let job = held
        .iter()
        .find(|o| matches!(o.body, JobResult::FinderMatched { .. }))
        .expect("照合の結果")
        .job;
    h.deliver(vec![JobOutput {
        job,
        body: JobResult::Failed {
            err: JobError::Panic,
        },
    }]);
    assert_eq!(count(&h, "msg.finder.match_failed"), 1);
}

// ---- ジャンプ ----

// verifies: AC-FR-405-1
#[test]
fn 開いている文書への_ジャンプは切り替えてすぐ移る() {
    let mut h = repo(&[("/w/proj/a.txt", b"one\ntwo hit\n")]);
    h.cmd_with("file.open", Args::new().with_str("path", "/w/proj/a.txt"));
    h.run_jobs();
    h.cmd_with(
        "file.open",
        Args::new().with_str("path", "/w/proj/README.md"),
    );
    h.run_jobs();
    search(&mut h, "hit");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(shown_path(&h).as_deref(), Some("/w/proj/a.txt"));
    assert_eq!(h.editor.state().documents_len(), 2, "新しく開かない");
    assert_eq!(sel(&h), (8, 11));
    // 今の文書の中の一致へは、開き直さずに移る (「既に開いています」を出さない)
    h.keys("ctrl+home");
    h.clear_log();
    h.keys("ctrl+shift+f enter");
    h.run_jobs();
    assert_eq!(sel(&h), (8, 11));
    assert_eq!(count(&h, "info.open.already_open"), 0);
}

// verifies: AC-FR-405-1
#[test]
fn 別の書き方のパスで開いている文書へは識別で突き合わせて移る() {
    let mut h = repo(&[("/w/proj/a.txt", b"one\ntwo hit\n")]);
    // ワークスペースの外のリンクから開く (canonical はワークスペースの中)
    h.fs.add_link("/w/link.txt", "/w/proj/a.txt");
    h.cmd_with("file.open", Args::new().with_str("path", "/w/link.txt"));
    h.run_jobs();
    h.cmd_with(
        "file.open",
        Args::new().with_str("path", "/w/proj/README.md"),
    );
    h.run_jobs();
    search(&mut h, "hit");
    assert!(h.editor.state().ws_search_hits().unwrap()[0].from_open_doc);
    h.clear_log();
    h.keys("enter");
    assert_eq!(
        h.pending_jobs(),
        0,
        "canonical のパスで開いている文書と分かるので、開くジョブを走らせない"
    );
    assert_eq!(
        shown_path(&h).as_deref(),
        Some("/w/link.txt"),
        "既存の文書に切り替える"
    );
    assert_eq!(
        h.editor.state().documents_len(),
        2,
        "同じファイルを 2 つの文書にしない"
    );
    assert_eq!(sel(&h), (8, 11), "既存の文書の一致の位置へ移る");
    assert_eq!(count(&h, "info.open.same_file"), 0);
    assert_eq!(count(&h, "info.open.already_open"), 0);
}

// verifies: AC-FR-405-1
#[test]
fn 再読込の間の文書へは再読込の完了で移る() {
    let mut h = repo(&[("/w/proj/a.txt", b"one\ntwo hit\n")]);
    h.cmd_with("file.open", Args::new().with_str("path", "/w/proj/a.txt"));
    h.run_jobs();
    search(&mut h, "hit");
    h.keys("esc");
    // 再読込を始めて (ジョブは走らせない)、その間に結果を選ぶ
    h.fs.set_file(&FsPath::from("/w/proj/a.txt"), b"zero\none\ntwo hit\n");
    h.cmd("file.reload");
    assert_eq!(h.pending_jobs(), 1, "再読込のジョブ");
    h.keys("ctrl+shift+f enter");
    assert_eq!(h.pending_jobs(), 1, "再読込の間の文書を開き直さない");
    h.run_jobs();
    assert_eq!(h.doc_text(), "zero\none\ntwo hit\n", "再読込した");
    // 結果は再読込の前の位置 (2 行目の 4..7)。今の本文の 2 行目 (one) は短いので、丸めてカーソルだけを置く
    assert_eq!(sel(&h), (8, 8), "再読込の完了で移る");
}

// verifies: AC-FR-405-1
#[test]
fn 別の書き方のパスで読込中の今の文書へは読込の完了で移る() {
    // 起動時の文書をリンクのパスで開く (canonical は /w/proj/a.txt)。ルートは目印の無い /w (カレントディレクトリ)
    let mut h = Harness::builder()
        .defer_start()
        .dir("/w/proj")
        .file("/w/proj/a.txt", b"one\ntwo hit\n")
        .build();
    h.fs.add_link("/w/link.txt", "/w/proj/a.txt");
    h.editor
        .open_initial_files(&[InitialFile::path("/w/link.txt")], false, None);
    h.started();
    // 開くジョブの識別 (canonical) だけを渡し、読込の完了は持っておく (今の文書は読込中のまま)
    let held = h.run_one_job_held();
    let (ident, rest): (Vec<_>, Vec<_>) = held
        .into_iter()
        .partition(|o| matches!(o.body, JobResult::Identity { .. }));
    assert_eq!(ident.len(), 1);
    h.deliver(ident);
    h.run_jobs();
    assert_eq!(
        shown_path(&h).as_deref(),
        Some("/w/link.txt"),
        "読込中の今の文書"
    );
    search(&mut h, "hit");
    assert_eq!(rows(&h), ["proj/a.txt:2"], "読込中の文書はディスクを探す");
    h.keys("enter");
    assert_eq!(
        h.pending_jobs(),
        0,
        "canonical で読込中の今の文書と分かるので、開き直さない"
    );
    h.deliver(rest);
    h.run_jobs();
    assert_eq!(shown_path(&h).as_deref(), Some("/w/link.txt"));
    assert_eq!(h.editor.state().documents_len(), 1);
    assert_eq!(sel(&h), (8, 11), "読込の完了で一致の位置へ移る");
}

// 偽 FS のパス (`/w/...`) は、Windows ではルートの正規化で `\w\...` の形になり、識別で比べるときのボリュームの部分
// (03 §12.7.6 の 0) が分からなくなる (実 FS のパスはドライブ文字を持つ)。識別で比べる経路は Unix (WSL) で確かめる
// verifies: AC-FR-405-1
#[test]
#[cfg(unix)]
fn 識別だけが同じハードリンクの結果は_開くジョブの識別で既存の文書に移る() {
    let mut h = repo(&[("/w/proj/x/a.txt", b"one\ntwo hit\n")]);
    h.fs.set_file_id_unique(true);
    h.fs.add_hardlink("/w/proj/y/a.txt", "/w/proj/x/a.txt");
    h.cmd_with("file.open", Args::new().with_str("path", "/w/proj/x/a.txt"));
    h.run_jobs();
    h.cmd_with(
        "file.open",
        Args::new().with_str("path", "/w/proj/README.md"),
    );
    h.run_jobs();
    search(&mut h, "hit");
    assert_eq!(rows(&h), ["x/a.txt:2", "y/a.txt:2"]);
    // y/a.txt の結果: canonical は違うので開くジョブを走らせ、識別で x/a.txt の文書と分かったら、その文書の一致へ移る
    h.keys("down enter");
    h.run_jobs();
    assert_eq!(shown_path(&h).as_deref(), Some("/w/proj/x/a.txt"));
    assert_eq!(h.editor.state().documents_len(), 2);
    assert_eq!(sel(&h), (8, 11));
}

// verifies: AC-FR-405-1
#[test]
fn 検索バーから開いた結果を選んでも一致の位置へ移る() {
    let mut h = repo(&[("/w/proj/a.txt", b"x\nhit\n")]);
    h.keys("ctrl+f");
    h.text("hit");
    h.keys("ctrl+shift+f");
    h.run_jobs();
    assert_eq!(rows(&h), ["a.txt:2"]);
    h.keys("enter");
    h.run_jobs();
    assert!(h.frame().search.is_none(), "結果を選んだら検索バーも閉じる");
    assert_eq!(shown_path(&h).as_deref(), Some("/w/proj/a.txt"));
    assert_eq!(sel(&h), (2, 5));
}

#[test]
fn 読込の間にオーバーレイを開いていたら閉じた後に移る() {
    let mut h = repo(&[("/w/proj/a.txt", b"x\nhit\n")]);
    search(&mut h, "hit");
    h.keys("enter");
    // 読込の完了の前にパレットを開く: 表示の切り替えは閉じるまで保留される
    h.keys("ctrl+p");
    h.run_jobs();
    assert_eq!(shown_path(&h).as_deref(), Some("/w/proj/README.md"));
    h.keys("esc");
    assert_eq!(shown_path(&h).as_deref(), Some("/w/proj/a.txt"));
    assert_eq!(sel(&h), (2, 5), "切り替えた時点で移る");
}

// verifies: AC-FR-405-1
#[test]
fn ファイルが変わって位置が無ければ丸めてカーソルだけを置く() {
    let mut h = repo(&[("/w/proj/a.txt", b"x\ny\nzz hit\n")]);
    search(&mut h, "hit");
    h.keys("esc");
    // 結果の後にファイルが縮んだ: 3 行目 (0 始まりの 2) は無い
    h.fs.set_file(&FsPath::from("/w/proj/a.txt"), b"x\n");
    h.keys("ctrl+shift+f enter");
    h.run_jobs();
    assert_eq!(shown_path(&h).as_deref(), Some("/w/proj/a.txt"));
    let (a, b) = sel(&h);
    assert_eq!(a, b, "丸めたときは選択にしない");
    assert_eq!(a, 2, "最後の行 (空の行) の行末");
    // 行は残っているが短くなった: 範囲の終わりが行を越えるので、選択にせず始まりにカーソルを置く (次の行まで選ばない)
    let mut h = repo(&[("/w/proj/a.txt", b"x\ny\nzz hit\nnext\n")]);
    search(&mut h, "hit");
    h.keys("esc");
    h.fs.set_file(&FsPath::from("/w/proj/a.txt"), b"x\ny\nzz h\nnext\n");
    h.keys("ctrl+shift+f enter");
    h.run_jobs();
    assert_eq!(sel(&h), (7, 7));
}

#[test]
fn 読込の間に別の文書に切り替えたら移らない() {
    let mut h = repo(&[("/w/proj/a.txt", b"x\nhit\n"), ("/w/proj/b.txt", b"b\n")]);
    h.cmd_with("file.open", Args::new().with_str("path", "/w/proj/b.txt"));
    h.run_jobs();
    h.cmd_with(
        "file.open",
        Args::new().with_str("path", "/w/proj/README.md"),
    );
    h.run_jobs();
    search(&mut h, "hit");
    h.keys("enter");
    // a.txt の読込の完了の前に、開いた時点のペインを読み終えた b.txt に切り替える
    h.cmd_with("file.open", Args::new().with_str("path", "/w/proj/b.txt"));
    h.run_jobs();
    assert_eq!(
        shown_path(&h).as_deref(),
        Some("/w/proj/b.txt"),
        "切り替えを奪わない"
    );
    // 後から a.txt に切り替えても、一致の位置へは移っていない
    h.cmd_with("file.open", Args::new().with_str("path", "/w/proj/a.txt"));
    h.run_jobs();
    assert_eq!(shown_path(&h).as_deref(), Some("/w/proj/a.txt"));
    assert_eq!(sel(&h), (0, 0));
}

// ---- ジョブを直に走らせる ----

fn fs() -> (Arc<MemFs>, ManualClock) {
    let clock = ManualClock::new();
    (Arc::new(MemFs::new(Arc::new(clock.clone()))), clock)
}

fn job(root: &str, text: &str, open_docs: Vec<OpenDocRef>, max_lines: usize) -> JobKind {
    JobKind::WorkspaceSearch {
        root: FsPath::from(root),
        generation: 7,
        query: SearchQuery::literal(text).with_case(Case::Sensitive),
        exclude: Arc::new(ExcludeConfig::new()),
        open_docs,
        max_file_size: WS_MAX_FILE_SIZE,
        max_lines,
        state: None,
    }
}

/// 終わりまで (積み直しを繰り返して) 走らせ、送った結果と区切りの数を返す。
fn run_steps(fs: &Arc<MemFs>, clock: &ManualClock, kind: JobKind) -> (Vec<JobOutput>, usize) {
    let ports = JobPorts {
        stdin: None,
        fs: fs.clone(),
        clock: Arc::new(clock.clone()),
        paths: Arc::new(FixedPaths::default()),
    };
    let cancel = CancelToken::new();
    let mut out = Vec::new();
    let mut kind = kind;
    let mut steps = 0;
    loop {
        steps += 1;
        let step = run_job(
            JobRequest {
                id: eve_view::JobId(1),
                doc: None,
                base: eve_view::Revision(0),
                kind,
            },
            &ports,
            &cancel,
            &mut |o| out.push(o),
        );
        match step {
            eve_core::JobStep::Done => return (out, steps),
            eve_core::JobStep::Continue(k) => kind = k,
        }
        assert!(steps < 10_000);
    }
}

fn run_to_end(fs: &Arc<MemFs>, clock: &ManualClock, kind: JobKind) -> Vec<JobOutput> {
    run_steps(fs, clock, kind).0
}

/// 結果の行と打ち切りの印。
fn collect(out: &[JobOutput]) -> (Vec<WsHit>, bool) {
    let mut hits = Vec::new();
    let mut truncated = None;
    for o in out {
        match &o.body {
            JobResult::WorkspaceHits { hits: h, .. } => hits.extend(h.iter().cloned()),
            JobResult::WorkspaceSearchDone { truncated: t, .. } => truncated = Some(*t),
            other => panic!("想定外の結果: {other:?}"),
        }
    }
    (hits, truncated.expect("終端がある"))
}

#[test]
fn 一区切りで探すファイルは_64_個までで_続きを積み直す() {
    let (fs, clock) = fs();
    for i in 0..(WS_FILES_PER_STEP * 2 + 3) {
        fs.add_file(&format!("/r/f{i:03}.txt"), b"hit\n");
    }
    let (out, steps) = run_steps(&fs, &clock, job("/r", "hit", Vec::new(), 100_000));
    let first = out
        .iter()
        .find_map(|o| match &o.body {
            JobResult::WorkspaceHits { hits, .. } => Some(hits.len()),
            _ => None,
        })
        .expect("最初の束");
    assert_eq!(first, WS_FILES_PER_STEP, "最初の区切りは 64 ファイル");
    assert!(steps >= 3, "積み直した: {steps}");
    let (hits, truncated) = collect(&out);
    assert_eq!(hits.len(), WS_FILES_PER_STEP * 2 + 3);
    assert!(!truncated);
}

#[test]
fn 中身の合計が_1_mib_を超えたら区切る() {
    let (fs, clock) = fs();
    let big = format!("hit\n{}", "a".repeat(700 * 1024));
    for i in 0..4 {
        fs.add_file(&format!("/r/f{i}.txt"), big.as_bytes());
    }
    let (out, _) = run_steps(&fs, &clock, job("/r", "hit", Vec::new(), 100));
    let sizes: Vec<usize> = out
        .iter()
        .filter_map(|o| match &o.body {
            JobResult::WorkspaceHits { hits, .. } => Some(hits.len()),
            _ => None,
        })
        .collect();
    assert_eq!(
        sizes,
        [2, 2],
        "700 KiB のファイル 2 つで 1 MiB を超えたら区切る"
    );
}

#[test]
fn 取り消されたら_cancelled_で終わる() {
    let (fs, _clock) = fs();
    fs.add_file("/r/a.txt", b"hit\n");
    let ports = JobPorts {
        stdin: None,
        fs: fs.clone(),
        clock: Arc::new(ManualClock::new()),
        paths: Arc::new(FixedPaths::default()),
    };
    let cancel = CancelToken::new();
    cancel.cancel();
    let mut out = Vec::new();
    let step = run_job(
        JobRequest {
            id: eve_view::JobId(1),
            doc: None,
            base: eve_view::Revision(0),
            kind: job("/r", "hit", Vec::new(), 10),
        },
        &ports,
        &cancel,
        &mut |o| out.push(o),
    );
    assert!(step.is_done());
    assert!(matches!(
        &out[..],
        [JobOutput {
            body: JobResult::Failed {
                err: JobError::Cancelled
            },
            ..
        }]
    ));
}

// ---- 時計と取消しを差し込む FS ----

/// 偽 FS に、`open_read` と `read_dir` のたびに時計を進める・読み終えたら取り消すを足したもの (区切りの時間と、復号・照合の
/// 途中の取消しの試験)。
struct HookFs {
    inner: Arc<MemFs>,
    clock: ManualClock,
    open_ms: u64,
    dir_ms: u64,
    /// 立っていれば、このパスのファイルを読み終えた (最後のチャンクの後) 時点で取り消す。
    cancel_after_read: Option<(String, CancelToken)>,
}

/// 最後のチャンクの後で取消しを立てる読み手。
struct CancelAtEnd {
    inner: Box<dyn eve_core::ReadChunks + Send>,
    cancel: CancelToken,
}

impl eve_core::ReadChunks for CancelAtEnd {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, eve_core::ChunkError> {
        let r = self.inner.next_chunk()?;
        if r.is_none() {
            self.cancel.cancel();
        }
        Ok(r)
    }
}

impl eve_core::FileSystem for HookFs {
    fn metadata(&self, path: &FsPath) -> Result<eve_core::FileMeta, eve_core::FsError> {
        self.inner.metadata(path)
    }
    fn canonicalize(&self, path: &FsPath) -> Result<FsPath, eve_core::FsError> {
        self.inner.canonicalize(path)
    }
    fn read_dir(
        &self,
        dir: &FsPath,
        max_entries: usize,
    ) -> Result<eve_core::DirListing, eve_core::FsError> {
        self.clock.advance(eve_core::Millis(self.dir_ms));
        self.inner.read_dir(dir, max_entries)
    }
    fn open_read(
        &self,
        path: &FsPath,
    ) -> Result<Box<dyn eve_core::ReadChunks + Send>, eve_core::FsError> {
        self.clock.advance(eve_core::Millis(self.open_ms));
        let r = self.inner.open_read(path)?;
        match &self.cancel_after_read {
            Some((p, c)) if path.as_path().to_string_lossy().replace('\\', "/") == *p => {
                Ok(Box::new(CancelAtEnd {
                    inner: r,
                    cancel: c.clone(),
                }))
            }
            _ => Ok(r),
        }
    }
    fn write_atomic(
        &self,
        path: &FsPath,
        data: &mut dyn eve_core::ReadChunks,
        opts: &eve_core::SaveOptions,
    ) -> Result<eve_core::SaveReport, eve_core::FsError> {
        self.inner.write_atomic(path, data, opts)
    }
}

/// `fs` でジョブを終わりまで走らせ、送った結果と、区切りごとに進んだ時計の時間 (ms) を返す。
fn run_hooked(
    fs: Arc<HookFs>,
    clock: &ManualClock,
    kind: JobKind,
    cancel: &CancelToken,
) -> (Vec<JobOutput>, Vec<u64>) {
    let ports = JobPorts {
        stdin: None,
        fs,
        clock: Arc::new(clock.clone()),
        paths: Arc::new(FixedPaths::default()),
    };
    let mut out = Vec::new();
    let mut kind = kind;
    let mut spans = Vec::new();
    loop {
        let before = clock.now();
        let step = run_job(
            JobRequest {
                id: eve_view::JobId(1),
                doc: None,
                base: eve_view::Revision(0),
                kind,
            },
            &ports,
            cancel,
            &mut |o| out.push(o),
        );
        spans.push(clock.now().saturating_sub(before).0);
        match step {
            eve_core::JobStep::Done => return (out, spans),
            eve_core::JobStep::Continue(k) => kind = k,
        }
        assert!(spans.len() < 10_000, "終わらない");
    }
}

#[test]
fn 一区切りは_8_ms_で譲り_走査も残りの時間で区切る() {
    let (fs, clock) = fs();
    // ディレクトリ 40 個 (各 1 ファイル)。ファイルを開くのに 3 ms、ディレクトリを読むのに 2 ms かかる FS
    for i in 0..40 {
        fs.add_file(&format!("/r/d{i:02}/f.txt"), b"hit\n");
    }
    let hook = Arc::new(HookFs {
        inner: fs.clone(),
        clock: clock.clone(),
        open_ms: 3,
        dir_ms: 2,
        cancel_after_read: None,
    });
    let (out, spans) = run_hooked(
        hook,
        &clock,
        job("/r", "hit", Vec::new(), 1000),
        &CancelToken::new(),
    );
    let (hits, _) = collect(&out);
    assert_eq!(hits.len(), 40);
    // 区切りの始めから 8 ms を過ぎたら、次のファイルも次の走査も始めない。超えるのは、最後に始めた 1 つ (ファイル 3 ms・
    // ディレクトリ 2 ms) の分まで
    let max = spans.iter().copied().max().unwrap_or(0);
    assert!(
        max <= eve_core::WALK_STEP_TIME.0 + 3,
        "区切りの長さ (ms): {spans:?}"
    );
    assert!(spans.len() > 10, "区切った: {spans:?}");

    // 列が空になった時点で 8 ms に達していたら、走査も呼ばずに譲る (ファイル 4 ms・ディレクトリ 8 ms。a.txt と b.txt を探し
    // 終えた区切りで、続けて s/t を読むと 16 ms になる)
    let (fs, clock) = self::fs();
    for p in ["/r/a.txt", "/r/b.txt", "/r/s/c.txt", "/r/s/t/d.txt"] {
        fs.add_file(
            p, b"hit
",
        );
    }
    let hook = Arc::new(HookFs {
        inner: fs,
        clock: clock.clone(),
        open_ms: 4,
        dir_ms: 8,
        cancel_after_read: None,
    });
    let (out, spans) = run_hooked(
        hook,
        &clock,
        job("/r", "hit", Vec::new(), 1000),
        &CancelToken::new(),
    );
    assert_eq!(collect(&out).0.len(), 4);
    assert!(
        spans.iter().all(|ms| *ms <= eve_core::WALK_STEP_TIME.0 + 4),
        "区切りの長さ (ms): {spans:?}"
    );
}

// verifies: AC-FR-405-3
#[test]
fn 区切りの途中の取消しでは_その区切りの結果を送らない() {
    let (fs, clock) = fs();
    fs.add_file("/r/a.txt", b"hit\n");
    fs.add_file("/r/b.txt", b"hit\n");
    let cancel = CancelToken::new();
    // b.txt を読み終えた時点で取り消す: 同じ区切りで先に探した a.txt の結果も送らずに、取消しで終える (復号の中の取消しは
    // io_jobs.rs の単体試験で確かめる)
    let hook = Arc::new(HookFs {
        inner: fs,
        clock: clock.clone(),
        open_ms: 0,
        dir_ms: 0,
        cancel_after_read: Some(("/r/b.txt".to_owned(), cancel.clone())),
    });
    let (out, _) = run_hooked(hook, &clock, job("/r", "hit", Vec::new(), 10), &cancel);
    assert!(
        matches!(
            &out[..],
            [JobOutput {
                body: JobResult::Failed {
                    err: JobError::Cancelled
                },
                ..
            }]
        ),
        "取消しで終え、途中の結果 (a.txt) も送らない: {out:?}"
    );
}

// verifies: AC-FR-405-2
#[test]
fn 先頭_64_kib_の後に_shift_jis_が現れるファイルは判定し直してエディタと同じに一致する() {
    let mut data = "ascii line for the sniff window\n"
        .repeat(3000)
        .into_bytes();
    assert!(data.len() > 64 * 1024);
    data.extend(sjis(JP));
    same_as_editor(
        &[("/w/proj/late.txt", data)],
        SearchQuery::literal("文字コード"),
    );
}

fn doc_ref(path: &str, canonical: &str, id: Option<u128>, body: &str) -> OpenDocRef {
    OpenDocRef {
        path: FsPath::from(path),
        canonical: FsPath::from(canonical),
        file_id: id.map(|index| FileId { volume: 1, index }),
        file_id_unique: true,
        body: OpenDocBody::Text(eve_text::Snapshot::from(body)),
    }
}

#[test]
fn ルートの下の同じ名前のハードリンクは識別で突き合わせ_古い識別では突き合わせない() {
    let (fs, clock) = fs();
    fs.set_file_id_unique(true);
    // /r/x/a.txt と /r/y/a.txt は同じファイル (ハードリンク)。文書は /r/x/a.txt で開いている
    fs.add_file("/r/x/a.txt", b"disk\n");
    fs.add_hardlink("/r/y/a.txt", "/r/x/a.txt");
    let id = fs.file_id_of("/r/x/a.txt").expect("識別");
    let doc = || doc_ref("/r/x/a.txt", "/r/x/a.txt", Some(id), "buffer\n");
    let (hits, _) = collect(&run_to_end(
        &fs,
        &clock,
        job("/r", "buffer", vec![doc()], 10),
    ));
    let rels: Vec<&str> = hits.iter().map(|h| &*h.rel).collect();
    assert_eq!(
        rels,
        ["x/a.txt", "y/a.txt"],
        "(i) と (ii) の両方で文書の本文を探す"
    );
    assert!(hits.iter().all(|h| h.from_open_doc));
    // 文書のパスのファイルが作り直されて識別が変わった: y/a.txt とは識別で比べず、ディスク (y/a.txt の中身) を探す
    fs.set_file_id("/r/x/a.txt", id + 1000);
    let (hits, _) = collect(&run_to_end(
        &fs,
        &clock,
        job("/r", "buffer", vec![doc()], 10),
    ));
    let rels: Vec<&str> = hits.iter().map(|h| &*h.rel).collect();
    assert_eq!(rels, ["x/a.txt"], "古い識別では突き合わせない");
    let (hits, _) = collect(&run_to_end(&fs, &clock, job("/r", "disk", vec![doc()], 10)));
    let rels: Vec<(&str, bool)> = hits.iter().map(|h| (&*h.rel, h.from_open_doc)).collect();
    assert_eq!(rels, [("y/a.txt", false)]);
}

#[test]
fn 開いている文書の上限超えとバイナリは飛ばしてディスクも探さない() {
    let (fs, clock) = fs();
    fs.add_file("/r/a.txt", b"hit\n");
    fs.add_file("/r/b.txt", b"hit\n");
    let mut big = doc_ref("/r/a.txt", "/r/a.txt", None, "");
    big.body = OpenDocBody::TooLarge;
    let mut bin = doc_ref("/r/b.txt", "/r/b.txt", None, "");
    bin.body = OpenDocBody::Binary;
    let out = run_to_end(&fs, &clock, job("/r", "hit", vec![big, bin], 10));
    let (hits, _) = collect(&out);
    assert_eq!(hits, Vec::<WsHit>::new());
    let p = out
        .iter()
        .find_map(|o| match &o.body {
            JobResult::WorkspaceSearchDone { progress, .. } => Some(*progress),
            _ => None,
        })
        .unwrap();
    assert_eq!((p.skipped_large, p.skipped_binary), (1, 1));
}

#[test]
fn 抜粋の外の一致は範囲に持たず_最初の一致は行の中の位置() {
    let (fs, clock) = fs();
    let line = format!("{}hit{}hit", "x".repeat(300), "y".repeat(300));
    fs.add_file("/r/a.txt", line.as_bytes());
    let (hits, _) = collect(&run_to_end(&fs, &clock, job("/r", "hit", Vec::new(), 10)));
    assert_eq!(hits.len(), 1);
    assert_eq!(
        (hits[0].ranges.len(), hits[0].ranges[0].clone()),
        (1, 300..303),
        "2 つ目は抜粋の窓の外"
    );
    assert!(hits[0].excerpt.contains("hit"));
    assert!(hits[0].excerpt.len() <= 256);
}
