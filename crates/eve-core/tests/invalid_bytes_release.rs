//! 不正バイトを含む文書の「編集を許可」(`file.toggle_read_only` で `InvalidBytes` を確認の後に外す)、解除の持ち越し、
//! 理由ごとの編集の拒否のメッセージ、復旧の提案と適用 (docs/03 §8.3.1 の「編集を許可」「関連する場面」、§9.6 の
//! 「M23 の規則」、§10.3、03 §22.3 の「不正バイトを含む文書の編集」の (a)(b)(h)。v0.4 の M23 の 6 本目)。

use eve_core::{
    Args, CommandInvocation, CommandResult, Document, Encoding, FileTime, FsPath, InvocationSource,
    OpenOptions, ReadOnlyReason, SwapBody, SwapEntry, SwapOwner,
};
use eve_encoding::PuaEscapeMode;
use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_text::LineEnding;
use eve_view::{OverlayModel, StatusKind};

/// 不正バイトの比率の確認 (FR-127、1% を超えると出る) を出さないための後ろの詰め物。
const PAD: &str = "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\n\
                   0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\n";

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

fn reasons(h: &Harness) -> Vec<ReadOnlyReason> {
    doc(h).read_only.iter().collect()
}

fn sjis(s: &str) -> Vec<u8> {
    encoding_rs::SHIFT_JIS.encode(s).0.into_owned()
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// 確認 (開くときの確認) が出ていれば「読み取り専用で開く」を選ぶ (確認が続くときは続けて答える)。
fn accept_read_only(h: &mut Harness) {
    for _ in 0..3 {
        if h.overlay() != "dialog" {
            break;
        }
        h.text("r");
        h.run_jobs();
    }
}

/// 文字コードを指定して開く (判定に任せない)。
fn open_as(h: &mut Harness, path: &str, enc: Encoding) {
    h.editor.state_mut().open_path(
        path,
        OpenOptions {
            forced: Some(enc),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    accept_read_only(h);
}

/// `bytes` のファイル `/w/a.txt` を `enc` で開いた状態。
fn opened(bytes: &[u8], enc: Encoding) -> Harness {
    let mut h = Harness::new()
        .with_size(100, 20)
        .with_file("/w/a.txt", bytes);
    open_as(&mut h, "/w/a.txt", enc);
    h
}

/// 出ている確認の (題, 本文, 詳細, 選択肢のアクセラレータ, 既定の番号)。
fn dialog(h: &mut Harness) -> (String, String, Vec<String>, Vec<Option<char>>, u16) {
    h.frame()
        .overlays
        .iter()
        .find_map(|o| match o {
            OverlayModel::Confirm {
                title,
                message,
                details,
                choices,
                default,
                ..
            } => Some((
                cell_text(title),
                cell_text(message),
                details
                    .as_ref()
                    .map(|d| d.iter().map(cell_text).collect())
                    .unwrap_or_default(),
                choices.iter().map(|c| c.key).collect(),
                default.0,
            )),
            _ => None,
        })
        .expect("確認が出ている")
}

/// 今のメッセージの (本文, ヒント)。
fn shown_message(h: &mut Harness) -> (String, String) {
    let m = h.frame().messages.current.expect("メッセージが出ている");
    (
        cell_text(&m.text),
        m.hint.as_ref().map(cell_text).unwrap_or_default(),
    )
}

/// 「編集を許可」(`file.toggle_read_only` → 確認の `[A]`)。
fn release(h: &mut Harness) {
    assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
    assert_eq!(
        h.overlay(),
        "dialog",
        "編集を許可の確認: {:?}",
        h.messages()
    );
    h.text("a");
    assert_eq!(h.overlay(), "");
    assert!(doc(h).is_editable(), "{:?}", reasons(h));
    assert!(doc(h).invalid_bytes_released);
}

/// パレットで `file.toggle_read_only` の行の、実行できない理由 (淡色でなければ `None`)。
fn palette_toggle_reason(h: &mut Harness) -> Option<String> {
    h.keys("ctrl+p");
    h.text("file.toggle_read_only");
    let f = h.frame();
    let item = f
        .overlays
        .iter()
        .find_map(|o| match o {
            OverlayModel::List { items, .. } => items
                .iter()
                .find(|i| {
                    i.detail
                        .as_ref()
                        .is_some_and(|d| cell_text(d) == "file.toggle_read_only")
                })
                .cloned(),
            _ => None,
        })
        .expect("パレットにコマンドがある");
    h.keys("esc");
    item.disabled_reason.as_ref().map(cell_text)
}

/// 解除できる文書の種類 (文字コード、ファイル、最初の不正なバイトの (行, 桁))。
fn releasable_cases() -> Vec<(Encoding, Vec<u8>, (usize, usize))> {
    vec![
        (
            Encoding::Utf8,
            [b"ab\n\x80cd\xFF\n", PAD.as_bytes()].concat(),
            (2, 1),
        ),
        (
            Encoding::ShiftJis,
            [
                sjis("日本\n"),
                b"x\xA0y\x81\n".to_vec(),
                PAD.as_bytes().to_vec(),
            ]
            .concat(),
            (2, 2),
        ),
        (
            Encoding::EucJp,
            [b"abc\nde\xFFf\x8F\n", PAD.as_bytes()].concat(),
            (2, 3),
        ),
        // UTF-16LE: 孤立サロゲート (D800) と、末尾の奇数長の 1 バイト
        (
            Encoding::Utf16Le,
            [
                utf16le("ab\nc"),
                b"\x00\xD8".to_vec(),
                utf16le(&format!("d\n{PAD}")),
                b"A".to_vec(),
            ]
            .concat(),
            (2, 2),
        ),
    ]
}

// ---------- (a) 解除と確認 ----------

// verifies: AC-NFR-304-4, FR-116
#[test]
fn 解除できる文書で確認が出て_キャンセルでは変えず_編集を許可で編集できる() {
    for (enc, bytes, (line, col)) in releasable_cases() {
        let mut h = opened(&bytes, enc);
        assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes], "{enc:?}");
        assert_eq!(doc(&h).format.escape_encoding, Some(enc));
        assert!(
            palette_toggle_reason(&mut h).is_none(),
            "{enc:?}: パレットで淡色にしない"
        );
        // 確認: 本文に名前・文字コード・数・最初の位置、詳細に 4 行、選択肢は [A] (既定)・[E]・キャンセル
        assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
        let (title, message, details, keys, default) = dialog(&mut h);
        assert_eq!(title, "不正なバイトを含む文書の編集");
        let count = doc(&h).format.invalid_bytes;
        assert!(
            message.starts_with(&format!(
                "/w/a.txt には {} として不正なバイトが {count} 個あります (最初: 行 {line}, 桁 {col})。",
                enc.name()
            )),
            "{enc:?}: {message}"
        );
        assert!(message.contains("文字化けして見えるときは"), "{message}");
        assert_eq!(
            details,
            [
                "保存すると、残っている <0xNN> は元のバイトに戻ります",
                "消すと、そのバイトはファイルから無くなります (保存の前なら元に戻すで戻せます)",
                "隣の文字とつながると、開き直したときに別の文字になります (保存の前に確認します)",
                "他のアプリに貼ると <0xNN> という文字列になります",
            ]
        );
        assert_eq!(keys, [Some('a'), Some('e'), None], "{enc:?}");
        assert_eq!(default, 0, "既定は「編集を許可」");
        // キャンセルでは何も変えない
        h.keys("esc");
        assert_eq!(h.overlay(), "");
        assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
        assert!(!doc(&h).invalid_bytes_released);
        // 編集を許可
        h.clear_log();
        release(&mut h);
        assert!(
            h.messages()
                .contains(&"info.read_only.released_invalid_bytes".to_owned())
        );
        let before = h.doc_text();
        h.keys("ctrl+home");
        h.text("Z");
        assert_eq!(h.doc_text(), format!("Z{before}"), "{enc:?}: 編集できる");
    }
}

// verifies: FR-127, AC-NFR-304-4
#[test]
fn 不正バイトの比率の確認で読み取り専用で開いた文書も解除できる() {
    let mut h = Harness::new().with_file("/w/bad.txt", b"ok \x9A\xFF end\n");
    h.open("/w/bad.txt");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "不正バイトの比率の確認");
    h.text("r");
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    release(&mut h);
    h.keys("ctrl+end");
    h.text("!");
    assert_eq!(h.doc_text(), "ok \u{10FF9A}\u{10FFFF} end\n!");
}

// verifies: FR-111, AC-NFR-304-4
#[test]
fn 確認の_e_で文字コードの入力に進み_指定した文字コードで開き直す() {
    let mut h = opened(&[b"a\x82\xA0b\n", PAD.as_bytes()].concat(), Encoding::Utf8);
    h.cmd("file.toggle_read_only");
    h.text("e");
    assert_eq!(h.overlay(), "prompt", "文字コードの入力");
    h.text("shift_jis");
    h.keys("enter");
    h.run_jobs();
    assert_eq!(doc(&h).format.encoding, Encoding::ShiftJis);
    // Shift_JIS では 82 A0 は「あ」なので、開き直した文書に不正バイトは無く、編集できる
    assert_eq!(h.doc_text(), format!("a\u{3042}b\n{PAD}"));
    assert!(doc(&h).is_editable(), "{:?}", reasons(&h));
}

// verifies: AC-NFR-304-4
#[test]
fn 無題の文書の確認には開き直しの選択肢を出さない() {
    // 無題の文書への復旧で由来の文字コードが付いた文書 (03 §8.3.1 の細目 (d))
    let mut h = Harness::new();
    h.fs.add_swap_entry(swap_entry(None), Some(swap_body("x\u{10FF80}y", true)));
    h.run_jobs();
    h.advance_ms(0);
    assert_eq!(h.overlay(), "dialog", "復旧の提案");
    h.answer(0);
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
    let (_, message, _, keys, _) = dialog(&mut h);
    assert_eq!(keys, [Some('a'), None], "開き直しは出さない");
    assert!(message.contains("(最初: 行 1, 桁 2)"), "{message}");
    h.text("a");
    assert!(doc(&h).is_editable());
}

// ---------- 解除できない文書 ----------

/// 解除できない文書: パレットで淡色になって解除できない理由を示し、切り替えても何もせず、編集の拒否は理由ごとの
/// メッセージ (「切替で編集できる」と言わない)。
fn assert_not_releasable(h: &mut Harness, reason_text: &str, key: &str, hint_has: &str) {
    let before = reasons(h);
    let why = palette_toggle_reason(h).expect("パレットで淡色");
    assert_eq!(why, format!("現在は実行できません: {reason_text}"));
    assert!(!matches!(
        h.cmd("file.toggle_read_only"),
        CommandResult::Done
    ));
    assert_eq!(h.overlay(), "", "確認を出さない");
    assert_eq!(reasons(h), before);
    h.clear_log();
    h.text("x");
    assert!(h.messages().contains(&key.to_owned()), "{:?}", h.messages());
    let (_, hint) = shown_message(h);
    assert!(hint.contains(hint_has), "{hint}");
    assert!(!hint.contains("確認の後に編集"), "{hint}");
}

// verifies: AC-NFR-304-4, FR-116
#[test]
fn 私用文字と衝突した文書は解除できず_理由の文言を分ける() {
    // 真の私用文字 U+10FF41 と不正バイト 0x80 の両方 (衝突の文書 (b))
    let mut h = opened(
        &[b"a\xF4\x8F\xBD\x81b\x80\n", PAD.as_bytes()].concat(),
        Encoding::Utf8,
    );
    assert_eq!(doc(&h).format.pua_escape, PuaEscapeMode::DisabledCollision);
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    assert_not_releasable(
        &mut h,
        "不正なバイトと私用文字を含むファイル",
        "msg.edit.read_only_pua_collision",
        "別の文字コードとして開き直してください",
    );
}

// verifies: AC-NFR-304-4, FR-116
#[test]
fn 不正バイトを含む_iso_2022_jp_は解除できない() {
    let mut h = opened(
        &[&b"\x1B$B$\"\n\x1B(Bx"[..], PAD.as_bytes()].concat(),
        Encoding::Iso2022Jp,
    );
    assert_eq!(
        reasons(&h),
        [ReadOnlyReason::StatefulEncodingWithInvalidBytes]
    );
    assert_not_releasable(
        &mut h,
        "不正なバイトを含む ISO-2022-JP のファイル",
        "msg.edit.read_only_stateful",
        "UTF-8 などとして開き直すと編集できます",
    );
}

// verifies: FR-115, FR-116
#[test]
fn バイナリは解除できない() {
    let mut h = Harness::new().with_file("/w/bin.dat", b"a\0b\0c\0\n");
    h.open("/w/bin.dat");
    h.run_jobs();
    accept_read_only(&mut h);
    assert_eq!(reasons(&h), [ReadOnlyReason::Binary]);
    assert_not_releasable(
        &mut h,
        "バイナリファイル",
        "msg.edit.read_only_binary",
        "テキストなら",
    );
}

// verifies: FR-127, FR-116
#[test]
fn 巨大ファイルとして開いた不正バイトの文書は解除できない() {
    let mut h = Harness::new().with_file("/w/big.txt", &[b"a\x80b\n", PAD.as_bytes()].concat());
    h.fs.set_fake_len("/w/big.txt", Some(512 * 1024 * 1024));
    h.open("/w/big.txt");
    h.run_jobs();
    accept_read_only(&mut h);
    assert_eq!(
        reasons(&h),
        [
            ReadOnlyReason::InvalidBytes,
            ReadOnlyReason::LargeFileViewer
        ]
    );
    // 専用のメッセージは無いので `msg.edit.read_only` で、理由は解除できない理由
    assert_not_releasable(
        &mut h,
        "巨大なファイル (読み取り専用で開く指定)",
        "msg.edit.read_only",
        "名前を付けて保存",
    );
    let (text, _) = shown_message(&mut h);
    assert!(text.contains("巨大なファイル"), "{text}");
}

// verifies: AC-NFR-304-4, FR-116
#[test]
fn 解除できる不正バイトの文書の編集の拒否は切替の案内を先頭に出す() {
    let mut h = opened(&[b"a\x80b\n", PAD.as_bytes()].concat(), Encoding::Utf8);
    h.clear_log();
    h.text("x");
    assert!(
        h.messages()
            .contains(&"msg.edit.read_only_invalid_bytes".to_owned())
    );
    let (text, hint) = shown_message(&mut h);
    assert_eq!(text, "不正なバイトを含むため読み取り専用です");
    assert!(
        hint.contains(" で、確認の後に編集できます。文字化けなら "),
        "{hint}"
    );
    // ステータスバーは読み取り専用
    let f = h.frame();
    assert!(
        f.status
            .items
            .iter()
            .any(|i| i.kind == StatusKind::ReadOnly),
        "[RO]"
    );
}

// ---------- 切替の繰り返し・属性との組 ----------

// verifies: FR-116, AC-NFR-304-4
#[test]
fn 解除した後にもう一度切り替えると確認なしに読み取り専用にし_次の解除も確認なし() {
    let mut h = opened(&[b"a\x80b\n", PAD.as_bytes()].concat(), Encoding::Utf8);
    release(&mut h);
    h.cmd("file.toggle_read_only");
    assert_eq!(h.overlay(), "");
    assert_eq!(reasons(&h), [ReadOnlyReason::UserRequested]);
    assert!(doc(&h).invalid_bytes_released, "解除の記録は残す");
    h.clear_log();
    assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
    assert_eq!(h.overlay(), "", "InvalidBytes が無いので確認しない");
    assert!(doc(&h).is_editable());
    assert!(h.messages().contains(&"info.read_only.released".to_owned()));
}

// verifies: FR-116, FR-106, AC-NFR-304-4
#[test]
fn 属性と不正バイトの両方の文書は確認の後に両方を外し_上書き保存は属性の確認を経る() {
    let bytes = [b"a\x80b\n", PAD.as_bytes()].concat();
    let mut h = opened(&bytes, Encoding::Utf8);
    // 属性を付けて開き直す (理由は FileAttribute と InvalidBytes)
    h.fs.set_readonly("/w/a.txt", true);
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    h.run_jobs();
    assert_eq!(
        reasons(&h),
        [ReadOnlyReason::FileAttribute, ReadOnlyReason::InvalidBytes]
    );
    release(&mut h);
    assert!(doc(&h).attribute_released);
    h.keys("ctrl+end");
    h.text("z");
    h.keys("ctrl+s");
    assert_eq!(
        h.overlay(),
        "dialog",
        "読み取り専用属性のファイルの上書き保存の確認"
    );
    h.text("w");
    h.run_jobs();
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), [&bytes[..], b"z"].concat());
}

// ---------- (b) 保存 (NFR-304 AC-4) ----------

// verifies: AC-NFR-304-4
#[test]
fn 解除の後に不正バイト以外を編集して保存すると不正バイトは元のバイト値のまま同じ順で残る() {
    for (enc, bytes, _) in releasable_cases() {
        let mut h = opened(&bytes, enc);
        release(&mut h);
        h.keys("ctrl+home");
        h.text("Z");
        h.cmd("file.save");
        h.run_jobs();
        assert_eq!(h.overlay(), "", "{enc:?}: {:?}", h.messages());
        let z = match enc {
            Encoding::Utf16Le => b"Z\x00".to_vec(),
            _ => b"Z".to_vec(),
        };
        let want = [z.clone(), bytes.clone()].concat();
        assert_eq!(h.fs_read("/w/a.txt").unwrap(), want, "{enc:?}: 上書き保存");
        // 同じ文字コードの名前を付けて保存も同じ
        h.cmd_with("file.save_as", Args::new().with_str("path", "/w/b.txt"));
        h.run_jobs();
        assert_eq!(
            h.fs_read("/w/b.txt").unwrap(),
            want,
            "{enc:?}: 名前を付けて保存"
        );
    }
}

// verifies: AC-NFR-304-4
#[test]
fn 不正バイトを含む行を上に動かして保存するとバイトのまま移る() {
    let rest = [b"x\xA0y\n", PAD.as_bytes()].concat();
    let mut h = opened(&[b"ab\n".as_slice(), &rest].concat(), Encoding::ShiftJis);
    release(&mut h);
    h.keys("ctrl+home");
    h.keys("down");
    h.cmd("edit.move_line_up");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "", "{:?}", h.messages());
    let mut want = b"x\xA0y\nab\n".to_vec();
    want.extend_from_slice(PAD.as_bytes());
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), want);
}

// ---------- 解除の持ち越し (再読込・開き直し・新しく開く) ----------

// verifies: AC-NFR-304-4, FR-117
#[test]
fn 解除した文書を再読込すると解除を持ち越し_再読込の_undo_redo_ができる() {
    let mut h = opened(&[b"a\x80b\n", PAD.as_bytes()].concat(), Encoding::Utf8);
    release(&mut h);
    // 外部で別の不正バイトを含む内容に変わった
    h.fs.add_file("/w/a.txt", &[b"c\xFEd\xFF\n", PAD.as_bytes()].concat());
    h.clear_log();
    h.cmd("file.reload");
    h.run_jobs();
    assert_eq!(h.doc_text(), format!("c\u{10FFFE}d\u{10FFFF}\n{PAD}"));
    assert!(doc(&h).is_editable(), "{:?}", reasons(&h));
    assert!(doc(&h).invalid_bytes_released);
    let msgs = h.messages();
    assert!(msgs.contains(&"info.reload.done".to_owned()), "{msgs:?}");
    assert!(!msgs.contains(&"info.reload.done_read_only".to_owned()));
    // 再読込の Undo / Redo ができ、どちらでも読み取り専用に戻らない
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), format!("a\u{10FF80}b\n{PAD}"));
    assert!(doc(&h).is_editable());
    h.keys("ctrl+y");
    assert_eq!(h.doc_text(), format!("c\u{10FFFE}d\u{10FFFF}\n{PAD}"));
    assert!(doc(&h).is_editable());
    // 編集して保存すると、新しい内容の不正バイトが元のまま残る
    h.keys("ctrl+home");
    h.text("Z");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(
        h.fs_read("/w/a.txt").unwrap(),
        [b"Zc\xFEd\xFF\n", PAD.as_bytes()].concat()
    );
}

// verifies: AC-NFR-304-4, FR-117
#[test]
fn 外部変更からの再読込でも解除を持ち越す() {
    let mut h = opened(&[b"a\x80b\n", PAD.as_bytes()].concat(), Encoding::Utf8);
    release(&mut h);
    h.fs.add_file("/w/a.txt", &[b"c\xFEd\n", PAD.as_bytes()].concat());
    h.focus(false);
    h.focus(true);
    h.run_jobs();
    assert_eq!(h.overlay(), "");
    assert_eq!(h.doc_text(), format!("c\u{10FFFE}d\n{PAD}"));
    assert!(doc(&h).is_editable(), "{:?}", reasons(&h));
}

// verifies: AC-NFR-304-4, FR-117
#[test]
fn 解除していない文書の再読込では読み取り専用のままで_元に戻せないと示す() {
    let mut h = opened(&[b"a\x80b\n", PAD.as_bytes()].concat(), Encoding::Utf8);
    h.fs.add_file("/w/a.txt", &[b"c\xFEd\n", PAD.as_bytes()].concat());
    h.clear_log();
    h.cmd("file.reload");
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    assert!(
        h.messages()
            .contains(&"info.reload.done_read_only".to_owned())
    );
    // 解除すると、再読込の Undo ができる
    release(&mut h);
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), format!("a\u{10FF80}b\n{PAD}"));
    assert!(doc(&h).is_editable(), "解除は Undo で戻らない");
}

// verifies: AC-NFR-304-4, FR-117
#[test]
fn 再読込で私用文字と衝突した内容になったら解除を持ち越さない() {
    let mut h = opened(&[b"a\x80b\n", PAD.as_bytes()].concat(), Encoding::Utf8);
    release(&mut h);
    h.fs.add_file(
        "/w/a.txt",
        &[b"a\xF4\x8F\xBD\x81b\x80\n", PAD.as_bytes()].concat(),
    );
    h.clear_log();
    h.cmd("file.reload");
    h.run_jobs();
    assert_eq!(doc(&h).format.pua_escape, PuaEscapeMode::DisabledCollision);
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    assert!(
        h.messages()
            .contains(&"info.reload.done_read_only".to_owned())
    );
    h.text("x");
    assert!(
        h.messages()
            .contains(&"msg.edit.read_only_pua_collision".to_owned())
    );
}

// verifies: AC-NFR-304-4, FR-111
#[test]
fn 解除した文書を文字コードを指定して開き直すと読み取り専用に戻る() {
    let mut h = opened(&[b"a\x80b\n", PAD.as_bytes()].concat(), Encoding::Utf8);
    release(&mut h);
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    assert!(!doc(&h).invalid_bytes_released, "記録を消す");
    // もう一度解除するには確認を経る
    h.cmd("file.toggle_read_only");
    assert_eq!(h.overlay(), "dialog");
}

// verifies: AC-NFR-304-4
#[test]
fn 解除した文書を閉じて新しく開くと読み取り専用に戻る() {
    let mut h = opened(&[b"a\x80b\n", PAD.as_bytes()].concat(), Encoding::Utf8);
    release(&mut h);
    h.cmd("buffer.close");
    h.run_jobs();
    open_as(&mut h, "/w/a.txt", Encoding::Utf8);
    assert_eq!(h.doc_path().as_deref(), Some("/w/a.txt"));
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    assert!(!doc(&h).invalid_bytes_released);
}

// ---------- (h) 復旧 ----------

fn swap_entry(path: Option<&str>) -> SwapEntry {
    SwapEntry {
        dir: FsPath::from(if path.is_some() {
            "/s/swap/aaaa-4242.swp"
        } else {
            "/s/swap/untitled-1-4242.swp"
        }),
        path: path.map(FsPath::from),
        untitled_name: path.is_none().then(|| "無題".to_owned()),
        revision: 7,
        len: 3,
        pid: 4242,
        owner: SwapOwner::Stale,
        sensitive: false,
        modified: Some(FileTime {
            secs: 4_000_000_000,
            nanos: 0,
        }),
        readable: true,
        encoding: None,
    }
}

fn swap_body(text: &str, pua_escaped: bool) -> SwapBody {
    SwapBody {
        text: text.to_owned(),
        encoding: Encoding::Utf8,
        has_bom: false,
        default_eol: LineEnding::Lf,
        pua_escaped,
    }
}

/// `bytes` の `/w/a.txt` を開き、スワップ `body` を積んだ状態 (提案は `advance_ms(0)` で出る)。
fn with_swap(bytes: &[u8], body: SwapBody, enc: Encoding) -> Harness {
    let mut h = Harness::new()
        .with_size(100, 20)
        .with_file("/w/a.txt", bytes);
    h.fs.add_swap_entry(swap_entry(Some("/w/a.txt")), Some(body));
    open_as(&mut h, "/w/a.txt", enc);
    h
}

// 異常終了の後に開くと、不正バイトを含む文書は InvalidBytes で読み取り専用に戻るが、復旧が提案される。適用すると
// 内容が一致し、読み取り専用のまま案内が出る。解除して編集を続け、保存すると不正バイトが元のまま残る
// verifies: AC-NFR-304-4, FR-118, NFR-307
#[test]
fn 解除して編集した文書の異常終了から復旧し_解除して続けられる() {
    let bytes = [
        sjis("日本\n"),
        b"x\xA0y\n".to_vec(),
        PAD.as_bytes().to_vec(),
    ]
    .concat();
    // 1 回目の起動: 解除して編集し、スワップが書かれる
    let mut first = opened(&bytes, Encoding::ShiftJis);
    release(&mut first);
    first.keys("ctrl+home");
    first.text("追加");
    first.advance_ms(0);
    first.advance_ms(2_000);
    first.run_jobs();
    let swapped = first
        .fs
        .swap_of(first.doc_id())
        .expect("スワップが書かれる");
    let text = String::from_utf8(swapped).unwrap();
    assert_eq!(text, first.doc_text());
    assert!(text.contains('\u{10FFA0}'), "本文はエスケープ文字のまま");
    // 「クラッシュ」: 2 回目の起動でスワップが見つかる (manifest の pua_escaped は escape_encoding.is_some())
    let mut h = Harness::new()
        .with_size(100, 20)
        .with_file("/w/a.txt", &bytes);
    h.fs.add_swap_entry(
        swap_entry(Some("/w/a.txt")),
        Some(SwapBody {
            text: text.clone(),
            encoding: Encoding::ShiftJis,
            has_bom: false,
            default_eol: LineEnding::Lf,
            pua_escaped: true,
        }),
    );
    open_as(&mut h, "/w/a.txt", Encoding::ShiftJis);
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    h.advance_ms(0);
    assert_eq!(h.overlay(), "dialog", "読み取り専用でも復旧を提案する");
    h.clear_log();
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), text, "内容が一致する");
    assert!(h.is_dirty());
    assert_eq!(
        reasons(&h),
        [ReadOnlyReason::InvalidBytes],
        "読み取り専用のまま"
    );
    assert_eq!(doc(&h).format.escape_encoding, Some(Encoding::ShiftJis));
    let msgs = h.messages();
    assert!(
        msgs.contains(&"info.swap.applied_read_only".to_owned()),
        "{msgs:?}"
    );
    assert!(!msgs.contains(&"info.swap.recovered".to_owned()));
    // 解除して続ける
    release(&mut h);
    h.keys("ctrl+end");
    h.text("!");
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "", "{:?}", h.messages());
    let mut want = sjis("追加日本\n");
    want.extend_from_slice(b"x\xA0y\n");
    want.extend_from_slice(PAD.as_bytes());
    want.push(b'!');
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), want);
}

// verifies: AC-NFR-304-4, FR-118
#[test]
fn 復旧を適用した後に解除すると復旧の_undo_ができる() {
    let bytes = [b"a\x80b\n", PAD.as_bytes()].concat();
    let mut h = with_swap(
        &bytes,
        swap_body(&format!("Za\u{10FF80}b\n{PAD}"), true),
        Encoding::Utf8,
    );
    h.advance_ms(0);
    h.answer(0);
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    // 読み取り専用の間は Undo できない
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), format!("Za\u{10FF80}b\n{PAD}"));
    release(&mut h);
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), format!("a\u{10FF80}b\n{PAD}"), "復旧の前");
    assert!(doc(&h).is_editable(), "解除は Undo で戻らない");
    h.keys("ctrl+y");
    assert_eq!(h.doc_text(), format!("Za\u{10FF80}b\n{PAD}"));
    assert!(doc(&h).is_editable());
}

// verifies: AC-NFR-304-4, FR-118
#[test]
fn 解除した文書に復旧を適用すると解除の記録を付け直す() {
    let bytes = [b"a\x80b\n", PAD.as_bytes()].concat();
    let mut h = Harness::new()
        .with_size(100, 20)
        .with_file("/w/a.txt", &bytes);
    h.fs.add_swap_entry(
        swap_entry(Some("/w/a.txt")),
        Some(swap_body(&format!("Za\u{10FF80}b\n{PAD}"), true)),
    );
    h.editor.state_mut().open_path(
        "/w/a.txt",
        OpenOptions {
            forced: Some(Encoding::Utf8),
            ..OpenOptions::default()
        },
    );
    // 起動時の走査と読込を 1 周だけ済ませる (提案は次の tick で出る)。提案の前に解除する (dirty でない編集できる
    // 文書にも提案は出る)。`Harness::cmd` は実行の前に tick を進めて提案を出すので、`Editor::execute` を直に呼ぶ
    h.run_jobs_once();
    assert_eq!(h.overlay(), "", "まだ提案していない");
    let r = h.editor.execute(CommandInvocation::new(
        "file.toggle_read_only",
        InvocationSource::Keymap,
    ));
    assert_eq!(r, CommandResult::Done);
    assert_eq!(h.overlay(), "dialog", "編集を許可の確認");
    h.editor.state_mut().dialog_accelerator('a');
    assert!(doc(&h).is_editable() && doc(&h).invalid_bytes_released);
    h.advance_ms(0);
    assert_eq!(h.overlay(), "dialog", "復旧の提案");
    h.answer(0);
    h.run_jobs();
    assert!(!doc(&h).invalid_bytes_released, "記録を消す");
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    assert!(
        h.messages()
            .contains(&"info.swap.applied_read_only".to_owned())
    );
}

// verifies: FR-118, NFR-307
#[test]
fn ほかの理由の読み取り専用の文書には復旧を提案しない() {
    let body = || swap_body(&format!("Za\u{10FF80}b\n{PAD}"), true);
    let bytes = [b"a\x80b\n", PAD.as_bytes()].concat();
    // 読み取り専用属性と不正バイトの両方
    let mut h = Harness::new()
        .with_size(100, 20)
        .with_file("/w/a.txt", &bytes);
    h.fs.set_readonly("/w/a.txt", true);
    h.fs.add_swap_entry(swap_entry(Some("/w/a.txt")), Some(body()));
    open_as(&mut h, "/w/a.txt", Encoding::Utf8);
    assert_eq!(
        reasons(&h),
        [ReadOnlyReason::FileAttribute, ReadOnlyReason::InvalidBytes]
    );
    h.advance_ms(0);
    assert_eq!(h.overlay(), "", "属性もある文書には出さない");
    // 私用文字と衝突した文書 (b)
    let mut h = with_swap(
        &[b"a\xF4\x8F\xBD\x81b\x80\n", PAD.as_bytes()].concat(),
        body(),
        Encoding::Utf8,
    );
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    h.advance_ms(0);
    assert_eq!(h.overlay(), "", "衝突の文書には出さない");
    // 不正バイトを含む ISO-2022-JP
    let mut h = with_swap(
        &[&b"\x1B$B$\"\n\x1B(Bx"[..], PAD.as_bytes()].concat(),
        body(),
        Encoding::Iso2022Jp,
    );
    h.advance_ms(0);
    assert_eq!(h.overlay(), "", "ISO-2022-JP には出さない");
    // `--readonly` で開いた文書
    let mut h = Harness::new()
        .with_size(100, 20)
        .with_file("/w/a.txt", b"abc\n");
    h.fs.add_swap_entry(swap_entry(Some("/w/a.txt")), Some(swap_body("xyz", false)));
    h.editor.state_mut().open_path(
        "/w/a.txt",
        OpenOptions {
            user_read_only: true,
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
    h.advance_ms(0);
    assert_eq!(h.overlay(), "", "--readonly の文書には出さない");
}

// ---------- データの損失の視点のレビュー (M23 の 6 本目) ----------

/// 文字コードを判定に任せて開く。
fn open_auto(h: &mut Harness, path: &str) {
    h.editor.state_mut().open_path(path, OpenOptions::default());
    h.run_jobs();
    accept_read_only(h);
}

fn count_of(h: &Harness, key: &str) -> usize {
    h.messages().iter().filter(|m| *m == key).count()
}

// 判定が外れて UTF-8 + 不正バイトで開いた Shift_JIS のファイルに、Shift_JIS で書かれたスワップがある。提案すると、
// 適用・解除・保存で Shift_JIS のファイルが確認なしに UTF-8 で書かれる (レビューの再現)。提案せずに案内し、利用者が
// Shift_JIS で開き直すと提案して、Shift_JIS のバイト列で保存する
// verifies: AC-NFR-304-4, FR-118, FR-111
#[test]
fn 文字コードの違うスワップは提案せずに案内し_その文字コードで開き直すと提案する() {
    let bytes = sjis("ｱｲｳｴｵ\n");
    // 1 回目の起動: 判定は UTF-8 (不正バイト)。Shift_JIS で開き直して編集し、スワップが書かれる
    let mut first = Harness::new()
        .with_size(100, 20)
        .with_file("/w/a.txt", &bytes);
    open_auto(&mut first, "/w/a.txt");
    assert_eq!(doc(&first).format.encoding, Encoding::Utf8);
    assert_eq!(reasons(&first), [ReadOnlyReason::InvalidBytes]);
    first.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    first.run_jobs();
    assert!(doc(&first).is_editable());
    first.keys("ctrl+end");
    first.text("カキク");
    first.advance_ms(0);
    first.advance_ms(2_000);
    first.run_jobs();
    let text = String::from_utf8(first.fs.swap_of(first.doc_id()).expect("スワップ")).unwrap();
    assert_eq!(text, "ｱｲｳｴｵ\nカキク");
    // 2 回目の起動: 同じ判定で UTF-8 + InvalidBytes で開く
    let mut h = Harness::new()
        .with_size(100, 20)
        .with_file("/w/a.txt", &bytes);
    h.fs.add_swap_entry(
        swap_entry(Some("/w/a.txt")),
        Some(SwapBody {
            text: text.clone(),
            encoding: Encoding::ShiftJis,
            has_bom: false,
            default_eol: LineEnding::Lf,
            pua_escaped: false,
        }),
    );
    open_auto(&mut h, "/w/a.txt");
    assert_eq!(doc(&h).format.encoding, Encoding::Utf8);
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    h.clear_log();
    h.advance_ms(0);
    assert_eq!(h.overlay(), "", "文字コードが違うので提案しない");
    assert_eq!(count_of(&h, "info.swap.encoding_mismatch"), 1);
    let (msg, _) = shown_message(&mut h);
    assert!(
        msg.starts_with("/w/a.txt の未保存の内容は Shift_JIS で書かれています。"),
        "{msg}"
    );
    // 案内は 1 回だけ
    h.advance_ms(100);
    assert_eq!(count_of(&h, "info.swap.encoding_mismatch"), 1);
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), bytes, "何も書いていない");
    // Shift_JIS で開き直すと提案する
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    h.run_jobs();
    h.advance_ms(0);
    assert_eq!(h.overlay(), "dialog", "復旧の提案");
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), text);
    assert!(doc(&h).is_editable());
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "", "{:?}", h.messages());
    assert_eq!(h.fs_read("/w/a.txt").unwrap(), sjis("ｱｲｳｴｵ\nカキク"));
}

// 同じく文字コードの違う (エスケープ文字を持つ) スワップ。開き直した先も不正バイトを含む (理由が InvalidBytes だけの)
// 文書なら、提案 → 適用 → 解除 → 保存で、不正バイトも元のバイトのまま Shift_JIS で書く
// verifies: AC-NFR-304-4, FR-118
#[test]
fn 文字コードの違うスワップは開き直した不正バイトの文書に適用し_解除して保存できる() {
    let bytes = [
        sjis("日本語\n"),
        b"x\xA0y\n".to_vec(),
        PAD.as_bytes().to_vec(),
    ]
    .concat();
    let swapped = format!("追加日本語\nx\u{10FFA0}y\n{PAD}");
    let mut h = with_swap(
        &bytes,
        SwapBody {
            text: swapped.clone(),
            encoding: Encoding::ShiftJis,
            has_bom: false,
            default_eol: LineEnding::Lf,
            pua_escaped: true,
        },
        Encoding::Utf8,
    );
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    h.advance_ms(0);
    assert_eq!(h.overlay(), "", "UTF-8 の文書には提案しない");
    assert!(
        h.messages()
            .contains(&"info.swap.encoding_mismatch".to_owned())
    );
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "shift_jis"),
    );
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    h.advance_ms(0);
    assert_eq!(h.overlay(), "dialog", "復旧の提案");
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), swapped);
    assert_eq!(doc(&h).format.escape_encoding, Some(Encoding::ShiftJis));
    assert!(
        h.messages()
            .contains(&"info.swap.applied_read_only".to_owned())
    );
    release(&mut h);
    h.cmd("file.save");
    h.run_jobs();
    assert_eq!(h.overlay(), "", "{:?}", h.messages());
    assert_eq!(
        h.fs_read("/w/a.txt").unwrap(),
        [
            sjis("追加日本語\n"),
            b"x\xA0y\n".to_vec(),
            PAD.as_bytes().to_vec()
        ]
        .concat()
    );
}

// 由来の文字コードの無い文書 (`escape_encoding == None`) の InvalidBytes は解除できない (03 §8.3.1 の「解除できる
// 文書」)。同じ文字コードでも pua_escaped が偽のスワップを適用すると、この状態になる。確認は「元のバイトに戻る」と
// 事実と違う説明になり、保存も E3 で止まるため。編集の拒否は切替の案内をしない一般のメッセージ
// verifies: AC-NFR-304-4, FR-116
#[test]
fn 由来の文字コードの無い文書の不正バイトは解除できない() {
    let bytes = [b"a\x80b\n", PAD.as_bytes()].concat();
    let mut h = with_swap(
        &bytes,
        swap_body(&format!("ab\n{PAD}"), false),
        Encoding::Utf8,
    );
    h.advance_ms(0);
    h.answer(0);
    h.run_jobs();
    assert_eq!(h.doc_text(), format!("ab\n{PAD}"));
    assert_eq!(doc(&h).format.escape_encoding, None);
    assert_eq!(doc(&h).format.pua_escape, PuaEscapeMode::Enabled);
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    assert!(!doc(&h).read_only_releasable());
    assert_not_releasable(
        &mut h,
        "不正なバイトを含むファイル",
        "msg.edit.read_only",
        "名前を付けて保存",
    );
    let (text, _) = shown_message(&mut h);
    assert!(text.contains("不正なバイトを含むファイル"), "{text}");
}

// 解除した文書の再読込で、不正バイトの比率の確認 (FR-127) に「読み取り専用で開く」と答えたら、解除を持ち越さない
// verifies: AC-NFR-304-4, FR-127, FR-117
#[test]
fn 再読込の比率の確認で読み取り専用を選ぶと解除を持ち越さない() {
    let mut h = opened(&[b"a\x80b\n", PAD.as_bytes()].concat(), Encoding::Utf8);
    release(&mut h);
    let bin: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    h.fs.add_file("/w/a.txt", &bin);
    h.cmd("file.reload");
    h.run_jobs();
    assert_eq!(h.overlay(), "dialog", "比率の確認");
    h.text("r");
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    assert!(!doc(&h).invalid_bytes_released, "記録を消す");
    assert!(
        h.messages()
            .contains(&"info.reload.done_read_only".to_owned())
    );
    // もう一度解除するには確認を経る
    h.cmd("file.toggle_read_only");
    assert_eq!(h.overlay(), "dialog");
}

// 本文が同じ復旧を、解除した文書に適用しても、記録を消して読み取り専用に戻す (全置換は恒等の ChangeSet にならず、
// 書式の変更が当たる)
// verifies: AC-NFR-304-4, FR-118
#[test]
fn 本文が同じ復旧でも記録を消して読み取り専用に戻す() {
    let bytes = [b"a\x80b\n", PAD.as_bytes()].concat();
    let mut h = Harness::new()
        .with_size(100, 20)
        .with_file("/w/a.txt", &bytes);
    h.fs.add_swap_entry(
        swap_entry(Some("/w/a.txt")),
        Some(swap_body(&format!("a\u{10FF80}b\n{PAD}"), true)),
    );
    h.editor.state_mut().open_path(
        "/w/a.txt",
        OpenOptions {
            forced: Some(Encoding::Utf8),
            ..OpenOptions::default()
        },
    );
    h.run_jobs_once();
    let r = h.editor.execute(CommandInvocation::new(
        "file.toggle_read_only",
        InvocationSource::Keymap,
    ));
    assert_eq!(r, CommandResult::Done);
    h.editor.state_mut().dialog_accelerator('a');
    assert!(doc(&h).invalid_bytes_released);
    h.advance_ms(0);
    assert_eq!(h.overlay(), "dialog");
    h.answer(0);
    h.run_jobs();
    assert!(!doc(&h).invalid_bytes_released);
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
}

// 確認の不正なバイトの数と最初の位置は、本文のエスケープ文字から求める (復旧で本文が変わると、読込の数 (2 個) と
// 合わない)
// verifies: AC-NFR-304-4
#[test]
fn 復旧の後の確認の数と位置は本文のエスケープ文字から求める() {
    let bytes = [b"a\x80b\x81c\n", PAD.as_bytes()].concat();
    let mut h = with_swap(
        &bytes,
        swap_body(&format!("xy\nab\u{10FF81}c\n{PAD}"), true),
        Encoding::Utf8,
    );
    assert_eq!(doc(&h).format.invalid_bytes, 2);
    h.advance_ms(0);
    h.answer(0);
    h.run_jobs();
    assert_eq!(reasons(&h), [ReadOnlyReason::InvalidBytes]);
    assert_eq!(h.cmd("file.toggle_read_only"), CommandResult::Done);
    let (_, message, ..) = dialog(&mut h);
    assert!(
        message.starts_with(
            "/w/a.txt には UTF-8 として不正なバイトが 1 個あります (最初: 行 2, 桁 3)。"
        ),
        "{message}"
    );
}
