//! コマンドラインの解析 (docs/07 §18.1〜§18.3。v0.1 の公開オプションは `--version` `--help` `--readonly` `--encoding` `--`
//! と、受理して無視する `--wait`。v0.2 で `--locale` を足した。v0.4 の M25 で標準入力の `-` を足した)。パーサは外部 crate を使わず、この小さな関数で済ませる (サブコマンドが要る v1.X で見直す、OQ-24)。

use std::ffi::OsString;

/// 開くファイル 1 つ (07 §18.2)。標準入力 (`-`、v0.4 の M25) は `stdin` が真で、`path` は `-`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileArg {
    pub path: String,
    /// 標準入力 (`-`。`--` の後ろの `-` も、03 §9.2.2 の「書き方」)。`-` という名前のファイルは `./-` と書く。
    pub stdin: bool,
    /// 開いた直後に移る行 (`+LINE` / `FILE:LINE`、1 始まり)。`+` 単独は最終行 (`usize::MAX`、範囲外は末尾に丸める)。
    pub line: Option<usize>,
    /// 同じく桁 (書記素数、1 始まり)。
    pub col: Option<usize>,
}

impl FileArg {
    fn plain(path: String) -> FileArg {
        FileArg {
            path,
            stdin: false,
            line: None,
            col: None,
        }
    }

    fn stdin() -> FileArg {
        FileArg {
            stdin: true,
            ..FileArg::plain("-".to_owned())
        }
    }
}

/// 設定の読込元の指定 (07 §18.3 の `--config` / `--no-config` / `--set`)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigArgs {
    /// `--config <file>`: ユーザー層の `config.toml` の代わりに、この順に重ねる。
    pub files: Vec<String>,
    /// `--no-config`: ユーザー層 (`config.toml` と `keymap.toml`) を読まない。
    pub no_config: bool,
    /// `--set <key>=<value>`: コマンドライン層。最後に重ねる。
    pub sets: Vec<String>,
}

/// 端末を使わずに終える動作 (07 §18.3)。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    /// 編集する (通常)。
    #[default]
    Edit,
    /// `--check-config`: 設定を読んで診断を出し、終える。
    CheckConfig,
    /// `--dump-config [--defaults] [--json]`: 解決済みの設定 (または既定値) を出し、終える。
    DumpConfig { defaults: bool, json: bool },
    /// `--crash-report`: 最新のクラッシュログを表示して終える (09 §10.2)。
    CrashReport,
    /// `--licenses` (隠し): 同梱の第三者告知を表示して終える (05 §17.1)。
    Licenses,
    /// `--list-commands [--json]` (隠し): 組込みのコマンドの一覧を出し、終える (07 §11.5、09 §11.2)。
    ListCommands { json: bool },
    /// `--dump-keymap [--json]` (隠し): 解決後のキーマップを出し、終える (07 §7.7、09 §11.2)。
    DumpKeymap { json: bool },
}

/// 解析の結果。
#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    Help,
    Version,
    Run {
        /// 開くファイル (0 個なら無題。複数は v0.2、最初のものを表示する)。標準入力 (`-`) は 1 つまで。
        files: Vec<FileArg>,
        read_only: bool,
        /// `--encoding` の値 (開く文書の文字コードの指定)。
        encoding: Option<String>,
        /// `--locale` の値 (UI の言語。選択順の最上位、07 §17.1)。同梱の言語に当たるかは `eve_config::select_locale` が見る。
        locale: Option<String>,
        /// `+LINE` など、このバージョンでは未対応のため無視した引数 (`info.cli.unsupported` の対象)。
        ignored: Vec<String>,
        config: ConfigArgs,
        mode: Mode,
        /// `--log-level <level>`: `EVE_LOG` より優先する。値の妥当性は main が確かめる。
        log_level: Option<String>,
    },
}

/// 使い方の誤り (終了コード 2、07 §18.4)。
#[derive(Debug, PartialEq, Eq)]
pub enum UsageError {
    UnknownOption(String),
    MissingValue(String),
    /// 文字コードのラベルが空など。
    BadEncoding(String),
    /// `--set` の値が `key=value` の形でない。
    BadSet(String),
    /// `--log-level` の値が水準の名前でない。
    BadLogLevel(String),
    /// 標準入力 (`-`) を 2 つ以上書いた (07 §18.2。`stderr.usage.generic`)。
    StdinTwice,
}

fn to_string(a: OsString) -> Result<String, UsageError> {
    a.into_string()
        .map_err(|o| UsageError::UnknownOption(o.to_string_lossy().into_owned()))
}

/// `+LINE[:COL]` / `+` を (行, 桁) にする。`+` 単独は最終行。数字でない (`+/PATTERN` など) は `None`。
fn parse_plus(s: &str) -> Option<(usize, Option<usize>)> {
    let body = s.strip_prefix('+')?;
    if body.is_empty() {
        return Some((usize::MAX, None));
    }
    let (line, col) = match body.split_once(':') {
        Some((l, c)) => (l, Some(c)),
        None => (body, None),
    };
    let digits =
        |t: &str| (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit())).then(|| t.parse().ok());
    let line = digits(line)??;
    let col = match col {
        Some(c) => Some(digits(c)??),
        None => None,
    };
    Some((line, col))
}

/// 末尾の `:数字` または `:数字:数字` を剥がす: (残り, 行, 桁)。
fn strip_position(s: &str) -> Option<(&str, usize, Option<usize>)> {
    let num = |t: &str| {
        (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
            .then(|| t.parse::<usize>().ok())
            .flatten()
    };
    let (rest, last) = s.rsplit_once(':')?;
    let last = num(last)?;
    if let Some((base, mid)) = rest.rsplit_once(':')
        && let Some(line) = num(mid)
    {
        return Some((base, line, Some(last)));
    }
    Some((rest, last, None))
}

/// `FILE:LINE[:COL]` の曖昧性の規則 (07 §18.2): 全体が存在するファイルなら位置指定なしで開く → 末尾の位置を
/// 剥がしたものが存在すれば位置指定とみなす → どちらも無ければ全体を新規ファイル名とする。
/// Windows の `C:\path` のドライブレターの `:` は、後ろが数字でないので位置とは読まれない。
fn split_position(raw: &str, exists: &dyn Fn(&str) -> bool) -> FileArg {
    // 存在確認 (ファイルシステムへのアクセス) は、末尾が位置の形のときだけ行う。到達できない UNC パスなどを
    // 引数に渡されても、端末を始める前にここでブロックしない (読込はワーカーで行う、NFR-502 AC-2)
    if let Some((base, line, col)) = strip_position(raw)
        && !base.is_empty()
        && !exists(raw)
        && exists(base)
    {
        return FileArg {
            path: base.to_owned(),
            stdin: false,
            line: Some(line),
            col,
        };
    }
    FileArg::plain(raw.to_owned())
}

/// `args` は実行ファイル名を除いた引数。`exists` は `FILE:LINE` の曖昧性の解消に使うファイルの存在確認。
pub fn parse(
    args: impl IntoIterator<Item = OsString>,
    exists: &dyn Fn(&str) -> bool,
) -> Result<Cli, UsageError> {
    let mut it = args.into_iter();
    let mut files: Vec<FileArg> = Vec::new();
    // `+LINE[:COL]`: 直後の FILE に適用する。FILE の後に書かれたもの (`file +10`) は、最後の FILE に適用する
    let mut pending: Option<(usize, Option<usize>)> = None;
    let mut read_only = false;
    let mut encoding = None;
    let mut locale = None;
    let mut ignored = Vec::new();
    let mut config = ConfigArgs::default();
    let (mut check, mut dump, mut defaults, mut json) = (false, false, false, false);
    let (mut crash_report, mut licenses) = (false, false);
    let (mut list_commands, mut dump_keymap) = (false, false);
    let mut log_level = None;
    let mut only_positional = false;
    // `-` は標準入力 (`--` の後ろでも)。`+LINE` はファイルと同じく直後 (後置なら直前) の `-` に掛かる (07 §18.2)
    let add_file =
        |files: &mut Vec<FileArg>, raw: String, pending: &mut Option<(usize, Option<usize>)>| {
            let mut f = if raw == "-" {
                FileArg::stdin()
            } else {
                split_position(&raw, exists)
            };
            if f.line.is_none()
                && let Some((line, col)) = pending.take()
            {
                f.line = Some(line);
                f.col = col;
            }
            files.push(f);
        };
    while let Some(a) = it.next() {
        let a = to_string(a)?;
        if only_positional {
            add_file(&mut files, a, &mut pending);
            continue;
        }
        match a.as_str() {
            "--" => only_positional = true,
            "--help" | "-h" => return Ok(Cli::Help),
            "--version" | "-V" => return Ok(Cli::Version),
            "--readonly" | "-R" => read_only = true,
            // 互換オプション: 受理して無視する (`$EDITOR="evoeditor --wait"`)
            "--wait" => {}
            // 計測用ビルドだけの隠しオプション (docs/09 §5.2)。配布ビルドでは未知のオプションとして終了コード 2
            #[cfg(feature = "bench-trace")]
            "--bench-exit-after-first-frame" => eve_tui::trace::request_exit_after_first_frame(),
            "--encoding" | "-e" => {
                let v = it
                    .next()
                    .ok_or_else(|| UsageError::MissingValue(a.clone()))
                    .and_then(to_string)?;
                if v.is_empty() {
                    return Err(UsageError::BadEncoding(v));
                }
                encoding = Some(v);
            }
            "--locale" => {
                locale = Some(
                    it.next()
                        .ok_or_else(|| UsageError::MissingValue(a.clone()))
                        .and_then(to_string)?,
                );
            }
            s if s.starts_with("--locale=") => locale = Some(s["--locale=".len()..].to_owned()),
            "--config" | "-c" => config.files.push(
                it.next()
                    .ok_or_else(|| UsageError::MissingValue(a.clone()))
                    .and_then(to_string)?,
            ),
            s if s.starts_with("--config=") => config.files.push(s["--config=".len()..].to_owned()),
            "--no-config" | "-N" => config.no_config = true,
            "--set" | "-s" => {
                let v = it
                    .next()
                    .ok_or_else(|| UsageError::MissingValue(a.clone()))
                    .and_then(to_string)?;
                config.sets.push(checked_set(v)?);
            }
            s if s.starts_with("--set=") => config
                .sets
                .push(checked_set(s["--set=".len()..].to_owned())?),
            "--check-config" => check = true,
            "--dump-config" => dump = true,
            "--defaults" => defaults = true,
            "--crash-report" => crash_report = true,
            "--licenses" => licenses = true,
            // 生成リファレンスと 07 §11.6 のカタログとの突合の入力 (隠し。`--help` に出さない、07 §18.3)
            "--list-commands" => list_commands = true,
            "--dump-keymap" => dump_keymap = true,
            "--log-level" => {
                log_level = Some(
                    it.next()
                        .ok_or_else(|| UsageError::MissingValue(a.clone()))
                        .and_then(to_string)?,
                );
            }
            s if s.starts_with("--log-level=") => {
                log_level = Some(s["--log-level=".len()..].to_owned());
            }
            "--json" => json = true,
            s if s.starts_with("--encoding=") => {
                let v = s["--encoding=".len()..].to_owned();
                if v.is_empty() {
                    return Err(UsageError::BadEncoding(v));
                }
                encoding = Some(v);
            }
            // `+LINE[:COL]` / `+`。読めない形 (`+/PATTERN` など) は未対応として無視する (ファイル名として開かない)
            s if s.starts_with('+') => match parse_plus(s) {
                Some(pos) => pending = Some(pos),
                None => ignored.push(s.to_owned()),
            },
            s if s.starts_with('-') && s != "-" => return Err(UsageError::UnknownOption(a)),
            _ => add_file(&mut files, a, &mut pending),
        }
    }
    // 標準入力は 1 つまで (07 §18.2)。駆動 API (`--headless --rpc stdio`) は標準入力を使うので併用できないが、配布ビルドの
    // この解析は `--headless` を未知のオプションとして先に拒む
    if files.iter().filter(|f| f.stdin).count() > 1 {
        return Err(UsageError::StdinTwice);
    }
    // FILE の後ろに書かれた `+LINE` は最後の FILE に適用する
    if let Some((line, col)) = pending
        && let Some(last) = files.last_mut()
        && last.line.is_none()
    {
        last.line = Some(line);
        last.col = col;
    }
    // `--defaults` は `--dump-config` の修飾だけ。`--json` は `--dump-config` / `--list-commands` / `--dump-keymap` の修飾
    if defaults && !dump {
        return Err(UsageError::UnknownOption("--defaults".to_owned()));
    }
    if json && !(dump || list_commands || dump_keymap) {
        return Err(UsageError::UnknownOption("--json".to_owned()));
    }
    let mode = if licenses {
        Mode::Licenses
    } else if crash_report {
        Mode::CrashReport
    } else if check {
        Mode::CheckConfig
    } else if dump {
        Mode::DumpConfig { defaults, json }
    } else if list_commands {
        Mode::ListCommands { json }
    } else if dump_keymap {
        Mode::DumpKeymap { json }
    } else {
        Mode::Edit
    };
    Ok(Cli::Run {
        files,
        read_only,
        encoding,
        locale,
        ignored,
        config,
        mode,
        log_level,
    })
}

/// `--set` の値が `key=value` の形か (キーが空でないこと)。値の妥当性は設定の読込が診断する。
fn checked_set(v: String) -> Result<String, UsageError> {
    match v.split_once('=') {
        Some((k, _)) if !k.trim().is_empty() => Ok(v),
        _ => Err(UsageError::BadSet(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn parse_with(args: &[&str], existing: &[&str]) -> Result<Cli, UsageError> {
        let set: BTreeSet<String> = existing.iter().map(|s| (*s).to_owned()).collect();
        parse(args.iter().map(OsString::from), &|p| set.contains(p))
    }

    fn p(args: &[&str]) -> Result<Cli, UsageError> {
        parse_with(args, &[])
    }

    fn f(path: &str, line: Option<usize>, col: Option<usize>) -> FileArg {
        FileArg {
            path: path.to_owned(),
            stdin: false,
            line,
            col,
        }
    }

    /// 標準入力 (`-`)。
    fn stdin(line: Option<usize>, col: Option<usize>) -> FileArg {
        FileArg {
            line,
            col,
            ..FileArg::stdin()
        }
    }

    fn plain(path: &str) -> FileArg {
        f(path, None, None)
    }

    fn run(files: Vec<FileArg>, read_only: bool) -> Cli {
        Cli::Run {
            files,
            read_only,
            encoding: None,
            locale: None,
            ignored: vec![],
            config: ConfigArgs::default(),
            mode: Mode::Edit,
            log_level: None,
        }
    }

    // verifies: AC-FR-101-1
    #[test]
    fn 引数なしは無題_ファイルは開く() {
        assert_eq!(p(&[]).unwrap(), run(vec![], false));
        assert_eq!(p(&["a.txt"]).unwrap(), run(vec![plain("a.txt")], false));
        assert_eq!(
            p(&["日本語.txt"]).unwrap(),
            run(vec![plain("日本語.txt")], false)
        );
    }

    #[test]
    fn 複数のファイルは開いた順に並ぶ() {
        assert_eq!(
            p(&["a.txt", "b.txt", "c.txt"]).unwrap(),
            run(vec![plain("a.txt"), plain("b.txt"), plain("c.txt")], false)
        );
    }

    #[test]
    fn 公開オプション() {
        assert_eq!(p(&["--help"]).unwrap(), Cli::Help);
        assert_eq!(p(&["-h"]).unwrap(), Cli::Help);
        assert_eq!(p(&["--version"]).unwrap(), Cli::Version);
        assert_eq!(p(&["-V", "a.txt"]).unwrap(), Cli::Version);
        assert_eq!(
            p(&["--readonly", "a.txt"]).unwrap(),
            run(vec![plain("a.txt")], true)
        );
        assert_eq!(
            p(&["a.txt", "-R"]).unwrap(),
            run(vec![plain("a.txt")], true)
        );
        assert_eq!(
            p(&["--wait", "a.txt"]).unwrap(),
            run(vec![plain("a.txt")], false)
        );
    }

    #[test]
    fn encoding_は値を取る() {
        let r = p(&["--encoding", "shift_jis", "a.txt"]).unwrap();
        assert!(matches!(&r, Cli::Run { encoding: Some(e), .. } if e == "shift_jis"));
        let r = p(&["--encoding=utf-8"]).unwrap();
        assert!(matches!(&r, Cli::Run { encoding: Some(e), .. } if e == "utf-8"));
        assert_eq!(
            p(&["-e"]).unwrap_err(),
            UsageError::MissingValue("-e".into())
        );
        assert!(matches!(
            p(&["--encoding="]).unwrap_err(),
            UsageError::BadEncoding(_)
        ));
    }

    #[test]
    fn locale_は値を取る() {
        let r = p(&["--locale", "en", "a.txt"]).unwrap();
        assert!(
            matches!(&r, Cli::Run { locale: Some(l), files, .. } if l == "en" && files.len() == 1)
        );
        let r = p(&["--locale=ja-JP"]).unwrap();
        assert!(matches!(&r, Cli::Run { locale: Some(l), .. } if l == "ja-JP"));
        assert_eq!(
            p(&["--locale"]).unwrap_err(),
            UsageError::MissingValue("--locale".into())
        );
    }

    #[test]
    fn 設定の読込元のオプション() {
        let r = p(&[
            "-c",
            "a.toml",
            "--config=b.toml",
            "--set",
            "view.wrap=soft",
            "-s",
            "editor.tab_width=2",
            "x.txt",
        ])
        .unwrap();
        let Cli::Run { config, files, .. } = r else {
            panic!()
        };
        assert_eq!(config.files, ["a.toml", "b.toml"]);
        assert_eq!(config.sets, ["view.wrap=soft", "editor.tab_width=2"]);
        assert!(!config.no_config);
        assert_eq!(files.len(), 1);
        let Cli::Run { config, .. } = p(&["-N"]).unwrap() else {
            panic!()
        };
        assert!(config.no_config);
        assert_eq!(
            p(&["--set", "novalue"]).unwrap_err(),
            UsageError::BadSet("novalue".into())
        );
        assert_eq!(
            p(&["--set=", "a"]).unwrap_err(),
            UsageError::BadSet("".into())
        );
        assert_eq!(
            p(&["--config"]).unwrap_err(),
            UsageError::MissingValue("--config".into())
        );
    }

    #[test]
    fn 端末を使わない動作() {
        let mode = |args: &[&str]| match p(args).unwrap() {
            Cli::Run { mode, .. } => mode,
            _ => panic!(),
        };
        assert_eq!(mode(&["--check-config"]), Mode::CheckConfig);
        assert_eq!(
            mode(&["--dump-config", "--defaults", "--json"]),
            Mode::DumpConfig {
                defaults: true,
                json: true
            }
        );
        assert_eq!(mode(&["--crash-report"]), Mode::CrashReport);
        assert_eq!(mode(&["--licenses"]), Mode::Licenses);
        assert_eq!(
            mode(&["--list-commands", "--json"]),
            Mode::ListCommands { json: true }
        );
        assert_eq!(mode(&["--dump-keymap"]), Mode::DumpKeymap { json: false });
        assert_eq!(
            p(&["--json"]).unwrap_err(),
            UsageError::UnknownOption("--json".into()),
            "--dump-config / --list-commands / --dump-keymap の修飾だけ"
        );
        assert_eq!(
            p(&["--list-commands", "--defaults"]).unwrap_err(),
            UsageError::UnknownOption("--defaults".into()),
            "--defaults は --dump-config の修飾だけ"
        );
        let Cli::Run { log_level, .. } = p(&["--log-level", "debug"]).unwrap() else {
            panic!()
        };
        assert_eq!(log_level.as_deref(), Some("debug"));
        assert_eq!(
            p(&["--log-level"]).unwrap_err(),
            UsageError::MissingValue("--log-level".into())
        );
        assert_eq!(
            p(&["--defaults"]).unwrap_err(),
            UsageError::UnknownOption("--defaults".into()),
            "--dump-config の修飾だけ"
        );
    }

    #[test]
    fn 未知のオプションは使い方の誤り() {
        assert_eq!(
            p(&["--nope"]).unwrap_err(),
            UsageError::UnknownOption("--nope".into())
        );
        assert_eq!(
            p(&["-x"]).unwrap_err(),
            UsageError::UnknownOption("-x".into())
        );
    }

    #[test]
    fn 区切りの後は位置引数() {
        assert_eq!(
            p(&["--", "-weird.txt"]).unwrap(),
            run(vec![plain("-weird.txt")], false)
        );
        assert_eq!(
            p(&["--", "--help"]).unwrap(),
            run(vec![plain("--help")], false)
        );
        // 区切りの後の `+10` はファイル名
        assert_eq!(p(&["--", "+10"]).unwrap(), run(vec![plain("+10")], false));
    }

    #[test]
    fn プラス_行は直後のファイルに掛かる() {
        assert_eq!(
            p(&["+10", "a.txt", "b.txt"]).unwrap(),
            run(vec![f("a.txt", Some(10), None), plain("b.txt")], false)
        );
        assert_eq!(
            p(&["+10:5", "a.txt"]).unwrap(),
            run(vec![f("a.txt", Some(10), Some(5))], false)
        );
        assert_eq!(
            p(&["a.txt", "+3", "b.txt"]).unwrap(),
            run(vec![plain("a.txt"), f("b.txt", Some(3), None)], false),
            "ファイルの間の +LINE は直後のファイル"
        );
    }

    #[test]
    fn ファイルの後ろの_プラス_行は直前のファイルに掛かる() {
        assert_eq!(
            p(&["a.txt", "+10"]).unwrap(),
            run(vec![f("a.txt", Some(10), None)], false)
        );
        assert_eq!(
            p(&["a.txt", "b.txt", "+7:2"]).unwrap(),
            run(vec![plain("a.txt"), f("b.txt", Some(7), Some(2))], false)
        );
    }

    #[test]
    fn プラス_単独は最終行() {
        assert_eq!(
            p(&["+", "a.txt"]).unwrap(),
            run(vec![f("a.txt", Some(usize::MAX), None)], false)
        );
    }

    #[test]
    fn 読めない_プラス_は未対応として無視しファイル名にしない() {
        let r = p(&["+/pat", "+abc", "a.txt"]).unwrap();
        assert_eq!(
            r,
            Cli::Run {
                files: vec![plain("a.txt")],
                read_only: false,
                encoding: None,
                locale: None,
                ignored: vec!["+/pat".into(), "+abc".into()],
                config: ConfigArgs::default(),
                mode: Mode::Edit,
                log_level: None,
            }
        );
    }

    #[test]
    fn ファイル_行_桁は存在するファイルなら位置指定() {
        assert_eq!(
            parse_with(&["a.txt:10"], &["a.txt"]).unwrap(),
            run(vec![f("a.txt", Some(10), None)], false)
        );
        assert_eq!(
            parse_with(&["a.txt:10:5"], &["a.txt"]).unwrap(),
            run(vec![f("a.txt", Some(10), Some(5))], false)
        );
    }

    #[test]
    fn 曖昧なときは全体が存在するファイルを優先し_どちらも無ければ全体を新規名にする() {
        // 全体が存在する: 位置指定なしで開く
        assert_eq!(
            parse_with(&["a.txt:10"], &["a.txt", "a.txt:10"]).unwrap(),
            run(vec![plain("a.txt:10")], false)
        );
        // どちらも無い: 全体を新規ファイル名とする
        assert_eq!(
            p(&["a.txt:10"]).unwrap(),
            run(vec![plain("a.txt:10")], false)
        );
        assert_eq!(
            p(&["a.txt:10:5"]).unwrap(),
            run(vec![plain("a.txt:10:5")], false)
        );
    }

    #[test]
    fn ドライブレターのコロンは位置指定にしない() {
        assert_eq!(
            parse_with(&["C:\\w\\a.txt:3"], &["C:\\w\\a.txt"]).unwrap(),
            run(vec![f("C:\\w\\a.txt", Some(3), None)], false)
        );
        assert_eq!(
            parse_with(&["C:\\w\\a.txt"], &["C:\\w\\a.txt"]).unwrap(),
            run(vec![plain("C:\\w\\a.txt")], false)
        );
        // 存在しなくても、ドライブレターの `:` で剥がしたりしない
        assert_eq!(
            p(&["C:\\w\\new.txt"]).unwrap(),
            run(vec![plain("C:\\w\\new.txt")], false)
        );
    }

    #[test]
    fn 末尾が位置の形でない引数では存在確認をしない() {
        let calls = std::cell::Cell::new(0);
        let r = parse(
            [r"\\10.255.255.1\share\a.txt", "b.txt", r"C:\w\c.txt"]
                .iter()
                .map(OsString::from),
            &|_| {
                calls.set(calls.get() + 1);
                false
            },
        )
        .unwrap();
        assert!(matches!(r, Cli::Run { files, .. } if files.len() == 3));
        assert_eq!(calls.get(), 0, "ファイルシステムに触れない");
    }

    #[test]
    fn 引数の位置指定は_プラス_より優先する() {
        assert_eq!(
            parse_with(&["+9", "a.txt:2:3"], &["a.txt"]).unwrap(),
            run(vec![f("a.txt", Some(2), Some(3))], false),
            "FILE:LINE:COL が明示されていればそれを使う"
        );
    }

    // ---- 標準入力の `-` (07 §18.2、03 §9.2.2 の「書き方」。v0.4 の M25) ----

    // verifies: AC-FR-120-1
    #[test]
    fn ハイフンは標準入力() {
        assert_eq!(p(&["-"]).unwrap(), run(vec![stdin(None, None)], false));
    }

    #[test]
    fn 標準入力はファイルと一緒に書いた順に並ぶ() {
        assert_eq!(
            p(&["a.txt", "-", "b.txt"]).unwrap(),
            run(
                vec![plain("a.txt"), stdin(None, None), plain("b.txt")],
                false
            )
        );
    }

    #[test]
    fn 区切りの後のハイフンも標準入力_ハイフンという名前のファイルは_dot_slash() {
        assert_eq!(
            p(&["--", "-", "-weird.txt"]).unwrap(),
            run(vec![stdin(None, None), plain("-weird.txt")], false)
        );
        assert_eq!(p(&["./-"]).unwrap(), run(vec![plain("./-")], false));
        assert_eq!(p(&[r".\-"]).unwrap(), run(vec![plain(r".\-")], false));
    }

    #[test]
    fn 標準入力は_1_つまで() {
        assert_eq!(p(&["-", "-"]).unwrap_err(), UsageError::StdinTwice);
        assert_eq!(
            p(&["-", "a.txt", "--", "-"]).unwrap_err(),
            UsageError::StdinTwice,
            "区切りの後の - も数える"
        );
    }

    #[test]
    fn プラス_行は直後の標準入力に掛かり_後置なら直前() {
        assert_eq!(
            p(&["+10", "-", "a.txt"]).unwrap(),
            run(vec![stdin(Some(10), None), plain("a.txt")], false)
        );
        assert_eq!(
            p(&["a.txt", "-", "+3:2"]).unwrap(),
            run(vec![plain("a.txt"), stdin(Some(3), Some(2))], false)
        );
    }

    #[test]
    fn readonly_と_encoding_は標準入力にも掛かる() {
        let r = p(&["-R", "-e", "shift_jis", "-"]).unwrap();
        assert!(matches!(
            &r,
            Cli::Run { files, read_only: true, encoding: Some(e), .. }
                if e == "shift_jis" && files == &vec![stdin(None, None)]
        ));
    }

    #[test]
    fn 駆動_api_の標準入力との併用は使い方の誤り() {
        // 配布ビルドは `--headless` を受け付けない (未知のオプション、終了コード 2)。標準入力と一緒でも同じ
        assert!(matches!(
            p(&["--headless", "--rpc", "stdio", "-"]).unwrap_err(),
            UsageError::UnknownOption(o) if o == "--headless"
        ));
    }
}
