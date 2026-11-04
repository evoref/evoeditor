//! 秘密ファイルパターン (docs/07 §5.12.2 が一覧と照合規則の正本)。名前だけで「秘密を含み得るファイル」を判定する固定の関数で、
//! 設定キーにしない。スワップの除外 (`file.swap.exclude_secret_files`、OQ-109) が使う。

use crate::fspath::FsPath;

/// ファイル名 (最後の要素) に照合するパターン。`*` は 0 文字以上。大文字小文字は区別しない (比較は小文字にしてから)。
const NAME_PATTERNS: &[&str] = &[
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "*.keystore",
    "*.jks",
    "id_rsa*",
    "id_ed25519*",
    "id_ecdsa*",
    "*.kdbx",
    ".netrc",
    ".npmrc",
    ".pypirc",
    "credentials*",
    "*.tfstate",
    "*.tfvars",
];

/// パスのどこかにその名前のディレクトリがあれば、その下のすべてのファイルが一致する。
const DIR_NAMES: &[&str] = &[".aws", ".ssh", ".gnupg"];

/// `path` が秘密ファイルパターンに一致するか。呼び側は正規化してシンボリックリンクを解決した実パスを渡す。
pub fn is_secret_path(path: &FsPath) -> bool {
    let lower = path.display().to_lowercase();
    let mut parts: Vec<&str> = lower.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let Some(name) = parts.pop() else {
        return false;
    };
    parts.iter().any(|d| DIR_NAMES.contains(d)) || NAME_PATTERNS.iter().any(|p| wildcard(p, name))
}

/// `*` (0 文字以上) だけを持つパターンの照合。
fn wildcard(pattern: &str, text: &str) -> bool {
    let mut segs = pattern.split('*');
    let first = segs.next().unwrap_or("");
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let mut segs: Vec<&str> = segs.collect();
    let Some(last) = segs.pop() else {
        return rest.is_empty();
    };
    for s in segs {
        match rest.find(s) {
            Some(i) => rest = &rest[i + s.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(p: &str) -> bool {
        is_secret_path(&FsPath::from(p))
    }

    #[test]
    fn 名前の一覧に一致する() {
        for p in [
            r"C:\w\.env",
            "/w/.env.local",
            "/w/server.PEM",
            "/w/a.key",
            "/w/id_rsa",
            "/w/id_rsa.pub",
            "/w/id_ed25519",
            "/w/vault.kdbx",
            "/w/.netrc",
            "/w/.npmrc",
            "/w/Credentials.json",
            "/w/prod.tfvars",
            "/w/terraform.tfstate",
        ] {
            assert!(secret(p), "{p}");
        }
    }

    #[test]
    fn ディレクトリの下のファイルは何でも一致する() {
        assert!(secret(r"C:\Users\a\.ssh\config"));
        assert!(secret("/home/a/.aws/credentials"));
        assert!(secret("/home/a/.gnupg/pubring.kbx"));
        assert!(secret("/w/proj/.SSH/deep/er/file.txt"));
    }

    #[test]
    fn 一致しないものは一致しない() {
        for p in [
            "/w/readme.md",
            "/w/env",
            "/w/dotenv.txt",
            "/w/key.txt",
            "/w/keyfile",
            "/w/pem",
            "/w/.envrc",
            "/w/ssh/config",
            "/w/my.ssh",
            "/w/rsa_id",
        ] {
            assert!(!secret(p), "{p}");
        }
        // ディレクトリ名そのものがファイル名のときは、その下ではない
        assert!(!secret("/w/.ssh"));
    }

    #[test]
    fn ワイルドカードは_0_文字以上() {
        assert!(wildcard("id_rsa*", "id_rsa"));
        assert!(wildcard("*.key", ".key"));
        assert!(wildcard("a*b*c", "axxbyyc"));
        assert!(!wildcard("a*b*c", "axxbyy"));
        assert!(!wildcard("*.key", "key"));
    }
}
