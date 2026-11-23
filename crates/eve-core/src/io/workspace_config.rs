//! ワークスペース層の読込 (`ReadWorkspaceConfig`。docs/07 §12.5 の WSC-1〜WSC-7、§12.1 の OWN-1〜OWN-7、01 FR-705)。
//!
//! - **ジョブ** (`io`): `<root>/.eve/config.toml` と `<root>/.eve/keymap.toml` を所有者の検査つきの読込
//!   (`FileSystem::read_checked`。`.eve` から先のリンクを辿らない) で 256 KiB まで読み、解析・条件付きセクションの解決・
//!   検証までをジョブの中で行う (WSC-2。他人のリポジトリの大きな・入れ子の深いファイルの解析で入力を止めない)。
//!   `.eve/languages/`・`.eve/themes/`・`.eve/queries/` は、あれば「このバージョンでは未対応」の情報診断だけ。
//! - **主スレッド**: 届いた層を、今のユーザー層と比べて分類し (07 §12.3。表の比較だけ)、ユーザー層の上・コマンドライン層の
//!   下に重ね直す (WSC-3)。ユーザー層を読み直したときも、そのユーザー層で分類し直す。読み直しは `config.reload` と同じ
//!   経路 (ユーザー層を読み直した後にこのジョブを積み、届いてから全体を差し替えて要約を 1 回出す)。
//!
//! 所有者の検査で読まなかったファイルは、ファイルごとに 1 件の警告 (OWN-7)。一時の失敗 (`Busy`) は診断を出さず、前に読んだ
//! 層とその診断をそのまま使う (次の読込で読み直す)。`config.reload` で構文エラー・読めないファイルは、前に読んだ層を保って
//! `msg.config.reload_kept` を出す (07 §6 の規則 1)。
//!
//! spec: 07 §12.5, 07 §12.1, 07 §12.3, 07 §7.4, 07 §6

use std::sync::Arc;

use eve_config::{DiagKind, Diagnostic, KeymapFile, Severity, WorkspaceLayer, WorkspaceProblem};
use eve_view::MessageLevel;

use crate::editor::state::EditorState;
use crate::io::config_reload::ReloadPlan;
use crate::io::error_keys::fs_reason;
use crate::io::fs::{
    CheckedObject, CheckedRead, DirLinks, FileSystem, FsErrorKind, OwnerVerdict,
    WORKSPACE_CONFIG_DIR,
};
use crate::io::fspath::FsPath;
use crate::io::pending::Pending;
use crate::jobs::{JobKind, JobResult};
use crate::ui_model::keys;

/// ワークスペース設定のファイルの名前 (`<root>/.eve/config.toml`、07 §12.1)。
pub const WORKSPACE_CONFIG_FILE: &str = "config.toml";
/// ワークスペース層のキーマップのファイルの名前 (`<root>/.eve/keymap.toml`、07 §7.4 の WKEY-1)。
pub const WORKSPACE_KEYMAP_FILE: &str = "keymap.toml";
/// 1 ファイルの大きさの上限 (バイト。07 §12.5 の WSC-1)。
pub const WORKSPACE_FILE_MAX_BYTES: u64 = 256 * 1024;
/// v0.5 では読まない `.eve` の下のディレクトリ (あれば未対応の情報診断を 1 件ずつ、WSC-1)。
const UNSUPPORTED_DIRS: [&str; 3] = ["languages", "themes", "queries"];

/// 1 つのファイルの読込の結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetched<T> {
    /// 無い・所有者の検査で読まなかった (層を外す)。
    Absent,
    /// 読んだ。
    Got(T),
    /// 構文エラー・UTF-8 でない (起動時は層を持たない。`config.reload` では前に読んだ層を保つ、07 §6 の規則 1)。
    Broken,
    /// 一時の失敗 (`Busy`): 前に読んだ層と診断を使い続ける。
    Keep,
}

impl<T> Fetched<T> {
    fn keeps_diagnostics(&self) -> bool {
        matches!(self, Fetched::Keep)
    }
}

/// `ReadWorkspaceConfig` の結果。
#[derive(Clone, Debug)]
pub struct WorkspaceRead {
    /// `<root>/.eve/config.toml` (診断と由来の表示名は、これの `display`)。
    pub config_path: FsPath,
    /// `<root>/.eve/keymap.toml`。
    pub keymap_path: FsPath,
    /// `.eve/config.toml` を読んだ層 (分類は主スレッドで重ねるときに行う)。
    pub layer: Fetched<WorkspaceLayer>,
    /// `.eve/keymap.toml` (検証は主スレッドで、下の層と比べる)。
    pub keymap: Fetched<KeymapFile>,
    /// `.eve/config.toml` の名前で出す、層の外の診断 (読まなかった理由・構文エラー・読めない置き場所の情報)。分類の診断は
    /// 重ねるときに足す。
    pub config_notes: Vec<Diagnostic>,
    /// `.eve/keymap.toml` の名前で出す、読まなかった理由と構文エラー (読めたときの診断は、キーマップを当てるときに出す)。
    pub keymap_notes: Vec<Diagnostic>,
}

/// 位置の無い (ファイル全体の) 診断 1 件。
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

/// `dir` が安全な場所 (`workspace.safe_directories`) のどれかか、その下か (書いた形で比べる。区切りと、Windows では ASCII の
/// 大文字小文字を区別しない)。検査を省くかは Port が開いた後の最終のパスで決める (07 §12.1 の OWN-5) ので、ここは診断の
/// 文言を選ぶのにだけ使う。
fn written_under(dir: &FsPath, safe_dirs: &[FsPath]) -> bool {
    let d = components(dir);
    safe_dirs.iter().any(|s| {
        let s = components(s);
        !s.is_empty() && d.len() >= s.len() && d[..s.len()] == s[..]
    })
}

/// 比べるためのパスの要素 (Windows は ASCII の小文字にする)。
fn components(p: &FsPath) -> Vec<String> {
    p.as_path()
        .components()
        .map(|c| {
            let s = c.as_os_str().to_string_lossy().into_owned();
            if cfg!(windows) {
                s.to_ascii_lowercase()
            } else {
                s
            }
        })
        .collect()
}

/// 読まなかった理由の診断 (07 §12.1 の OWN-7)。`value` は判定に落ちた物 (置き場所のディレクトリかファイル) のパス。
fn rejected(
    what: CheckedObject,
    verdict: OwnerVerdict,
    dir: &FsPath,
    file: &FsPath,
    safe_dirs: &[FsPath],
) -> Diagnostic {
    let path = match what {
        CheckedObject::Dir => dir.display(),
        CheckedObject::File => file.display(),
    };
    let problem = match verdict {
        OwnerVerdict::NotOwner { owner } => WorkspaceProblem::NotOwned { owner },
        OwnerVerdict::OthersCanWrite { who } => WorkspaceProblem::OthersCanWrite {
            who: who.join(", "),
        },
        // 安全な場所の下で開いた物を確かめられなかった: 安全な場所へ足すことを勧めない (07 §12.6.3 の段 4 から送った (2))
        OwnerVerdict::Unknown { fs } if fs == "query_failed" && written_under(dir, safe_dirs) => {
            WorkspaceProblem::OpenedElsewhere
        }
        OwnerVerdict::Unknown { fs } => WorkspaceProblem::OwnerUnknown { fs },
        OwnerVerdict::NotRegular => WorkspaceProblem::NotRegular,
    };
    whole(Severity::Warning, DiagKind::Workspace(problem), path)
}

/// 構文エラーと同じ形の、読めない理由の診断 (エラー)。
fn unreadable(message: String) -> Diagnostic {
    whole(Severity::Error, DiagKind::Syntax { message }, String::new())
}

/// 1 つのファイルを所有者の検査つきで読んだ結果。
enum Outcome {
    /// 無い。
    Missing,
    /// 読めた本文。
    Text(String),
    /// 所有者の検査・大きさで読まなかった (診断 1 件)。`dir` は置き場所のディレクトリで落ちた (`.eve` を信用しない)。
    Rejected { diag: Diagnostic, dir: bool },
    /// 読めない (UTF-8 でない・OS の失敗。診断 1 件、エラー)。`dir` は置き場所 (`.eve`) の階層の失敗 (層を外す)。
    Unreadable { diag: Diagnostic, dir: bool },
    /// 一時の失敗。
    Busy,
}

fn read_one(fs: &dyn FileSystem, dir: &FsPath, name: &str, safe_dirs: &[FsPath]) -> Outcome {
    let path = dir.join(name);
    let r = fs.read_checked(
        dir,
        std::ffi::OsStr::new(name),
        WORKSPACE_FILE_MAX_BYTES,
        safe_dirs,
        DirLinks::NoFollowFromEve,
    );
    match r {
        Ok(CheckedRead::Missing) => Outcome::Missing,
        Ok(CheckedRead::Read { bytes }) => match String::from_utf8(bytes) {
            Ok(t) => Outcome::Text(t),
            Err(_) => Outcome::Unreadable {
                diag: unreadable("not valid UTF-8".to_owned()),
                dir: false,
            },
        },
        Ok(CheckedRead::TooLarge) => Outcome::Rejected {
            diag: whole(
                Severity::Warning,
                DiagKind::Workspace(WorkspaceProblem::TooLarge),
                path.display(),
            ),
            dir: false,
        },
        Ok(CheckedRead::Rejected { what, verdict }) => Outcome::Rejected {
            diag: rejected(what, verdict, dir, &path, safe_dirs),
            dir: what == CheckedObject::Dir,
        },
        Err(e) => match e.kind {
            FsErrorKind::Busy => Outcome::Busy,
            FsErrorKind::NotFound => Outcome::Missing,
            // 判定できない Port (所有者の検査を持たない FS): 読まない
            FsErrorKind::Unsupported => Outcome::Rejected {
                diag: rejected(
                    CheckedObject::Dir,
                    OwnerVerdict::Unknown {
                        fs: "unsupported".to_owned(),
                    },
                    dir,
                    &path,
                    &[],
                ),
                dir: true,
            },
            _ => Outcome::Unreadable {
                diag: unreadable(fs_reason(&e)),
                dir: true,
            },
        },
    }
}

/// ワークスペース層を読む (ジョブの本体。07 §12.5 の WSC-1 / WSC-2)。`safe_dirs` は `workspace.safe_directories`。
pub fn read_workspace_config(
    fs: &dyn FileSystem,
    root: &FsPath,
    conditions: eve_config::Conditions,
    safe_dirs: &[FsPath],
) -> WorkspaceRead {
    let dir = root.join(WORKSPACE_CONFIG_DIR);
    let config_path = dir.join(WORKSPACE_CONFIG_FILE);
    let keymap_path = dir.join(WORKSPACE_KEYMAP_FILE);
    let mut config_notes = Vec::new();
    let mut keymap_notes = Vec::new();
    let mut dir_trusted = true;
    let layer = match read_one(fs, &dir, WORKSPACE_CONFIG_FILE, safe_dirs) {
        Outcome::Missing => Fetched::Absent,
        Outcome::Busy => Fetched::Keep,
        Outcome::Rejected { diag, dir } => {
            dir_trusted &= !dir;
            config_notes.push(diag);
            Fetched::Absent
        }
        // 置き場所 (`.eve`) の失敗 (権限など) は層を外す (信頼の取り消しを通す)。前の層を保つのはファイルの構文エラー・
        // UTF-8 でないときだけ
        Outcome::Unreadable { diag, dir } => {
            dir_trusted &= !dir;
            config_notes.push(diag);
            if dir {
                Fetched::Absent
            } else {
                Fetched::Broken
            }
        }
        Outcome::Text(text) => {
            let mut l = eve_config::load_workspace_str(&text, conditions);
            let name = config_path.display();
            for d in &mut l.languages {
                d.source.clone_from(&name);
            }
            if l.has_error() {
                // 構文エラーの診断は分類の診断と同じく、重ねるときに出す (`classify` が読込の診断を返す)
                config_notes.extend(l.classify(&eve_config::Config::default()).diagnostics);
                Fetched::Broken
            } else {
                Fetched::Got(l)
            }
        }
    };
    let keymap = match read_one(fs, &dir, WORKSPACE_KEYMAP_FILE, safe_dirs) {
        Outcome::Missing => Fetched::Absent,
        Outcome::Busy => Fetched::Keep,
        Outcome::Rejected { diag, dir } => {
            dir_trusted &= !dir;
            keymap_notes.push(diag);
            Fetched::Absent
        }
        // 置き場所 (`.eve`) の失敗 (権限など) は層を外す (信頼の取り消しを通す)。前の層を保つのはファイルの構文エラー・
        // UTF-8 でないときだけ
        Outcome::Unreadable { diag, dir } => {
            dir_trusted &= !dir;
            keymap_notes.push(diag);
            if dir {
                Fetched::Absent
            } else {
                Fetched::Broken
            }
        }
        Outcome::Text(text) => {
            let file = eve_config::load_keymap_str(text.strip_prefix('\u{feff}').unwrap_or(&text));
            if file
                .diagnostics
                .iter()
                .any(|d| d.severity == Severity::Error)
            {
                keymap_notes.extend(file.diagnostics);
                Fetched::Broken
            } else {
                Fetched::Got(file)
            }
        }
    };
    // 読まない置き場所 (WSC-1)。`.eve` を判定で信用できたときだけ一覧する (`.eve` のリンクを辿らない。中の項目は名前を
    // 見るだけで開かない)
    if dir_trusted && let Ok(names) = fs.list_dir_names(&dir) {
        let mut found: Vec<&str> = UNSUPPORTED_DIRS
            .into_iter()
            .filter(|u| names.iter().any(|n| n.eq_ignore_ascii_case(u)))
            .collect();
        found.sort_unstable();
        config_notes.extend(found.into_iter().map(|u| Diagnostic {
            key: format!("{WORKSPACE_CONFIG_DIR}/{u}"),
            ..whole(Severity::Info, DiagKind::Unsupported, String::new())
        }));
    }
    WorkspaceRead {
        config_path,
        keymap_path,
        layer,
        keymap,
        config_notes,
        keymap_notes,
    }
}

impl EditorState {
    /// ワークスペース層を読むか (07 §12.5 の WSC-1 / WSC-7): Host が読込元を渡していて、`--no-workspace-config` でも
    /// `--no-config` でもなく、ルートが決まっている。
    fn workspace_config_root(&self) -> Option<FsPath> {
        let src = self.io.config_sources.as_ref()?;
        if src.no_workspace_config {
            return None;
        }
        self.workspace().map(|w| w.root().clone())
    }

    /// ユーザー層 (Host が渡したもの。渡していなければ今の設定と由来)。
    pub(crate) fn current_user_layer(&self) -> eve_config::UserLayer {
        self.settings
            .user_layer
            .clone()
            .unwrap_or_else(|| eve_config::UserLayer {
                config: self.settings.config.clone(),
                origins: self.settings.config_origins.clone(),
                theme_name_at: self.io.theme_name_at.clone(),
                ui_locale_at: None,
            })
    }

    /// `ReadWorkspaceConfig` を積む (起動時はルートが決まったとき、`config.reload` はユーザー層を読み直した後。`reload` は
    /// 届いた後に続ける再読込)。`safe_dirs` は、その時点のユーザー層と `--set` の `workspace.safe_directories` (`config.reload`
    /// では読み直した値)。積まなかった (読まない設定・ルートが無い) ときは `reload` を返す。
    pub(crate) fn start_workspace_config_read(
        &mut self,
        safe_dirs: &[String],
        reload: Option<ReloadPlan>,
    ) -> Option<ReloadPlan> {
        let Some(root) = self.workspace_config_root() else {
            return reload;
        };
        let safe_dirs: Vec<FsPath> = safe_dirs.iter().map(|s| FsPath::from(s.as_str())).collect();
        let job = self.jobs.spawn(
            None,
            eve_view::Revision(0),
            JobKind::ReadWorkspaceConfig {
                root,
                conditions: self.config_conditions(),
                safe_dirs,
            },
        );
        self.io.pending.insert(
            job,
            Pending::WorkspaceConfig {
                reload: reload.map(Box::new),
            },
        );
        None
    }

    /// ワークスペース層の読込の結果 (07 §12.5 の WSC-3 / WSC-4)。`reload` があれば、層を差し替えた後に `config.reload` を
    /// 続ける (全体を差し替えて要約を 1 回出す)。無ければ (起動の後に届いた) 重ね直して、診断だけを出す (要約は出さない)。
    pub(crate) fn finish_workspace_config(&mut self, r: JobResult, reload: Option<ReloadPlan>) {
        let JobResult::WorkspaceConfigRead(read) = r else {
            // ジョブが走らなかった (パニックなど): 層は前のまま
            if let Some(plan) = reload {
                self.apply_config_reload(plan);
            }
            return;
        };
        let WorkspaceRead {
            config_path,
            keymap_path,
            layer,
            keymap,
            config_notes,
            keymap_notes,
        } = read;
        let config_name = config_path.display();
        let keymap_name = keymap_path.display();
        self.settings.workspace_files = vec![config_name.clone(), keymap_name.clone()];
        let reloading = reload.is_some();
        let languages_before = self.settings.workspace_languages.clone();
        // 一時の失敗のファイルは、前に読んだ層と診断を使い続ける
        if !layer.keeps_diagnostics() {
            self.settings.workspace_notes = config_notes;
        }
        let mut kept: Vec<&str> = Vec::new();
        match layer {
            Fetched::Absent => {
                self.settings.workspace_layer = None;
                self.settings.workspace_languages.clear();
            }
            Fetched::Got(l) => {
                self.settings.workspace_languages = l.languages.clone();
                self.settings.workspace_layer = Some(Arc::new(l));
            }
            // 起動時は層を持たない。`config.reload` では前に読んだ層を保つ (07 §6 の規則 1)
            Fetched::Broken if reloading => kept.push(&config_name),
            Fetched::Broken => {
                self.settings.workspace_layer = None;
                self.settings.workspace_languages.clear();
            }
            Fetched::Keep => {}
        }
        match keymap {
            Fetched::Absent => {
                self.report_config(&keymap_name, &keymap_notes);
                self.keys.workspace_keymap_pending = Some((None, keymap_name.clone()));
            }
            Fetched::Got(k) => {
                self.keys.workspace_keymap_pending = Some((Some(k), keymap_name.clone()));
            }
            Fetched::Broken => {
                self.report_config(&keymap_name, &keymap_notes);
                if reloading {
                    kept.push(&keymap_name);
                } else {
                    self.keys.workspace_keymap_pending = Some((None, keymap_name.clone()));
                }
            }
            Fetched::Keep => {}
        }
        for file in kept {
            self.show_message(
                MessageLevel::Warn,
                keys::MSG_CONFIG_RELOAD_KEPT,
                &[("file", file)],
            );
        }
        match reload {
            // `config.reload`: 言語定義の読み直しは、再読込が積む (判定し直さない、LANG-7)。新しいファイルの形は決め直す
            Some(plan) => {
                self.apply_config_reload(plan);
                self.refresh_new_file_formats();
            }
            None => {
                self.recompose_config();
                // 起動の後に届いたワークスペース層の言語定義を、明示の指定の無い文書にも効かせる (LANG-7)
                if self.settings.workspace_languages != languages_before {
                    self.spawn_languages_read_for_workspace();
                }
            }
        }
    }

    /// ワークスペース層の `.eve/config.toml` の診断を出す (層の外の診断と、今のユーザー層と比べた分類の診断。07 §12.5 の
    /// WSC-4)。重ね直すたびに置き換える。
    pub(crate) fn report_workspace_config(&mut self, classified: &[Diagnostic]) {
        let Some(name) = self.settings.workspace_files.first().cloned() else {
            return;
        };
        let mut diags = self.settings.workspace_notes.clone();
        diags.extend(classified.iter().cloned());
        eve_config::cap_diagnostics(&mut diags);
        self.report_config(&name, &diags);
    }

    /// ユーザー層・ワークスペース層・コマンドライン層を重ね直して差し替える (ワークスペース層が届いたとき。07 §12.5 の
    /// WSC-3)。文書ごとの値を決め直し、まだ編集していない新しいファイルの改行と文字コードも決め直す (07 §3.6 の LAYER-4)。
    fn recompose_config(&mut self) {
        let user = self.current_user_layer();
        let src = self.config_sources();
        let ws_name = self
            .settings
            .workspace_files
            .first()
            .cloned()
            .unwrap_or_default();
        let ws = self.settings.workspace_layer.clone();
        let composed = eve_config::compose(
            &user,
            ws.as_deref().map(|l| (ws_name.as_str(), l)),
            &src.sets,
            src.conditions,
        );
        self.report_workspace_config(&composed.workspace_diagnostics);
        let exclude_before = self.exclude_config();
        self.settings.user_layer = Some(user);
        self.settings.config_origins = composed.origins;
        self.apply_reloaded_config(composed.config);
        self.refresh_all_doc_settings();
        self.refresh_new_file_formats();
        // 起動直後に始めたファインダの走査は、届く前の除外で走っている (やり直す)
        self.finder_on_exclude_changed(&exclude_before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 安全な場所の下かは書いた形の要素で比べる() {
        let safe = [FsPath::from("/work/repo")];
        assert!(written_under(&FsPath::from("/work/repo/.eve"), &safe));
        assert!(written_under(&FsPath::from("/work/repo"), &safe));
        assert!(!written_under(&FsPath::from("/work/repo2/.eve"), &safe));
        assert!(!written_under(&FsPath::from("/work"), &safe));
        assert!(!written_under(&FsPath::from("/work/repo/.eve"), &[]));
    }
}
