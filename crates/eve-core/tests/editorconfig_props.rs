//! `.editorconfig` の探索の性質 (docs/07 §10.3 の EC-1 / EC-2 / EC-5)。任意の深さの文書と、任意の段に置いた `.editorconfig`
//! について、各段のファイルの glob に当てる相対パスが「その段のディレクトリから文書へのパス」であること、近いファイルが
//! 勝ち、`root = true` のファイルより上を読まないこと。Windows のディスクの上の名前の直し方 (`align_disk_names`) の性質。
//! 件数は `PROPTEST_CASES` で増やせる。

use std::collections::BTreeSet;
use std::sync::Arc;

use eve_config::EcWidth;
use eve_core::{EditorConfigRead, FsPath, align_disk_names, read_editorconfig};
use eve_testkit::{ManualClock, MemFs};
use proptest::prelude::*;

/// ディレクトリの名前の部品 (glob の記号を含まない)。
const NAMES: &[&str] = &["a", "b", "src", "d-2", "x_y", "あ"];

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(128),
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    // 文書 `/r/<dirs>/x.rs`。段 k (0 は文書のディレクトリ、最も上は `/`) に置いたファイルは、どの段の相対パスにも当てる
    // セクション `[/<相対パス>]` を持ち、値は段の番号 + 1。当たるのは自分の段の相対パスのセクションだけ
    #[test]
    fn 各段の相対パスは段のディレクトリから文書へのパスで_近いファイルが勝つ(
        dirs in prop::collection::vec(prop::sample::select(NAMES), 0..6),
        placed in prop::collection::vec((any::<bool>(), any::<bool>()), 8),
    ) {
        let mut names: Vec<&str> = vec!["r"];
        names.extend(&dirs);
        names.push("x.rs");
        let doc = format!("/{}", names.join("/"));
        // 段 k のディレクトリ (k = 0 は文書のディレクトリ、k = names.len() - 1 は `/`)
        let levels = names.len();
        let dir_of = |k: usize| format!("/{}", names[..levels - 1 - k].join("/"));
        let rel_of = |k: usize| names[levels - 1 - k..].join("/");
        let mut body = String::new();
        for k in 0..levels {
            body.push_str(&format!("[/{}]\nindent_size = {}\n", rel_of(k), k + 1));
        }
        let fs = MemFs::new(Arc::new(ManualClock::new()));
        fs.add_file(&doc, b"");
        let mut chosen: Vec<(usize, bool)> = Vec::new();
        for (k, (put, root)) in placed.iter().copied().enumerate().take(levels) {
            if !put {
                continue;
            }
            let text = if root { format!("root = true\n{body}") } else { body.clone() };
            fs.add_file(&format!("{}/.editorconfig", dir_of(k)).replace("//", "/"), text.as_bytes());
            chosen.push((k, root));
        }
        let EditorConfigRead::Done { values, reports, missing } =
            read_editorconfig(&fs, &FsPath::from(doc.as_str()), &[])
        else {
            panic!("一時の失敗は起きない");
        };
        // `root = true` の段より上は読まない
        let stop = chosen.iter().find(|(_, root)| *root).map_or(levels - 1, |(k, _)| *k);
        let read: Vec<usize> = chosen.iter().map(|(k, _)| *k).filter(|k| *k <= stop).collect();
        prop_assert_eq!(reports.len(), read.len(), "{:?}", reports);
        prop_assert_eq!(missing.len(), stop + 1 - read.len(), "{:?}", missing);
        let report_dirs: BTreeSet<String> = reports
            .iter()
            .map(|r| FsPath::from(r.path.as_str()).parent().expect("親").display())
            .collect();
        let want_dirs: BTreeSet<String> =
            read.iter().map(|k| FsPath::from(dir_of(*k).as_str()).display()).collect();
        prop_assert_eq!(report_dirs, want_dirs);
        // 近いファイル (段の番号の小さいもの) が勝ち、そのファイルの自分の段のセクションが当たる
        let want = read.first().map(|k| u8::try_from(*k + 1).expect("段は 8 未満"));
        prop_assert_eq!(values.indent_width.as_ref().map(|(w, _)| *w), want.map(EcWidth::Width));
        if let (Some((_, at)), Some(k)) = (values.indent_width.as_ref(), read.first()) {
            let want_file = FsPath::from(dir_of(*k).as_str()).join(".editorconfig").display();
            prop_assert_eq!(&at.file, &want_file);
        }
    }

    // `align_disk_names` は長さを保ち、各要素は開いたときの名前かディスクの上の名前のどちらかで、何度掛けても同じ
    #[test]
    fn ディスクの上の名前の直し方の性質(
        opened in prop::collection::vec(prop::sample::select(&["a", "A", "Src", "src", "SOURCE~1", "my~notes", "x~1.txt", "link"][..]), 0..6),
        canonical in prop::collection::vec(prop::sample::select(&["a", "src", "sources", "target", "x-long.txt"][..]), 0..6),
    ) {
        let opened: Vec<String> = opened.iter().map(|s| (*s).to_owned()).collect();
        let canonical: Vec<String> = canonical.iter().map(|s| (*s).to_owned()).collect();
        let out = align_disk_names(&opened, &canonical);
        prop_assert_eq!(out.len(), opened.len());
        let n = opened.len().min(canonical.len());
        let (head, tail) = out.split_at(opened.len() - n);
        // 揃わない先頭の要素は開いたときのまま
        prop_assert_eq!(head, &opened[..opened.len() - n]);
        let c_tail = &canonical[canonical.len() - n..];
        let o_tail = &opened[opened.len() - n..];
        for ((got, o), c) in tail.iter().zip(o_tail).zip(c_tail) {
            prop_assert!(got == o || got == c, "{got} は {o} か {c}");
            // 大文字小文字だけが違う名前はディスクの大文字小文字に直す
            if o.to_lowercase() == c.to_lowercase() {
                prop_assert_eq!(got, c);
            }
            // `~` を含まず大文字小文字以外も違う名前 (ジャンクションの先) は替えない
            if !o.contains('~') && o.to_lowercase() != c.to_lowercase() {
                prop_assert_eq!(got, o);
            }
        }
        prop_assert_eq!(align_disk_names(&out, &canonical), out);
    }
}
