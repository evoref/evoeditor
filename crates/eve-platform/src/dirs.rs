//! 設定・状態・データのディレクトリの解決 (docs/05 §7.1、Windows)。環境変数だけで決める純粋な計算 (I/O をしない。
//! ディレクトリは作らない。作るのは最初の書込の直前、05 §7.1「作成タイミング」)。
//!
//! 優先順位: `EVE_*_DIR` > `XDG_*_HOME` (明示されているときだけ。dotfile 管理ツール向け) > Windows の既定
//! (`%APPDATA%\evoeditor`、`%LOCALAPPDATA%\evoeditor\state` / `data`)。どれも解決できなければ `None`。

use eve_core::{FsError, FsPath, Paths};

use crate::path::{PathCtx, normalize_windows};

const APP: &str = "evoeditor";

/// 解決したディレクトリ。解決できないもの (環境変数が無い) は `None` (設定なし・状態なしで起動する、FR-702)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dirs {
    pub config: Option<FsPath>,
    pub state: Option<FsPath>,
    pub data: Option<FsPath>,
}

fn nonempty(env: &dyn Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    env(key).filter(|v| !v.is_empty())
}

fn pick(
    env: &dyn Fn(&str) -> Option<String>,
    override_key: &str,
    xdg_key: &str,
    xdg_sub: &str,
    win_key: &str,
    win_sub: &str,
) -> Option<FsPath> {
    let s = if let Some(v) = nonempty(env, override_key) {
        v
    } else if let Some(v) = nonempty(env, xdg_key) {
        format!("{}\\{APP}{xdg_sub}", v.trim_end_matches(['\\', '/']))
    } else {
        let base = nonempty(env, win_key)?;
        format!("{}\\{APP}{win_sub}", base.trim_end_matches(['\\', '/']))
    };
    Some(FsPath::new(s.replace('/', "\\")))
}

impl Dirs {
    /// 環境変数から解決する。`env` は環境変数の読み口 (本番は `std::env::var`)。
    pub fn resolve(env: &dyn Fn(&str) -> Option<String>) -> Dirs {
        Dirs {
            config: pick(env, "EVE_CONFIG_DIR", "XDG_CONFIG_HOME", "", "APPDATA", ""),
            state: pick(
                env,
                "EVE_STATE_DIR",
                "XDG_STATE_HOME",
                "",
                "LOCALAPPDATA",
                "\\state",
            ),
            data: pick(
                env,
                "EVE_DATA_DIR",
                "XDG_DATA_HOME",
                "",
                "LOCALAPPDATA",
                "\\data",
            ),
        }
    }

    /// プロセスの環境変数から解決する。
    pub fn from_process_env() -> Dirs {
        Dirs::resolve(&|k| std::env::var(k).ok())
    }
}

/// `Paths` Port の Windows 実装。解決済みの値を持つだけで、I/O をしない。
pub struct WinPaths {
    dirs: Dirs,
    ctx: PathCtx,
}

impl WinPaths {
    pub fn new(dirs: Dirs, ctx: PathCtx) -> WinPaths {
        WinPaths { dirs, ctx }
    }

    /// 起動時の環境から作る (作業ディレクトリは起動時に 1 回だけ読む。以後 `chdir` しない)。
    pub fn from_process_env() -> std::io::Result<WinPaths> {
        let cwd = std::env::current_dir()?.to_string_lossy().into_owned();
        let home = std::env::var("USERPROFILE").ok().filter(|v| !v.is_empty());
        Ok(WinPaths::new(
            Dirs::from_process_env(),
            PathCtx { cwd, home },
        ))
    }

    pub fn dirs(&self) -> &Dirs {
        &self.dirs
    }
}

/// 解決できなかったディレクトリの代わり (空のパス)。呼び側は `Dirs` の `None` を見て、使わない。
fn or_empty(p: &Option<FsPath>) -> FsPath {
    p.clone().unwrap_or_else(|| FsPath::new(""))
}

impl Paths for WinPaths {
    fn config_dir(&self) -> FsPath {
        or_empty(&self.dirs.config)
    }

    fn state_dir(&self) -> FsPath {
        or_empty(&self.dirs.state)
    }

    fn data_dir(&self) -> FsPath {
        or_empty(&self.dirs.data)
    }

    /// Windows は名前付きパイプを使うのでディレクトリは無い (05 §7.1)。呼ばれても使わない値を返す。
    fn runtime_dir(&self) -> FsPath {
        FsPath::new("")
    }

    fn normalize(&self, raw: &str) -> Result<FsPath, FsError> {
        normalize_windows(raw, &self.ctx)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| m.get(k).cloned()
    }

    fn p(s: &str) -> Option<FsPath> {
        Some(FsPath::new(s))
    }

    #[test]
    fn windows_の既定() {
        let e = env(&[
            ("APPDATA", "C:\\Users\\me\\AppData\\Roaming"),
            ("LOCALAPPDATA", "C:\\Users\\me\\AppData\\Local"),
        ]);
        let d = Dirs::resolve(&e);
        assert_eq!(d.config, p("C:\\Users\\me\\AppData\\Roaming\\evoeditor"));
        assert_eq!(
            d.state,
            p("C:\\Users\\me\\AppData\\Local\\evoeditor\\state")
        );
        assert_eq!(d.data, p("C:\\Users\\me\\AppData\\Local\\evoeditor\\data"));
    }

    #[test]
    fn xdg_が明示されていれば優先し_eve_が最優先() {
        let e = env(&[
            ("APPDATA", "C:\\A"),
            ("LOCALAPPDATA", "C:\\L"),
            ("XDG_CONFIG_HOME", "D:/dot/config/"),
            ("XDG_STATE_HOME", "D:\\dot\\state"),
            ("EVE_STATE_DIR", "E:\\evestate"),
        ]);
        let d = Dirs::resolve(&e);
        assert_eq!(d.config, p("D:\\dot\\config\\evoeditor"));
        assert_eq!(d.state, p("E:\\evestate"), "EVE_STATE_DIR はそのまま使う");
        assert_eq!(d.data, p("C:\\L\\evoeditor\\data"));
    }

    #[test]
    fn 空の環境変数は無いものとして扱う() {
        let e = env(&[
            ("APPDATA", ""),
            ("XDG_CONFIG_HOME", ""),
            ("LOCALAPPDATA", "C:\\L"),
        ]);
        let d = Dirs::resolve(&e);
        assert_eq!(d.config, None);
        assert_eq!(d.state, p("C:\\L\\evoeditor\\state"));
    }

    #[test]
    fn 何も解決できなければ_none() {
        let d = Dirs::resolve(&env(&[]));
        assert_eq!(
            d,
            Dirs {
                config: None,
                state: None,
                data: None
            }
        );
    }

    #[test]
    fn paths_の実装() {
        let dirs = Dirs::resolve(&env(&[("APPDATA", "C:\\R"), ("LOCALAPPDATA", "C:\\L")]));
        let paths = WinPaths::new(
            dirs,
            PathCtx {
                cwd: "C:\\w".into(),
                home: None,
            },
        );
        assert_eq!(paths.config_dir(), FsPath::new("C:\\R\\evoeditor"));
        assert_eq!(paths.state_dir(), FsPath::new("C:\\L\\evoeditor\\state"));
        assert_eq!(
            paths.normalize("a.txt").unwrap(),
            FsPath::new("C:\\w\\a.txt")
        );
        assert!(paths.normalize("con").is_err());
        let none = WinPaths::new(
            Dirs::resolve(&env(&[])),
            PathCtx {
                cwd: "C:\\w".into(),
                home: None,
            },
        );
        assert_eq!(none.config_dir(), FsPath::new(""));
        assert_eq!(none.dirs().config, None);
    }
}
