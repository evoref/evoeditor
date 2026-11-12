//! エスケープ文字のつながりの書く前の確認 (`ui.confirm.save.escape_merge`、docs/03 §8.3.1 の「利用者への示し方」〜
//! 「つながったまま保存した後」、§9.7 (e)・(f)、§9.8、03 §22.3 の「不正バイトを含む文書の編集」の (c)。M23 の 4 本目)。
//!
//! 不正バイトを含む文書は読み取り専用で開く。編集が要る場合は、製品の「編集を許可」(`file.toggle_read_only` と確認の
//! `[A]`、M23 の 6 本目) で解除する。

use eve_core::{
    Args, CommandResult, Document, Encoding, EscapeMergeAt, JobOutput, JobResult, OpenOptions,
    Origin, Tick, Transaction,
};
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_text::{ByteOff, ByteRange};
use eve_view::{Effect, JobId, OverlayModel, QuitRescue, StyleMods};

fn env() -> Harness {
    Harness::builder().defer_start().build()
}

fn run(h: &mut Harness, id: &str, args: Args) -> CommandResult {
    let r = h.cmd_with(id, args);
    h.run_jobs();
    r
}

fn save(h: &mut Harness) {
    run(h, "file.save", Args::new());
}

fn press(h: &mut Harness, k: &str) {
    h.text(k);
    h.run_jobs();
}

fn esc(h: &mut Harness) {
    h.keys("esc");
    h.run_jobs();
}

fn open_as(h: &mut Harness, path: &str, enc: Encoding) {
    h.editor.state_mut().open_path(
        path,
        OpenOptions {
            forced: Some(enc),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
}

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

/// 不正バイトの比率の確認 (FR-127) を出さないための後ろの詰め物。
const PAD: &str = "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\n\
                   0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\n";

/// 「編集を許可」(`file.toggle_read_only` → 確認 `ui.confirm.read_only.invalid_bytes` の `[A]`、03 §8.3.1)。
fn release_invalid_bytes(h: &mut Harness) {
    assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
    assert_eq!(
        h.overlay(),
        "dialog",
        "編集を許可の確認: {:?}",
        h.messages()
    );
    h.text("a");
    let id = h.doc_id();
    let d = h.editor.state().document(id).unwrap();
    assert!(
        d.is_editable() && d.invalid_bytes_released,
        "{:?} {:?}",
        d.read_only.iter().collect::<Vec<_>>(),
        d.load_state
    );
}

/// 本文のバイト範囲 `start..end` を `insert` に置き換える (1 つの Transaction)。
fn replace(h: &mut Harness, edits: &[(usize, usize, &str)]) {
    let id = h.doc_id();
    let d = h.editor.state_mut().document_mut(id).unwrap();
    let change = eve_text::ChangeSet::from_sorted_edits(
        d.text(),
        edits.iter().map(|&(a, b, s)| eve_text::Edit {
            range: ByteRange::new(ByteOff(a), ByteOff(b)),
            insert: s.into(),
        }),
    )
    .unwrap();
    d.apply(&Transaction::new(change, Origin::Paste, Tick(0)))
        .unwrap();
}

/// `Shift_JIS` の `x<0x81>` の行と `@yz` の行を開いて、行を結合した状態 (`<0x81>` の直後に `@` が来て、0x8140 (全角の
/// 空白) になる)。
fn sjis_joined_with(h: Harness, head: &str) -> Harness {
    let mut s = h;
    s.fs.add_file(
        "/w/a.txt",
        &[head.as_bytes(), b"x\x81\n@yz\n", PAD.as_bytes()].concat(),
    );
    open_as(&mut s, "/w/a.txt", Encoding::ShiftJis);
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::ShiftJis));
    release_invalid_bytes(&mut s);
    let at = s.doc_text().find("\u{10FF81}").unwrap() + '\u{10FF81}'.len_utf8();
    replace(&mut s, &[(at, at + 1, "")]);
    assert_eq!(s.doc_text(), format!("{head}x\u{10FF81}@yz\n{PAD}"));
    s
}

fn sjis_joined() -> Harness {
    sjis_joined_with(env(), "")
}

/// 結合した文書を保存したときに書かれるバイト列 (`<0x81>` を元のバイトに戻す)。
fn joined_bytes(head: &str) -> Vec<u8> {
    [head.as_bytes(), b"x\x81@yz\n", PAD.as_bytes()].concat()
}

/// 確認ダイアログの (見出し, 本文, details, 選択肢のアクセラレータ, 既定の番号)。
fn dialog(h: &mut Harness) -> (String, String, Vec<String>, Vec<Option<char>>, u16) {
    let f = h.frame();
    let Some(OverlayModel::Confirm {
        title,
        message,
        details,
        choices,
        default,
        ..
    }) = f.overlays.first()
    else {
        panic!("確認が出ていない: {:?}", h.messages())
    };
    (
        cell_text(title),
        cell_text(message),
        details.iter().flatten().map(cell_text).collect(),
        choices.iter().map(|c| c.key).collect(),
        default.0,
    )
}

/// つながりの確認が出ている。
fn assert_merge_confirm(h: &mut Harness) {
    assert_eq!(h.overlay(), "dialog", "{:?}", h.messages());
    let (title, ..) = dialog(h);
    assert_eq!(title, "開き直すと文字が変わる箇所");
}

/// 強調 (表現不能と同じ印) が付いた本文の文字列。
fn marked(h: &mut Harness) -> String {
    let f = h.frame();
    f.panes[0]
        .rows
        .iter()
        .flat_map(|r| r.spans.iter())
        .filter(|sp| {
            sp.style
                .mods
                .contains(StyleMods::REVERSE | StyleMods::UNDERLINE)
        })
        .map(|sp| sp.text.to_string())
        .collect()
}

fn view(h: &Harness) -> &eve_core::View {
    let state = h.editor.state();
    let v = state.window_view(h.window()).unwrap();
    state.view(v).unwrap()
}

fn last_message(h: &Harness) -> Option<String> {
    h.messages().last().cloned()
}

// ---------- 確認の表示 ----------

// verifies: NFR-304
#[test]
fn つながる保存は何も書かずに確認を出し_件数と最初の位置と変わる範囲と一覧を示す() {
    let mut s = sjis_joined();
    let before = s.fs.read("/w/a.txt").unwrap();
    let paths = s.fs.paths();
    save(&mut s);
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before, "何も書かない");
    assert_eq!(s.fs.paths(), paths, "一時ファイルも作らない");
    assert!(doc(&s).is_dirty());
    let (title, message, details, keys, default) = dialog(&mut s);
    assert_eq!(title, "開き直すと文字が変わる箇所");
    assert_eq!(
        message,
        "1 箇所で、不正なバイトが続くバイトや文字とつながり、開き直すと別の文字として読まれます (最初: 行 1, 桁 2)。\
         合わせて 2 文字 (1 行) が別の文字になります。まだ保存していません"
    );
    assert_eq!(details, ["行 1, 桁 2: <0x81>@ → \u{3000}"]);
    assert_eq!(keys, [Some('s'), None], "このまま保存とキャンセル");
    assert_eq!(default, 1, "既定はキャンセル");
}

// 確認を出すときに、最初の位置が画面に入るようにスクロールする (カーソルは動かさない)
#[test]
fn 確認を出すときに最初の位置が画面に入るようにスクロールし_カーソルは動かさない() {
    let head = "a\n".repeat(200);
    let mut s = sjis_joined_with(Harness::builder().defer_start().size(80, 12).build(), &head);
    s.keys("ctrl+home");
    assert_eq!(view(&s).scroll.top, ByteOff(0));
    save(&mut s);
    assert_merge_confirm(&mut s);
    assert!(view(&s).scroll.top.0 > 0, "最初の位置まで動かす");
    assert_eq!(
        view(&s).selection.primary().head,
        ByteOff(0),
        "カーソルは動かさない"
    );
    let (_, message, ..) = dialog(&mut s);
    assert!(message.contains("(最初: 行 201, 桁 2)"), "{message}");
}

// 100 件を超えるときは、先頭 100 件を 1 件 1 行で並べ、最後に「ほか N 箇所」を付ける
#[test]
fn 百件を超えるときは先頭百件と_ほか_n_箇所_を示す() {
    let mut s = env();
    let unit = b"\x81\n@ ";
    let mut file = Vec::new();
    for _ in 0..105 {
        file.extend_from_slice(unit);
    }
    file.extend_from_slice(b"\n");
    // 不正バイトの比率の確認 (1% を超えると出る) を出さない長さにする
    for _ in 0..80 {
        file.extend_from_slice(PAD.as_bytes());
    }
    s.fs.add_file("/w/a.txt", &file);
    open_as(&mut s, "/w/a.txt", Encoding::ShiftJis);
    release_invalid_bytes(&mut s);
    // 各 `<0x81>` の直後の改行を消す
    let text = s.doc_text();
    let edits: Vec<(usize, usize, &str)> = text
        .match_indices('\u{10FF81}')
        .map(|(i, c)| (i + c.len(), i + c.len() + 1, ""))
        .collect();
    assert_eq!(edits.len(), 105);
    replace(&mut s, &edits);
    save(&mut s);
    let (_, message, details, ..) = dialog(&mut s);
    assert!(message.starts_with("105 箇所で、"), "{message}");
    assert!(message.contains("(最初: 行 1, 桁 1)"), "{message}");
    assert_eq!(details.len(), 101, "先頭 100 件と「ほか」の行");
    assert_eq!(details[0], "行 1, 桁 1: <0x81>@ → \u{3000}");
    assert_eq!(details[1], "行 1, 桁 4: <0x81>@ → \u{3000}");
    assert_eq!(details[100], "ほか 5 箇所");
}

// UTF-16LE の文書の先頭で奇数個の並びにすると、後ろが 1 バイトずれてファイルの末尾まで別の文字になる
#[test]
fn 末尾まで戻らないつながりは_ファイルの末尾まで_と示す() {
    let mut s = env();
    let pad16: Vec<u8> = PAD.encode_utf16().flat_map(u16::to_le_bytes).collect();
    s.fs.add_file(
        "/w/a.txt",
        &[&b"\x00\xD8a\x00b\x00\n\x00"[..], &pad16].concat(),
    );
    open_as(&mut s, "/w/a.txt", Encoding::Utf16Le);
    assert_eq!(doc(&s).format.escape_encoding, Some(Encoding::Utf16Le));
    release_invalid_bytes(&mut s);
    // 先頭のエスケープ文字を 1 つ消して奇数個の並びにする
    let first = s.doc_text().chars().next().unwrap();
    assert_eq!(first, '\u{10FF00}');
    replace(&mut s, &[(0, first.len_utf8(), "")]);
    let before = s.fs.read("/w/a.txt").unwrap();
    save(&mut s);
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before, "何も書かない");
    let (_, message, ..) = dialog(&mut s);
    assert!(
        message.contains("そこからファイルの末尾まで別の文字になります"),
        "{message}"
    );
    assert!(message.contains("(最初: 行 1, 桁 1)"), "{message}");
}

// ---------- キャンセル ----------

// verifies: NFR-304
#[test]
fn キャンセルは書かずにカーソルを最初の位置へ移し_全部の位置を強調して知らせる() {
    let mut s = sjis_joined();
    let before = s.fs.read("/w/a.txt").unwrap();
    s.keys("ctrl+end");
    save(&mut s);
    assert_merge_confirm(&mut s);
    esc(&mut s);
    assert_eq!(s.overlay(), "");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before, "何も書かない");
    assert!(doc(&s).is_dirty());
    assert_eq!(
        view(&s).selection.primary().head,
        ByteOff(1),
        "最初の位置 (`<0x81>`) へ移る"
    );
    assert_eq!(
        marked(&mut s),
        "<0x81>@",
        "強調の範囲 (つながる並びと直後の 1 文字)"
    );
    assert_eq!(
        last_message(&s).as_deref(),
        Some("info.save.escape_merge_cancelled")
    );
    s.text("q");
    assert_eq!(marked(&mut s), "", "次の編集で消える");
}

// つながりを元に戻せば (間に改行を入れる) ふつうに保存できる。不正バイトは元のバイトのまま書く
#[test]
fn キャンセルの後につながりを直せばそのまま保存できる() {
    let mut s = sjis_joined();
    save(&mut s);
    esc(&mut s);
    // カーソルは `<0x81>` の前にある。並びの後ろ (`@` の前) に改行を入れる
    s.keys("right");
    s.keys("enter");
    save(&mut s);
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        [b"x\x81\n@yz\n", PAD.as_bytes()].concat()
    );
    assert_eq!(last_message(&s).as_deref(), Some("info.save.done"));
}

// ---------- このまま保存 ----------

// verifies: NFR-304
#[test]
fn このまま保存で書き_開き直すとつながった文字として読まれる() {
    let mut s = sjis_joined();
    save(&mut s);
    assert_merge_confirm(&mut s);
    press(&mut s, "s");
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), joined_bytes(""));
    assert!(!doc(&s).is_dirty(), "保存点はその Revision");
    assert_eq!(
        s.doc_text(),
        format!("x\u{10FF81}@yz\n{PAD}"),
        "本文はエスケープ文字のまま (自動では再読込しない)"
    );
    assert_eq!(last_message(&s).as_deref(), Some("info.save.escape_merged"));
    // 同じ文字コードを指定して開き直すと、つながった文字として読まれる
    run(
        &mut s,
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "Shift_JIS"),
    );
    assert_eq!(s.doc_text(), format!("x\u{3000}yz\n{PAD}"));
}

// 完了のメッセージの文言 (要点を先頭に)
#[test]
fn このまま保存の完了のメッセージは開き直すと変わる件数を言う() {
    let mut s = sjis_joined();
    save(&mut s);
    press(&mut s, "s");
    let f = s.frame();
    let text = cell_text(&f.messages.current.as_ref().unwrap().text);
    assert_eq!(
        text,
        "/w/a.txt を保存しました。開き直すと 1 箇所が別の文字になります"
    );
}

// 承認の後の保存が Busy で失敗しても、再試行・別名で保存では同じつながりの確認を繰り返さない (03 §9.7 の表)
#[test]
fn 承認の後の失敗からの再試行と別名で保存では確認を繰り返さない() {
    let mut s = sjis_joined();
    save(&mut s);
    s.fs.fail_next(eve_testkit::FsOp::WriteAtomic, eve_core::FsErrorKind::Busy);
    press(&mut s, "s");
    assert_eq!(s.overlay(), "dialog", "保存の失敗の選択");
    let (title, ..) = dialog(&mut s);
    assert_ne!(title, "開き直すと文字が変わる箇所");
    // 再試行
    press(&mut s, "t");
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), joined_bytes(""));
    assert_eq!(last_message(&s).as_deref(), Some("info.save.escape_merged"));

    // 別名で保存
    let mut s = sjis_joined();
    save(&mut s);
    s.fs.fail_next(eve_testkit::FsOp::WriteAtomic, eve_core::FsErrorKind::Busy);
    press(&mut s, "s");
    press(&mut s, "a");
    assert_eq!(s.overlay(), "prompt");
    s.text("/w/b.txt");
    s.keys("enter");
    s.run_jobs();
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(s.fs.read("/w/b.txt").unwrap(), joined_bytes(""));
}

// 確認の表示中 (保存の待ちの間) に編集してから「このまま保存」→ 承認を使わず、今の内容でもう一度確認する
#[test]
fn 保存の待ちの間に編集していたら承認を覚えずにもう一度確認する() {
    let mut s = sjis_joined();
    let before = s.fs.read("/w/a.txt").unwrap();
    s.cmd("file.save");
    // 保存のジョブが終わる前に編集する (単独の保存は編集を止めない)
    s.keys("ctrl+end");
    s.text("w");
    s.run_jobs();
    assert_merge_confirm(&mut s);
    press(&mut s, "s");
    assert_merge_confirm(&mut s);
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        before,
        "承認を使わずに今の Snapshot で保存し直し、もう一度確認する"
    );
    press(&mut s, "s");
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(
        s.fs.read("/w/a.txt").unwrap(),
        [joined_bytes("").as_slice(), b"w"].concat()
    );
}

// 承認した後に、別のつながりを作ってから保存すると、もう一度確認する (承認は結果で照らし合わせる)
#[test]
fn つながりが変わったらもう一度確認する() {
    let mut s = sjis_joined();
    save(&mut s);
    s.fs.fail_next(eve_testkit::FsOp::WriteAtomic, eve_core::FsErrorKind::Busy);
    press(&mut s, "s");
    esc(&mut s);
    assert_eq!(s.overlay(), "");
    // 2 つ目のつながりを足す (`<0x81>` を複製して `@` の前に置く)
    let at = s.doc_text().find('@').unwrap();
    replace(&mut s, &[(at + 1, at + 1, "\u{10FF81}@")]);
    let before = s.fs.read("/w/a.txt").unwrap();
    save(&mut s);
    assert_merge_confirm(&mut s);
    let (_, message, ..) = dialog(&mut s);
    assert!(message.starts_with("2 箇所で、"), "{message}");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before);
}

// 直接上書き (in-place) の経路: つながりの確認は乾式走査で出るので、ハードリンクの確認より先に出る (何も書かない)。
// 承認した後のハードリンクの確認で「リンクを保って直接上書き」を選ぶと、承認を引き継いで直接上書きで書く
#[test]
fn 直接上書きでも書く前に確認し_承認を引き継いで直接上書きで書く() {
    let mut s = sjis_joined();
    s.fs.set_nlink("/w/a.txt", 2);
    let before = s.fs.read("/w/a.txt").unwrap();
    let paths = s.fs.paths();
    save(&mut s);
    assert_merge_confirm(&mut s);
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before, "何も書かない");
    assert_eq!(s.fs.paths(), paths, "一時ファイルも作らない");
    press(&mut s, "s");
    let (title, ..) = dialog(&mut s);
    assert_eq!(title, "ハードリンクのファイル");
    s.clear_log();
    press(&mut s, "k");
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), joined_bytes(""));
    assert_eq!(
        s.messages(),
        ["info.save.in_place", "info.save.escape_merged"],
        "直接上書きで書いたことと、つながりを書いたこと"
    );
}

// 直接上書きのジョブが `EscapeMerge` を返したとき (承認が結果と合わない場合)、「このまま保存」は直接上書きのまま
// 投入し直す (保存方式を変えない、03 §9.7 の表の `method` の列)。ジョブの結果は試験の中で作る
#[test]
fn 直接上書きのジョブのつながりで_このまま保存_しても直接上書きのまま() {
    let mut s = sjis_joined();
    s.fs.set_nlink("/w/a.txt", 2);
    let before = s.fs.read("/w/a.txt").unwrap();
    let paths = s.fs.paths();
    save(&mut s);
    press(&mut s, "s");
    // ハードリンクの確認で「リンクを保って直接上書き」。投入した直接上書きのジョブは実行せずに、結果を作って渡す
    let CommandResult::Pending(job) = s.answer(0) else {
        panic!("直接上書きの保存を投入する")
    };
    s.cancel_jobs();
    s.editor.on_job_result(JobOutput {
        job,
        body: JobResult::EscapeMerge {
            total: 1,
            recorded: vec![EscapeMergeAt {
                char_index: 1,
                len_chars: 2,
                decoded: "\u{3000}".to_owned(),
            }],
            digest: 0,
            changed_chars: 2,
            changed_lines: 1,
            to_eof: false,
        },
    });
    assert_merge_confirm(&mut s);
    // このまま保存 → 直接上書きのジョブ (作った結果はダイジェストが違うので、本物の結果でもう一度確認する)
    s.answer(0);
    s.run_jobs();
    let (title, ..) = dialog(&mut s);
    assert_eq!(
        title, "開き直すと文字が変わる箇所",
        "ハードリンクの確認に戻らない"
    );
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before, "何も書かない");
    assert_eq!(s.fs.paths(), paths, "バックアップも作らない");
    s.clear_log();
    press(&mut s, "s");
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), joined_bytes(""));
    assert_eq!(
        s.messages(),
        ["info.save.in_place", "info.save.escape_merged"]
    );
}

// ---------- 表現不能とつながりの順序 ----------

// 表現不能の文字とつながりが両方ある → 先に E3 が出る (03 §8.3.1 の「見る順序」)
#[test]
fn 表現不能とつながりが両方あれば先に_e3_を出す() {
    let mut s = sjis_joined();
    s.keys("ctrl+end");
    s.text("😀");
    save(&mut s);
    assert_eq!(s.overlay(), "dialog");
    let (title, message, _, keys, _) = dialog(&mut s);
    assert_eq!(title, "表現できない文字");
    assert!(
        message.starts_with("1 個の文字は Shift_JIS で表せません"),
        "{message}"
    );
    assert!(
        !keys.contains(&Some('u')),
        "戻すエスケープ文字があるので UTF-8 で保存は出さない"
    );
    assert!(keys.contains(&Some('r')), "{keys:?}");
}

// ---------- 全保存 ----------

#[test]
fn 全保存ではつながる文書を保存せず_確認が必要です_と載せる() {
    let mut s = sjis_joined();
    let before = s.fs.read("/w/a.txt").unwrap();
    run(&mut s, "file.save_all", Args::new());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before, "何も書かない");
    assert!(doc(&s).is_dirty());
    let (title, _, details, ..) = dialog(&mut s);
    assert_eq!(title, "保存できなかった文書");
    assert_eq!(details, ["/w/a.txt: 確認が必要です"]);
}

// 承認が `SaveApprovals` にあり結果が一致すれば、全保存でも保存する
#[test]
fn 承認がある文書は全保存で保存する() {
    let mut s = sjis_joined();
    save(&mut s);
    s.fs.fail_next(eve_testkit::FsOp::WriteAtomic, eve_core::FsErrorKind::Busy);
    press(&mut s, "s");
    esc(&mut s);
    assert!(doc(&s).is_dirty());
    run(&mut s, "file.save_all", Args::new());
    assert_eq!(s.overlay(), "", "{:?}", s.messages());
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), joined_bytes(""));
    assert!(!doc(&s).is_dirty());
}

// ---------- 終了の前の保存 ----------

fn quit_effect(h: &Harness) -> Option<(u8, QuitRescue, Vec<String>)> {
    h.effects().iter().find_map(|e| match e {
        Effect::Quit {
            code,
            rescue,
            stderr,
        } => Some((
            *code,
            *rescue,
            stderr.iter().map(|l| l.key.clone()).collect(),
        )),
        _ => None,
    })
}

// verifies: AC-FR-107-1
#[test]
fn 終了の前の保存で確認をキャンセルすると終了しない() {
    let mut s = sjis_joined();
    let before = s.fs.read("/w/a.txt").unwrap();
    run(&mut s, "app.quit", Args::new());
    assert_eq!(s.overlay(), "dialog", "未保存の確認");
    press(&mut s, "s");
    assert_merge_confirm(&mut s);
    esc(&mut s);
    assert!(quit_effect(&s).is_none(), "終了しない");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before);
    assert!(doc(&s).is_dirty());
    assert_eq!(
        last_message(&s).as_deref(),
        Some("info.save.escape_merge_cancelled")
    );
}

// verifies: AC-FR-107-1
#[test]
fn 終了の前の保存で_このまま保存_を選ぶと書いて終了する() {
    let mut s = sjis_joined();
    run(&mut s, "app.quit", Args::new());
    press(&mut s, "s");
    assert_merge_confirm(&mut s);
    press(&mut s, "s");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), joined_bytes(""));
    assert_eq!(quit_effect(&s), Some((0, QuitRescue::None, Vec::new())));
}

// `app.save_and_quit` も同じ: キャンセルで終了しない
#[test]
fn 保存して終了でも確認をキャンセルすると終了しない() {
    let mut s = sjis_joined();
    run(&mut s, "app.save_and_quit", Args::new());
    assert_merge_confirm(&mut s);
    esc(&mut s);
    assert!(quit_effect(&s).is_none());
    assert!(doc(&s).is_dirty());
}

// 「保存を中止して終了」の後に届いた `EscapeMerge` は、何も書いていないのでそのまま終了へ進む (dirty な文書は退避される)
#[test]
fn 保存を中止して終了の後に届いたつながりはそのまま終了へ進む() {
    let mut s = sjis_joined();
    let before = s.fs.read("/w/a.txt").unwrap();
    let CommandResult::Pending(job) = s.cmd("file.save") else {
        panic!("保存のジョブを投入する")
    };
    s.cmd("app.quit");
    s.advance_ms(5000);
    assert_eq!(s.overlay(), "dialog", "保存中の終了の確認");
    // 保存を中止して終了。ジョブは取消しより先に走査を終えて `EscapeMerge` を返した (取消しの結果は捨てられる)
    s.answer(1);
    s.editor.on_job_result(JobOutput {
        job: JobId(job.0),
        body: JobResult::EscapeMerge {
            total: 1,
            recorded: vec![EscapeMergeAt {
                char_index: 1,
                len_chars: 2,
                decoded: "\u{3000}".to_owned(),
            }],
            digest: 0,
            changed_chars: 2,
            changed_lines: 1,
            to_eof: false,
        },
    });
    s.run_jobs();
    s.advance_ms(0);
    let (code, rescue, stderr) = quit_effect(&s).expect("終了する");
    assert_eq!(code, 0);
    assert_eq!(
        rescue,
        QuitRescue::DirtyDocuments,
        "dirty な文書は退避される"
    );
    assert_eq!(stderr, ["stderr.quit_abort.save_failed"]);
    assert_eq!(s.overlay(), "", "確認は出さない");
    assert_eq!(s.fs.read("/w/a.txt").unwrap(), before, "何も書かない");
}

// ---------- E3 のエスケープ文字が理由の本文 ----------

/// `Shift_JIS` の `x<0x81>` の文書 (解除した後)。
fn sjis_released() -> Harness {
    let mut s = env();
    s.fs.add_file("/w/a.txt", &[b"x\x81\n", PAD.as_bytes()].concat());
    open_as(&mut s, "/w/a.txt", Encoding::ShiftJis);
    release_invalid_bytes(&mut s);
    s
}

// verifies: NFR-304
#[test]
fn エスケープ文字が理由の_e3_は不正なバイトの説明を本文にする() {
    let mut s = sjis_released();
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "UTF-8"),
    );
    let (title, message, _, keys, _) = dialog(&mut s);
    assert_eq!(title, "表現できない文字");
    assert_eq!(
        message,
        "不正なバイト (<0xNN>) 1 個は UTF-8 では保存できません (最初: 行 1, 桁 2)。\
         消すか正しい文字に打ち直してから保存してください"
    );
    assert_eq!(keys, [None], "キャンセルだけ");
}

// ほかの表現不能の文字があれば、その数を添える。「最初」は最初のエスケープ文字の位置
#[test]
fn エスケープ文字が理由の_e3_はほかの表現不能の文字の数を添える() {
    let mut s = sjis_released();
    s.keys("ctrl+home");
    s.text("😀");
    run(
        &mut s,
        "file.set_encoding",
        Args::new().with_str("encoding", "EUC-JP"),
    );
    let (_, message, _, keys, _) = dialog(&mut s);
    assert_eq!(
        message,
        "不正なバイト (<0xNN>) 1 個は EUC-JP では保存できません (最初: 行 1, 桁 3)。\
         ほかに表せない文字が 1 個あります。消すか正しい文字に打ち直してから保存してください"
    );
    assert_eq!(keys, [None], "キャンセルだけ");
}
