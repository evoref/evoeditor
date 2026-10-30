//! ヘルプ画面 (docs/04 §8.6、07 §11.5、01 FR-514)。
//!
//! 全画面のオーバーレイで、節を縦に並べた 1 本のスクロール: 「はじめに」「キー一覧」「コマンド一覧」「端末の設定」。
//! キー一覧とコマンド一覧は、解決済みのキーマップとコマンドの一覧から作る。`help.open` は先頭から、
//! `help.keys` / `help.commands` は該当の節へスクロールして開く。操作は `↑` `↓` `PgUp` `PgDn`、閉じるのは `Esc`。

use std::collections::BTreeMap;

use eve_view::{CutPolicy, HelpLine, HelpLineKind, KeyStroke, OverlayModel};

use super::command::{
    CommandCtx, CommandKind, CommandRegistry, CommandResult, CommandSpec, Permission, RejectReason,
};
use super::keymap::format_sequence;
use super::layout::cell_text;
use super::overlay::Overlay;
use super::state::EditorState;

/// ヘルプのキー一覧で、この端末で届かない割当の後ろに付ける印 (04 §8.6)。
const UNREACHABLE_MARK: &str = "*";

/// ヘルプの節 (表示順)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HelpSection {
    Intro,
    Keys,
    Commands,
    Terminal,
}

/// 開いているヘルプ画面。
#[derive(Clone, Debug)]
pub(super) struct HelpState {
    /// 先頭に描く行の番号。
    pub scroll: usize,
    /// メッセージ履歴 (`ui.message_history`、04 §8.9) を表示している。同じ画面を履歴の行で使う。
    pub history: bool,
}

impl EditorState {
    /// ヘルプの行を節ごとに作る。戻り値の 2 つ目は各節の見出しの行番号。
    fn help_lines(&self) -> (Vec<HelpLine>, BTreeMap<u8, usize>) {
        if matches!(&self.overlay, Some(Overlay::Help(h)) if h.history) {
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

        section(&mut lines, HelpSection::Intro, "ui.help.section.intro");
        for k in [
            "ui.help.intro.save",
            "ui.help.intro.quit",
            "ui.help.intro.palette",
        ] {
            lines.push(line(HelpLineKind::Body, None, render(k)));
        }
        lines.push(line(HelpLineKind::Body, None, String::new()));

        // キー一覧: 割当のあるコマンドをカテゴリ別に。同じコマンドの割当はすべて並べる
        section(&mut lines, HelpSection::Keys, "ui.help.section.keys");
        lines.push(line(HelpLineKind::Body, None, render("ui.help.keys.note")));
        let note_at = lines.len();
        let mut any_unreachable = false;
        let mut last_ns = "";
        for id in self.palette_command_ids() {
            let Some(seqs) = self.key_all.get(id).filter(|s| !s.is_empty()) else {
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
            // この端末で届かない割当には印を付ける (04 §8.6 / §7.8)
            let keys = seqs
                .iter()
                .map(|(s, ok)| {
                    let k = format_sequence(s);
                    if *ok {
                        k
                    } else {
                        any_unreachable = true;
                        format!("{k}{UNREACHABLE_MARK}")
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
        if any_unreachable {
            lines.insert(
                note_at,
                line(HelpLineKind::Body, None, render("ui.help.keys.unreachable")),
            );
        }
        lines.push(line(HelpLineKind::Body, None, String::new()));

        // コマンド一覧: 全コマンドのタイトル・説明・CommandId
        section(
            &mut lines,
            HelpSection::Commands,
            "ui.help.section.commands",
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

        section(
            &mut lines,
            HelpSection::Terminal,
            "ui.help.section.terminal",
        );
        for k in [
            "ui.help.terminal.ime",
            "ui.help.terminal.chord",
            "ui.help.terminal.alt",
            "ui.help.terminal.readme",
        ] {
            lines.push(line(HelpLineKind::Body, None, render(k)));
        }
        (lines, starts)
    }

    /// メッセージ履歴の行 (古い順)。時刻は起動からの経過 `mm:ss` (core は壁時計を持たない)。
    fn history_lines(&self) -> Vec<HelpLine> {
        let cat = self.catalog();
        if self.message_log.is_empty() {
            return vec![HelpLine {
                kind: HelpLineKind::Body,
                key: None,
                text: cell_text(
                    &cat.render("ui.history.empty", &[], &|_| None),
                    CutPolicy::End,
                ),
            }];
        }
        self.message_log
            .iter()
            .map(|m| {
                let secs = m.at.0 / 1000;
                let level = cat.render(
                    match m.level {
                        eve_view::MessageLevel::Info => "ui.history.level.info",
                        eve_view::MessageLevel::Warn => "ui.history.level.warn",
                        _ => "ui.history.level.error",
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

    /// パレットに出るコマンドの ID (CommandId 順)。
    fn palette_command_ids(&self) -> Vec<&str> {
        self.palette_entries.iter().map(|e| e.id()).collect()
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
        usize::from(self.surface.1).saturating_sub(2).max(1)
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
        if let Some(Overlay::Help(h)) = &mut self.overlay {
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
        if let Some(Overlay::Help(h)) = &mut self.overlay {
            h.scroll = max;
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// `delta` 行スクロールする (端で止まる)。
    pub(super) fn help_scroll(&mut self, delta: isize) -> CommandResult {
        let (total, view) = (self.help_lines().0.len(), self.help_view_rows());
        let max = total.saturating_sub(view);
        if let Some(Overlay::Help(h)) = &mut self.overlay {
            h.scroll = h.scroll.saturating_add_signed(delta).min(max);
        } else {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        self.mark_dirty_render();
        CommandResult::Done
    }

    /// 1 画面ぶんスクロールする。
    pub(super) fn help_page(&mut self, forward: bool) -> CommandResult {
        let page = self.help_view_rows().saturating_sub(1).max(1) as isize;
        self.help_scroll(if forward { page } else { -page })
    }

    pub(super) fn help_model(&self, h: &HelpState) -> OverlayModel {
        let (lines, _) = self.help_lines();
        let max = lines.len().saturating_sub(self.help_view_rows());
        let render = |key: &str| self.catalog().render(key, &[], &|c| self.key_label(c));
        let (title, footer) = if h.history {
            ("ui.history.title", "ui.history.footer")
        } else {
            ("ui.help.title", "ui.help.footer")
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

/// v0.2 のヘルプコマンド (07 §11.6 の `help.open` / `help.keys` / `help.commands`)。
pub(super) fn register(r: &mut CommandRegistry) {
    let handlers: [(&str, crate::Handler); 4] = [
        ("ui.message_history", message_history),
        ("help.open", help_open),
        ("help.keys", help_keys),
        ("help.commands", help_commands),
    ];
    for (id, h) in handlers {
        r.register(
            CommandSpec::new(id, CommandKind::Ui, Permission::Ui, "", h)
                .expect("カタログの when 句は構文が正しい"),
        );
    }
}

/// すべての割当をコマンドごとに集める (ヘルプのキー一覧用)。層の順で、同じ割当は 1 つにまとめる。
/// 割当をコマンドごとに集める。値は (キー列, この端末で届くか)。同じキー列が複数あれば、どれかが届けば届く。
pub(super) fn collect_all_bindings(
    bindings: impl Iterator<Item = (String, Vec<KeyStroke>, bool)>,
) -> BTreeMap<String, Vec<(Vec<KeyStroke>, bool)>> {
    let mut out: BTreeMap<String, Vec<(Vec<KeyStroke>, bool)>> = BTreeMap::new();
    for (cmd, seq, ok) in bindings {
        let e = out.entry(cmd).or_default();
        match e.iter_mut().find(|(s, _)| *s == seq) {
            Some(x) => x.1 |= ok,
            None => e.push((seq, ok)),
        }
    }
    out
}
