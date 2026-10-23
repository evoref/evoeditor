//! コマンドラインの解析 (docs/07 §18.1〜§18.3。v0.1 の公開オプションは `--version` `--help` `--readonly` `--encoding` `--`
//! と、受理して無視する `--wait`)。パーサは外部 crate を使わず、この小さな関数で済ませる (サブコマンドが要る v1.X で見直す、OQ-24)。

use std::ffi::OsString;

/// 解析の結果。
#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    Help,
    Version,
    Run {
        /// 開くファイル (0 個なら無題)。
        file: Option<String>,
        read_only: bool,
        /// `--encoding` の値 (M3 で使う。v0.1 の M2 では受け取るだけ)。
        encoding: Option<String>,
        /// `+LINE` など、このバージョンでは未対応のため無視した引数 (`info.cli.unsupported` の対象)。
        ignored: Vec<String>,
    },
}

/// 使い方の誤り (終了コード 2、07 §18.4)。
#[derive(Debug, PartialEq, Eq)]
pub enum UsageError {
    UnknownOption(String),
    MissingValue(String),
    MultipleFiles,
    /// 文字コードのラベルが空など。
    BadEncoding(String),
}

fn to_string(a: OsString) -> Result<String, UsageError> {
    a.into_string()
        .map_err(|o| UsageError::UnknownOption(o.to_string_lossy().into_owned()))
}

/// `args` は実行ファイル名を除いた引数。
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Cli, UsageError> {
    let mut it = args.into_iter();
    let mut files: Vec<String> = Vec::new();
    let mut read_only = false;
    let mut encoding = None;
    let mut ignored = Vec::new();
    let mut only_positional = false;
    while let Some(a) = it.next() {
        let a = to_string(a)?;
        if only_positional {
            files.push(a);
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
            s if s.starts_with("--encoding=") => {
                let v = s["--encoding=".len()..].to_owned();
                if v.is_empty() {
                    return Err(UsageError::BadEncoding(v));
                }
                encoding = Some(v);
            }
            // v0.2 で意味が付く `+LINE` は、このバージョンでは無視する (ファイル名として開かない)
            s if s.starts_with('+') => ignored.push(s.to_owned()),
            s if s.starts_with('-') && s != "-" => return Err(UsageError::UnknownOption(a)),
            // `-` (標準入力) は v0.3。いまはファイル名として扱わず未対応として無視する
            "-" => ignored.push(a),
            _ => files.push(a),
        }
    }
    if files.len() > 1 {
        return Err(UsageError::MultipleFiles);
    }
    Ok(Cli::Run {
        file: files.pop(),
        read_only,
        encoding,
        ignored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Cli, UsageError> {
        parse(args.iter().map(OsString::from))
    }

    fn run(file: Option<&str>, read_only: bool) -> Cli {
        Cli::Run {
            file: file.map(str::to_owned),
            read_only,
            encoding: None,
            ignored: vec![],
        }
    }

    // verifies: AC-FR-101-1
    #[test]
    fn 引数なしは無題_ファイル_1_つは開く() {
        assert_eq!(p(&[]).unwrap(), run(None, false));
        assert_eq!(p(&["a.txt"]).unwrap(), run(Some("a.txt"), false));
        assert_eq!(p(&["日本語.txt"]).unwrap(), run(Some("日本語.txt"), false));
    }

    #[test]
    fn 公開オプション() {
        assert_eq!(p(&["--help"]).unwrap(), Cli::Help);
        assert_eq!(p(&["-h"]).unwrap(), Cli::Help);
        assert_eq!(p(&["--version"]).unwrap(), Cli::Version);
        assert_eq!(p(&["-V", "a.txt"]).unwrap(), Cli::Version);
        assert_eq!(
            p(&["--readonly", "a.txt"]).unwrap(),
            run(Some("a.txt"), true)
        );
        assert_eq!(p(&["a.txt", "-R"]).unwrap(), run(Some("a.txt"), true));
        assert_eq!(p(&["--wait", "a.txt"]).unwrap(), run(Some("a.txt"), false));
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

    // verifies: AC-FR-101-1
    #[test]
    fn 二つ以上のファイルは使い方の誤り() {
        assert_eq!(
            p(&["a.txt", "b.txt"]).unwrap_err(),
            UsageError::MultipleFiles
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
            run(Some("-weird.txt"), false)
        );
        assert_eq!(p(&["--", "--help"]).unwrap(), run(Some("--help"), false));
    }

    #[test]
    fn プラス_行指定は未対応として無視しファイル名にしない() {
        let r = p(&["+10", "a.txt"]).unwrap();
        assert_eq!(
            r,
            Cli::Run {
                file: Some("a.txt".into()),
                read_only: false,
                encoding: None,
                ignored: vec!["+10".into()],
            }
        );
    }

    #[test]
    fn コロン付きはそのままファイル名() {
        assert_eq!(p(&["a.txt:10"]).unwrap(), run(Some("a.txt:10"), false));
    }
}
