//! ヘルプ画面 (docs/04 §8.6、07 §11.5、01 FR-514)。
//!
//! 全画面のオーバーレイで、節を縦に並べた 1 本のスクロール: 「はじめに」「キー一覧」「コマンド一覧」「検索の書き方」
//! (v0.4 の M24。03 §12.3.1)「端末の設定」。
//! キー一覧とコマンド一覧は、解決済みのキーマップとコマンドの一覧から作る。`help.open` は先頭から、
//! `help.keys` / `help.commands` は該当の節へスクロールして開く。操作は `↑` `↓` `PgUp` `PgDn`、閉じるのは `Esc`。

use std::collections::BTreeMap;

use eve_view::{CutPolicy, HelpLine, HelpLineKind, KeyStroke, OverlayModel};

use crate::command::ids;
use crate::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, Permission, RejectReason,
};
use crate::editor::state::EditorState;
use crate::layout::cell_text;
use crate::ui_model::keys;
use crate::ui_model::overlay::Overlay;

/// ヘルプのキー一覧で、この端末で届かない割当の後ろに付ける印 (04 §8.6)。
const UNREACHABLE_MARK: &str = "*";

/// ヘルプのキー一覧で、ユーザーのチョードが先頭に使うため使えない割当 (プレフィックスの衝突、07 §7.6 / §7.7) の後ろに
/// 付ける印 (04 §8.6)。ASCII だけの端末でも描ける字にする。
const DISABLED_MARK: &str = "!";

/// ヘルプのキー一覧で、ワークスペース層の割当の後ろに添える由来 (07 §7.4 の WKEY-3)。層の名前のまま訳さない
/// (`--dump-keymap` の層の名前と同じ語)。
const WORKSPACE_ORIGIN: &str = " (workspace)";

/// 「検索の書き方」の節の表 (書き方, 説明の文言キー)。書き方は正規表現の構文で、言語に依らないのでここに書く
/// (04 §8.6、03 §12.3.1 の表)。行をまたぐ書き方・またがない書き方・改行の続き・文書の端の順。
const SEARCH_SYNTAX: [(&str, &str); 6] = [
    (r"\n", keys::UI_HELP_SEARCH_NEWLINE),
    (r"[\s\n]*", keys::UI_HELP_SEARCH_CLASS_NEWLINE),
    (r"(?s).  [\s\S]", keys::UI_HELP_SEARCH_DOT_ALL),
    (r"\s  [^x]  .", keys::UI_HELP_SEARCH_SINGLE_LINE),
    (r"(?:\r\n)+  \n+", keys::UI_HELP_SEARCH_NEWLINE_RUN),
    (r"\A  \z", keys::UI_HELP_SEARCH_ANCHORS),
];

/// ヘルプの節 (表示順)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HelpSection {
    Intro,
    Keys,
    Commands,
    Search,
    Terminal,
}

/// 開いているヘルプ画面。
#[derive(Clone, Debug)]
pub(crate) struct HelpState {
    /// 先頭に描く行の番号。
    pub scroll: usize,
    /// メッセージ履歴 (`ui.message_history`、04 §8.9) を表示している。同じ画面を履歴の行で使う。
    pub history: bool,
}

impl EditorState {
    /// ヘルプの行を節ごとに作る。戻り値の 2 つ目は各節の見出しの行番号。
    fn help_lines(&self) -> (Vec<HelpLine>, BTreeMap<u8, usize>) {
        if matches!(&self.ui.overlay, Some(Overlay::Help(h)) if h.history) {
            return (self.history_lines(), BTreeMap::new());
        }
        let cat = self.catalog();
        let render = |key: &str| cat.render(key, &[], &|c| self.key_label(c));
        let line = |kind: HelpLineKind, key: Option<String>, text: String| HelpLine {
            kind,
            key: key.map(|k| cell_text(&k, CutPolicy::End)),
            text: cell_text(&text, CutPolicy::End),
        };
        let mut lines: Vec<HelpLine> = Vec::new();
        let mut starts: BTreeMap<u8, usize> = BTreeMap::new();
        let mut section = |lines: &mut Vec<HelpLine>, s: HelpSection, key: &str| {
            starts.insert(s as u8, lines.len());
            lines.push(line(HelpLineKind::Heading, None, render(key)));
        };

        section(&mut lines, HelpSection::Intro, keys::UI_HELP_SECTION_INTRO);
        for k in [
            keys::UI_HELP_INTRO_SAVE,
            keys::UI_HELP_INTRO_QUIT,
            keys::UI_HELP_INTRO_PALETTE,
        ] {
            lines.push(line(HelpLineKind::Body, None, render(k)));
        }
        lines.push(line(HelpLineKind::Body, None, String::new()));

        // キー一覧: 割当のあるコマンドをカテゴリ別に。同じコマンドの割当はすべて並べる
        section(&mut lines, HelpSection::Keys, keys::UI_HELP_SECTION_KEYS);
        lines.push(line(
            HelpLineKind::Body,
            None,
            render(keys::UI_HELP_KEYS_NOTE),
        ));
        let note_at = lines.len();
        let (mut any_unreachable, mut any_disabled) = (false, false);
        let mut last_ns = "";
        for id in self.palette_command_ids() {
            let Some(seqs) = self.keys.key_all.get(id).filter(|s| !s.is_empty()) else {
                continue;
            };
            let ns = id.split('.').next().unwrap_or(id);
            if ns != last_ns {
                last_ns = ns;
                lines.push(line(
                    HelpLineKind::Subheading,
                    None,
                    self.category_label(ns),
                ));
            }
            // この端末で届かない割当と、ユーザーのチョードに負けて使えない割当には印を付ける (04 §8.6 / §7.8、07 §7.7)
            let keys = seqs
                .iter()
                .map(|(s, state, origin)| {
                    let k = eve_view::key_notation(s, self.term.caps.ascii_only);
                    let k = match state {
                        KeyState::Reachable => k,
                        KeyState::Unreachable => {
                            any_unreachable = true;
                            format!("{k}{UNREACHABLE_MARK}")
                        }
                        KeyState::Disabled => {
                            any_disabled = true;
                            format!("{k}{DISABLED_MARK}")
                        }
                    };
                    match origin {
                        KeyOrigin::Workspace => format!("{k}{WORKSPACE_ORIGIN}"),
                        KeyOrigin::Other => k,
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(line(
                HelpLineKind::Entry,
                Some(keys),
                self.command_title_of(id),
            ));
        }
        let notes = [
            (any_unreachable, keys::UI_HELP_KEYS_UNREACHABLE),
            (any_disabled, keys::UI_HELP_KEYS_DISABLED),
        ];
        for (i, (_, key)) in notes.iter().filter(|(on, _)| *on).enumerate() {
            lines.insert(note_at + i, line(HelpLineKind::Body, None, render(key)));
        }
        lines.push(line(HelpLineKind::Body, None, String::new()));

        // コマンド一覧: 全コマンドのタイトル・説明・CommandId
        section(
            &mut lines,
            HelpSection::Commands,
            keys::UI_HELP_SECTION_COMMANDS,
        );
        let mut last_ns = "";
        for id in self.palette_command_ids() {
            let ns = id.split('.').next().unwrap_or(id);
            if ns != last_ns {
                last_ns = ns;
                lines.push(line(
                    HelpLineKind::Subheading,
                    None,
                    self.category_label(ns),
                ));
            }
            let title = self.command_title_of(id);
            let desc = cat.get(&format!("cmd.{id}.description")).unwrap_or("");
            lines.push(line(
                HelpLineKind::Entry,
                Some(id.to_owned()),
                if desc.is_empty() {
                    title
                } else {
                    format!("{title} — {desc}")
                },
            ));
        }
        lines.push(line(HelpLineKind::Body, None, String::new()));

        // 検索の書き方 (v0.4 の M24、04 §8.6): 行をまたぐ書き方とまたがない書き方の表、他の道具との対応、上限、
        // CP932 互換文字の同一視の組と `¥` / `‾` を除く注意 (03 §12.4.1)
        section(
            &mut lines,
            HelpSection::Search,
            keys::UI_HELP_SECTION_SEARCH,
        );
        lines.push(line(
            HelpLineKind::Body,
            None,
            render(keys::UI_HELP_SEARCH_INTRO),
        ));
        for (syntax, k) in SEARCH_SYNTAX {
            lines.push(line(
                HelpLineKind::Entry,
                Some(syntax.to_owned()),
                render(k),
            ));
        }
        for k in [
            keys::UI_HELP_SEARCH_OTHER_TOOLS,
            keys::UI_HELP_SEARCH_LIMIT,
            keys::UI_HELP_SEARCH_SUGAR,
        ] {
            lines.push(line(HelpLineKind::Body, None, render(k)));
        }
        // CP932 互換文字の同一視 (03 §12.4.1、04 §8.6)。80 桁の端末で切れないよう、1 行を 78 セル以下の短い文にする
        for k in [
            keys::UI_HELP_SEARCH_COMPAT_INTRO,
            keys::UI_HELP_SEARCH_COMPAT_PAIRS,
            keys::UI_HELP_SEARCH_COMPAT_REGEX,
            keys::UI_HELP_SEARCH_COMPAT_REPLACE,
            keys::UI_HELP_SEARCH_COMPAT_OFF,
            keys::UI_HELP_SEARCH_COMPAT_MARK,
            keys::UI_HELP_SEARCH_COMPAT_MATCH,
            keys::UI_HELP_SEARCH_COMPAT_EXCLUDE,
            keys::UI_HELP_SEARCH_COMPAT_YEN_KEY,
        ] {
            lines.push(line(HelpLineKind::Body, None, render(k)));
        }
        lines.push(line(HelpLineKind::Body, None, String::new()));

        section(
            &mut lines,
            HelpSection::Terminal,
            keys::UI_HELP_SECTION_TERMINAL,
        );
        for k in [
            keys::UI_HELP_TERMINAL_IME,
            keys::UI_HELP_TERMINAL_CHORD,
            keys::UI_HELP_TERMINAL_ALT,
            // 複数カーソルへの貼り付けの既知の制約 (03 §4.5.3 の MCUR-10。v0.5 の M32)
            keys::UI_HELP_TERMINAL_MULTI_CURSOR_PASTE,
            // `Ctrl+L` を行の選択に割り当てたことによる案内 (07 §11.6 の「編集の拡張」のキーの選び方。v0.5 の M34)
            keys::UI_HELP_TERMINAL_CTRL_SHIFT_L,
            keys::UI_HELP_TERMINAL_REDRAW,
            // 自動閉じと端末の短い貼り付けの既知の制約 (03 §15.7 の APR-8、04 §8.6。v0.5 の M34 の段 4)
            keys::UI_HELP_TERMINAL_AUTO_PAIRS_PASTE,
            // 標準入力 (`cmd | evoeditor -`) の既知の制約 (07 §18.5、04 §8.6。v0.4 の M25)
            keys::UI_HELP_TERMINAL_STDIN_KEYS,
            keys::UI_HELP_TERMINAL_STDIN_REDRAW,
            keys::UI_HELP_TERMINAL_STDIN_POWERSHELL,
            keys::UI_HELP_TERMINAL_STDIN_WRITER,
            keys::UI_HELP_TERMINAL_README,
        ] {
            lines.push(line(HelpLineKind::Body, None, render(k)));
        }
        (lines, starts)
    }

    /// メッセージ履歴の行 (古い順)。時刻は起動からの経過 `mm:ss` (core は壁時計を持たない)。
    fn history_lines(&self) -> Vec<HelpLine> {
        let cat = self.catalog();
        if self.ui.message_log.is_empty() {
            return vec![HelpLine {
                kind: HelpLineKind::Body,
                key: None,
                text: cell_text(
                    &cat.render(keys::UI_HISTORY_EMPTY, &[], &|_| None),
                    CutPolicy::End,
                ),
            }];
        }
        self.ui
            .message_log
            .iter()
            .map(|m| {
                let secs = m.at.0 / 1000;
                let level = cat.render(
                    match m.level {
                        eve_view::MessageLevel::Info => keys::UI_HISTORY_LEVEL_INFO,
                        eve_view::MessageLevel::Warn => keys::UI_HISTORY_LEVEL_WARN,
                        _ => keys::UI_HISTORY_LEVEL_ERROR,
                    },
                    &[],
                    &|_| None,
                );
                let key = format!("{:02}:{:02} {level}", secs / 60, secs % 60);
                let text = match &m.hint {
                    Some(h) => format!("{} {h}", m.text),
                    None => m.text.clone(),
                };
                HelpLine {
                    kind: HelpLineKind::Entry,
                    key: Some(cell_text(&key, CutPolicy::End)),
                    text: cell_text(&text, CutPolicy::End),
                }
            })
            .collect()
    }

    /// パレットに出るコマンドの ID (`CommandId` 順。この frontend で使えないものを除く)。
    fn palette_command_ids(&self) -> Vec<&str> {
        self.ui
            .palette_entries
            .iter()
            .filter(|e| self.supported_here(e))
            .map(|e| e.id())
            .collect()
    }

    fn category_label(&self, ns: &str) -> String {
        self.catalog()
            .get(&format!("ui.palette.category.{ns}"))
            .map_or_else(|| ns.to_owned(), str::to_owned)
    }

    fn command_title_of(&self, id: &str) -> String {
        self.catalog()
            .get(&format!("cmd.{id}.title"))
            .unwrap_or(id)
            .to_owned()
    }

    /// 本文を描ける行数 (見出しの行と操作の案内の行を除く)。
    fn help_view_rows(&self) -> usize {
        usize::from(self.term.surface.1).saturating_sub(2).max(1)
    }

    /// 開く: `section` の見出しが先頭に来るようにスクロールする。
    fn open_help(&mut self, section: HelpSection) -> CommandResult {
        self.set_overlay(Some(Overlay::Help(HelpState {
            scroll: 0,
            history: false,
        })));
        let (lines, starts) = self.help_lines();
        let max = lines.len().saturating_sub(self.help_view_rows());
        let scroll = starts.get(&(section as u8)).copied().unwrap_or(0).min(max);
        if let Some(Overlay::Help(h)) = &mut self.ui.overlay {
            h.scroll = scroll;
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// メッセージ履歴を開く: 最新が見えるよう末尾へスクロールする (04 §8.9)。
    fn open_message_history(&mut self) -> CommandResult {
        self.set_overlay(Some(Overlay::Help(HelpState {
            scroll: 0,
            history: true,
        })));
        let max = self
            .help_lines()
            .0
            .len()
            .saturating_sub(self.help_view_rows());
        if let Some(Overlay::Help(h)) = &mut self.ui.overlay {
            h.scroll = max;
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// メッセージ履歴の全件を、画面と同じ「時刻 レベル 本文」の行にしてクリップボードへ写す (04 §8.9、FR-505)。
    /// 履歴を開いていないときは使えない。
    fn copy_message_history(&mut self) -> CommandResult {
        if !matches!(&self.ui.overlay, Some(Overlay::Help(h)) if h.history) {
            return CommandResult::Rejected(crate::command::RejectReason::NotAvailable);
        }
        if self.ui.message_log.is_empty() {
            return CommandResult::Done;
        }
        let text_of = |t: &eve_view::CellText| -> String {
            t.spans.iter().map(|s| s.text.as_str()).collect()
        };
        let mut out = String::new();
        for l in self.history_lines() {
            if let Some(k) = &l.key {
                out.push_str(&text_of(k));
                out.push(' ');
            }
            out.push_str(&text_of(&l.text));
            out.push('\n');
        }
        let count = self.ui.message_log.len();
        self.copy_text(&out);
        self.show_message(
            eve_view::MessageLevel::Info,
            keys::INFO_HISTORY_COPIED,
            &[("count", &count.to_string())],
        );
        CommandResult::Done
    }

    /// `delta` 行スクロールする (端で止まる)。
    pub(crate) fn help_scroll(&mut self, delta: isize) -> CommandResult {
        let (total, view) = (self.help_lines().0.len(), self.help_view_rows());
        let max = total.saturating_sub(view);
        if let Some(Overlay::Help(h)) = &mut self.ui.overlay {
            h.scroll = h.scroll.saturating_add_signed(delta).min(max);
        } else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// 1 画面ぶんスクロールする。
    pub(crate) fn help_page(&mut self, forward: bool) -> CommandResult {
        let page =
            isize::try_from(self.help_view_rows().saturating_sub(1).max(1)).unwrap_or(isize::MAX);
        self.help_scroll(if forward { page } else { -page })
    }

    pub(crate) fn help_model(&self, h: &HelpState) -> OverlayModel {
        let (lines, _) = self.help_lines();
        let max = lines.len().saturating_sub(self.help_view_rows());
        let render = |key: &str| self.catalog().render(key, &[], &|c| self.key_label(c));
        let (title, footer) = if h.history {
            (keys::UI_HISTORY_TITLE, keys::UI_HISTORY_FOOTER)
        } else {
            (keys::UI_HELP_TITLE, keys::UI_HELP_FOOTER)
        };
        OverlayModel::Help {
            title: cell_text(&render(title), CutPolicy::Never),
            lines,
            scroll: h.scroll.min(max),
            footer: cell_text(&render(footer), CutPolicy::End),
        }
    }
}

fn help_open(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.open_help(HelpSection::Intro)
}

fn help_keys(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.open_help(HelpSection::Keys)
}

fn help_commands(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.open_help(HelpSection::Commands)
}

fn message_history(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.open_message_history()
}

fn message_history_copy(ctx: &mut CommandCtx<'_>) -> CommandResult {
    ctx.state.copy_message_history()
}

/// v0.2 のヘルプコマンド (07 §11.6 の `help.open` / `help.keys` / `help.commands`)。
pub(crate) fn register(r: &mut CommandRegistry) {
    let handlers: [(&str, crate::Handler); 5] = [
        (ids::UI_MESSAGE_HISTORY, message_history),
        (ids::UI_MESSAGE_HISTORY_COPY, message_history_copy),
        (ids::HELP_OPEN, help_open),
        (ids::HELP_KEYS, help_keys),
        (ids::HELP_COMMANDS, help_commands),
    ];
    for (id, h) in handlers {
        r.add(id, CommandKind::Ui, Permission::Ui, "", h);
    }
}

/// ヘルプのキー一覧での割当の状態 (04 §8.6)。並びは「使える」側が先 (同じキー列をまとめるときに強い方を採る)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum KeyState {
    /// この端末で届く。
    Reachable,
    /// この端末では届かない (拡張キーが要る・横取りされる、04 §7.8)。
    Unreachable,
    /// ユーザーのチョードが先頭に使うため、使えない (プレフィックスの衝突、07 §7.6 / §7.7)。
    Disabled,
}

/// ヘルプのキー一覧での割当の由来 (07 §7.4 の WKEY-3)。ワークスペース層の割当だけを示す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyOrigin {
    /// ワークスペース層 (`.eve/keymap.toml`)。
    Workspace,
    /// ほかの層 (既定・ユーザー)。
    Other,
}

impl KeyOrigin {
    /// 層の名前 (`Keymap::listed_bindings` の層) から。
    pub(crate) fn of_layer(layer: &str) -> KeyOrigin {
        if layer == crate::keymap::WORKSPACE_LAYER {
            KeyOrigin::Workspace
        } else {
            KeyOrigin::Other
        }
    }
}

/// ヘルプのキー一覧の 1 つの割当 (キー列, 状態, 由来)。
pub(crate) type KeyEntry = (Vec<KeyStroke>, KeyState, KeyOrigin);

/// すべての割当をコマンドごとに集める (ヘルプのキー一覧用)。層の順で、同じ割当は 1 つにまとめる。
/// 同じキー列が複数あれば、使える側の状態を採り (どれかが届けば届く)、どれかがワークスペース層の外にあれば由来を示さない。
pub(crate) fn collect_all_bindings(
    bindings: impl Iterator<Item = (String, Vec<KeyStroke>, KeyState, KeyOrigin)>,
) -> BTreeMap<String, Vec<KeyEntry>> {
    let mut out: BTreeMap<String, Vec<KeyEntry>> = BTreeMap::new();
    for (cmd, seq, state, origin) in bindings {
        let e = out.entry(cmd).or_default();
        match e.iter_mut().find(|(s, _, _)| *s == seq) {
            Some(x) => {
                x.1 = x.1.min(state);
                if origin == KeyOrigin::Other {
                    x.2 = KeyOrigin::Other;
                }
            }
            None => e.push((seq, state, origin)),
        }
    }
    out
}
