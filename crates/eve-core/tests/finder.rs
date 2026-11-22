//! ファインダ (docs/03 §12.7.4、04 §8.11、01 FR-125。v0.5 の M31 の段 6)。
//!
//! 偽 FS の木で、開く・絞り込む・選んで開く・キャンセル・同じファイル・消えたファイル・照合の取消しと世代・最初の走査の間の
//! 照合・一覧の寿命・起動の `DIR` を確かめる。偽 FS のカレントディレクトリは `/w`。

use eve_core::{FsPath, InitialFile, JobResult};
use eve_testkit::Harness;
use eve_view::{OverlayModel, Span};

const NOT_FOUND: &str = "msg.finder.not_found";

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

/// 出している行 (相対パス) と選択中の行。
fn rows(h: &Harness) -> (Vec<String>, usize) {
    let (_, _, rows, sel) = h
        .editor
        .state()
        .finder_rows()
        .expect("ファインダが開いている");
    (rows, sel)
}

/// 一覧のオーバーレイの (行の文字列, 強調した Span の文字列, 件数の欄)。
fn list_model(h: &mut Harness) -> (Vec<String>, Vec<String>, String) {
    let f = h.frame();
    let Some(OverlayModel::List { items, footer, .. }) = f.overlays.first() else {
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
    (labels, marked, text(&footer.spans))
}

/// 表示中の文書のパス (区切りを `/` に揃える。候補のパスはルートに OS の区切りで足すので、偽 FS のパスの書き方と違い得る)。
fn shown_path(h: &Harness) -> Option<String> {
    h.doc_path().map(|p| p.replace('\\', "/"))
}

fn count(h: &Harness, key: &str) -> usize {
    h.messages().iter().filter(|k| *k == key).count()
}

// ---- 開く・絞り込む・選んで開く (AC-1) ----

// verifies: AC-FR-125-1
#[test]
fn ctrl_o_でファインダを開き_絞り込んで選んだファイルを開く() {
    let mut h = repo(&[
        ("/w/proj/src/main.rs", b"fn main() {}\n"),
        ("/w/proj/src/lib.rs", b"\n"),
        ("/w/proj/docs/guide.md", b"\n"),
    ]);
    h.keys("ctrl+o");
    assert_eq!(h.overlay(), "finder");
    h.run_jobs();
    let (all, _) = rows(&h);
    assert_eq!(
        all,
        ["README.md", "src/lib.rs", "src/main.rs", "docs/guide.md"],
        "空の検索語は相対パスの短い順 (同じ長さはバイト列の順)"
    );
    h.text("main");
    h.run_jobs();
    assert_eq!(rows(&h).0, ["src/main.rs"]);
    let (_, marked, footer) = list_model(&mut h);
    assert_eq!(marked, ["main"], "一致した文字を強調する");
    assert_eq!(footer, "1/4");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "", "開いたら閉じる");
    assert_eq!(shown_path(&h).as_deref(), Some("/w/proj/src/main.rs"));
    assert_eq!(h.editor.state().documents_len(), 2);
}

// verifies: AC-FR-125-1
#[test]
fn gitignore_と既定の除外に当たるファイルは出ず_ドットで始まる名前は出る() {
    let mut h = repo(&[
        ("/w/proj/.gitignore", b"*.log\n"),
        ("/w/proj/debug.log", b"\n"),
        ("/w/proj/.env", b"\n"),
        ("/w/proj/node_modules/x/index.js", b"\n"),
        ("/w/proj/target/debug/app", b"\n"),
        ("/w/proj/src/a.rs", b"\n"),
    ]);
    h.keys("ctrl+o");
    h.run_jobs();
    let (mut all, _) = rows(&h);
    all.sort();
    assert_eq!(all, [".env", ".gitignore", "README.md", "src/a.rs"]);
}

// verifies: AC-FR-125-1
#[test]
fn ファイル名の中で一致する候補を上に並べる() {
    let mut h = repo(&[
        ("/w/proj/ma/in/x.rs", b"\n"),
        ("/w/proj/src/domain.rs", b"\n"),
        ("/w/proj/deep/dir/main.rs", b"\n"),
    ]);
    h.keys("ctrl+o");
    h.text("main");
    h.run_jobs();
    let (r, sel) = rows(&h);
    assert_eq!(
        r,
        ["deep/dir/main.rs", "src/domain.rs", "ma/in/x.rs"],
        "ファイル名の中の一致 (語頭・連続) → ファイル名の中の一致 (語の途中) → ディレクトリにまたがる一致"
    );
    assert_eq!(sel, 0);
}

// verifies: AC-FR-125-1
#[test]
fn キャンセルでは何も変わらない() {
    let mut h = repo(&[("/w/proj/src/main.rs", b"\n")]);
    let doc = h.doc_id();
    h.keys("ctrl+o");
    h.run_jobs();
    h.text("main");
    h.run_jobs();
    h.keys("esc");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_id(), doc);
    assert_eq!(h.editor.state().documents_len(), 1);
    assert!(h.messages().is_empty(), "{:?}", h.messages());
}

// verifies: AC-FR-125-1
#[test]
fn 開いている文書と同じファイルを選ぶと切り替えるだけ() {
    let mut h = repo(&[("/w/proj/src/main.rs", b"\n")]);
    h.open("/w/proj/src/main.rs");
    h.run_jobs();
    h.keys("ctrl+o");
    h.text("readme");
    h.run_jobs();
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.doc_path().as_deref(), Some("/w/proj/README.md"));
    assert_eq!(h.editor.state().documents_len(), 2, "新しく開かない");
}

// verifies: AC-FR-125-1
#[test]
fn 走査の後に消えたファイルは新規の文書にせず知らせる() {
    let mut h = repo(&[("/w/proj/gone.txt", b"\n")]);
    h.keys("ctrl+o");
    h.text("gone");
    h.run_jobs();
    assert_eq!(rows(&h).0, ["gone.txt"]);
    h.fs.remove(&FsPath::from("/w/proj/gone.txt"));
    h.keys("enter");
    h.run_jobs();
    assert_eq!(count(&h, NOT_FOUND), 1, "{:?}", h.messages());
    assert_eq!(
        h.editor.state().documents_len(),
        1,
        "空の新規の文書を作らない"
    );
    assert_eq!(h.doc_path().as_deref(), Some("/w/proj/README.md"));
    assert!(!h.fs.exists("/w/proj/gone.txt"));
}

#[test]
fn パスを入力する_file_open_はパレットから使える() {
    let mut h = repo(&[]);
    h.cmd("file.open");
    assert_eq!(h.overlay(), "prompt");
}

// ---- 照合の取消しと世代 (AC-2) ----

// verifies: AC-FR-125-2
#[test]
fn 入力が変わると前の照合を取り消して投げ直す() {
    let mut h = repo(&[("/w/proj/abc.txt", b"\n"), ("/w/proj/xyz.txt", b"\n")]);
    h.keys("ctrl+o");
    h.run_jobs();
    assert_eq!(rows(&h).0.len(), 3);
    h.text("a");
    h.text("b");
    // 前の照合 (`a`) は実行される前に取り消される
    let mut cancelled = 0;
    let mut matched = 0;
    h.run_jobs_with(|o| match o.body {
        JobResult::Failed { .. } => cancelled += 1,
        JobResult::FinderMatched { .. } => matched += 1,
        _ => {}
    });
    assert_eq!((cancelled, matched), (1, 1));
    assert_eq!(rows(&h).0, ["abc.txt"]);
}

// verifies: AC-FR-125-2
#[test]
fn 古い世代の照合の結果は捨てる() {
    let mut h = repo(&[("/w/proj/abc.txt", b"\n"), ("/w/proj/xyz.txt", b"\n")]);
    h.keys("ctrl+o");
    h.run_jobs();
    let before = rows(&h).0;
    assert_eq!(before.len(), 3);
    // `zz` の照合を実行し、結果を渡す前に入力を変える (取消しが間に合わなかった結果)
    h.text("zz");
    let stale = h.run_one_job_held();
    assert!(
        stale
            .iter()
            .any(|o| matches!(&o.body, JobResult::FinderMatched { matched: 0, .. })),
        "{stale:?}"
    );
    h.keys("backspace");
    h.keys("backspace");
    h.text("y");
    h.deliver(stale);
    assert_eq!(rows(&h).0, before, "古い世代 (`zz`) の 0 件で行を消さない");
    h.run_jobs();
    assert_eq!(rows(&h).0, ["xyz.txt"]);
}

// verifies: AC-FR-125-2
#[test]
fn 照合を待つ間の_enter_は結果が届いてから開く() {
    let mut h = repo(&[("/w/proj/abc.txt", b"\n"), ("/w/proj/xyz.txt", b"\n")]);
    h.keys("ctrl+o");
    h.run_jobs();
    h.text("xyz");
    // 行はまだ空の検索語のもの (先頭は README.md)。ここで開くと別のファイルになる
    h.keys("enter");
    assert_eq!(h.overlay(), "finder", "結果を待つ");
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(shown_path(&h).as_deref(), Some("/w/proj/xyz.txt"));
}

#[test]
fn 同じ検索語の照合し直しでは選択を同じパスに保つ() {
    let mut h = repo(&[("/w/proj/ab.txt", b"\n"), ("/w/proj/abc.txt", b"\n")]);
    h.keys("ctrl+o");
    h.run_jobs();
    h.keys("esc");
    // 開き直すと前の一覧を見せたまま走査する。走査の結果を渡す前に `ab` の照合を走らせて選ぶ
    h.fs.add_file("/w/proj/aab.txt", b"\n");
    h.keys("ctrl+o");
    h.text("ab");
    let scan = h.run_one_job_held();
    assert!(h.run_one_job());
    assert_eq!(rows(&h).0, ["ab.txt", "abc.txt"]);
    h.keys("down");
    assert_eq!(rows(&h).1, 1);
    // 走査し直した一覧への差し替えの後の照合は同じ検索語: 選択していたパスが残っていれば保つ
    h.deliver(scan);
    h.run_jobs();
    let (r, sel) = rows(&h);
    assert_eq!(r.len(), 3, "{r:?}");
    assert_eq!(r[sel], "abc.txt", "選択していたパスを保つ: {r:?}");
}

#[test]
fn 入力を変えたら次の結果では先頭を選ぶ() {
    let mut h = repo(&[("/w/proj/ab.txt", b"\n"), ("/w/proj/abc.txt", b"\n")]);
    h.keys("ctrl+o");
    h.text("ab");
    h.run_jobs();
    h.keys("down");
    assert_eq!(rows(&h).1, 1);
    h.text(".");
    h.run_jobs();
    let (r, sel) = rows(&h);
    assert!(
        r.contains(&"abc.txt".to_owned()),
        "前の選択も結果に残る: {r:?}"
    );
    assert_eq!(sel, 0, "入力を変えたら先頭: {r:?}");
    // `Enter` で開くのは今の検索語の最上位
    h.keys("enter");
    h.run_jobs();
    assert_eq!(
        shown_path(&h).as_deref(),
        Some(&*format!("/w/proj/{}", r[0]))
    );
}

// ---- 走査の間 (AC-3) ----

// verifies: AC-FR-125-3
#[test]
fn 走査が終わる前もそれまでの候補で絞り込め_走査中と件数を示す() {
    // 1 区切りは 4,096 項目。3 区切りかかる木
    // ルートはカレントディレクトリ `/w` (目印が無い)
    let mut b = Harness::builder().defer_start();
    for i in 0..10_000 {
        b = b.file(&format!("/w/a{i:05}.txt"), b"");
    }
    let mut h = b.build();
    h.editor.open_initial_files(&[], false, None);
    h.started();
    h.run_jobs();
    h.keys("ctrl+o");
    h.text("a");
    // 1 区切り目: 4,096 件の束が届き、すぐに照合を投げる
    assert!(h.run_one_job());
    let total = |h: &Harness| h.editor.state().finder_rows().expect("開いている").0;
    assert_eq!(total(&h), 4096);
    // 2 区切り目 (前の照合から 100 ms 経っていないので、まだ投げ直さない)
    assert!(h.run_one_job());
    assert_eq!(total(&h), 8192);
    // 1 回目の照合 (最初の束だけ) が届く
    assert!(h.run_one_job());
    let (r, _) = rows(&h);
    assert_eq!(r.len(), 100, "上位 100 件");
    assert_eq!(r[0], "a00000.txt");
    let (_, _, footer) = list_model(&mut h);
    assert_eq!(footer, "4096/8192 走査中…");
    assert_eq!(h.pending_jobs(), 1, "走査の続きだけ (照合は間隔を待つ)");
    // 100 ms 経つと、届いた候補で照合し直す
    h.advance_ms(100);
    assert_eq!(h.pending_jobs(), 2);
    h.run_jobs();
    let (_, _, footer) = list_model(&mut h);
    assert_eq!(footer, "10000/10000");
}

// verifies: AC-FR-125-3
#[test]
fn ルートが決まる前に開くと待ち_決まったら走査する() {
    let mut h = Harness::builder()
        .defer_start()
        .dir("/w/proj/.git")
        .file("/w/proj/a.txt", b"\n")
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/proj/a.txt")], false, None);
    h.started();
    h.keys("ctrl+o");
    let (_, _, footer) = list_model(&mut h);
    assert_eq!(footer, "ワークスペースを調べています…");
    h.run_jobs();
    assert_eq!(rows(&h).0, ["a.txt"]);
}

// verifies: AC-FR-125-3
#[test]
fn 起動の_dir_でファインダが開く() {
    let mut h = Harness::builder()
        .defer_start()
        .dir("/w/proj")
        .file("/w/proj/a.txt", b"\n")
        .build();
    h.editor
        .open_initial_files(&[InitialFile::path("/w/proj")], false, None);
    h.started();
    h.run_jobs();
    assert_eq!(h.overlay(), "finder");
    assert_eq!(rows(&h).0, ["a.txt"]);
}

#[test]
fn 起動の_file_だけではファインダを開かない() {
    let h = repo(&[]);
    assert_eq!(h.overlay(), "");
}

// verifies: AC-FR-125-3
#[test]
fn 読めないフォルダがあっても走査は止まらず数を示す() {
    let mut h = repo(&[("/w/proj/locked/x.txt", b"\n"), ("/w/proj/ok/y.txt", b"\n")]);
    h.fs.set_unreadable("/w/proj/locked", true);
    h.keys("ctrl+o");
    h.run_jobs();
    let (labels, _, footer) = list_model(&mut h);
    assert!(labels.contains(&"ok/y.txt".to_owned()), "{labels:?}");
    assert_eq!(footer, "2/2・読めないフォルダ 1");
}

// ---- 一覧の寿命 ----

#[test]
fn 開き直すと前の一覧を見せたまま走査し_終わったら差し替える() {
    let mut h = repo(&[("/w/proj/a.txt", b"\n")]);
    h.keys("ctrl+o");
    h.run_jobs();
    h.keys("esc");
    h.fs.add_file("/w/proj/b.txt", b"\n");
    h.keys("ctrl+o");
    // 前の一覧で照合し直す (新しい走査の束は混ぜない)
    let state = h.editor.state().finder_rows().expect("開いている");
    assert!(state.1, "走査中");
    assert_eq!(state.0, 2);
    // 走査 (1 区切りで終わる) の結果を渡す前に、前の一覧の照合を走らせる
    let scan = h.run_one_job_held();
    assert!(
        scan.iter()
            .any(|o| matches!(o.body, JobResult::ScanDone { total: 3, .. })),
        "{scan:?}"
    );
    assert!(h.run_one_job());
    assert_eq!(rows(&h).0, ["a.txt", "README.md"], "前の一覧のまま");
    h.deliver(scan);
    h.run_jobs();
    assert_eq!(rows(&h).0, ["a.txt", "b.txt", "README.md"]);
}

#[test]
fn 閉じて_5_分開かなければ一覧を_bg_で手放す() {
    let mut h = repo(&[("/w/proj/a.txt", b"\n")]);
    h.keys("ctrl+o");
    h.run_jobs();
    h.keys("esc");
    assert!(h.editor.state().finder_list_held(), "閉じても持つ");
    h.advance_ms(5 * 60 * 1000 - 1);
    assert!(h.editor.state().finder_list_held());
    h.advance_ms(1);
    assert!(!h.editor.state().finder_list_held(), "手放した");
    let mut released = 0;
    h.run_jobs_with(|o| {
        if matches!(o.body, JobResult::Released) {
            released += 1;
        }
    });
    assert_eq!(
        released, 1,
        "主スレッドで drop せず ReleaseCandidates に渡す"
    );
    // 次に開くと最初の走査から
    h.keys("ctrl+o");
    h.run_jobs();
    assert_eq!(rows(&h).0, ["a.txt", "README.md"]);
}

// ---- 表示 ----

#[test]
fn 双方向の制御文字は可視の記法にし_強調は記法の後の位置() {
    let mut h = repo(&[("/w/proj/a\u{202E}b.txt", b"\n")]);
    h.keys("ctrl+o");
    h.text("b.t");
    h.run_jobs();
    let (labels, marked, _) = list_model(&mut h);
    assert_eq!(labels.len(), 1);
    assert!(
        !labels[0].contains('\u{202E}'),
        "制御文字をそのまま出さない: {labels:?}"
    );
    assert_eq!(marked, ["b.t"]);
}

#[test]
fn 全角英字は畳まない() {
    let mut h = repo(&[("/w/proj/ＡＢ.txt", b"\n")]);
    h.keys("ctrl+o");
    h.text("ａｂ");
    h.run_jobs();
    assert_eq!(rows(&h).0, Vec::<String>::new());
    h.keys("ctrl+a");
    h.text("ＡＢ");
    h.run_jobs();
    assert_eq!(rows(&h).0, ["ＡＢ.txt"]);
}

#[test]
fn 一致が無ければ_0_件の文言() {
    let mut h = repo(&[]);
    h.keys("ctrl+o");
    h.text("zzzz");
    h.run_jobs();
    let (labels, _, footer) = list_model(&mut h);
    assert_eq!(labels, Vec::<String>::new());
    assert_eq!(footer, "0/1");
    let f = h.frame();
    let Some(OverlayModel::List { no_match, .. }) = f.overlays.first() else {
        panic!("一覧のオーバーレイが無い");
    };
    let text: String = no_match.spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(text, "一致するファイルはありません");
}

// ---- 照合の結果の並び (ファイル名の中で当てる) ----

// verifies: AC-FR-125-1
#[test]
fn ディレクトリ名にも同じ語がある候補もファイル名の一致として上に並べる() {
    let mut h = repo(&[
        ("/w/proj/lib/domain.rs", b"\n"),
        ("/w/proj/m/a/i/n.rs", b"\n"),
        ("/w/proj/src/main/java/Main.java", b"\n"),
    ]);
    h.keys("ctrl+o");
    h.text("main");
    h.run_jobs();
    let (r, _) = rows(&h);
    assert_eq!(r[0], "src/main/java/Main.java", "{r:?}");
    let (labels, marked, _) = list_model(&mut h);
    assert_eq!(labels[0], "src/main/java/Main.java");
    assert_eq!(marked[0], "Main", "強調はファイル名の中の窓");
}

// ---- 失敗 ----

// verifies: AC-FR-125-3
#[test]
fn 最初の走査がルートを読めずに失敗したら知らせ_次の走査は途中の候補を出す() {
    let mut h = repo(&[]);
    h.fs.set_unreadable("/w/proj", true);
    h.keys("ctrl+o");
    h.run_jobs();
    assert_eq!(
        count(&h, "msg.workspace.scan_failed"),
        1,
        "{:?}",
        h.messages()
    );
    let (_, _, footer) = list_model(&mut h);
    assert_eq!(footer, "0/0 (一覧が途中までです)", "完了に見せない");
    h.keys("esc");
    h.fs.set_unreadable("/w/proj", false);
    for i in 0..9000 {
        h.fs.add_file(&format!("/w/proj/f{i:04}.txt"), b"");
    }
    h.keys("ctrl+o");
    // 束 (4,096 件) が、走査の途中から一覧に入る (前の空の一覧を見せたまま、終わるまで待たない)
    let state = |h: &Harness| {
        let (total, scanning, _, _) = h.editor.state().finder_rows().expect("開いている");
        (total, scanning)
    };
    while state(&h) == (0, true) {
        assert!(h.run_one_job(), "走査が進まない");
    }
    let (total, scanning) = state(&h);
    assert!(scanning, "走査の途中で候補が入る");
    assert!((1..9001).contains(&total), "{total}");
    h.run_jobs();
    let (_, _, footer) = list_model(&mut h);
    assert_eq!(footer, "9001/9001");
}

#[test]
fn 走査のジョブがパニックしたら知らせ_件数の欄を完了に見せない() {
    let mut h = repo(&[("/w/proj/a.txt", b"\n")]);
    h.keys("ctrl+o");
    let held = h.run_one_job_held();
    let job = held.first().expect("走査の結果").job;
    h.deliver(vec![eve_core::JobOutput {
        job,
        body: JobResult::Failed {
            err: eve_core::JobError::Panic,
        },
    }]);
    h.run_jobs();
    assert_eq!(
        count(&h, "msg.workspace.scan_failed"),
        1,
        "{:?}",
        h.messages()
    );
    let (_, _, footer) = list_model(&mut h);
    assert_eq!(footer, "0/0 (一覧が途中までです)");
}

#[test]
fn 照合のジョブが失敗したら_enter_の待ちを解き_古い行を開かない() {
    let mut h = repo(&[("/w/proj/abc.txt", b"\n"), ("/w/proj/xyz.txt", b"\n")]);
    let doc = h.doc_id();
    h.keys("ctrl+o");
    h.run_jobs();
    h.text("xyz");
    h.keys("enter");
    let held = h.run_one_job_held();
    let job = held
        .iter()
        .find(|o| matches!(o.body, JobResult::FinderMatched { .. }))
        .expect("照合の結果")
        .job;
    h.deliver(vec![eve_core::JobOutput {
        job,
        body: JobResult::Failed {
            err: eve_core::JobError::Panic,
        },
    }]);
    assert_eq!(h.overlay(), "finder", "失敗の結果では開かない");
    // 行は空の検索語のもの (先頭は README.md)。`Enter` でも開かない
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.overlay(), "finder");
    assert_eq!(h.doc_id(), doc);
    assert_eq!(h.editor.state().documents_len(), 1);
}

#[test]
fn リンク先の無いリンクを選ぶとリンク先が無いと知らせる() {
    let mut h = repo(&[]);
    h.fs.add_link("/w/proj/dead.txt", "/w/proj/nowhere.txt");
    h.keys("ctrl+o");
    h.text("dead");
    h.run_jobs();
    assert_eq!(rows(&h).0, ["dead.txt"]);
    h.keys("enter");
    h.run_jobs();
    assert_eq!(
        count(&h, "msg.finder.dangling_link"),
        1,
        "{:?}",
        h.messages()
    );
    assert_eq!(count(&h, NOT_FOUND), 0);
    assert_eq!(h.editor.state().documents_len(), 1, "新規の文書にしない");
}

#[test]
fn 上限で打ち切った一覧は件数の欄で示す() {
    let mut h = repo(&[("/w/proj/a.txt", b"\n")]);
    h.keys("ctrl+o");
    let held = h.run_one_job_held();
    let held = held
        .into_iter()
        .map(|mut o| {
            if let JobResult::ScanDone { truncated, .. } = &mut o.body {
                *truncated = true;
            }
            o
        })
        .collect();
    h.deliver(held);
    h.run_jobs();
    let (_, _, footer) = list_model(&mut h);
    assert_eq!(footer, "2/2 (ファイルが多いため 200000 件で打ち切りました)");
}

#[test]
fn 走査し直した一覧に差し替えたら古い一覧を_bg_で手放す() {
    let mut h = repo(&[("/w/proj/a.txt", b"\n")]);
    h.keys("ctrl+o");
    h.run_jobs();
    h.keys("esc");
    h.keys("ctrl+o");
    let scan = h.run_one_job_held();
    h.deliver(scan);
    // 差し替えの直後に積まれるのは古い一覧の解放 (照合し直しより先)
    let next = h.run_one_job_held();
    assert!(
        next.iter().any(|o| matches!(o.body, JobResult::Released)),
        "{next:?}"
    );
    h.deliver(next);
    h.run_jobs();
    assert_eq!(rows(&h).0, ["a.txt", "README.md"]);
}

#[test]
fn 照合の失敗で_enter_の待ちを解き_後の同じ検索語の結果で開かない() {
    let mut h = repo(&[("/w/proj/xyz.txt", b"\n")]);
    h.keys("ctrl+o");
    h.run_jobs();
    h.keys("esc");
    // 開き直す: 走査し直しと、前の一覧の照合が積まれる
    h.keys("ctrl+o");
    h.text("xyz");
    h.keys("enter");
    let scan = h.run_one_job_held();
    let held = h.run_one_job_held();
    let job = held
        .iter()
        .find(|o| matches!(o.body, JobResult::FinderMatched { .. }))
        .expect("照合の結果")
        .job;
    h.deliver(vec![eve_core::JobOutput {
        job,
        body: JobResult::Failed {
            err: eve_core::JobError::Panic,
        },
    }]);
    // 差し替えの後の同じ検索語の照合し直しの結果が届いても、待ちは解けているので開かない
    h.deliver(scan);
    h.run_jobs();
    assert_eq!(rows(&h).0, ["xyz.txt"]);
    assert_eq!(h.overlay(), "finder", "失敗の前に押した Enter で開かない");
}
