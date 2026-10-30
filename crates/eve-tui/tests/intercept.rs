//! 横取りの定数表 (eve-tui) が docs/04 §7.3a の表と一致する (表を直したら定数表も同じ PR で直す)。

use std::collections::BTreeSet;

use eve_tui::caps::TerminalKind;
use eve_tui::intercept::{intercepted_keys, table};

/// 04 §7.3a の `terminal` の行の「横取りするキー」列のキー表記。
fn from_docs(terminal: &str) -> BTreeSet<&'static str> {
    let doc: &'static str = include_str!("../../../docs/04-tui-design.md");
    let section = &doc[doc.find("### 7.3a ").expect("7.3a がある")..];
    let row = section
        .lines()
        .find(|l| l.starts_with(&format!("| {terminal} |")))
        .expect("行がある");
    let cell = row.split(" | ").nth(1).expect("横取りするキーの列");
    cell.split('`')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, k)| k)
        .collect()
}

#[test]
fn 定数表は_docs_の表と一致し_すべて読める() {
    for (name, kind) in [
        ("Windows Terminal", TerminalKind::WindowsTerminal),
        ("conhost", TerminalKind::Conhost),
    ] {
        let got: BTreeSet<&str> = table(kind).iter().copied().collect();
        assert_eq!(got, from_docs(name), "{name}");
        assert_eq!(intercepted_keys(kind).len(), table(kind).len(), "{name}");
    }
}
