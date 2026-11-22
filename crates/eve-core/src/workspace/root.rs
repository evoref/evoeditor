//! ワークスペースのルートの検出 (docs/03 §12.7.1、規則の正本は 07 §12.1) と、起動の引数のディレクトリの知らせ (07 §18.2)。
//!
//! - **ジョブ** (`JobKind::WorkspaceRoot`、`io`): 起動のたびに 1 回、起動時のパスを開くジョブの後ろに積む。`io` は直列なので
//!   最初の文書の読込を遅らせず、最初の Frame も待たない (01 FR-125 AC-3)。規則は [`detect_root`]。
//! - **知らせの保留**: 起動時のパスがディレクトリで開くジョブが失敗しても、`msg.open.not_a_file` を出さずに結果を待つ。
//!   結果が届いたら、ルートにしたディレクトリには何も出さず、それ以外のディレクトリには `msg.cli.directory_ignored` を出す。
//!   `--workspace` を使えなかったときは `msg.workspace.root_invalid` を出す。
//!
//! ルートの検出は目印の有無を見るだけで、ワークスペース設定を読まない (03 §12.7.1 の「所有者の検査はしない」)。
//!
//! spec: 03 §12.7.1, 07 §12.1, 07 §18.2, 07 §18.3

use std::collections::BTreeSet;
use std::path::{Component, PathBuf};

use eve_view::{DocumentId, MessageLevel};

use crate::editor::state::EditorState;
use crate::io::error_keys::fs_reason;
use crate::io::fs::{FileKind, FileSystem, FsError, FsErrorDetail, FsErrorKind};
use crate::io::fspath::FsPath;
use crate::io::pending::Pending;
use crate::jobs::{JobError, JobKind, JobResult};
use crate::ui_model::keys;

/// ルートの検出の目印の既定 (07 §5.9 の `workspace.root_markers` の既定値)。
///
/// 設定のキーはまだ公開していない (予約のまま。公開は 03 §12.7.9 の段の表)。公開するまでは、この値で探す。
pub(crate) const DEFAULT_ROOT_MARKERS: [&str; 4] = [".eve", ".git", ".hg", ".svn"];

/// どの規則でルートが決まったか (07 §12.1 の 1〜4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RootSource {
    /// `--workspace` / `-w` で明示したディレクトリ (規則 1)。
    Explicit,
    /// 位置引数の最初のディレクトリ (規則 2)。
    Dir,
    /// 起点から上へ探して見つけた目印のあるディレクトリ (規則 3)。
    Marker,
    /// カレントディレクトリ (規則 4)。
    Cwd,
}

/// 決まったワークスペース (1 プロセスに 1 つ、07 §12.1)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    root: FsPath,
    source: RootSource,
}

impl Workspace {
    /// ルート (canonicalize して 05 §3.3 の表示用の形にしたもの。できなければ正規化しただけのパス)。
    pub fn root(&self) -> &FsPath {
        &self.root
    }

    /// どの規則で決まったか。
    pub fn source(&self) -> RootSource {
        self.source
    }
}

/// core が持つワークスペースの状態 (`EditorState::workspace`)。
#[derive(Debug, Default)]
pub(crate) struct WorkspaceState {
    /// 決まったワークスペース。結果が届くまでは `None`。
    current: Option<Workspace>,
    /// `--workspace` の値 (入力されたまま)。ジョブを投入するときに取り出す。
    explicit_arg: Option<String>,
    /// `msg.workspace.root_invalid` の `{path}` (正規化したパスの表示。正規化できなければ入力されたまま)。
    explicit_shown: Option<String>,
    /// `--workspace` の値を正規化できなかった理由 (ジョブには渡さず、結果が届いた時点で知らせる)。
    explicit_invalid: Option<FsError>,
    /// 起動時のパスを開いた文書 (開くジョブがディレクトリで失敗したら、知らせを保留する。失敗の結果で外す。起動時の
    /// 引数の数だけなので、読み終えた文書は外さない)。
    startup_docs: BTreeSet<DocumentId>,
    /// 結果が届くまで保留した、ディレクトリだった起動時のパス (正規化したもの)。
    held_dirs: Vec<FsPath>,
    /// 届いた結果のうち、知らせに使うもの。
    settled: Option<Settled>,
    /// ファインダの候補の一覧と走査 (03 §12.7.2 の「一覧の寿命」)。
    pub(crate) finder: crate::workspace::finder::FinderList,
}

/// 届いた結果のうち、起動の引数のディレクトリの知らせに使うもの。
#[derive(Debug)]
struct Settled {
    /// ディレクトリだった位置引数 (正規化したもの)。
    dir_args: Vec<FsPath>,
    /// ルートにした位置引数 (規則 2 で決まったとき)。
    root_arg: Option<FsPath>,
    /// ルートの表示 (結果が失敗だったときは `None`)。
    root_shown: Option<String>,
}

/// ルートを検出する (07 §12.1 の規則 1〜4。`JobKind::WorkspaceRoot` の本体)。
///
/// `explicit` は `--workspace` の値、`args` は位置引数 (正規化したパス。`-` を除く)、`cwd` はカレントディレクトリ、
/// `markers` は目印の名前。I/O は位置引数と `--workspace` と目印の `metadata`、ルートの `canonicalize` だけ。
pub(crate) fn detect_root(
    fs: &dyn FileSystem,
    explicit: Option<&FsPath>,
    args: &[FsPath],
    cwd: &FsPath,
    markers: &[String],
) -> JobResult {
    // 位置引数を、ディレクトリ (DIR) とそれ以外 (FILE。無いパス・読めないパスを含む) に分ける
    let (dir_args, files): (Vec<FsPath>, Vec<FsPath>) = args
        .iter()
        .cloned()
        .partition(|a| fs.metadata(a).is_ok_and(|m| m.kind == FileKind::Dir));
    // 1. `--workspace`
    let mut explicit_error = None;
    let mut chosen = None;
    if let Some(e) = explicit {
        match fs.metadata(e) {
            Ok(m) if m.kind == FileKind::Dir => chosen = Some((e.clone(), RootSource::Explicit)),
            Ok(_) => explicit_error = Some(not_a_directory()),
            Err(err) => explicit_error = Some(err),
        }
    }
    // 2. 位置引数の最初のディレクトリ
    if chosen.is_none()
        && let Some(d) = dir_args.first()
    {
        chosen = Some((d.clone(), RootSource::Dir));
    }
    // 3. 起点から上へ目印を探す。4. 見つからなければカレントディレクトリ
    let (root, source) = chosen.unwrap_or_else(|| {
        let parents: Vec<FsPath> = files.iter().filter_map(FsPath::parent).collect();
        let origin = common_ancestor(&parents).unwrap_or_else(|| cwd.clone());
        match find_marker(fs, &origin, markers) {
            Some(dir) => (dir, RootSource::Marker),
            None => (cwd.clone(), RootSource::Cwd),
        }
    });
    let root = fs.canonicalize(&root).unwrap_or(root);
    JobResult::WorkspaceRoot {
        root,
        source,
        dir_args,
        explicit_error,
    }
}

/// `--workspace` がディレクトリでなかったときの理由 (`{reason}` は `not a directory`、[`root_invalid_reason`])。
fn not_a_directory() -> FsError {
    FsError::with_detail(FsErrorKind::InvalidPath, FsErrorDetail::NotAFile)
}

/// `msg.workspace.root_invalid` の `{reason}`。ディレクトリでなかったときは `not a directory`、ほかは種類の名前。
fn root_invalid_reason(e: &FsError) -> String {
    match (&e.kind, &e.detail) {
        (FsErrorKind::InvalidPath, Some(FsErrorDetail::NotAFile)) => "not a directory".to_owned(),
        _ => fs_reason(e),
    }
}

/// パスの並びの共通の祖先 (構文で比べる)。ドライブ・UNC の共有が違って共通の祖先が無ければ最初のもの。空なら `None`。
fn common_ancestor(paths: &[FsPath]) -> Option<FsPath> {
    let (first, rest) = paths.split_first()?;
    let mut common: Vec<Component<'_>> = first.as_path().components().collect();
    for p in rest {
        let same = common
            .iter()
            .zip(p.as_path().components())
            .take_while(|(a, b)| **a == *b)
            .count();
        common.truncate(same);
    }
    let common: PathBuf = common.iter().collect();
    if common.has_root() {
        Some(FsPath::new(common))
    } else {
        Some(first.clone())
    }
}

/// `origin` から上へ、各ディレクトリで目印のどれかの名前の項目 (ファイルでもディレクトリでもよい) を探し、最初に見つかった
/// ディレクトリを返す。上への探索はファイルシステムのルートで止める (ホームディレクトリでは止めない)。
fn find_marker(fs: &dyn FileSystem, origin: &FsPath, markers: &[String]) -> Option<FsPath> {
    if markers.is_empty() {
        return None;
    }
    let mut dir = Some(origin.clone());
    while let Some(d) = dir {
        if markers.iter().any(|m| fs.metadata(&d.join(m)).is_ok()) {
            return Some(d);
        }
        dir = d.parent();
    }
    None
}

impl WorkspaceState {
    /// 決まったワークスペース (結果が届くまでは `None`)。
    pub(crate) fn current(&self) -> Option<&Workspace> {
        self.current.as_ref()
    }

    /// `--workspace` の値を覚える (起動の前に Host が渡す)。
    pub(crate) fn set_explicit_arg(&mut self, raw: Option<String>) {
        self.explicit_arg = raw;
    }

    /// 起動時のパスを開いた文書を覚える (ディレクトリで失敗したら知らせを保留する)。
    pub(crate) fn note_startup_doc(&mut self, doc: DocumentId) {
        self.startup_docs.insert(doc);
    }
}

impl EditorState {
    /// 決まったワークスペース (ルートの検出の結果が届くまでは `None`。03 §12.7.1)。
    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspace.current()
    }

    /// ルートの検出のジョブを投入する (起動時に 1 回。起動時のパスを開くジョブの後、退避の走査の前に呼ぶ)。`args` は
    /// 起動時のパス (入力されたまま。`-` は含めない)。
    pub(crate) fn start_workspace_root(&mut self, args: &[&str]) {
        let explicit = match self.workspace.explicit_arg.take() {
            Some(raw) => match self.paths.normalize(&raw) {
                Ok(p) => {
                    self.workspace.explicit_shown = Some(p.display());
                    Some(p)
                }
                Err(e) => {
                    self.workspace.explicit_shown = Some(raw);
                    self.workspace.explicit_invalid = Some(e);
                    None
                }
            },
            None => None,
        };
        let args: Vec<FsPath> = args
            .iter()
            .filter_map(|a| self.paths.normalize(a).ok())
            .collect();
        // カレントディレクトリ (`Paths` は起動時に解決した作業ディレクトリを基準に正規化する)
        let cwd = self
            .paths
            .normalize(".")
            .unwrap_or_else(|_| FsPath::from("."));
        let markers = DEFAULT_ROOT_MARKERS.map(str::to_owned).to_vec();
        let job = self.jobs.spawn(
            None,
            eve_view::Revision::INITIAL,
            JobKind::WorkspaceRoot {
                explicit,
                args,
                cwd,
                markers,
            },
        );
        self.io.pending.insert(job, Pending::WorkspaceRoot);
    }

    /// 起動時のパスを開くジョブがディレクトリで失敗したら、知らせを保留する (結果が届いていれば、その場で出す)。保留したら
    /// `true` (呼び側は `msg.open.not_a_file` を出さずに文書を閉じる)。
    pub(crate) fn hold_startup_dir(
        &mut self,
        doc: DocumentId,
        path: &FsPath,
        err: &JobError,
    ) -> bool {
        if !self.workspace.startup_docs.remove(&doc) {
            return false;
        }
        let not_a_file = matches!(
            err,
            JobError::Fs(FsError {
                detail: Some(FsErrorDetail::NotAFile),
                ..
            })
        );
        if !not_a_file {
            return false;
        }
        if self.workspace.settled.is_some() {
            self.notify_startup_dir(path);
        } else {
            self.workspace.held_dirs.push(path.clone());
        }
        true
    }

    /// ルートの検出の結果を受け取る: ワークスペースを決め、保留した知らせを出す。
    pub(crate) fn finish_workspace_root(&mut self, r: JobResult) {
        let explicit_error = match r {
            JobResult::WorkspaceRoot {
                root,
                source,
                dir_args,
                explicit_error,
            } => {
                let root_arg = dir_args
                    .first()
                    .filter(|_| source == RootSource::Dir)
                    .cloned();
                self.workspace.settled = Some(Settled {
                    dir_args,
                    root_arg,
                    root_shown: Some(root.display()),
                });
                self.workspace.current = Some(Workspace { root, source });
                explicit_error.or_else(|| self.workspace.explicit_invalid.take())
            }
            // ジョブが走らなかった (パニックなど): ルートは決まらない。保留した知らせは v0.4 までと同じ文言で出す
            _ => {
                self.workspace.settled = Some(Settled {
                    dir_args: Vec::new(),
                    root_arg: None,
                    root_shown: None,
                });
                None
            }
        };
        for path in std::mem::take(&mut self.workspace.held_dirs) {
            self.notify_startup_dir(&path);
        }
        // ファインダ: 待っていれば走査を始め、`DIR` で決まったら開く (03 §12.7.1、07 §18.2)
        if let Some(source) = self.workspace.current.as_ref().map(Workspace::source) {
            self.finder_on_root(source);
        }
        // `--workspace` を使えなかった知らせは、利用者が明示した指定なので最後に出す (現在のメッセージにする)
        if let (Some(e), Some(root)) = (explicit_error, self.workspace.current.as_ref()) {
            let root = root.root.display();
            let path = self.workspace.explicit_shown.take().unwrap_or_default();
            let reason = root_invalid_reason(&e);
            self.show_message(
                MessageLevel::Warn,
                keys::MSG_WORKSPACE_ROOT_INVALID,
                &[("path", &path), ("reason", &reason), ("root", &root)],
            );
        }
    }

    /// ディレクトリだった起動時のパス 1 つの知らせ: ルートにしたものには出さず、ほかのディレクトリには
    /// `msg.cli.directory_ignored`。ディレクトリでなかった (FIFO・デバイスなど) ときは `msg.open.not_a_file`。
    fn notify_startup_dir(&mut self, path: &FsPath) {
        let Some(s) = self.workspace.settled.as_ref() else {
            return;
        };
        if s.root_arg.as_ref() == Some(path) {
            return;
        }
        let shown = path.display();
        match (&s.root_shown, s.dir_args.contains(path)) {
            (Some(root), true) => {
                let root = root.clone();
                self.show_message(
                    MessageLevel::Warn,
                    keys::MSG_CLI_DIRECTORY_IGNORED,
                    &[("path", &shown), ("root", &root)],
                );
            }
            _ => self.show_message(
                MessageLevel::Error,
                keys::MSG_OPEN_NOT_A_FILE,
                &[("path", &shown), ("reason", "invalidpath")],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> FsPath {
        FsPath::from(s)
    }

    #[test]
    fn 共通の祖先は構文で比べる() {
        assert_eq!(common_ancestor(&[]), None);
        assert_eq!(common_ancestor(&[p("/a/b")]), Some(p("/a/b")));
        assert_eq!(
            common_ancestor(&[p("/a/b/c"), p("/a/b/d"), p("/a/b")]),
            Some(p("/a/b"))
        );
        assert_eq!(common_ancestor(&[p("/a/x"), p("/b/y")]), Some(p("/")));
    }

    #[cfg(windows)]
    #[test]
    fn ドライブが違えば最初のもの() {
        assert_eq!(
            common_ancestor(&[p(r"C:\a\b"), p(r"D:\a\b")]),
            Some(p(r"C:\a\b"))
        );
        assert_eq!(
            common_ancestor(&[p(r"\\srv\share\a"), p(r"\\srv\other\a")]),
            Some(p(r"\\srv\share\a"))
        );
        assert_eq!(
            common_ancestor(&[p(r"C:\w\a"), p(r"C:\w\b")]),
            Some(p(r"C:\w"))
        );
    }

    #[test]
    fn ディレクトリでなかった理由() {
        assert_eq!(root_invalid_reason(&not_a_directory()), "not a directory");
        assert_eq!(
            root_invalid_reason(&FsError::new(FsErrorKind::NotFound)),
            "notfound"
        );
        assert_eq!(
            root_invalid_reason(&FsError::new(FsErrorKind::PermissionDenied)),
            "permissiondenied"
        );
    }
}
