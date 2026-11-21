//! ペインの見出しの項目と、同じ名前の区別 (docs/03 §9.9 の PANE-10 (b) / PANE-11、04 §3.4 (f))。
//!
//! 見出しの項目はステータスバーの左群と同じ文言・優先度の `StatusItem` (`Path`・`Dirty`・`ReadOnly`) で、名前だけが違う
//! (`(2/3)` の番号を付けず、同じファイル名の文書を親のディレクトリで区別する)。
//!
//! spec: 03 §9.9 (PANE-10, PANE-11)

use std::collections::BTreeMap;
use std::path::{Component, MAIN_SEPARATOR_STR};

use eve_view::{CutPolicy, DocumentId, StatusAlign, StatusItem, StatusKind, ViewId};

use crate::FsPath;
use crate::editor::state::EditorState;
use crate::layout::cell_text;
use crate::ui_model::keys;

/// 開いている全文書の、見出し (とタブライン) に出す名前 (PANE-11)。ファイル名が同じ文書が 2 つ以上あれば `名前 (親)` にし、
/// 無題の文書は今の無題の名前 (`無題-1` など)。
pub(crate) fn display_names(state: &EditorState) -> BTreeMap<DocumentId, String> {
    let paths: Vec<(DocumentId, Option<&FsPath>)> = state
        .docs
        .documents
        .iter()
        .map(|(id, d)| (*id, d.path.as_ref()))
        .collect();
    let mut out = disambiguate(&paths);
    for (id, d) in &state.docs.documents {
        out.entry(*id).or_insert_with(|| state.untitled_label(d));
    }
    out
}

/// パスのある文書の名前 (PANE-11)。ファイル名 (パスの最後の要素) が同じものが 2 つ以上ある組では、各文書の名前を
/// `名前 (親)` にする。`親` は親のディレクトリのパスの末尾から k 個の要素を OS の区切りで繋いだもので、k は組の全部が
/// 区別できる最小の数。要素が足りない文書は親のパスの全体。パスの無い (無題の) 文書は入れない。
pub(crate) fn disambiguate(docs: &[(DocumentId, Option<&FsPath>)]) -> BTreeMap<DocumentId, String> {
    let mut groups: BTreeMap<String, Vec<(DocumentId, &FsPath)>> = BTreeMap::new();
    for (id, path) in docs {
        let Some(path) = path else {
            continue;
        };
        let name = path.file_name().unwrap_or_else(|| path.display());
        groups.entry(name).or_default().push((*id, path));
    }
    let mut out = BTreeMap::new();
    for (name, members) in groups {
        if members.len() < 2 {
            for (id, _) in members {
                out.insert(id, name.clone());
            }
            continue;
        }
        let parents: Vec<(DocumentId, Parent)> =
            members.iter().map(|(id, p)| (*id, Parent::of(p))).collect();
        let longest = parents
            .iter()
            .map(|(_, p)| p.names.len())
            .max()
            .unwrap_or(0);
        // 区別できる最小の k (どの k でも区別できなければ、親のパスの全体)
        let k = (1..=longest.max(1))
            .find(|&k| {
                let labels: Vec<String> = parents.iter().map(|(_, p)| p.label(k)).collect();
                let mut sorted = labels.clone();
                sorted.sort();
                sorted.dedup();
                sorted.len() == labels.len()
            })
            .unwrap_or(usize::MAX);
        for (id, p) in parents {
            out.insert(id, format!("{name} ({})", p.label(k)));
        }
    }
    out
}

/// 親のディレクトリ (要素の名前と、表示用のパスの全体)。
struct Parent {
    names: Vec<String>,
    whole: String,
}

impl Parent {
    fn of(path: &FsPath) -> Parent {
        let parent = path.parent();
        let names = parent
            .as_ref()
            .map(|p| {
                p.as_path()
                    .components()
                    .filter_map(|c| match c {
                        Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Parent {
            names,
            whole: parent.map(|p| p.display()).unwrap_or_default(),
        }
    }

    /// 末尾から `k` 個の要素。足りなければ親のパスの全体。
    fn label(&self, k: usize) -> String {
        if k > self.names.len() {
            return self.whole.clone();
        }
        self.names[self.names.len() - k..].join(MAIN_SEPARATOR_STR)
    }
}

/// View `view` のペインの見出しの項目 (PANE-10 (b)): 名前 (`names`、PANE-11)・変更・読み取り専用を、ステータスバーの
/// 左群と同じ文言・優先度で入れる (04 §8.7)。
pub(crate) fn bar_items(
    state: &EditorState,
    view: ViewId,
    names: &BTreeMap<DocumentId, String>,
) -> Vec<StatusItem> {
    let Some(doc_id) = state.view(view).map(|v| v.doc) else {
        return Vec::new();
    };
    let Some(doc) = state.document(doc_id) else {
        return Vec::new();
    };
    let item = |kind, long: &str, short: Option<&str>, priority, cut| StatusItem {
        kind,
        long: cell_text(long, cut),
        short: short.map(|s| cell_text(s, CutPolicy::Never)),
        priority,
        align: StatusAlign::Left,
    };
    let name = names
        .get(&doc_id)
        .cloned()
        .unwrap_or_else(|| state.untitled_label(doc));
    let mut items = vec![item(StatusKind::Path, &name, None, 4, CutPolicy::Middle)];
    if doc.is_dirty() {
        items.push(item(StatusKind::Dirty, "[+]", None, 5, CutPolicy::Never));
    }
    if !doc.read_only.is_empty() {
        let label = |c: &str| state.key_label(c);
        let long = state
            .catalog()
            .render(keys::STATUS_READ_ONLY_LONG, &[], &label);
        let short = state
            .catalog()
            .render(keys::STATUS_READ_ONLY_SHORT, &[], &label);
        items.push(item(
            StatusKind::ReadOnly,
            &long,
            Some(&short),
            5,
            CutPolicy::Never,
        ));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(paths: &[&str]) -> Vec<String> {
        let owned: Vec<FsPath> = paths.iter().map(|p| FsPath::from(*p)).collect();
        let docs: Vec<(DocumentId, Option<&FsPath>)> = owned
            .iter()
            .enumerate()
            .map(|(i, p)| (DocumentId(i as u64 + 1), Some(p)))
            .collect();
        let m = disambiguate(&docs);
        (1..=paths.len() as u64)
            .map(|i| m[&DocumentId(i)].clone())
            .collect()
    }

    fn sep(s: &str) -> String {
        s.replace('/', MAIN_SEPARATOR_STR)
    }

    #[test]
    fn 名前が重ならなければファイル名のまま() {
        assert_eq!(names(&["/w/a.rs", "/w/b.rs"]), vec!["a.rs", "b.rs"]);
    }

    #[test]
    fn 同じファイル名は親のディレクトリの末尾の要素で区別する() {
        assert_eq!(
            names(&["/p/a/mod.rs", "/p/b/mod.rs", "/p/lib.rs"]),
            vec!["mod.rs (a)", "mod.rs (b)", "lib.rs"]
        );
    }

    #[test]
    fn 末尾の_1_要素で区別できなければ要素を増やす() {
        assert_eq!(
            names(&["/x/src/mod.rs", "/y/src/mod.rs", "/y/tests/mod.rs"]),
            vec![
                format!("mod.rs ({})", sep("x/src")),
                format!("mod.rs ({})", sep("y/src")),
                format!("mod.rs ({})", sep("y/tests")),
            ]
        );
    }

    #[test]
    fn 要素が足りない文書は親のパスの全体() {
        let got = names(&["/a.txt", "/d/a.txt"]);
        assert_eq!(got[1], "a.txt (d)");
        assert_eq!(got[0], format!("a.txt ({})", FsPath::from("/").display()));
    }

    #[test]
    fn 無題の文書は入れない() {
        let p = FsPath::from("/w/a.rs");
        let m = disambiguate(&[(DocumentId(1), None), (DocumentId(2), Some(&p))]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[&DocumentId(2)], "a.rs");
    }
}
