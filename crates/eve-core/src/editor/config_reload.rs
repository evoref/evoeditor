//! 設定の再読込 (`config.reload`、docs/07 §6、01 FR-706)。
//!
//! `config.toml` と `keymap.toml` を io ジョブで読み (メインスレッドで I/O しない)、届いたら解析して差し替える。
//! エラー (構文エラー・読めない) のあるファイルは前の設定のまま維持し、診断だけ出す (起動時と違い、半端な状態にしない)。
//! 変わった項目は 1 行に要約し、再起動しないと効かない項目は別の 1 行で知らせる。

use eve_config::{Config, KeymapFile, Severity};
use eve_view::{Effect, MessageLevel};

use super::Editor;
use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission,
};
use super::file::Pending;
use super::state::EditorState;
use crate::jobs::{ConfigFile, JobKind, JobResult};

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
        self.config_sources = Some(sources);
    }

    fn config_sources(&self) -> ConfigSources {
        if let Some(s) = &self.config_sources {
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
        self.pending.insert(job, Pending::ConfigReload);
        CommandResult::Done
    }

    /// 読込の結果。`config.toml` はここで差し替え、`keymap.toml` は解析の結果を Editor に渡す (キーマップは Editor が持つ)。
    pub(super) fn finish_config_reload(&mut self, r: JobResult) {
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
                "msg.config.reload_kept",
                &[("file", &name)],
            );
        } else {
            changed = eve_config::changed_keys(self.config(), &resolved.config);
            self.apply_reloaded_config(resolved.config);
        }
        self.reloaded = Some((keymap, changed));
    }

    /// 新しい設定に差し替える。マウスの取得は frontend に伝える (即時に効く)。
    fn apply_reloaded_config(&mut self, new: Config) {
        let mouse_changed = self.config().mouse != new.mouse;
        self.set_config(new);
        if mouse_changed {
            self.push_effect(Effect::SetMouseCapture { enabled: new.mouse });
        }
    }

    /// 変わった項目の要約 (07 §6 の規則 4)。キーマップが変わったら `keymap.toml` も項目に数える。
    fn report_reload(&mut self, mut changed: Vec<&'static str>, keymap_changed: bool) {
        if keymap_changed {
            changed.push(KEYMAP_FILE);
        }
        if changed.is_empty() {
            self.show_message(MessageLevel::Info, "info.config.reload_unchanged", &[]);
            return;
        }
        let keys = changed.join(", ");
        self.show_message(
            MessageLevel::Info,
            "info.config.reloaded",
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
                "info.config.reload_restart",
                &[("keys", &restart.join(", "))],
            );
        }
    }
}

impl Editor {
    /// 再読込の結果が届いていれば、キーマップを差し替えて要約を出す (`on_job_result` の後に呼ぶ)。
    pub(super) fn finish_reload_keymap(&mut self) {
        let Some((keymap, changed)) = self.state.reloaded.take() else {
            return;
        };
        let mut keymap_changed = false;
        if has_error(&keymap.diagnostics) {
            self.state.report_config(KEYMAP_FILE, &keymap.diagnostics);
            self.state.show_message(
                MessageLevel::Warn,
                "msg.config.reload_kept",
                &[("file", KEYMAP_FILE)],
            );
        } else {
            // まだ一度も設定していない (キーマップのファイルが無い起動) は空と同じ
            keymap_changed = self.state.user_keymap_entries.as_deref().unwrap_or(&[])
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

pub(super) fn register(r: &mut CommandRegistry) {
    r.register(
        CommandSpec::new(
            "config.reload",
            CommandKind::Ui,
            Permission::None,
            "",
            config_reload,
        )
        .expect("カタログの when 句は構文が正しい"),
    );
}
