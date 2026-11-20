//! 共通の補助。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// リポジトリのルート (`xtask/` の 1 つ上)。
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask は workspace のルート直下にある")
        .to_path_buf()
}

/// cargo のビルドの出力先 (`cargo metadata` の `target_directory`)。
/// 環境変数 `CARGO_TARGET_DIR` と設定の `build.target-dir` を反映する。求められなければ `<リポジトリ>/target`。
/// ビルドの成果物 (`release/evoeditor.exe` など) を探すときに使う。xtask 自身の出力 (`target/dist` など) の置き場所は変えない。
pub fn cargo_target_dir() -> PathBuf {
    run_capture(Command::new("cargo").args(["metadata", "--format-version", "1", "--no-deps"]))
        .ok()
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|v| v["target_directory"].as_str().map(PathBuf::from))
        .unwrap_or_else(|| workspace_root().join("target"))
}

/// UTF-8 のテキストファイルを読む。改行は LF に揃える (CRLF の作業ツリーでも同じ結果にする)。
pub fn read_text(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map(|s| s.replace("\r\n", "\n"))
        .map_err(|e| format!("{} を読めません: {e}", path.display()))
}

/// `dir` の下の `*.rs` ファイルを再帰的に集める (`target` と隠しディレクトリは除く)。
pub fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name != "target" && !name.starts_with('.') {
                out.extend(rust_files(&path));
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// 外部コマンドを実行し、標準出力を文字列で返す。失敗したら標準エラーを添えて `Err`。
pub fn run_capture(cmd: &mut Command) -> Result<String, String> {
    let out = cmd
        .current_dir(workspace_root())
        .output()
        .map_err(|e| format!("{cmd:?}: 実行できません: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd:?}: 失敗しました ({})\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("{cmd:?}: 出力が UTF-8 ではありません: {e}"))
}

/// 1 ステップの時間の上限 (秒)。環境変数 `XTASK_STEP_TIMEOUT_SECS` で変えられる。
/// 超えたら子を強制終了して失敗にする (子が何も出力せず止まったとき、`xtask check` が終わらなくなるのを防ぐ)。
fn step_timeout() -> Duration {
    let secs = std::env::var("XTASK_STEP_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20 * 60);
    Duration::from_secs(secs)
}

/// 外部コマンドを標準入出力を引き継いで実行する。失敗したら `Err`。時間の上限 (`step_timeout`) を超えたら
/// 子のプロセスの木を表示して強制終了し、`Err` にする。
pub fn run(label: &str, cmd: &mut Command) -> Result<(), String> {
    println!("▶ {label}");
    let start = Instant::now();
    let mut child = cmd
        .current_dir(workspace_root())
        .spawn()
        .map_err(|e| format!("{label}: 実行できません: {e}"))?;
    let limit = step_timeout();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() >= limit => {
                eprintln!(
                    "{label}: {} 秒たっても終わりません。子のプロセスの状況:",
                    limit.as_secs()
                );
                dump_processes(child.id());
                kill_tree(child.id());
                let _ = child.wait();
                return Err(format!(
                    "{label}: 時間切れで中断しました ({} 秒。XTASK_STEP_TIMEOUT_SECS で変えられます)",
                    limit.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => return Err(format!("{label}: 待てません: {e}")),
        }
    };
    println!("  ({:.1} 秒)", start.elapsed().as_secs_f32());
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label}: 失敗しました ({status})"))
    }
}

/// `root` の子孫のプロセスを表示する (どこで止まったかを後から調べるため。Windows のみ。失敗しても何もしない)。
fn dump_processes(root: u32) {
    if !cfg!(windows) {
        return;
    }
    let script = format!(
        "$all = Get-CimInstance Win32_Process;          function Kids($id, $d) {{ $all | Where-Object ParentProcessId -eq $id | ForEach-Object {{            ('  ' * $d) + \"$($_.ProcessId) $($_.Name) $($_.CreationDate) \" + ($_.CommandLine + '').Substring(0, [Math]::Min(140, ($_.CommandLine + '').Length));            Kids $_.ProcessId ($d + 1) }} }};          Kids {root} 1"
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status();
}

/// `root` とその子孫を強制終了する。Unix では、先に木の全体を `SIGSTOP` で止めてから (止めている間は子を増やせない)
/// `SIGKILL` を送る。子孫を親子の関係でたどれるのは Linux だけ (`/proc`)。他の Unix では `root` だけを止める。
pub fn kill_tree(root: u32) {
    if cfg!(windows) {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &root.to_string()])
            .status();
        return;
    }
    let mut pids = process_tree(root);
    send_signal("STOP", &pids);
    // 止める前に増えた子を拾い直す (止めた親はもう子を増やせない)
    for pid in process_tree(root) {
        if !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    send_signal("STOP", &pids);
    send_signal("KILL", &pids);
}

/// `root` とその子孫に `SIGTERM` を送り、行儀よく終わらせる (cargo-nextest は実行中の試験を止め、要約を出してから終わる)。
/// 子孫にも送るのは、`root` が先に終わって子 (ビルド中の cargo・rustc) が残らないようにするため。
/// Windows には相当するものが無いので `kill_tree` と同じ。
pub fn terminate_tree(root: u32) {
    if cfg!(windows) {
        kill_tree(root);
    } else {
        send_signal("TERM", &process_tree(root));
    }
}

/// `kill -<signal> <pid>…` (Unix)。消えたプロセスへの送信の失敗は無視する。
fn send_signal(signal: &str, pids: &[u32]) {
    if pids.is_empty() {
        return;
    }
    let _ = Command::new("kill")
        .arg(format!("-{signal}"))
        .args(pids.iter().map(u32::to_string))
        .stderr(std::process::Stdio::null())
        .status();
}

/// `root` と、その子孫のプロセス ID (根から順)。子孫を求められない OS では `root` だけ。
fn process_tree(root: u32) -> Vec<u32> {
    #[cfg(target_os = "linux")]
    {
        let mut pairs = Vec::new();
        for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse().ok()) else {
                continue;
            };
            if let Some(ppid) = fs::read_to_string(entry.path().join("stat"))
                .ok()
                .and_then(|stat| parent_pid(&stat))
            {
                pairs.push((pid, ppid));
            }
        }
        descendants(root, &pairs)
    }
    #[cfg(not(target_os = "linux"))]
    {
        vec![root]
    }
}

/// `/proc/<pid>/stat` の親のプロセス ID (4 つ目の欄)。2 つ目の欄 (コマンド名) は括弧で囲まれ、空白や括弧を含みうるので、
/// 最後の `)` の後から数える。
#[cfg(any(target_os = "linux", test))]
fn parent_pid(stat: &str) -> Option<u32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// (プロセス, 親) の組から、`root` とその子孫を根から順に集める。
#[cfg(any(target_os = "linux", test))]
fn descendants(root: u32, pairs: &[(u32, u32)]) -> Vec<u32> {
    let mut out = vec![root];
    let mut i = 0;
    while let Some(&parent) = out.get(i) {
        for &(pid, ppid) in pairs {
            if ppid == parent && !out.contains(&pid) {
                out.push(pid);
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_から親を読む() {
        assert_eq!(parent_pid("42 (cargo) S 7 42 42 0 -1"), Some(7));
        // コマンド名に空白と括弧があっても、最後の `)` の後から数える
        assert_eq!(parent_pid("43 (a b) (c)) R 9 43 43 0 -1"), Some(9));
        assert_eq!(parent_pid("壊れた行"), None);
    }

    #[test]
    fn 子孫を根から順に集める() {
        // 1 ─ 2 ─ 4
        //   └ 3     5 (無関係) ─ 6
        let pairs = [(2, 1), (3, 1), (4, 2), (5, 0), (6, 5)];
        assert_eq!(descendants(1, &pairs), [1, 2, 3, 4]);
        assert_eq!(descendants(4, &pairs), [4]);
        assert_eq!(descendants(9, &pairs), [9]);
    }
}
