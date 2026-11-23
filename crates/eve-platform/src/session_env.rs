//! 端末のセッションの環境 (SSH・tmux・WSL の中か) を環境変数から判定する (docs/07 §3.4 の COND-2、05 §8.2 / §15)。
//!
//! 条件付きセクションの `env.*`・`clipboard.osc52 = auto` の SSH の判定 (eve-host)・Unix のクリップボードの手段の選択が
//! 同じ規則を使う。どれも空の値は無いものとする。
//!
//! spec: 07 §3.4

/// セッションの環境。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionEnv {
    /// SSH の中 (`SSH_TTY` か `SSH_CONNECTION` が空でない)。
    pub ssh: bool,
    /// tmux の中 (`TMUX` が空でない)。
    pub tmux: bool,
    /// WSL の中 (`WSL_DISTRO_NAME` か `WSL_INTEROP` が空でない)。[`session_env`] は Linux でだけ真にする。
    pub wsl: bool,
}

fn set(env: &dyn Fn(&str) -> Option<String>, key: &str) -> bool {
    env(key).is_some_and(|v| !v.is_empty())
}

/// 環境変数を引く関数 `env` から判定する (OS は見ない。試験と、環境を渡して判定する呼び出し側のため)。
pub fn session_env_from(env: &dyn Fn(&str) -> Option<String>) -> SessionEnv {
    SessionEnv {
        ssh: set(env, "SSH_TTY") || set(env, "SSH_CONNECTION"),
        tmux: set(env, "TMUX"),
        wsl: set(env, "WSL_DISTRO_NAME") || set(env, "WSL_INTEROP"),
    }
}

/// このプロセスの環境変数から判定する。`wsl` は Linux の実行ファイルでだけ真 (07 §3.4 の COND-2)。
#[must_use]
pub fn session_env() -> SessionEnv {
    let mut s = session_env_from(&|k| std::env::var(k).ok());
    s.wsl &= cfg!(target_os = "linux");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn それぞれの変数で判定し_空の値は無いものとする() {
        assert_eq!(session_env_from(&env(&[])), SessionEnv::default());
        for (pairs, want) in [
            (
                &[("SSH_TTY", "/dev/pts/1")][..],
                SessionEnv {
                    ssh: true,
                    ..SessionEnv::default()
                },
            ),
            (
                &[("SSH_CONNECTION", "1 2 3 4")][..],
                SessionEnv {
                    ssh: true,
                    ..SessionEnv::default()
                },
            ),
            (
                &[("TMUX", "/tmp/tmux-1000/default,1,0")][..],
                SessionEnv {
                    tmux: true,
                    ..SessionEnv::default()
                },
            ),
            (
                &[("WSL_DISTRO_NAME", "Ubuntu")][..],
                SessionEnv {
                    wsl: true,
                    ..SessionEnv::default()
                },
            ),
            (
                &[("WSL_INTEROP", "/run/WSL/1_interop")][..],
                SessionEnv {
                    wsl: true,
                    ..SessionEnv::default()
                },
            ),
            (
                &[
                    ("SSH_TTY", ""),
                    ("SSH_CONNECTION", ""),
                    ("TMUX", ""),
                    ("WSL_DISTRO_NAME", ""),
                    ("WSL_INTEROP", ""),
                ][..],
                SessionEnv::default(),
            ),
        ] {
            assert_eq!(session_env_from(&env(pairs)), want, "{pairs:?}");
        }
    }
}
