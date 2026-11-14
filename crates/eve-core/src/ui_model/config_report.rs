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

/// 型の語 (`ui.config.type.*`) のキー。toml の型名から。
fn type_key(toml_type: &str) -> String {
    let t = match toml_type {
        "boolean" => "bool",
        other => other,
    };
    format!("ui.config.type.{t}")
}

impl EditorState {
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
        let (detail, hint) = self.diagnostic_texts(first);
        let line = first.line.to_string();
        if shown.len() == 1 {
            self.show_message(
                MessageLevel::Warn,
                keys::MSG_CONFIG_PROBLEM,
                &[
                    ("file", file),
                    ("line", &line),
                    ("detail", &detail),
                    ("hint", &hint),
                ],
            );
        } else {
            let count = shown.len().to_string();
            let log = self
                .io
                .log_path
                .clone()
                .unwrap_or_else(|| "ログなし".to_owned());
            self.show_message(
                MessageLevel::Warn,
                keys::MSG_CONFIG_PROBLEMS,
                &[("count", &count), ("detail", &detail), ("log", &log)],
            );
        }
    }

    /// 診断 1 件の種別の文言 (本文, 対処)。
    fn diagnostic_texts(&self, d: &Diagnostic) -> (String, String) {
        diagnostic_texts(self.catalog(), d)
    }

    /// 覚えている診断のうち、`drop` が真のファイルのものを忘れる (読み直す前に、読み直すファイルの分を消す)。
    pub(crate) fn forget_config_diagnostics(&mut self, drop: impl Fn(&str) -> bool) {
        self.ui.config_diagnostics.retain(|(f, _)| !drop(f));
    }

    /// テーマファイルの診断か (表示名が `themes/` で始まる)。
    pub(crate) fn is_theme_diagnostic_file(file: &str) -> bool {
        file.starts_with(THEME_PREFIX)
    }

    /// 一覧の 1 行の文言: `{level}: {text} / {hint}` (対処が無ければ `{level}: {text}`)。
    fn config_diagnostic_line(&self, d: &Diagnostic) -> String {
        let (text, hint) = self.diagnostic_texts(d);
        let level = self.catalog().render(
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
        self.catalog().render(key, &args, &|_| None)
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
                    || find(&lower_graphemes(&self.config_diagnostic_line(d)), &q).is_some()
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
                    label: cell_text(&self.config_diagnostic_line(d), CutPolicy::End),
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
pub fn diagnostic_texts(cat: &eve_config::Catalog, d: &Diagnostic) -> (String, String) {
    let default = default_display(&d.key);
    let mut args: Vec<(&str, String)> = vec![("item", d.key.clone()), ("value", d.value.clone())];
    let kind = match &d.kind {
        // 「このバージョンでは未対応」は情報の 1 文だけ (`info.config.unsupported`、07 §4.3。対処は無い)
        DiagKind::Unsupported => {
            let item = [("item", d.key.as_str())];
            return (
                cat.render(keys::INFO_CONFIG_UNSUPPORTED, &item, &|_| None),
                String::new(),
            );
        }
        DiagKind::Syntax { message } => {
            args.push(("reason", message.clone()));
            "syntax"
        }
        DiagKind::UnknownKey { suggestion } => {
            if let Some(s) = suggestion {
                args.push(("suggestion", s.clone()));
            }
            "unknown_key"
        }
        DiagKind::TypeMismatch { expected, found } => {
            let word = |t: &str| cat.render(&type_key(t), &[], &|_| None);
            args.push(("expected", word(expected)));
            args.push(("actual", word(found)));
            args.push(("default", default.to_owned()));
            // 既定値を持たない項目 (節が表でない・テーマの値) は、既定値の文の無い文言にする (04 §15.2)
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
        // ユーザーの言語ファイルの診断 (07 §17.4)。まだどこからも作らない (M26 の (2) で読込に載せ、文言を言語ファイルに
        // 足す。理由の語 (`{reason}` の `ui.config.locale_char.*` など) も (2) で決める)
        DiagKind::Locale(p) => {
            use eve_config::LocaleProblem as L;
            match p {
                L::Syntax { message } => args.push(("reason", message.clone())),
                L::LanguageMismatch { lang } => args.push(("lang", lang.clone())),
                L::TooLong { count } | L::More { count } => {
                    args.push(("count", count.to_string()));
                }
                L::BadChar { code, .. } => args.push(("reason", format!("<U+{code:04X}>"))),
                L::Placeholder { expected } => {
                    let list: Vec<String> = expected.iter().map(|n| format!("{{{n}}}")).collect();
                    args.push(("expected", list.join(", ")));
                }
                L::HintWidth { width } => args.push(("width", width.to_string())),
                L::NotUtf8
                | L::InlineValue
                | L::TooDeep
                | L::TooManyKeys
                | L::TooLarge
                | L::UnknownKey
                | L::Reserved
                | L::DuplicateKey
                | L::Empty
                | L::Accelerator => {}
            }
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

#[cfg(test)]
mod tests {
    use super::diagnostic_texts;
    use eve_config::{Catalog, DiagKind, Diagnostic, Severity};

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
                    allowed: &["#rrggbb"],
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
}
