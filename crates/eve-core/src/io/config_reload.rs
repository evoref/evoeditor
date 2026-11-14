//! 設定の再読込 (`config.reload`、docs/07 §6、01 FR-706)。
//!
//! `config.toml` と `keymap.toml` を io ジョブで読み (メインスレッドで I/O しない)、届いたら解析して差し替える。
//! エラー (構文エラー・読めない) のあるファイルは前の設定のまま維持し、診断だけ出す (起動時と違い、半端な状態にしない)。
//! 変わった項目は 1 行に要約し、再起動しないと効かない項目は別の 1 行で知らせる。

use eve_config::{Config, KeymapFile, Severity};
use eve_view::{Effect, MessageLevel};

use crate::command::ids;
use crate::command::{CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission};
use crate::editor::Editor;
use crate::editor::state::EditorState;
use crate::io::pending::Pending;
use crate::jobs::{ConfigFile, JobKind, JobResult};
use crate::ui_model::keys;

/// 設定ファイルの名前 (設定ディレクトリの直下、07 §2)。
pub const CONFIG_FILE: &str = "config.toml";
/// キーマップファイルの名前。
pub const KEYMAP_FILE: &str = "keymap.toml";

/// 設定の読込元 (07 §3・§18.3)。起動時に bin が決め (`--config` / `--no-config` / `--set`)、再読込も同じ元を読む。
#[derive(Clone, Debug, Default)]
pub struct ConfigSources {
    /// ユーザー層の設定ファイル (診断に出す名前, パス)。この順に重ねる。
    pub files: Vec<(String, crate::FsPath)>,
    /// `keymap.toml`。`None` は読まない (`--no-config`)。
    pub keymap: Option<crate::FsPath>,
    /// コマンドライン層 (`--set key=value`)。最後に重ねる。
    pub sets: Vec<String>,
    /// テーマファイルの置き場 (`<設定ディレクトリ>/themes`)。`None` は読まない。
    pub themes: Option<crate::FsPath>,
}

impl EditorState {
    /// `theme.name` が組込みでなければ、テーマファイルを読むジョブを投入する (`inherits` の親も読む)。
    fn start_theme_read(&mut self) {
        let name = {
            let c = self.config();
            eve_config::theme::with_background(&c.theme_name, c.theme_background).to_owned()
        };
        if eve_config::theme::builtin(&name).is_some() {
            return;
        }
        let Some(dir) = self.config_sources().themes else {
            // 置き場が無い: default で動く
            self.set_user_theme(&name, None);
            return;
        };
        let job = self.jobs.spawn(
            None,
            eve_view::Revision(0),
            JobKind::ReadTheme {
                dir,
                name: name.clone(),
                origin: self.io.theme_name_at.clone(),
            },
        );
        self.io.pending.insert(job, Pending::ThemeRead);
    }

    /// テーマファイルの読込の結果。読めなければ `default` で動き、診断を出す。
    pub(crate) fn finish_theme_read(&mut self, r: JobResult) {
        let JobResult::ThemeRead {
            name,
            theme,
            reports,
            ..
        } = r
        else {
            return;
        };
        // 一覧 (`config.show_diagnostics`) のテーマの診断は、読み直した結果に置き換える。テーマが無い診断は
        // `theme.name` を書いた設定ファイルの名前で出す (07 §8.7) ので、そのファイルの診断には足す
        self.forget_config_diagnostics(EditorState::is_theme_diagnostic_file);
        self.forget_theme_not_found();
        for (file, diags) in &reports {
            if EditorState::is_theme_diagnostic_file(file) {
                self.report_config(file, diags);
            } else {
                self.report_config_more(file, diags);
            }
        }
        self.set_user_theme(&name, theme);
    }
}

/// 再起動しないと効かない項目 (07 §6 の 3 分類の「再起動が必要」。端末の初期化・言語の選択で 1 回だけ使う)。
const RESTART_KEYS: [&str; 3] = ["theme.color_mode", "view.ambiguous_width", "ui.locale"];

/// 読んだバイト列を文字列にする (メモ帳が付ける BOM は読み飛ばす)。読めなければ理由。
fn text_of(f: &ConfigFile) -> Result<Option<String>, String> {
    match f {
        Ok(None) => Ok(None),
        Ok(Some(bytes)) => match std::str::from_utf8(bytes) {
            Ok(t) => Ok(Some(t.strip_prefix('\u{feff}').unwrap_or(t).to_owned())),
            Err(_) => Err("UTF-8 として読めません".to_owned()),
        },
        Err(e) => Err(e.clone()),
    }
}

impl EditorState {
    /// 設定の読込元を置く (Host が起動時に、bin の決めたものを渡す)。置かなければ設定ディレクトリの 2 つのファイル。
    pub fn set_config_sources(&mut self, sources: ConfigSources) {
        self.io.config_sources = Some(sources);
    }

    pub(crate) fn config_sources(&self) -> ConfigSources {
        if let Some(s) = &self.io.config_sources {
            return s.clone();
        }
        let dir = self.paths.config_dir();
        if dir.as_path().as_os_str().is_empty() {
            return ConfigSources::default();
        }
        ConfigSources {
            files: vec![(CONFIG_FILE.to_owned(), dir.join(CONFIG_FILE))],
            keymap: Some(dir.join(KEYMAP_FILE)),
            sets: Vec::new(),
            themes: Some(dir.join("themes")),
        }
    }
}

fn unreadable(message: String) -> eve_config::Diagnostic {
    eve_config::Diagnostic {
        severity: Severity::Error,
        kind: eve_config::DiagKind::Syntax { message },
        key: String::new(),
        value: String::new(),
        line: 0,
        column: 0,
    }
}

fn has_error(diags: &[eve_config::Diagnostic]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

impl EditorState {
    /// `config.reload`: 設定ディレクトリの 2 つのファイルを読むジョブを投入する。
    fn start_config_reload(&mut self) -> CommandResult {
        // 二重に始めない
        if self
            .io
            .pending
            .values()
            .any(|p| matches!(p, Pending::ConfigReload))
        {
            return CommandResult::Done;
        }
        let src = self.config_sources();
        let job = self.jobs.spawn(
            None,
            eve_view::Revision(0),
            JobKind::ReadConfig {
                files: src.files.iter().map(|(_, p)| p.clone()).collect(),
                keymap: src.keymap.clone(),
            },
        );
        self.io.pending.insert(job, Pending::ConfigReload);
        CommandResult::Done
    }

    /// 読込の結果。`config.toml` はここで差し替え、`keymap.toml` は解析の結果を Editor に渡す (キーマップは Editor が持つ)。
    pub(crate) fn finish_config_reload(&mut self, r: JobResult) {
        let JobResult::ConfigRead { config, keymap, .. } = r else {
            return;
        };
        let src = self.config_sources();
        let sources: Vec<eve_config::Source> = src
            .files
            .iter()
            .zip(&config)
            .map(|((name, _), f)| eve_config::Source {
                name: name.clone(),
                text: text_of(f),
            })
            .collect();
        let resolved = eve_config::resolve(&sources, &src.sets);
        let keymap: KeymapFile = match keymap.as_ref().map(text_of) {
            None | Some(Ok(None)) => KeymapFile::default(),
            Some(Ok(Some(t))) => eve_config::load_keymap_str(&t),
            Some(Err(e)) => KeymapFile {
                entries: Vec::new(),
                diagnostics: vec![unreadable(e)],
            },
        };
        let mut changed: Vec<&'static str> = Vec::new();
        // 一覧 (`config.show_diagnostics`) の設定ファイル (と `--set`) の診断は、読み直した結果に置き換える。
        // キーマップの診断は `finish_reload_keymap` で置き換え、テーマの診断はテーマを読み直すときに置き換える
        self.forget_config_diagnostics(|f| {
            f != KEYMAP_FILE && !EditorState::is_theme_diagnostic_file(f)
        });
        for (name, diags) in &resolved.reports {
            self.report_config(name, diags);
        }
        if resolved.has_error {
            let name = resolved
                .reports
                .iter()
                .find(|(_, d)| has_error(d))
                .map_or(CONFIG_FILE, |(n, _)| n.as_str())
                .to_owned();
            self.show_message(
                MessageLevel::Warn,
                keys::MSG_CONFIG_RELOAD_KEPT,
                &[("file", &name)],
            );
        } else {
            changed = eve_config::changed_keys(self.config(), &resolved.config);
            self.io.theme_name_at = resolved.theme_name_at.clone();
            self.apply_reloaded_config(resolved.config);
            // 組込みのテーマに替えたときは読まないので、前のテーマファイルの診断をここで消す
            self.forget_config_diagnostics(EditorState::is_theme_diagnostic_file);
            // テーマファイルも読み直す (名前が同じでも、ファイルが変わっていることがある)
            self.start_theme_read();
        }
        self.keys.reloaded = Some((keymap, changed));
    }

    /// 新しい設定に差し替える。マウスの取得は frontend に伝える (即時に効く)。
    fn apply_reloaded_config(&mut self, new: Config) {
        let (mouse_changed, mouse) = (self.config().mouse != new.mouse, new.mouse);
        self.set_config(new);
        if mouse_changed {
            self.push_effect(Effect::SetMouseCapture { enabled: mouse });
        }
    }

    /// 変わった項目の要約 (07 §6 の規則 4)。キーマップが変わったら `keymap.toml` も項目に数える。
    fn report_reload(&mut self, mut changed: Vec<&'static str>, keymap_changed: bool) {
        if keymap_changed {
            changed.push(KEYMAP_FILE);
        }
        if changed.is_empty() {
            self.show_message(MessageLevel::Info, keys::INFO_CONFIG_RELOAD_UNCHANGED, &[]);
            return;
        }
        let keys = changed.join(", ");
        self.show_message(
            MessageLevel::Info,
            keys::INFO_CONFIG_RELOADED,
            &[("count", &changed.len().to_string()), ("keys", &keys)],
        );
        let restart: Vec<&str> = changed
            .iter()
            .copied()
            .filter(|k| RESTART_KEYS.contains(k))
            .collect();
        if !restart.is_empty() {
            self.show_message(
                MessageLevel::Info,
                keys::INFO_CONFIG_RELOAD_RESTART,
                &[("keys", &restart.join(", "))],
            );
        }
    }
}

impl Editor {
    /// 再読込の結果が届いていれば、キーマップを差し替えて要約を出す (`on_job_result` の後に呼ぶ)。
    pub(crate) fn finish_reload_keymap(&mut self) {
        let Some((keymap, changed)) = self.state.keys.reloaded.take() else {
            return;
        };
        let mut keymap_changed = false;
        if has_error(&keymap.diagnostics) {
            self.state.report_config(KEYMAP_FILE, &keymap.diagnostics);
            self.state.show_message(
                MessageLevel::Warn,
                keys::MSG_CONFIG_RELOAD_KEPT,
                &[("file", KEYMAP_FILE)],
            );
        } else {
            // まだ一度も設定していない (キーマップのファイルが無い起動) は空と同じ
            keymap_changed = self
                .state
                .keys
                .user_keymap_entries
                .as_deref()
                .unwrap_or(&[])
                != keymap.entries.as_slice();
            let diags = self.set_user_keymap(&keymap);
            self.state.report_config(KEYMAP_FILE, &diags);
        }
        self.state.report_reload(changed, keymap_changed);
    }
}

fn config_reload(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.start_config_reload()
}

pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(
        ids::CONFIG_RELOAD,
        CommandKind::Ui,
        Permission::None,
        "",
        config_reload,
    );
}
