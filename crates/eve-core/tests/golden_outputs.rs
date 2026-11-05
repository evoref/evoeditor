//! 外から見える出力のゴールデン (docs/review/v0.3-findings.md の STR-T1、v0.3 の M13)。
//!
//! v0.3 の整理 (M14〜M16) で振る舞いが変わらないことを確かめるため、登録したコマンドの一覧と既定のキーマップを
//! テキストで固定する。比べ方と書き換え方は `eve_testkit::golden` (`EVE_UPDATE_GOLDEN=1` で作る・書き換える)。

use std::fmt::Write as _;
use std::path::PathBuf;

use eve_core::format_stroke;
use eve_testkit::{Harness, golden};

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

/// 登録したコマンド (ID・種類・権限・`when` の式)。ID の順。
#[test]
fn コマンドの一覧() {
    let h = Harness::new();
    let mut out = String::from("# id\tkind\tpermission\twhen\n");
    for s in h.editor.registry().iter() {
        let when = if s.when_src.is_empty() {
            "—"
        } else {
            &s.when_src
        };
        let _ = writeln!(out, "{}\t{:?}\t{:?}\t{when}", s.id, s.kind, s.permission);
    }
    golden::check(&path("commands.tsv"), &out);
}

/// 既定のキーマップ (コンテキスト・キーの列・コマンド・引数・`when`・拡張キーボードの印)。層の優先順・登録の順。
#[test]
fn 既定のキーマップ() {
    let h = Harness::new();
    let mut out = String::from("# context\tkeys\tcommand\targs\twhen\tenhanced\n");
    for b in h.editor.keymap().all_bindings() {
        let keys: Vec<String> = b.sequence.iter().map(|k| format_stroke(*k)).collect();
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{:?}\t{:?}\t{}",
            b.context,
            keys.join(" "),
            b.command,
            b.args,
            b.when,
            b.enhanced
        );
    }
    golden::check(&path("default_keymap.tsv"), &out);
}
