//! 組込みコマンドの `CommandSpec.permission` が 07 §11.6 のカタログの `perm` 列と一致すること
//! (01 FR-601 AC-2、docs/09 §6.2 の `docs-tables` の (3))。docs の表を `eve-docs-tables` で読む。
//! 「コマンドの実行は権限判定フックを必ず通る」は `Editor::execute` が唯一の入口であることのレビュー項目 (09 §8.4)。

use std::collections::BTreeMap;

use eve_core::Permission;
use eve_docs_tables::{identifiers, read_tables, strip_parentheticals};
use eve_testkit::Harness;

const CATALOG_HEADING: &str = "### 11.6 v0.1 CommandId カタログ (付録)";

fn perm_of(cell: &str) -> Permission {
    match cell.trim().trim_matches('`') {
        "-" => Permission::None,
        "read" => Permission::Read,
        "propose" => Permission::Propose,
        "ui" => Permission::Ui,
        other => panic!("07 §11.6 の perm 列の値が不明: {other:?}"),
    }
}

/// カタログの CommandId → (perm, 後の版の行か)。後の版の行は、実装した版から登録してよい (07 §11.6)。
fn catalog() -> BTreeMap<String, (Permission, bool)> {
    let doc = include_str!("../../../docs/07-config-extensibility.md");
    let tables = read_tables(doc, CATALOG_HEADING, &["ID", "perm"]).expect("カタログを読める");
    let mut out = BTreeMap::new();
    for t in &tables {
        for row in &t.rows {
            let id_cell = t.cell(row, "ID").unwrap_or_default();
            let later = id_cell.contains("(v");
            let ids = identifiers(&strip_parentheticals(id_cell));
            let perm_cell = strip_parentheticals(t.cell(row, "perm").unwrap_or_default());
            let perms: Vec<&str> = perm_cell.split('/').collect();
            assert!(
                perms.len() == 1 || perms.len() == ids.len(),
                "行 {}: ID と perm の個数が合いません",
                row.line
            );
            for (i, id) in ids.iter().enumerate() {
                let p = perm_of(if perms.len() == 1 { perms[0] } else { perms[i] });
                out.insert(id.clone(), (p, later));
            }
        }
    }
    out
}

// verifies: AC-FR-601-2
#[test]
fn 登録済みの全コマンドの_permission_がカタログの_perm_列と一致する() {
    let cat = catalog();
    assert!(
        cat.values().filter(|(_, later)| !later).count() >= 60,
        "v0.1 のカタログは 63 個: {}",
        cat.len()
    );
    let h = Harness::new();
    let reg = h.editor.registry();
    let mut checked = 0;
    for spec in reg.iter() {
        let id = spec.id.as_str();
        let want = cat
            .get(id)
            .unwrap_or_else(|| panic!("登録済みの `{id}` が 07 §11.6 のカタログにありません"));
        assert_eq!(spec.permission, want.0, "{id} の permission");
        checked += 1;
    }
    assert!(checked >= 30, "登録済みのコマンドが少なすぎる: {checked}");
}

// verifies: AC-FR-601-2
#[test]
fn 外部からの呼出しは_perm_が_none_のコマンドを拒否する() {
    // 権限判定フックは execute の中にあり、User 以外の主体は perm が `-` のコマンドを実行できない
    let mut h = Harness::new();
    let r = h.editor.execute(eve_core::CommandInvocation::new(
        "app.quit",
        eve_core::InvocationSource::External,
    ));
    assert!(matches!(r, eve_core::CommandResult::Rejected(_)), "{r:?}");
    assert_eq!(h.quit_code(), None);
}

// verifies: AC-FR-601-1
#[test]
fn 登録済みのコマンドの_id_はカタログと一致する_v0_1_の行は全部登録されている() {
    let cat = catalog();
    let h = Harness::new();
    let reg: std::collections::BTreeSet<String> = h
        .editor
        .registry()
        .iter()
        .map(|s| s.id.as_str().to_owned())
        .collect();
    // 登録済みはカタログのどれかの行にある。v0.1 の行は必ず登録されている (後の版の行は実装した版から)
    let doc: std::collections::BTreeSet<String> = cat.keys().cloned().collect();
    let v01: std::collections::BTreeSet<String> = cat
        .iter()
        .filter(|(_, (_, later))| !later)
        .map(|(id, _)| id.clone())
        .collect();
    let only_reg: Vec<_> = reg.difference(&doc).collect();
    let only_doc: Vec<_> = v01.difference(&reg).collect();
    assert!(
        only_reg.is_empty() && only_doc.is_empty(),
        "登録だけ: {only_reg:?} / カタログだけ: {only_doc:?}"
    );
}
