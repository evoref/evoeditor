//! 起動時の設定の読込 (07 §18.3): 読込元の決定、`config.toml`・`keymap.toml`・テーマファイルの読込、UI の言語の選択、
//! 診断のログの行。端末を始める前なので、ファイルはここで直接読む (`config.reload` は io ジョブで読む)。

use eve_core::Paths as _;
use eve_core::{CONFIG_FILE, KEYMAP_FILE};

use crate::NativePaths;
use crate::cli;

/// 設定ファイル 1 つを読む。無ければ `Ok(None)`、読めない (UTF-8 でない・権限がない等) ときは理由。
/// 起動時は端末を始める前なので、ここで直接読む (`config.reload` は io ジョブで読む)。
pub(crate) fn read_source(path: &std::path::Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "UTF-8 として読めません".to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// 設定の読込元 (07 §18.3): 既定は設定ディレクトリの `config.toml` と `keymap.toml`。`--config` はユーザー層の
/// `config.toml` の代わりに指定のファイルを順に重ね、`--no-config` はユーザー層を読まない。`--set` は最後に重ねる。
pub(crate) fn config_sources(
    paths: &NativePaths,
    args: &cli::ConfigArgs,
) -> eve_core::ConfigSources {
    let dir = paths.config_dir();
    let has_dir = !dir.as_path().as_os_str().is_empty();
    let files = if args.no_config {
        Vec::new()
    } else if !args.files.is_empty() {
        args.files
            .iter()
            .map(|f| {
                let path = paths
                    .normalize(f)
                    .unwrap_or_else(|_| eve_core::FsPath::new(f));
                (f.clone(), path)
            })
            .collect()
    } else if has_dir {
        vec![(CONFIG_FILE.to_owned(), dir.join(CONFIG_FILE))]
    } else {
        Vec::new()
    };
    eve_core::ConfigSources {
        files,
        keymap: (!args.no_config && has_dir).then(|| dir.join(KEYMAP_FILE)),
        sets: args.sets.clone(),
        themes: (!args.no_config && has_dir).then(|| dir.join("themes")),
    }
}

/// `theme.name` が組込みでなければ、テーマファイル (`themes/<name>.toml`、`inherits` の親も) を読む (07 §8.6)。
/// 戻り値は (名前, テーマ (読めなければ `None`)、ファイルごとの診断)。組込みなら `None`。`origin` は `theme.name` を
/// 書いた所 (`Resolved.theme_name_at`。テーマが無いときの診断をそこで出す、07 §8.7)。
#[allow(clippy::type_complexity)]
pub(crate) fn load_user_theme(
    src: &eve_core::ConfigSources,
    config: &eve_config::Config,
    origin: Option<&eve_config::KeyOrigin>,
) -> Option<(
    String,
    Option<eve_config::theme::Theme>,
    Vec<(String, Vec<eve_config::Diagnostic>)>,
)> {
    let name =
        eve_config::theme::with_background(&config.theme_name, config.theme_background).to_owned();
    if eve_config::theme::builtin(&name).is_some() {
        return None;
    }
    let read = |n: &str| match &src.themes {
        Some(dir) => read_source(dir.join(format!("{n}.toml")).as_path()),
        None => Ok(None),
    };
    let (theme, reports) = eve_config::theme_file::load_theme_at(&name, origin, &read);
    Some((name, theme, reports))
}

/// 読込元から設定を合成する。読めない・構文エラーのファイルは重ねずに診断にする (起動を妨げない、FR-703)。
pub(crate) fn load_config(src: &eve_core::ConfigSources) -> eve_config::Resolved {
    let sources: Vec<eve_config::Source> = src
        .files
        .iter()
        .map(|(name, path)| eve_config::Source {
            name: name.clone(),
            text: read_source(path.as_path()),
        })
        .collect();
    eve_config::resolve(&sources, &src.sets)
}

/// `keymap.toml` を読む。無ければ空。読めないときは診断を 1 件出して空。
pub(crate) fn load_keymap(src: &eve_core::ConfigSources) -> eve_config::KeymapFile {
    use eve_config::{DiagKind, Diagnostic, KeymapFile, Severity};
    let Some(path) = &src.keymap else {
        return KeymapFile::default();
    };
    match read_source(path.as_path()) {
        Ok(Some(text)) => {
            eve_config::load_keymap_str(text.strip_prefix('\u{feff}').unwrap_or(&text))
        }
        Ok(None) => KeymapFile::default(),
        Err(message) => KeymapFile {
            entries: Vec::new(),
            diagnostics: vec![Diagnostic {
                severity: Severity::Error,
                kind: DiagKind::Syntax { message },
                key: String::new(),
                value: String::new(),
                line: 0,
                column: 0,
            }],
        },
    }
}

/// UI の言語: --locale > ui.locale > `EVE_LANG` > OS の言語 > ja (07 §17.1)。
pub(crate) fn choose_locale(
    cli_locale: Option<&str>,
    config: &eve_config::Config,
) -> eve_config::LocaleChoice {
    let env_lang = std::env::var("EVE_LANG").ok();
    let os_tag = eve_platform::locale::ui_language_tag();
    eve_config::select_locale(
        cli_locale,
        config.ui_locale,
        env_lang.as_deref(),
        os_tag.as_deref(),
    )
}

/// 診断の重さ → ログのレベル。
pub(crate) fn log_level(s: eve_config::Severity) -> eve_platform::log::Level {
    match s {
        eve_config::Severity::Info => eve_platform::log::Level::Info,
        eve_config::Severity::Warning => eve_platform::log::Level::Warn,
        eve_config::Severity::Error => eve_platform::log::Level::Error,
    }
}

/// 診断 1 件のログの行 (ログは英語のみ、05 §7.3)。
pub(crate) fn describe(name: &str, d: &eve_config::Diagnostic) -> String {
    format!(
        "config {name}:{}:{} {} {:?}",
        d.line, d.column, d.key, d.kind
    )
}

/// 設定 `theme.color_mode` を TUI の型へ。
pub(crate) fn color_mode(mode: eve_config::ColorMode) -> eve_tui::caps::ColorMode {
    use eve_config::ColorMode as C;
    use eve_tui::caps::ColorMode as T;
    match mode {
        C::Auto => T::Auto,
        C::TrueColor => T::TrueColor,
        C::Color256 => T::Ansi256,
        C::Color16 => T::Ansi16,
        C::None => T::None,
    }
}
