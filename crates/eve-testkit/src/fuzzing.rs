//! `fuzz_editor` の本体 (docs/09 §3.3 の L2、NFR-305)。cargo-fuzz のターゲット (`fuzz/fuzz_targets/fuzz_editor.rs`、Linux の夜間) と、
//! Windows の CI で決定的な乱数の入力を回す試験 (`tests/fuzz_editor.rs`) が同じ関数を使う (libFuzzer は MSVC 非対応のため)。
//!
//! 入力のバイト列を「初期文書 + ビューポートの大きさ + 操作の列」に読み替え、ヘッドレスの `Editor` に流す。各操作の後に Frame まで作り、
//! 次を検査する (違反はパニック = fuzz のクラッシュ):
//! - Frame の各行の幅 (ガター + Span) が Pane の幅以下 (`Span` の宣言幅の合計で数える)
//! - Span の文字列に制御文字 (`\x00..\x1f`・`\x7f`・`\u{80}..\u{9f}`) が無い (NFR-503 の無害化)
//! - 主カーソルの位置が文書の長さ以下で、char の境界にある
//! - 文書の Revision が単調に増える (同じ文書で減らない)
//! - 文書・ペイン・View の関係 (docs/03 §9.9 の PANE-1。`EditorState::check_panes`)
//! - 不正バイトを含む文書の編集 (docs/03 §8.3.1、v0.4 の M23) の不変条件:
//!   (a) `escape_encoding` は `None` か `Some(encoding)` (`Some` なら `pua_escape == Enabled` で状態付きでない)、
//!   (b) 解除の記録があるバイトの文書に `InvalidBytes` の理由が付いていない、
//!   (c) 外から入れた文字列の `U+10FF00..` が、衝突の文書でない文書に生のまま入らない、
//!   (d) 保存したファイルを同じ文字コードで開き直すと本文と一致する (つながりの確認を承認した保存は対象外)
//! - 先行表示 (docs/03 §9.2.1、v0.4 の M25) の不変条件: 先行表示の文書 (`LoadState::Partial`) は dirty でなく、その本文は
//!   遅い読込で開いたファイルの接頭辞 (BOM の無い UTF-8 で、開いた後にファイルが変わっていなければ)

use std::collections::BTreeMap;

use eve_core::{
    Args, Encoding, FileTime, FsPath, LoadState, Millis, OpenOptions, ReadOnlyReason, SwapBody,
    SwapEntry, SwapOwner,
};
use eve_view::{Answer, DocumentId, Frame, Query};

use crate::Harness;

/// 操作に使うキー (07 §7.2 の設定表記)。
const KEYS: &[&str] = &[
    "left",
    "right",
    "up",
    "down",
    "home",
    "end",
    "pageup",
    "pagedown",
    "ctrl+home",
    "ctrl+end",
    "shift+right",
    "shift+down",
    "shift+end",
    "ctrl+right",
    "ctrl+left",
    "enter",
    "backspace",
    "delete",
    "tab",
    "shift+tab",
    "ctrl+z",
    "ctrl+y",
    "ctrl+a",
    "ctrl+c",
    "ctrl+x",
    "ctrl+v",
    "escape",
    "ctrl+f",
    "ctrl+h",
    "ctrl+p",
    "ctrl+g",
    "ctrl+o",
    "ctrl+s",
    "ctrl+w",
    "alt+z",
    "ctrl+d",
    "ctrl+l",
    "alt+up",
    "alt+down",
    "ctrl+n",
    "ctrl+tab",
    "f1",
];

/// 入力のバイト列を順に読む。尽きたら 0 を返す。
struct Bytes<'a> {
    data: &'a [u8],
    at: usize,
}

impl Bytes<'_> {
    fn u8(&mut self) -> u8 {
        let b = self.data.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        b
    }

    fn done(&self) -> bool {
        self.at >= self.data.len()
    }

    /// 長さ (先頭 1 バイト、`max` 以下) 付きの文字列。不正な UTF-8 は置き換える。
    fn text(&mut self, max: usize) -> String {
        let n = usize::from(self.u8()) % (max + 1);
        let end = (self.at + n).min(self.data.len());
        let s = String::from_utf8_lossy(&self.data[self.at.min(end)..end]).into_owned();
        self.at = end;
        s
    }
}

fn is_control(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}' | '\u{80}'..='\u{9f}')
}

fn check_frame(f: &Frame, step: usize) {
    // 複数ペインの Frame の不変条件 (03 §9.9 の PANE-10): Window のフォーカスのペイン (`active`) はちょうど 1 つ。
    // 見出しはペインが 2 つ以上のときだけ全ペインにある (縮退表示ではフォーカスのペインだけで見出しが無い)。ペインが 1 つなら
    // 区切りは無い。見出しと区切りの矩形は編集面 (`Frame.size`) の中
    let active = f.panes.iter().filter(|p| p.active).count();
    assert_eq!(active, 1, "step {step}: active のペインが {active} 個");
    let split = f.panes.len() >= 2;
    for p in &f.panes {
        assert_eq!(
            p.bar.is_some(),
            split,
            "step {step}: ペイン {} 個で {:?} の見出しが {:?}",
            f.panes.len(),
            p.view,
            p.bar.as_ref().map(|b| b.rect)
        );
    }
    if !split {
        assert_eq!(
            f.dividers,
            Vec::new(),
            "step {step}: ペインが 1 つなのに区切りがある"
        );
    }
    // タブライン (FR-510、03 §9.9 の PANE-12): タブは 1 つ以上で、アクティブはその中。タブの文書は重ならない
    if let Some(t) = &f.tab_line {
        assert!(!t.tabs.is_empty(), "step {step}: タブの無いタブライン");
        if let Some(a) = t.active {
            assert!(a < t.tabs.len(), "step {step}: アクティブ {a} がタブの外");
        }
        let mut docs: Vec<DocumentId> = t.tabs.iter().map(|x| x.doc).collect();
        docs.dedup();
        assert_eq!(docs.len(), t.tabs.len(), "step {step}: タブの文書が重なる");
        assert!(
            t.tabs.windows(2).all(|w| w[0].doc < w[1].doc),
            "step {step}: タブが開いた順でない"
        );
    }
    let inside = |r: &eve_view::CellRect| {
        u32::from(r.col) + u32::from(r.cols) <= u32::from(f.size.cols)
            && u32::from(r.row) + u32::from(r.rows) <= u32::from(f.size.rows)
    };
    for r in f
        .panes
        .iter()
        .filter_map(|p| p.bar.as_ref().map(|b| b.rect))
        .chain(f.dividers.iter().map(|d| d.rect))
    {
        assert!(
            inside(&r),
            "step {step}: 見出し・区切りの {r:?} が編集面 {:?} の外に出る",
            f.size
        );
    }
    for p in &f.panes {
        for (r, row) in p.rows.iter().enumerate() {
            let w: u32 = row
                .gutter
                .iter()
                .chain(&row.spans)
                .map(|s| u32::from(s.width))
                .sum();
            assert!(
                w <= u32::from(p.rect.cols),
                "step {step}: 行 {r} の幅 {w} が Pane の幅 {} を超える",
                p.rect.cols
            );
            for s in row.gutter.iter().chain(&row.spans) {
                assert!(
                    !s.text.chars().any(is_control),
                    "step {step}: 行 {r} の Span に制御文字がある: {:?}",
                    s.text
                );
            }
        }
    }
}

/// 各ペインの先頭の行は View の先頭の論理行で、折り返しの表示行の番号 (`wrap_idx`) はその行の表示行の数より小さい
/// (03 §14.3.1。折り返し幅が変わった後に戻し忘れると、先頭の論理行を飛ばして描く)。
fn check_wrap_top(h: &Harness, f: &Frame, step: usize) {
    use eve_text::TextRead;
    let st = h.editor.state();
    for p in &f.panes {
        let Some(eve_view::RowSource::Line { line, wrap_idx }) = p.rows.first().map(|r| r.source)
        else {
            continue;
        };
        let Some(v) = st.view(p.view) else {
            continue;
        };
        let Some(d) = st.document(v.doc) else {
            continue;
        };
        let top = d.text().byte_to_line(v.scroll.top).0;
        assert_eq!(
            usize::try_from(line).unwrap_or(usize::MAX),
            top,
            "step {step}: {:?} の先頭の行が先頭の論理行でない (wrap_idx {} が表示行の数を超えた)",
            p.view,
            v.scroll.wrap_idx
        );
        if wrap_idx > 0 {
            assert_eq!(
                wrap_idx, v.scroll.wrap_idx,
                "step {step}: {:?} の先頭の表示行の番号",
                p.view
            );
        }
    }
}

/// タブラインの不変条件 (03 §9.9 の PANE-12): タブラインがあれば、アクティブは `Some` でフォーカスのペインの文書と一致し、
/// 編集面は 3 行以上残る (PANE-12 (b))。
fn check_tab_line(h: &Harness, f: &Frame, step: usize) {
    let Some(t) = &f.tab_line else {
        return;
    };
    let Some(a) = t.active else {
        panic!("step {step}: タブラインのアクティブが無い");
    };
    let focus = f.panes.iter().find(|p| p.active).map(|p| p.view);
    let doc = focus.and_then(|v| h.editor.state().view(v)).map(|v| v.doc);
    assert_eq!(
        t.tabs.get(a).map(|x| x.doc),
        doc,
        "step {step}: アクティブなタブがフォーカスのペインの文書でない"
    );
    assert!(
        f.size.rows >= 3,
        "step {step}: タブラインを出して編集面が {} 行",
        f.size.rows
    );
}

fn check_state(h: &Harness, revs: &mut BTreeMap<DocumentId, u64>, step: usize) {
    let st = h.editor.state();
    if let Ok(Answer::Documents(docs)) = h.query(&Query::Documents) {
        for d in docs {
            let r = d.revision.0;
            let prev = revs.insert(d.doc, r).unwrap_or(0);
            assert!(
                r >= prev,
                "step {step}: {:?} の Revision が {prev} → {r} に減った",
                d.doc
            );
        }
    }
    // 文書・ペイン・View の関係 (docs/03 §9.9 の PANE-1)
    if let Err(e) = st.check_panes() {
        panic!("step {step}: ペインの不変条件に反する: {e}");
    }
    // 全 View (ペインの表示中の View・表に残っている View・ペインに属さない View) の全 Range の anchor / head が、文書の
    // 範囲内で char の境界にあり、選択が正規化済み (09 §3.2 の「eve-core 複数ペイン」の (e))。ペインが表示中の View は
    // 必ずある
    if let Ok(Answer::Windows(ws)) = h.query(&Query::Windows) {
        for v in ws.iter().flat_map(|w| w.views.iter()) {
            assert!(
                st.view(*v).is_some_and(|x| st.document(x.doc).is_some()),
                "step {step}: ペインの View {v:?} か、その文書が無い"
            );
        }
    }
    if let Ok(Answer::Documents(docs)) = h.query(&Query::Documents) {
        use eve_text::TextRead as _;
        for info in docs {
            let Some(d) = st.document(info.doc) else {
                continue;
            };
            let t = d.text();
            for v in &info.views {
                let Some(view) = st.view(*v) else {
                    panic!("step {step}: {:?} の View {v:?} が無い", info.doc);
                };
                for r in view.selection.iter() {
                    for p in [r.anchor, r.head] {
                        assert!(
                            p.0 <= t.len_bytes() && t.is_char_boundary(p),
                            "step {step}: {v:?} の選択の端 {p:?} が文書 ({} バイト) の範囲・char の境界の外",
                            t.len_bytes()
                        );
                    }
                }
                assert_eq!(
                    view.selection.clone().normalized(t),
                    view.selection,
                    "step {step}: {v:?} の選択が正規化されていない"
                );
                // INV-S3 (docs/03 §4.5.1 の MCUR-6): Range の数は上限以下で、主は範囲内
                assert!(
                    view.selection.len() <= eve_text::MAX_CURSORS
                        && view.selection.primary_index() < view.selection.len(),
                    "step {step}: {v:?} の Range の数 {} / 主 {}",
                    view.selection.len(),
                    view.selection.primary_index()
                );
            }
        }
    }
    if let Some(w) = st.first_window()
        && let Some(v) = st.window_view(w).and_then(|v| st.view(v))
        && let Some(d) = st.document(v.doc)
    {
        let head = v.selection.primary().head.0;
        let len = d.len_bytes();
        assert!(
            head <= len,
            "step {step}: カーソル {head} が文書の長さ {len} を超える"
        );
        use eve_text::TextRead as _;
        assert!(
            d.text().is_char_boundary(eve_text::ByteOff(head)),
            "step {step}: カーソル {head} が char の境界にない"
        );
        // (a) escape_encoding は None か Some(encoding) (03 §8.3.1。INV-D7)
        assert!(
            d.format.escape_encoding_ok(),
            "step {step}: escape_encoding が不変条件に反する: {:?} (文字コード {:?}、pua_escape {:?})",
            d.format.escape_encoding,
            d.format.encoding,
            d.format.pua_escape
        );
        // (b) 解除の記録があるバイトの文書に InvalidBytes は付かない (解除は Undo・再読込・復旧でも持ち越す)
        assert!(
            !(d.load_state == LoadState::Ready
                && d.invalid_bytes_released
                && d.format.is_bytes_document()
                && d.read_only.contains(ReadOnlyReason::InvalidBytes)),
            "step {step}: 解除の記録があるバイトの文書に InvalidBytes が付いている"
        );
    }
}

/// 先行表示 (docs/03 §9.2.1) の不変条件。`opened` は遅い読込で開いたパスと、開いた時点のファイルの中身。
fn check_partial(h: &Harness, opened: &BTreeMap<String, Vec<u8>>, step: usize) {
    let st = h.editor.state();
    let Ok(Answer::Documents(docs)) = h.query(&Query::Documents) else {
        return;
    };
    for info in docs {
        let Some(d) = st.document(info.doc) else {
            continue;
        };
        if d.load_state != LoadState::Partial {
            continue;
        }
        assert!(!d.is_dirty(), "step {step}: 先行表示の文書が dirty");
        // 先頭は全文の接頭辞 (BOM の無い UTF-8 で読み、開いた後にファイルが書き換わっていないときだけ見る)
        let Some(path) = d.path.as_ref().map(FsPath::display) else {
            continue;
        };
        let (Some(bytes), Some(now)) = (opened.get(&path), h.fs_read(&path)) else {
            continue;
        };
        if *bytes != now || d.format.encoding != Encoding::Utf8 || d.format.has_bom {
            continue;
        }
        let head = d.to_string_lossy();
        if head
            .chars()
            .any(|c| ('\u{10FF00}'..='\u{10FFFF}').contains(&c))
        {
            continue;
        }
        assert!(
            bytes.starts_with(head.as_bytes()),
            "step {step}: 先行表示の先頭 ({} バイト) が {path} の接頭辞でない",
            head.len()
        );
    }
}

/// 遅い読込で開く操作 (docs/03 §9.2.1、v0.4 の M25 の 3)。`/w/slow<n>.txt` (無ければ作る) を、チャンクを小さくして
/// チャンクごとに時計を進める読込で開く。読込を積むだけ・先行表示の先頭で止める (後の結果は取り置く)・先頭で止めてから
/// 最後まで進める・取り置いた結果を渡すだけ、のどれか。遅い読込の設定はその後の読込にも残る。開いたパスと中身を `opened` に記録する。
fn slow_open(h: &mut Harness, b: &mut Bytes<'_>, opened: &mut BTreeMap<String, Vec<u8>>) -> String {
    let sel = b.u8();
    let mode = sel % 4;
    if mode == 3 {
        h.finish_held_jobs();
        return "finish_held_jobs".to_owned();
    }
    let path = format!("/w/slow{}.txt", (sel >> 2) % 3);
    if h.fs_read(&path).is_none() {
        // 行 (改行・CR・日本語・結合文字を含みうる) を繰り返す。チャンクの数が 200 ms をまたぐよう、ある程度の長さにする
        let line = b.text(24);
        let reps = 1 + usize::from(b.u8() % 64);
        h.fs.add_file(&path, format!("{line}\n").repeat(reps).as_bytes());
    }
    let chunk = 4 + usize::from(b.u8() % 125);
    let per_chunk = [30u64, 70, 110, 250][usize::from(sel >> 4) % 4];
    h.fs.set_chunk_size(chunk);
    h.fs.set_read_delay(Some((h.clock.clone(), Millis(per_chunk))));
    if let Some(bytes) = h.fs_read(&path) {
        opened.insert(path.clone(), bytes);
    }
    h.open(&path);
    let what = match mode {
        0 => "積むだけ",
        1 => {
            if h.run_jobs_until_partial() {
                "先頭で止めた"
            } else {
                "先頭が届かずに最後まで"
            }
        }
        _ => {
            let hit = h.run_jobs_until_partial();
            h.finish_held_jobs();
            if hit {
                "先頭の後に最後まで"
            } else {
                "先頭が届かずに最後まで"
            }
        }
    };
    format!("slow_open {path} chunk {chunk} per {per_chunk} ms ({what})")
}

/// 衝突の文書 (`DisabledCollision`。私用文字が通常の文字) か。`PuaEscapeMode` は eve-encoding の型で、eve-testkit の依存に
/// 無いので、Debug の表記で見分ける。
fn is_collision_doc(f: &eve_core::TextFormat) -> bool {
    format!("{:?}", f.pua_escape) == "DisabledCollision"
}

/// 今の文書 (ID と、本文の中の `U+10FF00..U+10FFFF` の数)。衝突の文書は `None`。
fn escape_count(h: &Harness) -> Option<(DocumentId, usize)> {
    let st = h.editor.state();
    let id = st.active_document()?;
    let d = st.document(id)?;
    if is_collision_doc(&d.format) {
        return None;
    }
    let n = d
        .to_string_lossy()
        .chars()
        .filter(|c| ('\u{10FF00}'..='\u{10FFFF}').contains(c))
        .count();
    Some((id, n))
}

/// 外から入る文字列 (`U+10FF00..` を含む。03 §8.3.1 の「本文に入れる規則」の経路 (2)。スワップの本文にも使う)。バイトの値・
/// ASCII・結合しうる文字・日本語・改行・表記の先頭を混ぜる。
fn outside_text(b: &mut Bytes<'_>, high_only: bool) -> String {
    let n = 1 + usize::from(b.u8() % 5);
    let mut s = String::new();
    for _ in 0..n {
        let x = b.u8();
        match x % 6 {
            0..=2 => {
                // `high_only`: 実在のスワップが持つエスケープは不正バイト (0x80 以上) だけ。ASCII のバイトのエスケープが
                // 本文にあると、保存で元のバイトに戻して開き直しても同じ本文にならない (起こり得ない入力を作らない)
                let v = if high_only { x | 0x80 } else { x };
                s.push(char::from_u32(0x10_FF00 + u32::from(v)).unwrap_or('?'));
            }
            3 => s.push(char::from(b'0' + x % 10)),
            4 => s.push_str(["\u{3099}", "あ", "\n", "@", "\u{200d}"][usize::from(x / 6) % 5]),
            _ => s.push_str("<0x"),
        }
    }
    s
}

/// 1 つの操作の前の、保存の検査 (d) のための記録。
struct SaveWatch {
    doc: Option<DocumentId>,
    path: Option<String>,
    bytes: Option<Vec<u8>>,
    /// 操作の前につながりの確認 (`ui.confirm.save.escape_merge`) が出ていた (承認すると、開き直すと別の文字になる
    /// 保存を書く。検査の対象外)。
    merge_dialog: bool,
}

impl SaveWatch {
    fn capture(h: &mut Harness) -> SaveWatch {
        let merge_dialog = h.overlay() == "dialog" && dialog_title(h).contains("開き直すと");
        let st = h.editor.state();
        let doc = st.active_document();
        let path = doc
            .and_then(|d| st.document(d))
            .and_then(|d| d.path.as_ref().map(FsPath::display));
        let bytes = path.as_deref().and_then(|p| h.fs_read(p));
        SaveWatch {
            doc,
            path,
            bytes,
            merge_dialog,
        }
    }
}

/// 出ている確認の見出し (なければ空)。
fn dialog_title(h: &mut Harness) -> String {
    h.frame()
        .overlays
        .iter()
        .find_map(|o| match o {
            eve_view::OverlayModel::Confirm { title, .. } => Some(crate::helpers::cell_text(title)),
            _ => None,
        })
        .unwrap_or_default()
}

/// (d) この操作で今の文書のファイルが書き換わり、文書が未変更なら (保存が完了した)、書かれたバイト列を同じ文字コードで
/// 開き直した本文が、今の本文と一致する。つながりの確認を承認した保存・外部の変更の操作は対象外。
fn check_saved(h: &Harness, pre: &SaveWatch, external: bool, step: usize) {
    if external || pre.merge_dialog || h.pending_jobs() > 0 {
        return;
    }
    let st = h.editor.state();
    let Some(id) = st.active_document().filter(|&d| Some(d) == pre.doc) else {
        return;
    };
    let Some(d) = st.document(id) else { return };
    let Some(path) = d.path.as_ref().map(FsPath::display) else {
        return;
    };
    if pre.path.as_deref() != Some(&path) || d.is_dirty() || d.load_state != LoadState::Ready {
        return;
    }
    let enc = d.format.encoding;
    let text = d.to_string_lossy();
    let Some(now) = h.fs_read(&path) else { return };
    if pre.bytes.as_ref() == Some(&now) {
        return;
    }
    let mut r = Harness::builder().file("/w/reopen.bin", &now).build();
    open_forced(&mut r, "/w/reopen.bin", enc);
    settle_dialogs(&mut r);
    let opened = r
        .editor
        .state()
        .active_document()
        .and_then(|d| r.editor.state().document(d))
        .is_some_and(|d| d.path.is_some() && d.load_state == LoadState::Ready);
    if !opened {
        return;
    }
    assert_eq!(
        r.doc_text(),
        text,
        "step {step}: 保存した {} バイトを {enc:?} で開き直した本文が、保存した本文と違う",
        now.len()
    );
}

/// 文字コードを指定して開く (判定に任せない。FR-111)。ジョブまで進める。
#[allow(
    clippy::disallowed_methods,
    reason = "文字コードを指定して開くコマンドの引数が無いので、testkit は EditorState に直接触れる (STR-B3)"
)]
fn open_forced(h: &mut Harness, path: &str, enc: Encoding) {
    h.editor.state_mut().open_path(
        path,
        OpenOptions {
            forced: Some(enc),
            ..OpenOptions::default()
        },
    );
    h.run_jobs();
}

/// 開くときの確認 (不正バイトの比率など) に「読み取り専用で開く」を答えて閉じる。
fn settle_dialogs(h: &mut Harness) {
    for _ in 0..3 {
        if h.overlay() != "dialog" {
            break;
        }
        h.text("r");
        h.run_jobs();
    }
}

/// 不正なバイトを含むファイルの中身を作る (文字コードを指定して開く初期文書)。ASCII の詰め物 (不正バイトの比率の確認を
/// 避ける) と、その文字コードで不正な並びを置き、続けて `body` (任意のバイト列) を足す。
fn invalid_bytes_file(enc: Encoding, body: &[u8]) -> Vec<u8> {
    const PAD: &str = "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\n\
                       0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\n";
    let mut v: Vec<u8> = Vec::new();
    match enc {
        Encoding::Utf16Le => {
            v.extend(PAD.encode_utf16().flat_map(u16::to_le_bytes));
            // 孤立したサロゲート (不正) と通常の文字
            v.extend_from_slice(b"x\0\x00\xd8y\0\n\0");
        }
        Encoding::ShiftJis => {
            v.extend_from_slice(PAD.as_bytes());
            v.extend_from_slice(b"x\xa0y\x81\n@z\n");
        }
        Encoding::EucJp => {
            v.extend_from_slice(PAD.as_bytes());
            v.extend_from_slice(b"x\x8f\xa1y\xff\n");
        }
        _ => {
            v.extend_from_slice(PAD.as_bytes());
            v.extend_from_slice(b"x\xe3\x81y\xff\n");
        }
    }
    v.extend_from_slice(body);
    v
}

/// 異常終了の後のスワップ (復旧の提案) を偽 FS に積む。本文は `U+10FF00..` を含みうる。
fn add_swap(h: &Harness, text: String, enc: Encoding, pua_escaped: bool) {
    let entry = SwapEntry {
        dir: FsPath::from("/s/swap/aaaa-4242.swp"),
        path: Some(FsPath::from("/w/f.txt")),
        untitled_name: None,
        revision: 7,
        len: text.len() as u64,
        pid: 4242,
        owner: SwapOwner::Stale,
        sensitive: false,
        modified: Some(FileTime {
            secs: 4_000_000_000,
            nanos: 0,
        }),
        readable: true,
        encoding: None,
    };
    let body = SwapBody {
        text,
        encoding: enc,
        has_bom: false,
        default_eol: eve_text::LineEnding::Lf,
        pua_escaped,
    };
    h.fs.add_swap_entry(entry, Some(body));
}

const ENCODINGS: [&str; 6] = [
    "utf-8",
    "shift_jis",
    "euc-jp",
    "iso-2022-jp",
    "utf-16le",
    "utf-16be",
];

/// 引数付きのコマンドを 1 つ選ぶ (07 §11.6 の `args` 列のあるもの)。
fn command_with_args(b: &mut Bytes<'_>) -> (&'static str, Args) {
    match b.u8() % 6 {
        0 => (
            "file.set_line_ending",
            Args::new().with_str("eol", ["lf", "crlf", "cr"][usize::from(b.u8() % 3)]),
        ),
        1 => (
            "edit.insert_text",
            Args::new().with_str("text", &b.text(32)),
        ),
        2 => (
            "file.reopen_with_encoding",
            Args::new().with_str("encoding", ENCODINGS[usize::from(b.u8()) % ENCODINGS.len()]),
        ),
        3 => (
            "file.set_encoding",
            Args::new().with_str("encoding", ENCODINGS[usize::from(b.u8()) % ENCODINGS.len()]),
        ),
        4 => (
            "cursor.goto_line",
            Args::new().with_int("line", i64::from(b.u8()) - 8),
        ),
        _ => (
            "cursor.move_right",
            Args::new().with_int("count", i64::from(b.u8()) * 4),
        ),
    }
}

/// ペインの操作を 1 つ (docs/03 §9.9、09 §3.2 の「eve-core 複数ペイン」)。分割を多めに選び、2 つ以上のペインに届かせる。
fn pane_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    use eve_view::{Modifiers, MouseButton, MouseKind};
    const CMDS: [&str; 12] = [
        "pane.split_right",
        "pane.split_down",
        "pane.close",
        "pane.focus_next",
        "pane.focus_prev",
        "pane.focus_left",
        "pane.focus_right",
        "pane.focus_up",
        "pane.focus_down",
        "pane.grow",
        "pane.shrink",
        "pane.equalize",
    ];
    let x = b.u8();
    match x % 18 {
        i @ 0..=11 => {
            let id = CMDS[usize::from(i)];
            let cells = b.u8();
            if matches!(i, 9 | 10) && cells & 1 == 1 {
                let n = i64::from(cells >> 1);
                h.cmd_with(id, Args::new().with_int("cells", n));
                format!("{id} cells {n}")
            } else {
                let docs = h.editor.state().documents_len();
                h.cmd(id);
                if id == "pane.close" {
                    // ペインを閉じても文書の数は変わらない (09 §3.2 の「eve-core 複数ペイン」の (d))
                    assert_eq!(
                        h.editor.state().documents_len(),
                        docs,
                        "pane.close で文書の数が変わった"
                    );
                }
                id.to_owned()
            }
        }
        12 | 13 => {
            let id = if x % 18 == 12 {
                "pane.split_right"
            } else {
                "pane.split_down"
            };
            h.cmd(id);
            id.to_owned()
        }
        14 | 15 => {
            // 表示中の View の i 番目のペインを押す・ホイール (フォーカスの無いペインのクリック、ポインタの下のホイール)
            let Ok(Answer::Windows(ws)) = h.query(&Query::Windows) else {
                return "pane mouse (Window なし)".to_owned();
            };
            let views = ws.first().map(|w| w.views.clone()).unwrap_or_default();
            if views.is_empty() {
                return "pane mouse (ペインなし)".to_owned();
            }
            let v = views[usize::from(b.u8()) % views.len()];
            let (r, c) = (i32::from(b.u8() % 30), i32::from(b.u8() % 60));
            if x % 18 == 14 {
                h.mouse_in(v, MouseKind::Down(MouseButton::Left), Modifiers::NONE, r, c);
                h.mouse_in(v, MouseKind::Up(MouseButton::Left), Modifiers::NONE, r, c);
                format!("pane click {v:?} {r},{c}")
            } else {
                h.mouse_in(v, MouseKind::WheelDown, Modifiers::NONE, r, c);
                format!("pane wheel {v:?} {r},{c}")
            }
        }
        16 => {
            // 最小の前後 (左右 2 つは 41 列、上下 2 つは 8 行) の大きさ
            let c = 38 + u16::from(b.u8() % 8);
            let r = 5 + u16::from(b.u8() % 6);
            h.resize(c, r);
            format!("resize {c}x{r} (最小の前後)")
        }
        _ => {
            let c = 60 + u16::from(b.u8() % 61);
            let r = 16 + u16::from(b.u8() % 30);
            h.resize(c, r);
            format!("resize {c}x{r}")
        }
    }
}

/// カーソルを足すコマンドを 1 つ以上 (docs/03 §4.5.1、§4.7.1 の fuzz。v0.5 の M32 の段 3)。次の一致を足す・すべて選ぶ・上下に
/// 足すを 1〜8 回実行し (ときどき検索のジョブを走らせる)、続けて `esc`・文字の入力・ジョブの実行のどれかをする。
fn add_cursor_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    const CMDS: [&str; 4] = [
        "select.add_next_match",
        "select.all_matches",
        "select.add_cursor_above",
        "select.add_cursor_below",
    ];
    let n = 1 + usize::from(b.u8() % 8);
    let mut done = Vec::with_capacity(n);
    for _ in 0..n {
        let id = CMDS[usize::from(b.u8()) % CMDS.len()];
        h.cmd(id);
        done.push(id);
        if b.u8() & 3 == 0 {
            h.run_jobs();
        }
    }
    let what = format!("add_cursor {done:?}");
    match b.u8() % 4 {
        0 => {
            h.keys("escape");
            format!("{what} + esc")
        }
        1 => {
            let t = b.text(4);
            h.text(&t);
            format!("{what} + text {t:?}")
        }
        2 => {
            h.run_jobs();
            format!("{what} + run_jobs")
        }
        _ => what,
    }
}

/// 対応括弧の操作 (docs/03 §5.7、§4.7.1 の fuzz。v0.5 の M32 の段 4)。半分は先に括弧を含む文字列を入力し、
/// `cursor.goto_bracket` を 1〜4 回実行して Frame を作る (強調の経路)。移動は文書を変えない (Motion)。
fn bracket_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    const PIECES: [&str; 8] = ["(", ")", "「", "」", "[", "]", "a", "\n"];
    let typed = if b.u8() & 1 == 0 {
        let n = 1 + usize::from(b.u8() % 6);
        let t: String = (0..n)
            .map(|_| PIECES[usize::from(b.u8()) % PIECES.len()])
            .collect();
        h.text(&t);
        Some(t)
    } else {
        None
    };
    let before = h.doc_text();
    let n = 1 + b.u8() % 4;
    for _ in 0..n {
        h.cmd("cursor.goto_bracket");
    }
    assert_eq!(h.doc_text(), before, "cursor.goto_bracket は文書を変えない");
    let _ = h.frame();
    format!("bracket {typed:?} + goto_bracket ×{n}")
}

/// 全文書の (ID, Revision)。
fn revisions(h: &Harness) -> Vec<(DocumentId, u64)> {
    match h.query(&Query::Documents) {
        Ok(Answer::Documents(docs)) => docs.iter().map(|d| (d.doc, d.revision.0)).collect(),
        _ => Vec::new(),
    }
}

/// ジャンプ履歴 (docs/03 §5.9 の JMP-1〜JMP-7、§15.9.1 と docs/09 §3.3 の `fuzz_editor`。v0.5 の M34 の段 8)。行へ移動・
/// 文書の端・文書の切り替えを 0〜3 回して、`cursor.jump_back` / `cursor.jump_forward` を 1〜6 回実行する。戻る・進むは
/// どの文書も変えない (Motion)。履歴の不変条件 (上限・今の位置・閉じた文書・位置が本文の範囲と書記素の境界) は、各操作の
/// 後の `check_panes` が確かめる (編集・閉じる・再読込・分割などのほかの操作と混ざる)。
fn jump_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    let moves = b.u8() % 4;
    for _ in 0..moves {
        match b.u8() % 4 {
            0 => {
                let line = 1 + i64::from(b.u8() % 40);
                h.cmd_with("cursor.goto_line", Args::new().with_int("line", line));
            }
            1 => {
                h.cmd("cursor.doc_end");
            }
            2 => {
                h.cmd("cursor.doc_start");
            }
            _ => {
                h.cmd("buffer.next");
            }
        }
    }
    let before = revisions(h);
    let k = 1 + b.u8() % 6;
    let mut dirs = String::new();
    for _ in 0..k {
        if b.u8() & 1 == 0 {
            h.cmd("cursor.jump_back");
            dirs.push('<');
        } else {
            h.cmd("cursor.jump_forward");
            dirs.push('>');
        }
    }
    assert_eq!(revisions(h), before, "戻る・進むは文書を変えない");
    let _ = h.frame();
    format!("jump moves {moves} {dirs}")
}

/// フォーカスのペインの View に 1〜8 個の Range (重なり・同じ位置・逆向き・文書の外を含む。置くときに正規化される) を置き、
/// 置いた数を返す。View が無ければ `None`。
fn place_ranges(h: &mut Harness, b: &mut Bytes<'_>) -> Option<usize> {
    let st = h.editor.state();
    let view = st.first_window().and_then(|w| st.window_view(w))?;
    let len = st
        .view(view)
        .and_then(|v| st.document(v.doc))
        .map_or(0, eve_core::Document::len_bytes);
    let n = 1 + usize::from(b.u8() % 8);
    // 文書の長さ + 2 までを選ぶ (外の位置は正規化で末尾に収まる)
    let at = |b: &mut Bytes<'_>| {
        eve_text::ByteOff(usize::from(u16::from_le_bytes([b.u8(), b.u8()])) % (len + 3))
    };
    let ranges: Vec<eve_text::Range> = (0..n)
        .map(|_| {
            let anchor = at(b);
            let head = if b.u8() & 1 == 0 { anchor } else { at(b) };
            eve_text::Range::new(anchor, head)
        })
        .collect();
    let primary = usize::from(b.u8()) % n;
    if let Some(sel) = eve_text::Selection::new(ranges, primary) {
        h.set_selection(sel);
    }
    Some(n)
}

/// フォーカスのペインの View の Selection。
fn focused_selection(h: &Harness) -> Option<eve_text::Selection> {
    let st = h.editor.state();
    let view = st.first_window().and_then(|w| st.window_view(w))?;
    st.view(view).map(|v| v.selection.clone())
}

/// 選択の拡張 (docs/03 §5.7 の SEL-1〜SEL-7、§15.9.1 と docs/09 §3.3 の `fuzz_editor`。v0.5 の M34 の段 5)。フォーカスの
/// ペインの View に 1〜8 個の Range を置き、`select.expand` を k 回 (1〜6)、`select.shrink` を k 回実行して、元の Selection に
/// 戻ることを確かめる。半分は続けて `select.line` を 1〜2 回実行する。どれも文書を変えない (Motion)。
fn select_expand_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    let Some(n) = place_ranges(h, b) else {
        return "select_expand (View なし)".to_owned();
    };
    let before = h.doc_text();
    let start = focused_selection(h);
    let k = 1 + b.u8() % 6;
    for _ in 0..k {
        h.cmd("select.expand");
    }
    for _ in 0..k {
        h.cmd("select.shrink");
    }
    assert_eq!(
        focused_selection(h),
        start,
        "select.expand ×{k} と select.shrink ×{k} で元の Selection に戻る"
    );
    let lines = if b.u8() & 1 == 0 { 1 + b.u8() % 2 } else { 0 };
    for _ in 0..lines {
        h.cmd("select.line");
    }
    assert_eq!(h.doc_text(), before, "選択の拡張は文書を変えない");
    let _ = h.frame();
    format!("select_expand ranges {n} ×{k} line ×{lines}")
}

/// ダブル / トリプルクリック (docs/03 §5.8.2 の CLK-1〜CLK-7、§15.9.1 と docs/09 §3.3 の `fuzz_editor`。v0.5 の M34 の段 6)。
/// 半分はフォーカスのペインの View に 1〜8 個の Range を置いてから、面の任意のセル (面の外を含む) を 2〜4 回続けて押し、
/// 半分は最後の押下から単位のドラッグ (面の外へ出て自動スクロールの時間を進めることを含む) をして離す。文書を変えず、押下が
/// 届いたとき (オーバーレイ・検索バーが無い) は Range が 1 つになることを確かめる。
fn multi_click_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    use eve_view::{Modifiers, MouseButton, MouseKind};
    let placed = if b.u8() & 1 == 0 {
        place_ranges(h, b)
    } else {
        None
    };
    let before = h.doc_text();
    let (r, c) = (i32::from(b.u8() % 40) - 4, i32::from(b.u8()));
    let n = 2 + b.u8() % 3;
    // 1 回目は時計を多重クリックの間隔より進めてから押す (`Harness::click`)
    h.click(r, c);
    for _ in 2..n {
        h.mouse(MouseKind::Down(MouseButton::Left), Modifiers::NONE, r, c);
        h.mouse(MouseKind::Up(MouseButton::Left), Modifiers::NONE, r, c);
    }
    h.mouse(MouseKind::Down(MouseButton::Left), Modifiers::NONE, r, c);
    let reached = h.overlay().is_empty() && h.frame().search.is_none();
    let mut what = format!("multi_click ranges {placed:?} {r},{c} ×{n}");
    if b.u8() & 1 == 0 {
        let (r1, c1) = (i32::from(b.u8() % 80) - 20, i32::from(b.u8()));
        let ms = u64::from(b.u8() % 4) * 50;
        h.mouse(MouseKind::Drag(MouseButton::Left), Modifiers::NONE, r1, c1);
        h.advance_ms(ms);
        h.mouse(MouseKind::Up(MouseButton::Left), Modifiers::NONE, r1, c1);
        what.push_str(&format!(" drag {r1},{c1} +{ms}ms"));
    } else {
        h.mouse(MouseKind::Up(MouseButton::Left), Modifiers::NONE, r, c);
    }
    assert_eq!(
        h.doc_text(),
        before,
        "ダブル / トリプルクリックは文書を変えない"
    );
    if reached {
        let len = focused_selection(h).map_or(1, |s| s.len());
        assert_eq!(len, 1, "どの回数の押下も Range を 1 つにする ({what})");
    }
    let _ = h.frame();
    what
}

/// 矩形選択のモードで選択を伸ばすコマンド (docs/03 §4.8 の BOX-2)。
const BOX_EXTENDS: [&str; 12] = [
    "select.extend_left",
    "select.extend_right",
    "select.extend_up",
    "select.extend_down",
    "select.extend_word_left",
    "select.extend_word_right",
    "select.extend_home",
    "select.extend_end",
    "select.extend_page_up",
    "select.extend_page_down",
    "select.extend_to_doc_start",
    "select.extend_to_doc_end",
];

/// フォーカスのペインの View の矩形 (docs/03 §4.8 の BOX-3) の検査: 矩形があれば、Range の数が矩形の行の数以下で、各 Range が
/// 1 つの行の中にあり、その行は矩形の行の範囲にあって、同じ行に 2 つの Range が無い。
fn check_box(h: &Harness, what: &str) {
    use eve_text::TextRead;
    let st = h.editor.state();
    let Some(view) = st.first_window().and_then(|w| st.window_view(w)) else {
        return;
    };
    let (Some((rows, _)), Some((top, bottom)), Some(v)) =
        (st.box_size(view), st.box_lines(view), st.view(view))
    else {
        return;
    };
    let Some(d) = st.document(v.doc) else {
        return;
    };
    let t = d.text();
    assert!(
        v.selection.len() <= rows,
        "矩形の Range の数 {} が行の数 {rows} 以下 ({what})",
        v.selection.len()
    );
    let mut prev: Option<usize> = None;
    for r in v.selection.iter() {
        let line = t.byte_to_line(r.start()).0;
        assert_eq!(
            line,
            t.byte_to_line(r.end()).0,
            "矩形の Range {r:?} は 1 つの行の中 ({what})"
        );
        assert!(
            (top..=bottom).contains(&line),
            "矩形の Range {r:?} の行 {line} は矩形の行 {top}〜{bottom} の中 ({what})"
        );
        assert!(
            prev.is_none_or(|p| p < line),
            "矩形の Range は行ごとに 1 つ ({what})"
        );
        prev = Some(line);
    }
}

/// 矩形選択 (docs/03 §4.8 の BOX-1〜BOX-9、§15.9.1 と docs/09 §3.3 の `fuzz_editor`。v0.5 の M34 の段 7)。半分は 1〜8 個の
/// Range を置いてから矩形選択のモードを入れ、選択を伸ばすコマンド (`count` 付きを含む)・面のセル (面の外を含む) のドラッグと
/// 自動スクロール・`Shift` 付きのクリックを 1〜6 回混ぜる。編集の無い間は文書を変えず、矩形の検査 (`check_box`) を満たすことを
/// 確かめる。半分は続けてコピーか切り取りと貼り付け・文字の入力・Undo のどれかをし、最後に `Esc` を 0〜3 回押す。
fn box_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    use eve_view::{Modifiers, MouseButton, MouseKind};
    let placed = if b.u8() & 1 == 0 {
        place_ranges(h, b)
    } else {
        None
    };
    let st = h.editor.state();
    let Some(view) = st.first_window().and_then(|w| st.window_view(w)) else {
        return "box (View なし)".to_owned();
    };
    if !st.box_mode(view) {
        h.cmd("select.toggle_box");
    }
    let before = h.doc_text();
    let mut what = format!("box ranges {placed:?}");
    for _ in 0..1 + b.u8() % 6 {
        match b.u8() % 4 {
            0 | 1 => {
                let id = BOX_EXTENDS[usize::from(b.u8()) % BOX_EXTENDS.len()];
                if b.u8() & 1 == 0 {
                    h.cmd(id);
                    what.push_str(&format!(" {id}"));
                } else {
                    let n = 1 + i64::from(b.u8() % 20);
                    h.cmd_with(id, Args::new().with_int("count", n));
                    what.push_str(&format!(" {id}×{n}"));
                }
            }
            2 => {
                let (r0, c0) = (i32::from(b.u8() % 40) - 4, i32::from(b.u8()));
                let (r1, c1) = (i32::from(b.u8() % 80) - 20, i32::from(b.u8()));
                let ms = u64::from(b.u8() % 4) * 50;
                // 多重クリックに数えないよう、押す前に時計を間隔より進める
                h.advance_ms(eve_core::DOUBLE_CLICK_MS.0 + 1);
                h.mouse(MouseKind::Down(MouseButton::Left), Modifiers::NONE, r0, c0);
                h.mouse(MouseKind::Drag(MouseButton::Left), Modifiers::NONE, r1, c1);
                h.advance_ms(ms);
                h.mouse(MouseKind::Up(MouseButton::Left), Modifiers::NONE, r1, c1);
                what.push_str(&format!(" drag {r0},{c0} → {r1},{c1} +{ms}ms"));
            }
            _ => {
                let (r, c) = (i32::from(b.u8() % 40) - 4, i32::from(b.u8()));
                h.mouse(MouseKind::Down(MouseButton::Left), Modifiers::SHIFT, r, c);
                h.mouse(MouseKind::Up(MouseButton::Left), Modifiers::SHIFT, r, c);
                what.push_str(&format!(" shift+click {r},{c}"));
            }
        }
        check_box(h, &what);
    }
    assert_eq!(h.doc_text(), before, "矩形の選択は文書を変えない ({what})");
    if b.u8() & 1 == 0 {
        match b.u8() % 4 {
            0 | 1 => {
                let cut = b.u8() & 1 == 0;
                h.cmd(if cut {
                    "clipboard.cut"
                } else {
                    "clipboard.copy"
                });
                // 貼り付ける前に矩形を動かす (高さが同じ・違う矩形)
                if b.u8() & 1 == 0 {
                    let id = BOX_EXTENDS[usize::from(b.u8()) % BOX_EXTENDS.len()];
                    h.cmd(id);
                }
                let seen = h.effects().len();
                h.cmd("edit.paste");
                let token = h.effects()[seen..].iter().rev().find_map(|e| match e {
                    eve_view::Effect::RequestClipboard { token, .. } => Some(*token),
                    _ => None,
                });
                let t = (b.u8() & 1 == 0).then(|| b.text(24));
                if let Some(token) = token {
                    h.clipboard_content(token, t.as_deref());
                }
                what.push_str(&format!(" cut {cut} + edit.paste {t:?}"));
            }
            2 => {
                let t = b.text(4);
                h.text(&t);
                what.push_str(&format!(" text {t:?}"));
            }
            _ => {
                h.cmd("edit.undo");
                what.push_str(" undo");
            }
        }
        check_box(h, &what);
    }
    let esc = b.u8() % 4;
    for _ in 0..esc {
        h.keys("escape");
    }
    let _ = h.frame();
    format!("{what} esc ×{esc}")
}

/// 行コメントの切替 (docs/03 §15.6、docs/09 §3.3 の `fuzz_editor`。v0.5 の M33 の段 3)。半分は先に文書の言語を記号のある言語
/// (`#`・`//`・`;`) か `plain` に替え、フォーカスのペインの View に 1〜8 個の Range (重なり・同じ位置・逆向き・文書の外を
/// 含む) を置いて `edit.toggle_comment` を 1〜2 回実行し、ときどき Undo する。
fn toggle_comment_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    const LANGS: [&str; 4] = ["python", "rust", "ini", "plain"];
    let lang = (b.u8() & 1 == 0).then(|| LANGS[usize::from(b.u8()) % LANGS.len()]);
    if let Some(l) = lang {
        h.cmd_with("lang.set", Args::new().with_str("language", l));
    }
    let Some(n) = place_ranges(h, b) else {
        return "toggle_comment (View なし)".to_owned();
    };
    let times = 1 + b.u8() % 2;
    for _ in 0..times {
        h.cmd("edit.toggle_comment");
    }
    let undo = b.u8().is_multiple_of(4);
    if undo {
        h.cmd("edit.undo");
    }
    let _ = h.frame();
    format!("toggle_comment lang {lang:?} ranges {n} ×{times} undo {undo}")
}

/// テキスト変換のコマンド (docs/03 §15.8、docs/09 §3.3 の `fuzz_editor`。v0.5 の M34 の段 2)。フォーカスのペインの View に
/// 1〜8 個の Range (重なり・同じ位置・逆向き・文書の外を含む) を置いて 5 つのうちの 1 つを実行し、半分は続けて文字を
/// 入れ、ジョブを 1 周だけ走らせ、ときどき Undo する。fuzz の文書は 1 MiB に届かず、この crate は eve-core の `test-hooks`
/// (上限を小さくする口) を使わない (workspace の build で配布物と同じ eve-core に feature が混ざるため) ので、ジョブの経路は
/// 通らない (ジョブの経路は eve-core の `tests/text_transform.rs` で確かめる)。
fn text_transform_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    const IDS: [&str; 5] = [
        "edit.transform_upper",
        "edit.transform_lower",
        "edit.sort_lines_ascending",
        "edit.sort_lines_descending",
        "edit.trim_trailing_whitespace",
    ];
    let st = h.editor.state();
    let Some(view) = st.first_window().and_then(|w| st.window_view(w)) else {
        return "text_transform (View なし)".to_owned();
    };
    let len = st
        .view(view)
        .and_then(|v| st.document(v.doc))
        .map_or(0, eve_core::Document::len_bytes);
    let n = 1 + usize::from(b.u8() % 8);
    // 文書の長さ + 2 までを選ぶ (外の位置は正規化で末尾に収まる)
    let at = |b: &mut Bytes<'_>| {
        eve_text::ByteOff(usize::from(u16::from_le_bytes([b.u8(), b.u8()])) % (len + 3))
    };
    let ranges: Vec<eve_text::Range> = (0..n)
        .map(|_| {
            let anchor = at(b);
            let head = if b.u8() & 1 == 0 { anchor } else { at(b) };
            eve_text::Range::new(anchor, head)
        })
        .collect();
    let primary = usize::from(b.u8()) % n;
    if let Some(sel) = eve_text::Selection::new(ranges, primary) {
        h.set_selection(sel);
    }
    let id = IDS[usize::from(b.u8()) % IDS.len()];
    h.cmd(id);
    let typed = b.u8() & 1 == 0;
    if typed {
        h.text("x ");
    }
    h.run_jobs_once();
    let undo = b.u8().is_multiple_of(4);
    if undo {
        h.cmd("edit.undo");
    }
    let _ = h.frame();
    format!("text_transform {id} ranges {n} typed {typed} undo {undo}")
}

/// 保存時の変換の設定を切り替えて保存する (docs/03 §9.3.1、§15.9.1 の fuzz。v0.5 の M34 の段 3)。2 つの設定
/// (`editor.trim_trailing_whitespace_on_save`・`editor.insert_final_newline_on_save`) を引数で選んで設定に入れ、
/// `file.save` してジョブを走らせ、ときどき Undo する。保存の検査 (d) は、変換を当てた本文 (バッファ) と書いたバイト列が
/// 一致することを確かめる (変換はバッファにも当たる、§8.5)。
#[allow(
    clippy::disallowed_methods,
    reason = "設定を差し替えるコマンドが無いので、testkit は EditorState に直接触れる (STR-B3)"
)]
fn save_transform_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    let x = b.u8();
    let (trim, final_newline) = (x & 1 != 0, x & 2 != 0);
    let config = eve_config::Config {
        trim_trailing_whitespace_on_save: trim,
        insert_final_newline_on_save: final_newline,
        ..h.editor.state().config().clone()
    };
    h.editor.state_mut().set_config(config);
    h.cmd("file.save");
    h.run_jobs();
    let undo = x & 4 != 0;
    if undo {
        h.cmd("edit.undo");
    }
    let _ = h.frame();
    format!("save_transform trim {trim} final_newline {final_newline} undo {undo}")
}

/// 自動閉じを入れて文字を打つ (docs/03 §15.7 の APR-1〜APR-8、§15.9.1 の fuzz。v0.5 の M34 の段 4)。`editor.auto_pairs` を
/// 真にし、半分は 1〜8 個の Range を置いてから、組の文字・英数字・空白・全角の文字を混ぜた 1〜4 書記素の `Text` を 1〜6 回
/// 打ち、ときどき `edit.delete_backward` を挟む。打つ前に時計を結合の時間窓より進め、増えた Undo のノードの数だけ Undo すると
/// 打つ前の本文に戻ることを確かめる (自動閉じの入力が、ノードを作らずに本文を変えない。APR-7)。
#[allow(
    clippy::disallowed_methods,
    reason = "設定を差し替えるコマンドが無いので、testkit は EditorState に直接触れる (STR-B3)"
)]
fn auto_pairs_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    const KEYS: [&str; 14] = [
        "(", ")", "[", "]", "{", "}", "\"", "`", "a", "1", " ", "日", "（", "e\u{301}",
    ];
    if !h.editor.state().config().auto_pairs {
        let config = eve_config::Config {
            auto_pairs: true,
            ..h.editor.state().config().clone()
        };
        h.editor.state_mut().set_config(config);
    }
    let ranges = if b.u8() & 1 == 0 {
        place_ranges(h, b).unwrap_or(0)
    } else {
        0
    };
    let nodes = |h: &Harness| {
        let st = h.editor.state();
        st.active_document()
            .and_then(|d| st.document(d))
            .map_or(0, |d| d.history().undo_len())
    };
    h.advance_ms(5_000);
    // 文字がオーバーレイ・検索バーの入力欄 (確認のアクセラレータを含む) へ行くときは、本文の検査をしない
    let in_editor = |h: &mut Harness| h.overlay().is_empty() && h.frame().search.is_none();
    let checked = in_editor(h);
    let before = h.doc_text();
    let base = nodes(h);
    let mut typed = Vec::new();
    for _ in 0..1 + b.u8() % 6 {
        if b.u8().is_multiple_of(5) {
            h.keys("backspace");
            typed.push("⌫".to_owned());
            continue;
        }
        let s: String = (0..1 + b.u8() % 4)
            .map(|_| KEYS[usize::from(b.u8()) % KEYS.len()])
            .collect();
        h.text(&s);
        typed.push(s);
    }
    let added = nodes(h).saturating_sub(base);
    let undo = b.u8() & 1 == 0;
    if undo && checked && in_editor(h) && nodes(h) >= base {
        for _ in 0..added {
            h.cmd("edit.undo");
        }
        assert_eq!(
            h.doc_text(),
            before,
            "増えた Undo のノード ({added}) だけ戻すと、打つ前の本文に戻る: {typed:?}"
        );
    }
    let _ = h.frame();
    format!("auto_pairs ranges {ranges} typed {typed:?} nodes +{added} undo {undo}")
}

/// 複数カーソルの操作を 1 つ (docs/03 §4.5、§4.7.1 の fuzz。v0.5 の M32 の段 2)。フォーカスのペインの View に 1〜8 個の
/// Range (重なり・同じ位置・逆向き・文書の外を含む。置くときに正規化される) を置き、半分は続けて既存の編集・クリップボード・
/// 行のコマンドを 1 つ実行する (カーソルを足すコマンドは `cmd` の一覧の外の操作 26 で選ぶ)。
fn multi_cursor_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    const EDITS: &[&str] = &[
        "edit.insert_text",
        "edit.newline",
        "edit.delete_backward",
        "edit.delete_forward",
        "edit.delete_word_backward",
        "edit.indent",
        "edit.outdent",
        "edit.duplicate_line",
        "edit.delete_line",
        "edit.move_line_up",
        "edit.move_line_down",
        "clipboard.copy",
        "clipboard.cut",
        "edit.undo",
        "edit.redo",
        "paste",
        "edit.paste",
        "edit.delete_word_forward",
        "file.reload",
        "search.replace_all",
    ];
    let st = h.editor.state();
    let Some(view) = st.first_window().and_then(|w| st.window_view(w)) else {
        return "multi_cursor (View なし)".to_owned();
    };
    let len = st
        .view(view)
        .and_then(|v| st.document(v.doc))
        .map_or(0, eve_core::Document::len_bytes);
    let n = 1 + usize::from(b.u8() % 8);
    // 文書の長さ + 2 までを選ぶ (外の位置は正規化で末尾に収まる)
    let at = |b: &mut Bytes<'_>| {
        eve_text::ByteOff(usize::from(u16::from_le_bytes([b.u8(), b.u8()])) % (len + 3))
    };
    let ranges: Vec<eve_text::Range> = (0..n)
        .map(|_| {
            let anchor = at(b);
            let head = if b.u8() & 1 == 0 { anchor } else { at(b) };
            eve_text::Range::new(anchor, head)
        })
        .collect();
    let primary = usize::from(b.u8()) % n;
    let Some(sel) = eve_text::Selection::new(ranges, primary) else {
        return "multi_cursor (Selection を作れない)".to_owned();
    };
    let what = format!("multi_cursor {:?}", sel.ranges());
    h.set_selection(sel);
    if b.u8() & 1 == 0 {
        return what;
    }
    match EDITS[usize::from(b.u8()) % EDITS.len()] {
        "edit.insert_text" => {
            let t = b.text(8);
            h.cmd_with("edit.insert_text", Args::new().with_str("text", &t));
            format!("{what} + insert_text {t:?}")
        }
        "paste" => {
            let t = b.text(24);
            h.paste(&t);
            format!("{what} + paste {t:?}")
        }
        "edit.paste" => {
            // システムのクリップボードの応答 (半分は読み取れなかった扱いで内部レジスタを使う)
            let seen = h.effects().len();
            h.cmd("edit.paste");
            let token = h.effects()[seen..].iter().rev().find_map(|e| match e {
                eve_view::Effect::RequestClipboard { token, .. } => Some(*token),
                _ => None,
            });
            let t = (b.u8() & 1 == 0).then(|| b.text(24));
            if let Some(token) = token {
                h.clipboard_content(token, t.as_deref());
            }
            format!("{what} + edit.paste {t:?}")
        }
        "file.reload" => {
            h.cmd("file.reload");
            h.run_jobs();
            format!("{what} + file.reload")
        }
        "search.replace_all" => {
            let (find, with) = (b.text(4), b.text(4));
            h.keys("ctrl+r");
            h.text(&find);
            h.keys("tab");
            h.text(&with);
            h.keys("alt+a");
            h.run_jobs();
            h.keys("escape");
            format!("{what} + search.replace_all {find:?} → {with:?}")
        }
        id => {
            h.cmd(id);
            format!("{what} + {id}")
        }
    }
}

/// タブラインの操作を 1 つ (docs/03 §9.9 の PANE-12、03 §4.7.1 の fuzz)。`ui.tab_line` の 3 値の切り替え、開いている・
/// 閉じた文書のタブへのマウス (左の押下・離す・ホイール・右)、文書を足して 2 つ以上にする。
fn tab_op(h: &mut Harness, b: &mut Bytes<'_>) -> String {
    use eve_config::TabLineMode;
    use eve_view::{Modifiers, MouseButton, MouseKind};
    let x = b.u8();
    match x % 6 {
        0 => {
            let mode = [TabLineMode::Auto, TabLineMode::Always, TabLineMode::Never]
                [usize::from(b.u8() % 3)];
            h.set_tab_line(mode);
            format!("ui.tab_line {mode:?}")
        }
        1 => {
            h.cmd("file.new");
            "file.new".to_owned()
        }
        n => {
            // 開いている文書 (閉じた文書の ID も混ぜる) のタブ
            let docs: Vec<DocumentId> = h
                .frame()
                .tab_line
                .map(|t| t.tabs.iter().map(|x| x.doc).collect())
                .unwrap_or_default();
            let pick = usize::from(b.u8());
            let doc = docs
                .get(pick % (docs.len() + 1))
                .copied()
                .unwrap_or(DocumentId(u64::from(b.u8())));
            let kind = match n {
                2 | 3 => MouseKind::Down(MouseButton::Left),
                4 => MouseKind::Up(MouseButton::Left),
                _ => [MouseKind::WheelDown, MouseKind::Down(MouseButton::Right)]
                    [usize::from(b.u8() % 2)],
            };
            let before = h.editor.state().documents_len();
            h.tab_mouse(doc, kind, Modifiers::NONE);
            // タブのクリックで文書の数は変わらない
            assert_eq!(
                h.editor.state().documents_len(),
                before,
                "タブのマウスで文書の数が変わった"
            );
            format!("tab {kind:?} {doc:?}")
        }
    }
}

/// 1 つの入力を流す。先頭: [0..2] 初期文書の長さ (下位 12 ビット。ビット 12〜14 は初期文書の形、ビット 15 はスワップを積むか。
/// 下)、続いて文書の本文、[次の 2 バイト] ビューポートの大きさ
/// (行の数のバイトの最上位ビットが立っていれば、長大行の閾値を 64 バイトに下げた設定で起動する。長大行とジョブの経路に届かせる)、
/// 残りが操作の列 (操作の種類 1 バイト + 引数)。操作は 24 種類: 文字・キー・貼り付け・大きさ・引数なしのコマンド・ジョブ・時刻・
/// 確認への答え・クリック・引数付きのコマンド・外部の変更・ジョブを 1 周だけ・ジョブの取消し・マウスのドラッグ・再読込と外部の変更の確認
/// (docs/review/v0.3-findings.md の §2.7 の 10)、v0.4 の M23 で足した 6 種類 (`file.toggle_read_only`・その確認への答え
/// (`[A]` / `[E]` / Esc)・再読込 (ジョブまで)・`U+10FF00..` を含む外からの文字列の入力と貼り付け・保存 (ジョブまで)・
/// 隣とつなげる ASCII の入力)、v0.4 の M25 で足した遅い読込で開く操作 (`slow_open`。先行表示の先頭で止める・取り置いた
/// 結果を渡す)、v0.5 の M30 で足したペインの操作 (`pane_op`。分割・閉じる・移動・大きさ・ペインを指すマウス・最小の前後の大きさ)、
/// v0.5 の M32 で足したタブラインの操作 (`tab_op`。`ui.tab_line` の切り替え・タブのマウス・文書を足す)。
///
/// 初期文書の形 (ビット 12〜14 の値): 0 / 5 = 本文を判定に任せて開く、1〜4 (6・7 は 1・2 と同じ) = 不正なバイトを含むファイルを
/// UTF-8 / `Shift_JIS` / EUC-JP / UTF-16LE で指定して開く (`invalid_bytes_file`。03 §8.3.1)。ビット 15 が立っていれば、
/// 同じパスのスワップを積み、復旧の提案を出す (確認の答えで適用できる)。
///
/// # Panics
///
/// 不変条件に違反したとき (fuzz のクラッシュ)。
pub fn editor_sequence(data: &[u8]) {
    let mut b = Bytes { data, at: 0 };
    let head = u16::from_le_bytes([b.u8(), b.u8()]);
    let doc_len = usize::from(head & 0x0FFF);
    let end = (b.at + doc_len).min(data.len());
    let body = data[b.at.min(end)..end].to_vec();
    b.at = end;
    let forced = match (head >> 12) & 0x7 {
        1 | 6 => Some(Encoding::Utf8),
        2 | 7 => Some(Encoding::ShiftJis),
        3 => Some(Encoding::EucJp),
        4 => Some(Encoding::Utf16Le),
        _ => None,
    };
    let cols = 1 + u16::from(b.u8()) % 200;
    let rows_byte = b.u8();
    let rows = 1 + u16::from(rows_byte & 0x7F) % 60;
    let file_bytes = forced.map_or_else(|| body.clone(), |e| invalid_bytes_file(e, &body));
    let mut builder = Harness::builder()
        .size(cols, rows)
        .file("/w/f.txt", &file_bytes);
    if rows_byte & 0x80 != 0 {
        builder = builder.config(eve_config::Config {
            long_line_threshold: 64,
            ..eve_config::Config::default()
        });
    }
    let mut h = builder.build();
    if head & 0x8000 != 0 {
        let text = outside_text(&mut b, true) + &b.text(16);
        let pua_escaped = b.u8() & 1 == 1;
        add_swap(&h, text, forced.unwrap_or(Encoding::Utf8), pua_escaped);
    }
    if let Some(e) = forced {
        open_forced(&mut h, "/w/f.txt", e);
        settle_dialogs(&mut h);
    } else {
        h.open("/w/f.txt");
        h.run_jobs();
    }
    if head & 0x8000 != 0 {
        h.advance_ms(0);
    }
    let commands: Vec<String> = h
        .editor
        .registry()
        .iter()
        .map(|s| s.id.as_str().to_owned())
        // 終了とプロセスの外に出るものは除く (終了は下の Quit の検出で止める)。ペインのコマンドは操作 23 で、カーソルを足す
        // コマンドは操作 26 で、対応括弧への移動は操作 27 で選ぶ (この一覧の番号の読み替えを変えず、既存の種の意味を保つ)。
        // 行コメントの切替 (M33) は操作 28 で、テキスト変換 (M34) は操作 29 で、選択の拡張 (M34) は操作 30 で、保存時の変換 (M34) は操作 31 で、ダブル / トリプルクリック (M34) は操作 32 で、ジャンプ履歴
        // (M34) は操作 33 で、矩形選択 (M34) は操作 34 で、自動閉じの入力 (M34) は操作 35 で選ぶ
        .filter(|id| {
            !id.starts_with("app.quit")
                && !id.starts_with("pane.")
                && !matches!(
                    id.as_str(),
                    "select.add_next_match"
                        | "select.all_matches"
                        | "select.add_cursor_above"
                        | "select.add_cursor_below"
                        | "cursor.goto_bracket"
                        | "edit.toggle_comment"
                        | "edit.transform_upper"
                        | "edit.transform_lower"
                        | "edit.sort_lines_ascending"
                        | "edit.sort_lines_descending"
                        | "edit.trim_trailing_whitespace"
                        | "select.line"
                        | "select.expand"
                        | "select.shrink"
                        | "cursor.jump_back"
                        | "cursor.jump_forward"
                        | "select.toggle_box"
                )
        })
        .collect();
    let mut revs = BTreeMap::new();
    // 遅い読込で開いたパスと、開いた時点の中身 (先行表示の不変条件の検査に使う)
    let mut opened: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut step = 0usize;
    // 再現のとき、操作を 1 つずつ標準エラーに出す (`EVE_FUZZ_TRACE=1`)
    let trace = std::env::var("EVE_FUZZ_TRACE").is_ok_and(|v| v == "1");
    #[allow(clippy::print_stderr)]
    let log = |step: usize, what: &str| {
        if trace {
            eprintln!("step {step}: {what}");
        }
    };
    while !b.done() && step < 200 {
        step += 1;
        let pre = SaveWatch::capture(&mut h);
        let mut external = false;
        // (c) 外から入れた文字列の検査の対象 (入れた文字列と、入れる前の今の文書の ID・エスケープ文字の数)
        let mut outside: Option<(String, Option<(DocumentId, usize)>)> = None;
        // 操作の種類。足すときは剰余を増やし、既存の番号を変えない (コミットした corpus の操作の番号は 24 未満)
        match b.u8() % 36 {
            0 => {
                let t = b.text(16);
                log(step, &format!("text {t:?}"));
                h.text(&t);
            }
            1 | 2 => {
                let k = KEYS[usize::from(b.u8()) % KEYS.len()];
                log(step, &format!("keys {k}"));
                h.keys(k);
            }
            3 => {
                let t = b.text(64);
                log(step, &format!("paste {t:?}"));
                h.paste(&t);
            }
            4 => {
                let c = 1 + u16::from(b.u8()) % 200;
                let r = 1 + u16::from(b.u8()) % 60;
                log(step, &format!("resize {c}x{r}"));
                h.resize(c, r);
            }
            5 => {
                if !commands.is_empty() {
                    let i = usize::from(u16::from_le_bytes([b.u8(), b.u8()])) % commands.len();
                    log(step, &format!("cmd {}", commands[i]));
                    h.cmd(&commands[i]);
                }
            }
            6 => {
                log(step, "run_jobs");
                h.run_jobs();
            }
            7 => {
                let ms = u64::from(b.u8()) * 20;
                log(step, &format!("advance_ms {ms}"));
                h.advance_ms(ms);
            }
            8 => {
                let c = usize::from(b.u8() % 4);
                log(step, &format!("answer {c} (overlay {:?})", h.overlay()));
                let _ = h.answer(c);
            }
            9 => {
                let r = i32::from(b.u8() % 70);
                let c = i32::from(b.u8());
                log(step, &format!("click {r},{c}"));
                h.click(r, c);
            }
            10 => {
                let (id, args) = command_with_args(&mut b);
                log(step, &format!("cmd {id} {args:?}"));
                h.cmd_with(id, args);
            }
            11 => {
                // 外部の変更: 本文を書き換える (更新時刻を進めるか保つか)
                let t = b.text(32);
                let bump = b.u8() & 1 == 1;
                log(step, &format!("external {t:?} bump {bump}"));
                external = true;
                if bump {
                    h.fs.add_file("/w/f.txt", t.as_bytes());
                    h.fs.bump_mtime("/w/f.txt", 1);
                } else {
                    h.fs.set_data_keep_mtime("/w/f.txt", t.as_bytes());
                }
            }
            12 => {
                log(step, "run_jobs_once");
                h.run_jobs_once();
            }
            13 => {
                log(step, "cancel_jobs");
                h.cancel_jobs();
            }
            14 => {
                use eve_view::{Modifiers, MouseButton, MouseKind};
                let (r0, c0) = (i32::from(b.u8() % 70), i32::from(b.u8()));
                let (r1, c1) = (i32::from(b.u8() % 70), i32::from(b.u8()));
                log(step, &format!("drag {r0},{c0} → {r1},{c1}"));
                h.mouse(MouseKind::Down(MouseButton::Left), Modifiers::NONE, r0, c0);
                h.mouse(MouseKind::Drag(MouseButton::Left), Modifiers::NONE, r1, c1);
                h.mouse(MouseKind::Up(MouseButton::Left), Modifiers::NONE, r1, c1);
            }
            16 => {
                // 確認が出たら、半分は続けて [A] 編集を許可を選ぶ (編集できる不正バイトの文書に早く届かせる)
                let allow = b.u8() & 1 == 1;
                log(step, &format!("file.toggle_read_only (allow {allow})"));
                h.cmd("file.toggle_read_only");
                if allow && h.overlay() == "dialog" {
                    let _ = h.answer(0);
                }
            }
            17 => {
                // 確認の選択: [A] 編集を許可 / [E] 文字コードを指定して開き直す / Esc
                let c = b.u8() % 3;
                log(step, &format!("choose {c} (overlay {:?})", h.overlay()));
                match c {
                    0 | 1 => {
                        let _ = h.answer(usize::from(c));
                    }
                    _ => h.keys("escape"),
                }
            }
            18 => {
                log(step, "file.reload + run_jobs");
                h.cmd("file.reload");
                h.run_jobs();
            }
            19 => {
                // 外から入る文字列 (`U+10FF00..` を含む) の入力・貼り付け・edit.insert_text
                let t = outside_text(&mut b, false);
                let via = b.u8() % 3;
                log(step, &format!("outside {t:?} via {via}"));
                outside = Some((t.clone(), escape_count(&h)));
                match via {
                    0 => h.text(&t),
                    1 => h.paste(&t),
                    _ => {
                        h.cmd_with("edit.insert_text", Args::new().with_str("text", &t));
                    }
                }
            }
            20 => {
                log(step, "file.save + run_jobs");
                h.cmd("file.save");
                h.run_jobs();
            }
            21 => {
                // 隣とつなげる ASCII (不正バイトの直後に置くと別の文字になる並び)
                let t = ["@", "A", "a", "0", "\u{80}", "\n"][usize::from(b.u8()) % 6];
                log(step, &format!("join {t:?}"));
                h.text(t);
            }
            22 => {
                // 遅い読込で開く (先行表示。v0.4 の M25)
                let what = slow_open(&mut h, &mut b, &mut opened);
                log(step, &what);
            }
            23 => {
                // ペイン (v0.5 の M30。docs/03 §9.9): 分割・閉じる・移動・大きさ・揃える、ペインを指すマウス、最小の前後の大きさ
                let what = pane_op(&mut h, &mut b);
                log(step, &what);
            }
            24 => {
                // タブライン (v0.5 の M32。docs/03 §9.9 の PANE-12): `ui.tab_line` の切り替え・タブのマウス・新しい文書
                let what = tab_op(&mut h, &mut b);
                log(step, &what);
            }
            25 => {
                // 複数カーソル (v0.5 の M32 の段 2。docs/03 §4.5): 複数の Range を置き、既存の編集を 1 つ
                let what = multi_cursor_op(&mut h, &mut b);
                log(step, &what);
            }
            26 => {
                // カーソルを足すコマンド (v0.5 の M32 の段 3。docs/03 §4.5.1)
                let what = add_cursor_op(&mut h, &mut b);
                log(step, &what);
            }
            27 => {
                // 対応括弧への移動と強調 (v0.5 の M32 の段 4。docs/03 §5.7)
                let what = bracket_op(&mut h, &mut b);
                log(step, &what);
            }
            28 => {
                // 行コメントの切替 (v0.5 の M33 の段 3。docs/03 §15.6)
                let what = toggle_comment_op(&mut h, &mut b);
                log(step, &what);
            }
            29 => {
                // テキスト変換 (v0.5 の M34 の段 2。docs/03 §15.8)
                let what = text_transform_op(&mut h, &mut b);
                log(step, &what);
            }
            30 => {
                // 選択の拡張 (v0.5 の M34 の段 5。docs/03 §5.7)
                let what = select_expand_op(&mut h, &mut b);
                log(step, &what);
            }
            31 => {
                // 保存時の変換の設定を切り替えて保存 (v0.5 の M34 の段 3。docs/03 §9.3.1)
                let what = save_transform_op(&mut h, &mut b);
                log(step, &what);
            }
            32 => {
                // ダブル / トリプルクリック (v0.5 の M34 の段 6。docs/03 §5.8.2)
                let what = multi_click_op(&mut h, &mut b);
                log(step, &what);
            }
            33 => {
                // ジャンプ履歴 (v0.5 の M34 の段 8。docs/03 §5.9)
                let what = jump_op(&mut h, &mut b);
                log(step, &what);
            }
            34 => {
                // 矩形選択 (v0.5 の M34 の段 7。docs/03 §4.8)
                let what = box_op(&mut h, &mut b);
                log(step, &what);
            }
            35 => {
                // 自動閉じを入れて文字を打つ (v0.5 の M34 の段 4。docs/03 §15.7)
                let what = auto_pairs_op(&mut h, &mut b);
                log(step, &what);
            }
            _ => {
                let id = if b.u8() & 1 == 0 {
                    "file.reload"
                } else {
                    "file.check_external_changes"
                };
                log(step, id);
                h.cmd(id);
                if b.u8() & 1 == 0 {
                    h.focus(true);
                }
            }
        }
        if trace {
            let st = h.editor.state();
            let w = st.first_window();
            let v = w.and_then(|w| st.window_view(w));
            let d = v.and_then(|v| st.view(v)).map(|v| v.doc);
            log(
                step,
                &format!(
                    "  window {w:?} view {v:?} doc {d:?} (文書 {} 件、doc あり {})",
                    st.documents_len(),
                    d.is_some_and(|d| st.document(d).is_some())
                ),
            );
        }
        let f = h.frame();
        check_frame(&f, step);
        check_tab_line(&h, &f, step);
        check_wrap_top(&h, &f, step);
        check_state(&h, &mut revs, step);
        check_partial(&h, &opened, step);
        if let Some((t, Some((doc, before)))) = outside
            && let Some((now_doc, after)) = escape_count(&h)
            && now_doc == doc
        {
            // (c) 外から入れた文字列の `U+10FF00..` は、衝突の文書でなければ生のまま入らない (`<U+10FFNN>` になる)。
            // 置換で消える分があるので、増えないことだけを見る。
            assert!(
                after <= before,
                "step {step}: 外から入れた {t:?} の `U+10FF00..` が本文に生のまま入った ({before} → {after})"
            );
        }
        check_saved(&h, &pre, external, step);
        if h.quit_code().is_some() {
            break;
        }
    }
}
