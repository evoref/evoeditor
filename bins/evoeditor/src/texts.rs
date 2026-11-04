//! 端末を復元した後に stderr へ出す文言の定数表 (docs/04 §15.2 の `stderr.*` / `panic.*` / `cli.help.*`)。言語ファイルには入れない
//! (パニックの経路で言語ファイルを引かないため)。**v0.2 から英語 (ASCII) の行を先に、日本語の行を 2 字下げで次に出す** (OQ-67、04 §14.4)。
//! パニック時の行の順は `panic.header` → 退避の結果 (`stderr.recovery.*`。`saved` / `fallback` のときは `stderr.recovery.next` も)
//! → `panic.crash_log` → `panic.report`。

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
            format!("evoeditor stopped due to an error: {reason}"),
            format!("エラーのため evoeditor を終了しました: {reason}"),
        ),
        _ => return None,
    })
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
    match (&o.dir, o.written, o.complete) {
        (Some(_), w, true) if w == total && !o.fallback => {
            lines.push(Bi::new(
                format!("Unsaved changes were saved to: {dir} ({total} documents)"),
                format!("保存されていない変更を退避しました: {dir} ({total} 件)"),
            ));
            lines.push(recovery_next());
        }
        (Some(_), w, true) if w == total => {
            lines.push(Bi::new(
                format!(
                    "Could not write to the usual location. Unsaved changes were saved to: {dir}"
                ),
                format!(
                    "いつもの場所に書けなかったため、別の場所に退避しました: {dir} (早めに別の場所へ移してください)"
                ),
            ));
            lines.push(recovery_next());
        }
        (Some(_), w, _) if w > 0 => {
            lines.push(Bi::new(
                format!("Only {w} of {total} documents could be saved to: {dir}"),
                format!("{total} 件のうち {w} 件だけを退避できました: {dir}"),
            ));
        }
        _ => {
            let reason = o.error.as_deref();
            lines.push(Bi::new(
                format!(
                    "Unsaved changes could NOT be saved: {}",
                    reason.unwrap_or("unknown reason")
                ),
                format!(
                    "保存されていない変更を退避できませんでした: {}",
                    reason.unwrap_or("理由は不明です")
                ),
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
    }
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
    const OPTIONS: [(&str, &str, &str); 14] = [
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
            "--",
            "Treat the rest as file names",
            "以降をファイル名として扱います",
        ),
    ];
    let width = OPTIONS.iter().map(|o| o.0.len()).max().unwrap_or(0) + 2;
    let mut out = String::new();
    let mut pair = |en: &str, ja: &str| out.push_str(&format!("{en}\n  {ja}\n\n"));
    pair(
        "Usage: evoeditor [OPTIONS] [--] [FILE]",
        "使い方: evoeditor [オプション] [--] [ファイル]",
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
                total: 1,
                ..RescueOutcome::default()
            },
        ] {
            all.extend(recovery_lines(&o));
        }
        for e in [
            UsageError::BadEncoding("x".into()),
            UsageError::BadEncoding(String::new()),
            UsageError::UnknownOption("--x".into()),
            UsageError::MissingValue("-e".into()),
        ] {
            all.push(usage_error(&e));
        }
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

    #[test]
    fn 退避できなかった() {
        let mut o = outcome(1, 0, false, false);
        o.dir = None;
        o.error = Some("アクセスが拒否されました".into());
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
        assert!(recovery_lines(&RescueOutcome::default()).is_empty());
        let mut o = outcome(1, 1, true, false);
        o.sensitive = true;
        let l = recovery_lines(&o);
        assert!(
            l.iter().any(|x| x.ja.contains("秘密を含むかもしれない")),
            "{l:?}"
        );
    }
}
