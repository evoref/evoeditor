//! スワップの復旧の提案と適用 (docs/03 §10.3、FR-118)。`MemFs` に他のプロセスが残したスワップを積み、
//! 起動時の探索 → 提案 → 適用 / 破棄 → 古いスワップの片付け、を確かめる。

use eve_core::{Encoding, FileTime, FsPath, SwapBody, SwapEntry, SwapOwner};
use eve_testkit::Harness;
use eve_text::LineEnding;
use eve_view::OverlayModel;

fn ct(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.as_str()).collect()
}

/// 確認ダイアログの本文 (タイトル・メッセージ・詳細・選択肢を改行でつなげたもの)。
fn dialog_text(h: &mut Harness) -> String {
    assert_eq!(h.overlay(), "dialog");
    let f = h.frame();
    let OverlayModel::Confirm {
        title,
        message,
        details,
        choices,
        ..
    } = &f.overlays[0]
    else {
        panic!("確認ダイアログではない");
    };
    let mut out = vec![ct(title), ct(message)];
    out.extend(details.iter().flatten().map(ct));
    out.extend(choices.iter().map(|c| ct(&c.label)));
    out.join("\n")
}

fn entry(dir: &str, path: Option<&str>, owner: SwapOwner, readable: bool) -> SwapEntry {
    SwapEntry {
        dir: FsPath::from(dir),
        path: path.map(FsPath::from),
        untitled_name: path.is_none().then(|| "無題".to_owned()),
        revision: 7,
        len: 12,
        pid: 4242,
        owner,
        sensitive: false,
        modified: Some(FileTime {
            secs: 4_000_000_000,
            nanos: 0,
        }),
        readable,
    }
}

fn body(text: &str) -> SwapBody {
    SwapBody {
        text: text.to_owned(),
        encoding: Encoding::Utf8,
        has_bom: false,
        default_eol: LineEnding::Lf,
    }
}

/// スワップを積んだ状態で起動し、起動時の走査を終えて tick まで進める。
fn started(entries: Vec<(SwapEntry, Option<SwapBody>)>) -> Harness {
    let mut h = Harness::new().with_file("/w/a.txt", b"old");
    for (e, b) in entries {
        h.fs.add_swap_entry(e, b);
    }
    h.run_jobs();
    h.advance_ms(0);
    h
}

/// 起動時の走査の前にスワップを積み、ファイルを開いて tick まで進める (走査の結果は読込中の文書に対して届く)。
fn opened(entries: Vec<(SwapEntry, Option<SwapBody>)>) -> Harness {
    let mut h = Harness::new().with_file("/w/a.txt", b"old");
    for (e, b) in entries {
        h.fs.add_swap_entry(e, b);
    }
    h.open("/w/a.txt");
    h.run_jobs();
    h.advance_ms(0);
    h
}

// verifies: FR-118, NFR-307
#[test]
fn 無題で空の文書に_無題のスワップを提案し_適用すると_dirty_で_undo_できる() {
    let mut h = started(vec![(
        entry("/s/swap/untitled-1-4242.swp", None, SwapOwner::Stale, true),
        Some(SwapBody {
            text: "こんにちは\r\n".to_owned(),
            encoding: Encoding::ShiftJis,
            has_bom: false,
            default_eol: LineEnding::Crlf,
        }),
    )]);
    assert_eq!(h.overlay(), "dialog", "{:?}", h.messages());
    let t = dialog_text(&mut h);
    assert!(t.contains("未保存の内容が見つかりました"), "{t}");
    assert!(
        t.contains("[A] 復旧内容を適用") && t.contains("[D] スワップを破棄"),
        "{t}"
    );
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), "こんにちは\r\n");
    assert!(h.is_dirty());
    assert!(h.messages().contains(&"info.swap.recovered".to_owned()));
    // 元に戻すと空に戻る (Origin::Recovery の Transaction)
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "");
}

#[test]
fn 古いスワップは_新しいスワップが書けてから消す() {
    let mut h = started(vec![(
        entry("/s/swap/untitled-1-4242.swp", None, SwapOwner::Stale, true),
        Some(body("abc")),
    )]);
    h.answer(0);
    h.run_jobs();
    h.advance_ms(0);
    h.run_jobs();
    assert_eq!(
        h.fs.swap_entries().len(),
        1,
        "新しいスワップはまだ無いので残す"
    );
    h.advance_ms(2000);
    h.run_jobs();
    h.advance_ms(0);
    h.run_jobs();
    assert_eq!(h.fs.swap_count(), 1, "自分の新しいスワップ");
    assert!(
        h.fs.swap_entries().is_empty(),
        "新しいスワップが書けたので古いのを消す"
    );
}

#[test]
fn 保存すれば_新しいスワップを待たずに古いスワップを消す() {
    let mut h = opened(vec![(
        entry(
            "/s/swap/aaaa-4242.swp",
            Some("/w/a.txt"),
            SwapOwner::Stale,
            true,
        ),
        Some(body("recovered")),
    )]);
    assert_eq!(h.overlay(), "dialog");
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), "recovered");
    h.keys("ctrl+s");
    h.run_jobs();
    h.advance_ms(0);
    h.run_jobs();
    assert!(!h.is_dirty());
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), b"recovered");
    assert!(h.fs.swap_entries().is_empty());
}

// verifies: FR-118, NFR-307
#[test]
fn パスのある文書は開いた形式のまま本文だけ置き換える() {
    let mut h = opened(vec![(
        entry(
            "/s/swap/aaaa-4242.swp",
            Some("/w/a.txt"),
            SwapOwner::Stale,
            true,
        ),
        Some(SwapBody {
            text: "new".to_owned(),
            encoding: Encoding::ShiftJis,
            has_bom: true,
            default_eol: LineEnding::Crlf,
        }),
    )]);
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), "new");
    let doc = h.doc_id();
    let fmt = &h.editor.state().document(doc).unwrap().format;
    assert_eq!(fmt.encoding, Encoding::Utf8, "開いた文書の形式を保つ");
    assert!(!fmt.has_bom);
}

#[test]
fn 破棄を選ぶとスワップを消し_文書は変えない() {
    let mut h = started(vec![(
        entry("/s/swap/untitled-1-4242.swp", None, SwapOwner::Stale, true),
        Some(body("abc")),
    )]);
    h.answer(1);
    h.run_jobs();
    assert_eq!(h.doc_text(), "");
    assert!(!h.is_dirty());
    assert!(h.fs.swap_entries().is_empty());
    assert!(h.messages().contains(&"info.swap.discarded".to_owned()));
}

#[test]
fn esc_で閉じれば何もせず_スワップは残り_もう提案しない() {
    let mut h = started(vec![(
        entry("/s/swap/untitled-1-4242.swp", None, SwapOwner::Stale, true),
        Some(body("abc")),
    )]);
    assert_eq!(h.overlay(), "dialog");
    h.keys("esc");
    h.advance_ms(0);
    assert_eq!(h.overlay(), "");
    assert_eq!(h.fs.swap_entries().len(), 1, "残す");
    assert_eq!(h.doc_text(), "");
    h.advance_ms(5000);
    assert_eq!(h.overlay(), "", "このセッションでは再提案しない");
}

#[test]
fn 動いている別のプロセスのスワップは提案せず_同じファイルなら警告する() {
    let h = opened(vec![(
        entry(
            "/s/swap/aaaa-4242.swp",
            Some("/w/a.txt"),
            SwapOwner::Alive,
            true,
        ),
        Some(body("abc")),
    )]);
    assert_eq!(h.overlay(), "");
    assert!(h.messages().contains(&"msg.swap.other_instance".to_owned()));
    assert_eq!(h.fs.swap_entries().len(), 1);
}

#[test]
fn 読めないスワップは場所だけ示して提案しない() {
    let h = started(vec![(
        entry("/s/swap/untitled-1-4242.swp", None, SwapOwner::Stale, false),
        None,
    )]);
    assert_eq!(h.overlay(), "");
    assert!(h.messages().contains(&"msg.swap.unreadable".to_owned()));
    assert_eq!(h.fs.swap_entries().len(), 1, "移動も削除もしない");
}

#[test]
fn 別のファイルのスワップは案内だけで_そのファイルを開くと提案する() {
    let mut h = Harness::new().with_file("/w/b.txt", b"bee");
    h.fs.add_swap_entry(
        entry(
            "/s/swap/bbbb-4242.swp",
            Some("/w/b.txt"),
            SwapOwner::Stale,
            true,
        ),
        Some(body("bee!")),
    );
    h.run_jobs();
    h.advance_ms(0);
    assert_eq!(h.overlay(), "", "無題の文書には該当しない");
    assert!(h.messages().contains(&"msg.swap.found".to_owned()));
    h.open("/w/b.txt");
    h.run_jobs();
    h.advance_ms(0);
    assert_eq!(h.overlay(), "dialog", "該当のファイルを開いたので提案する");
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), "bee!");
}

#[test]
fn 編集済みの文書には提案しない() {
    let mut h = Harness::new();
    h.text("typing");
    h.fs.add_swap_entry(
        entry("/s/swap/untitled-1-4242.swp", None, SwapOwner::Stale, true),
        Some(body("abc")),
    );
    h.run_jobs();
    h.advance_ms(0);
    assert_eq!(h.overlay(), "", "編集中の内容を上書きしない");
    assert_eq!(h.doc_text(), "typing");
}

#[test]
fn 提案の後で文書が変われば適用せず_スワップは残す() {
    let mut h = started(vec![(
        entry("/s/swap/untitled-1-4242.swp", None, SwapOwner::Stale, true),
        Some(body("abc")),
    )]);
    h.answer(0);
    // 読み込みの結果が届く前に文書が変わる
    h.text("z");
    h.run_jobs();
    assert_eq!(h.doc_text(), "z");
    assert!(
        h.messages()
            .contains(&"msg.swap.changed_since_offer".to_owned())
    );
    assert_eq!(h.fs.swap_entries().len(), 1);
}

#[test]
fn 本文を読めなければ_スワップは残してエラーを出す() {
    let mut h = started(vec![(
        entry("/s/swap/untitled-1-4242.swp", None, SwapOwner::Stale, true),
        None,
    )]);
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), "");
    assert!(h.messages().contains(&"msg.swap.read_failed".to_owned()));
    assert_eq!(h.fs.swap_entries().len(), 1);
}

#[test]
fn 元のファイルがスワップの後に変わっていれば_提案にその旨を添える() {
    // modified を過去にして、開いたファイルの更新時刻の方を新しくする
    let mut e = entry(
        "/s/swap/aaaa-4242.swp",
        Some("/w/a.txt"),
        SwapOwner::Stale,
        true,
    );
    e.modified = Some(FileTime { secs: -1, nanos: 0 });
    let mut h = opened(vec![(e, Some(body("abc")))]);
    assert_eq!(h.overlay(), "dialog");
    let t = dialog_text(&mut h);
    assert!(t.contains("スワップを書いた後で変更されています"), "{t}");

    // 逆 (スワップの方が新しい) なら添えない
    let mut h = opened(vec![(
        entry(
            "/s/swap/aaaa-4242.swp",
            Some("/w/a.txt"),
            SwapOwner::Stale,
            true,
        ),
        Some(body("abc")),
    )]);
    assert_eq!(h.overlay(), "dialog");
    let t = dialog_text(&mut h);
    assert!(!t.contains("スワップを書いた後で変更されています"), "{t}");
}

#[test]
fn 秘密ファイルのスワップの提案は削除を促す() {
    let mut e = entry("/s/swap/untitled-1-4242.swp", None, SwapOwner::Stale, true);
    e.sensitive = true;
    let mut h = started(vec![(e, Some(body("K=1")))]);
    assert_eq!(h.overlay(), "dialog");
    let t = dialog_text(&mut h);
    assert!(t.contains("秘密を含み得るファイルです"), "{t}");
}
