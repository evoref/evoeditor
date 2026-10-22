//! コマンドパレット (docs/04 §8.1、07 §11.4、01 FR-602・FR-511・FR-514)。
//! 入力欄はプロンプトと共通の編集で、候補は `hidden` でない全コマンド。照合は core が行う。
//! v0.1: 前方一致 → 部分一致 → CommandId 順。照合の対象は日本語の title と CommandId。

use std::ops::Range;

use eve_text::grapheme_indices;
use eve_view::{CutPolicy, InputFieldModel, KeyLabel, ListItem, OverlayModel};

use super::command::{Args, CommandResult, RejectReason};
use super::context::{ContextExpr, CtxValue};
use super::layout::cell_text;
use super::overlay::Overlay;
use super::state::EditorState;

/// 候補の窓の最大行数 (04 §8.1)。
const WINDOW: usize = 12;

/// パレットに出さないコマンド (オーバーレイ内専用と内部用、07 §11.6)。
fn hidden(id: &str) -> bool {
    id.starts_with("internal.")
        || matches!(
            id,
            "ui.confirm"
                | "ui.list_next"
                | "ui.list_prev"
                | "ui.list_page_down"
                | "ui.list_page_up"
                | "ui.dialog_choose"
        )
}

/// 既定値の無い必須の列挙引数を持つコマンド (07 §11.2)。パレットで選ぶと、同じオーバーレイで値の一覧を尋ねる。
/// 省略可能な引数 (`path?` など) は尋ねず、各コマンドの既定の動作 (プロンプトを開く等) に任せる。
#[derive(Debug)]
pub(super) struct ArgDef {
    command: &'static str,
    /// 引数名。
    arg: &'static str,
    /// 見出しに入れる引数の名前の MessageKey (`ui.palette.arg.*`)。
    label_key: &'static str,
    /// (値, 表示)。
    choices: &'static [(&'static str, &'static str)],
}

const ENCODINGS: &[(&str, &str)] = &[
    ("utf-8", "UTF-8"),
    ("utf-16le", "UTF-16LE"),
    ("utf-16be", "UTF-16BE"),
    ("shift_jis", "Shift_JIS"),
    ("euc-jp", "EUC-JP"),
    ("iso-2022-jp", "ISO-2022-JP"),
];

const ARG_DEFS: &[ArgDef] = &[
    ArgDef {
        command: "file.set_line_ending",
        arg: "eol",
        label_key: "ui.palette.arg.eol",
        choices: &[("lf", "LF"), ("crlf", "CRLF"), ("cr", "CR")],
    },
    ArgDef {
        command: "file.reopen_with_encoding",
        arg: "encoding",
        label_key: "ui.palette.arg.encoding",
        choices: ENCODINGS,
    },
    ArgDef {
        command: "file.set_encoding",
        arg: "encoding",
        label_key: "ui.palette.arg.encoding",
        choices: ENCODINGS,
    },
];

fn arg_def(command: &str) -> Option<&'static ArgDef> {
    ARG_DEFS.iter().find(|d| d.command == command)
}

/// パレットの候補 1 つ (登録済みコマンドの ID と実行条件)。
#[derive(Clone, Debug)]
pub struct PaletteEntry {
    id: String,
    when: ContextExpr,
}

impl PaletteEntry {
    /// `hidden` のコマンドは `None`。
    pub fn new(id: &str, when: &ContextExpr) -> Option<PaletteEntry> {
        (!hidden(id)).then(|| PaletteEntry {
            id: id.to_owned(),
            when: when.clone(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct PaletteState {
    pub text: String,
    /// `text` 内のカーソル (バイト。書記素境界)。
    pub cursor: usize,
    /// 一致した候補の中での選択位置。
    pub selected: usize,
    /// 引数を尋ねている最中のコマンド (`Esc` で 1 段戻る)。
    pub stage: Option<&'static ArgDef>,
}

struct Hit {
    entry: usize,
    ranges: Vec<Range<usize>>,
}

fn lower_graphemes(s: &str) -> Vec<String> {
    grapheme_indices(s)
        .into_iter()
        .map(|(_, g)| g.to_lowercase())
        .collect()
}

/// `hay` の中で `needle` が最初に現れる書記素番号。
fn find(hay: &[String], needle: &[String]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

impl EditorState {
    pub fn set_palette_entries(&mut self, entries: Vec<PaletteEntry>) {
        self.palette_entries = entries;
    }

    /// 引数の選択肢のうち入力に合うもの (値または表示の部分一致、大小を区別しない)。
    fn arg_hits(&self, def: &'static ArgDef, query: &str) -> Vec<Hit> {
        let q = lower_graphemes(query.trim());
        def.choices
            .iter()
            .enumerate()
            .filter_map(|(i, (value, label))| {
                if q.is_empty() {
                    return Some(Hit {
                        entry: i,
                        ranges: Vec::new(),
                    });
                }
                let shown = find(&lower_graphemes(label), &q);
                let raw = find(&lower_graphemes(value), &q);
                (shown.is_some() || raw.is_some()).then(|| Hit {
                    entry: i,
                    ranges: shown.map(|s| s..s + q.len()).into_iter().collect(),
                })
            })
            .collect()
    }

    /// 引数を尋ねている途中なら 1 段戻って `true` (コマンドの一覧に戻る)。
    pub(super) fn palette_back(&mut self) -> bool {
        if let Some(Overlay::Palette(p)) = &mut self.overlay
            && p.stage.is_some()
        {
            *p = PaletteState::default();
            self.mark_dirty_render();
            return true;
        }
        false
    }

    fn command_title(&self, id: &str) -> String {
        self.catalog()
            .get(&format!("cmd.{id}.title"))
            .unwrap_or(id)
            .to_owned()
    }

    /// 一致した候補。空の入力は全件を CommandId 順で返す。
    fn palette_hits(&self, query: &str) -> Vec<Hit> {
        let q = lower_graphemes(query.trim());
        let mut hits: Vec<(u8, Hit)> = Vec::new();
        for (i, e) in self.palette_entries.iter().enumerate() {
            if q.is_empty() {
                hits.push((
                    0,
                    Hit {
                        entry: i,
                        ranges: Vec::new(),
                    },
                ));
                continue;
            }
            let title = lower_graphemes(&self.command_title(&e.id));
            let (t, d) = (find(&title, &q), find(&lower_graphemes(&e.id), &q));
            if t.is_none() && d.is_none() {
                continue;
            }
            let rank = u8::from(t != Some(0) && d != Some(0));
            let ranges = t.map(|s| s..s + q.len()).into_iter().collect();
            hits.push((rank, Hit { entry: i, ranges }));
        }
        // entries は CommandId 順。安定ソートなので同順位は CommandId 順のまま
        hits.sort_by_key(|(r, _)| *r);
        hits.into_iter().map(|(_, h)| h).collect()
    }

    /// `when` が偽の理由の文言キーの末尾 (`ui.palette.reason.*`)。実行できるなら `None`。
    /// パレットが出ている間は `overlay.active` が `palette` なので、出ていない状態で評価する。
    fn disabled_reason_key(&self, e: &PaletteEntry) -> Option<&'static str> {
        let lookup = |n: &str| {
            if n == "overlay.active" {
                Some(CtxValue::Str(String::new()))
            } else {
                self.context_value(n)
            }
        };
        if e.when.eval(&lookup) {
            return None;
        }
        let key = e
            .when
            .vars()
            .iter()
            .find_map(|v| match v.as_str() {
                "doc.read_only" => Some("read_only"),
                "doc.has_path" => Some("no_path"),
                "doc.dirty" => Some("no_changes"),
                "selection.empty" => Some("no_selection"),
                _ => None,
            })
            .unwrap_or("unavailable");
        Some(key)
    }

    pub(super) fn open_palette(&mut self) -> CommandResult {
        self.set_overlay(Some(Overlay::Palette(PaletteState::default())));
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// 入力が変わったら先頭の候補を選ぶ。
    pub(super) fn palette_reset_selection(&mut self) {
        if let Some(Overlay::Palette(p)) = &mut self.overlay {
            p.selected = 0;
        }
    }

    /// `ui.list_next` / `ui.list_prev` (折り返す)。
    pub(super) fn palette_move(&mut self, forward: bool) -> CommandResult {
        let Some(Overlay::Palette(p)) = &self.overlay else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        let n = match p.stage {
            Some(def) => self.arg_hits(def, &p.text).len(),
            None => self.palette_hits(&p.text).len(),
        };
        if let Some(Overlay::Palette(p)) = &mut self.overlay
            && n > 0
        {
            p.selected = if forward {
                (p.selected + 1) % n
            } else {
                (p.selected + n - 1) % n
            };
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// `ui.confirm`: 選択中のコマンドを閉じてから実行する (実行は Editor が行う)。
    /// 候補が無い・実行できないコマンドのときは、パレットを閉じない。
    pub(super) fn palette_confirm(&mut self) -> CommandResult {
        let Some(Overlay::Palette(p)) = &self.overlay else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        if let Some(def) = p.stage {
            // 引数の値を選んだ: パレットを閉じて、その値でコマンドを実行する
            let hits = self.arg_hits(def, &p.text);
            let Some(hit) = hits.get(p.selected.min(hits.len().saturating_sub(1))) else {
                return CommandResult::Done;
            };
            let value = def.choices[hit.entry].0;
            self.set_overlay(None);
            self.deferred_command =
                Some((def.command.to_owned(), Args::new().with_str(def.arg, value)));
            self.mark_dirty_render();
            return CommandResult::Done;
        }
        let hits = self.palette_hits(&p.text);
        let Some(hit) = hits.get(p.selected.min(hits.len().saturating_sub(1))) else {
            return CommandResult::Done;
        };
        let entry = &self.palette_entries[hit.entry];
        if self.disabled_reason_key(entry).is_some() {
            return CommandResult::Done;
        }
        let id = entry.id.clone();
        // 必須の列挙引数があるコマンドは、同じオーバーレイで値を尋ねる
        if let Some(def) = arg_def(&id) {
            self.set_overlay(Some(Overlay::Palette(PaletteState {
                stage: Some(def),
                ..PaletteState::default()
            })));
            self.mark_dirty_render();
            return CommandResult::Done;
        }
        self.set_overlay(None);
        self.deferred_command = Some((id, Args::new()));
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// 次に実行するコマンド (パレットで選んだもの)。
    pub fn take_deferred_command(&mut self) -> Option<(String, Args)> {
        self.deferred_command.take()
    }

    /// 引数の値の一覧 (`ui.palette.arg_title`)。
    fn arg_model(&self, p: &PaletteState, def: &'static ArgDef) -> OverlayModel {
        let hits = self.arg_hits(def, &p.text);
        let selected = p.selected.min(hits.len().saturating_sub(1));
        let render = |key: &str, args: &[(&str, &str)]| {
            self.catalog().render(key, args, &|c| self.key_label(c))
        };
        let first = if selected < WINDOW {
            0
        } else {
            selected + 1 - WINDOW
        };
        let items: Vec<ListItem> = hits
            .iter()
            .skip(first)
            .take(WINDOW)
            .map(|h| ListItem {
                label: cell_text(def.choices[h.entry].1, CutPolicy::End),
                detail: None,
                match_ranges: h.ranges.clone(),
                keys: Vec::new(),
                disabled_reason: None,
            })
            .collect();
        let cells = cell_text(&p.text, CutPolicy::End).spans;
        let cursor = cell_text(&p.text[..p.cursor], CutPolicy::End).spans.len();
        let title = render(
            "ui.palette.arg_title",
            &[
                ("command", &self.command_title(def.command)),
                ("arg", &render(def.label_key, &[])),
            ],
        );
        let footer = render(
            "ui.palette.footer",
            &[
                ("matched", &hits.len().to_string()),
                ("total", &def.choices.len().to_string()),
            ],
        );
        OverlayModel::List {
            title: cell_text(&title, CutPolicy::Never),
            input: InputFieldModel {
                len: cells.len(),
                cells,
                first: 0,
                cursor,
                selection: None,
                placeholder: None,
            },
            items,
            first,
            selected,
            footer: cell_text(&footer, CutPolicy::Never),
            no_match: cell_text(&render("ui.palette.no_match", &[]), CutPolicy::End),
        }
    }

    pub(super) fn palette_model(&self, p: &PaletteState) -> OverlayModel {
        if let Some(def) = p.stage {
            return self.arg_model(p, def);
        }
        let hits = self.palette_hits(&p.text);
        let selected = p.selected.min(hits.len().saturating_sub(1));
        let first = if selected < WINDOW {
            0
        } else {
            selected + 1 - WINDOW
        };
        let render = |key: &str, args: &[(&str, &str)]| {
            self.catalog().render(key, args, &|c| self.key_label(c))
        };
        let items: Vec<ListItem> = hits
            .iter()
            .skip(first)
            .take(WINDOW)
            .map(|h| {
                let e = &self.palette_entries[h.entry];
                let disabled_reason = self.disabled_reason_key(e).map(|k| {
                    let why = render(&format!("ui.palette.reason.{k}"), &[]);
                    let text = render("ui.palette.disabled", &[("reason", &why)]);
                    cell_text(&text, CutPolicy::End)
                });
                ListItem {
                    label: cell_text(&self.command_title(&e.id), CutPolicy::End),
                    detail: Some(cell_text(&e.id, CutPolicy::End)),
                    match_ranges: h.ranges.clone(),
                    keys: self
                        .key_sequence(&e.id)
                        .map(|strokes| vec![KeyLabel { strokes }])
                        .unwrap_or_default(),
                    disabled_reason,
                }
            })
            .collect();
        let cells = cell_text(&p.text, CutPolicy::End).spans;
        let cursor = cell_text(&p.text[..p.cursor], CutPolicy::End).spans.len();
        let placeholder = p
            .text
            .is_empty()
            .then(|| cell_text(&render("ui.palette.placeholder", &[]), CutPolicy::End));
        let matched = hits.len().to_string();
        let total = self.palette_entries.len().to_string();
        let footer = render(
            "ui.palette.footer",
            &[("matched", &matched), ("total", &total)],
        );
        OverlayModel::List {
            title: cell_text(&render("ui.palette.title", &[]), CutPolicy::Never),
            input: InputFieldModel {
                len: cells.len(),
                cells,
                first: 0,
                cursor,
                selection: None,
                placeholder,
            },
            items,
            first,
            selected,
            footer: cell_text(&footer, CutPolicy::Never),
            no_match: cell_text(&render("ui.palette.no_match", &[]), CutPolicy::End),
        }
    }
}
