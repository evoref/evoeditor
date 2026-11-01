//! `tests/ac-checklist.toml` の軽い検査 (docs/11-dev-workflow.md §3.3)。v0.1 の `xtask trace` の代用。
//!
//! (1) 01 §5.1〜§5.4 の v0.1 の AC がすべてチェックリストにあり、チェックリストに余計な項目がない。
//! (2) テストの `verifies:` に書かれた AC ID がすべて 01 に存在する (打ち間違いの検出)。
//! (3) `status = "done"` の項目は、`verified_by` に実在する検証の実体が 1 つ以上ある:
//!     - テスト関数名: その関数の直前 (5 行以内) に同じ AC ID の `verifies:` がある
//!     - ベンチ ID (`BM-…`): docs/09 に載っている
//!     - 手順の記録: `docs/compat/` の下の実在するファイルのパス。人手のレビューの記録は `docs/review/` の下

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::util::{read_text, rust_files, workspace_root};

#[derive(Debug, Deserialize)]
struct File {
    #[serde(default)]
    ac: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    id: String,
    requirement: String,
    #[allow(dead_code)]
    kind: String,
    #[serde(default)]
    verified_by: Vec<String>,
    status: String,
}

/// ソースの中の `verifies:` 行から (AC ID, その直後の関数名) を集める。
fn scan_verifies(root: &Path) -> (Vec<(String, String)>, BTreeSet<String>) {
    let mut pairs = Vec::new();
    let mut mentioned = BTreeSet::new();
    for dir in ["crates", "xtask", "bins"] {
        for file in rust_files(&root.join(dir)) {
            let Ok(text) = read_text(&file) else { continue };
            let lines: Vec<&str> = text.lines().collect();
            for (i, l) in lines.iter().enumerate() {
                let Some(pos) = l.find("verifies:") else {
                    continue;
                };
                let ids: Vec<String> = ac_ids_in(&l[pos + "verifies:".len()..]);
                for id in &ids {
                    mentioned.insert(id.clone());
                }
                // 直後 5 行以内の `fn <name>` を、この verifies: の対象とする
                let name = lines[i + 1..lines.len().min(i + 6)].iter().find_map(|n| {
                    let n = n.trim_start();
                    let n = n.strip_prefix("pub ").unwrap_or(n);
                    let n = n.strip_prefix("async ").unwrap_or(n);
                    n.strip_prefix("fn ").map(|rest| {
                        rest.chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect::<String>()
                    })
                });
                if let Some(name) = name {
                    for id in ids {
                        pairs.push((id, name.clone()));
                    }
                }
            }
        }
    }
    (pairs, mentioned)
}

/// 文字列の中の `AC-FR-105-1` / `AC-NFR-203-2` の形をすべて取り出す。
fn ac_ids_in(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(p) = rest.find("AC-") {
        let tail = &rest[p..];
        let len = tail
            .char_indices()
            .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '-'))
            .map_or(tail.len(), |(i, _)| i);
        out.push(tail[..len].trim_end_matches('-').to_owned());
        rest = &tail[len.max(3)..];
    }
    out
}

pub fn run() -> Result<(), String> {
    let root = workspace_root();
    println!("▶ AC のチェックリストの検査");
    let list: File = toml::from_str(&read_text(&root.join("tests/ac-checklist.toml"))?)
        .map_err(|e| format!("tests/ac-checklist.toml を読めません: {e}"))?;
    let doc01 = read_text(&root.join("docs/01-requirements.md"))?;
    let docs_acs = eve_docs_tables::ac::read_acs(&doc01)
        .map_err(|e| format!("01 §5 の AC 表を読めません: {e}"))?;

    let mut errors: Vec<String> = Vec::new();
    let in_docs: BTreeSet<&str> = docs_acs.iter().map(|a| a.id.as_str()).collect();
    let in_list: BTreeMap<&str, &Entry> = list.ac.iter().map(|e| (e.id.as_str(), e)).collect();

    // (1) 欠落と余剰
    for id in &in_docs {
        if !in_list.contains_key(id) {
            errors.push(format!("(1) 01 の AC `{id}` がチェックリストにありません"));
        }
    }
    for e in &list.ac {
        if !in_docs.contains(e.id.as_str()) {
            errors.push(format!("(1) チェックリストの `{}` は 01 §5 の v0.1 の AC にありません (打ち間違いか、後の版の AC)", e.id));
        }
        if !e.id.contains(&e.requirement) {
            errors.push(format!(
                "(1) `{}` の requirement `{}` が ID と合いません",
                e.id, e.requirement
            ));
        }
        if e.status != "todo" && e.status != "done" {
            errors.push(format!(
                "(1) `{}` の status は todo か done です: {}",
                e.id, e.status
            ));
        }
    }
    if in_list.len() != list.ac.len() {
        errors.push("(1) チェックリストに重複した id があります".into());
    }

    // (2) verifies: の打ち間違い
    let (pairs, mentioned) = scan_verifies(&root);
    for id in &mentioned {
        if !in_docs.contains(id.as_str()) {
            errors.push(format!(
                "(2) ソースの `verifies: {id}` は 01 に存在しません"
            ));
        }
    }

    // (3) done の裏づけ
    let doc09 = read_text(&root.join("docs/09-testing-quality.md")).unwrap_or_default();
    for e in list.ac.iter().filter(|e| e.status == "done") {
        let backed = e.verified_by.iter().any(|v| {
            if v.starts_with("BM-") {
                doc09.contains(v.as_str())
            } else if v.starts_with("docs/compat/") || v.starts_with("docs/review/") {
                root.join(v.split('#').next().unwrap_or(v)).exists()
            } else {
                pairs.iter().any(|(id, name)| id == &e.id && name == v)
            }
        });
        if !backed {
            errors.push(format!("(3) `{}` は done ですが、verified_by に実在する検証 (テスト関数・ベンチ ID・docs/compat/ または docs/review/ の記録) がありません", e.id));
        }
    }

    let done = list.ac.iter().filter(|e| e.status == "done").count();
    println!(
        "  v0.1 の AC {} 件 (01 §5)、チェックリスト {} 件、done {done} 件",
        docs_acs.len(),
        list.ac.len()
    );
    if errors.is_empty() {
        println!("  OK");
        Ok(())
    } else {
        Err(format!(
            "AC のチェックリストに {} 件の問題があります\n  - {}",
            errors.len(),
            errors.join("\n  - ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // verifies: AC-NFR-707-2
    #[test]
    fn 区分が_t_だけの_ac_は試験関数に結びついている() {
        // 01 NFR-707 AC-2: ヘッドレスで検証できる Must の AC が、Rust の試験 (Harness 等) で表現されている。
        // 区分が T だけの AC (自動試験だけで確かめると定めたもの) は、done なら必ず `verifies:` つきの試験関数を持つ
        let root = workspace_root();
        let list: File = toml::from_str(&read_text(&root.join("tests/ac-checklist.toml")).unwrap())
            .expect("チェックリストを読める");
        let (pairs, _) = scan_verifies(&root);
        // 試験関数でなく別の裏づけを使う T の AC (理由つき。増やさない)
        let others = [
            "AC-FR-117-2",  // 性能の判定 (BM-103b-reconcile)
            "AC-FR-206-3",  // 実機の IME の手動確認の記録
            "AC-NFR-401-1", // CI のジョブの実行の記録
        ];
        let bad: Vec<&str> = list
            .ac
            .iter()
            .filter(|e| {
                e.kind.trim() == "T" && e.status == "done" && !others.contains(&e.id.as_str())
            })
            .filter(|e| {
                !e.verified_by
                    .iter()
                    .any(|v| pairs.iter().any(|(id, name)| id == &e.id && name == v))
            })
            .map(|e| e.id.as_str())
            .collect();
        assert!(
            bad.is_empty(),
            "試験関数に結びついていない T の AC: {bad:?}"
        );
        let n = list.ac.iter().filter(|e| e.kind.trim() == "T").count();
        assert!(n >= 20, "T の AC が少なすぎる (表の読み違い?): {n}");
    }

    // verifies: AC-NFR-501-1
    #[test]
    fn 製品のコードと依存にネットワーク通信が無い() {
        let root = crate::util::workspace_root();
        // (1) 依存: ネットワークを使う crate が配布ビルドの依存に無い (Cargo.lock の全体で見る。xtask 用も含めて 1 つも無い)
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).expect("Cargo.lock");
        let banned = [
            "reqwest",
            "hyper",
            "ureq",
            "curl",
            "openssl",
            "native-tls",
            "rustls",
            "socket2",
            "mio",
            "tokio",
            "tungstenite",
            "h2",
            "isahc",
            "attohttpc",
            "surf",
            "hickory-resolver",
        ];
        let mut found: Vec<String> = Vec::new();
        for b in banned {
            if lock.contains(&format!("name = \"{b}\"")) {
                found.push(format!("依存 {b}"));
            }
        }
        // (2) windows-sys のネットワークの feature を使わない
        for dir in ["crates", "bins"] {
            let Ok(rd) = std::fs::read_dir(root.join(dir)) else {
                continue;
            };
            for e in rd.flatten() {
                let toml = std::fs::read_to_string(e.path().join("Cargo.toml")).unwrap_or_default();
                if toml.contains("Win32_Networking") || toml.contains("Win32_NetworkManagement") {
                    found.push(format!(
                        "{} の windows-sys がネットワークの feature を使う",
                        e.path().display()
                    ));
                }
            }
        }
        // (3) ソース: 試験・ベンチ・補助を除く製品のコードに、ソケットや HTTP の呼出しが無い
        let patterns = [
            "std::net",
            "TcpStream",
            "TcpListener",
            "UdpSocket",
            "WSAStartup",
            "WinHttp",
            "InternetOpen",
            "URLDownload",
        ];
        for dir in ["crates", "bins"] {
            for f in crate::util::rust_files(&root.join(dir)) {
                let p = f.to_string_lossy().replace('\\', "/");
                if p.contains("/tests/")
                    || p.contains("/benches/")
                    || p.contains("/eve-test-helpers/")
                {
                    continue;
                }
                let text = std::fs::read_to_string(&f).unwrap_or_default();
                let body = text.split("#[cfg(test)]").next().unwrap_or("");
                for pat in patterns {
                    if body.contains(pat) {
                        found.push(format!("{p}: {pat}"));
                    }
                }
            }
        }
        assert!(found.is_empty(), "ネットワーク通信の可能性: {found:?}");
    }

    // verifies: AC-NFR-502-1
    #[test]
    fn 製品のコードは外部のプロセスを起動しない() {
        // モードラインやワークスペース設定に外部コマンドがあっても起動されない (v0.1 はそもそも起動する経路が無い)。
        // 試験・ベンチ・補助の crate を除いた製品のソースに、プロセスを起動する呼出しが無いことを確かめる。
        // 例外は Unix のクリップボードのコマンド (`clipboard_cmd`): コピー・貼り付けの操作でだけ、root しか書けない場所で
        // 検出した既知のコマンドか WSL の絶対パスの `powershell.exe` を起こす。ファイルの中身からは起動しない (05 §8.3)
        const ALLOWED: &[&str] = &["crates/eve-platform/src/clipboard_cmd.rs"];
        let root = crate::util::workspace_root();
        let mut found = Vec::new();
        for dir in ["crates", "bins"] {
            for f in crate::util::rust_files(&root.join(dir)) {
                let p = f.to_string_lossy().replace('\\', "/");
                if p.contains("/tests/")
                    || p.contains("/benches/")
                    || p.contains("/eve-test-helpers/")
                    || p.contains("/eve-testkit/")
                    || ALLOWED.iter().any(|a| p.ends_with(a))
                {
                    continue;
                }
                let text = std::fs::read_to_string(&f).unwrap_or_default();
                // `#[cfg(test)]` の以降は試験のコード
                let body = text.split("#[cfg(test)]").next().unwrap_or("");
                if body.contains("Command::new") || body.contains("CreateProcess") {
                    found.push(p);
                }
            }
        }
        assert!(found.is_empty(), "プロセスを起動する呼出し: {found:?}");
    }

    use super::ac_ids_in;

    #[test]
    fn verifies_行から_ac_id_を取り出す() {
        assert_eq!(
            ac_ids_in(" AC-FR-105-1, AC-NFR-203-2"),
            ["AC-FR-105-1", "AC-NFR-203-2"]
        );
        assert_eq!(ac_ids_in(" AC-FR-105-1."), ["AC-FR-105-1"]);
        assert!(ac_ids_in("なし").is_empty());
    }
}
