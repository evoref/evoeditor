//! 設定ファイルの診断を Message キューへ積む (docs/07 §4.3、04 §15.2 の `msg.config.*`)。
//! 最後に読んだときの診断を覚えておき、`config.show_diagnostics` で一覧 (意味的モデル `List`) にして出す。
//! 一覧の `Enter` は、診断のファイルの該当の行を開く (04 §8.5a)。

use eve_config::{DiagKind, Diagnostic, Severity, default_display};
use eve_view::{CutPolicy, InputFieldModel, ListItem, MessageLevel, OverlayModel};

use crate::command::ids;
use crate::command::{CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission};
use crate::editor::state::EditorState;
use crate::io::config_reload::KEYMAP_FILE;
use crate::layout::cell_text;
use crate::ui_model::keys;
use crate::ui_model::overlay::Overlay;
use crate::ui_model::palette::{PaletteState, WINDOW, find, lower_graphemes};

/// テーマファイルの診断の表示名の接頭辞 (`themes/<name>.toml`。eve-config の `load_theme`)。
const THEME_PREFIX: &str = "themes/";

/// ユーザーの言語ファイルの診断の表示名の接頭辞 (`locales/<lang>.toml`。名前の規則に合わないファイルの診断は `locales/`、
/// 07 §17.4)。
pub const LOCALES_PREFIX: &str = "locales/";

/// 見つからない言語の診断 (`msg.config.locale_not_found`) の表示名 (その値を書いた所)。
pub const LOCALE_CLI_SOURCE: &str = "--locale";
/// 同上 (環境変数)。
pub const LOCALE_ENV_SOURCE: &str = "EVE_LANG";

/// 型の語 (`ui.config.type.*`) のキー。toml の型名から。
fn type_key(toml_type: &str) -> String {
    let t = match toml_type {
        "boolean" => "bool",
        other => other,
    };
    format!("ui.config.type.{t}")
}

impl EditorState {
    /// 言語ファイル・言語の選択の診断を出す (07 §17.4)。`report_config` と同じだが、文言を補う言語の同梱の表で出し、
    /// `config.reload` でも一覧に残す。
    pub fn report_locale(&mut self, file: &str, diagnostics: &[Diagnostic]) {
        self.ui.locale_reports.insert(file.to_owned());
        self.report_config(file, diagnostics);
    }

    /// 診断を 1 件または要約 1 件のメッセージ (`warn`) にして出す。0 件なら何もしない。
    /// 最初の Frame の後に呼ぶ (起動を妨げない、FR-703)。
    pub fn report_config(&mut self, file: &str, diagnostics: &[Diagnostic]) {
        // 一覧 (`config.show_diagnostics`) に出す診断は、ファイルごとに最後に読んだときのものに置き換える
        self.ui.config_diagnostics.retain(|(f, _)| f != file);
        if !diagnostics.is_empty() {
            self.ui
                .config_diagnostics
                .push((file.to_owned(), diagnostics.to_vec()));
        }
        self.show_config_problems(file, diagnostics);
    }

    /// `report_config` と同じだが、一覧のそのファイルの診断を置き換えずに足す (テーマが無い診断を、`theme.name` を
    /// 書いた設定ファイルの名前で出すとき。07 §8.7)。
    pub(crate) fn report_config_more(&mut self, file: &str, diagnostics: &[Diagnostic]) {
        if !diagnostics.is_empty() {
            match self
                .ui
                .config_diagnostics
                .iter_mut()
                .find(|(f, _)| f == file)
            {
                Some((_, ds)) => ds.extend(diagnostics.iter().cloned()),
                None => self
                    .ui
                    .config_diagnostics
                    .push((file.to_owned(), diagnostics.to_vec())),
            }
        }
        self.show_config_problems(file, diagnostics);
    }

    /// 一覧から、テーマが無い診断 (`DiagKind::ThemeNotFound`) を消す (テーマを読み直す前)。
    pub(crate) fn forget_theme_not_found(&mut self) {
        for (_, ds) in &mut self.ui.config_diagnostics {
            ds.retain(|d| !matches!(d.kind, DiagKind::ThemeNotFound { .. }));
        }
        self.ui.config_diagnostics.retain(|(_, ds)| !ds.is_empty());
    }

    /// 診断を 1 件または要約 1 件のメッセージにする (`report_config` の画面の側)。
    fn show_config_problems(&mut self, file: &str, diagnostics: &[Diagnostic]) {
        // 「このバージョンでは未対応」の情報診断は画面に出さず、件数にも数えない (07 §4.3)
        let shown: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|d| d.severity != eve_config::Severity::Info)
            .collect();
        let Some(&first) = shown.first() else {
            return;
        };
        let (detail, hint) = diagnostic_texts(self.report_catalog(file, first), first);
        // 行の分からない診断 (`--set`・言語ファイルの名前と言語の選択・ファイル全体の問題) は `:{line}` を省く
        let line = if first.line == 0 {
            String::new()
        } else {
            first.line.to_string()
        };
        // 言語ファイル・言語の選択の診断の要約も、補う言語の同梱の文言で出す (07 §17.4)
        let locale = self.uses_builtin_texts(file, first);
        let show = |st: &mut EditorState, key: &str, args: &[(&str, &str)]| {
            if locale {
                st.show_builtin_message(MessageLevel::Warn, key, args);
            } else {
                st.show_message(MessageLevel::Warn, key, args);
            }
        };
        if shown.len() == 1 {
            show(
                self,
                keys::MSG_CONFIG_PROBLEM,
                &[
                    ("file", file),
                    ("line", &line),
                    ("detail", &detail),
                    ("hint", &hint),
                ],
            );
        } else {
            // 「ほか N 件」の 1 件は、省いた件数として数える
            let count = problem_count(&shown).to_string();
            show(
                self,
                keys::MSG_CONFIG_PROBLEMS,
                &[("count", &count), ("detail", &detail)],
            );
        }
    }

    /// 診断の文言を補う言語の同梱の表で出すか: 言語ファイル・言語の選択の診断 (`report_locale` で渡されたファイルか、
    /// 言語ファイルの種類の診断。設定ファイルの `ui.locale` の見つからない言語を含む)。
    fn uses_builtin_texts(&self, file: &str, d: &Diagnostic) -> bool {
        self.ui.locale_reports.contains(file) || matches!(d.kind, DiagKind::Locale(_))
    }

    /// 診断 `d` (表示名 `file`) の文言を引く表: 言語ファイル・言語の選択の診断は補う言語の同梱の表、ほかは選んだ言語の表。
    fn report_catalog(&self, file: &str, d: &Diagnostic) -> &eve_config::Catalog {
        if self.uses_builtin_texts(file, d) {
            self.builtin_catalog()
        } else {
            self.catalog()
        }
    }

    /// 覚えている診断のうち、`drop` が真のファイルのものを忘れる (読み直す前に、読み直すファイルの分を消す)。
    pub(crate) fn forget_config_diagnostics(&mut self, drop: impl Fn(&str) -> bool) {
        self.ui.config_diagnostics.retain(|(f, _)| !drop(f));
    }

    /// テーマファイルの診断か (表示名が `themes/` で始まる)。
    pub(crate) fn is_theme_diagnostic_file(file: &str) -> bool {
        file.starts_with(THEME_PREFIX)
    }

    /// 一覧の 1 行の文言: `{level}: {text} / {hint}` (対処が無ければ `{level}: {text}`)。言語ファイルの診断の行は、
    /// 補う言語の同梱の文言で出す (07 §17.4)。
    fn config_diagnostic_line(&self, file: &str, d: &Diagnostic) -> String {
        let cat = self.report_catalog(file, d);
        let (text, hint) = diagnostic_texts_long(cat, d);
        let level = cat.render(
            match d.severity {
                Severity::Error => keys::UI_HISTORY_LEVEL_ERROR,
                Severity::Warning => keys::UI_HISTORY_LEVEL_WARN,
                Severity::Info => keys::UI_HISTORY_LEVEL_INFO,
            },
            &[],
            &|_| None,
        );
        let mut args = vec![("level", level.as_str()), ("text", text.as_str())];
        let key = if hint.is_empty() {
            keys::UI_CONFIG_DIAGNOSTICS_ITEM_NO_HINT
        } else {
            args.push(("hint", hint.as_str()));
            keys::UI_CONFIG_DIAGNOSTICS_ITEM
        };
        cat.render(key, &args, &|_| None)
    }

    /// 一覧の右の列: `ファイル:行` (行の分からない診断はファイルだけ)。
    fn config_diagnostic_place(file: &str, d: &Diagnostic) -> String {
        if d.line == 0 {
            file.to_owned()
        } else {
            format!("{file}:{}", d.line)
        }
    }

    /// 入力に合う診断 (ファイルの番号, 診断の番号)。照合は 1 行の文言と `ファイル:行` の部分一致 (大小を区別しない)。
    /// 並びは読んだ順 (ファイルごと、その中は診断の順)。空の入力は全件。
    pub(crate) fn config_diagnostic_hits(&self, query: &str) -> Vec<(usize, usize)> {
        let q = lower_graphemes(query.trim());
        let mut out = Vec::new();
        for (fi, (file, diags)) in self.ui.config_diagnostics.iter().enumerate() {
            for (di, d) in diags.iter().enumerate() {
                let hit = q.is_empty()
                    || find(&lower_graphemes(&self.config_diagnostic_line(file, d)), &q).is_some()
                    || find(
                        &lower_graphemes(&Self::config_diagnostic_place(file, d)),
                        &q,
                    )
                    .is_some();
                if hit {
                    out.push((fi, di));
                }
            }
        }
        out
    }

    /// `config.show_diagnostics`: 設定の診断の一覧を開く。診断が 1 件も無ければ、開かずにそう示す。
    fn open_config_diagnostics(&mut self) -> CommandResult {
        if self.ui.config_diagnostics.iter().all(|(_, d)| d.is_empty()) {
            self.show_message(MessageLevel::Info, keys::INFO_CONFIG_NO_DIAGNOSTICS, &[]);
            return CommandResult::Done;
        }
        self.set_overlay(Some(Overlay::Palette(PaletteState {
            diagnostics: true,
            ..PaletteState::default()
        })));
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// 診断の表示名から、開くファイルのパス。`--set` (コマンドラインの値) はファイルが無いので `None`。
    fn config_diagnostic_path(&self, file: &str) -> Option<crate::FsPath> {
        let src = self.config_sources();
        if let Some((_, p)) = src.files.iter().find(|(name, _)| name == file) {
            return Some(p.clone());
        }
        if file == KEYMAP_FILE {
            return src.keymap;
        }
        // 言語ファイル (`locales/<lang>.toml`)。名前の規則に合わないファイルの診断 (`locales/`) は開かない。表示名が同じ
        // `--config` のファイルと取り違えないよう、言語ファイルの経路で渡された表示名だけを見る
        if self.ui.locale_reports.contains(file) {
            let name = file.strip_prefix(LOCALES_PREFIX)?;
            return src
                .locales
                .filter(|_| eve_config::Locale::from_file_name(name).is_some())
                .map(|dir| dir.join(name));
        }
        let theme = file.strip_prefix(THEME_PREFIX)?;
        src.themes.map(|dir| dir.join(theme))
    }

    /// `ui.confirm`: 選んだ診断のファイルを開き、該当の行・桁へ移る (開いていればそのバッファへ切り替える)。
    /// 候補が無ければ閉じない。ファイルの無い診断 (`--set`) は、一覧を閉じてそう示す。
    pub(crate) fn config_diagnostics_confirm(&mut self) -> CommandResult {
        let Some(Overlay::Palette(p)) = &self.ui.overlay else {
            return CommandResult::Done;
        };
        let hits = self.config_diagnostic_hits(&p.text);
        let Some(&(fi, di)) = hits.get(p.selected.min(hits.len().saturating_sub(1))) else {
            return CommandResult::Done;
        };
        let (file, diags) = &self.ui.config_diagnostics[fi];
        let at = |n: u32| usize::try_from(n).unwrap_or(usize::MAX).max(1);
        let (file, line, col) = (file.clone(), at(diags[di].line), at(diags[di].column));
        self.set_overlay(None);
        self.mark_dirty_render();
        let Some(path) = self.config_diagnostic_path(&file) else {
            self.show_message(
                MessageLevel::Info,
                keys::INFO_CONFIG_DIAGNOSTIC_NO_FILE,
                &[("file", &file)],
            );
            return CommandResult::Done;
        };
        self.open_at(&path, line, col)
    }

    /// `path` を開いて (開いていれば切り替えて) 行・桁 (1 始まり) へ移る。読込中なら読み終わってから移る。
    fn open_at(&mut self, path: &crate::FsPath, line: usize, col: usize) -> CommandResult {
        let find_doc = |st: &EditorState| {
            st.docs
                .documents
                .iter()
                .find(|(_, d)| d.path.as_ref() == Some(path))
                .map(|(id, d)| (*id, d.load_state == crate::document::LoadState::Ready))
        };
        let res = match find_doc(self) {
            Some((id, _)) => {
                self.activate_document(id);
                CommandResult::Done
            }
            None => self.open_path(&path.display(), crate::OpenOptions::default()),
        };
        match find_doc(self) {
            Some((id, true)) if self.active_document() == Some(id) => {
                if let Some(view) = self.focused_view() {
                    self.jump_to_line_col(view, id, line, col);
                }
            }
            Some((id, _)) => {
                self.io.open_jumps.insert(id, (line, col));
            }
            None => {}
        }
        res
    }

    /// 一覧の表示 (パレットと同じ `OverlayModel::List`)。行: 重さ・本文・対処、右に `ファイル:行`。
    pub(crate) fn config_diagnostics_model(&self, p: &PaletteState) -> OverlayModel {
        let render = |key: &str, args: &[(&str, &str)]| {
            self.catalog().render(key, args, &|c| self.key_label(c))
        };
        let hits = self.config_diagnostic_hits(&p.text);
        let selected = p.selected.min(hits.len().saturating_sub(1));
        let first = if selected < WINDOW {
            0
        } else {
            selected + 1 - WINDOW
        };
        let items: Vec<ListItem> = hits
            .iter()
            .skip(first)
            .take(WINDOW)
            .map(|&(fi, di)| {
                let (file, diags) = &self.ui.config_diagnostics[fi];
                let d = &diags[di];
                ListItem {
                    label: cell_text(&self.config_diagnostic_line(file, d), CutPolicy::End),
                    detail: Some(cell_text(
                        &Self::config_diagnostic_place(file, d),
                        CutPolicy::Middle,
                    )),
                    match_ranges: Vec::new(),
                    keys: Vec::new(),
                    disabled_reason: None,
                }
            })
            .collect();
        let total: usize = self
            .ui
            .config_diagnostics
            .iter()
            .map(|(_, d)| d.len())
            .sum();
        let cells = cell_text(&p.text, CutPolicy::End).spans;
        let cursor = cell_text(&p.text[..p.cursor], CutPolicy::End).spans.len();
        let selection = crate::ui_model::overlay::field_selection(p.all_selected, cells.len());
        let placeholder = p.text.is_empty().then(|| {
            cell_text(
                &render(keys::UI_CONFIG_DIAGNOSTICS_PLACEHOLDER, &[]),
                CutPolicy::End,
            )
        });
        let footer = render(
            keys::UI_PALETTE_FOOTER,
            &[
                ("matched", &hits.len().to_string()),
                ("total", &total.to_string()),
            ],
        );
        OverlayModel::List {
            title: cell_text(
                &render(keys::UI_CONFIG_DIAGNOSTICS_TITLE, &[]),
                CutPolicy::Never,
            ),
            input: InputFieldModel {
                len: cells.len(),
                cells,
                first: 0,
                cursor,
                selection,
                placeholder,
            },
            items,
            first,
            selected,
            footer: cell_text(&footer, CutPolicy::Never),
            no_match: cell_text(
                &render(keys::UI_CONFIG_DIAGNOSTICS_NO_MATCH, &[]),
                CutPolicy::End,
            ),
        }
    }
}

fn config_show_diagnostics(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.open_config_diagnostics()
}

/// `config.show_diagnostics` (07 §11.6)。
pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(
        ids::CONFIG_SHOW_DIAGNOSTICS,
        CommandKind::Ui,
        Permission::Ui,
        "",
        config_show_diagnostics,
    );
}

/// 診断 1 件の種別の文言 (本文, 対処)。`--check-config` (端末を使わない) も使う。
///
/// 診断に埋めるファイル由来の文字列 (`{item}` (キーの名前)・`{value}` (書かれた値)・`{reason}` (パーサや OS の説明)) は、
/// 設定ファイル・キーマップ・テーマファイル・言語ファイルのどれから来たものも、埋める前に無害化する (制御文字などを
/// `<U+XXXX>` に直し、64 文字で切る。07 §4.3、01 NFR-503)。
pub fn diagnostic_texts(cat: &eve_config::Catalog, d: &Diagnostic) -> (String, String) {
    use eve_config::sanitize_untrusted as clean;
    let default = default_display(&d.key);
    let word = |key: &str, args: &[(&str, &str)]| cat.render(key, args, &|_| None);
    let mut args: Vec<(&str, String)> = vec![("item", clean(&d.key)), ("value", clean(&d.value))];
    let kind = match &d.kind {
        // 「このバージョンでは未対応」は情報の 1 文だけ (`info.config.unsupported`、07 §4.3。対処は無い)
        DiagKind::Unsupported => {
            let item = clean(&d.key);
            return (
                word(keys::INFO_CONFIG_UNSUPPORTED, &[("item", &item)]),
                String::new(),
            );
        }
        DiagKind::Syntax { message } => {
            args.push(("reason", clean(message)));
            "syntax"
        }
        // 書ける構造の上限を超えた (07 §4.2)。構文エラーと同じ文言で、`{reason}` は理由の語 (`ui.config.syntax_reason.*`)
        DiagKind::Structure(p) => {
            let reason = word(&format!("ui.config.syntax_reason.{}", p.reason_name()), &[]);
            args.push(("reason", reason));
            "syntax"
        }
        DiagKind::UnknownKey { suggestion } => {
            if let Some(s) = suggestion {
                args.push(("suggestion", s.clone()));
            }
            "unknown_key"
        }
        DiagKind::TypeMismatch { expected, found } => {
            args.push(("expected", word(&type_key(expected), &[])));
            args.push(("actual", word(&type_key(found), &[])));
            args.push(("default", default.to_owned()));
            // 既定値を持たない項目 (節が表でない・テーマの値・言語ファイルの文言) は、既定値の文の無い文言にする (04 §15.2)
            if default.is_empty() {
                "type_mismatch_no_default"
            } else {
                "type_mismatch"
            }
        }
        DiagKind::OutOfRange { min, max } => {
            args.push(("range", format!("{min}〜{max}")));
            args.push(("default", default.to_owned()));
            "out_of_range"
        }
        DiagKind::Keymap(p) => {
            use eve_config::KeymapProblem as P;
            match p {
                P::UnknownContext => "keymap_unknown_context",
                P::UnknownCommand => "keymap_unknown_command",
                P::BadKey => "keymap_bad_key",
                P::BadWhen => "keymap_bad_when",
                P::UnknownWhenVar => "keymap_unknown_when_var",
                P::PrintableKey => "keymap_printable_key",
                P::ChordPrefix => "keymap_chord_prefix",
                P::EscapeKey => "keymap_escape_key",
            }
        }
        DiagKind::ThemeNotFound { candidates } => {
            // 候補は `"…"` で囲んで ` / ` でつなぐ。無ければ 〔 〕 の文を省く (04 §15.2)
            if !candidates.is_empty() {
                let quoted: Vec<String> = candidates.iter().map(|c| format!("\"{c}\"")).collect();
                args.push(("candidates", quoted.join(" / ")));
            }
            "theme_not_found"
        }
        // ユーザーの言語ファイルの診断 (07 §17.4、04 §15.2)。理由の語は `ui.config.locale_reason.*` (ファイル全体の問題)・
        // `ui.config.locale_char.*` (使えない文字の種類)・`ui.config.locale_placeholder.*` (差し込みの違い)
        DiagKind::Locale(p) => {
            // 訳の無いキー (`--check-config` だけに出す情報) は 1 文
            if matches!(
                p,
                eve_config::LocaleProblem::Untranslated { .. }
                    | eve_config::LocaleProblem::UntranslatedKey
            ) {
                let count = match p {
                    eve_config::LocaleProblem::Untranslated { count } => count.to_string(),
                    _ => String::new(),
                };
                let item = clean(&d.key);
                let key = format!("info.config.{}", p.message_name());
                return (
                    word(&key, &[("count", &count), ("item", &item)]),
                    String::new(),
                );
            }
            locale_args(cat, p, &mut args);
            p.message_name()
        }
        DiagKind::InvalidValue { allowed } => {
            args.push(("allowed", allowed.join(", ")));
            args.push(("default", default.to_owned()));
            if default.is_empty() {
                "invalid_value_no_default"
            } else {
                "invalid_value"
            }
        }
    };
    let refs: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let key = |part: &str| format!("msg.config.{kind}.{part}");
    (
        cat.render(&key("text"), &refs, &|_| None),
        cat.render(&key("hint"), &refs, &|_| None),
    )
}

/// 言語ファイルの診断 (`msg.config.locale_*`) の引数。理由の語は `cat` で解決する。
fn locale_args(
    cat: &eve_config::Catalog,
    p: &eve_config::LocaleProblem,
    args: &mut Vec<(&str, String)>,
) {
    use eve_config::LocaleProblem as L;
    let word = |key: &str, a: &[(&str, &str)]| cat.render(key, a, &|_| None);
    let reason = |name: &str| word(&format!("ui.config.locale_reason.{name}"), &[]);
    match p {
        L::Syntax { message } => args.push(("reason", eve_config::sanitize_untrusted(message))),
        L::Unreadable { message } => args.push(("reason", eve_config::sanitize_untrusted(message))),
        L::InlineValue => args.push(("reason", reason("inline_value"))),
        L::TooDeep => args.push(("reason", reason("too_deep"))),
        L::TooManyKeys => args.push(("reason", reason("too_many_keys"))),
        L::NotRegular => args.push(("reason", reason("not_regular"))),
        L::TooLarge => args.push(("size", reason("too_large"))),
        L::NotFound { dir, lang } => {
            // パスは切らない (場所を示すため)。制御文字などだけを直す
            args.push(("dir", eve_config::escape_untrusted(dir)));
            args.push(("lang", lang.clone()));
        }
        L::LanguageMismatch { lang } => args.push(("lang", lang.clone())),
        L::TooLong { count } | L::More { count } => args.push(("count", count.to_string())),
        L::BadChar { class, code } => {
            let cp = *code;
            let code = format!("<U+{cp:04X}>");
            let name = match class {
                eve_config::CharClass::Control => "control",
                // 異体字セレクタ (U+FE0F の絵文字の列・IVS) は書式文字の中で別の語にする
                eve_config::CharClass::Format
                    if (0xFE00..=0xFE0F).contains(&cp) || (0xE0100..=0xE01EF).contains(&cp) =>
                {
                    "variation_selector"
                }
                eve_config::CharClass::Format => "format",
                eve_config::CharClass::PrivateUse => "private_use",
                eve_config::CharClass::Noncharacter => "noncharacter",
                eve_config::CharClass::CombiningMarks => "combining_marks",
            };
            args.push((
                "reason",
                word(&format!("ui.config.locale_char.{name}"), &[("code", &code)]),
            ));
        }
        L::Placeholder { expected, issue } => {
            args.push(("expected", expected_placeholders(expected)));
            args.push((
                "reason",
                word(
                    &format!("ui.config.locale_placeholder.{}", issue.name()),
                    &[],
                ),
            ));
        }
        L::HintWidth { width } => args.push(("width", width.to_string())),
        L::NotUtf8
        | L::BadName
        | L::UnknownKey
        | L::Reserved
        | L::DuplicateKey
        | L::Empty
        | L::Accelerator
        | L::Untranslated { .. }
        | L::UntranslatedKey => {}
    }
}

/// `diagnostic_texts` の、一覧 (`config.show_diagnostics`) と `--check-config` の形: 見つからない言語の診断の対処に、
/// 探した場所 (`ui.config.locale_searched`) を足す (メッセージ行には長いパスを出さない、04 §15.2)。
pub fn diagnostic_texts_long(cat: &eve_config::Catalog, d: &Diagnostic) -> (String, String) {
    let (text, mut hint) = diagnostic_texts(cat, d);
    if let DiagKind::Locale(eve_config::LocaleProblem::NotFound { dir, .. }) = &d.kind
        && !dir.is_empty()
    {
        let dir = eve_config::escape_untrusted(dir);
        let searched = cat.render("ui.config.locale_searched", &[("dir", &dir)], &|_| None);
        hint = if hint.is_empty() {
            searched
        } else {
            format!("{hint} {searched}")
        };
    }
    (text, hint)
}

/// 診断の種類の名前 (`msg.config.<これ>` の部分。「未対応」は `unsupported`)。ログの行に使う (英語のみ、05 §7.3)。
pub fn diagnostic_kind(d: &Diagnostic) -> &'static str {
    match &d.kind {
        DiagKind::Unsupported => "unsupported",
        DiagKind::Syntax { .. } | DiagKind::Structure(_) => "syntax",
        DiagKind::UnknownKey { .. } => "unknown_key",
        DiagKind::TypeMismatch { .. } => "type_mismatch",
        DiagKind::OutOfRange { .. } => "out_of_range",
        DiagKind::InvalidValue { .. } => "invalid_value",
        DiagKind::Keymap(_) => "keymap",
        DiagKind::ThemeNotFound { .. } => "theme_not_found",
        DiagKind::Locale(p) => p.message_name(),
    }
}

/// 診断の件数 (要約と `--check-config` の件数の行): 「ほか N 件」(`locale_more`) の 1 件は N 件と数える。
pub fn problem_count(diags: &[&Diagnostic]) -> usize {
    diags
        .iter()
        .map(|d| match &d.kind {
            DiagKind::Locale(eve_config::LocaleProblem::More { count }) => *count,
            _ => 1,
        })
        .sum()
}

/// `msg.config.locale_placeholder` の `{expected}` (04 §15.2): 同梱の文言のプレースホルダを、現れる順に波括弧付きで
/// `, ` で並べる。同じ名前は 1 回だけ書き、2 回以上現れるものは ` ×N` (N は現れる数。ユーザーの文言で使える上限) を付ける。
fn expected_placeholders(names: &[String]) -> String {
    let mut seen: Vec<(&str, usize)> = Vec::new();
    for n in names {
        match seen.iter_mut().find(|(s, _)| *s == n.as_str()) {
            Some((_, c)) => *c += 1,
            None => seen.push((n.as_str(), 1)),
        }
    }
    seen.iter()
        .map(|(n, c)| {
            if *c > 1 {
                format!("{{{n}}} ×{c}")
            } else {
                format!("{{{n}}}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::diagnostic_texts;
    use eve_config::{Catalog, DiagKind, Diagnostic, Severity};

    // verifies: FR-703
    #[test]
    fn 書ける構造の上限の診断は構文エラーの文言に理由の語を入れる() {
        use eve_config::StructureProblem as P;
        for (p, ja, en) in [
            (
                P::Nesting,
                "インラインの表と配列の入れ子が 16 段を超えています",
                "nested more than 16 levels deep",
            ),
            (
                P::KeyDepth,
                "16 段を超えるキーがあります",
                "a key has more than 16 levels",
            ),
        ] {
            let d = Diagnostic {
                severity: Severity::Error,
                kind: DiagKind::Structure(p),
                key: String::new(),
                value: String::new(),
                line: 1,
                column: 5,
            };
            assert_eq!(super::diagnostic_kind(&d), "syntax");
            let (text, hint) = diagnostic_texts(Catalog::builtin_ja(), &d);
            assert!(
                text.starts_with("この設定ファイルは書式が正しくない"),
                "{text}"
            );
            assert!(text.ends_with(ja), "{text}");
            assert_ne!(hint, "");
            let (text, _) = diagnostic_texts(Catalog::builtin_en(), &d);
            assert!(text.ends_with(en), "{text}");
        }
    }

    #[test]
    fn 未対応の診断は情報の文言で対処は空() {
        let d = Diagnostic {
            severity: Severity::Info,
            kind: DiagKind::Unsupported,
            key: "ai".to_owned(),
            value: String::new(),
            line: 1,
            column: 2,
        };
        for cat in [Catalog::builtin_ja(), Catalog::builtin_en()] {
            let (text, hint) = diagnostic_texts(cat, &d);
            assert!(text.starts_with("ai "), "{text}");
            assert!(!text.contains("msg.config"), "{text}");
            assert_eq!(hint, "");
        }
    }

    #[test]
    fn テーマが無い診断は候補を対処に入れ_候補が無ければその文を省く() {
        let d = |candidates| Diagnostic {
            severity: Severity::Error,
            kind: DiagKind::ThemeNotFound { candidates },
            key: "theme.name".to_owned(),
            value: "high-contrast".to_owned(),
            line: 3,
            column: 1,
        };
        let with = d(vec!["high-contrast-dark", "high-contrast-light"]);
        let (text, hint) = diagnostic_texts(Catalog::builtin_ja(), &with);
        assert_eq!(text, "テーマ high-contrast がありません");
        assert!(
            hint.starts_with(
                "theme.name を \"high-contrast-dark\" / \"high-contrast-light\" にしてください。"
            ),
            "{hint}"
        );
        assert!(hint.ends_with("今は default で描いています"), "{hint}");
        let (text, hint) = diagnostic_texts(Catalog::builtin_en(), &with);
        assert_eq!(text, "There is no theme named high-contrast");
        assert!(
            hint.starts_with(
                "Set theme.name to \"high-contrast-dark\" / \"high-contrast-light\". "
            ),
            "{hint}"
        );
        let without = d(Vec::new());
        for cat in [Catalog::builtin_ja(), Catalog::builtin_en()] {
            let (_, hint) = diagnostic_texts(cat, &without);
            assert!(!hint.contains("high-contrast"), "{hint}");
            assert!(!hint.contains('{') && !hint.contains('〔'), "{hint}");
        }
    }

    #[test]
    fn 既定値を持たない項目の対処の文に空の既定値を入れない() {
        let d = |key: &str, kind| Diagnostic {
            severity: Severity::Warning,
            kind,
            key: key.to_owned(),
            value: "nope".to_owned(),
            line: 1,
            column: 1,
        };
        let cases = [
            d(
                "editor",
                DiagKind::TypeMismatch {
                    expected: "table",
                    found: "integer",
                },
            ),
            d(
                "ui.selection.fg",
                DiagKind::InvalidValue {
                    allowed: vec!["#rrggbb".to_owned()],
                },
            ),
        ];
        for d in &cases {
            let (_, ja) = diagnostic_texts(Catalog::builtin_ja(), d);
            assert!(ja.ends_with("この項目は使いません"), "{ja}");
            assert!(!ja.contains("既定値"), "{ja}");
            let (_, en) = diagnostic_texts(Catalog::builtin_en(), d);
            assert!(en.ends_with("This item is ignored"), "{en}");
        }
        // 既定値を持つ項目は今までどおり
        let t = d(
            "editor.tab_width",
            DiagKind::TypeMismatch {
                expected: "integer",
                found: "string",
            },
        );
        let (_, ja) = diagnostic_texts(Catalog::builtin_ja(), &t);
        assert!(ja.ends_with("既定値 4 を使います"), "{ja}");
    }

    // verifies: NFR-503
    #[test]
    fn 診断に埋めるファイル由来の文字列を無害化する() {
        let d = Diagnostic {
            severity: Severity::Warning,
            kind: DiagKind::InvalidValue {
                allowed: vec!["a".to_owned()],
            },
            key: "editor.x\u{1b}[2J".to_owned(),
            value: format!("\u{202e}{}", "v".repeat(100)),
            line: 1,
            column: 1,
        };
        for cat in [Catalog::builtin_ja(), Catalog::builtin_en()] {
            let (text, hint) = diagnostic_texts(cat, &d);
            for s in [&text, &hint] {
                assert!(
                    !s.chars().any(|c| c.is_control() || c == '\u{202e}'),
                    "{s:?}"
                );
            }
            assert!(text.contains("editor.x<U+001B>[2J"), "{text}");
            assert!(text.contains("<U+202E>vvv"), "{text}");
            assert!(text.contains('…'), "64 文字で切る: {text}");
        }
        // パーサの説明 (`{reason}`) も
        let s = Diagnostic {
            severity: Severity::Error,
            kind: DiagKind::Syntax {
                message: "bad\nline\u{9b}".to_owned(),
            },
            key: String::new(),
            value: String::new(),
            line: 0,
            column: 0,
        };
        let (text, _) = diagnostic_texts(Catalog::builtin_ja(), &s);
        assert!(text.ends_with("bad<U+000A>line<U+009B>"), "{text}");
        // 「未対応」の情報診断の `{item}` も
        let u = Diagnostic {
            severity: Severity::Info,
            kind: DiagKind::Unsupported,
            key: "ai\u{7}".to_owned(),
            value: String::new(),
            line: 1,
            column: 1,
        };
        let (text, _) = diagnostic_texts(Catalog::builtin_en(), &u);
        assert!(text.starts_with("ai<U+0007> "), "{text}");
    }

    // verifies: NFR-603
    #[test]
    fn 言語ファイルの診断はどの種類も文言と理由の語を解決する() {
        use eve_config::{CharClass, LocaleProblem as L};
        let problems = vec![
            L::Syntax {
                message: "expected `=`".to_owned(),
            },
            L::NotUtf8,
            L::TooLarge,
            L::InlineValue,
            L::TooDeep,
            L::TooManyKeys,
            L::Unreadable {
                message: "denied".to_owned(),
            },
            L::NotRegular,
            L::BadName,
            L::NotFound {
                dir: "/c/locales".to_owned(),
                lang: "ja".to_owned(),
            },
            L::LanguageMismatch {
                lang: "fr".to_owned(),
            },
            L::UnknownKey,
            L::Reserved,
            L::DuplicateKey,
            L::TooLong { count: 2000 },
            L::Empty,
            L::BadChar {
                class: CharClass::Control,
                code: 0x1b,
            },
            L::BadChar {
                class: CharClass::Format,
                code: 0x202e,
            },
            L::BadChar {
                class: CharClass::PrivateUse,
                code: 0xe000,
            },
            L::BadChar {
                class: CharClass::Noncharacter,
                code: 0xfffe,
            },
            L::BadChar {
                class: CharClass::CombiningMarks,
                code: 0x301,
            },
            L::BadChar {
                class: CharClass::Format,
                code: 0xfe0f,
            },
            L::Placeholder {
                expected: vec!["path".to_owned()],
                issue: eve_config::PlaceholderIssue::Missing,
            },
            L::Placeholder {
                expected: Vec::new(),
                issue: eve_config::PlaceholderIssue::Unbalanced,
            },
            L::Placeholder {
                expected: vec!["a".to_owned()],
                issue: eve_config::PlaceholderIssue::Conditional,
            },
            L::Placeholder {
                expected: vec!["a".to_owned()],
                issue: eve_config::PlaceholderIssue::Unknown,
            },
            L::Placeholder {
                expected: vec!["a".to_owned()],
                issue: eve_config::PlaceholderIssue::TooMany,
            },
            L::NotFound {
                dir: String::new(),
                lang: "ja".to_owned(),
            },
            L::Accelerator,
            L::HintWidth { width: 12 },
            L::More { count: 3 },
        ];
        for p in problems {
            let d = Diagnostic {
                severity: Severity::Warning,
                kind: DiagKind::Locale(p.clone()),
                key: "msg.save.failed.text".to_owned(),
                value: "zz".to_owned(),
                line: 2,
                column: 1,
            };
            for cat in [Catalog::builtin_ja(), Catalog::builtin_en()] {
                let (text, hint) = diagnostic_texts(cat, &d);
                for s in [&text, &hint] {
                    assert!(!s.is_empty(), "{p:?}");
                    assert!(
                        !s.contains("msg.config")
                            && !s.contains("ui.config")
                            && ![
                                "{item",
                                "{value",
                                "{reason",
                                "{count",
                                "{dir",
                                "{lang",
                                "{size",
                                "{width",
                                "{code",
                                "{expected",
                            ]
                            .iter()
                            .any(|n| s.contains(n)),
                        "{p:?}: {s}"
                    );
                }
            }
        }
        let reason = |p| {
            let d = Diagnostic {
                severity: Severity::Error,
                kind: DiagKind::Locale(p),
                key: String::new(),
                value: String::new(),
                line: 0,
                column: 0,
            };
            diagnostic_texts(Catalog::builtin_ja(), &d).0
        };
        assert_eq!(
            reason(L::TooLarge),
            "この言語ファイルは大きすぎるため使っていません (1 MiB を超えています)"
        );
        assert_eq!(
            reason(L::NotRegular),
            "この言語ファイルを読めないため使っていません: 通常のファイルではありません"
        );
        assert_eq!(
            reason(L::BadChar {
                class: CharClass::Control,
                code: 0x1b
            }),
            " の文言に使えない文字があります (制御文字 <U+001B>)"
        );
    }

    #[test]
    fn 差し込みの一覧は同じ名前を_1_回にして数を付ける() {
        let names = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            super::expected_placeholders(&names(&["path", "reason", "path"])),
            "{path} ×2, {reason}"
        );
        assert_eq!(
            super::expected_placeholders(&names(&["key:file.save"])),
            "{key:file.save}"
        );
        assert_eq!(super::expected_placeholders(&[]), "");
        let d = Diagnostic {
            severity: Severity::Warning,
            kind: DiagKind::Locale(eve_config::LocaleProblem::Placeholder {
                expected: names(&["n", "n"]),
                issue: eve_config::PlaceholderIssue::TooMany,
            }),
            key: "status.untitled".to_owned(),
            value: String::new(),
            line: 1,
            column: 1,
        };
        let (text, hint) = diagnostic_texts(Catalog::builtin_ja(), &d);
        assert!(
            text.ends_with("(同じ差し込みが多すぎます)。使うもの: {n} ×2"),
            "{text}"
        );
        assert_eq!(hint, "同梱と同じ差し込みを使ってください");
        let none = Diagnostic {
            kind: DiagKind::Locale(eve_config::LocaleProblem::Placeholder {
                expected: Vec::new(),
                issue: eve_config::PlaceholderIssue::Unknown,
            }),
            ..d
        };
        let (text, _) = diagnostic_texts(Catalog::builtin_en(), &none);
        assert!(
            text.ends_with("(an unknown placeholder)"),
            "空なら 〔 〕 の文を省く: {text}"
        );
    }

    /// 言語ファイルの診断のすべての種類 (同梱の ja / en の文言で出す)。
    fn locale_problems() -> Vec<eve_config::LocaleProblem> {
        use eve_config::{CharClass, LocaleProblem as L, PlaceholderIssue as I};
        let mut v = vec![
            L::Syntax {
                message: "expected `=`".to_owned(),
            },
            L::NotUtf8,
            L::TooLarge,
            L::InlineValue,
            L::TooDeep,
            L::TooManyKeys,
            L::Unreadable {
                message: "Access is denied. (os error 5)".to_owned(),
            },
            L::NotRegular,
            L::BadName,
            L::NotFound {
                dir: "C:\\Users\\someone\\AppData\\Roaming\\evoeditor\\locales".to_owned(),
                lang: "ja".to_owned(),
            },
            L::NotFound {
                dir: String::new(),
                lang: "ja".to_owned(),
            },
            L::LanguageMismatch {
                lang: "fr".to_owned(),
            },
            L::UnknownKey,
            L::Reserved,
            L::DuplicateKey,
            L::TooLong { count: 2000 },
            L::Empty,
            L::Accelerator,
            L::HintWidth { width: 12 },
            L::More { count: 3 },
        ];
        for class in [
            CharClass::Control,
            CharClass::Format,
            CharClass::PrivateUse,
            CharClass::Noncharacter,
            CharClass::CombiningMarks,
        ] {
            v.push(L::BadChar { class, code: 0x1b });
        }
        for issue in [
            I::Unbalanced,
            I::Conditional,
            I::Unknown,
            I::Missing,
            I::TooMany,
        ] {
            v.push(L::Placeholder {
                expected: vec!["path".to_owned(), "reason".to_owned()],
                issue,
            });
        }
        v
    }

    // メッセージ行は対処 (hint) を省略せずに本文を詰める (04 §8.9) ので、言語ファイルの診断の対処は短くし、80 桁でも
    // 何が起きたかの本文が残るようにする (04 §15.2 の「共通の規則」)
    // verifies: NFR-603
    #[test]
    fn 言語ファイルの診断の対処は短く_80_桁で本文が残る() {
        use eve_config::hint_width;
        // メッセージ行の 1 行: 印 (1 セル) と空白、本文、空白 2 つ、対処 (eve-tui の `render_message`)
        const COLS: usize = 80;
        const HINT_MAX: usize = 40;
        for cat in [Catalog::builtin_ja(), Catalog::builtin_en()] {
            for p in locale_problems() {
                let d = Diagnostic {
                    severity: Severity::Warning,
                    kind: DiagKind::Locale(p.clone()),
                    key: "ui.confirm.save.unmappable_replace.message".to_owned(),
                    value: "zz".to_owned(),
                    line: 123,
                    column: 1,
                };
                let (_, hint) = diagnostic_texts(cat, &d);
                assert!(
                    hint_width(&hint) <= HINT_MAX,
                    "{p:?}: {hint} ({})",
                    hint_width(&hint)
                );
                assert!(COLS - 2 - 2 - hint_width(&hint) >= 36, "{p:?}");
            }
            // 2 件以上の要約の対処も短い (ログのパスを出さない)
            let problems = cat.render("msg.config.problems.hint", &[], &|_| None);
            assert!(hint_width(&problems) <= HINT_MAX, "{problems}");
            assert!(!problems.contains('{'), "{problems}");
        }
    }

    // verifies: NFR-603
    #[test]
    fn 見つからない言語の探した場所は一覧と_check_config_の形だけに出す() {
        let d = Diagnostic {
            severity: Severity::Warning,
            kind: DiagKind::Locale(eve_config::LocaleProblem::NotFound {
                dir: "/home/u/.config/evoeditor/locales".to_owned(),
                lang: "ja".to_owned(),
            }),
            key: String::new(),
            value: "fr".to_owned(),
            line: 0,
            column: 0,
        };
        let (_, short) = diagnostic_texts(Catalog::builtin_ja(), &d);
        assert!(!short.contains("/home/u"), "{short}");
        assert_eq!(short, "locales/ に置くか ja / en を指定");
        let (_, long) = super::diagnostic_texts_long(Catalog::builtin_ja(), &d);
        assert_eq!(
            long,
            "locales/ に置くか ja / en を指定 (探した場所: /home/u/.config/evoeditor/locales)"
        );
        // 探していない (`--no-config`) ときは別の文
        let n = Diagnostic {
            kind: DiagKind::Locale(eve_config::LocaleProblem::NotFound {
                dir: String::new(),
                lang: "en".to_owned(),
            }),
            ..d
        };
        let (_, hint) = super::diagnostic_texts_long(Catalog::builtin_ja(), &n);
        assert_eq!(hint, "言語ファイルを読まない起動です");
        // 異体字セレクタは別の語
        let vs = Diagnostic {
            kind: DiagKind::Locale(eve_config::LocaleProblem::BadChar {
                class: eve_config::CharClass::Format,
                code: 0xfe0f,
            }),
            key: "a.b".to_owned(),
            ..n
        };
        let (text, _) = diagnostic_texts(Catalog::builtin_ja(), &vs);
        assert!(text.ends_with("(異体字セレクタ <U+FE0F>)"), "{text}");
    }
}
