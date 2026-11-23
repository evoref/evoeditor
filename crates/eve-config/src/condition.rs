//! 条件付きセクション (`[os.windows.editor]` など、docs/07 §3.4 の COND-1〜COND-8、01 NFR-405)。
//!
//! 条件の値 ([`Conditions`]) は起動時に bin が 1 回だけ決めて渡し、`config.reload` も同じ値を使う (COND-2)。
//! 解決 (どの条件のセクションを、どの順で重ねるか) は `config.rs` の読込が行う。
//!
//! spec: 07 §3.4

/// 実行ファイルの対象 OS (07 §3.4 の `os.<name>`、§7.3 の `when` の `os`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    /// Windows。
    Windows,
    /// macOS。
    Macos,
    /// Linux (WSL の中を含む。WSL かどうかは `env.wsl` で分ける)。
    Linux,
}

impl Os {
    /// この実行ファイルの対象 OS (コンパイル時の定数)。v0.5 の対象の 3 つの OS でなければ `None` (どの `os.*` も偽)。
    #[must_use]
    pub const fn current() -> Option<Os> {
        if cfg!(windows) {
            Some(Os::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Os::Macos)
        } else if cfg!(target_os = "linux") {
            Some(Os::Linux)
        } else {
            None
        }
    }

    /// `when` の `os` の値と、条件の名前の後半 (`windows` / `macos` / `linux`)。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::Macos => "macos",
            Os::Linux => "linux",
        }
    }
}

/// 起動した frontend (07 §3.4 の `frontend.<name>`、§7.3 の `when` の `frontend`)。v0.5 は TUI だけ。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Frontend {
    /// 端末の frontend (eve-tui)。
    #[default]
    Tui,
}

impl Frontend {
    /// `when` の `frontend` の値と、条件の名前の後半 (`tui`)。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Frontend::Tui => "tui",
        }
    }
}

/// 環境の条件 (07 §3.4 の `env.<name>`。判定の規則は COND-2)。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct EnvConditions {
    /// SSH の中 (`SSH_TTY` か `SSH_CONNECTION` が空でない)。
    pub ssh: bool,
    /// tmux の中 (`TMUX` が空でない)。
    pub tmux: bool,
    /// WSL の中 (Linux で `WSL_DISTRO_NAME` か `WSL_INTEROP` が空でない)。
    pub wsl: bool,
}

/// 条件の値 (07 §3.4 の COND-2)。起動時に bin が 1 回だけ決め、`config.reload` も同じ値を使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Conditions {
    /// 実行ファイルの対象 OS。
    pub os: Option<Os>,
    /// 起動した frontend。
    pub frontend: Frontend,
    /// 環境の条件。
    pub env: EnvConditions,
}

impl Default for Conditions {
    /// この実行ファイルの OS・TUI・環境の条件なし (環境変数を読まない。試験と、bin が値を渡さないときの形)。
    fn default() -> Conditions {
        Conditions {
            os: Os::current(),
            frontend: Frontend::Tui,
            env: EnvConditions::default(),
        }
    }
}

impl Conditions {
    /// 条件 `c` が真か。`os.unix` は macOS と Linux で真 (07 §3.4 の表)。
    #[must_use]
    pub fn holds(&self, c: Condition) -> bool {
        match c {
            Condition::OsUnix => matches!(self.os, Some(Os::Macos | Os::Linux)),
            Condition::OsWindows => self.os == Some(Os::Windows),
            Condition::OsMacos => self.os == Some(Os::Macos),
            Condition::OsLinux => self.os == Some(Os::Linux),
            Condition::FrontendTui => self.frontend == Frontend::Tui,
            Condition::EnvSsh => self.env.ssh,
            Condition::EnvTmux => self.env.tmux,
            Condition::EnvWsl => self.env.wsl,
        }
    }
}

/// v0.5 で解決する条件 (07 §3.4 の表のうち、`host.*` と `env.ci` を除いたもの)。
///
/// 並び (`Ord`) は 1 つのファイルの中で重ねる順 (COND-3): `os` (`unix` → 具体的な OS) → `frontend` → `env`
/// (`ssh` → `tmux` → `wsl`)。後ほど強い。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Condition {
    /// `os.unix` (macOS と Linux)。
    OsUnix,
    /// `os.windows`。
    OsWindows,
    /// `os.macos`。
    OsMacos,
    /// `os.linux`。
    OsLinux,
    /// `frontend.tui`。
    FrontendTui,
    /// `env.ssh`。
    EnvSsh,
    /// `env.tmux`。
    EnvTmux,
    /// `env.wsl`。
    EnvWsl,
}

impl Condition {
    /// すべての条件 (重ねる順)。
    pub const ALL: [Condition; 8] = [
        Condition::OsUnix,
        Condition::OsWindows,
        Condition::OsMacos,
        Condition::OsLinux,
        Condition::FrontendTui,
        Condition::EnvSsh,
        Condition::EnvTmux,
        Condition::EnvWsl,
    ];

    /// 条件の名前 (`os.windows` など。診断と `config.show` の由来に出す)。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Condition::OsUnix => "os.unix",
            Condition::OsWindows => "os.windows",
            Condition::OsMacos => "os.macos",
            Condition::OsLinux => "os.linux",
            Condition::FrontendTui => "frontend.tui",
            Condition::EnvSsh => "env.ssh",
            Condition::EnvTmux => "env.tmux",
            Condition::EnvWsl => "env.wsl",
        }
    }

    /// 条件の種類 (`os` / `frontend` / `env`) と名前から。知らない組は `None`。
    #[must_use]
    pub fn from_parts(kind: &str, name: &str) -> Option<Condition> {
        Condition::ALL
            .into_iter()
            .find(|c| c.name().split_once('.') == Some((kind, name)))
    }
}

/// 解決する条件の種類 (`[os.*]`・`[frontend.*]`・`[env.*]`)。`host` は v0.5 では解決しない (名前の予約のみ)。
pub(crate) const RESOLVED_KINDS: [&str; 3] = ["os", "frontend", "env"];

/// 条件の名前のうち、v0.5 では解決せず「このバージョンでは未対応」の情報診断にするもの (COND-5 (e))。
pub(crate) const UNRESOLVED_NAMES: [&str; 1] = ["env.ci"];

#[cfg(test)]
mod tests {
    use super::*;

    fn on(os: Option<Os>, ssh: bool, tmux: bool, wsl: bool) -> Conditions {
        Conditions {
            os,
            frontend: Frontend::Tui,
            env: EnvConditions { ssh, tmux, wsl },
        }
    }

    #[test]
    fn unix_は_macos_と_linux_で真() {
        let truth = |os| {
            Condition::ALL
                .into_iter()
                .filter(|c| on(os, false, false, false).holds(*c))
                .map(Condition::name)
                .collect::<Vec<_>>()
        };
        assert_eq!(truth(Some(Os::Windows)), ["os.windows", "frontend.tui"]);
        assert_eq!(
            truth(Some(Os::Macos)),
            ["os.unix", "os.macos", "frontend.tui"]
        );
        assert_eq!(
            truth(Some(Os::Linux)),
            ["os.unix", "os.linux", "frontend.tui"]
        );
        assert_eq!(truth(None), ["frontend.tui"]);
    }

    #[test]
    fn env_の条件はそれぞれの値で決まる() {
        let c = on(Some(Os::Linux), true, false, true);
        assert!(c.holds(Condition::EnvSsh));
        assert!(!c.holds(Condition::EnvTmux));
        assert!(c.holds(Condition::EnvWsl));
        assert!(on(Some(Os::Linux), false, true, false).holds(Condition::EnvTmux));
    }

    #[test]
    fn 名前と種類の組から引ける() {
        for c in Condition::ALL {
            let (kind, name) = c.name().split_once('.').unwrap();
            assert_eq!(Condition::from_parts(kind, name), Some(c));
            assert!(RESOLVED_KINDS.contains(&kind));
        }
        assert_eq!(Condition::from_parts("os", "win"), None);
        assert_eq!(Condition::from_parts("env", "ci"), None);
        assert_eq!(Condition::from_parts("host", "x"), None);
        // 重ねる順は宣言の順 (COND-3)
        let mut sorted = Condition::ALL;
        sorted.sort();
        assert_eq!(sorted, Condition::ALL);
    }
}
