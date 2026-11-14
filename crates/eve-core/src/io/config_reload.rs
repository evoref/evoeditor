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
    /// ユーザーの言語ファイルの置き場 (`<設定ディレクトリ>/locales`、07 §17.4)。`None` は見ない (`--no-config`)。
    /// 読むのは起動時の bin だけで、ここは診断の一覧から開くときと、`config.reload` で変わったかを見るときに使う。
    pub locales: Option<crate::FsPath>,
}

/// 起動時に bin が決めた、ユーザーの言語ファイルの事実 (07 §17.4)。`config.reload` は言語ファイルの中身を読み直さず、
/// `locales/` を 1 回一覧して ([`LocaleScan`]) ここと比べ、変わっていれば再起動を促すだけにする。
#[derive(Clone, Debug, Default)]
pub struct LocaleFiles {
    /// 起動時に `locales/` で見つけた候補の言語 (名前の順)。
    pub available: Vec<eve_config::Locale>,
    /// 一覧する `locales/`。`None` は見ない (`--no-config`)。
    pub dir: Option<crate::FsPath>,
    /// 選んだ言語。
    pub chosen: eve_config::Locale,
    /// 起動時の、選んだ言語のファイル (一覧で得た名前のもの) の (大きさ, 更新日時)。無かった・調べられなかったときは `None`。
    pub stamp: Option<(u64, crate::FileTime)>,
}

/// `locales/` を一覧した結果 (07 §17.4。中身は読まない)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocaleScan {
    /// 候補の言語 (名前の規則 `^[a-z]{2,3}\.toml$` に合うもの。名前の順)。
    pub available: Vec<eve_config::Locale>,
    /// `chosen` の言語のファイル (一覧で得た名前のもの) の (大きさ, 更新日時)。無い・調べられないときは `None`。
    pub stamp: Option<(u64, crate::FileTime)>,
}

/// `dir` (`locales/`) を 1 回一覧して候補を作り、`chosen` の言語のファイルの大きさと更新日時を調べる (中身は読まない)。
/// パスは一覧で得た名前で組み立てる (選択の値から組み立てない。大文字小文字を区別しない FS でも綴りで照合する)。
/// ディレクトリが無い・一覧できないときは候補なし。
pub fn scan_locales(
    fs: &dyn crate::io::fs::FileSystem,
    dir: &crate::FsPath,
    chosen: eve_config::Locale,
) -> LocaleScan {
    let mut found: Vec<(eve_config::Locale, String)> = fs
        .list_dir_names(dir)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|n| eve_config::Locale::from_file_name(&n).map(|l| (l, n)))
        .collect();
    found.sort();
    found.dedup_by_key(|(l, _)| *l);
    let stamp = found
        .iter()
        .find(|(l, _)| *l == chosen)
        .and_then(|(_, n)| fs.metadata(&dir.join(n)).ok())
        .map(|m| (m.len, m.mtime));
    LocaleScan {
        available: found.into_iter().map(|(l, _)| l).collect(),
        stamp,
    }
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
            Err(_) => Err("not valid UTF-8".to_owned()),
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
            locales: Some(dir.join("locales")),
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
                locales: self
                    .io
                    .locale_files
                    .dir
                    .clone()
                    .map(|d| (d, self.io.locale_files.chosen)),
            },
        );
        self.io.pending.insert(job, Pending::ConfigReload);
        CommandResult::Done
    }

    /// 読込の結果。`config.toml` はここで差し替え、`keymap.toml` は解析の結果を Editor に渡す (キーマップは Editor が持つ)。
    pub(crate) fn finish_config_reload(&mut self, r: JobResult) {
        let JobResult::ConfigRead {
            config,
            keymap,
            locales,
        } = r
        else {
            return;
        };
        // 言語ファイルは読み直さない。`locales/` の候補か、選んだ言語のファイルが起動時から変わっていれば (無くなった・
        // 現れたを含む)、要約の後に再起動を促す (07 §17.4 の「読む時期」)
        let files = &self.io.locale_files;
        self.io.locale_restart = locales
            .as_ref()
            .is_some_and(|s| s.stamp != files.stamp || s.available != files.available);
        // `ui.locale` は今の候補と照らす (新しく置いた言語を「見つからない」にしない。選ぶのは再起動の後)
        let available = locales.map_or_else(|| files.available.clone(), |s| s.available);
        let dir = files
            .dir
            .as_ref()
            .map_or_else(String::new, crate::FsPath::display);
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
        let mut resolved = eve_config::resolve(&sources, &src.sets);
        eve_config::check_ui_locale(&mut resolved, &available, &dir, self.locale());
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
        // キーマップの診断は `finish_reload_keymap` で置き換え、テーマの診断はテーマを読み直すときに置き換える。
        // 言語ファイル・言語の選択の診断は起動時のものを残す (読み直さない、07 §17.4)
        let locale_reports = self.ui.locale_reports.clone();
        self.forget_config_diagnostics(|f| {
            f != KEYMAP_FILE
                && !EditorState::is_theme_diagnostic_file(f)
                && !locale_reports.contains(f)
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
            self.notify_locale_restart();
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
        self.notify_locale_restart();
    }

    /// 言語ファイルが起動時から変わっていたら、再起動を促す (`info.config.locale_restart`、07 §17.4)。
    fn notify_locale_restart(&mut self) {
        if std::mem::take(&mut self.io.locale_restart) {
            self.show_message(MessageLevel::Info, keys::INFO_CONFIG_LOCALE_RESTART, &[]);
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
