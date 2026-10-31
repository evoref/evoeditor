//! `cargo xtask no-network`: NFR-501 AC-2 の検査 (docs/09 §3.6 の「無通信」、v0.2、Linux)。eve-testkit の `no_network`
//! (ヘッドレスのシナリオ: 開く → 編集 → 保存 → 終了。Host の実行ループ・本物の時計とワーカースレッド) を
//! `strace -f -e trace=network` の下で走らせ、ネットワーク系のシステムコールが 0 であること (と、シナリオが成功すること) を確かめる。
//! FS は偽物 (Linux の実 FS は M7 で入る)。

use std::process::Command;

use crate::perf_diff::build_bench;
use crate::util::workspace_root;

/// strace の出力 (`-o`) から、システムコールの行だけを取り出す (`+++ exited` や `--- SIG…` の行は除く)。
fn syscalls(trace: &str) -> Vec<String> {
    trace
        .lines()
        .map(|l| {
            // `-f` では先頭に pid が付く
            let l = l.trim_start();
            l.split_once(' ')
                .filter(|(pid, _)| pid.bytes().all(|b| b.is_ascii_digit()))
                .map_or(l, |(_, rest)| rest.trim_start())
        })
        .filter(|l| !l.starts_with("+++") && !l.starts_with("---") && l.contains('('))
        .map(str::to_owned)
        .collect()
}

pub fn run() -> Result<(), String> {
    println!("▶ no-network (NFR-501 AC-2)");
    let exe = build_bench("no_network", None, None)?
        .ok_or("eve-testkit の no_network をビルドできません")?;
    let out_dir = workspace_root().join("target").join("no-network");
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let trace = out_dir.join("strace.txt");
    let status = Command::new("strace")
        .args(["-f", "-qq", "-e", "trace=network", "-o"])
        .arg(&trace)
        .arg(&exe)
        .status()
        .map_err(|e| format!("strace を実行できません (Linux で apt-get install strace): {e}"))?;
    if !status.success() {
        return Err(format!("シナリオが失敗しました ({status})"));
    }
    let text = std::fs::read_to_string(&trace).map_err(|e| format!("{}: {e}", trace.display()))?;
    let calls = syscalls(&text);
    if calls.is_empty() {
        println!("  ネットワーク系のシステムコール 0 件。OK");
        Ok(())
    } else {
        Err(format!(
            "ネットワーク系のシステムコールが {} 件あります (NFR-501):\n  - {}",
            calls.len(),
            calls
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  - ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strace_の出力からシステムコールの行だけを取り出す() {
        let t = "1234  socket(AF_INET, SOCK_STREAM, IPPROTO_IP) = 3\n1235  +++ exited with 0 +++\n--- SIGCHLD {si_signo=SIGCHLD} ---\nconnect(3, {sa_family=AF_INET}, 16) = 0\n";
        assert_eq!(
            syscalls(t),
            [
                "socket(AF_INET, SOCK_STREAM, IPPROTO_IP) = 3",
                "connect(3, {sa_family=AF_INET}, 16) = 0"
            ]
        );
        assert!(syscalls("1  +++ exited with 0 +++\n").is_empty());
    }
}
