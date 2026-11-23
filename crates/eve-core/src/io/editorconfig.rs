//! `.editorconfig` の読込 (`ReadEditorConfig`。docs/07 §10.3 の EC-1〜EC-8、§3.6 の LAYER-1 の (5) と LAYER-3 / LAYER-4、
//! 01 FR-705 AC-3 / AC-4)。
//!
//! - **ジョブ** (`io`): 文書のディレクトリから上へ、各ディレクトリの `.editorconfig` を所有者の検査つきの読込
//!   (`FileSystem::read_checked`、`DirLinks::Follow`) で読み、読込・解析・glob の照合・重ね合わせまでを行う (EC-1 / EC-2 /
//!   EC-5)。祖先 1 段ごとに区切って譲る (`JobStep::Continue`。区切りの間に積んだ保存・読込が先に走る、EC-8)。主スレッドへは、
//!   重ねた値 (由来つき) と、読んだファイルごとの診断だけを返す。
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
use eve_view::{DocumentId, JobId, MessageLevel};

use crate::editor::state::EditorState;
use crate::io::fs::{CheckedRead, DirLinks, FileSystem, FsErrorKind};
use crate::io::fspath::FsPath;
use crate::io::pending::Pending;
use crate::jobs::{JobKind, JobOutput, JobPorts, JobResult, JobStep};
use crate::ui_model::keys;

/// 文書に当てた `.editorconfig` の値と、読んだときの `workspace.safe_directories` (書いた形)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditorConfigApplied {
    /// 重ねた値 (07 §3.6 の LAYER-1 の (5))。
    pub(crate) values: Arc<EditorConfigValues>,
    /// 読んだときの安全な場所 (`config.reload` で変わったら、一時の失敗でも前の値を保たない)。
    pub(crate) safe_dirs: Vec<String>,
}

/// `.editorconfig` の読込を積んだ契機 (結果の扱いが変わる、07 §10.3 の段 6 の細目)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorConfigTrigger {
    /// 読込の完了・名前を付けて保存でパスが変わった (値を持たずに読む)。`Busy` なら 1 回だけ積み直す。
    Open,
    /// 値を持たない文書の読込 (`Open`、または開いた直後に置き換わった `Reload`) が `Busy` で終わり、積み直した読込
    /// (もう積み直さない)。
    Retry,
    /// `config.reload` (届くまで前の値を使う)。`round` は読み直しの回 (知らせを同じファイルごとに 1 回にする)。値を
    /// 持たない文書で `Busy` なら 1 回だけ積み直す。
    Reload {
        /// 読み直しの回。
        round: u64,
    },
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

/// Windows の 8.3 の短い名前の形か (`^.{1,6}~[0-9]+(\..{0,3})?$`。`SOURCE~1`・`PROGRA~2.TXT`)。`my~notes` のような長い名前は
/// 当たらない。
fn is_short_name(name: &str) -> bool {
    let chars: Vec<char> = name.chars().collect();
    chars.iter().enumerate().any(|(i, c)| {
        if *c != '~' || !(1..=6).contains(&i) {
            return false;
        }
        let rest = &chars[i + 1..];
        let digits = rest.iter().take_while(|d| d.is_ascii_digit()).count();
        if digits == 0 {
            return false;
        }
        match &rest[digits..] {
            [] => true,
            ['.', ext @ ..] => ext.len() <= 3,
            _ => false,
        }
    })
}

/// 開いたときのパスの名前の要素を、ディスクの上の名前 (canonical のパスの、末尾から揃えた要素) に直す (Windows、EC-1)。
/// 直すのは、大文字小文字を除いて同じ名前 (ディスクの大文字小文字にする) と、8.3 の短い名前の形の名前だけ。
/// ジャンクションなどで名前が違う要素は、開いたときの名前のまま (置き場所からの相対パスは開いたパスで決まる)。
#[doc(hidden)]
#[must_use]
pub fn align_disk_names(opened: &[String], canonical: &[String]) -> Vec<String> {
    let mut out = opened.to_vec();
    for (o, c) in out.iter_mut().rev().zip(canonical.iter().rev()) {
        if o.to_lowercase() == c.to_lowercase() || is_short_name(o) {
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

/// `ReadEditorConfig` の途中の状態 (祖先 1 段ごとに区切って譲る、07 §10.3 の EC-8、02 §9.3)。
#[derive(Clone, Debug)]
pub struct EditorConfigWalk {
    /// glob に当てる名前の要素 (最初の区切りで決める)。
    names: Vec<String>,
    /// 次に見る祖先の段 (0 は文書のディレクトリ)。
    depth: usize,
    /// 近い順 (表示名, 読んだファイル, 置き場所から文書への相対パス)。
    loaded: Vec<(String, EditorConfigFile, String)>,
    reports: Vec<EditorConfigReport>,
    missing: Vec<String>,
    /// 読んだバイト数の合計 (使わなかったファイルも数える、EC-2)。
    total_bytes: u64,
    total_sections: usize,
}

/// 1 段を見た後。
enum WalkStep {
    /// 上の段が残っている (譲って続ける)。
    Yield,
    /// 探し終えた (`root`・合計の上限・ファイルシステムのルート・64 段・所有者の検査を持たない Port)。
    Done,
    /// 一時の失敗で探すのをやめた (読めなかったファイルのパス)。
    Busy(String),
}

impl EditorConfigWalk {
    fn new(fs: &dyn FileSystem, path: &FsPath) -> EditorConfigWalk {
        EditorConfigWalk {
            names: match_names(fs, path),
            depth: 0,
            loaded: Vec::new(),
            reports: Vec::new(),
            missing: Vec::new(),
            total_bytes: 0,
            total_sections: 0,
        }
    }

    /// 段 `depth` の祖先のディレクトリと、そこから文書への相対パス (無ければ探し終えた)。
    fn level(&self, path: &FsPath, depth: usize) -> Option<(FsPath, String)> {
        if depth >= EDITORCONFIG_MAX_DEPTH {
            return None;
        }
        let dir = path.as_path().ancestors().nth(depth + 1)?;
        let start = self.names.len().checked_sub(depth + 1)?;
        Some((FsPath::new(dir), self.names[start..].join("/")))
    }

    /// 読んだ分を合計に足す。合計の上限を超えたら、そのファイルを合計の上限の診断にして `true` (より上を読まない、EC-2)。
    fn exceeds_total(&mut self, name: &str, bytes: u64, sections: usize, data: &[u8]) -> bool {
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.total_sections = self.total_sections.saturating_add(sections);
        if self.total_bytes <= EDITORCONFIG_TOTAL_MAX_BYTES
            && self.total_sections <= EDITORCONFIG_TOTAL_MAX_SECTIONS
        {
            return false;
        }
        self.reports.push(report(
            name,
            digest("total_limit", data),
            vec![not_read_diagnostic(NotReadReason::TotalLimit, name)],
        ));
        true
    }

    /// 祖先 1 段の `.editorconfig` を見る。
    fn step(&mut self, fs: &dyn FileSystem, path: &FsPath, safe_dirs: &[FsPath]) -> WalkStep {
        let Some((dir, rel)) = self.level(path, self.depth) else {
            return WalkStep::Done;
        };
        self.depth += 1;
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
            Ok(CheckedRead::Missing) => self.missing.push(name),
            // 大きすぎる: 読んだ分 (上限 + 1 バイト) を合計に数え、そのファイルだけを飛ばして上へ (`root` は見ない、EC-2)
            Ok(CheckedRead::TooLarge) => {
                if self.exceeds_total(&name, EDITORCONFIG_FILE_MAX_BYTES + 1, 0, &[]) {
                    return WalkStep::Done;
                }
                self.reports.push(report(
                    &name,
                    digest("too_large", &[]),
                    vec![not_read_diagnostic(NotReadReason::TooLarge, &name)],
                ));
            }
            // 所有者の検査で読まなかった: そのファイルだけを飛ばして上へ (そのファイルの `root` は見ない、EC-2)
            Ok(CheckedRead::Rejected { what, verdict }) => {
                let d =
                    crate::io::workspace_config::rejected(what, verdict, &dir, &file, safe_dirs);
                let key = format!("{d:?}");
                self.reports
                    .push(report(&name, digest("rejected", key.as_bytes()), vec![d]));
            }
            Ok(CheckedRead::Read { bytes }) => {
                let len = bytes.len() as u64;
                let text = match String::from_utf8(bytes) {
                    Ok(t) => t,
                    // UTF-8 でない: 読んだ長さを合計に数え、そのファイルだけを飛ばして上へ (EC-2)
                    Err(e) => {
                        if self.exceeds_total(&name, len, 0, e.as_bytes()) {
                            return WalkStep::Done;
                        }
                        self.reports.push(report(
                            &name,
                            digest("not_utf8", e.as_bytes()),
                            vec![not_read_diagnostic(NotReadReason::NotUtf8, &name)],
                        ));
                        return self.next(path);
                    }
                };
                let parsed = parse_editorconfig(&text);
                // 1 つの文書で読む量の上限: このファイルとより上のファイルを使わない
                if self.exceeds_total(&name, len, parsed.section_count(), text.as_bytes()) {
                    return WalkStep::Done;
                }
                self.reports.push(report(
                    &name,
                    digest("read", text.as_bytes()),
                    parsed.diagnostics().to_vec(),
                ));
                let root = parsed.root();
                self.loaded.push((name, parsed, rel));
                if root {
                    return WalkStep::Done;
                }
            }
            Err(e) => match e.kind {
                FsErrorKind::Busy => return WalkStep::Busy(name),
                // 所有者の検査を持たない Port: 読まない (診断なし)
                FsErrorKind::Unsupported => return WalkStep::Done,
                // 開けない祖先 (権限など) は黙って次へ (07 §12.6.3 の段 4 から送った (1))
                _ => {}
            },
        }
        self.next(path)
    }

    /// 上の段が残っていれば譲る。
    fn next(&self, path: &FsPath) -> WalkStep {
        if self.level(path, self.depth).is_some() {
            WalkStep::Yield
        } else {
            WalkStep::Done
        }
    }

    /// 集めたファイルを重ねる (遠い順、EC-5)。
    fn finish(self) -> EditorConfigRead {
        let sources: Vec<EditorConfigSource<'_>> = self
            .loaded
            .iter()
            .rev()
            .map(|(n, f, r)| (n.as_str(), f, r.as_str()))
            .collect();
        EditorConfigRead::Done {
            values: Box::new(resolve_editorconfig(&sources)),
            reports: self.reports,
            missing: self.missing,
        }
    }
}

/// `ReadEditorConfig` を 1 区切り (祖先 1 段) 進める。上の段が残っていれば、状態を持った `JobKind::ReadEditorConfig` を
/// `JobStep::Continue` で返す (レーンが同じ `JobId`・`CancelToken` のまま自分のキューの末尾へ積む。取り消されていれば
/// 続きを積まない、02 §9.3)。終われば `JobResult::EditorConfigRead` を送る (07 §10.3 の EC-8)。
pub(crate) fn run_editorconfig_step(
    job: JobId,
    path: FsPath,
    safe_dirs: Arc<[FsPath]>,
    walk: Option<Box<EditorConfigWalk>>,
    ports: &JobPorts,
    emit: &mut dyn FnMut(JobOutput),
) -> JobStep {
    let fs = &*ports.fs;
    let mut walk = walk.unwrap_or_else(|| Box::new(EditorConfigWalk::new(fs, &path)));
    let body = match walk.step(fs, &path, &safe_dirs) {
        WalkStep::Yield => {
            return JobStep::Continue(JobKind::ReadEditorConfig {
                path,
                safe_dirs,
                walk: Some(walk),
            });
        }
        WalkStep::Busy(file) => EditorConfigRead::Busy { path: file },
        WalkStep::Done => walk.finish(),
    };
    emit(JobOutput {
        job,
        body: JobResult::EditorConfigRead(body),
    });
    JobStep::Done
}

/// `.editorconfig` を集めて重ねる (ジョブの本体を区切らずに最後まで回したもの。07 §10.3 の EC-1 / EC-2 / EC-5)。`path` は
/// 文書の開いたときの絶対パス、`safe_dirs` は `workspace.safe_directories`。試験と性質の検査が使う。
#[doc(hidden)]
pub fn read_editorconfig(
    fs: &dyn FileSystem,
    path: &FsPath,
    safe_dirs: &[FsPath],
) -> EditorConfigRead {
    let mut walk = EditorConfigWalk::new(fs, path);
    loop {
        match walk.step(fs, path, safe_dirs) {
            WalkStep::Yield => {}
            WalkStep::Busy(file) => return EditorConfigRead::Busy { path: file },
            WalkStep::Done => return walk.finish(),
        }
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
    /// 読むかはパスの有無だけで決める (無題の文書は読まない。標準入力の文書は名前を付けて保存でパスを持ってから読む)。
    /// 読まない起動では積まない。前に積んだ読込の結果は、届いても捨てる (後のものが勝つ)。
    pub(crate) fn start_editorconfig_read(
        &mut self,
        doc: DocumentId,
        trigger: EditorConfigTrigger,
    ) {
        if !self.editorconfig_enabled() {
            return;
        }
        let Some(path) = self.docs.documents.get(&doc).and_then(|d| d.path.clone()) else {
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
                walk: None,
            },
        );
        self.io.pending.insert(
            job,
            Pending::EditorConfig {
                doc,
                seq,
                path,
                safe_dirs: written,
                trigger,
            },
        );
    }

    /// 開いている、パスのある全文書の `.editorconfig` を読み直す (`config.reload`、EC-1)。届くまでは前の値を使う。
    pub(crate) fn reread_editorconfig_all(&mut self) {
        self.settings.editorconfig_round += 1;
        self.settings.editorconfig_round_noted.clear();
        let round = self.settings.editorconfig_round;
        let ids: Vec<DocumentId> = self.docs.documents.keys().copied().collect();
        for id in ids {
            self.start_editorconfig_read(id, EditorConfigTrigger::Reload { round });
        }
    }

    /// 名前を付けて保存で文書のパスが変わった: 前のパスで決まった値を外し、新しいパスで読み直す (EC-1。届くまでは (5) を
    /// 除いた値、LAYER-3)。
    pub(crate) fn editorconfig_on_path_changed(&mut self, doc: DocumentId) {
        if let Some(d) = self.docs.documents.get_mut(&doc) {
            d.editorconfig = None;
        }
        self.refresh_doc_settings(doc);
        self.start_editorconfig_read(doc, EditorConfigTrigger::Open);
    }

    /// 前に当てた値が、今回の読込と違う `workspace.safe_directories` で読んだものか (一時の失敗でも前の値を保たない)。
    fn editorconfig_safe_dirs_changed(&self, doc: DocumentId, safe_dirs: &[String]) -> bool {
        self.docs
            .documents
            .get(&doc)
            .and_then(|d| d.editorconfig.as_ref())
            .is_some_and(|e| e.safe_dirs != safe_dirs)
    }

    /// 前に当てた値を外し、文書ごとの値と新しいファイルの形を決め直す。
    fn drop_editorconfig(&mut self, doc: DocumentId) {
        if let Some(d) = self.docs.documents.get_mut(&doc) {
            d.editorconfig = None;
        }
        self.refresh_doc_settings(doc);
        self.refresh_new_file_format(doc);
    }

    /// 読み直しの知らせ (`key` の `{arg}` に `file`)。`config.reload` の読込なら、その回の中で同じ知らせ・同じファイルは
    /// 1 回だけ出す (同じディレクトリの文書の数だけ出さない、07 §10.3 の段 6 の細目)。
    fn note_editorconfig_reread(
        &mut self,
        trigger: EditorConfigTrigger,
        key: &'static str,
        arg: &str,
        file: &str,
    ) {
        if let EditorConfigTrigger::Reload { round } = trigger
            && round == self.settings.editorconfig_round
            && !self
                .settings
                .editorconfig_round_noted
                .insert((key, file.to_owned()))
        {
            return;
        }
        self.show_message(MessageLevel::Warn, key, &[(arg, file)]);
    }

    /// `ReadEditorConfig` の結果 (07 §10.3 の EC-1 / EC-2 / EC-6、§3.6 の LAYER-3 / LAYER-4)。
    pub(crate) fn finish_editorconfig(
        &mut self,
        doc: DocumentId,
        seq: u64,
        path: &FsPath,
        safe_dirs: Vec<String>,
        trigger: EditorConfigTrigger,
        r: JobResult,
    ) {
        // 古い結果: 後に積んだ読込がある (`config.reload`・名前を付けて保存・積み直し) か、文書のパスが変わった・閉じた
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
            // ジョブが結果を返さなかった (パニックなど): `Busy` と同じく、安全な場所が変わっていれば前の値を外す (知らせる
            // ファイルが分からないので知らせない)。変わっていなければ前のまま
            if self.editorconfig_safe_dirs_changed(doc, &safe_dirs) {
                self.drop_editorconfig(doc);
            }
            return;
        };
        match read {
            EditorConfigRead::Busy { path: file } => {
                let had_value = d.editorconfig.is_some();
                if self.editorconfig_safe_dirs_changed(doc, &safe_dirs) {
                    // 安全な場所が変わった: 一時の失敗でも前の値を外す (逃げ道を外したのに古い値が残らないように)
                    self.drop_editorconfig(doc);
                    self.note_editorconfig_reread(
                        trigger,
                        keys::MSG_WORKSPACE_NOT_REREAD,
                        "path",
                        &file,
                    );
                } else if had_value {
                    // 前の値を保つ。`config.reload` では知らせる (知らせなく古い値が残らないように。WSC-1 と同じ)
                    if matches!(trigger, EditorConfigTrigger::Reload { .. }) {
                        self.note_editorconfig_reread(
                            trigger,
                            keys::MSG_CONFIG_RELOAD_BUSY_KEPT,
                            "file",
                            &file,
                        );
                    }
                } else if trigger != EditorConfigTrigger::Retry {
                    // 値を持たない文書の一時の失敗 (開いた直後の読込が `config.reload` に置き換わった場合も): 1 回だけ積み直す
                    // (番号を新しくする)。2 回目も `Busy` なら値を持たない
                    self.start_editorconfig_read(doc, EditorConfigTrigger::Retry);
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
    /// 1 回、EC-6)。一覧は、最後に読んだ中身の診断に置き換える (別の中身が届いたら替え、同じなら触らない)。要約は非同期に
    /// 届くので、表示中の警告・エラーを置き換えない (§10.3 の段 6 の細目)。
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

    #[test]
    fn 短い名前の形だけを直し_チルダを含む長い名前はジャンクションの先の名前にしない() {
        for short in [
            "SOURCE~1",
            "a~1",
            "PROGRA~12",
            "DOCUME~1.TXT",
            "ab~3.x",
            "x~9.",
        ] {
            assert!(is_short_name(short), "{short}");
        }
        for long in [
            "my~notes",
            "~1",
            "SEVENCH~1",
            "a~",
            "a~x1",
            "a~1.long",
            "a~1x",
            "notes~",
            "~",
        ] {
            assert!(!is_short_name(long), "{long}");
        }
        // ジャンクションの先の名前が違う、`~` を含む長い名前は替えない
        assert_eq!(
            align_disk_names(&v(&["w", "my~notes", "a.rs"]), &v(&["w", "target", "a.rs"])),
            v(&["w", "my~notes", "a.rs"])
        );
        // 短い名前は替える (大文字小文字を問わない)
        assert_eq!(
            align_disk_names(
                &v(&["w", "progra~1", "a.rs"]),
                &v(&["w", "Program Files", "a.rs"])
            ),
            v(&["w", "Program Files", "a.rs"])
        );
    }
}
