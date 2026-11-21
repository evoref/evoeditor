//! リポジトリの設定 (`core.ignorecase`) と `.git` のファイル (`gitdir:`・`commondir`) の解釈 (docs/03 §12.7.3 の
//! 「リポジトリ」と「大文字小文字」)。I/O はしない。

/// `.git` のファイルの `gitdir:` の行・`commondir` が指す先の形。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitPath {
    /// 相対パス (そのファイルのあるディレクトリから解決する)。
    Relative(String),
    /// ローカルの絶対パス (`/` で始まる、またはドライブ文字 `C:\` / `C:/` で始まる)。
    Absolute(String),
    /// 辿らない: UNC のパス (`\\server\share`・`//server/share`・verbatim の UNC)・デバイスの名前空間 (`\\.\`・`\\?\`)・
    /// ドライブの相対パス (`C:foo`)・空・UTF-8 でないもの (ファインダを開くだけで SMB に接続し、資格情報を相手に渡すことを
    /// 防ぐ、03 §12.7.3)。
    Refused,
}

/// `.git` のファイル (`git worktree`・サブモジュール) の中身から `gitdir: <path>` の行の行き先を読む。形が違えば `None`。
#[must_use]
pub fn parse_gitdir_file(content: &[u8]) -> Option<GitPath> {
    let text = std::str::from_utf8(content).ok()?;
    let line = text.lines().next()?;
    let path = line.strip_prefix("gitdir:")?;
    Some(classify(path.trim()))
}

/// 共通の git ディレクトリを指す `commondir` のファイルの中身を読む (相対パスは `commondir` のあるディレクトリから解決する)。
#[must_use]
pub fn parse_commondir_file(content: &[u8]) -> Option<GitPath> {
    let text = std::str::from_utf8(content).ok()?;
    let line = text.lines().next()?;
    Some(classify(line.trim()))
}

fn classify(path: &str) -> GitPath {
    let b = path.as_bytes();
    let sep = |c: u8| c == b'/' || c == b'\\';
    if b.is_empty() || b.contains(&0) {
        return GitPath::Refused;
    }
    // `\\server`・`//server`・`\\?\`・`\\.\`・`//?/` などの 2 つの区切りで始まる形は辿らない
    if b.len() >= 2 && sep(b[0]) && sep(b[1]) {
        return GitPath::Refused;
    }
    if sep(b[0]) {
        return GitPath::Absolute(path.to_owned());
    }
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return if b.len() >= 3 && sep(b[2]) {
            GitPath::Absolute(path.to_owned())
        } else {
            GitPath::Refused
        };
    }
    GitPath::Relative(path.to_owned())
}

/// git の `config` の本文から `[core]` の節の `ignorecase` を読む (03 §12.7.3 の「大文字小文字」)。キーが無ければ `None`
/// (呼び出し側は `false` にする)。節とキーの名前は大文字小文字を区別しない。値は `true` / `false` / `yes` / `no` / `on` /
/// `off` / `1` / `0` (ほかの整数は 0 でなければ真。値の無いキーは真、空の値は偽)。同じキーが何度あれば最後のもの。
/// `[include]` / `[includeIf]` は辿らない。読めない値の行は無視する。
#[must_use]
pub fn parse_core_ignorecase(config: &[u8]) -> Option<bool> {
    let text = String::from_utf8_lossy(config);
    let mut in_core = false;
    let mut found = None;
    for raw in text.lines() {
        let mut line = raw.trim_start();
        if let Some(rest) = line.strip_prefix('[') {
            let Some(end) = rest.find(']') else {
                in_core = false;
                continue;
            };
            let header = rest[..end].trim();
            in_core = header.eq_ignore_ascii_case("core");
            line = rest[end + 1..].trim_start();
        }
        if !in_core {
            continue;
        }
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), Some(v.trim())),
            None => (line, None),
        };
        if !key.eq_ignore_ascii_case("ignorecase") {
            continue;
        }
        if let Some(v) = parse_bool(value) {
            found = Some(v);
        }
    }
    found
}

/// 引用符の外の `#` / `;` から後ろを落とす。
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' | ';' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse_bool(value: Option<&str>) -> Option<bool> {
    let Some(v) = value else {
        return Some(true);
    };
    let v = v.trim_matches('"').trim();
    if v.is_empty() {
        return Some(false);
    }
    for (word, b) in [
        ("true", true),
        ("yes", true),
        ("on", true),
        ("false", false),
        ("no", false),
        ("off", false),
    ] {
        if v.eq_ignore_ascii_case(word) {
            return Some(b);
        }
    }
    v.parse::<i64>().ok().map(|n| n != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignorecase_の読み方() {
        assert_eq!(parse_core_ignorecase(b""), None);
        assert_eq!(
            parse_core_ignorecase(b"[core]\n\tignorecase = true\n"),
            Some(true)
        );
        assert_eq!(
            parse_core_ignorecase(b"[Core]\n\tIgnoreCase = Off ; comment\n"),
            Some(false)
        );
        assert_eq!(parse_core_ignorecase(b"[core]\n\tignorecase\n"), Some(true));
        assert_eq!(
            parse_core_ignorecase(b"[core]\n\tignorecase =\n"),
            Some(false)
        );
        assert_eq!(
            parse_core_ignorecase(b"[core] ignorecase = yes\n[user]\nignorecase = no\n"),
            Some(true)
        );
        assert_eq!(
            parse_core_ignorecase(b"[core]\nignorecase=1\n[core]\nignorecase=0\n"),
            Some(false)
        );
        // 下位の節は core ではない
        assert_eq!(
            parse_core_ignorecase(b"[core \"x\"]\nignorecase = true\n"),
            None
        );
        // 読めない値は無視する
        assert_eq!(parse_core_ignorecase(b"[core]\nignorecase = maybe\n"), None);
        // CRLF
        assert_eq!(
            parse_core_ignorecase(b"[core]\r\n\tignorecase = false\r\n"),
            Some(false)
        );
    }

    #[test]
    fn gitdir_の行き先() {
        assert_eq!(
            parse_gitdir_file(b"gitdir: ../.git/worktrees/wt\n"),
            Some(GitPath::Relative("../.git/worktrees/wt".into()))
        );
        assert_eq!(
            parse_gitdir_file(b"gitdir: C:/repo/.git/worktrees/wt\r\n"),
            Some(GitPath::Absolute("C:/repo/.git/worktrees/wt".into()))
        );
        assert_eq!(
            parse_gitdir_file(b"gitdir: /home/u/r/.git/modules/m\n"),
            Some(GitPath::Absolute("/home/u/r/.git/modules/m".into()))
        );
        for refused in [
            "gitdir: \\\\server\\share\\r.git",
            "gitdir: //server/share/r.git",
            "gitdir: \\\\?\\UNC\\server\\share",
            "gitdir: \\\\.\\pipe\\x",
            "gitdir: //?/C:/r",
            "gitdir: C:relative",
            "gitdir: ",
        ] {
            assert_eq!(
                parse_gitdir_file(refused.as_bytes()),
                Some(GitPath::Refused),
                "{refused}"
            );
        }
        assert_eq!(parse_gitdir_file(b"not a gitdir"), None);
        assert_eq!(
            parse_commondir_file(b"../..\n"),
            Some(GitPath::Relative("../..".into()))
        );
    }
}
