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
pub(super) const WINDOW: usize = 12;

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
    pub(super) fn id(&self) -> &str {
        &self.id
    }

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
    /// バッファ一覧 (`buffer.list`、04 §8.5)。パレットと同じリスト部品で、候補が文書になる。
    pub buffers: bool,
}

struct Hit {
    entry: usize,
    ranges: Vec<Range<usize>>,
}

pub(super) fn lower_graphemes(s: &str) -> Vec<String> {
    grapheme_indices(s)
        .into_iter()
        .map(|(_, g)| g.to_lowercase())
        .collect()
}

/// パレットから実行したコマンドを覚える数 (直近 20 件、起動している間だけ。07 §11.4)。
pub(super) const RECENT_MAX: usize = 20;

/// 書記素 `i` が語の先頭か (先頭、または直前が空白・区切り記号)。
fn word_start(hay: &[String], i: usize) -> bool {
    i == 0 || matches!(hay[i - 1].as_str(), " " | "." | "_" | "-" | ":" | "/")
}

/// ファジー一致 (04 §8.1、07 §11.4): `needle` の書記素が `hay` に順に現れれば (スコア, 一致した書記素番号)。
/// 連続した部分一致を最も高くし (先頭ならさらに高く)、部分列の一致は、語頭に当たる文字・連続する文字が多く、
/// 間が少ないほど高い。どれも一致しなければ `None`。
pub(super) fn fuzzy(hay: &[String], needle: &[String]) -> Option<(i32, Vec<usize>)> {
    if needle.is_empty() {
        return None;
    }
    if let Some(s) = find(hay, needle) {
        let bonus = if s == 0 {
            200
        } else if word_start(hay, s) {
            100
        } else {
            0
        };
        return Some((1000 + bonus - s as i32, (s..s + needle.len()).collect()));
    }
    // 部分列: 前から順に当てる。語頭の文字を優先して当て、それで当てきれなければ前から最初の文字に当て直す
    let pos = subsequence(hay, needle, true).or_else(|| subsequence(hay, needle, false))?;
    let mut score = 500 - pos[0] as i32;
    for (k, &i) in pos.iter().enumerate() {
        if word_start(hay, i) {
            score += 20;
        }
        if k > 0 {
            let gap = i - pos[k - 1] - 1;
            score += if gap == 0 { 15 } else { -(gap.min(20) as i32) };
        }
    }
    Some((score, pos))
}

/// `needle` を `hay` の部分列として当てた位置。`prefer_word_start` なら、各文字をその先で最初の語頭に当てる
/// (語頭が無ければ最初の出現)。当てきれなければ `None`。
fn subsequence(hay: &[String], needle: &[String], prefer_word_start: bool) -> Option<Vec<usize>> {
    let mut pos = Vec::with_capacity(needle.len());
    let mut from = 0;
    for g in needle {
        let mut cand = (from..hay.len()).filter(|&i| hay[i] == *g);
        let first = cand.next()?;
        let pick = if prefer_word_start && !word_start(hay, first) {
            cand.find(|&i| word_start(hay, i)).unwrap_or(first)
        } else {
            first
        };
        pos.push(pick);
        from = pick + 1;
    }
    Some(pos)
}

/// 書記素番号の列を、連続する区間にまとめる。
fn to_ranges(pos: &[usize]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for &p in pos {
        match out.last_mut() {
            Some(r) if r.end == p => r.end = p + 1,
            _ => out.push(p..p + 1),
        }
    }
    out
}

/// `hay` の中で `needle` が最初に現れる書記素番号。
pub(super) fn find(hay: &[String], needle: &[String]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

impl EditorState {
    pub fn set_palette_entries(&mut self, entries: Vec<PaletteEntry>) {
        self.palette_entries = entries;
    }

    /// 一覧に出す名前 `カテゴリ: タイトル` (04 §8.1)。カテゴリは CommandId の名前空間。
    fn command_label(&self, id: &str) -> String {
        let title = self.command_title(id);
        let ns = id.split('.').next().unwrap_or(id);
        let key = format!("ui.palette.category.{ns}");
        match self.catalog().get(&key) {
            Some(cat) => format!("{cat}: {title}"),
            None => title,
        }
    }

    /// `command_label` のうちタイトルが始まる書記素番号 (照合した箇所をずらす分)。
    fn label_offset(&self, id: &str) -> usize {
        let ns = id.split('.').next().unwrap_or(id);
        self.catalog()
            .get(&format!("ui.palette.category.{ns}"))
            .map_or(0, |cat| grapheme_indices(cat).len() + 2)
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

    /// 一致した候補 (04 §8.1、07 §11.4)。順序は ファジー一致のスコア → 最近実行 → CommandId 順。
    /// 照合の対象はタイトル (表示の言語) と英語のタイトルと CommandId (そのままの形と単語に分けた形)。
    /// 空の入力は 最近実行 → CommandId 順の全件。
    fn palette_hits(&self, query: &str) -> Vec<Hit> {
        let q = lower_graphemes(query.trim());
        let en = eve_config::Catalog::builtin(eve_config::Locale::En);
        // 最近実行の順位 (新しいほど小さい)。覚えていないものは最後
        let recency = |id: &str| {
            self.palette_recent
                .iter()
                .position(|r| r == id)
                .unwrap_or(usize::MAX)
        };
        let mut hits: Vec<(i32, usize, Hit)> = Vec::new();
        for (i, e) in self.palette_entries.iter().enumerate() {
            if q.is_empty() {
                hits.push((
                    0,
                    recency(&e.id),
                    Hit {
                        entry: i,
                        ranges: Vec::new(),
                    },
                ));
                continue;
            }
            let title = lower_graphemes(&self.command_title(&e.id));
            let en_title = en
                .get(&format!("cmd.{}.title", e.id))
                .map(lower_graphemes)
                .unwrap_or_default();
            let id_raw = lower_graphemes(&e.id);
            let id_words = lower_graphemes(&e.id.replace(['.', '_'], " "));
            let on_title = fuzzy(&title, &q);
            let best = [
                on_title.as_ref().map(|(s, _)| *s),
                fuzzy(&en_title, &q).map(|(s, _)| s),
                fuzzy(&id_raw, &q).map(|(s, _)| s),
                fuzzy(&id_words, &q).map(|(s, _)| s),
            ]
            .into_iter()
            .flatten()
            .max();
            let Some(score) = best else {
                continue;
            };
            // 強調するのは表示しているタイトルで一致した箇所だけ
            let ranges = on_title.map(|(_, p)| to_ranges(&p)).unwrap_or_default();
            hits.push((score, recency(&e.id), Hit { entry: i, ranges }));
        }
        // entries は CommandId 順。安定ソートなので同順位は CommandId 順のまま
        hits.sort_by_key(|(score, recent, _)| (std::cmp::Reverse(*score), *recent));
        hits.into_iter().map(|(_, _, h)| h).collect()
    }

    /// パレットから実行したコマンドを覚える (新しいものを先頭に、直近 `RECENT_MAX` 件)。
    fn remember_palette_command(&mut self, id: &str) {
        self.palette_recent.retain(|r| r != id);
        self.palette_recent.insert(0, id.to_owned());
        self.palette_recent.truncate(RECENT_MAX);
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

    /// `ui.list_page_down` / `ui.list_page_up`: 表示窓の行数ぶん動かす (端で止まる)。
    pub(super) fn palette_page(&mut self, forward: bool) -> CommandResult {
        let Some(Overlay::Palette(p)) = &self.overlay else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        };
        let n = match p.stage {
            _ if p.buffers => self.buffer_hits(&p.text).len(),
            Some(def) => self.arg_hits(def, &p.text).len(),
            None => self.palette_hits(&p.text).len(),
        };
        if let Some(Overlay::Palette(p)) = &mut self.overlay
            && n > 0
        {
            p.selected = if forward {
                (p.selected + WINDOW).min(n - 1)
            } else {
                p.selected.saturating_sub(WINDOW)
            };
        }
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
            _ if p.buffers => self.buffer_hits(&p.text).len(),
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
        if p.buffers {
            return self.buffer_list_confirm();
        }
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
        self.remember_palette_command(&id);
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
        if p.buffers {
            return self.buffer_list_model(p);
        }
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
                    label: cell_text(&self.command_label(&e.id), CutPolicy::End),
                    detail: Some(cell_text(&e.id, CutPolicy::End)),
                    match_ranges: {
                        let off = self.label_offset(&e.id);
                        h.ranges
                            .iter()
                            .map(|r| r.start + off..r.end + off)
                            .collect()
                    },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn g(s: &str) -> Vec<String> {
        lower_graphemes(s)
    }

    #[test]
    fn 部分一致は部分列より高く_先頭の一致が最も高い() {
        let (prefix, _) = fuzzy(&g("save as"), &g("save")).unwrap();
        let (inner, _) = fuzzy(&g("file save"), &g("save")).unwrap();
        let (sub, pos) = fuzzy(&g("select all"), &g("sa")).unwrap();
        assert!(prefix > inner && inner > sub, "{prefix} {inner} {sub}");
        assert_eq!(pos, [0, 7], "語頭に当てる");
    }

    #[test]
    fn 語頭を優先して当てきれなければ前から当て直す() {
        let (_, pos) = fuzzy(&g("xa-b a"), &g("ab")).unwrap();
        assert_eq!(pos, [1, 3]);
        assert!(
            fuzzy(&g("abc"), &g("cb")).is_none(),
            "順序が逆なら一致しない"
        );
        assert!(fuzzy(&g("保存"), &g("存保")).is_none());
        assert!(fuzzy(&g("名前を付けて保存"), &g("名保")).is_some());
    }

    #[test]
    fn 一致した位置は連続する区間にまとめる() {
        assert_eq!(to_ranges(&[0, 1, 2, 5, 7, 8]), [0..3, 5..6, 7..9]);
    }
}
