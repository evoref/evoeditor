//! 夜間の proptest の段 (`cargo xtask nightly` の手順 `proptest`) を、時間の予算の中で回す (docs/09-testing-quality.md §6.3)。
//!
//! 予算を使い切ったら cargo-nextest を `SIGTERM` で止める (nextest は実行中の試験を止め、要約を出してから終わる。
//! 猶予の後も残っていればプロセスの木ごと `SIGKILL`)。そのときは、止める前に失敗した試験が無ければ失敗にせず
//! 「時間切れで打ち切った」と記録し、残りの段 (audit・coverage・crash・soak) に時間を回す。打ち切ったか、どの試験が
//! 失敗したかは、nextest が標準エラーに出す試験ごとの行 (`PASS [ 1.0s] (1/9) crate::bin 名前` など) から読む。

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::util::{kill_tree, terminate_tree, workspace_root};

/// proptest の段の予算 (秒。ビルドを含む)。夜間の段の合計がジョブの上限 240 分に収まるように決める (09 §6.3 の表)。
/// 2026-11-20 の夜間の実行では、この段だけで 67 分かかった。
pub const BUDGET_SECONDS: u64 = 45 * 60;

/// `SIGTERM` を送ってから、プロセスの木ごと強制終了するまでの猶予。nextest が試験を止める猶予 (既定 10 秒) より長くする。
const TERMINATE_GRACE: Duration = Duration::from_secs(60);

/// nextest が終わった後に、残りの出力を待つ上限 (孫が標準エラーを開いたまま残っても止まらないように)。
const DRAIN_LIMIT: Duration = Duration::from_secs(10);

/// 打ち切りの要約に名前を書く、止めた試験の数の上限。
const NOTE_STOPPED_MAX: usize = 6;

/// 子の終わりと出力を見に行く間隔。
const POLL: Duration = Duration::from_millis(250);

/// 段の結果。
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// すべての試験が通った。
    Passed,
    /// 予算を使い切って打ち切った。打ち切るまでに失敗した試験は無い。
    CutOff(CutOff),
    /// 失敗した (理由)。
    Failed(String),
}

/// 打ち切ったときの記録。
#[derive(Debug, PartialEq)]
pub struct CutOff {
    /// nextest が数えた試験の数 (`Starting N tests`)。
    pub total: usize,
    /// 通った試験の数。
    pub passed: usize,
    /// 打ち切った (実行中に止めた) 試験の名前と、止めるまでの秒数。
    pub stopped: Vec<(String, Option<f64>)>,
}

impl CutOff {
    /// 要約の 1 行 (`proptest: ` の後ろ)。
    /// 止めた試験は、長く回っていた順に `NOTE_STOPPED_MAX` 件まで名前を書く (同時に回る数はランナーのコア数なので、
    /// CI では 4 件ほど。手元の多コアの機械でも 1 行が長くなりすぎないように)。
    pub fn note(&self, budget: Duration) -> String {
        let mut stopped: Vec<&(String, Option<f64>)> = self.stopped.iter().collect();
        stopped.sort_by(|a, b| b.1.unwrap_or(0.0).total_cmp(&a.1.unwrap_or(0.0)));
        let mut names: Vec<String> = stopped
            .iter()
            .take(NOTE_STOPPED_MAX)
            .map(|(name, secs)| match secs {
                Some(s) => format!("{name} ({s:.0} 秒)"),
                None => name.clone(),
            })
            .collect();
        if stopped.len() > NOTE_STOPPED_MAX {
            names.push(format!("ほか {} 件", stopped.len() - NOTE_STOPPED_MAX));
        }
        format!(
            "時間切れで打ち切った (予算 {} 分。試験 {} 件のうち {} 件が通過、失敗は無し。実行中に止めた {} 件: {})",
            budget.as_secs() / 60,
            self.total,
            self.passed,
            stopped.len(),
            if names.is_empty() {
                "-".to_owned()
            } else {
                names.join(", ")
            }
        )
    }
}

/// nextest の出力から読んだ進み具合。
#[derive(Debug, Default)]
pub struct Progress {
    total: Option<usize>,
    passed: BTreeSet<String>,
    failed: BTreeSet<String>,
    stopped: Vec<(String, Option<f64>)>,
}

impl Progress {
    /// 標準エラーの 1 行を読む。`after_cutoff` は、打ち切りの `SIGTERM` を送った後の行か。
    pub fn observe(&mut self, line: &str, after_cutoff: bool) {
        let plain = strip_ansi(line);
        if let Some(total) = starting_total(&plain) {
            self.total = Some(total);
            return;
        }
        let Some(st) = status_line(&plain) else {
            return;
        };
        match st.status {
            "PASS" | "LEAK" | "FLAKY" => {
                self.passed.insert(st.name.to_owned());
            }
            // 打ち切りで止めた試験 (nextest が試験に SIGTERM を送り、残れば SIGKILL を送る)
            "SIGTERM" | "SIGKILL" if after_cutoff => {
                if !self.failed.contains(st.name) && !self.stopped.iter().any(|(n, _)| n == st.name)
                {
                    self.stopped.push((st.name.to_owned(), st.seconds));
                }
            }
            s if is_failure(s) => {
                self.failed.insert(st.name.to_owned());
            }
            _ => {}
        }
    }

    /// 段の結果を決める。`success` は nextest の終了状態、`cut_off` は予算で止めたか。
    pub fn verdict(&self, success: bool, cut_off: bool) -> Verdict {
        if success {
            return Verdict::Passed;
        }
        if !self.failed.is_empty() {
            let names: Vec<&str> = self.failed.iter().map(String::as_str).collect();
            return Verdict::Failed(format!(
                "失敗した試験 {} 件: {}",
                names.len(),
                names.join(", ")
            ));
        }
        if !cut_off {
            return Verdict::Failed(
                "nextest が失敗しました (試験の失敗の行は無い。ビルドか nextest 自身の失敗)"
                    .to_owned(),
            );
        }
        match self.total {
            Some(total) => Verdict::CutOff(CutOff {
                total,
                passed: self.passed.len(),
                stopped: self.stopped.clone(),
            }),
            // ビルドが予算の中で終わらないのは異常なので、失敗にして気づけるようにする
            None => Verdict::Failed(
                "予算の中で試験が始まりませんでした (ビルドが終わらない)".to_owned(),
            ),
        }
    }
}

/// 失敗を表す nextest の状態 (`FAIL`・`TIMEOUT`・`LEAK-FAIL`・`ABORT`・`SIGSEGV` などのシグナル)。
fn is_failure(status: &str) -> bool {
    matches!(status, "FAIL" | "TIMEOUT" | "ABORT")
        || status.ends_with("-FAIL")
        || status.starts_with("SIG")
}

/// nextest の試験ごとの行 (`   PASS [   0.059s] (1323/2554) crate::bin 名前`)。
#[derive(Debug, PartialEq)]
struct StatusLine<'a> {
    status: &'a str,
    seconds: Option<f64>,
    name: &'a str,
}

fn status_line(line: &str) -> Option<StatusLine<'_>> {
    let (status, rest) = line.trim_start().split_once(' ')?;
    if status.is_empty()
        || !status
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
    {
        return None;
    }
    let (time, rest) = rest.trim_start().strip_prefix('[')?.split_once(']')?;
    let seconds = time
        .trim()
        .trim_start_matches('>')
        .trim_end_matches('s')
        .parse()
        .ok();
    let rest = rest.trim_start();
    // 進み具合の `(1323/2554)`。SLOW の行では `(─────────)`
    let name = match rest.strip_prefix('(') {
        Some(r) => r.split_once(')')?.1,
        None => rest,
    }
    .trim();
    (!name.is_empty()).then_some(StatusLine {
        status,
        seconds,
        name,
    })
}

/// `Starting 2554 tests across 163 binaries` の試験の数。
fn starting_total(line: &str) -> Option<usize> {
    let rest = line.trim_start().strip_prefix("Starting ")?;
    let (n, rest) = rest.split_once(' ')?;
    if !rest.starts_with("test") {
        return None;
    }
    n.parse().ok()
}

/// 端末の色の指定 (`ESC [ … m`) を取り除く (nightly.yml は `CARGO_TERM_COLOR=always`)。
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // `[` と、数字・`;` の列と、終わりの英字を読み飛ばす
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `cmd` (cargo-nextest) を予算の中で回す。標準エラーはそのままログに流しながら読み、試験ごとの結果を数える。
///
/// # Errors
///
/// 子を起動できない、待てないときは `Err`。試験の失敗は `Ok(Verdict::Failed)`。
#[expect(
    clippy::disallowed_methods,
    reason = "R11 (スレッドは eve-host だけ) は製品のコードの規則で、xtask は対象外 (check-deps も xtask を除く)。\
              子の標準エラーを読む間も、予算の時刻を見張るため"
)]
pub fn run(label: &str, cmd: &mut Command, budget: Duration) -> Result<Verdict, String> {
    println!("▶ {label} (予算 {} 分)", budget.as_secs() / 60);
    let start = Instant::now();
    let mut child = cmd
        .current_dir(workspace_root())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{label}: 実行できません: {e}"))?;
    let stderr = child
        .stderr
        .take()
        .expect("標準エラーは Stdio::piped() にした");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || echo_lines(stderr, &tx));

    let mut progress = Progress::default();
    let mut cut_at: Option<Instant> = None;
    let mut killed = false;
    let status: ExitStatus = loop {
        while let Ok(line) = rx.try_recv() {
            progress.observe(&line, cut_at.is_some());
        }
        if let Some(st) = child
            .try_wait()
            .map_err(|e| format!("{label}: 待てません: {e}"))?
        {
            break st;
        }
        match cut_at {
            None if start.elapsed() >= budget => {
                eprintln!(
                    "{label}: 予算 {} 分を使い切ったので打ち切ります (SIGTERM)",
                    budget.as_secs() / 60
                );
                terminate_tree(child.id());
                cut_at = Some(Instant::now());
            }
            Some(at) if !killed && at.elapsed() >= TERMINATE_GRACE => {
                eprintln!(
                    "{label}: SIGTERM の後 {} 秒たっても終わらないので、プロセスの木ごと止めます",
                    TERMINATE_GRACE.as_secs()
                );
                kill_tree(child.id());
                killed = true;
            }
            _ => {}
        }
        match rx.recv_timeout(POLL) {
            Ok(line) => progress.observe(&line, cut_at.is_some()),
            Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(POLL),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    // 残りの出力 (nextest の最後の要約) を読む
    let drain_end = Instant::now() + DRAIN_LIMIT;
    while let Ok(line) = rx.recv_timeout(drain_end.saturating_duration_since(Instant::now())) {
        progress.observe(&line, cut_at.is_some());
    }
    println!("  ({:.1} 秒)", start.elapsed().as_secs_f32());
    Ok(progress.verdict(status.success(), cut_at.is_some()))
}

/// 子の標準エラーを 1 行ずつ自分の標準エラーに写し (ログに残す)、読んだ行を `tx` に送る。
fn echo_lines(stderr: impl Read, tx: &mpsc::Sender<String>) {
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let _ = std::io::stderr().write_all(&buf);
                // 受け手が先に終わっても、ログへの写しは続ける
                let _ = tx.send(String::from_utf8_lossy(&buf).trim_end().to_owned());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// nextest 0.9.148 の出力 (2026-11-20 の夜間の実行のログと、手元で SIGTERM を送ったときの出力) から。
    const PASS: &str = "        \u{1b}[32;1mPASS\u{1b}[0m [   0.059s] (1323/2554) \u{1b}[35;1meve-core::search_callers\u{1b}[0m 次へを待つ";
    const SLOW: &str = "        SLOW [>300.000s] (─────────) eve-core::fuzz_regex_search 複合の検索語でも参照と一致する";
    const FAIL: &str = "        FAIL [   0.010s] (2/4) nbexp::t fails";
    const SIGTERM: &str = "     SIGTERM [ 512.743s] (4/4) eve-text::storage 逆順の範囲は空になる";
    const STARTING: &str = "    Starting 2554 tests across 163 binaries (12 tests skipped)";

    #[test]
    fn 試験ごとの行を読む() {
        assert_eq!(
            status_line(&strip_ansi(PASS)),
            Some(StatusLine {
                status: "PASS",
                seconds: Some(0.059),
                name: "eve-core::search_callers 次へを待つ",
            })
        );
        assert_eq!(
            status_line(SLOW),
            Some(StatusLine {
                status: "SLOW",
                seconds: Some(300.0),
                name: "eve-core::fuzz_regex_search 複合の検索語でも参照と一致する",
            })
        );
        assert_eq!(status_line(SIGTERM).map(|s| s.seconds), Some(Some(512.743)));
        // 試験の行でないもの
        assert_eq!(status_line("   Compiling libc v0.2.191"), None);
        assert_eq!(status_line(STARTING), None);
        assert_eq!(
            status_line("     Summary [3975.209s] 2554 tests run: 2554 passed"),
            None
        );
        assert_eq!(status_line("────────────"), None);
        assert_eq!(starting_total(STARTING), Some(2554));
        assert_eq!(starting_total("    Starting the build"), None);
    }

    #[test]
    fn 打ち切らずに通れば成功() {
        let mut p = Progress::default();
        p.observe(STARTING, false);
        p.observe(PASS, false);
        assert_eq!(p.verdict(true, false), Verdict::Passed);
    }

    #[test]
    fn 打ち切った試験だけなら失敗にしない() {
        let mut p = Progress::default();
        p.observe(STARTING, false);
        p.observe(PASS, false);
        p.observe(SLOW, false);
        // 打ち切りの後の行と、nextest の最後の要約で繰り返される行
        p.observe(SIGTERM, true);
        p.observe(SIGTERM, true);
        let Verdict::CutOff(c) = p.verdict(false, true) else {
            panic!("打ち切りになるはず: {:?}", p.verdict(false, true));
        };
        assert_eq!(c.total, 2554);
        assert_eq!(c.passed, 1);
        assert_eq!(
            c.stopped,
            [(
                "eve-text::storage 逆順の範囲は空になる".to_owned(),
                Some(512.743)
            )]
        );
        assert_eq!(
            c.note(Duration::from_secs(45 * 60)),
            "時間切れで打ち切った (予算 45 分。試験 2554 件のうち 1 件が通過、失敗は無し。実行中に止めた 1 件: \
             eve-text::storage 逆順の範囲は空になる (513 秒))"
        );
    }

    #[test]
    fn 要約に書く止めた試験は長い順に上限まで() {
        let c = CutOff {
            total: 100,
            passed: 90,
            stopped: (1..=8)
                .map(|i| (format!("t{i}"), Some(f64::from(i))))
                .collect(),
        };
        let note = c.note(Duration::from_secs(60));
        assert!(
            note.ends_with(
                "実行中に止めた 8 件: t8 (8 秒), t7 (7 秒), t6 (6 秒), t5 (5 秒), t4 (4 秒), t3 (3 秒), ほか 2 件)"
            ),
            "{note}"
        );
    }

    #[test]
    fn 打ち切る前に失敗した試験があれば失敗() {
        let mut p = Progress::default();
        p.observe(STARTING, false);
        p.observe(FAIL, false);
        p.observe(SIGTERM, true);
        p.observe(FAIL, true);
        assert_eq!(
            p.verdict(false, true),
            Verdict::Failed("失敗した試験 1 件: nbexp::t fails".to_owned())
        );
    }

    #[test]
    fn 打ち切る前のシグナルでの終わりは失敗() {
        // 打ち切りの前に SIGTERM で終わった試験は、打ち切りのせいではない
        let mut p = Progress::default();
        p.observe(STARTING, false);
        p.observe(SIGTERM, false);
        assert!(matches!(p.verdict(false, true), Verdict::Failed(_)));
        assert!(matches!(p.verdict(false, false), Verdict::Failed(_)));
    }

    #[test]
    fn 試験が始まる前に打ち切ったら失敗() {
        let p = Progress::default();
        assert_eq!(
            p.verdict(false, true),
            Verdict::Failed("予算の中で試験が始まりませんでした (ビルドが終わらない)".to_owned())
        );
    }

    #[test]
    fn 失敗の行が無い失敗はビルドか_nextest_の失敗() {
        let mut p = Progress::default();
        p.observe(STARTING, false);
        assert!(matches!(p.verdict(false, false), Verdict::Failed(m) if m.contains("ビルド")));
    }

    #[test]
    fn 失敗を表す状態() {
        for s in [
            "FAIL",
            "TIMEOUT",
            "ABORT",
            "LEAK-FAIL",
            "SIGSEGV",
            "SIGABRT",
        ] {
            assert!(is_failure(s), "{s}");
        }
        for s in ["PASS", "SLOW", "LEAK", "SKIP", "START"] {
            assert!(!is_failure(s), "{s}");
        }
    }

    #[test]
    fn 色の指定を取り除く() {
        assert_eq!(strip_ansi("\u{1b}[32;1mPASS\u{1b}[0m x"), "PASS x");
        assert_eq!(strip_ansi("色なし"), "色なし");
    }

    /// 実際に子を予算で止める (Unix だけ。`sh` の子が `sleep` の孫を持つ)。打ち切りの経路が子と孫を確実に止めることを確かめる。
    #[cfg(unix)]
    #[test]
    fn 予算を使い切ったら子と孫を止める() {
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "echo '    Starting 2 tests across 1 binary' >&2; \
             echo '        PASS [   0.010s] (1/2) x::t a' >&2; \
             sleep 300 & wait",
        ]);
        let t = Instant::now();
        let v = run("試し", &mut cmd, Duration::from_secs(1)).unwrap();
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "止まるまでに {:?}",
            t.elapsed()
        );
        // sh は SIGTERM で終わり、試験の行に SIGTERM は無いので、止めた試験は 0 件
        assert_eq!(
            v,
            Verdict::CutOff(CutOff {
                total: 2,
                passed: 1,
                stopped: Vec::new(),
            })
        );
    }
}
