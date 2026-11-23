//! ユーザー層の言語定義のファイル (`<config_dir>/languages/*.toml`) の読込 (docs/07 §9.6 の LANG-2)。`ReadLanguages` の
//! ジョブ (io) と、bin の `--check-config`・`--dump-languages` が同じ関数で読む。
//!
//! 読むのは拡張子が `.toml` のファイルだけ (ディレクトリは読まない。シンボリックリンクは辿る)。名前の順に 256 ファイルまで、
//! 1 ファイル 256 KiB まで。超えた分は読まずに警告を 1 件出す。1 つのファイルに `[[language]]` を幾つ書いてもよく、
//! ファイル名と言語の名前は結び付けない。

use eve_config::{DiagKind, Diagnostic, LanguageDef, LanguageProblem, Severity};

use crate::io::fs::{EntryKind, FileKind, FileSystem, FsErrorKind};
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
    let mut names = list_toml(fs, dir);
    names.sort();
    let mut defs = Vec::new();
    let mut reports: Vec<(String, Vec<Diagnostic>)> = Vec::new();
    let mut read = 0usize;
    for (name, known) in &names {
        let path = dir.join(name);
        let shown = format!("{LANGUAGES_PREFIX}{name}");
        // 列挙で種類と大きさが分かったファイルは、項目ごとの `stat` をしない (Windows。起動時の文書を開くのを待たせない)
        let meta = match known {
            Some(len) => (FileKind::File, *len),
            None => match fs.metadata(&path) {
                Ok(m) => (m.kind, m.len),
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
            },
        };
        let (kind, len) = meta;
        if kind != FileKind::File {
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
        if len > LANGUAGE_FILE_MAX_BYTES {
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

/// 列挙の項目の上限 (`languages/` にほかのファイルがあっても、言語定義の上限の数より多く見る)。
const LIST_MAX_ENTRIES: usize = 4_096;

/// `dir` の `*.toml` の (名前, 列挙で分かった大きさ)。大きさは、列挙で通常のファイルとその大きさが分かったとき (Windows) だけ。
/// ディレクトリ・FIFO などは除き、シンボリックリンクと大きさの分からないものは `None` (呼び側が `metadata` で辿る)。列挙を
/// 実装していない Port は名前の一覧で代える。無ければ空。
fn list_toml(fs: &dyn FileSystem, dir: &FsPath) -> Vec<(String, Option<u64>)> {
    let toml = |n: &str| n.ends_with(".toml");
    match fs.read_dir(dir, LIST_MAX_ENTRIES) {
        Ok(listing) => listing
            .entries
            .into_iter()
            .filter_map(|e| {
                let name = e.name.to_str()?.to_owned();
                let known = match e.kind {
                    EntryKind::File => e.len,
                    EntryKind::Symlink => None,
                    EntryKind::Dir | EntryKind::Other => return None,
                };
                toml(&name).then_some((name, known))
            })
            .collect(),
        Err(e) if e.kind == FsErrorKind::Unsupported => fs
            .list_dir_names(dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|n| toml(n))
            .map(|n| (n, None))
            .collect(),
        Err(_) => Vec::new(),
    }
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
