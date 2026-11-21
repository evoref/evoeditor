//! 走査が読む git の源 (`.git` の種類、`info/exclude`・`config`・`.gitignore`) を `FileSystem` で読む (docs/03 §12.7.3 の
//! 「リポジトリ」と「読み方」)。解釈は `gitignore` の純粋関数が行う。

use std::path::{Component, Path};

use crate::gitignore::{
    CaseFold, GitPath, IGNORE_FILE_MAX_BYTES, IgnoreRules, RepoIgnore, parse_commondir_file,
    parse_core_ignorecase, parse_gitdir_file,
};
use crate::io::fs::{EntryKind, FileKind, FileSystem};
use crate::io::fspath::FsPath;

/// git の `config`・`.git` のファイル・`commondir` を読む上限 (ignore のファイルと同じ 1 MiB)。
const GIT_FILE_MAX_BYTES: usize = IGNORE_FILE_MAX_BYTES;

/// 名前が `.git` か (Windows・macOS は ASCII の大文字小文字を畳む、03 §12.7.2 の 4)。
pub(super) fn is_dot_git(name: &[u8]) -> bool {
    name_eq(name, b".git")
}

/// 名前が `.gitignore` か (同上)。
pub(super) fn is_gitignore(name: &[u8]) -> bool {
    name_eq(name, b".gitignore")
}

fn name_eq(name: &[u8], want: &[u8]) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        name.eq_ignore_ascii_case(want)
    } else {
        name == want
    }
}

/// `core.ignorecase` を読めないときの既定 (Windows・macOS は畳む、Linux は区別する)。
fn os_default_fold() -> CaseFold {
    if cfg!(any(windows, target_os = "macos")) {
        CaseFold::Ascii
    } else {
        CaseFold::Sensitive
    }
}

/// `path` を最大 `cap` + 1 バイトまで読む (`/dev/zero` などへの取り違えに備える、03 §12.7.3 の「読み方」)。開けない・途中で
/// 失敗したら `None`。
pub(super) fn read_capped(fs: &dyn FileSystem, path: &FsPath, cap: usize) -> Option<Vec<u8>> {
    let mut r = fs.open_read(path).ok()?;
    let mut out = Vec::new();
    loop {
        match r.next_chunk() {
            Ok(Some(c)) => {
                let room = (cap + 1).saturating_sub(out.len());
                out.extend_from_slice(&c[..c.len().min(room)]);
                if out.len() > cap {
                    return Some(out);
                }
            }
            Ok(None) => return Some(out),
            Err(_) => return None,
        }
    }
}

/// 通常のファイル (リンクでない) のときだけ読む (`metadata` で種類を確かめる。シンボリックリンクは辿らない)。
fn read_regular(fs: &dyn FileSystem, path: &FsPath, cap: usize) -> Option<Vec<u8>> {
    let meta = fs.metadata(path).ok()?;
    if meta.kind != FileKind::File || meta.link.is_some() {
        return None;
    }
    read_capped(fs, path, cap)
}

/// `.git` の項目の種類 (`read_dir` の種類、またはルートより上の探索の `metadata`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DotGit {
    /// ディレクトリ (ふつうのリポジトリ)。
    Dir,
    /// ファイル (`git worktree`・サブモジュール。`gitdir:` の行が git のディレクトリを指す)。
    File,
    /// シンボリックリンク・その他 (源を読まない。リポジトリの境目としてだけ扱う)。
    Other,
}

impl DotGit {
    pub(super) fn from_entry(kind: EntryKind) -> DotGit {
        match kind {
            EntryKind::Dir => DotGit::Dir,
            EntryKind::File => DotGit::File,
            EntryKind::Symlink | EntryKind::Other => DotGit::Other,
        }
    }
}

/// ルート `dir` から上へ (ルートを含む) `.git` を探し、最初に見つかったディレクトリと `.git` の種類を返す (03 §12.7.3 の
/// 「リポジトリ」。上方の探索はファイルシステムのルートで止める)。
pub(super) fn find_repo_upward(fs: &dyn FileSystem, dir: &FsPath) -> Option<(FsPath, DotGit)> {
    let mut cur = Some(dir.clone());
    while let Some(d) = cur {
        if let Ok(meta) = fs.metadata(&d.join(".git")) {
            let kind = if meta.link.is_some() {
                DotGit::Other
            } else {
                match meta.kind {
                    FileKind::Dir => DotGit::Dir,
                    FileKind::File => DotGit::File,
                    FileKind::Other => DotGit::Other,
                }
            };
            return Some((d, kind));
        }
        cur = d.parent();
    }
    None
}

/// リポジトリ `repo_root` の除外の源 (`info/exclude` と `core.ignorecase`) を読む。`base` は `info/exclude` の置き場所
/// (走査の最上位からのリポジトリのルートの相対パス)。`respect_gitignore = false` なら `info/exclude` は読まない
/// (`core.ignorecase` は `workspace.exclude` の照合に使うので読む)。
pub(super) fn load_repo(
    fs: &dyn FileSystem,
    repo_root: &FsPath,
    dotgit: DotGit,
    base: &[u8],
    respect_gitignore: bool,
) -> RepoIgnore {
    let Some(common) = common_git_dir(fs, repo_root, dotgit) else {
        return RepoIgnore::new(IgnoreRules::default(), os_default_fold());
    };
    let info = if respect_gitignore {
        read_regular(
            fs,
            &common.join("info").join("exclude"),
            IGNORE_FILE_MAX_BYTES,
        )
        .map(|b| IgnoreRules::parse(base, &b).0)
        .unwrap_or_default()
    } else {
        IgnoreRules::default()
    };
    let fold = match read_regular(fs, &common.join("config"), GIT_FILE_MAX_BYTES) {
        Some(cfg) if parse_core_ignorecase(&cfg) == Some(true) => CaseFold::Ascii,
        Some(_) => CaseFold::Sensitive,
        None => os_default_fold(),
    };
    RepoIgnore::new(info, fold)
}

/// `info/exclude` と `config` を読む git のディレクトリ (`.git` がディレクトリならそれ。ファイルなら `gitdir:` の行き先と、
/// そこに `commondir` があればそれが指す共通の git ディレクトリ)。UNC・デバイスの名前空間は辿らない (`GitPath::Refused`)。
fn common_git_dir(fs: &dyn FileSystem, repo_root: &FsPath, dotgit: DotGit) -> Option<FsPath> {
    let dotgit_path = repo_root.join(".git");
    match dotgit {
        DotGit::Dir => Some(dotgit_path),
        DotGit::Other => None,
        DotGit::File => {
            let gitdir = resolve(
                repo_root,
                parse_gitdir_file(&read_capped(fs, &dotgit_path, GIT_FILE_MAX_BYTES)?)?,
            )?;
            match read_regular(fs, &gitdir.join("commondir"), GIT_FILE_MAX_BYTES) {
                Some(c) => resolve(&gitdir, parse_commondir_file(&c)?),
                None => Some(gitdir),
            }
        }
    }
}

/// `gitdir:`・`commondir` の行き先を解決する。相対パスは `base` から字句だけで解決する (`..` を親に、`.` を飛ばす。
/// Windows の verbatim のルートでは OS が `..` と `/` を解釈しないため)。
fn resolve(base: &FsPath, target: GitPath) -> Option<FsPath> {
    match target {
        GitPath::Relative(p) => Some(join_lexical(base, &p)),
        GitPath::Absolute(p) => Some(FsPath::from(p.as_str())),
        GitPath::Refused => None,
    }
}

fn join_lexical(base: &FsPath, rel: &str) -> FsPath {
    let mut out = base.as_path().to_path_buf();
    let is_sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    for part in rel.split(is_sep) {
        match part {
            "" | "." => {}
            ".." => {
                // ルートより上には出ない (ルートの要素は残す)
                if out
                    .components()
                    .next_back()
                    .is_some_and(|c| matches!(c, Component::Normal(_)))
                {
                    out.pop();
                }
            }
            _ => out.push(part),
        }
    }
    FsPath::new(out)
}

/// ディレクトリ `dir` の `.gitignore` (名前は列挙で見つけたもの) を読んで解釈する。`base` は置き場所 (最上位からの相対パス)。
/// 読めなければ `None`。
pub(super) fn read_gitignore(
    fs: &dyn FileSystem,
    path: &FsPath,
    base: &[u8],
) -> Option<IgnoreRules> {
    let text = read_capped(fs, path, IGNORE_FILE_MAX_BYTES)?;
    Some(IgnoreRules::parse(base, &text).0)
}

/// ルートより上の祖先 `dir` の `.gitignore` (通常のファイルのときだけ) を読む。
pub(super) fn read_ancestor_gitignore(
    fs: &dyn FileSystem,
    dir: &FsPath,
    base: &[u8],
) -> Option<IgnoreRules> {
    let text = read_regular(fs, &dir.join(".gitignore"), IGNORE_FILE_MAX_BYTES)?;
    Some(IgnoreRules::parse(base, &text).0)
}

/// `path` を `/` で区切ったバイト列にする (Unix は元のバイト列、Windows は WTF-8)。
pub(super) fn rel_bytes(path: &Path) -> Vec<u8> {
    let mut out = Vec::new();
    for c in path.components() {
        if let Component::Normal(n) = c {
            if !out.is_empty() {
                out.push(b'/');
            }
            out.extend_from_slice(n.as_encoded_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 相対の行き先は字句で解決する() {
        let base = FsPath::from("/r/sub");
        let p = join_lexical(&base, "../.git/worktrees/w");
        let want: std::path::PathBuf = ["/r", ".git", "worktrees", "w"].iter().collect();
        assert_eq!(p.as_path(), want.as_path());
        let p = join_lexical(&FsPath::from("/"), "../../x");
        assert_eq!(p.as_path(), Path::new("/x"));
    }

    #[test]
    fn 相対パスは区切りを斜線にしたバイト列にする() {
        let p: std::path::PathBuf = ["a", "b", "c.txt"].iter().collect();
        assert_eq!(rel_bytes(&p), b"a/b/c.txt");
        assert_eq!(rel_bytes(Path::new("")), b"");
    }

    #[test]
    fn git_の名前() {
        assert!(is_dot_git(b".git"));
        assert!(!is_dot_git(b".gitx"));
        assert!(is_gitignore(b".gitignore"));
        assert_eq!(
            is_dot_git(b".GIT"),
            cfg!(any(windows, target_os = "macos")),
            "Windows・macOS は大文字小文字を畳む"
        );
    }
}
