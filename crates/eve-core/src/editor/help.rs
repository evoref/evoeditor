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
}

impl EditorState {
    /// ヘルプの行を節ごとに作る。戻り値の 2 つ目は各節の見出しの行番号。
    fn help_lines(&self) -> (Vec<HelpLine>, BTreeMap<u8, usize>) {
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
            let keys = seqs
                .iter()
                .map(|s| format_sequence(s))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(line(
                HelpLineKind::Entry,
                Some(keys),
                self.command_title_of(id),
            ));
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
        let (lines, starts) = self.help_lines();
        let max = lines.len().saturating_sub(self.help_view_rows());
        let scroll = starts.get(&(section as u8)).copied().unwrap_or(0).min(max);
        self.set_overlay(Some(Overlay::Help(HelpState { scroll })));
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
        OverlayModel::Help {
            title: cell_text(&render("ui.help.title"), CutPolicy::Never),
            lines,
            scroll: h.scroll.min(max),
            footer: cell_text(&render("ui.help.footer"), CutPolicy::End),
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

/// v0.2 のヘルプコマンド (07 §11.6 の `help.open` / `help.keys` / `help.commands`)。
pub(super) fn register(r: &mut CommandRegistry) {
    let handlers: [(&str, crate::Handler); 3] = [
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
pub(super) fn collect_all_bindings(
    bindings: impl Iterator<Item = (String, Vec<KeyStroke>)>,
) -> BTreeMap<String, Vec<Vec<KeyStroke>>> {
    let mut out: BTreeMap<String, Vec<Vec<KeyStroke>>> = BTreeMap::new();
    for (cmd, seq) in bindings {
        let e = out.entry(cmd).or_default();
        if !e.contains(&seq) {
            e.push(seq);
        }
    }
    out
}
