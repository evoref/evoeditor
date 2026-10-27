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

/// `root` とその子孫を強制終了する。
fn kill_tree(root: u32) {
    if cfg!(windows) {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &root.to_string()])
            .status();
    }
}
