//! `.editorconfig` の読込 (`ReadEditorConfig`。docs/07 §10.3 の EC-1〜EC-8、§3.6 の LAYER-1 の (5) と LAYER-3 / LAYER-4、
//! 01 FR-705 AC-3 / AC-4)。
//!
//! - **ジョブ** (`io`): 文書のディレクトリから上へ、各ディレクトリの `.editorconfig` を所有者の検査つきの読込
//!   (`FileSystem::read_checked`、`DirLinks::Follow`) で読み、読込・解析・glob の照合・重ね合わせまでを行う (EC-1 / EC-2 /
//!   EC-5)。主スレッドへは、重ねた値 (由来つき) と、読んだファイルごとの診断だけを返す。
//! - **主スレッド**: 届いた値を文書に当てて文書ごとの値を決め直し (LAYER-3)、まだ編集していない新しいファイルの改行と
//!   文字コードを決め直す (LAYER-4)。古い結果 (後に積んだ読込がある・文書のパスが変わった) は捨てる。診断は、同じファイルの
//!   同じ中身について、メッセージ行の要約をセッションで 1 回だけ出す (EC-6)。
//!
//! spec: 07 §10.3, 07 §10.1, 07 §3.6, 07 §12.1

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use eve_config::{
    Diagnostic, EDITORCONFIG_FILE, EDITORCONFIG_FILE_MAX_BYTES, EDITORCONFIG_MAX_DEPTH,
    EDITORCONFIG_TOTAL_MAX_BYTES, EDITORCONFIG_TOTAL_MAX_SECTIONS, EditorConfigFile,
    EditorConfigSource, EditorConfigValues, NotReadReason, not_read_diagnostic, parse_editorconfig,
    resolve_editorconfig,
};
use eve_view::{DocumentId, MessageLevel};

use crate::editor::state::EditorState;
use crate::io::fs::{CheckedRead, DirLinks, FileSystem, FsErrorKind};
use crate::io::fspath::FsPath;
use crate::io::pending::Pending;
use crate::jobs::{JobKind, JobResult};
use crate::ui_model::keys;

/// 文書に当てた `.editorconfig` の値と、読んだときの `workspace.safe_directories` (書いた形)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditorConfigApplied {
    /// 重ねた値 (07 §3.6 の LAYER-1 の (5))。
    pub(crate) values: Arc<EditorConfigValues>,
    /// 読んだときの安全な場所 (`config.reload` で変わったら、一時の失敗でも前の値を保たない)。
    pub(crate) safe_dirs: Vec<String>,
}

/// 読んだ (か、読まなかった) `.editorconfig` 1 つの診断。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorConfigReport {
    /// ファイルの絶対パス (診断の一覧の表示名。`Enter` で開く)。
    pub path: String,
    /// 中身 (読まなかったときは理由) の要約。同じ中身の診断を、セッションで 1 回だけ要約に出すのに使う (EC-6)。
    pub digest: u64,
    /// 診断 (1 つのファイルの上限を掛けたもの、07 §4.3)。
    pub diagnostics: Vec<Diagnostic>,
}

/// `ReadEditorConfig` の結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorConfigRead {
    /// 一時の失敗 (共有違反・リース): 探すのをやめた。前に当てた値を使い続ける (次の読込で読み直す)。`path` は読めなかった
    /// `.editorconfig`。
    Busy {
        /// 読めなかった `.editorconfig` のパス。
        path: String,
    },
    /// 読んだ。
    Done {
        /// 文書に当たる値。
        values: Box<EditorConfigValues>,
        /// 見つけたファイル (近い順) ごとの診断。無い (`Missing`) ディレクトリは含めない。
        reports: Vec<EditorConfigReport>,
        /// 探したが無かった `.editorconfig` のパス (前に診断を出したファイルが消えていれば、一覧から外す)。
        missing: Vec<String>,
    },
}

fn digest(tag: &str, data: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    tag.hash(&mut h);
    data.hash(&mut h);
    h.finish()
}

fn report(path: &str, digest: u64, mut diagnostics: Vec<Diagnostic>) -> EditorConfigReport {
    eve_config::cap_diagnostics(&mut diagnostics);
    EditorConfigReport {
        path: path.to_owned(),
        digest,
        diagnostics,
    }
}

/// パスの名前の要素 (ドライブ・ルートを除く)。
fn names(p: &FsPath) -> Vec<String> {
    p.as_path()
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// 開いたときのパスの名前の要素を、ディスクの上の名前 (canonical のパスの、末尾から揃えた要素) に直す (Windows、EC-1)。
/// 直すのは、大文字小文字を除いて同じ名前 (ディスクの大文字小文字にする) と、`~` を含む名前 (8.3 の短い名前) だけ。
/// ジャンクションなどで名前が違う要素は、開いたときの名前のまま (置き場所からの相対パスは開いたパスで決まる)。
pub(crate) fn align_disk_names(opened: &[String], canonical: &[String]) -> Vec<String> {
    let mut out = opened.to_vec();
    for (o, c) in out.iter_mut().rev().zip(canonical.iter().rev()) {
        if o.to_lowercase() == c.to_lowercase() || o.contains('~') {
            o.clone_from(c);
        }
    }
    out
}

/// glob に当てる名前の要素 (Windows はディスクの上の名前に直す。Unix は開いたときのパスのまま、EC-1)。ディスクに無い
/// 新しいファイルは、親のディレクトリを canonicalize して名前を足す。
fn match_names(fs: &dyn FileSystem, path: &FsPath) -> Vec<String> {
    let opened = names(path);
    if !cfg!(windows) {
        return opened;
    }
    let canonical = fs.canonicalize(path).ok().or_else(|| {
        let parent = fs.canonicalize(&path.parent()?).ok()?;
        Some(parent.join(path.file_name()?))
    });
    match canonical {
        Some(c) => align_disk_names(&opened, &names(&c)),
        None => opened,
    }
}

/// `.editorconfig` を集めて重ねる (ジョブの本体。07 §10.3 の EC-1 / EC-2 / EC-5)。`path` は文書の開いたときの絶対パス、
/// `safe_dirs` は `workspace.safe_directories`。
pub fn read_editorconfig(
    fs: &dyn FileSystem,
    path: &FsPath,
    safe_dirs: &[FsPath],
) -> EditorConfigRead {
    let names = match_names(fs, path);
    // 近い順 (表示名, 読んだファイル, 置き場所から文書への相対パス)
    let mut loaded: Vec<(String, EditorConfigFile, String)> = Vec::new();
    let mut reports = Vec::new();
    let mut missing = Vec::new();
    let mut total_bytes = 0u64;
    let mut total_sections = 0usize;
    let ancestors = path
        .as_path()
        .ancestors()
        .skip(1)
        .take(EDITORCONFIG_MAX_DEPTH);
    for (depth, dir) in ancestors.enumerate() {
        let Some(start) = names.len().checked_sub(depth + 1) else {
            break;
        };
        let rel = names[start..].join("/");
        let dir = FsPath::new(dir);
        let file = dir.join(EDITORCONFIG_FILE);
        let name = file.display();
        let r = fs.read_checked(
            &dir,
            std::ffi::OsStr::new(EDITORCONFIG_FILE),
            EDITORCONFIG_FILE_MAX_BYTES,
            safe_dirs,
            DirLinks::Follow,
        );
        match r {
            Ok(CheckedRead::Missing) => missing.push(name),
            Ok(CheckedRead::TooLarge) => reports.push(report(
                &name,
                digest("too_large", &[]),
                vec![not_read_diagnostic(NotReadReason::TooLarge, &name)],
            )),
            // 所有者の検査で読まなかった: そのファイルだけを飛ばして上へ (そのファイルの `root` は見ない、EC-2)
            Ok(CheckedRead::Rejected { what, verdict }) => {
                let d =
                    crate::io::workspace_config::rejected(what, verdict, &dir, &file, safe_dirs);
                let key = format!("{d:?}");
                reports.push(report(&name, digest("rejected", key.as_bytes()), vec![d]));
            }
            Ok(CheckedRead::Read { bytes }) => {
                let len = bytes.len() as u64;
                let text = match String::from_utf8(bytes) {
                    Ok(t) => t,
                    Err(e) => {
                        reports.push(report(
                            &name,
                            digest("not_utf8", e.as_bytes()),
                            vec![not_read_diagnostic(NotReadReason::NotUtf8, &name)],
                        ));
                        continue;
                    }
                };
                let parsed = parse_editorconfig(&text);
                // 1 つの文書で読む量の上限: このファイルとより上のファイルを使わない
                if total_bytes + len > EDITORCONFIG_TOTAL_MAX_BYTES
                    || total_sections + parsed.section_count() > EDITORCONFIG_TOTAL_MAX_SECTIONS
                {
                    reports.push(report(
                        &name,
                        digest("total_limit", text.as_bytes()),
                        vec![not_read_diagnostic(NotReadReason::TotalLimit, &name)],
                    ));
                    break;
                }
                total_bytes += len;
                total_sections += parsed.section_count();
                reports.push(report(
                    &name,
                    digest("read", text.as_bytes()),
                    parsed.diagnostics().to_vec(),
                ));
                let root = parsed.root();
                loaded.push((name, parsed, rel));
                if root {
                    break;
                }
            }
            Err(e) => match e.kind {
                FsErrorKind::Busy => return EditorConfigRead::Busy { path: name },
                // 所有者の検査を持たない Port: 読まない (診断なし)
                FsErrorKind::Unsupported => break,
                // 開けない祖先 (権限など) は黙って次へ (07 §12.6.3 の段 4 から送った (1))
                _ => {}
            },
        }
    }
    let sources: Vec<EditorConfigSource<'_>> = loaded
        .iter()
        .rev()
        .map(|(n, f, r)| (n.as_str(), f, r.as_str()))
        .collect();
    EditorConfigRead::Done {
        values: Box::new(resolve_editorconfig(&sources)),
        reports,
        missing,
    }
}

impl EditorState {
    /// `.editorconfig` を読むか (07 §10.3 の EC-7): Host が読込元を渡していて、`--no-workspace-config` でも `--no-config`
    /// でもない。
    fn editorconfig_enabled(&self) -> bool {
        self.io
            .config_sources
            .as_ref()
            .is_some_and(|s| !s.no_workspace_config)
    }

    /// 文書 `doc` の `ReadEditorConfig` を積む (読込の完了・名前を付けて保存でパスが変わった・`config.reload`、EC-1)。
    /// パスの無い文書 (無題・標準入力) と、読まない起動では積まない。前に積んだ読込の結果は、届いても捨てる (後のものが勝つ)。
    pub(crate) fn start_editorconfig_read(&mut self, doc: DocumentId, reload: bool) {
        if !self.editorconfig_enabled() {
            return;
        }
        let Some(path) = self
            .docs
            .documents
            .get(&doc)
            .filter(|d| d.stdin.is_none())
            .and_then(|d| d.path.clone())
        else {
            return;
        };
        self.settings.editorconfig_seq += 1;
        let seq = self.settings.editorconfig_seq;
        self.settings.editorconfig_latest.insert(doc, seq);
        let written = self.settings.config.safe_directories.clone();
        let safe_dirs: Arc<[FsPath]> = written.iter().map(|s| FsPath::from(s.as_str())).collect();
        let job = self.jobs.spawn(
            Some(doc),
            eve_view::Revision(0),
            JobKind::ReadEditorConfig {
                path: path.clone(),
                safe_dirs,
            },
        );
        self.io.pending.insert(
            job,
            Pending::EditorConfig {
                doc,
                seq,
                path,
                safe_dirs: written,
                reload,
            },
        );
    }

    /// 開いている、パスのある全文書の `.editorconfig` を読み直す (`config.reload`、EC-1)。届くまでは前の値を使う。
    pub(crate) fn reread_editorconfig_all(&mut self) {
        let ids: Vec<DocumentId> = self.docs.documents.keys().copied().collect();
        for id in ids {
            self.start_editorconfig_read(id, true);
        }
    }

    /// 名前を付けて保存で文書のパスが変わった: 前のパスで決まった値を外し、新しいパスで読み直す (EC-1。届くまでは (5) を
    /// 除いた値、LAYER-3)。
    pub(crate) fn editorconfig_on_path_changed(&mut self, doc: DocumentId) {
        if let Some(d) = self.docs.documents.get_mut(&doc) {
            d.editorconfig = None;
        }
        self.refresh_doc_settings(doc);
        self.start_editorconfig_read(doc, false);
    }

    /// `ReadEditorConfig` の結果 (07 §10.3 の EC-1 / EC-6、§3.6 の LAYER-3 / LAYER-4)。
    pub(crate) fn finish_editorconfig(
        &mut self,
        doc: DocumentId,
        seq: u64,
        path: &FsPath,
        safe_dirs: Vec<String>,
        reload: bool,
        r: JobResult,
    ) {
        // 古い結果: 後に積んだ読込がある (`config.reload`・名前を付けて保存・開き直し) か、文書のパスが変わった・閉じた
        if self.settings.editorconfig_latest.get(&doc) != Some(&seq) {
            return;
        }
        self.settings.editorconfig_latest.remove(&doc);
        let Some(d) = self.docs.documents.get(&doc) else {
            return;
        };
        if d.path.as_ref() != Some(path) {
            return;
        }
        let JobResult::EditorConfigRead(read) = r else {
            // ジョブが走らなかった (パニックなど): 値は前のまま
            return;
        };
        match read {
            EditorConfigRead::Busy { path: file } => {
                let before = d.editorconfig.as_ref().map(|e| e.safe_dirs.clone());
                match before {
                    // 前の値を保つ。`config.reload` では知らせる (知らせなく古い値が残らないように。WSC-1 と同じ)
                    Some(before) if before == safe_dirs => {
                        if reload {
                            self.show_message(
                                MessageLevel::Warn,
                                keys::MSG_CONFIG_RELOAD_KEPT,
                                &[("file", &file)],
                            );
                        }
                    }
                    // 安全な場所が変わった: 一時の失敗でも前の値を外す (逃げ道を外したのに古い値が残らないように)
                    Some(_) => {
                        if let Some(d) = self.docs.documents.get_mut(&doc) {
                            d.editorconfig = None;
                        }
                        self.show_message(
                            MessageLevel::Warn,
                            keys::MSG_WORKSPACE_NOT_REREAD,
                            &[("path", &file)],
                        );
                        self.refresh_doc_settings(doc);
                        self.refresh_new_file_format(doc);
                    }
                    None => {}
                }
            }
            EditorConfigRead::Done {
                values,
                reports,
                missing,
            } => {
                if let Some(d) = self.docs.documents.get_mut(&doc) {
                    d.editorconfig = (!values.is_empty()).then(|| EditorConfigApplied {
                        values: Arc::new(*values),
                        safe_dirs,
                    });
                }
                self.refresh_doc_settings(doc);
                self.refresh_new_file_format(doc);
                for rep in reports {
                    self.report_editorconfig(rep);
                }
                // 消えたファイルの診断は一覧から外す (最後に読んだときの診断、07 §4.3)
                for file in missing {
                    if self.settings.editorconfig_reported.remove(&file).is_some() {
                        self.record_config_diagnostics(&file, &[]);
                    }
                }
            }
        }
    }

    /// 1 つの `.editorconfig` の診断を一覧に置き、まだ要約していない中身なら要約を出す (同じファイルの同じ中身はセッションで
    /// 1 回、EC-6)。一覧は、最後に読んだ中身の診断に置き換える (別の中身が届いたら替え、同じなら触らない)。
    fn report_editorconfig(&mut self, rep: EditorConfigReport) {
        let EditorConfigReport {
            path,
            digest,
            diagnostics,
        } = rep;
        if self.settings.editorconfig_reported.get(&path) != Some(&digest) {
            self.settings
                .editorconfig_reported
                .insert(path.clone(), digest);
            self.record_config_diagnostics(&path, &diagnostics);
        }
        if self
            .settings
            .editorconfig_summarized
            .insert((path.clone(), digest))
        {
            self.summarize_config_diagnostics(&path, &diagnostics);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn ディスクの上の名前は大文字小文字の違いと短い名前だけを直す() {
        assert_eq!(
            align_disk_names(&v(&["w", "Src", "Main.rs"]), &v(&["W", "src", "main.rs"])),
            v(&["W", "src", "main.rs"])
        );
        assert_eq!(
            align_disk_names(
                &v(&["w", "SOURCE~1", "a.rs"]),
                &v(&["w", "sources", "a.rs"])
            ),
            v(&["w", "sources", "a.rs"])
        );
        // ジャンクションで名前が違う要素は開いたときの名前のまま
        assert_eq!(
            align_disk_names(
                &v(&["repo", "link", "a.rs"]),
                &v(&["other", "target", "a.rs"])
            ),
            v(&["repo", "link", "a.rs"])
        );
        // canonical の要素が少なくても末尾から揃える
        assert_eq!(
            align_disk_names(&v(&["a", "B.rs"]), &v(&["b.rs"])),
            v(&["a", "b.rs"])
        );
    }
}
