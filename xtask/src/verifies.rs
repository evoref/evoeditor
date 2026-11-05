//! ソースのコメントの `verifies:` の走査 (docs/11 §3.3、docs/review/v0.3-findings.md の STR-E5)。`trace` と `ac-checklist` が使う。
//!
//! 規則: `// verifies: <ID>, <ID> …` の **次の、コメント・属性・空行でない行** が、対象の試験の `fn <名前>` であること。
//! その `fn` には `#[test]` が付いていること (proptest の中の試験も `#[test]` を書く)。例外は `benches/` の下の `fn main`
//! (ベンチのバイナリ全体が検証)。合わないもの (間に別の関数・構造体がある、`#[test]` が無い) は誤りとして返す
//! (前は「直後 5 行以内の最初の `fn`」を対象にしていたので、間に補助の関数を挟むと黙って取り違えていた)。

use std::path::Path;

use crate::util::rust_files;

/// `verifies:` 1 件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verify {
    /// 書かれた ID (`AC-FR-105-1`・`FR-118`・`NFR-307` など。区切りは `,` / `、`)。
    pub ids: Vec<String>,
    /// ワークスペース相対のパス (`/` 区切り)。
    pub file: String,
    /// `verifies:` の行番号 (1 始まり)。
    pub line: usize,
    /// 対象の試験の関数名。
    pub test: String,
}

impl Verify {
    /// `<file>:<line>` (報告用)。
    pub fn at(&self) -> String {
        format!("{}:{}", self.file, self.line)
    }

    /// `verified_by` の書き方に合うか: 関数名だけ (`<fn>`) か、`<file>::<fn>`。
    pub fn matches(&self, verified_by: &str) -> bool {
        match verified_by.rsplit_once("::") {
            Some((file, test)) => self.file == file && self.test == test,
            None => self.test == verified_by,
        }
    }
}

/// `crates` / `bins` / `xtask` の Rust のソースから `verifies:` を集める。戻り値は (集めたもの, 規則に合わないものの説明)。
pub fn scan(root: &Path) -> (Vec<Verify>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for dir in ["crates", "bins", "xtask"] {
        for f in rust_files(&root.join(dir)) {
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            let rel = f
                .strip_prefix(root)
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            scan_text(&rel, &text, &mut out, &mut errors);
        }
    }
    (out, errors)
}

/// 1 ファイル分 (試験しやすいように本文を受け取る)。
pub fn scan_text(rel: &str, text: &str, out: &mut Vec<Verify>, errors: &mut Vec<String>) {
    let lines: Vec<&str> = text.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        if !t.starts_with("//") {
            continue;
        }
        let body = t
            .trim_start_matches('/')
            .trim_start_matches('!')
            .trim_start();
        let Some(rest) = body.strip_prefix("verifies:") else {
            continue;
        };
        let ids = ids_in(rest);
        let at = format!("{rel}:{}", i + 1);
        if ids.is_empty() {
            errors.push(format!("{at}: `verifies:` に ID がありません"));
            continue;
        }
        // 次のコメント・属性・空行でない行と、その間に `#[test]` があったか
        let mut has_test = false;
        let mut target: Option<&str> = None;
        for n in &lines[i + 1..] {
            let s = n.trim();
            if s.is_empty() || s.starts_with("//") {
                continue;
            }
            if s.starts_with("#[") {
                has_test |= s.starts_with("#[test]");
                continue;
            }
            target = Some(s);
            break;
        }
        let name = target.and_then(fn_name);
        let Some(name) = name else {
            errors.push(format!(
                "{at}: `verifies:` の次の行が試験の `fn` ではありません: {:?}",
                target.unwrap_or("(ファイルの終わり)")
            ));
            continue;
        };
        let bench_main = name == "main" && rel.contains("/benches/");
        if !has_test && !bench_main {
            errors.push(format!(
                "{at}: `verifies:` の対象の `fn {name}` に `#[test]` がありません (試験だけを対象にする)"
            ));
            continue;
        }
        out.push(Verify {
            ids,
            file: rel.to_owned(),
            line: i + 1,
            test: name,
        });
    }
}

/// `pub fn` / `async fn` / `fn` の名前。
fn fn_name(line: &str) -> Option<String> {
    let s = line.strip_prefix("pub ").unwrap_or(line);
    let s = s.strip_prefix("async ").unwrap_or(s);
    let rest = s.strip_prefix("fn ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// `verifies:` の後ろの ID。`,` / `、` 区切りの各項目の最初の語 (説明の語を後ろに続けてよい: `FR-118 (保存)`) と、
/// 同じ項目の中で空白で並べた ID の形の語 (`AC-FR-303-1 AC-FR-303-2`)。末尾の句点は除く。
fn ids_in(rest: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tok in rest.split([',', '、']) {
        for (k, w) in tok.split_whitespace().enumerate() {
            let w = w.trim_end_matches(['.', '。']);
            let id_like = ["AC-", "FR-", "NFR-"].iter().any(|p| w.starts_with(p));
            if !w.is_empty() && (k == 0 || id_like) {
                out.push(w.to_owned());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> (Vec<Verify>, Vec<String>) {
        let (mut out, mut errors) = (Vec::new(), Vec::new());
        scan_text("crates/x/tests/a.rs", text, &mut out, &mut errors);
        (out, errors)
    }

    #[test]
    fn 属性とコメントを挟んだ試験を対象にする() {
        let (v, e) = run(
            "// verifies: AC-FR-101-1, FR-102\n// 説明\n#[cfg(windows)]\n\n#[test]\nfn a_の試験() {}\n",
        );
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].ids, ["AC-FR-101-1", "FR-102"]);
        assert_eq!(v[0].test, "a_の試験");
        assert_eq!(v[0].line, 1);
        assert!(v[0].matches("a_の試験"));
        assert!(v[0].matches("crates/x/tests/a.rs::a_の試験"));
        assert!(!v[0].matches("crates/x/tests/b.rs::a_の試験"));
    }

    #[test]
    fn 空白で並べた_id_と説明の語() {
        assert_eq!(
            ids_in(" AC-FR-303-1 AC-FR-303-2, FR-118 (保存の途中)、NFR-307。"),
            ["AC-FR-303-1", "AC-FR-303-2", "FR-118", "NFR-307"]
        );
    }

    #[test]
    fn 間に別の関数があれば誤り() {
        // 前の規則 (5 行以内の最初の fn) では、補助の関数 helper を対象と取り違えていた
        let (v, e) =
            run("// verifies: AC-FR-107-2\n/// 補助\nfn helper() {}\n\n#[test]\nfn 試験() {}\n");
        assert!(v.is_empty());
        assert!(e[0].contains("`#[test]` がありません"), "{e:?}");
    }

    #[test]
    fn 試験でない行が続けば誤り() {
        let (_, e) = run("// verifies: FR-1\nstruct S;\n");
        assert!(e[0].contains("試験の `fn` ではありません"), "{e:?}");
        let (_, e) = run("// verifies:\n#[test]\nfn t() {}\n");
        assert!(e[0].contains("ID がありません"), "{e:?}");
    }

    #[test]
    fn ベンチの_main_は対象にしてよい() {
        let (mut out, mut errors) = (Vec::new(), Vec::new());
        scan_text(
            "crates/x/benches/b.rs",
            "// verifies: NFR-501\nfn main() {}\n",
            &mut out,
            &mut errors,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(out[0].test, "main");
    }
}
