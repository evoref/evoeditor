//! 外部コマンドによるシステムクリップボード (`command` provider、docs/05 §8.2 / §8.3)。Linux と WSL で使う。
//!
//! - **検出** ([`detect`]): SSH 接続中は検出しない (外部コマンドはリモート側に書いてしまう)。Wayland は `wl-copy` / `wl-paste`、
//!   X11 は `xclip` → `xsel` を、**root しか書けないディレクトリ** (`/usr/bin`・`/bin`) に限って探す (PATH は見ない)。
//!   見つからず WSL なら、絶対パスの `powershell.exe` の `Set-Clipboard` / `Get-Clipboard` を使う (WSLg があっても `wl-copy` が
//!   無い環境のため)。`clip.exe` は使わない (UTF-16LE の BOM 付きで渡すと BOM がクリップボードに残り、BOM が無いと漢字だけの
//!   短い文を UTF-16 と判定できずに化ける。WSL で実測、2026-11-01)。
//!   検出は環境変数とファイルの確認だけの純粋な関数で、どの OS でも試験できる。
//! - **起動** ([`UnixClipboard`]): 最初の読み書きで 1 回だけ検出し (起動経路では行わない、05 §8.1)、`std::process::Command` で
//!   シェルを通さずに起動する。入出力は使い捨てのスレッドで行い、2 s で終わらなければ子プロセスを kill する。copy 側の
//!   stdout / stderr は `Stdio::null()` (`xclip` / `xsel` は常駐して受け継いだ出力を握るので、パイプだと EOF が来ない)。
//! - **文字コード**: 子プロセスとの入出力は UTF-8 (`powershell.exe` も入出力の文字コードを UTF-8 に固定する)。

/// 検出した copy / paste のコマンド (argv。シェル解釈なし)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub copy: Vec<String>,
    pub paste: Vec<String>,
}

/// 検出の結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detection {
    Command(CommandSpec),
    /// SSH 接続中 (`SSH_TTY` / `SSH_CONNECTION`)。外部コマンドは使わず、OSC 52 に任せる。
    Ssh,
    /// 画面が無い (`WAYLAND_DISPLAY` / `DISPLAY` が無く、WSL でもない)。
    Headless,
    /// 画面はあるが、使えるコマンドが無い。
    Missing,
}

impl Detection {
    /// コマンドが無い理由 (`msg.clipboard.fallback` の `{reason}`)。
    pub fn reason(&self) -> Option<&'static str> {
        match self {
            Detection::Command(_) => None,
            Detection::Ssh => Some("SSH 接続中は外部のコマンドを使いません"),
            Detection::Headless => Some("画面 (DISPLAY / WAYLAND_DISPLAY) がありません"),
            Detection::Missing => {
                Some("wl-clipboard または xclip を入れるとシステムのクリップボードが使えます")
            }
        }
    }
}

/// root しか書けないディレクトリ (05 §8.3)。
const TRUSTED_DIRS: [&str; 2] = ["/usr/bin", "/bin"];
const WSL_POWERSHELL: &str = "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe";
/// 標準入力を UTF-8 で読んでそのまま入れる (改行も変えない)。
const WSL_COPY_SCRIPT: &str = "[Console]::InputEncoding=[Text.UTF8Encoding]::new($false);      Set-Clipboard -Value ([Console]::In.ReadToEnd())";
/// `Get-Clipboard -Raw` を改行を足さずに UTF-8 で書く (`Get-Clipboard` だけでは行の配列になり、末尾に CRLF が付く)。
const WSL_PASTE_SCRIPT: &str =
    "[Console]::OutputEncoding=[Text.Encoding]::UTF8; [Console]::Out.Write((Get-Clipboard -Raw))";

fn set(env: &dyn Fn(&str) -> Option<String>, key: &str) -> bool {
    env(key).is_some_and(|v| !v.is_empty())
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

/// コマンドを選ぶ。`trusted(path)` は「root しか書けない場所にある実行ファイルか」、`exists(path)` は WSL の絶対パスの確認。
pub fn detect(
    env: &dyn Fn(&str) -> Option<String>,
    trusted: &dyn Fn(&str) -> bool,
    exists: &dyn Fn(&str) -> bool,
) -> Detection {
    if set(env, "SSH_TTY") || set(env, "SSH_CONNECTION") {
        return Detection::Ssh;
    }
    let find = |name: &str| {
        TRUSTED_DIRS
            .iter()
            .map(|d| format!("{d}/{name}"))
            .find(|p| trusted(p))
    };
    let wayland = set(env, "WAYLAND_DISPLAY");
    let x11 = set(env, "DISPLAY");
    if wayland && let (Some(c), Some(p)) = (find("wl-copy"), find("wl-paste")) {
        return Detection::Command(CommandSpec {
            copy: vec![c],
            paste: vec![
                p,
                "--no-newline".to_owned(),
                "--type".to_owned(),
                "text".to_owned(),
            ],
        });
    }
    if x11 {
        if let Some(x) = find("xclip") {
            return Detection::Command(CommandSpec {
                copy: vec![
                    x.clone(),
                    "-selection".to_owned(),
                    "clipboard".to_owned(),
                    "-in".to_owned(),
                ],
                paste: vec![
                    x,
                    "-selection".to_owned(),
                    "clipboard".to_owned(),
                    "-out".to_owned(),
                ],
            });
        }
        if let Some(x) = find("xsel") {
            return Detection::Command(CommandSpec {
                copy: vec![x.clone(), "--clipboard".to_owned(), "--input".to_owned()],
                paste: vec![x, "--clipboard".to_owned(), "--output".to_owned()],
            });
        }
    }
    let wsl = set(env, "WSL_DISTRO_NAME") || set(env, "WSL_INTEROP");
    if wsl && exists(WSL_POWERSHELL) {
        let ps = |script| {
            argv(&[
                WSL_POWERSHELL,
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
        };
        return Detection::Command(CommandSpec {
            copy: ps(WSL_COPY_SCRIPT),
            paste: ps(WSL_PASTE_SCRIPT),
        });
    }
    if wayland || x11 {
        Detection::Missing
    } else {
        Detection::Headless
    }
}

/// paste の出力を文字列にする。空なら `None`。
#[cfg(any(unix, test))]
fn decode(bytes: &[u8]) -> Option<String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    (!bytes.is_empty()).then(|| String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(unix)]
mod unix {
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::MetadataExt as _;
    use std::process::{Command, Stdio};
    use std::sync::OnceLock;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::{CommandSpec, Detection, decode, detect};
    use crate::clipboard::{ClipboardFailure, ClipboardProvider, SYSTEM_MAX_BYTES};

    /// コマンドの期限 (05 §8.3)。
    const TIMEOUT: Duration = Duration::from_secs(2);

    /// root が持ち、グループと他人が書けない通常のファイルで、実行できる。
    fn trusted(path: &str) -> bool {
        std::fs::metadata(path).is_ok_and(|m| {
            m.is_file() && m.uid() == 0 && m.mode() & 0o022 == 0 && m.mode() & 0o111 != 0
        })
    }

    fn exists(path: &str) -> bool {
        std::fs::metadata(path).is_ok_and(|m| m.is_file())
    }

    /// 子プロセスを起動し、`input` を stdin へ書き、`capture` なら stdout を読む。期限を過ぎたら kill する。
    // 入出力の使い捨てスレッドは 02 §9.1 の表のスレッド (R11 の例外。deps-policy.toml の `allowed_files`)。
    #[allow(clippy::disallowed_methods)]
    pub(super) fn run(
        argv: &[String],
        input: Option<Vec<u8>>,
        capture: bool,
        timeout: Duration,
    ) -> Result<Option<Vec<u8>>, ClipboardFailure> {
        let (prog, args) = argv.split_first().ok_or(ClipboardFailure::Unsupported)?;
        let deadline = Instant::now() + timeout;
        let mut child = Command::new(prog)
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(if capture {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| ClipboardFailure::Unavailable)?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let (tx, rx) = mpsc::channel::<Result<Vec<u8>, ClipboardFailure>>();
        let spawned = std::thread::Builder::new()
            .name("eve-clip-cmd".to_owned())
            .spawn(move || {
                if let (Some(mut w), Some(data)) = (stdin, input) {
                    // 書き終えたら閉じる (EOF を渡す)
                    if w.write_all(&data).is_err() {
                        let _ = tx.send(Err(ClipboardFailure::Failed));
                        return;
                    }
                }
                let mut out = Vec::new();
                if let Some(r) = stdout {
                    let limit = u64::try_from(SYSTEM_MAX_BYTES).unwrap_or(u64::MAX) + 1;
                    if r.take(limit).read_to_end(&mut out).is_err() {
                        let _ = tx.send(Err(ClipboardFailure::Failed));
                        return;
                    }
                    if out.len() > SYSTEM_MAX_BYTES {
                        let _ = tx.send(Err(ClipboardFailure::TooLarge));
                        return;
                    }
                }
                let _ = tx.send(Ok(out));
            });
        if spawned.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ClipboardFailure::Failed);
        }
        let io = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if Instant::now() < deadline && io.is_ok() => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => break None,
            }
        };
        let Some(status) = status else {
            // 期限切れ (または入出力が終わらない): 子を止める。入出力のスレッドはパイプが閉じて戻る
            let _ = child.kill();
            let _ = child.wait();
            return Err(ClipboardFailure::Unavailable);
        };
        let out = io.map_err(|_| ClipboardFailure::Unavailable)??;
        if !status.success() {
            // paste はクリップボードが空でも失敗を返す (`wl-paste`・`xclip -out`)。出力が無ければ「無い」とする
            return if capture && out.is_empty() {
                Ok(None)
            } else {
                Err(ClipboardFailure::Failed)
            };
        }
        Ok(Some(out))
    }

    /// Linux / WSL のシステムクリップボード。最初の読み書きでコマンドを検出する。
    #[derive(Default)]
    pub struct UnixClipboard {
        detected: OnceLock<Detection>,
    }

    impl UnixClipboard {
        pub fn new() -> UnixClipboard {
            UnixClipboard::default()
        }

        /// 検出済みの結果で作る (試験と、設定で明示したコマンド用)。
        pub fn with_detection(d: Detection) -> UnixClipboard {
            let detected = OnceLock::new();
            let _ = detected.set(d);
            UnixClipboard { detected }
        }

        fn spec(&self) -> Result<&CommandSpec, ClipboardFailure> {
            let d = self
                .detected
                .get_or_init(|| detect(&|k| std::env::var(k).ok(), &trusted, &exists));
            match d {
                Detection::Command(s) => Ok(s),
                _ => Err(ClipboardFailure::Unsupported),
            }
        }
    }

    impl ClipboardProvider for UnixClipboard {
        fn set_text(&self, text: &str) -> Result<(), ClipboardFailure> {
            if text.len() > SYSTEM_MAX_BYTES {
                return Err(ClipboardFailure::TooLarge);
            }
            let spec = self.spec()?;
            run(&spec.copy, Some(text.as_bytes().to_vec()), false, TIMEOUT).map(|_| ())
        }

        fn get_text(&self) -> Result<Option<String>, ClipboardFailure> {
            let spec = self.spec()?;
            Ok(run(&spec.paste, None, true, TIMEOUT)?
                .as_deref()
                .and_then(decode))
        }

        fn unsupported_reason(&self) -> Option<&'static str> {
            // 検出の前 (失敗を返す前) には呼ばれない。検出していなければ理由は無い
            self.detected.get().and_then(Detection::reason)
        }
    }
}

#[cfg(unix)]
pub use unix::UnixClipboard;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let m: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| m.get(k).cloned()
    }

    fn only(paths: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |p| paths.contains(&p)
    }

    fn none(_: &str) -> bool {
        false
    }

    fn first(d: &Detection) -> Option<&str> {
        match d {
            Detection::Command(s) => s.copy.first().map(String::as_str),
            _ => None,
        }
    }

    #[test]
    fn wayland_は_wl_copy_を使い_両方が無ければ_x11_へ() {
        let e = env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]);
        let all = only(&["/usr/bin/wl-copy", "/usr/bin/wl-paste", "/usr/bin/xclip"]);
        let d = detect(&e, &all, &none);
        assert_eq!(first(&d), Some("/usr/bin/wl-copy"));
        let Detection::Command(s) = &d else {
            unreachable!()
        };
        assert_eq!(
            s.paste,
            ["/usr/bin/wl-paste", "--no-newline", "--type", "text"]
        );
        // wl-paste が無い: X11 の xclip
        let d = detect(&e, &only(&["/usr/bin/wl-copy", "/usr/bin/xclip"]), &none);
        assert_eq!(first(&d), Some("/usr/bin/xclip"));
    }

    #[test]
    fn x11_は_xclip_を先に_無ければ_xsel_で_bin_も探す() {
        let e = env(&[("DISPLAY", ":0")]);
        let d = detect(&e, &only(&["/usr/bin/xclip", "/bin/xsel"]), &none);
        assert_eq!(first(&d), Some("/usr/bin/xclip"));
        let d = detect(&e, &only(&["/bin/xsel"]), &none);
        let Detection::Command(s) = &d else {
            panic!("{d:?}")
        };
        assert_eq!(s.copy, ["/bin/xsel", "--clipboard", "--input"]);
        assert_eq!(s.paste, ["/bin/xsel", "--clipboard", "--output"]);
        // DISPLAY が無ければ X11 のコマンドは使わない
        let d = detect(&env(&[]), &only(&["/usr/bin/xclip"]), &none);
        assert_eq!(d, Detection::Headless);
    }

    #[test]
    fn ssh_中は検出しない() {
        for key in ["SSH_TTY", "SSH_CONNECTION"] {
            let e = env(&[(key, "x"), ("DISPLAY", ":0"), ("WSL_DISTRO_NAME", "Ubuntu")]);
            let d = detect(&e, &|_| true, &|_| true);
            assert_eq!(d, Detection::Ssh);
            assert!(d.reason().is_some());
        }
    }

    #[test]
    fn wsl_は_powershell_を絶対パスで_wslg_でも_wl_copy_が無ければ使う() {
        let e = env(&[
            ("WSL_DISTRO_NAME", "Ubuntu"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("DISPLAY", ":0"),
        ]);
        let d = detect(&e, &none, &|_| true);
        let Detection::Command(s) = &d else {
            panic!("{d:?}")
        };
        assert_eq!(first(&d), Some(WSL_POWERSHELL));
        assert!(s.copy.last().is_some_and(|c| c.contains("Set-Clipboard")));
        assert_eq!(s.paste.first().map(String::as_str), Some(WSL_POWERSHELL));
        assert!(
            s.paste
                .last()
                .is_some_and(|c| c.contains("Get-Clipboard -Raw"))
        );
        // WSLg に wl-copy があればそちら
        let d = detect(
            &e,
            &only(&["/usr/bin/wl-copy", "/usr/bin/wl-paste"]),
            &|_| true,
        );
        assert_eq!(first(&d), Some("/usr/bin/wl-copy"));
        // Windows 側の実行ファイルが見えない (interop の無効): 画面があるので Missing
        assert_eq!(detect(&e, &none, &none), Detection::Missing);
        assert_eq!(
            detect(&env(&[("WSL_INTEROP", "/run/WSL/1_interop")]), &none, &none),
            Detection::Headless
        );
    }

    #[test]
    fn 空の環境変数は無いものとする() {
        let e = env(&[("SSH_TTY", ""), ("DISPLAY", "")]);
        assert_eq!(detect(&e, &|_| true, &none), Detection::Headless);
    }

    #[test]
    fn 出力は_utf8_で読み_空なら_none() {
        assert_eq!(
            decode(
                "あ
"
                .as_bytes()
            )
            .as_deref(),
            Some(
                "あ
"
            )
        );
        assert_eq!(decode(b""), None);
    }

    #[cfg(unix)]
    mod run {
        use std::time::Duration;

        use super::super::unix::run;
        use super::super::*;
        use crate::clipboard::{ClipboardFailure, ClipboardProvider as _};

        fn sh(script: &str) -> Vec<String> {
            vec!["/bin/sh".to_owned(), "-c".to_owned(), script.to_owned()]
        }

        #[test]
        fn copy_は_stdin_へ渡し_paste_は_stdout_を読む() {
            let dir = std::env::temp_dir().join(format!("eve-clip-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("clip");
            let f = file.display();
            let spec = CommandSpec {
                copy: sh(&format!("cat > '{f}'")),
                paste: sh(&format!("cat '{f}'")),
            };
            let c = UnixClipboard::with_detection(Detection::Command(spec));
            c.set_text("日本語\nabc😀").unwrap();
            assert_eq!(c.get_text().unwrap().as_deref(), Some("日本語\nabc😀"));
            assert_eq!(c.unsupported_reason(), None);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn 常駐する_copy_でも出力を待たずに戻る() {
            // xclip のように子が常駐して stdout / stderr を握っても、copy は null にしているので待たない
            let t = std::time::Instant::now();
            let r = run(
                &sh("cat >/dev/null; (sleep 5 &) ; exit 0"),
                Some(b"x".to_vec()),
                false,
                Duration::from_secs(2),
            );
            assert_eq!(r, Ok(Some(Vec::new())));
            assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        }

        #[test]
        fn 期限を過ぎたら止めて_unavailable() {
            let t = std::time::Instant::now();
            let r = run(&sh("sleep 10"), None, true, Duration::from_millis(200));
            assert_eq!(r, Err(ClipboardFailure::Unavailable));
            assert!(t.elapsed() < Duration::from_secs(2));
        }

        #[test]
        fn paste_の失敗は出力が無ければ_none_copy_の失敗は_failed() {
            assert_eq!(
                run(&sh("exit 1"), None, true, Duration::from_secs(2)),
                Ok(None)
            );
            assert_eq!(
                run(
                    &sh("echo partial; exit 1"),
                    None,
                    true,
                    Duration::from_secs(2)
                ),
                Err(ClipboardFailure::Failed)
            );
            assert_eq!(
                run(
                    &sh("cat >/dev/null; exit 1"),
                    Some(b"x".to_vec()),
                    false,
                    Duration::from_secs(2)
                ),
                Err(ClipboardFailure::Failed)
            );
            assert_eq!(
                run(
                    &["/nonexistent/clip".to_owned()],
                    None,
                    true,
                    Duration::from_secs(2)
                ),
                Err(ClipboardFailure::Unavailable)
            );
        }

        #[test]
        fn 検出できなければ_unsupported_で理由を返す() {
            let c = UnixClipboard::with_detection(Detection::Missing);
            assert_eq!(c.set_text("x"), Err(ClipboardFailure::Unsupported));
            assert_eq!(c.get_text(), Err(ClipboardFailure::Unsupported));
            assert!(c.unsupported_reason().is_some_and(|r| r.contains("xclip")));
        }
    }
}
