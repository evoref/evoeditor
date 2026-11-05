//! `fuzz_editor` の本体 (docs/09 §3.3 の L2、NFR-305)。cargo-fuzz のターゲット (`fuzz/fuzz_targets/fuzz_editor.rs`、Linux の夜間) と、
//! Windows の CI で決定的な乱数の入力を回す試験 (`tests/fuzz_editor.rs`) が同じ関数を使う (libFuzzer は MSVC 非対応のため)。
//!
//! 入力のバイト列を「初期文書 + ビューポートの大きさ + 操作の列」に読み替え、ヘッドレスの `Editor` に流す。各操作の後に Frame まで作り、
//! 次を検査する (違反はパニック = fuzz のクラッシュ):
//! - Frame の各行の幅 (ガター + Span) が Pane の幅以下 (`Span` の宣言幅の合計で数える)
//! - Span の文字列に制御文字 (`\x00..\x1f`・`\x7f`・`\u{80}..\u{9f}`) が無い (NFR-503 の無害化)
//! - 主カーソルの位置が文書の長さ以下で、char の境界にある
//! - 文書の Revision が単調に増える (同じ文書で減らない)

use std::collections::BTreeMap;

use eve_core::Args;
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
    }
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

/// 1 つの入力を流す。先頭: [0..2] 初期文書の長さ (下位 12 ビット)、続いて文書の本文、[次の 2 バイト] ビューポートの大きさ
/// (行の数のバイトの最上位ビットが立っていれば、長大行の閾値を 64 バイトに下げた設定で起動する。長大行とジョブの経路に届かせる)、
/// 残りが操作の列 (操作の種類 1 バイト + 引数)。操作は 16 種類: 文字・キー・貼り付け・大きさ・引数なしのコマンド・ジョブ・時刻・
/// 確認への答え・クリック・引数付きのコマンド・外部の変更・ジョブを 1 周だけ・ジョブの取消し・マウスのドラッグ・再読込と外部の変更の確認
/// (docs/review/v0.3-findings.md の §2.7 の 10)。
pub fn editor_sequence(data: &[u8]) {
    let mut b = Bytes { data, at: 0 };
    let doc_len = usize::from(u16::from_le_bytes([b.u8(), b.u8()]) & 0x0FFF);
    let end = (b.at + doc_len).min(data.len());
    let body = data[b.at.min(end)..end].to_vec();
    b.at = end;
    let cols = 1 + u16::from(b.u8()) % 200;
    let rows_byte = b.u8();
    let rows = 1 + u16::from(rows_byte & 0x7F) % 60;
    let mut builder = Harness::builder().size(cols, rows).file("/w/f.txt", &body);
    if rows_byte & 0x80 != 0 {
        builder = builder.config(eve_config::Config {
            long_line_threshold: 64,
            ..eve_config::Config::default()
        });
    }
    let mut h = builder.build();
    h.open("/w/f.txt");
    h.run_jobs();
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
        match b.u8() % 16 {
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
        if h.quit_code().is_some() {
            break;
        }
    }
}
