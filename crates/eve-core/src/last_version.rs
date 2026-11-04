//! 前回起動した版の記録 (FR-518、07 §18.8 の (2))。`state_dir/last_version.toml` を読み、今の版と比べて、
//! 更新後の初回起動なら要点を表示する。通信しない。読み書きは `io` ワーカーのジョブ (`JobKind::LastVersion`) で行う。
//!
//! 書式は自分で書く 2〜3 行の TOML だけなので、TOML の crate を使わずに `key = value` の行で読む。

use crate::fs::{FileSystem, FsErrorKind};

/// 記録のファイル名 (`state_dir` の直下)。
pub const LAST_VERSION_FILE: &str = "last_version.toml";

/// この版が書く書式の版。
const FORMAT_VERSION: u32 = 1;

/// 比べた結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LastVersionOutcome {
    /// 前回より新しい版で起動した: 要点を表示する。
    Updated,
    /// 表示しない (初回・同じ版・ダウングレード・記録が壊れている・読めない・書けない)。
    Silent,
}

/// SemVer の比較に使う形 (`major.minor.patch` と、あればプレリリースの識別子)。ビルドメタデータは無視する。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Version {
    core: (u64, u64, u64),
    pre: Vec<String>,
}

fn parse_version(s: &str) -> Option<Version> {
    let s = s.split('+').next()?;
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, p.split('.').map(str::to_owned).collect()),
        None => (s, Vec::new()),
    };
    let mut it = core.split('.').map(|n| n.parse::<u64>().ok());
    let v = (it.next()??, it.next()??, it.next()??);
    if it.next().is_some() || (pre.iter().any(String::is_empty)) {
        return None;
    }
    Some(Version { core: v, pre })
}

/// SemVer の優先順位 (プレリリースは正式版より前。識別子は数字なら数値、それ以外は文字列で比べ、数字が先)。
fn cmp_version(a: &Version, b: &Version) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    a.core
        .cmp(&b.core)
        .then_with(|| match (a.pre.is_empty(), b.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (x, y) in a.pre.iter().zip(&b.pre) {
                    let o = match (x.parse::<u64>(), y.parse::<u64>()) {
                        (Ok(m), Ok(n)) => m.cmp(&n),
                        (Ok(_), Err(_)) => Ordering::Less,
                        (Err(_), Ok(_)) => Ordering::Greater,
                        (Err(_), Err(_)) => x.cmp(y),
                    };
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                a.pre.len().cmp(&b.pre.len())
            }
        })
}

/// 記録の読み取り結果。
#[derive(Debug, PartialEq, Eq)]
enum Record {
    /// 読めた (記録の版の文字列)。
    Version(String),
    /// 未知の書式の版 (05 §7.5 (a): 読まずに退避する)。
    UnknownFormat,
    /// 壊れている・版の文字列が不正 (書き直す)。
    Broken,
}

fn parse_record(bytes: &[u8]) -> Record {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Record::Broken;
    };
    let mut format = None;
    let mut version = None;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "format_version" => format = v.parse::<u32>().ok(),
            "last_run_version" => {
                version = v
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .map(str::to_owned);
            }
            _ => {}
        }
    }
    match format {
        Some(FORMAT_VERSION) => match version {
            Some(v) if parse_version(&v).is_some() => Record::Version(v),
            _ => Record::Broken,
        },
        Some(_) => Record::UnknownFormat,
        None => Record::Broken,
    }
}

/// 記録の本文 (任意の `updated_at` は書かない)。
fn record_text(current: &str) -> String {
    format!(
        "# evoeditor が最後に起動した版 (更新後の初回起動の要点表示に使う、FR-518)\nformat_version = {FORMAT_VERSION}\nlast_run_version = \"{current}\"\n"
    )
}

/// 記録を読み、今の版 `current` と比べ、必要なら書き直す (07 §18.8 の「比較」の表)。読めない・書けないときは表示しない
/// (メッセージ行にエラーを出さない。次の編集セッションで再び試す)。
pub fn check_and_record(fs: &dyn FileSystem, current: &str) -> LastVersionOutcome {
    let Some(cur) = parse_version(current) else {
        return LastVersionOutcome::Silent;
    };
    let write = || fs.write_state_file(LAST_VERSION_FILE, record_text(current).as_bytes());
    match fs.read_state_file(LAST_VERSION_FILE) {
        // 初めての起動: 表示せずに記録だけを作る
        Err(e) if e.kind == FsErrorKind::NotFound => {
            let _ = write();
            LastVersionOutcome::Silent
        }
        // `state_dir` を使えない・読めない: 表示も記録もしない
        Err(_) => LastVersionOutcome::Silent,
        Ok(bytes) => match parse_record(&bytes) {
            Record::Version(v) => {
                let prev = parse_version(&v).expect("parse_record で確かめた");
                match cmp_version(&prev, &cur) {
                    std::cmp::Ordering::Equal => LastVersionOutcome::Silent,
                    // 更新: 書けたときだけ表示する (書けないと毎回表示してしまう)
                    std::cmp::Ordering::Less => {
                        if write().is_ok() {
                            LastVersionOutcome::Updated
                        } else {
                            LastVersionOutcome::Silent
                        }
                    }
                    // ダウングレード: 表示せずに今の版へ書き換える
                    std::cmp::Ordering::Greater => {
                        let _ = write();
                        LastVersionOutcome::Silent
                    }
                }
            }
            Record::Broken => {
                let _ = write();
                LastVersionOutcome::Silent
            }
            // 未知の書式: 読まずに退避してから、今の書式で作り直す (退避できなければ触らない)
            Record::UnknownFormat => {
                if fs.set_aside_state_file(LAST_VERSION_FILE).is_ok() {
                    let _ = write();
                }
                LastVersionOutcome::Silent
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        parse_version(s).unwrap()
    }

    #[test]
    fn 版の比較は_semver_の優先順位() {
        use std::cmp::Ordering::*;
        let cases = [
            ("0.1.0", "0.2.0", Less),
            ("0.1.10", "0.1.9", Greater),
            ("0.1.0-rc.1", "0.1.0", Less),
            ("0.1.0-rc.2", "0.1.0-rc.10", Less),
            ("0.1.0-alpha", "0.1.0-alpha.1", Less),
            ("0.1.0-1", "0.1.0-alpha", Less),
            ("1.0.0+build.5", "1.0.0", Equal),
        ];
        for (a, b, o) in cases {
            assert_eq!(cmp_version(&v(a), &v(b)), o, "{a} vs {b}");
        }
        for bad in ["", "1", "1.2", "1.2.3.4", "a.b.c", "1.2.3-", "1.2.3-a..b"] {
            assert!(parse_version(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn 記録の読み取り() {
        let ok = record_text("0.2.0");
        assert_eq!(parse_record(ok.as_bytes()), Record::Version("0.2.0".into()));
        assert_eq!(
            parse_record(b"format_version = 2\nlast_run_version = \"9.0.0\"\n"),
            Record::UnknownFormat
        );
        assert_eq!(
            parse_record(b"format_version = 1\nlast_run_version = \"x\"\n"),
            Record::Broken
        );
        assert_eq!(parse_record(b"\xff\xfe"), Record::Broken);
        assert_eq!(parse_record(b""), Record::Broken);
    }
}
