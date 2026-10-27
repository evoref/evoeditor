//! `FixedPaths`: 固定の仮想ディレクトリを返す `Paths` (docs/02 §4.4)。

use eve_core::{FsError, FsErrorKind, FsPath, Paths};

/// ルートの下に `config` / `state` / `data` / `run` を置く。相対パスは `cwd` 基準。
#[derive(Clone, Debug)]
pub struct FixedPaths {
    root: FsPath,
    cwd: FsPath,
}

impl FixedPaths {
    pub fn new(root: &str, cwd: &str) -> FixedPaths {
        FixedPaths {
            root: FsPath::from(root),
            cwd: FsPath::from(cwd),
        }
    }
}

impl Default for FixedPaths {
    fn default() -> FixedPaths {
        FixedPaths::new("/evo", "/w")
    }
}

impl Paths for FixedPaths {
    fn config_dir(&self) -> FsPath {
        self.root.join("config")
    }
    fn state_dir(&self) -> FsPath {
        self.root.join("state")
    }
    fn data_dir(&self) -> FsPath {
        self.root.join("data")
    }
    fn runtime_dir(&self) -> FsPath {
        self.root.join("run")
    }

    fn normalize(&self, raw: &str) -> Result<FsPath, FsError> {
        if raw.is_empty() || raw.contains('\0') {
            return Err(FsError::new(FsErrorKind::InvalidPath));
        }
        let joined = if raw.starts_with('/') {
            FsPath::from(raw)
        } else {
            self.cwd.join(raw)
        };
        // `.` と `..` を畳む (存在は確認しない)
        let mut parts: Vec<String> = Vec::new();
        for comp in joined.display().split(['/', '\\']) {
            match comp {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                c => parts.push(c.to_owned()),
            }
        }
        Ok(FsPath::from(format!("/{}", parts.join("/")).as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 正規化は相対を絶対にして畳む() {
        let p = FixedPaths::default();
        assert_eq!(
            p.normalize("a/./b/../c.txt").unwrap(),
            FsPath::from("/w/a/c.txt")
        );
        assert_eq!(p.normalize("/x//y/../z").unwrap(), FsPath::from("/x/z"));
        assert_eq!(
            p.normalize("../../..").unwrap(),
            FsPath::from("/"),
            "ルートより上には行かない"
        );
        assert_eq!(p.normalize("").unwrap_err().kind, FsErrorKind::InvalidPath);
    }

    #[test]
    fn ディレクトリは互いに違う() {
        let p = FixedPaths::default();
        let all = [p.config_dir(), p.state_dir(), p.data_dir(), p.runtime_dir()];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
