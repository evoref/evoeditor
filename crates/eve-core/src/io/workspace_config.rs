//! ワークスペース層の読込 (`ReadWorkspaceConfig`。docs/07 §12.5 の WSC-1〜WSC-7、§12.1 の OWN-1〜OWN-7、01 FR-705)。
//!
//! - **ジョブ** (`io`): `<root>/.eve/config.toml` と `<root>/.eve/keymap.toml` を所有者の検査つきの読込
//!   (`FileSystem::read_checked`。`.eve` から先のリンクを辿らない) で 256 KiB まで読み、解析・条件付きセクションの解決・
//!   検証・分類 (07 §12.3) までをジョブの中で行う (WSC-2。他人のリポジトリの大きな・入れ子の深いファイルの解析で入力を
//!   止めない)。`.eve/languages/`・`.eve/themes/`・`.eve/queries/` は、あれば「このバージョンでは未対応」の情報診断だけ。
//! - **主スレッド**: 届いた層を、ユーザー層の上・コマンドライン層の下に重ね直す (WSC-3)。読み直しは `config.reload` と同じ
//!   経路 (ユーザー層を読み直した後にこのジョブを積み、届いてから全体を差し替えて要約を 1 回出す)。
//!
//! 所有者の検査で読まなかったファイルは、ファイルごとに 1 件の警告 (OWN-7)。一時の失敗 (`Busy`) は診断を出さず、前に読んだ
//! 層をそのまま使う (次の読込で読み直す)。
//!
//! spec: 07 §12.5, 07 §12.1, 07 §12.3, 07 §7.4

use std::sync::Arc;

use eve_config::{
    Config, DiagKind, Diagnostic, KeymapFile, Severity, WorkspaceLayer, WorkspaceProblem,
};

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

/// ワークスペース設定のファイルの名前 (`<root>/.eve/config.toml`、07 §12.1)。
pub const WORKSPACE_CONFIG_FILE: &str = "config.toml";
/// ワークスペース層のキーマップのファイルの名前 (`<root>/.eve/keymap.toml`、07 §7.4 の WKEY-1)。
pub const WORKSPACE_KEYMAP_FILE: &str = "keymap.toml";
/// 1 ファイルの大きさの上限 (バイト。07 §12.5 の WSC-1)。
pub const WORKSPACE_FILE_MAX_BYTES: u64 = 256 * 1024;
/// ワークスペース層のキーの割当の上限 (1 ファイル。07 §7.4 の WKEY-3)。
pub const WORKSPACE_KEYMAP_MAX: usize = 1_000;
/// v0.5 では読まない `.eve` の下のディレクトリ (あれば未対応の情報診断を 1 件ずつ、WSC-1)。
const UNSUPPORTED_DIRS: [&str; 3] = ["languages", "themes", "queries"];

/// 1 つのファイルの読込の結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetched<T> {
    /// 無い・読まなかった (層を外す)。
    Absent,
    /// 読んだ。
    Got(T),
    /// 一時の失敗 (`Busy`): 前に読んだものを使い続ける。
    Keep,
}

/// `ReadWorkspaceConfig` の結果。
#[derive(Clone, Debug)]
pub struct WorkspaceRead {
    /// `<root>/.eve/config.toml` (診断と由来の表示名は、これの `display`)。
    pub config_path: FsPath,
    /// `<root>/.eve/keymap.toml`。
    pub keymap_path: FsPath,
    /// `.eve/config.toml` を読んで分類した層。
    pub layer: Fetched<WorkspaceLayer>,
    /// `.eve/keymap.toml` (割当の上限を超えた分を除き、その警告を診断に足したもの。検証は主スレッドで、下の層と比べる)。
    pub keymap: Fetched<KeymapFile>,
    /// ファイルごとの診断 (表示名はファイルの絶対パス)。読まなかった理由と、`.eve/config.toml` の読込・分類の診断。
    pub reports: Vec<(String, Vec<Diagnostic>)>,
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
    let parts = |p: &FsPath| -> Vec<String> {
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
    };
    let d = parts(dir);
    safe_dirs.iter().any(|s| {
        let s = parts(s);
        !s.is_empty() && d.len() >= s.len() && d[..s.len()] == s[..]
    })
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

/// 1 つのファイルを所有者の検査つきで読んだ結果。
enum Outcome {
    /// 無い。
    Missing,
    /// 読めた本文。
    Text(String),
    /// 読まなかった (診断 1 件)。`dir` は置き場所のディレクトリで落ちた (`.eve` を信用しない)。
    NotRead { diag: Diagnostic, dir: bool },
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
            Err(_) => Outcome::NotRead {
                diag: whole(
                    Severity::Error,
                    DiagKind::Syntax {
                        message: "not valid UTF-8".to_owned(),
                    },
                    String::new(),
                ),
                dir: false,
            },
        },
        Ok(CheckedRead::TooLarge) => Outcome::NotRead {
            diag: whole(
                Severity::Warning,
                DiagKind::Workspace(WorkspaceProblem::TooLarge),
                path.display(),
            ),
            dir: false,
        },
        Ok(CheckedRead::Rejected { what, verdict }) => Outcome::NotRead {
            diag: rejected(what, verdict, dir, &path, safe_dirs),
            dir: what == CheckedObject::Dir,
        },
        Err(e) => match e.kind {
            FsErrorKind::Busy => Outcome::Busy,
            FsErrorKind::NotFound => Outcome::Missing,
            // 判定できない Port (所有者の検査を持たない FS): 読まない
            FsErrorKind::Unsupported => Outcome::NotRead {
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
            _ => Outcome::NotRead {
                diag: whole(
                    Severity::Error,
                    DiagKind::Syntax {
                        message: fs_reason(&e),
                    },
                    String::new(),
                ),
                dir: true,
            },
        },
    }
}

/// ワークスペース層を読む (ジョブの本体。07 §12.5 の WSC-1 / WSC-2)。`lower` はユーザー層の値 (「方向のみ」の比較)、
/// `safe_dirs` は `workspace.safe_directories`。
pub fn read_workspace_config(
    fs: &dyn FileSystem,
    root: &FsPath,
    lower: &Config,
    conditions: eve_config::Conditions,
    safe_dirs: &[FsPath],
) -> WorkspaceRead {
    let dir = root.join(WORKSPACE_CONFIG_DIR);
    let config_path = dir.join(WORKSPACE_CONFIG_FILE);
    let keymap_path = dir.join(WORKSPACE_KEYMAP_FILE);
    let config_name = config_path.display();
    let keymap_name = keymap_path.display();
    let mut reports: Vec<(String, Vec<Diagnostic>)> = Vec::new();
    let mut dir_trusted = true;
    let layer = match read_one(fs, &dir, WORKSPACE_CONFIG_FILE, safe_dirs) {
        Outcome::Missing => Fetched::Absent,
        Outcome::Busy => Fetched::Keep,
        Outcome::NotRead { diag, dir } => {
            dir_trusted &= !dir;
            reports.push((config_name.clone(), vec![diag]));
            Fetched::Absent
        }
        Outcome::Text(text) => {
            let mut l = eve_config::load_workspace_str(&text, conditions, lower);
            for d in &mut l.languages {
                d.source.clone_from(&config_name);
            }
            if !l.diagnostics.is_empty() {
                reports.push((config_name.clone(), l.diagnostics.clone()));
            }
            Fetched::Got(l)
        }
    };
    let keymap = match read_one(fs, &dir, WORKSPACE_KEYMAP_FILE, safe_dirs) {
        Outcome::Missing => Fetched::Absent,
        Outcome::Busy => Fetched::Keep,
        Outcome::NotRead { diag, dir } => {
            dir_trusted &= !dir;
            reports.push((keymap_name, vec![diag]));
            Fetched::Absent
        }
        Outcome::Text(text) => {
            let mut file =
                eve_config::load_keymap_str(text.strip_prefix('\u{feff}').unwrap_or(&text));
            if let Some(over) = file.entries.get(WORKSPACE_KEYMAP_MAX) {
                let d = Diagnostic {
                    severity: Severity::Warning,
                    kind: DiagKind::Workspace(WorkspaceProblem::TooManyKeys),
                    key: String::new(),
                    value: WORKSPACE_KEYMAP_MAX.to_string(),
                    line: over.line,
                    column: over.column,
                    condition: None,
                };
                file.entries.truncate(WORKSPACE_KEYMAP_MAX);
                file.diagnostics.push(d);
            }
            Fetched::Got(file)
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
        let diags: Vec<Diagnostic> = found
            .into_iter()
            .map(|u| Diagnostic {
                key: format!("{WORKSPACE_CONFIG_DIR}/{u}"),
                ..whole(Severity::Info, DiagKind::Unsupported, String::new())
            })
            .collect();
        if !diags.is_empty() {
            eve_config::merge_reports(&mut reports, [(config_name, diags)]);
        }
    }
    WorkspaceRead {
        config_path,
        keymap_path,
        layer,
        keymap,
        reports,
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
    /// 届いた後に続ける再読込)。積まなかった (読まない設定・ルートが無い) ときは `reload` を返す。
    pub(crate) fn start_workspace_config_read(
        &mut self,
        lower: &Config,
        reload: Option<ReloadPlan>,
    ) -> Option<ReloadPlan> {
        let Some(root) = self.workspace_config_root() else {
            return reload;
        };
        let safe_dirs: Vec<FsPath> = self
            .settings
            .config
            .safe_directories
            .iter()
            .map(|s| FsPath::from(s.as_str()))
            .collect();
        let job = self.jobs.spawn(
            None,
            eve_view::Revision(0),
            JobKind::ReadWorkspaceConfig {
                root,
                lower: Arc::new(lower.clone()),
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
            reports,
        } = read;
        let config_name = config_path.display();
        let keymap_name = keymap_path.display();
        // 一覧の診断は読み直した結果に置き換える (`.eve/keymap.toml` の検証の診断は、キーマップを当てるときに置き換える)
        let names = [config_name.clone(), keymap_name.clone()];
        self.settings.workspace_files = names.to_vec();
        self.forget_config_diagnostics(|f| f == config_name);
        if !matches!(keymap, Fetched::Got(_)) {
            self.forget_config_diagnostics(|f| f == keymap_name);
        }
        for (file, diags) in &reports {
            self.report_config(file, diags);
        }
        let languages_before = self.settings.workspace_languages.clone();
        match layer {
            Fetched::Absent => {
                self.settings.workspace_layer = None;
                self.settings.workspace_languages.clear();
            }
            Fetched::Got(l) => {
                self.settings.workspace_languages = l.languages.clone();
                self.settings.workspace_layer = Some(Arc::new(l));
            }
            Fetched::Keep => {}
        }
        match keymap {
            Fetched::Absent => self.keys.workspace_keymap_pending = Some((None, keymap_name)),
            Fetched::Got(k) => self.keys.workspace_keymap_pending = Some((Some(k), keymap_name)),
            Fetched::Keep => {}
        }
        match reload {
            // `config.reload`: 言語定義の読み直しは、再読込が積む (判定し直さない、LANG-7)
            Some(plan) => self.apply_config_reload(plan),
            None => {
                self.recompose_config();
                // 起動の後に届いたワークスペース層の言語定義を、明示の指定の無い文書にも効かせる (LANG-7)
                if self.settings.workspace_languages != languages_before {
                    self.spawn_languages_read(true);
                }
            }
        }
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
        self.settings.user_layer = Some(user);
        self.settings.config_origins = composed.origins;
        self.apply_reloaded_config(composed.config);
        self.refresh_all_doc_settings();
        self.refresh_new_file_formats();
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
