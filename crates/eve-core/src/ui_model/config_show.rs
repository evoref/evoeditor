//! 設定の値と由来の一覧 (`config.show`、docs/07 §3.5・§3.4 の COND-7、04 §8.15)。
//!
//! 公開キーを宣言の表 (07 §4.1) の順に 1 行ずつ `キー = 値` で出し、右の列に由来 (既定・ユーザー: ファイル:行・`--set`。
//! 条件付きセクションの値は `[os.windows]` を足す) を出す。部品は `config.show_diagnostics` と同じ一覧 (意味的モデル
//! `List`、04 §8.5a)。`Enter` は由来にファイルと行があれば、そのファイルのその行を開く。文書ごとに決まるキー (07 §3.6) は、
//! フォーカスの文書の値と、勝った所 (言語定義なら `言語 rust (組込み)`・`言語 rust: languages/rust.toml:3`) を出し、見出しに
//! 文書の名前と言語を出す。
//!
//! spec: 07 §3.5, 07 §3.4, 07 §3.6

use eve_config::{LangOrigin, Origin, ValueOrigin, escape_untrusted, sanitize_untrusted};
use eve_view::{CutPolicy, InputFieldModel, ListItem, OverlayModel};

use crate::command::ids;
use crate::command::{CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission};
use crate::editor::state::EditorState;
use crate::layout::cell_text;
use crate::ui_model::keys;
use crate::ui_model::overlay::Overlay;
use crate::ui_model::palette::{PaletteState, WINDOW, find, lower_graphemes};

/// 一覧の 1 行: `キー = 値`・由来の文言・由来。
struct Row {
    label: String,
    origin_text: String,
    origin: ValueOrigin,
}

impl EditorState {
    /// 由来の文言 (`ui.config_origin.*`)。ファイルの名前は切らずに制御文字などだけを直す (場所を示すため)。
    fn config_origin_text(&self, origin: &Origin) -> String {
        let render = |key: &str, args: &[(&str, &str)]| self.catalog().render(key, args, &|_| None);
        let base = match origin {
            Origin::Default => render(keys::UI_CONFIG_ORIGIN_DEFAULT, &[]),
            Origin::User { source, line, .. } => render(
                keys::UI_CONFIG_ORIGIN_USER,
                &[
                    ("file", &escape_untrusted(source)),
                    ("line", &line.to_string()),
                ],
            ),
            Origin::Cli { .. } => render(keys::UI_CONFIG_ORIGIN_CLI, &[]),
        };
        match origin.condition() {
            Some(c) => render(
                keys::UI_CONFIG_ORIGIN_CONDITION,
                &[("origin", &base), ("condition", c.name())],
            ),
            None => base,
        }
    }

    /// 文書ごとに決まる値の由来の文言 (言語定義なら `ui.config_origin.language_*`、ほかは層の由来)。
    fn value_origin_text(&self, origin: &ValueOrigin) -> String {
        let render = |key: &str, args: &[(&str, &str)]| self.catalog().render(key, args, &|_| None);
        match origin {
            ValueOrigin::Config(o) => self.config_origin_text(o),
            ValueOrigin::Language { language, at } => {
                let language = sanitize_untrusted(language);
                match at {
                    LangOrigin::Builtin => render(
                        keys::UI_CONFIG_ORIGIN_LANGUAGE_BUILTIN,
                        &[("language", &language)],
                    ),
                    LangOrigin::File { source, line, .. } => render(
                        keys::UI_CONFIG_ORIGIN_LANGUAGE_FILE,
                        &[
                            ("language", &language),
                            ("file", &escape_untrusted(source)),
                            ("line", &line.to_string()),
                        ],
                    ),
                }
            }
        }
    }

    /// 公開キーの行 (宣言の表の順)。文書ごとに決まるキーは、フォーカスの文書の値と由来 (07 §3.6 の LAYER-5)。値は設定
    /// ファイル由来の文字列を含み得るので無害化する (07 §4.3)。
    fn config_show_rows(&self) -> Vec<Row> {
        let doc = self
            .active_document()
            .and_then(|d| self.docs.documents.get(&d))
            .map(|d| self.doc_values_of(d));
        eve_config::declared(self.config())
            .into_iter()
            .map(|(key, value)| {
                let per_doc = doc.as_ref().and_then(|v| match key {
                    "editor.tab_width" => {
                        Some((v.tab_width.value.to_string(), &v.tab_width.origin))
                    }
                    "editor.indent_width" => {
                        Some((v.indent_width.value.to_string(), &v.indent_width.origin))
                    }
                    "editor.indent_unit" => Some((
                        format!("\"{}\"", v.indent_unit.value.name()),
                        &v.indent_unit.origin,
                    )),
                    _ => None,
                });
                let (value, origin) = match per_doc {
                    Some((value, origin)) => (value, origin.clone()),
                    None => (
                        value,
                        ValueOrigin::Config(self.settings.config_origins.of(key).clone()),
                    ),
                };
                let label = self.catalog().render(
                    keys::UI_CONFIG_SHOW_ITEM,
                    &[("key", key), ("value", &sanitize_untrusted(&value))],
                    &|_| None,
                );
                Row {
                    label,
                    origin_text: self.value_origin_text(&origin),
                    origin,
                }
            })
            .collect()
    }

    /// 入力に合う行の番号。照合は `キー = 値` と由来の部分一致 (大小を区別しない)。空の入力は全件。
    pub(crate) fn config_show_hits(&self, query: &str) -> Vec<usize> {
        let q = lower_graphemes(query.trim());
        self.config_show_rows()
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                q.is_empty()
                    || find(&lower_graphemes(&r.label), &q).is_some()
                    || find(&lower_graphemes(&r.origin_text), &q).is_some()
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// `config.show`: 設定の値と由来の一覧を開く。
    fn open_config_show(&mut self) -> CommandResult {
        self.set_overlay(Some(Overlay::Palette(PaletteState {
            settings: true,
            ..PaletteState::default()
        })));
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// `ui.confirm`: 選んだ行の由来がファイルの行なら、一覧を閉じてそのファイルのその行を開く (`config.show_diagnostics` の
    /// `Enter` と同じ)。由来にファイルが無い行 (既定・`--set`) は何もしない (一覧を開いたまま)。
    pub(crate) fn config_show_confirm(&mut self) -> CommandResult {
        let Some(Overlay::Palette(p)) = &self.ui.overlay else {
            return CommandResult::Done;
        };
        let hits = self.config_show_hits(&p.text);
        let Some(&i) = hits.get(p.selected.min(hits.len().saturating_sub(1))) else {
            return CommandResult::Done;
        };
        let rows = self.config_show_rows();
        let (source, line, column) = match &rows[i].origin {
            ValueOrigin::Config(Origin::User {
                source,
                line,
                column,
                ..
            })
            | ValueOrigin::Language {
                at:
                    LangOrigin::File {
                        source,
                        line,
                        column,
                    },
                ..
            } => (source, line, column),
            _ => return CommandResult::Done,
        };
        let Some(path) = self.config_diagnostic_path(source) else {
            return CommandResult::Done;
        };
        let at = |n: u32| usize::try_from(n).unwrap_or(usize::MAX).max(1);
        let (line, column) = (at(*line), at(*column));
        self.set_overlay(None);
        self.mark_dirty_render();
        self.open_at(&path, line, column)
    }

    /// 一覧の表示 (パレットと同じ `OverlayModel::List`)。行: `キー = 値`、右に由来。見出しにフォーカスの文書の名前と言語。
    pub(crate) fn config_show_model(&self, p: &PaletteState) -> OverlayModel {
        let render = |key: &str, args: &[(&str, &str)]| {
            self.catalog().render(key, args, &|c| self.key_label(c))
        };
        let rows = self.config_show_rows();
        let hits = self.config_show_hits(&p.text);
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
            .map(|&i| ListItem {
                label: cell_text(&rows[i].label, CutPolicy::End),
                detail: Some(cell_text(&rows[i].origin_text, CutPolicy::Middle)),
                match_ranges: Vec::new(),
                keys: Vec::new(),
                disabled_reason: None,
            })
            .collect();
        let name = self
            .active_document()
            .and_then(|d| crate::ui_model::pane_bar::display_names(self).remove(&d))
            .unwrap_or_default();
        // 文書の言語 (決めていない読込中の文書・文書が無いときは `plain`)
        let language = self
            .active_document()
            .and_then(|d| self.docs.documents.get(&d))
            .map(EditorState::language_id)
            .filter(|l| !l.is_empty())
            .unwrap_or(eve_config::PLAIN);
        let cells = cell_text(&p.text, CutPolicy::End).spans;
        let cursor = cell_text(&p.text[..p.cursor], CutPolicy::End).spans.len();
        let selection = crate::ui_model::overlay::field_selection(p.all_selected, cells.len());
        let placeholder = p.text.is_empty().then(|| {
            cell_text(
                &render(keys::UI_CONFIG_SHOW_PLACEHOLDER, &[]),
                CutPolicy::End,
            )
        });
        let footer = render(
            keys::UI_PALETTE_FOOTER,
            &[
                ("matched", &hits.len().to_string()),
                ("total", &rows.len().to_string()),
            ],
        );
        OverlayModel::List {
            title: cell_text(
                &render(
                    keys::UI_CONFIG_SHOW_TITLE,
                    &[
                        ("name", &escape_untrusted(&name)),
                        ("language", &sanitize_untrusted(language)),
                    ],
                ),
                CutPolicy::Middle,
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
            no_match: cell_text(&render(keys::UI_CONFIG_SHOW_NO_MATCH, &[]), CutPolicy::End),
        }
    }
}

fn config_show(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.open_config_show()
}

/// `config.show` (07 §11.6)。
pub(crate) fn register(r: &mut CommandRegistry) {
    r.add(
        ids::CONFIG_SHOW,
        CommandKind::Ui,
        Permission::Ui,
        "",
        config_show,
    );
}
