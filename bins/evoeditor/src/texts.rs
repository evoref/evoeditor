//! 端末を復元した後に stderr へ出す文言の定数表 (docs/04 §15.2 の `stderr.*` / `panic.*` / `cli.help.*`)。言語ファイルには入れない
//! (パニックの経路で言語ファイルを引かないため)。**v0.2 から英語 (ASCII) の行を先に、日本語の行を 2 字下げで次に出す** (OQ-67、04 §14.4)。
//! パニック時の行の順は `panic.header` → 退避の結果 (`stderr.recovery.*`。確定できなかった文書があれば次に `unsynced`、
//! `saved` / `fallback` のときは最後に `stderr.recovery.next` も) → `panic.crash_log` → `panic.report`。

use std::io::Write;
use std::path::Path;

use eve_core::RescueOutcome;

use crate::cli::UsageError;

/// 英語 (ASCII) と日本語の 1 組。表示は英語の行、2 字下げの日本語の行の順。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bi {
    pub en: String,
    pub ja: String,
}

impl Bi {
    pub fn new(en: impl Into<String>, ja: impl Into<String>) -> Bi {
        Bi {
            en: en.into(),
            ja: ja.into(),
        }
    }

    /// 英語の行、日本語の行の順に書く。
    pub fn write_to(&self, w: &mut impl Write) {
        let _ = writeln!(w, "{}", self.en);
        let _ = writeln!(w, "  {}", self.ja);
    }
}

/// ログオフ・シャットダウンの画面に出す、終了を待たせている理由 (`ShutdownBlockReasonCreate`、05 §11.2)。
/// 1 行に収めるので英語と日本語を ` / ` でつなぐ。
#[cfg(windows)]
pub fn session_block_reason() -> String {
    "evoeditor: saving unsaved changes for recovery / 未保存の内容を退避しています".to_owned()
}

pub fn panic_header() -> Bi {
    Bi::new(
        "evoeditor terminated due to an unexpected error.",
        "予期しないエラーのため evoeditor を終了しました。",
    )
}

fn recovery_next() -> Bi {
    Bi::new(
        "You can recover them the next time you start evoeditor.",
        "次に evoeditor を起動したときに退避の場所を表示します。そのファイルを開いて内容を確かめてください。",
    )
}

pub fn panic_crash_log(path: &Path) -> Bi {
    let p = path.display();
    Bi::new(
        format!("Crash log: {p}  (contains no document content)"),
        format!("クラッシュログ: {p} (文書の内容は含みません)"),
    )
}

/// `stderr.quit.waiting_save`: 保存の完了を待つ前の 1 行 (04 §15.2)。
pub fn waiting_save(path: &str) -> Bi {
    Bi::new(
        format!("Waiting for the save to finish: {path}"),
        format!("保存の完了を待っています: {path}"),
    )
}

pub fn panic_report(url: &str) -> Bi {
    Bi::new(
        format!("To report this problem, attach the crash log to an issue: {url}"),
        format!("問題を報告するときは、クラッシュログを Issue に添付してください: {url}"),
    )
}

/// 終了時の行 (`Effect::Quit.stderr` と Host の起動失敗) のうち、定数表にあるもの。無ければ `None`。
pub fn stderr_line(key: &str, args: &[(String, String)]) -> Option<Bi> {
    let arg = |name: &str| {
        args.iter()
            .find(|(k, _)| k == name)
            .map_or("", |(_, v)| v.as_str())
    };
    let (reason, path, backup) = (arg("reason"), arg("path"), arg("backup"));
    // Host の起動の失敗は、理由を文言のキーと OS の詳細で渡す (STR-C2)。無ければ `reason` をそのまま使う
    let start =
        start_reason(arg("reason_key"), arg("detail")).unwrap_or_else(|| Bi::new(reason, reason));
    Some(match key {
        "stderr.quit_abort.save_failed" => Bi::new(
            format!("Saving {path} failed before exit: {reason}"),
            format!("終了の前に {path} の保存が失敗しました: {reason}"),
        ),
        "stderr.quit_abort.moved_to_backup" => Bi::new(
            format!("Saving {path} failed before exit. The original file was moved to: {backup}"),
            format!(
                "終了の前に {path} の保存が失敗し、元のファイルは {backup} に移っています。名前を元に戻してください"
            ),
        ),
        "stderr.quit_abort.in_place_failed" => Bi::new(
            format!(
                "Overwriting {path} failed before exit. The file may be damaged. Backup: {backup}"
            ),
            format!(
                "終了の前に {path} の直接上書きが失敗しました。ファイルが壊れている可能性があります。書き込む前の内容: {backup}"
            ),
        ),
        "stderr.quit_abort.in_place_failed_no_backup" => Bi::new(
            format!(
                "Overwriting {path} failed before exit. The file may be damaged, and there is no backup."
            ),
            format!(
                "終了の前に {path} の直接上書きが失敗しました。ファイルが壊れている可能性があり、バックアップはありません"
            ),
        ),
        "stderr.not_a_terminal" => Bi::new(
            "evoeditor must be run in a terminal.",
            "evoeditor は端末の中で起動してください。",
        ),
        "stderr.background_job" => Bi::new(
            "evoeditor cannot run as a background job. Start it in the foreground.",
            "evoeditor はバックグラウンドのジョブでは起動できません。前面 (フォアグラウンド) で起動してください。",
        ),
        "stderr.unsupported_console" => Bi::new(
            "This console is not supported. Please use Windows Terminal.",
            "この端末は非対応です。Windows Terminal をご利用ください。",
        ),
        "stderr.fatal" => Bi::new(
            format!("evoeditor stopped due to an error: {}", start.en),
            format!("エラーのため evoeditor を終了しました: {}", start.ja),
        ),
        _ => return None,
    })
}

/// Host の起動の失敗の理由 (`StartError::reason_key` のキー) の文。`detail` (OS の誤りの文) があれば後ろに付ける。
/// 知らないキーなら `None`。
fn start_reason(key: &str, detail: &str) -> Option<Bi> {
    let (en, ja) = match key {
        "frontend.unusable_terminal" => (
            "this terminal cannot be used (TERM=dumb)",
            "この端末では起動できません (TERM=dumb)",
        ),
        "frontend.console_output" => (
            "the console output could not be set up (VT mode)",
            "コンソールの出力を VT モードにできません",
        ),
        "frontend.console_input" => (
            "the console input could not be set up",
            "コンソールの入力を設定できません",
        ),
        "frontend.input_before_init" => (
            "internal error: input was started before initialization",
            "内部の誤り: 初期化の前に入力を始めました",
        ),
        "frontend.input_thread" => (
            "the input thread could not be started",
            "入力スレッドを起動できません",
        ),
        "frontend.other" => (
            "the terminal could not be initialized",
            "端末を初期化できません",
        ),
        "start.no_window" => (
            "the window could not be created at startup",
            "起動時に Window を作れませんでした",
        ),
        _ => return None,
    };
    Some(if detail.is_empty() {
        Bi::new(en, ja)
    } else {
        Bi::new(format!("{en}: {detail}"), format!("{ja}: {detail}"))
    })
}

/// 退避できなかった理由の文 (英語と日本語)。OS の誤りの文は OS が作ったもので、両方の行にそのまま入れる。
fn rescue_reason(e: Option<&eve_core::RescueError>) -> Bi {
    use eve_core::RescueError as E;
    match e {
        Some(E::Io(s)) => Bi::new(s.clone(), s.clone()),
        Some(E::NotWritten) => Bi::new(
            "nothing could be written in time",
            "時間内に書けませんでした",
        ),
        Some(E::WriterPanicked) => Bi::new(
            "the rescue writer stopped unexpectedly",
            "退避の書き手が異常終了しました",
        ),
        Some(E::NoWriter) => Bi::new("there is no rescue writer", "退避の書き手がありません"),
        _ => Bi::new("unknown reason", "理由は不明です"),
    }
}

/// 退避の結果の行 (`stderr.recovery.*`)。退避する文書が無かったときは空。
pub fn recovery_lines(o: &RescueOutcome) -> Vec<Bi> {
    if o.nothing_to_save() {
        return Vec::new();
    }
    let dir = o
        .dir
        .as_ref()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let total = o.total;
    let mut lines = Vec::new();
    // 「退避できた」は本文を OS に渡した文書 (`stored`) で数える (01 NFR-302 AC-5、02 §11.2.1)。fsync で確定できなかった文書
    // (`stored - written`) は `stderr.recovery.unsynced` で別に示す。ただし入出力の誤り (`Io`) で完了しなかったときは、確定した
    // 文書 (`written`) だけを数え、`unsynced` は出さない (誤りの後の、確定していない本文を「退避した」と言わない。04 §15.2)
    let io_failed = !o.complete && matches!(o.error, Some(eve_core::RescueError::Io(_)));
    let saved = if io_failed { o.written } else { o.stored };
    match (&o.dir, saved) {
        (Some(_), s) if s == total && !o.fallback => {
            lines.push(Bi::new(
                format!("Unsaved changes were saved to: {dir} ({total} documents)"),
                format!("保存されていない変更を退避しました: {dir} ({total} 件)"),
            ));
            push_unsynced(&mut lines, o);
            lines.push(recovery_next());
        }
        (Some(_), s) if s == total => {
            lines.push(Bi::new(
                format!(
                    "Could not write to the usual location. Unsaved changes were saved to: {dir}"
                ),
                format!(
                    "いつもの場所に書けなかったため、別の場所に退避しました: {dir} (早めに別の場所へ移してください)"
                ),
            ));
            push_unsynced(&mut lines, o);
            lines.push(recovery_next());
        }
        (Some(_), stored) if stored > 0 => {
            lines.push(Bi::new(
                format!("Only {stored} of {total} documents could be saved to: {dir}"),
                format!("{total} 件のうち {stored} 件だけを退避できました: {dir}"),
            ));
            push_unsynced(&mut lines, o);
        }
        _ => {
            let reason = rescue_reason(o.error.as_ref());
            lines.push(Bi::new(
                format!("Unsaved changes could NOT be saved: {}", reason.en),
                format!("保存されていない変更を退避できませんでした: {}", reason.ja),
            ));
        }
    }
    if o.sensitive && o.dir.is_some() {
        lines.push(Bi::new(
            format!(
                "The saved changes include files that may contain secrets. Delete {dir} when you no longer need it."
            ),
            format!(
                "退避した中に秘密を含むかもしれないファイルがあります。要らなくなったら {dir} を削除してください。"
            ),
        ));
    }
    lines
}

/// 退避した (OS に渡した) 文書のうち、fsync で確定できなかったものがあれば `stderr.recovery.unsynced` を足す。
fn push_unsynced(lines: &mut Vec<Bi>, o: &RescueOutcome) {
    let io_failed = !o.complete && matches!(o.error, Some(eve_core::RescueError::Io(_)));
    let count = o.stored.saturating_sub(o.written);
    if count > 0 && !io_failed {
        lines.push(Bi::new(
            format!(
                "{count} of the saved documents were not yet confirmed on disk. They may be lost if the power fails soon."
            ),
            format!(
                "退避したうち {count} 件は、ディスクへの書込の完了を確かめられませんでした。この後すぐに電源が切れると、失われることがあります。"
            ),
        ));
    }
}

/// 使い方の誤り (`stderr.usage.*`、終了コード 2) の英日 1 組。
pub fn usage_error(e: &UsageError) -> Bi {
    let (tail_en, tail_ja) = (
        "Run 'evoeditor --help' for usage.",
        "使い方は evoeditor --help で確認してください。",
    );
    match e {
        UsageError::BadEncoding(v) if !v.is_empty() => Bi::new(
            format!("Unknown encoding: {v}. Run 'evoeditor --help' for supported encodings."),
            format!(
                "{v} は対応していない文字コード名です。使える名前は evoeditor --help で確認してください。"
            ),
        ),
        UsageError::BadEncoding(_) => Bi::new(
            format!("An encoding name is required. {tail_en}"),
            format!("文字コードの名前を指定してください。{tail_ja}"),
        ),
        UsageError::UnknownOption(o) => Bi::new(
            format!("Unknown option: {o}. {tail_en}"),
            format!("未知のオプションです: {o}。{tail_ja}"),
        ),
        UsageError::MissingValue(o) => Bi::new(
            format!("{o} requires a value. {tail_en}"),
            format!("{o} には値が必要です。{tail_ja}"),
        ),
        UsageError::BadLogLevel(v) => Bi::new(
            format!("Unknown log level: {v}. Use error, warn, info, debug, or trace."),
            format!(
                "{v} はログの水準の名前ではありません。error / warn / info / debug / trace のどれかを指定してください。"
            ),
        ),
        UsageError::BadSet(v) => Bi::new(
            format!("--set needs key=value (got: {v}). {tail_en}"),
            format!("--set には key=value の形で指定してください (指定: {v})。{tail_ja}"),
        ),
        // `stderr.usage.generic` (07 §18.2: `-` は 1 つまで)
        UsageError::StdinTwice => Bi::new(
            format!("Standard input (-) can be given only once. {tail_en}"),
            format!("標準入力 (-) は 1 つだけ指定できます。{tail_ja}"),
        ),
    }
}

/// `stderr.stdin.waiting`: 標準入力の最初のデータを待つ間に、端末を初期化する前に出す (03 §9.2.2 の「端末の初期化の
/// 時期」、OQ-219 の 9。04 §15.2)。
pub fn stdin_waiting() -> Bi {
    Bi::new(
        "Waiting for standard input (Ctrl+C to abort)",
        "標準入力を待っています (Ctrl+C で中止)",
    )
}

/// `--crash-report`: クラッシュログが無い。
pub fn no_crash_log() -> Bi {
    Bi::new("No crash logs.", "クラッシュログはありません。")
}

/// `--crash-report`: 最新のクラッシュログの場所 (本文の前に出す)。
pub fn crash_log_header(path: &Path) -> Bi {
    let p = path.display();
    Bi::new(
        format!("Latest crash log: {p}"),
        format!("最新のクラッシュログ: {p}"),
    )
}

/// `--crash-report`: Issue への貼り付けの案内 (本文の後に出す。貼るかどうかは利用者が決める、09 §10.2)。
pub fn crash_report_guide(url: &str) -> Bi {
    Bi::new(
        format!(
            "If you report this, check the text above and paste it into a new issue: {url} (the log contains no document text)"
        ),
        format!(
            "報告するときは、上の内容を確かめてから Issue に貼り付けてください: {url} (ログに文書の内容は含まれません)"
        ),
    )
}

/// `--check-config` の最後の行。
pub fn check_config_summary(count: usize) -> Bi {
    if count == 0 {
        Bi::new("No problems in the settings.", "設定に問題はありません。")
    } else {
        Bi::new(
            format!("{count} problem(s) in the settings."),
            format!("設定に {count} 件の問題があります。"),
        )
    }
}

/// 無視した引数 (`info.cli.unsupported` 相当を、画面を出さずに stderr へ出す)。
pub fn ignored_args(args: &[String]) -> Bi {
    let a = args.join(" ");
    Bi::new(
        format!("Ignored arguments that are not supported in this version: {a}"),
        format!("このバージョンでは未対応の引数を無視しました: {a}"),
    )
}

/// `--help` の本文 (`cli.help.*`)。オプション名の後ろに英語の説明、次の行に日本語の説明を揃えて出す。
pub fn help() -> String {
    const OPTIONS: [(&str, &str, &str); 15] = [
        (
            "-R, --readonly",
            "Open the file read-only",
            "読み取り専用で開きます",
        ),
        (
            "-e, --encoding <label>",
            "Open the file with the given encoding",
            "文字コードを指定して開きます",
        ),
        (
            "--locale <lang>",
            "Set the UI language: ja or en (overrides ui.locale and EVE_LANG)",
            "画面の言語を指定します: ja / en (設定 ui.locale と EVE_LANG より優先)",
        ),
        (
            "-c, --config <file>",
            "Read this settings file instead of config.toml (repeatable; later files win)",
            "config.toml の代わりにこの設定ファイルを読みます (複数指定可。後のものが優先)",
        ),
        (
            "-N, --no-config",
            "Do not read config.toml or keymap.toml (use the defaults)",
            "config.toml と keymap.toml を読みません (既定値で動きます)",
        ),
        (
            "-s, --set <key>=<value>",
            "Override one setting (e.g. --set view.wrap=soft)",
            "設定を 1 項目上書きします (例: --set view.wrap=soft)",
        ),
        (
            "--log-level <level>",
            "Set the log level: error, warn, info, debug, or trace (overrides EVE_LOG)",
            "ログの水準を指定します: error / warn / info / debug / trace (EVE_LOG より優先)",
        ),
        (
            "--crash-report",
            "Show the latest crash log and exit",
            "最新のクラッシュログを表示して終了します",
        ),
        (
            "--check-config",
            "Check the settings, print the problems, and exit",
            "設定を確かめて問題を表示し、終了します",
        ),
        (
            "--dump-config [--defaults] [--json]",
            "Print the resolved settings (or the defaults) and exit",
            "解決した設定 (または既定値) を表示して終了します",
        ),
        (
            "-V, --version",
            "Show the version and build information",
            "版とビルド情報を表示して終了します",
        ),
        (
            "-h, --help",
            "Show this help",
            "このヘルプを表示して終了します",
        ),
        (
            "--wait",
            "Accepted for $EDITOR compatibility; has no effect",
            "$EDITOR に設定するときの互換用です (何もしません)",
        ),
        (
            "-",
            "Read standard input into an untitled document",
            "標準入力を読んで無題の文書として開きます",
        ),
        (
            "--",
            "Treat the rest as file names",
            "以降をファイル名として扱います",
        ),
    ];
    let width = OPTIONS.iter().map(|o| o.0.len()).max().unwrap_or(0) + 2;
    let mut out = String::new();
    let mut pair = |en: &str, ja: &str| out.push_str(&format!("{en}\n  {ja}\n\n"));
    pair(
        "Usage: evoeditor [OPTIONS] [--] [FILE | -]...",
        "使い方: evoeditor [オプション] [--] [ファイル | -]...",
    );
    pair(
        "A modeless text editor for the terminal.",
        "端末で動く、モードのないテキストエディタです。",
    );
    out.push_str("Options:\n  オプション:\n");
    for (name, en, ja) in OPTIONS {
        out.push_str(&format!("  {name:<width$}{en}\n  {:<width$}{ja}\n", ""));
    }
    out.push('\n');
    let mut pair = |en: &str, ja: &str| out.push_str(&format!("{en}\n  {ja}\n"));
    pair(
        "Encodings: utf-8, utf-8-bom, utf-16le, utf-16be, shift_jis, euc-jp, iso-2022-jp",
        "使える文字コード名: utf-8, utf-8-bom, utf-16le, utf-16be, shift_jis, euc-jp, iso-2022-jp",
    );
    pair(
        "Exit codes: 0 normal, 1 runtime error, 2 usage error, 3 errors in the settings (--check-config), 5 no terminal, 70 internal error",
        "終了コード: 0 正常、1 実行時エラー、2 使い方の誤り、3 設定にエラー (--check-config)、5 端末なし、70 内部エラー",
    );
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn outcome(total: usize, written: usize, complete: bool, fallback: bool) -> RescueOutcome {
        RescueOutcome {
            dir: Some(PathBuf::from("C:\\s\\recovery\\20261021T000000Z-1")),
            total,
            written,
            stored: written,
            complete,
            fallback,
            ..RescueOutcome::default()
        }
    }

    #[test]
    fn 英語の行は_ascii_だけ() {
        let mut all = vec![panic_header(), recovery_next(), waiting_save("a")];
        all.push(panic_crash_log(Path::new("C:\\x")));
        all.push(panic_report("https://example.com"));
        all.push(ignored_args(&["+1".into()]));
        for k in [
            "stderr.not_a_terminal",
            "stderr.background_job",
            "stderr.unsupported_console",
            "stderr.fatal",
        ] {
            all.push(stderr_line(k, &[]).unwrap());
        }
        for o in [
            outcome(2, 2, true, false),
            outcome(1, 1, true, true),
            outcome(3, 1, false, false),
            RescueOutcome {
                stored: 3,
                ..outcome(3, 1, false, false)
            },
            RescueOutcome {
                total: 1,
                ..RescueOutcome::default()
            },
        ] {
            all.extend(recovery_lines(&o));
        }
        // Host の起動の失敗の理由と、退避できなかった理由 (STR-C2: 英語の行に日本語の理由が混ざらない)
        for key in [
            "frontend.unusable_terminal",
            "frontend.console_output",
            "frontend.console_input",
            "frontend.input_before_init",
            "frontend.input_thread",
            "frontend.other",
            "start.no_window",
        ] {
            let args = [("reason_key".to_owned(), key.to_owned())];
            let b = stderr_line("stderr.fatal", &args).unwrap();
            assert!(!b.ja.ends_with(": "), "{key}: {}", b.ja);
            all.push(b);
        }
        for e in [
            eve_core::RescueError::NotWritten,
            eve_core::RescueError::WriterPanicked,
            eve_core::RescueError::NoWriter,
        ] {
            let mut o = outcome(1, 0, false, false);
            o.dir = None;
            o.error = Some(e);
            all.extend(recovery_lines(&o));
        }
        for e in [
            UsageError::BadEncoding("x".into()),
            UsageError::BadEncoding(String::new()),
            UsageError::UnknownOption("--x".into()),
            UsageError::MissingValue("-e".into()),
            UsageError::StdinTwice,
        ] {
            all.push(usage_error(&e));
        }
        all.push(stdin_waiting());
        for b in &all {
            assert!(b.en.is_ascii(), "{}", b.en);
            assert!(!b.ja.is_ascii(), "日本語の行がある: {}", b.ja);
        }
    }

    #[test]
    fn 英語の行を先に_日本語の行を_2_字下げて出す() {
        let mut buf = Vec::new();
        Bi::new("hello", "こんにちは").write_to(&mut buf);
        assert_eq!(String::from_utf8(buf).unwrap(), "hello\n  こんにちは\n");
    }

    #[test]
    fn help_の英語の行は_ascii_で_オプションの説明が揃う() {
        let h = help();
        let lines: Vec<&str> = h.lines().collect();
        assert!(lines[0].starts_with("Usage: evoeditor"), "{h}");
        for opt in [
            "--readonly",
            "--encoding",
            "--locale",
            "--version",
            "--help",
            "--wait",
            "--config",
            "--no-config",
            "--set",
            "--check-config",
            "--dump-config",
            "--log-level",
            "--crash-report",
        ] {
            assert!(h.contains(opt), "{opt}");
        }
        // 日本語を含む行は、必ず 2 字以上下げて英語の行の次に出る
        for l in &lines {
            assert!(l.is_ascii() || l.starts_with("  "), "{l}");
        }
        // 説明の列が揃う
        let col = |l: &str, s: &str| l.find(s).unwrap();
        let a = lines
            .iter()
            .find(|l| l.contains("Open the file read-only"))
            .unwrap();
        let b = lines.iter().find(|l| l.contains("Show this help")).unwrap();
        assert_eq!(col(a, "Open"), col(b, "Show"));
    }

    #[test]
    fn 退避した() {
        let l = recovery_lines(&outcome(2, 2, true, false));
        assert!(
            l[0].ja.starts_with("保存されていない変更を退避しました:"),
            "{l:?}"
        );
        assert!(l[0].ja.contains("(2 件)") && l[0].ja.contains("20261021T000000Z-1"));
        assert!(l[0].en.contains("(2 documents)"), "{l:?}");
        assert_eq!(l[1], recovery_next());
    }

    #[test]
    fn 代替の退避先() {
        let l = recovery_lines(&outcome(1, 1, true, true));
        assert!(
            l[0].ja.contains("別の場所に退避しました") && l[0].ja.contains("早めに"),
            "{l:?}"
        );
        assert!(l[0].en.starts_with("Could not write to the usual location"));
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn 一部だけ() {
        let l = recovery_lines(&outcome(3, 1, false, false));
        assert!(l[0].ja.contains("3 件のうち 1 件だけ"), "{l:?}");
        assert!(l[0].en.starts_with("Only 1 of 3 documents"), "{l:?}");
        assert_eq!(l.len(), 1, "next は付けない");
    }

    // 全文書を OS に渡したが確定できなかった文書がある (段 2 の時間切れ): 退避したと示し、確定できなかった件数を添える
    #[test]
    fn 退避したが確定できなかった文書がある() {
        let o = RescueOutcome {
            stored: 2,
            ..outcome(2, 0, false, false)
        };
        let l = recovery_lines(&o);
        assert!(
            l[0].ja.starts_with("保存されていない変更を退避しました:")
                && l[0].ja.contains("(2 件)"),
            "{l:?}"
        );
        assert!(
            l[1].ja
                .starts_with("退避したうち 2 件は、ディスクへの書込の完了を確かめられませんでした")
                && l[1]
                    .en
                    .starts_with("2 of the saved documents were not yet confirmed on disk"),
            "{l:?}"
        );
        assert_eq!(l[2], recovery_next());
        assert_eq!(l.len(), 3);
        // 一部だけ OS に渡せて、そのうち確定できなかった文書がある
        let o = RescueOutcome {
            stored: 2,
            ..outcome(3, 1, false, false)
        };
        let l = recovery_lines(&o);
        assert!(l[0].ja.contains("3 件のうち 2 件だけ"), "{l:?}");
        assert!(l[1].ja.starts_with("退避したうち 1 件は"), "{l:?}");
        assert_eq!(l.len(), 2, "next は付けない");
    }

    // 全文書を OS に渡したが、入出力の誤りで完了しなかった (fsync・最後の manifest の失敗): 「退避しました」とは言わず、確定した
    // 文書だけを数える
    #[test]
    fn 入出力の誤りで完了しなければ確定した文書だけを退避したと数える() {
        let mut o = RescueOutcome {
            stored: 2,
            ..outcome(2, 1, false, false)
        };
        o.error = Some(eve_core::RescueError::Io("fsync failed".into()));
        let l = recovery_lines(&o);
        assert!(
            !l.iter()
                .any(|x| x.ja.starts_with("保存されていない変更を退避しました")),
            "{l:?}"
        );
        assert!(l[0].ja.contains("2 件のうち 1 件だけ"), "{l:?}");
        assert_eq!(l.len(), 1, "unsynced も next も付けない: {l:?}");
        // 確定した文書が無ければ、退避できなかったと理由を示す
        o.written = 0;
        let l = recovery_lines(&o);
        assert!(
            l[0].ja.contains("退避できませんでした: fsync failed"),
            "{l:?}"
        );
        assert_eq!(l.len(), 1, "{l:?}");
    }

    #[test]
    fn 退避できなかった() {
        let mut o = outcome(1, 0, false, false);
        o.dir = None;
        o.error = Some(eve_core::RescueError::Io("アクセスが拒否されました".into()));
        let l = recovery_lines(&o);
        assert!(
            l[0].ja
                .contains("退避できませんでした: アクセスが拒否されました"),
            "{l:?}"
        );
        assert!(l[0].en.starts_with("Unsaved changes could NOT be saved"));
    }

    #[test]
    fn 文書が無ければ出さない_秘密は削除を促す() {
        let actual = recovery_lines(&RescueOutcome::default());
        assert!(actual.is_empty(), "{actual:?}");
        let mut o = outcome(1, 1, true, false);
        o.sensitive = true;
        let l = recovery_lines(&o);
        assert!(
            l.iter().any(|x| x.ja.contains("秘密を含むかもしれない")),
            "{l:?}"
        );
    }
}
