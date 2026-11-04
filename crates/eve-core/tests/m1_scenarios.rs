//! M1 の受入基準 (AC) のシナリオ (docs/08 §2.2「M1 で通す AC」、01 §5)。`Harness` (eve-testkit) で書く。
//! 各テストの直前の `// verifies:` が、`tests/ac-checklist.toml` の `verified_by` に書く AC の ID。
//! 既存の単体・結合テストが検証している AC は、ここには重ねず ac-checklist から直接それを指す。

use eve_core::Args;
use eve_testkit::Harness;
use eve_view::{Answer, Query};

/// 主 Range の (anchor, head)。
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

fn opened(path: &str, data: &[u8]) -> Harness {
    let mut h = Harness::new().with_file(path, data);
    h.open(path);
    h.run_jobs();
    h
}

// verifies: AC-FR-201-1
#[test]
fn 書記素クラスタは_backspace_と_delete_で_1_回で消える() {
    for cluster in [
        "e\u{301}",
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        "\u{1F1EF}\u{1F1F5}",
        "\u{304B}\u{309A}",
    ] {
        // 直後で Backspace
        let mut h = opened("/w/a.txt", format!("a{cluster}b").as_bytes());
        h.keys("ctrl+home right right");
        assert_eq!(sel(&h).1, (1 + cluster.len()) as u64, "{cluster:?}");
        h.keys("backspace");
        assert_eq!(h.doc_text(), "ab", "Backspace {cluster:?}");
        // 直前で Delete
        let mut h = opened("/w/a.txt", format!("a{cluster}b").as_bytes());
        h.keys("ctrl+home right delete");
        assert_eq!(h.doc_text(), "ab", "Delete {cluster:?}");
        // 1 回の Undo で戻る
        h.keys("ctrl+z");
        assert_eq!(h.doc_text(), format!("a{cluster}b"));
    }
}

// verifies: AC-FR-301-1
#[test]
fn 左右の移動は全角_結合文字_絵文字_国旗を_1_書記素ずつ() {
    let parts = [
        "\u{3042}",
        "e\u{301}",
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        "\u{1F1EF}\u{1F1F5}",
        "z",
    ];
    let mut h = opened("/w/a.txt", parts.concat().as_bytes());
    h.keys("ctrl+home");
    let mut at = 0u64;
    for p in parts {
        h.keys("right");
        at += p.len() as u64;
        assert_eq!(sel(&h), (at, at), "→ {p:?}");
    }
    for p in parts.iter().rev() {
        h.keys("left");
        at -= p.len() as u64;
        assert_eq!(sel(&h), (at, at), "← {p:?}");
    }
}

// verifies: AC-FR-301-2
#[test]
fn 上下の移動は全角と半角の混在する行の間で目標の表示列を保つ() {
    // 表示列: 1 行目 (全角) 0,2,4,6,8 / 2 行目 (半角) 0..8 / 3 行目 (全角)
    let doc = "あいうえお\nabcdefgh\nかきくけこ";
    let mut h = opened("/w/a.txt", doc.as_bytes());
    h.keys("ctrl+home right right"); // 1 行目の表示列 4 (「あい」の後ろ、バイト 6)
    assert_eq!(sel(&h).1, 6);
    h.keys("down");
    assert_eq!(sel(&h).1, 16 + 4, "2 行目の表示列 4 (abcd の後ろ)");
    h.keys("down");
    assert_eq!(sel(&h).1, 16 + 9 + 6, "3 行目の表示列 4 (かき の後ろ)");
    h.keys("up");
    assert_eq!(sel(&h).1, 16 + 4);
    h.keys("up");
    assert_eq!(sel(&h).1, 6, "往復しても 1 行目の元の位置");
    // 奇数列の目標: 全角の途中は手前に寄せ、目標列は保たれる
    h.keys("ctrl+home right");
    h.keys("right"); // 表示列 4
    h.keys("left"); // 表示列 2 (水平移動で目標列が更新される)
    h.keys("down");
    assert_eq!(sel(&h).1, 16 + 2);
}

// verifies: AC-FR-304-1
#[test]
fn ページ移動は可視行数マイナス1行ずつ進み_カーソルは画面内に留まる() {
    let doc: String = (0..60).map(|i| format!("line{i}\n")).collect();
    let mut h = Harness::new()
        .with_size(40, 10)
        .with_file("/w/a.txt", doc.as_bytes());
    h.open("/w/a.txt");
    h.run_jobs();
    let visible = h.frame().panes[0].rows.len();
    assert!(visible >= 4, "{visible}");
    let cursor_row = |h: &mut Harness| h.frame().panes[0].cursors[0].row as usize;
    let mut expect_line = 0usize;
    for _ in 0..4 {
        h.keys("pagedown");
        expect_line += visible - 1;
        assert!(cursor_row(&mut h) < visible, "画面内");
        // 現在の行は "line<N>" の N
        let text = h.doc_text();
        let head = sel(&h).1 as usize;
        let n = text[..head].matches('\n').count();
        assert_eq!(n, expect_line, "可視行数 − 1 行ずつ");
    }
    for _ in 0..4 {
        h.keys("pageup");
        expect_line -= visible - 1;
        let text = h.doc_text();
        let head = sel(&h).1 as usize;
        assert_eq!(text[..head].matches('\n').count(), expect_line);
        assert!(cursor_row(&mut h) < visible);
    }
}

// verifies: AC-FR-305-1
#[test]
fn shift_付きの各移動で選択が拡張され_全選択で文書全体になる() {
    let doc = "abc def\nghi jkl\nmno pqr";
    let mut h = opened("/w/a.txt", doc.as_bytes());
    h.keys("ctrl+home right right"); // 2
    for (keys, expect_head) in [
        ("shift+right", 3),
        ("shift+left", 2),
        ("ctrl+shift+right", 4),
        ("ctrl+shift+left", 0),
        ("shift+down", 8),
        ("shift+up", 0),
        ("shift+end", 7),
        ("shift+home", 0),
    ] {
        h.keys(keys);
        let (anchor, head) = sel(&h);
        assert_eq!(anchor, 2, "{keys}: anchor は動かない");
        assert_eq!(head, expect_head, "{keys}");
    }
    h.keys("ctrl+home shift+pagedown");
    let (a, hd) = sel(&h);
    assert_eq!(a, 0);
    assert!(hd > 0, "ページ単位の拡張");
    h.keys("ctrl+a");
    assert_eq!(sel(&h), (0, doc.len() as u64), "全選択");
}

// verifies: AC-FR-124-1
#[test]
fn 新規バッファを作り_保存すると名前を尋ねられる() {
    let mut h = Harness::new();
    h.keys("ctrl+n");
    assert_eq!(h.overlay(), "", "clean なら確認なしで作られる");
    assert_eq!(h.doc_path(), None);
    assert_eq!(h.doc_text(), "");
    h.text("memo");
    h.keys("ctrl+s");
    assert_eq!(h.overlay(), "prompt", "名前を尋ねられる");
    h.text("/w/new.txt");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.fs_read("/w/new.txt").as_deref(), Some(&b"memo"[..]));
    assert_eq!(h.doc_path().as_deref(), Some("/w/new.txt"));
    assert!(!h.is_dirty());
}

// verifies: AC-FR-107-3
#[test]
fn 新規バッファの作成は確認なしで足され_dirty_な文書はそのまま残る() {
    let mut h = opened("/w/a.txt", b"abc");
    h.text("X");
    h.keys("ctrl+n");
    assert_eq!(h.overlay(), "", "何も捨てないので確認は出ない");
    assert_eq!(h.doc_text(), "");
    assert_eq!(h.doc_path(), None);
    assert!(!h.is_dirty());
    // 元の文書は編集も dirty も残っている
    h.cmd("buffer.prev");
    assert_eq!(h.doc_text(), "Xabc");
    assert!(h.is_dirty());
    assert_eq!(h.doc_path().as_deref(), Some("/w/a.txt"));
}

// verifies: AC-FR-107-3
#[test]
fn 読込が失敗したら今の文書がそのまま残る() {
    let mut h = opened("/w/a.txt", b"abc");
    h.text("X");
    // 別のファイルを開く: 読込が失敗しても、今の文書は変わらない
    h.fs.add_file("/w/b.txt", b"bbb");
    h.fs.fail_next(
        eve_testkit::FsOp::OpenRead,
        eve_core::FsErrorKind::PermissionDenied,
    );
    h.cmd_with("file.open", Args::new().with_str("path", "/w/b.txt"));
    assert_eq!(h.overlay(), "", "確認は出ない");
    h.run_jobs();
    assert_eq!(
        h.doc_path().as_deref(),
        Some("/w/a.txt"),
        "失敗したので元の文書"
    );
    assert_eq!(h.doc_text(), "Xabc", "編集も残る");
    assert!(
        h.messages()
            .iter()
            .any(|k| k == "msg.open.permission_denied"),
        "{:?}",
        h.messages()
    );
}

// verifies: AC-FR-104-1
#[test]
fn 名前を付けて保存は既定のキーから実行できる() {
    let mut h = opened("/w/a.txt", b"abc");
    h.text("X");
    h.keys("ctrl+shift+s");
    assert_eq!(h.overlay(), "prompt", "ctrl+shift+s");
    h.keys("escape");
    h.keys("ctrl+k s");
    assert_eq!(h.overlay(), "prompt", "ctrl+k s");
    h.keys("ctrl+a"); // 入力欄のテキストを選ぶ代わりに、全部消して入れ直す
    for _ in 0.."/w/a.txt".len() {
        h.keys("backspace");
    }
    h.text("/w/copy.txt");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(h.fs_read("/w/copy.txt").as_deref(), Some(&b"Xabc"[..]));
    assert_eq!(
        h.fs_read("/w/a.txt").as_deref(),
        Some(&b"abc"[..]),
        "元は変えない"
    );
}

// verifies: AC-FR-204-1
#[test]
fn 保存時点に戻ると未変更の表示になる() {
    let mut h = opened("/w/a.txt", b"abc");
    assert!(!h.frame_contains("[+]"));
    h.text("X");
    assert!(h.frame_contains("[+]"));
    h.keys("ctrl+s");
    h.run_jobs();
    assert!(!h.frame_contains("[+]"), "保存したら未変更");
    h.advance_ms(2000);
    h.text("Y");
    assert!(h.frame_contains("[+]"));
    h.keys("ctrl+z");
    assert!(!h.frame_contains("[+]"), "Undo で保存時点に戻ると未変更");
    h.keys("ctrl+z");
    assert!(h.frame_contains("[+]"), "保存点より前は変更あり");
    h.keys("ctrl+y");
    assert!(!h.frame_contains("[+]"), "Redo で保存時点に戻ると未変更");
}

// verifies: AC-FR-204-3
#[test]
fn 保存点より後を_undo_してから編集すると_以後_dirty_のまま() {
    let mut h = opened("/w/a.txt", b"abc");
    h.text("X");
    h.keys("ctrl+s");
    h.run_jobs();
    assert!(!h.is_dirty());
    h.keys("ctrl+z"); // 保存点より前へ (保存点は Redo 側に残る)
    assert_eq!(h.doc_text(), "abc");
    assert!(h.is_dirty());
    h.text("Z"); // 分岐: Redo の履歴と保存点を捨てる
    assert!(h.is_dirty());
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "abc", "内容は元と同じに見えても");
    assert!(
        h.is_dirty(),
        "保存したのは Xabc なので、以後は dirty のまま"
    );
    assert!(h.frame_contains("[+]"));
    h.keys("ctrl+y");
    assert_eq!(h.doc_text(), "Zabc");
    assert!(h.is_dirty());
}

/// 改行を 1 つずつ入れる編集を `n` 回 (結合しないよう時間を空ける)。
fn many_edits(h: &mut Harness, n: usize) {
    for _ in 0..n {
        h.text("a\n");
        h.advance_ms(1500);
    }
}

// verifies: AC-FR-204-4
#[test]
fn 履歴の上限で古い履歴を破棄しても_未変更の表示は変わらない() {
    // 保存点より前が破棄されるだけなら、保存点の判定は変わらない (保存 → 編集 → 破棄が起きる → Undo で保存点に戻る)
    let mut h = Harness::new();
    many_edits(&mut h, 10_000);
    h.keys("ctrl+s");
    h.text("/w/big.txt");
    h.keys("enter");
    h.run_jobs();
    assert!(!h.is_dirty());
    assert!(!h.frame_contains("[+]"));
    h.text("x"); // 1 ステップ増える → 上限を超えて最古が破棄される
    assert!(h.is_dirty());
    assert!(
        h.messages()
            .contains(&"info.undo.history_trimmed".to_owned())
    );
    h.keys("ctrl+z");
    assert!(!h.is_dirty(), "破棄が起きても、保存点に戻れば未変更");
    assert!(!h.frame_contains("[+]"));
    // 保存点そのものが破棄されたら、以後は dirty のまま (戻れない)
    many_edits(&mut h, 10_100);
    assert!(h.is_dirty());
    for _ in 0..12_000 {
        h.keys("ctrl+z");
    }
    assert!(
        h.is_dirty(),
        "保存点は履歴から消えているので未変更には戻らない"
    );
    assert!(h.frame_contains("[+]"));
}

// verifies: AC-FR-203-2
#[test]
fn undo_と_redo_を繰り返しても各時点の文書と選択に一致する() {
    // 疑似乱数 (線形合同法) で 1,000 回の編集 → 全部 Undo → 全部 Redo を、各時点の状態と突き合わせる
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rnd = move |n: u64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    let mut h = Harness::new();
    let mut states = vec![(h.doc_text(), sel(&h))];
    for _ in 0..1000 {
        match rnd(5) {
            0 => h.text("あ"),
            1 => h.text("ab"),
            2 => h.keys("enter"),
            3 => h.keys("backspace"),
            _ => h.keys("delete"),
        }
        h.advance_ms(1500);
        let now = (h.doc_text(), sel(&h));
        if now != *states.last().unwrap() {
            states.push(now);
        }
    }
    let n = states.len();
    assert!(n > 500, "十分な数の履歴: {n}");
    for k in (0..n - 1).rev() {
        h.keys("ctrl+z");
        assert_eq!(&(h.doc_text(), sel(&h)), &states[k], "Undo → 時点 {k}");
    }
    for (k, st) in states.iter().enumerate().skip(1) {
        h.keys("ctrl+y");
        assert_eq!(&(h.doc_text(), sel(&h)), st, "Redo → 時点 {k}");
    }
    // 繰り返し: 同じ往復をもう一度
    for k in (0..n - 1).rev() {
        h.keys("ctrl+z");
        assert_eq!(&(h.doc_text(), sel(&h)), &states[k], "2 周目 Undo → {k}");
    }
}

// verifies: AC-FR-107-1
#[test]
fn 終了の確認は_3_択で_キャンセルでは何も変わらない() {
    let mut h = opened("/w/a.txt", b"abc");
    h.text("X");
    h.keys("ctrl+q");
    assert_eq!(h.overlay(), "dialog");
    let window = h.editor.state().first_window().unwrap();
    let Answer::Overlay(Some(eve_view::OverlayModel::Confirm { choices, .. })) =
        h.query(&Query::Overlay { window }).unwrap()
    else {
        panic!("確認ダイアログ");
    };
    assert_eq!(choices.len(), 3, "保存 / 破棄 / キャンセル");
    h.answer(2);
    assert_eq!(h.overlay(), "");
    assert_eq!(h.quit_code(), None);
    assert_eq!(h.doc_text(), "Xabc");
    assert!(h.is_dirty());
    // 破棄を選ぶと終了する
    h.keys("ctrl+q");
    h.answer(1);
    assert_eq!(h.quit_code(), Some(0));
}

// verifies: AC-FR-107-3
#[test]
fn 読込が取り消されたら今の文書がそのまま残る() {
    let mut h = opened("/w/a.txt", b"abc");
    h.text("X");
    h.fs.add_file("/w/b.txt", b"bbb");
    h.cmd_with("file.open", Args::new().with_str("path", "/w/b.txt"));
    h.answer(1); // 破棄して開く (読込が始まる)
    assert_eq!(h.doc_text(), "Xabc", "読込が終わるまでは今の文書のまま");
    h.cancel_jobs();
    h.run_jobs();
    assert_eq!(h.doc_path().as_deref(), Some("/w/a.txt"));
    assert_eq!(h.doc_text(), "Xabc");
    assert!(h.is_dirty());
    assert!(h.messages().contains(&"info.open.cancelled".to_owned()));
}
