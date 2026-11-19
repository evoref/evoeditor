//! 起動時の設定の読込 (07 §18.3): 読込元の決定、`config.toml`・`keymap.toml`・テーマファイルの読込、UI の言語の選択と
//! ユーザーの言語ファイルの読込 (07 §17.4)、診断のログの行。端末を始める前なので、ファイルはここで直接読む
//! (`config.reload` は io ジョブで読む。言語ファイルは読み直さない)。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use eve_config::{Catalog, DiagKind, Diagnostic, Locale, LocaleProblem, Severity};
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
            .map_err(|_| "not valid UTF-8".to_owned()),
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
        // ユーザーの言語ファイル (07 §17.4)。`--no-config` では見ない
        locales: (!args.no_config && has_dir).then(|| dir.join("locales")),
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

/// UI の言語: --locale > ui.locale > `EVE_LANG` > OS の言語 > ja (07 §17.1)。`available` は `locales/` の候補の言語。
fn choose_locale(
    cli_locale: Option<&str>,
    config: &eve_config::Config,
    available: &[Locale],
) -> eve_config::LocaleChoice {
    let env_lang = std::env::var("EVE_LANG").ok();
    let os_tag = eve_platform::locale::ui_language_tag();
    eve_config::select_locale(
        cli_locale,
        config.ui_locale,
        env_lang.as_deref(),
        os_tag.as_deref(),
        available,
    )
}

/// 言語の選択と、ユーザーの言語ファイルの読込の結果 (07 §17.4)。
pub(crate) struct LocaleSetup {
    /// 選んだ言語と段。
    pub choice: eve_config::LocaleChoice,
    /// 選んだ言語の文言の表 (補う言語の同梱の表に、言語ファイルの検査に通った文言を重ねたもの)。
    pub catalog: Arc<Catalog>,
    /// `locales/` の候補 (言語 → 一覧で得たパス)。
    pub candidates: BTreeMap<Locale, PathBuf>,
    /// 探した `locales/` (`--no-config` では `None`)。
    pub dir: Option<eve_core::FsPath>,
    /// 選んだ言語のファイルを、ファイル全体の問題で使わなかった。
    pub unusable: bool,
    /// 診断 (表示名, 診断): 名前の規則に合わないファイル (`locales/`)、見つからない言語 (`--locale` / `EVE_LANG`)、
    /// 選んだ言語のファイル (`locales/<lang>.toml`) の順。`ui.locale` の見つからない言語は、設定ファイルの診断に足す。
    pub reports: Vec<(String, Vec<Diagnostic>)>,
}

impl LocaleSetup {
    /// 選んだ言語のユーザーの言語ファイル (候補にあれば)。
    pub fn file(&self) -> Option<&PathBuf> {
        self.candidates.get(&self.choice.locale)
    }

    /// 候補の言語。
    pub fn available(&self) -> Vec<Locale> {
        self.candidates.keys().copied().collect()
    }

    /// エラー (ファイル全体の問題) の診断があるか (`--check-config` の終了コード 3)。
    pub fn has_error(&self) -> bool {
        self.reports
            .iter()
            .flat_map(|(_, ds)| ds)
            .any(|d| d.severity == Severity::Error)
    }
}

/// 言語ファイルの読み方: 1 つのファイルの診断の上限と、訳の無いキーを数えるか (`--check-config` だけ、07 §17.4)。
#[derive(Clone, Copy)]
pub(crate) struct LocaleRead {
    /// 1 つのファイルの診断の上限 (画面・ログは 100 件、`--check-config` は 1 万件)。
    pub max_diagnostics: usize,
    /// 追加言語のファイルで、訳の無いキーを情報として出すか。
    pub untranslated: bool,
}

impl LocaleRead {
    /// 起動と一覧 (`--list-commands` など) の読み方。
    pub const STARTUP: LocaleRead = LocaleRead {
        max_diagnostics: eve_config::LOCALE_MAX_DIAGNOSTICS,
        untranslated: false,
    };
    /// `--check-config` の読み方。
    pub const CHECK: LocaleRead = LocaleRead {
        max_diagnostics: eve_config::LOCALE_CHECK_MAX_DIAGNOSTICS,
        untranslated: true,
    };
}

/// 位置の無い言語ファイル・言語の選択の診断 1 件。
fn locale_diag(severity: Severity, problem: LocaleProblem, key: &str, value: &str) -> Diagnostic {
    Diagnostic {
        severity,
        kind: DiagKind::Locale(problem),
        key: key.to_owned(),
        value: value.to_owned(),
        line: 0,
        column: 0,
    }
}

/// 診断を `max` 件までにし、超えた分を「ほか N 件」(`locale_more`) の 1 件にまとめる。
fn cap(mut diags: Vec<Diagnostic>, max: usize) -> Vec<Diagnostic> {
    if diags.len() > max {
        let omitted = diags.len() - max;
        diags.truncate(max);
        diags.push(locale_diag(
            Severity::Warning,
            LocaleProblem::More { count: omitted },
            "",
            "",
        ));
    }
    diags
}

/// `locales/` を 1 回一覧して候補を作る (07 §17.4)。名前が `^[a-z]{2,3}\.toml$` に完全に一致する項目 (デバイス名を除く) が
/// 候補で、規則に合わない `*.toml` (拡張子の大文字小文字を問わない) はファイルごとに警告の診断にする (読まない。`max` 件と
/// 「ほか N 件」まで)。名前は一覧で得た綴りで照合する。ディレクトリが無い・読めないときは候補なし。
fn list_locales(dir: &std::path::Path, max: usize) -> (BTreeMap<Locale, PathBuf>, Vec<Diagnostic>) {
    let mut candidates = BTreeMap::new();
    let mut bad: Vec<String> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (candidates, Vec::new());
    };
    for e in entries.flatten() {
        let raw = e.file_name();
        let name = raw.to_string_lossy();
        if !name.to_ascii_lowercase().ends_with(".toml") {
            continue;
        }
        match raw.to_str().and_then(Locale::from_file_name) {
            Some(l) => {
                candidates.insert(l, e.path());
            }
            None => bad.push(name.into_owned()),
        }
    }
    bad.sort();
    let diags = bad
        .iter()
        .map(|n| locale_diag(Severity::Warning, LocaleProblem::BadName, "", n))
        .collect();
    (candidates, cap(diags, max))
}

/// 選んだ言語のユーザーの言語ファイルを開いて検査する (07 §17.4 の「開き方」)。開いた後に通常のファイルと確かめ、
/// 1 MiB + 1 バイトまで読む。
fn read_locale_file(
    locale: Locale,
    path: &std::path::Path,
    max_diagnostics: usize,
) -> eve_config::LocaleFile {
    use eve_platform::user_file::{UserFileError, read_regular_limited};
    let base = Catalog::builtin(locale);
    match read_regular_limited(path, eve_config::LOCALE_MAX_BYTES) {
        Ok(bytes) => eve_config::load_locale_file(locale.code(), &bytes, base, max_diagnostics),
        Err(UserFileError::NotRegular) => {
            eve_config::LocaleFile::unusable(locale.code(), LocaleProblem::NotRegular)
        }
        Err(UserFileError::Io(e)) => eve_config::LocaleFile::unusable(
            locale.code(),
            LocaleProblem::Unreadable {
                message: e.to_string(),
            },
        ),
    }
}

/// 訳の無いキーの情報 (`--check-config` だけ、07 §17.4): 件数の 1 件と、キーごとの 1 件 (`max` 件と「ほか N 件」まで)。
fn untranslated_diags(
    file: &eve_config::LocaleFile,
    locale: Locale,
    max: usize,
) -> Vec<Diagnostic> {
    let keys = file.untranslated(Catalog::builtin(locale));
    if keys.is_empty() {
        return Vec::new();
    }
    let mut out = vec![locale_diag(
        Severity::Info,
        LocaleProblem::Untranslated { count: keys.len() },
        "",
        "",
    )];
    out.extend(cap(
        keys.iter()
            .map(|k| locale_diag(Severity::Info, LocaleProblem::UntranslatedKey, k, ""))
            .collect(),
        max,
    ));
    out
}

/// UI の言語を選び、選んだ言語のユーザーの言語ファイルだけを読んで、同梱の表に重ねる (07 §17.4)。`resolved` の
/// `ui.locale` は使える言語と照らして診断を足す。`--no-config` (`src.locales` が `None`) では `locales/` を見ず、
/// 同梱の言語だけで選ぶ。
pub(crate) fn setup_locale(
    src: &eve_core::ConfigSources,
    cli_locale: Option<&str>,
    resolved: &mut eve_config::Resolved,
    read: LocaleRead,
) -> LocaleSetup {
    let (candidates, bad_names) = match &src.locales {
        Some(dir) => list_locales(dir.as_path(), read.max_diagnostics),
        None => (BTreeMap::new(), Vec::new()),
    };
    let available: Vec<Locale> = candidates.keys().copied().collect();
    let choice = choose_locale(cli_locale, &resolved.config, &available);
    // 探した場所 (絶対パス)。探していない (`--no-config` など) ときは空で、診断は場所を示さない文になる
    let dir_display = src.locales.as_ref().map_or_else(String::new, |d| {
        std::path::absolute(d.as_path())
            .unwrap_or_else(|_| d.as_path().to_path_buf())
            .display()
            .to_string()
    });
    eve_config::check_ui_locale(resolved, &available, &dir_display, choice.locale);
    let mut reports = Vec::new();
    if !bad_names.is_empty() {
        reports.push((eve_core::LOCALES_PREFIX.to_owned(), bad_names));
    }
    for w in &choice.warnings {
        let source = match w.source {
            eve_config::LocaleSource::Cli => eve_core::LOCALE_CLI_SOURCE,
            _ => eve_core::LOCALE_ENV_SOURCE,
        };
        let d = locale_diag(
            Severity::Warning,
            LocaleProblem::NotFound {
                dir: dir_display.clone(),
                lang: choice.locale.code().to_owned(),
            },
            "",
            &w.value,
        );
        eve_config::merge_reports(&mut reports, [(source.to_owned(), vec![d])]);
    }
    let mut catalog = Catalog::builtin_shared(choice.locale);
    let mut unusable = false;
    if let Some(path) = candidates.get(&choice.locale) {
        let file = read_locale_file(choice.locale, path, read.max_diagnostics);
        unusable = file.is_unusable();
        let mut extra = Vec::new();
        if !unusable {
            catalog = Arc::new(Catalog::builtin(choice.locale).overlay(&file));
            if read.untranslated && !choice.locale.is_builtin() {
                extra = untranslated_diags(&file, choice.locale, read.max_diagnostics);
            }
        }
        let mut diags = file.into_diagnostics();
        diags.extend(extra);
        if !diags.is_empty() {
            reports.push((
                format!("{}{}.toml", eve_core::LOCALES_PREFIX, choice.locale.code()),
                diags,
            ));
        }
    }
    LocaleSetup {
        choice,
        catalog,
        candidates,
        dir: src.locales.clone(),
        unusable,
        reports,
    }
}

/// 診断の重さ → ログのレベル。
pub(crate) fn log_level(s: eve_config::Severity) -> eve_platform::log::Level {
    match s {
        eve_config::Severity::Info => eve_platform::log::Level::Info,
        eve_config::Severity::Warning => eve_platform::log::Level::Warn,
        eve_config::Severity::Error => eve_platform::log::Level::Error,
    }
}

/// 診断 1 件のログの行 (ログは英語のみ、05 §7.3): `config <表示名>[:行:桁] <種類>[ key=…][ value=…][ reason=…]`。
/// ファイル由来の文字列 (キーの名前・値・パーサや OS の説明) は、先に `sanitize_untrusted` (制御文字などを `<U+XXXX>` に、
/// 64 文字で切る) に通し、行の全体も `escape_untrusted` に通す (07 §4.3)。
pub(crate) fn describe(name: &str, d: &eve_config::Diagnostic) -> String {
    use eve_config::sanitize_untrusted as clean;
    let mut line = if d.line > 0 {
        format!("config {name}:{}:{}", d.line, d.column)
    } else {
        format!("config {name}")
    };
    line.push(' ');
    line.push_str(eve_core::config_diagnostic_kind(d));
    if !d.key.is_empty() {
        line.push_str(&format!(" key={}", clean(&d.key)));
    }
    if !d.value.is_empty() {
        line.push_str(&format!(" value=\"{}\"", clean(&d.value)));
    }
    let reason = match &d.kind {
        DiagKind::Syntax { message }
        | DiagKind::Locale(
            LocaleProblem::Syntax { message } | LocaleProblem::Unreadable { message },
        ) => Some(clean(message)),
        DiagKind::Locale(LocaleProblem::NotFound { dir, .. }) if !dir.is_empty() => {
            Some(format!("searched {dir}"))
        }
        DiagKind::Locale(LocaleProblem::More { count }) => Some(format!("{count} more")),
        _ => None,
    };
    if let Some(r) = reason {
        line.push_str(&format!(" reason={r}"));
    }
    eve_config::escape_untrusted(&line)
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

#[cfg(test)]
mod tests {
    use super::*;

    // verifies: NFR-503
    #[test]
    fn ログの行はファイル由来の制御文字などを直す() {
        let d = Diagnostic {
            severity: Severity::Warning,
            kind: DiagKind::Locale(LocaleProblem::Syntax {
                message: "bad\u{1b}]0;x\u{7}".to_owned(),
            }),
            key: "a\u{202e}b".to_owned(),
            value: "v\u{9b}".to_owned(),
            line: 3,
            column: 1,
        };
        let line = describe("locales/fr.toml", &d);
        assert!(
            !line
                .chars()
                .any(|c| c.is_control() || eve_config::forbidden_char(c).is_some()),
            "{line:?}"
        );
        assert_eq!(
            line,
            "config locales/fr.toml:3:1 locale_syntax key=a<U+202E>b value=\"v<U+009B>\" reason=bad<U+001B>]0;x<U+0007>"
        );
        // 長いキーは 64 文字で切る。空のキーは書かない
        let long = Diagnostic {
            key: "k".repeat(200),
            value: String::new(),
            kind: DiagKind::Unsupported,
            line: 0,
            ..d
        };
        assert_eq!(
            describe("config.toml", &long),
            format!("config config.toml unsupported key={}…", "k".repeat(63))
        );
    }

    // verifies: AC-NFR-603-9
    #[test]
    fn 言語ファイルの一覧は名前の規則で候補と警告に分ける() {
        let d = std::env::temp_dir().join(format!("evo-list-locales-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for n in [
            "fr.toml",
            "ja.toml",
            "FR.toml",
            "zh-TW.toml",
            "nul.toml",
            "x.txt",
        ] {
            std::fs::write(d.join(n), "").unwrap();
        }
        let (candidates, bad) = list_locales(&d, 100);
        let names: Vec<&str> = candidates.keys().map(Locale::code).collect();
        assert_eq!(names, ["fr", "ja"]);
        let bad: Vec<&str> = bad.iter().map(|d| d.value.as_str()).collect();
        // 大文字小文字を区別しない FS では FR.toml と fr.toml は同じ項目で、一覧の綴りが残る
        assert!(
            bad.contains(&"zh-TW.toml") && bad.contains(&"nul.toml"),
            "{bad:?}"
        );
        assert!(!bad.contains(&"x.txt"));
        assert!(
            list_locales(&d.join("none"), 100).0.is_empty(),
            "無いディレクトリは候補なし"
        );
        // 規則に合わない名前の警告も上限で「ほか N 件」にまとめる
        let (_, capped) = list_locales(&d, 1);
        assert_eq!(capped.len(), 2, "{capped:?}");
        assert!(matches!(
            capped[1].kind,
            DiagKind::Locale(LocaleProblem::More { .. })
        ));
        let _ = std::fs::remove_dir_all(&d);
    }
}
