//! 「編集を許可」の記録 (`invalid_bytes_released`) の不変条件 (docs/03 §8.3.1 の「解除の持ち越し」、§7.2: 解除は Undo / Redo で
//! 戻らない。docs/09 §4 の「不正バイトを含む文書の編集」。v0.4 の M23 の 7 本目)。
//!
//! 不正バイトを含む文書を解除した後、入力・削除・Undo / Redo・保存・別の文字コードでの保存・再読込・カーソルの移動を
//! ランダムに並べ、各操作の後で次を検査する。
//! - 解除の記録があって、バイトの文書 (`pua_escape == Enabled` で `escape_encoding` が `Some`) なら、`InvalidBytes` が無い。
//!   (編集できる。Undo / Redo で書式が戻っても、再読込でも、読み取り専用に戻らない)
//! - 解除の記録が無い (別の文字コードで保存した後に開き直した等) ときは、この検査をしない。

use eve_core::{Args, Document, Encoding, OpenOptions, ReadOnlyReason};
use eve_testkit::Harness;
use proptest::prelude::*;

/// 不正バイトの比率の確認 (FR-127、1% 超) を出さないための後ろの詰め物。
const PAD: &str = "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\n\
                   0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\n";

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// 解除できる文書の種類 (文字コード, ファイル)。
fn files() -> Vec<(Encoding, Vec<u8>)> {
    vec![
        (
            Encoding::Utf8,
            [b"ab\n\x80cd\xFF\n", PAD.as_bytes()].concat(),
        ),
        (
            Encoding::ShiftJis,
            [b"ab\n".as_slice(), b"x\xA0y\x81\n", PAD.as_bytes()].concat(),
        ),
        (
            Encoding::EucJp,
            [b"abc\nde\xFFf\x8F\n", PAD.as_bytes()].concat(),
        ),
        (
            Encoding::Utf16Le,
            [
                utf16le("ab\nc"),
                b"\x00\xD8".to_vec(),
                utf16le(&format!("d\n{PAD}")),
                b"A".to_vec(),
            ]
            .concat(),
        ),
    ]
}

#[derive(Clone, Copy, Debug)]
enum Act {
    Type(&'static str),
    Backspace,
    Delete,
    Undo,
    Redo,
    Save,
    /// 別の文字コードで保存 (不正バイトがあれば表現不能の確認が出て、何も書かない)。
    SaveAsOther,
    Reload,
    Move(&'static str),
}

fn act() -> impl Strategy<Value = Act> {
    prop_oneof![
        5 => proptest::sample::select(vec!["a", "@", "あ", "\n", "\u{FF}"]).prop_map(Act::Type),
        3 => Just(Act::Backspace),
        2 => Just(Act::Delete),
        4 => Just(Act::Undo),
        3 => Just(Act::Redo),
        2 => Just(Act::Save),
        1 => Just(Act::SaveAsOther),
        1 => Just(Act::Reload),
        4 => proptest::sample::select(vec!["left", "right", "up", "down", "home", "end", "ctrl+home", "ctrl+end"])
            .prop_map(Act::Move),
    ]
}

fn doc(h: &Harness) -> &Document {
    let id = h.editor.state().active_document().expect("文書がある");
    h.editor.state().document(id).unwrap()
}

/// 出ている確認・入力欄は Esc で閉じる (保存の前の確認はキャンセルする)。
fn close_overlays(h: &mut Harness) {
    for _ in 0..3 {
        if h.overlay().is_empty() {
            break;
        }
        h.keys("esc");
        h.run_jobs();
    }
}

fn run(h: &mut Harness, a: Act) {
    match a {
        Act::Type(s) => h.text(s),
        Act::Backspace => h.keys("backspace"),
        Act::Delete => h.keys("delete"),
        Act::Undo => {
            h.cmd("edit.undo");
        }
        Act::Redo => {
            h.cmd("edit.redo");
        }
        Act::Save => {
            h.cmd("file.save");
            h.run_jobs();
        }
        Act::SaveAsOther => {
            let target = if doc(h).format.encoding == Encoding::Utf8 {
                "shift_jis"
            } else {
                "utf-8"
            };
            h.cmd_with(
                "file.set_encoding",
                Args::new().with_str("encoding", target),
            );
            h.run_jobs();
        }
        Act::Reload => {
            h.cmd("file.reload");
            h.run_jobs();
        }
        Act::Move(k) => h.keys(k),
    }
    close_overlays(h);
}

/// 解除の記録があってバイトの文書なら、`InvalidBytes` が無い。
fn check(h: &Harness, step: &str) -> Result<(), TestCaseError> {
    let d = doc(h);
    if d.invalid_bytes_released && d.format.is_bytes_document() {
        prop_assert!(
            !d.read_only.contains(ReadOnlyReason::InvalidBytes),
            "{step}: 解除した文書に InvalidBytes がある ({:?})",
            d.read_only.iter().collect::<Vec<_>>()
        );
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    // verifies: NFR-304
    #[test]
    fn 解除した文書は_undo_redo_再読込の後も_バイトの文書なら_invalid_bytes_が無い(
        which in 0usize..4,
        acts in proptest::collection::vec(act(), 0..40),
    ) {
        let (enc, bytes) = files().swap_remove(which);
        let mut h = Harness::builder().defer_start().build();
        h.fs.add_file("/w/a.txt", &bytes);
        h.editor.state_mut().open_path(
            "/w/a.txt",
            OpenOptions { forced: Some(enc), ..OpenOptions::default() },
        );
        h.run_jobs();
        for _ in 0..3 {
            if h.overlay() == "dialog" {
                h.text("r");
                h.run_jobs();
            }
        }
        prop_assert!(doc(&h).read_only.contains(ReadOnlyReason::InvalidBytes));
        // 編集を許可
        h.cmd("file.toggle_read_only");
        prop_assert_eq!(h.overlay(), "dialog");
        h.text("a");
        prop_assert!(doc(&h).invalid_bytes_released && doc(&h).is_editable());
        // 検査が当たる文書 (バイトの文書) から始める
        prop_assert!(doc(&h).format.is_bytes_document());
        for (i, a) in acts.iter().enumerate() {
            run(&mut h, *a);
            check(&h, &format!("{i} 番目の {a:?} の後"))?;
        }
        // 戻せるだけ Undo し、やり直せるだけ Redo しても、同じ
        for _ in 0..acts.len().min(30) {
            run(&mut h, Act::Undo);
            check(&h, "Undo")?;
        }
        for _ in 0..acts.len().min(30) {
            run(&mut h, Act::Redo);
            check(&h, "Redo")?;
        }
    }
}
