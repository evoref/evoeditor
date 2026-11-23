//! ユーザー層の言語定義のファイル (`<config_dir>/languages/*.toml`) の読込 (docs/07 §9.6 の LANG-2)。`ReadLanguages` の
//! ジョブ (io) と、bin の `--check-config`・`--dump-languages` が同じ関数で読む。
//!
//! 読むのは拡張子が `.toml` のファイルだけ (ディレクトリは読まない。シンボリックリンクは辿る)。名前の順に 256 ファイルまで、
//! 1 ファイル 256 KiB まで。超えた分は読まずに警告を 1 件出す。1 つのファイルに `[[language]]` を幾つ書いてもよく、
//! ファイル名と言語の名前は結び付けない。

use eve_config::{DiagKind, Diagnostic, LanguageDef, LanguageProblem, Severity};

use crate::io::fs::{FileKind, FileSystem, FsErrorKind};
use crate::io::fspath::FsPath;

/// 言語定義のファイルの診断の表示名の接頭辞 (`languages/<file>.toml`)。
pub const LANGUAGES_PREFIX: &str = "languages/";
/// 読むファイルの数の上限。
pub const LANGUAGE_FILES_MAX: usize = 256;
/// 1 ファイルの大きさの上限 (バイト)。
pub const LANGUAGE_FILE_MAX_BYTES: u64 = 256 * 1024;

/// 診断 1 件 (位置の無いファイル全体の問題)。
fn whole(severity: Severity, kind: DiagKind, value: String) -> Diagnostic {
    Diagnostic {
        severity,
        kind,
        key: String::new(),
        value,
        line: 0,
        column: 0,
        condition: None,
    }
}

/// `dir` の言語定義のファイルを名前の順に読む。戻り値は (定義 (読んだ順), 表示名ごとの診断)。ディレクトリが無ければ空。
pub fn read_language_dir(
    fs: &dyn FileSystem,
    dir: &FsPath,
) -> (Vec<LanguageDef>, Vec<(String, Vec<Diagnostic>)>) {
    let mut names: Vec<String> = match fs.list_dir_names(dir) {
        Ok(n) => n,
        Err(_) => return (Vec::new(), Vec::new()),
    };
    names.retain(|n| n.ends_with(".toml"));
    names.sort();
    let mut defs = Vec::new();
    let mut reports: Vec<(String, Vec<Diagnostic>)> = Vec::new();
    let mut read = 0usize;
    for name in &names {
        let path = dir.join(name);
        let shown = format!("{LANGUAGES_PREFIX}{name}");
        let meta = match fs.metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                reports.push((
                    shown,
                    vec![whole(
                        Severity::Error,
                        DiagKind::Syntax {
                            message: format!("{:?}", e.kind),
                        },
                        String::new(),
                    )],
                ));
                continue;
            }
        };
        if meta.kind != FileKind::File {
            continue;
        }
        if read == LANGUAGE_FILES_MAX {
            reports.push((
                LANGUAGES_PREFIX.to_owned(),
                vec![whole(
                    Severity::Warning,
                    DiagKind::Language(LanguageProblem::TooManyFiles),
                    LANGUAGE_FILES_MAX.to_string(),
                )],
            ));
            break;
        }
        read += 1;
        if meta.len > LANGUAGE_FILE_MAX_BYTES {
            reports.push((
                shown,
                vec![whole(
                    Severity::Warning,
                    DiagKind::Language(LanguageProblem::FileTooLarge),
                    String::new(),
                )],
            ));
            continue;
        }
        let text = match read_limited(fs, &path) {
            Ok(t) => t,
            Err(message) => {
                reports.push((
                    shown,
                    vec![whole(
                        Severity::Error,
                        DiagKind::Syntax { message },
                        String::new(),
                    )],
                ));
                continue;
            }
        };
        let (mut d, diags) = eve_config::load_language_str(&shown, &text);
        defs.append(&mut d);
        if !diags.is_empty() {
            reports.push((shown, diags));
        }
    }
    (defs, reports)
}

/// 1 ファイルを上限まで読み、UTF-8 の文字列にする。
fn read_limited(fs: &dyn FileSystem, path: &FsPath) -> Result<String, String> {
    let mut r = match fs.open_read(path) {
        Ok(r) => r,
        Err(e) if e.kind == FsErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(format!("{:?}", e.kind)),
    };
    let mut out = Vec::new();
    loop {
        match r.next_chunk() {
            Ok(Some(c)) => {
                out.extend_from_slice(c);
                if u64::try_from(out.len()).unwrap_or(u64::MAX) > LANGUAGE_FILE_MAX_BYTES {
                    return Err("too large".to_owned());
                }
            }
            Ok(None) => break,
            Err(e) => return Err(format!("{e:?}")),
        }
    }
    String::from_utf8(out).map_err(|_| "not valid UTF-8".to_owned())
}
