//! 共通の補助。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

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

/// 外部コマンドを標準入出力を引き継いで実行する。失敗したら `Err`。
pub fn run(label: &str, cmd: &mut Command) -> Result<(), String> {
    println!("▶ {label}");
    let start = Instant::now();
    let status = cmd
        .current_dir(workspace_root())
        .status()
        .map_err(|e| format!("{label}: 実行できません: {e}"))?;
    println!("  ({:.1} 秒)", start.elapsed().as_secs_f32());
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label}: 失敗しました ({status})"))
    }
}
