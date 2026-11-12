//! `fuzz_editor` の本体 (docs/09 §3.3 の L2、NFR-305)。cargo-fuzz のターゲット (`fuzz/fuzz_targets/fuzz_editor.rs`、Linux の夜間) と、
//! Windows の CI で決定的な乱数の入力を回す試験 (`tests/fuzz_editor.rs`) が同じ関数を使う (libFuzzer は MSVC 非対応のため)。
//!
//! 入力のバイト列を「初期文書 + ビューポートの大きさ + 操作の列」に読み替え、ヘッドレスの `Editor` に流す。各操作の後に Frame まで作り、
//! 次を検査する (違反はパニック = fuzz のクラッシュ):
//! - Frame の各行の幅 (ガター + Span) が Pane の幅以下 (`Span` の宣言幅の合計で数える)
//! - Span の文字列に制御文字 (`\x00..\x1f`・`\x7f`・`\u{80}..\u{9f}`) が無い (NFR-503 の無害化)
//! - 主カーソルの位置が文書の長さ以下で、char の境界にある
//! - 文書の Revision が単調に増える (同じ文書で減らない)
//! - 不正バイトを含む文書の編集 (docs/03 §8.3.1、v0.4 の M23) の不変条件:
//!   (a) `escape_encoding` は `None` か `Some(encoding)` (`Some` なら `pua_escape == Enabled` で状態付きでない)、
//!   (b) 解除の記録があるバイトの文書に `InvalidBytes` の理由が付いていない、
//!   (c) 外から入れた文字列の `U+10FF00..` が、衝突の文書でない文書に生のまま入らない、
//!   (d) 保存したファイルを同じ文字コードで開き直すと本文と一致する (つながりの確認を承認した保存は対象外)

use std::collections::BTreeMap;

use eve_core::{
    Args, Encoding, FileTime, FsPath, LoadState, OpenOptions, ReadOnlyReason, SwapBody, SwapEntry,
    SwapOwner,
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

/// 1 つの入力を流す。先頭: [0..2] 初期文書の長さ (下位 12 ビット。ビット 12〜14 は初期文書の形、ビット 15 はスワップを積むか。
/// 下)、続いて文書の本文、[次の 2 バイト] ビューポートの大きさ
/// (行の数のバイトの最上位ビットが立っていれば、長大行の閾値を 64 バイトに下げた設定で起動する。長大行とジョブの経路に届かせる)、
/// 残りが操作の列 (操作の種類 1 バイト + 引数)。操作は 22 種類: 文字・キー・貼り付け・大きさ・引数なしのコマンド・ジョブ・時刻・
/// 確認への答え・クリック・引数付きのコマンド・外部の変更・ジョブを 1 周だけ・ジョブの取消し・マウスのドラッグ・再読込と外部の変更の確認
/// (docs/review/v0.3-findings.md の §2.7 の 10)、v0.4 の M23 で足した 6 種類 (`file.toggle_read_only`・その確認への答え
/// (`[A]` / `[E]` / Esc)・再読込 (ジョブまで)・`U+10FF00..` を含む外からの文字列の入力と貼り付け・保存 (ジョブまで)・
/// 隣とつなげる ASCII の入力)。
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
        // 終了とプロセスの外に出るものは除く (終了は下の Quit の検出で止める)
        .filter(|id| !id.starts_with("app.quit"))
        .collect();
    let mut revs = BTreeMap::new();
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
        match b.u8() % 22 {
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
        check_state(&h, &mut revs, step);
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
