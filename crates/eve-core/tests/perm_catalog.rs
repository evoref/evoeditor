//! 組込みコマンドの `CommandSpec` (ID・`permission`・`kind`・`when`) が 07 §11.6 のカタログと一致すること
//! (01 FR-601 AC-1 / AC-2、docs/09 §6.2 の `docs-tables` の (3)、docs/review/v0.3-findings.md の STR-B4 (1))。
//! カタログは `eve_docs_tables::catalog` で読む (読み手は 1 か所、STR-E1)。
//! 「コマンドの実行は権限判定フックを必ず通る」は `Editor::execute` が唯一の入口であることのレビュー項目 (09 §8.4)。

use std::collections::BTreeMap;

use eve_core::{CommandKind, Permission};
use eve_docs_tables::{catalog, identifiers, strip_parentheticals};
use eve_testkit::Harness;

fn perm_of(cell: &str) -> Permission {
    match cell {
        "-" => Permission::None,
        "read" => Permission::Read,
        "propose" => Permission::Propose,
        "ui" => Permission::Ui,
        other => panic!("07 §11.6 の perm 列の値が不明: {other:?}"),
    }
}

fn kind_of(cell: &str) -> CommandKind {
    match cell {
        "Edit" => CommandKind::Edit,
        "Motion" => CommandKind::Motion,
        "Ui" => CommandKind::Ui,
        "File" => CommandKind::File,
        other => panic!("07 §11.6 の kind 列の値が不明: {other:?}"),
    }
}

/// カタログの `CommandId` 1 個分。
struct Entry {
    line: usize,
    perm: String,
    kind: String,
    /// `when` 列 (行のまま)。
    when: String,
    /// 後の版の行。実装した版から登録してよい (07 §11.6)。
    later: bool,
    /// 複数の ID をまとめた行。
    shared: bool,
}

/// カタログの `CommandId` → その行の内容 (`eve_docs_tables::catalog` で読む)。
fn catalog() -> BTreeMap<String, Entry> {
    let doc = include_str!("../../../docs/07-config-extensibility.md");
    let rows = catalog::read(doc).expect("カタログを読める");
    let mut out = BTreeMap::new();
    for r in rows {
        for (i, id) in r.ids.iter().enumerate() {
            out.insert(
                id.clone(),
                Entry {
                    line: r.line,
                    perm: r.perms[i].clone(),
                    kind: r.kinds[i].clone(),
                    when: r.when.clone(),
                    later: r.later,
                    shared: r.ids.len() > 1,
                },
            );
        }
    }
    out
}

/// `when` 列の、全コンテキスト・全 ID に共通の 1 つの式 (`` `!doc.read_only` `` → `!doc.read_only`、`—` → 空)。
/// コンテキストごとの書き分け (`` `editor`: … / `list`: … ``) や、複数 ID の行の括弧の注記 (ID ごとの違い) は
/// `None` (突合しない)。
fn plain_when(cell: &str, shared: bool) -> Option<String> {
    if shared && cell.contains('(') {
        return None;
    }
    let body = strip_parentheticals(cell);
    let body = body.split('。').next().unwrap_or_default().trim();
    if body == "—" {
        return Some(String::new());
    }
    let ids = identifiers(body);
    let only_code = body.starts_with('`') && body.ends_with('`') && ids.len() == 1;
    (only_code && !cell.contains("`:")).then(|| ids[0].clone())
}

// verifies: AC-FR-601-1
#[test]
fn 登録済みのコマンドの_kind_と_when_がカタログと一致する() {
    let cat = catalog();
    let h = Harness::new();
    let mut errors = Vec::new();
    let mut when_checked = 0;
    for spec in h.editor.registry().iter() {
        let id = spec.id.as_str();
        let Some(e) = cat.get(id) else {
            continue; // カタログに無いものは下の試験が報告する
        };
        if spec.kind != kind_of(&e.kind) {
            errors.push(format!(
                "{id} (07 行 {}): kind {:?} / カタログ {}",
                e.line, spec.kind, e.kind
            ));
        }
        if let Some(want) = plain_when(&e.when, e.shared) {
            when_checked += 1;
            if spec.when_src != want {
                errors.push(format!(
                    "{id} (07 行 {}): when `{}` / カタログ `{want}`",
                    e.line, spec.when_src
                ));
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    // 88 個のうち 85 個 (2026-11 の時点)。書き分けのある行・注記の付いた複数 ID の行だけを除く
    assert!(
        when_checked >= 80,
        "when を突合した数が少なすぎる: {when_checked}"
    );
}

// verifies: AC-FR-601-2
#[test]
fn 登録済みの全コマンドの_permission_がカタログの_perm_列と一致する() {
    let cat = catalog();
    assert!(
        cat.values().filter(|e| !e.later).count() >= 60,
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
        assert_eq!(spec.permission, perm_of(&want.perm), "{id} の permission");
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
        .filter(|(_, e)| !e.later)
        .map(|(id, _)| id.clone())
        .collect();
    let only_reg: Vec<_> = reg.difference(&doc).collect();
    let only_doc: Vec<_> = v01.difference(&reg).collect();
    assert!(
        only_reg.is_empty() && only_doc.is_empty(),
        "登録だけ: {only_reg:?} / カタログだけ: {only_doc:?}"
    );
}
