//! 設定・状態・データのディレクトリの解決 (docs/05 §7.1。Windows は `Dirs::resolve`・`WinPaths`、Unix は `Dirs::resolve_unix`・
//! `UnixPaths`)。環境変数だけで決める純粋な計算 (I/O をしない。
//! ディレクトリは作らない。作るのは最初の書込の直前、05 §7.1「作成タイミング」)。
//!
//! 優先順位: `EVE_*_DIR` > `XDG_*_HOME` (明示されているときだけ。dotfile 管理ツール向け) > Windows の既定
//! (`%APPDATA%\evoeditor`、`%LOCALAPPDATA%\evoeditor\state` / `data`)。どれも解決できなければ `None`。

use eve_core::{FsError, FsPath, Paths};

use crate::path::{PathCtx, normalize_unix, normalize_windows};

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

    /// Unix の解決 (05 §7.1 の表): `EVE_*_DIR` > `$XDG_*_HOME/evoeditor` > `$HOME/.config` / `.local/state` / `.local/share`
    /// の下の `evoeditor`。`home` は 05 §3.6 で決めたホーム (無ければ既定は解決できない)。
    pub fn resolve_unix(env: &dyn Fn(&str) -> Option<String>, home: Option<&str>) -> Dirs {
        let pick = |over: &str, xdg: &str, fallback: &str| -> Option<FsPath> {
            if let Some(v) = nonempty(env, over) {
                return Some(FsPath::new(v));
            }
            let base = match nonempty(env, xdg) {
                Some(v) => v.trim_end_matches('/').to_owned(),
                None => format!(
                    "{}/{fallback}",
                    home.filter(|h| !h.is_empty())?.trim_end_matches('/')
                ),
            };
            Some(FsPath::new(format!("{base}/{APP}")))
        };
        Dirs {
            config: pick("EVE_CONFIG_DIR", "XDG_CONFIG_HOME", ".config"),
            state: pick("EVE_STATE_DIR", "XDG_STATE_HOME", ".local/state"),
            data: pick("EVE_DATA_DIR", "XDG_DATA_HOME", ".local/share"),
        }
    }

    /// プロセスの環境変数から解決する (OS の規則で)。
    pub fn from_process_env() -> Dirs {
        #[cfg(unix)]
        {
            Dirs::resolve_unix(&|k| std::env::var(k).ok(), unix_home().as_deref())
        }
        #[cfg(not(unix))]
        {
            Dirs::resolve(&|k| std::env::var(k).ok())
        }
    }
}

/// Unix のホーム (05 §3.6): `$HOME` → パスワードデータベース (`getpwuid_r(geteuid())`)。
#[cfg(unix)]
pub fn unix_home() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(crate::sys::passwd_home)
}

/// `Paths` Port の Unix 実装 (05 §7.1)。解決済みの値を持つだけで、I/O をしない。
pub struct UnixPaths {
    dirs: Dirs,
    ctx: PathCtx,
    runtime: Option<FsPath>,
}

impl UnixPaths {
    /// `runtime` は `$XDG_RUNTIME_DIR/evoeditor` (無ければ `None`)。
    pub fn new(dirs: Dirs, ctx: PathCtx, runtime: Option<FsPath>) -> UnixPaths {
        UnixPaths { dirs, ctx, runtime }
    }

    /// 起動時の環境から作る (作業ディレクトリは起動時に 1 回だけ読む)。
    #[cfg(unix)]
    pub fn from_process_env() -> std::io::Result<UnixPaths> {
        let cwd = std::env::current_dir()?.to_string_lossy().into_owned();
        let home = unix_home();
        let runtime = std::env::var("XDG_RUNTIME_DIR")
            .ok()
            .filter(|v| !v.is_empty())
            .map(|v| FsPath::new(format!("{}/{APP}", v.trim_end_matches('/'))));
        Ok(UnixPaths::new(
            Dirs::from_process_env(),
            PathCtx { cwd, home },
            runtime,
        ))
    }

    pub fn dirs(&self) -> &Dirs {
        &self.dirs
    }
}

impl Paths for UnixPaths {
    fn config_dir(&self) -> FsPath {
        or_empty(&self.dirs.config)
    }

    fn state_dir(&self) -> FsPath {
        or_empty(&self.dirs.state)
    }

    fn data_dir(&self) -> FsPath {
        or_empty(&self.dirs.data)
    }

    fn runtime_dir(&self) -> FsPath {
        or_empty(&self.runtime)
    }

    fn normalize(&self, raw: &str) -> Result<FsPath, FsError> {
        normalize_unix(raw, &self.ctx)
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
    fn unix_の既定と_xdg_と_eve() {
        let e = env(&[("XDG_STATE_HOME", "/x/state/"), ("EVE_DATA_DIR", "/d")]);
        let d = Dirs::resolve_unix(&e, Some("/home/me"));
        assert_eq!(d.config, p("/home/me/.config/evoeditor"));
        assert_eq!(d.state, p("/x/state/evoeditor"));
        assert_eq!(d.data, p("/d"));
        let none = Dirs::resolve_unix(&env(&[("XDG_CONFIG_HOME", "")]), None);
        assert_eq!(none.config, None, "ホームが無ければ既定は解決できない");
        let paths = UnixPaths::new(
            Dirs::resolve_unix(&env(&[]), Some("/h")),
            PathCtx {
                cwd: "/w".into(),
                home: Some("/h".into()),
            },
            None,
        );
        assert_eq!(paths.state_dir(), FsPath::new("/h/.local/state/evoeditor"));
        assert_eq!(paths.runtime_dir(), FsPath::new(""));
        assert_eq!(paths.normalize("~/a").unwrap(), FsPath::new("/h/a"));
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
