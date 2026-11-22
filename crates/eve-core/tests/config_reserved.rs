//! 公開前の設定キーの一覧 (`eve_config::reserved`) が、07 §5 の表のキーと一致すること
//! (docs/review/v0.3-findings.md の STR-E3)。表は `eve-docs-tables` で読む (docs/11 §8.5)。
//!
//! 読み方: §5.1〜§5.11 と §5.13 の表の `キー` (§5.13 は `キー (公開版)`) 列の、先頭から続くキーの識別子
//! (バッククォートの中が英小文字・数字・`_`・`.` だけのもの。括弧の中は読まない)。見出しのセクションが 1 つ
//! (例: 見出しの `### 5.1` の後の `[editor]`) なら表のキーはその中の名前で、`editor.` を前に付ける。見出しのセクションが 2 つ以上
//! (§5.3) か無い (§5.13) なら、表のキーはフルキー。§5.12 (`[ai]` 等) はセクションごと予約するので、キーは並べない。

use std::collections::BTreeSet;

use eve_config::reserved::{CONDITIONAL_SECTIONS, RESERVED_KEYS, RESERVED_SECTIONS};
use eve_docs_tables::{identifiers, section, strip_parentheticals, tables_in};

const DOC: &str = include_str!("../../../docs/07-config-extensibility.md");

/// 公開キー (`--dump-config` が出すキー)。
fn public_keys() -> BTreeSet<String> {
    eve_config::values(&eve_config::Config::default())
        .into_iter()
        .map(|(k, _)| k.to_owned())
        .collect()
}

fn is_key(id: &str) -> bool {
    id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
}

/// 見出しのセクション名 (`[editor]`・`[[language]]`・`[language-server.<name>]` → `editor`・`language`・`language-server`)。
fn heading_sections(heading: &str) -> BTreeSet<String> {
    identifiers(heading)
        .iter()
        .filter_map(|id| id.strip_prefix('['))
        .map(|id| {
            let id = id.trim_start_matches('[');
            id[..id.find(['.', ']', '<']).unwrap_or(id.len())].to_owned()
        })
        .collect()
}

/// 07 §5 の表のキー (フルキー) と、見出しのセクション名。
fn spec() -> (BTreeSet<String>, BTreeSet<String>) {
    let body = section(DOC, "## 5. 設定項目一覧 (スキーマと既定値)").expect("07 §5 を読める");
    let (mut keys, mut sections) = (BTreeSet::new(), BTreeSet::new());
    // `### 5.x` ごとに分ける (`####` はその下に含める)
    let starts: Vec<usize> = body
        .iter()
        .enumerate()
        .filter(|(_, (_, l))| l.starts_with("### 5."))
        .map(|(i, _)| i)
        .collect();
    for (n, &s) in starts.iter().enumerate() {
        let heading = body[s].1;
        let end = starts.get(n + 1).copied().unwrap_or(body.len());
        let secs = heading_sections(heading);
        sections.extend(secs.iter().cloned());
        if heading.starts_with("### 5.0 ") || heading.starts_with("### 5.12 ") {
            continue;
        }
        let prefix = match secs.len() {
            1 => secs.iter().next().map(|s| format!("{s}.")),
            _ => None,
        };
        for t in tables_in(&body[s + 1..end]) {
            let Some(col) = t.column("キー").or_else(|| t.column("キー (公開版)")) else {
                continue;
            };
            for r in &t.rows {
                let cell = strip_parentheticals(&r.cells[col]);
                for id in identifiers(&cell).into_iter().take_while(|id| is_key(id)) {
                    let full = match &prefix {
                        Some(p) => format!("{p}{id}"),
                        None => id,
                    };
                    keys.insert(full);
                }
            }
        }
    }
    for k in &keys {
        sections.insert(k[..k.find('.').unwrap_or(k.len())].to_owned());
    }
    (keys, sections)
}

#[test]
fn 公開前のキーは_07_の_5_の表の公開キー以外のキーと一致する() {
    let (spec_keys, _) = spec();
    let public = public_keys();
    let missing: Vec<&String> = public.iter().filter(|k| !spec_keys.contains(*k)).collect();
    assert!(missing.is_empty(), "07 §5 の表に無い公開キー: {missing:?}");
    let want: Vec<&String> = spec_keys.difference(&public).collect();
    let have: Vec<&str> = RESERVED_KEYS.to_vec();
    let mut sorted = have.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(have, sorted, "RESERVED_KEYS は昇順で重複なし");
    assert_eq!(
        have,
        want.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "eve-config の reserved.rs を 07 §5 の表に合わせる"
    );
}

#[test]
fn 公開前のセクションは_07_の_5_のセクションのうち公開キーを持たないもの() {
    let (_, spec_sections) = spec();
    let public: BTreeSet<String> = public_keys()
        .iter()
        .map(|k| k[..k.find('.').unwrap_or(k.len())].to_owned())
        .collect();
    let want: Vec<&String> = spec_sections.difference(&public).collect();
    let mut have: Vec<&str> = RESERVED_SECTIONS.to_vec();
    have.sort_unstable();
    assert_eq!(have, want.iter().map(|s| s.as_str()).collect::<Vec<_>>());
}

#[test]
fn 条件付きセクションの名前は_07_の_3_4_の表の条件キーと一致する() {
    let body = section(DOC, "### 3.4 条件付きセクション (NFR-405)").expect("07 §3.4 を読める");
    let tables = tables_in(&body);
    let t = tables
        .iter()
        .find(|t| t.column("条件キー").is_some())
        .expect("07 §3.4 に条件キーの表がある");
    let col = t.column("条件キー").expect("条件キーの列");
    // `os.<name>` → `os`
    let mut want: Vec<String> = t
        .rows
        .iter()
        .flat_map(|r| identifiers(&r.cells[col]))
        .map(|id| id[..id.find('.').unwrap_or(id.len())].to_owned())
        .collect();
    want.sort_unstable();
    want.dedup();
    assert_eq!(
        CONDITIONAL_SECTIONS, want,
        "eve-config の reserved.rs の CONDITIONAL_SECTIONS を 07 §3.4 の表に合わせる (昇順)"
    );
    // 公開キーのセクションや公開前のセクションとは重ならない
    for c in CONDITIONAL_SECTIONS {
        assert!(!RESERVED_SECTIONS.contains(c), "{c}");
        assert!(
            !public_keys()
                .iter()
                .any(|k| k.starts_with(&format!("{c}."))),
            "{c}"
        );
    }
    // 書くと未知のキーの警告にはならない: 解決する条件 (`os`・`frontend`・`env`) の知らない名前は条件の警告、`host` は
    // v0.5 では解決しないので未対応の情報診断 (07 §3.4 の COND-5)
    for c in CONDITIONAL_SECTIONS {
        let l = eve_config::load_str(&format!("[{c}.x.editor]\ntab_width = 2\n"));
        assert_eq!(l.config, eve_config::Config::default(), "{c}");
        assert_eq!(l.diagnostics.len(), 1, "{c}: {:?}", l.diagnostics);
        let (severity, kind) = (l.diagnostics[0].severity, &l.diagnostics[0].kind);
        if *c == "host" {
            assert_eq!(severity, eve_config::Severity::Info, "{c}");
            assert_eq!(kind, &eve_config::DiagKind::Unsupported, "{c}");
        } else {
            assert_eq!(severity, eve_config::Severity::Warning, "{c}");
            assert!(
                matches!(
                    kind,
                    eve_config::DiagKind::Condition(eve_config::ConditionProblem::Unknown { .. })
                ),
                "{c}: {kind:?}"
            );
        }
    }
}

// verifies: AC-NFR-405-1
#[test]
fn 解決する条件の名前は_07_の_3_4_の表の値と一致する() {
    let body = section(DOC, "### 3.4 条件付きセクション (NFR-405)").expect("07 §3.4 を読める");
    let tables = tables_in(&body);
    let t = tables
        .iter()
        .find(|t| t.column("条件キー").is_some())
        .expect("07 §3.4 に条件キーの表がある");
    let (key_col, value_col) = (
        t.column("条件キー").expect("条件キーの列"),
        t.column("値").expect("値の列"),
    );
    // 値の列の先頭から続く名前 (`windows` / `macos` / …) と、条件キーの種類を組にする。v0.5 で解決しないもの
    // (`host.*` と `env.ci`) は除く
    let mut want: Vec<String> = Vec::new();
    for r in &t.rows {
        let kind = identifiers(&r.cells[key_col])
            .into_iter()
            .next()
            .map(|id| id[..id.find('.').unwrap_or(id.len())].to_owned())
            .expect("条件キー");
        if kind == "host" {
            continue;
        }
        let cell = strip_parentheticals(&r.cells[value_col]);
        for name in identifiers(&cell)
            .into_iter()
            .take_while(|id| id.chars().all(|c| c.is_ascii_lowercase()))
        {
            if kind == "env" && name == "ci" {
                continue;
            }
            want.push(format!("{kind}.{name}"));
        }
    }
    want.sort_unstable();
    let mut have: Vec<String> = eve_config::Condition::ALL
        .iter()
        .map(|c| c.name().to_owned())
        .collect();
    have.sort_unstable();
    assert_eq!(
        have, want,
        "eve-config の Condition を 07 §3.4 の表に合わせる"
    );
}

#[test]
fn 設定キーでない名前は警告になる() {
    // コマンド ID・テーマのロール名は設定キーではない (以前は一覧に混ざり、情報診断で黙っていた)
    for (text, key) in [
        ("[ui]\npalette_open = 1\n", "ui.palette_open"),
        ("[ui]\ntext = 1\n", "ui.text"),
        ("[file]\nsave = 1\n", "file.save"),
    ] {
        let l = eve_config::load_str(text);
        assert_eq!(l.diagnostics.len(), 1, "{text}");
        let d = &l.diagnostics[0];
        assert_eq!(d.key, key);
        assert_eq!(d.severity, eve_config::Severity::Warning, "{text}");
    }
}
