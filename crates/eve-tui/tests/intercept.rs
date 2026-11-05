//! 横取りの定数表 (eve-tui) が docs/04 §7.3a の表と一致する (表を直したら定数表も同じ PR で直す)。

use std::collections::BTreeSet;

use eve_tui::caps::TerminalKind;
use eve_tui::intercept::{intercepted_keys, table};

/// 04 §7.3a の `terminal` の行の「横取りするキー」列のキー表記 (`eve_docs_tables::intercept` で読む)。
fn from_docs(terminal: &str) -> BTreeSet<String> {
    let doc = include_str!("../../../docs/04-tui-design.md");
    eve_docs_tables::intercept::keys_of(doc, terminal)
        .expect("04 §7.3a を読める")
        .into_iter()
        .collect()
}

#[test]
fn 定数表は_docs_の表と一致し_すべて読める() {
    for (name, kind) in [
        ("Windows Terminal", TerminalKind::WindowsTerminal),
        ("conhost", TerminalKind::Conhost),
    ] {
        let got: BTreeSet<String> = table(kind).iter().map(|k| (*k).to_owned()).collect();
        assert_eq!(got, from_docs(name), "{name}");
        assert_eq!(intercepted_keys(kind).len(), table(kind).len(), "{name}");
    }
}
