//! 組込みのコマンドと既定のキーマップの `when` に、知らない識別子が無いこと (docs/review/v0.3-findings.md の STR-B4 (4))。
//! `when` の評価は、知らない識別子を黙って偽にする (`EditorState::context_value` が `None` を返す)。書き間違いは
//! 実行時には分からないので、ここで 2 つの一覧と突き合わせる:
//!
//! - core が値を返す識別子 (`context_value` が `Some`)。
//! - 07 §7.3 のコンテキスト変数の表 (仕様の固定集合)。

use std::collections::BTreeMap;

use eve_docs_tables::{identifiers, read_tables};
use eve_testkit::Harness;

/// 07 §7.3 の表の変数名。`caps.<field>` のような `<…>` を含む名前は、その前までを接頭辞として返す。
fn spec_vars() -> (Vec<String>, Vec<String>) {
    let doc = include_str!("../../../docs/07-config-extensibility.md");
    let tables = read_tables(
        doc,
        "### 7.3 `when` 句 ([02 §6.4](02-architecture.md#64-when-コンテキスト))",
        &["変数", "型", "意味"],
    )
    .expect("07 §7.3 の表を読める");
    let (mut names, mut prefixes) = (Vec::new(), Vec::new());
    for t in &tables {
        for r in &t.rows {
            for id in identifiers(t.cell(r, "変数").unwrap_or_default()) {
                match id.find('<') {
                    Some(i) => prefixes.push(id[..i].to_owned()),
                    None => names.push(id),
                }
            }
        }
    }
    assert!(!names.is_empty(), "07 §7.3 の表に変数が無い");
    (names, prefixes)
}

/// 組込みの `when` に現れる識別子 → 使っている所 (最初の 1 つ)。
fn used_vars(h: &Harness) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for spec in h.editor.registry().iter() {
        for v in spec.when.vars() {
            out.entry(v)
                .or_insert_with(|| format!("コマンド {}", spec.id.as_str()));
        }
    }
    for b in h.editor.keymap().all_bindings() {
        for v in b.when.vars() {
            out.entry(v).or_insert_with(|| {
                format!(
                    "既定のキーマップ {} の {:?} → {}",
                    b.context,
                    b.sequence,
                    b.command.as_str()
                )
            });
        }
    }
    out
}

#[test]
fn 組込みの_when_の識別子はどれも_core_が値を返し仕様の表にある() {
    let h = Harness::new();
    let used = used_vars(&h);
    assert!(!used.is_empty(), "組込みの when に識別子が 1 つも無い");
    let (names, prefixes) = spec_vars();
    let mut errors = Vec::new();
    for (v, at) in &used {
        if h.editor.state().context_value(v).is_none() {
            errors.push(format!("{v} ({at}): core が値を返さない (いつも偽になる)"));
        }
        if !names.contains(v) && !prefixes.iter().any(|p| v.starts_with(p.as_str())) {
            errors.push(format!("{v} ({at}): 07 §7.3 の表に無い"));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
