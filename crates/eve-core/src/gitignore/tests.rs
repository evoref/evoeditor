//! 行の解釈と連鎖の表の試験 (docs/03 §12.7.8 の L0。例は gitignore(5) の記述から自分で作った。git の試験から写していない)。

use super::*;

fn rules(base: &str, text: &str) -> IgnoreRules {
    let (r, report) = IgnoreRules::parse(base.as_bytes(), text.as_bytes());
    assert!(report.is_clean(), "{report:?}");
    r
}

fn hit(r: &IgnoreRules, path: &str, kind: PathKind) -> Option<bool> {
    r.decide(path.as_bytes(), kind, CaseFold::Sensitive)
        .map(|h| h.excluded)
}

fn repo_chain(ws: &[&str], info: &str, fold: CaseFold) -> ExcludeChain {
    let (w, _) = IgnoreRules::from_lines(b"", ws);
    let (c, dropped) = ExcludeChain::new(w).enter_repo(RepoIgnore::new(rules("", info), fold));
    assert_eq!(dropped, 0);
    c
}

#[test]
fn 空行と注釈と否定とエスケープ() {
    let r = rules("", "# 注釈\n\n*.log\n!keep.log\n\\#hash\n\\!bang\n  \n");
    assert_eq!(r.len(), 4);
    assert_eq!(hit(&r, "a.log", PathKind::File), Some(true));
    assert_eq!(hit(&r, "keep.log", PathKind::File), Some(false));
    assert_eq!(hit(&r, "#hash", PathKind::File), Some(true));
    assert_eq!(hit(&r, "!bang", PathKind::File), Some(true));
    assert_eq!(hit(&r, "# 注釈", PathKind::File), None);
    assert_eq!(hit(&r, "a.txt", PathKind::File), None);
}

#[test]
fn 後ろの行が勝つ() {
    let r = rules("", "!keep.log\n*.log\n");
    assert_eq!(hit(&r, "keep.log", PathKind::File), Some(true));
}

#[test]
fn bom_と_crlf_と末尾の空白() {
    let r = rules("", "\u{feff}a.txt\r\nb.txt   \r\nc\\ \r\nd\\  \nt\t\n");
    assert_eq!(hit(&r, "a.txt", PathKind::File), Some(true));
    assert_eq!(hit(&r, "b.txt", PathKind::File), Some(true));
    assert_eq!(hit(&r, "c ", PathKind::File), Some(true));
    assert_eq!(hit(&r, "d ", PathKind::File), Some(true));
    // タブは落とさない
    assert_eq!(hit(&r, "t", PathKind::File), None);
    assert_eq!(hit(&r, "t\t", PathKind::File), Some(true));
}

#[test]
fn ディレクトリだけと置き場所への固定() {
    let r = rules("", "build/\n/local\ndoc/*.txt\n");
    assert_eq!(hit(&r, "build", PathKind::Dir), Some(true));
    assert_eq!(hit(&r, "build", PathKind::File), None);
    assert_eq!(hit(&r, "x/build", PathKind::Dir), Some(true));
    assert_eq!(hit(&r, "local", PathKind::File), Some(true));
    assert_eq!(hit(&r, "x/local", PathKind::File), None);
    assert_eq!(hit(&r, "doc/a.txt", PathKind::File), Some(true));
    assert_eq!(hit(&r, "x/doc/a.txt", PathKind::File), None);
    assert_eq!(hit(&r, "doc/x/a.txt", PathKind::File), None);
}

#[test]
fn 入れ子の置き場所からの相対パスに当てる() {
    let r = rules("sub/dir", "/only\nname\n");
    assert_eq!(hit(&r, "sub/dir/only", PathKind::File), Some(true));
    assert_eq!(hit(&r, "sub/dir/x/only", PathKind::File), None);
    assert_eq!(hit(&r, "sub/dir/x/name", PathKind::File), Some(true));
    // 置き場所の外には効かない
    assert_eq!(hit(&r, "only", PathKind::File), None);
    assert_eq!(hit(&r, "sub/dirx/name", PathKind::File), None);
}

#[test]
fn 空のパターンは何にも当たらない() {
    let r = rules("", "!\n/\n!/\n");
    assert!(r.is_empty());
}

#[test]
fn 源の優先() {
    let c = repo_chain(&["!keep.env"], "*.env\nsecret*\n", CaseFold::Sensitive);
    let (c, _) = c.with_gitignore(rules("", "!secret-ok.txt\n"));
    let (c, _) = c.with_gitignore(rules("sub", "*.tmp\n"));
    // workspace.exclude の否定は gitignore より先
    assert!(!c.is_excluded(b"keep.env", PathKind::File));
    assert!(c.is_excluded(b"other.env", PathKind::File));
    // 内側の .gitignore → 外側 → info/exclude
    assert!(!c.is_excluded(b"secret-ok.txt", PathKind::File));
    assert!(c.is_excluded(b"secret.txt", PathKind::File));
    assert!(c.is_excluded(b"sub/a.tmp", PathKind::File));
    assert_eq!(
        c.decide(b"sub/a.tmp", PathKind::File).map(|h| h.source),
        Some(IgnoreSource::GitIgnore(0))
    );
    assert_eq!(
        c.decide(b"x/secret.txt", PathKind::File).map(|h| h.source),
        Some(IgnoreSource::InfoExclude)
    );
}

#[test]
fn 入れ子のリポジトリで外側の規則は効かない() {
    let outer = repo_chain(&["*.bak"], "*.swp\n", CaseFold::Sensitive);
    let (outer, _) = outer.with_gitignore(rules("", "*.log\n"));
    assert!(outer.is_excluded(b"inner/a.log", PathKind::File));
    let (inner, _) = outer.enter_repo(RepoIgnore::new(rules("inner", "*.o\n"), CaseFold::Ascii));
    let (inner, _) = inner.with_gitignore(rules("inner", "*.tmp\n"));
    assert!(!inner.is_excluded(b"inner/a.log", PathKind::File));
    assert!(!inner.is_excluded(b"inner/a.swp", PathKind::File));
    assert!(inner.is_excluded(b"inner/A.TMP", PathKind::File));
    assert!(inner.is_excluded(b"inner/a.O", PathKind::File));
    // workspace.exclude は境目に依らず効き、内側の畳み方で照らす
    assert!(inner.is_excluded(b"inner/x.BAK", PathKind::File));
}

#[test]
fn リポジトリの外では_gitignore_を足さない() {
    let (w, _) = IgnoreRules::from_lines(b"", &["/build/"]);
    let c = ExcludeChain::new(w);
    let (c, _) = c.with_gitignore(rules("", "*.log\n"));
    assert!(!c.in_repo());
    assert!(!c.is_excluded(b"a.log", PathKind::File));
    assert!(c.is_excluded(b"build", PathKind::Dir));
    assert!(!c.is_excluded(b"x/build", PathKind::Dir));
    assert_eq!(c.fold(), CaseFold::Sensitive);
}

#[test]
fn 大文字小文字は_core_ignorecase_に従う() {
    let c = repo_chain(&[], "", CaseFold::Ascii);
    let (c, _) = c.with_gitignore(rules("", "*.log\nbuild/\n"));
    assert!(c.is_excluded(b"FOO.LOG", PathKind::File));
    assert!(c.is_excluded(b"Build", PathKind::Dir));
    let c = repo_chain(&[], "", CaseFold::Sensitive);
    let (c, _) = c.with_gitignore(rules("", "*.log\nbuild/\n"));
    assert!(!c.is_excluded(b"FOO.LOG", PathKind::File));
    assert!(!c.is_excluded(b"Build", PathKind::Dir));
}

#[test]
fn 許可リストの形() {
    let c = repo_chain(&[], "", CaseFold::Sensitive);
    let (c, _) = c.with_gitignore(rules("", "*\n!*/\n!*.rs\n"));
    assert!(c.is_excluded(b"a.txt", PathKind::File));
    assert!(!c.is_excluded(b"src", PathKind::Dir));
    assert!(!c.is_excluded(b"src/main.rs", PathKind::File));
    assert!(c.is_excluded(b"src/data.bin", PathKind::File));
}

#[test]
fn 資源の上限() {
    // 大きすぎるファイルは読まない
    let big = vec![b'a'; IGNORE_FILE_MAX_BYTES + 1];
    let (r, report) = IgnoreRules::parse(b"", &big);
    assert!(r.is_empty() && report.too_large);
    // 長すぎる行は飛ばす
    let text = format!("{}\nok\n", "x".repeat(IGNORE_LINE_MAX_BYTES + 1));
    let (r, report) = IgnoreRules::parse(b"", text.as_bytes());
    assert_eq!((r.len(), report.long_lines), (1, 1));
    // 1 ファイルのパターンの数
    let text = "a\n".repeat(IGNORE_FILE_MAX_PATTERNS + 5);
    let (r, report) = IgnoreRules::parse(b"", text.as_bytes());
    assert_eq!(r.len(), IGNORE_FILE_MAX_PATTERNS);
    assert_eq!(report.dropped_patterns, 5);
    // 連鎖の合計
    let mut c = repo_chain(&[], "", CaseFold::Sensitive);
    let mut dropped = 0;
    for _ in 0..6 {
        let (r, _) = IgnoreRules::parse(b"", text.as_bytes());
        let (next, d) = c.with_gitignore(r);
        c = next;
        dropped += d;
    }
    assert_eq!(c.pattern_count(), IGNORE_CHAIN_MAX_PATTERNS);
    assert_eq!(
        usize::try_from(dropped).unwrap(),
        6 * IGNORE_FILE_MAX_PATTERNS - IGNORE_CHAIN_MAX_PATTERNS
    );
}

#[test]
fn nul_から後ろは読まない() {
    let r = rules("", "abc\0def\n");
    assert_eq!(hit(&r, "abc", PathKind::File), Some(true));
}
