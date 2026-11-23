//! ワークスペースのキーマップ (`<root>/.eve/keymap.toml` の `[keys.<コンテキスト>]`) を検証して、キーマップの
//! ワークスペース層にする (docs/07 §7.4 の WKEY-1〜WKEY-3、01 FR-705)。
//!
//! ワークスペース層は追加だけ (02 §7.2 K5): 下の層 (ユーザー層・既定層) の割当を上書き・解除・無効にする割当と、文字入力・
//! 取消し・確定のキーで始まる割当、`args` を持つ割当、許可リストの外のコマンドの割当は使わず、割当ごとに情報の診断
//! (`WorkspaceProblem::KeyRejected`) を出す。同じファイルのほかの割当は使う。ほかの検証 (知らないコンテキスト・
//! コマンド・キーの書式・`when`) と診断はユーザー層 (`user_keymap`) と同じ。
//!
//! spec: 07 §7.4 (WKEY-1〜WKEY-3), 07 §7.1, 07 §7.6, 02 §7.2 K3 / K5

use eve_config::{
    DiagKind, Diagnostic, KeyAction, KeyRejectReason, KeymapFile, KeymapProblem, Severity,
    WorkspaceProblem,
};
use eve_view::{CommandId, KeyCode, KeyStroke, Modifiers};

use super::user_keymap::{CONTEXTS, Origin, entry_diagnostic, is_printable, prefix_diagnostics};
use super::{Keymap, Layer, WORKSPACE_LAYER, contexts_meet, parse_sequence};
use crate::command::context::ContextExpr;
use crate::command::{CommandKind, CommandRegistry, Permission};
use crate::editor::Editor;

/// 1 ファイルで読む割当の上限 (07 §7.4 の WKEY-3)。超えた分は読まずに 1 件の警告を出す。
const MAX_WORKSPACE_KEYS: usize = 1_000;

impl Editor {
    /// ワークスペースのキーマップ (`.eve/keymap.toml`) でワークスペース層を置き換える。`None` なら層を外す。
    ///
    /// 戻り値は、読込の診断 (`file.diagnostics`) に検証の診断を書かれた行の順で足したもの (`None` なら空)。キーの表記
    /// (パレット・ヒントバー・ヘルプ) も作り直す。ファイルの写しを持ち、ユーザー層を替えたときに検証し直す
    /// (`set_user_keymap`)。
    pub fn set_workspace_keymap(&mut self, file: Option<&KeymapFile>) -> Vec<Diagnostic> {
        self.state.keys.workspace_keymap_file = file.cloned();
        let diags = match file {
            Some(f) => apply_workspace_keymap(&self.registry, &mut self.keymap, f),
            None => {
                self.keymap.remove_layer(WORKSPACE_LAYER);
                Vec::new()
            }
        };
        self.refresh_key_labels();
        diags
    }
}

/// `file` を今の下の層 (`keymap` のワークスペース層以外) に照らして検証し、`keymap` のワークスペース層を置き換える。
/// 戻り値は、読込の診断に検証の診断を足したもの。検証の診断は書かれた位置 (行・桁) の順に並べる。
pub(super) fn apply_workspace_keymap(
    registry: &CommandRegistry,
    keymap: &mut Keymap,
    file: &KeymapFile,
) -> Vec<Diagnostic> {
    keymap.remove_layer(WORKSPACE_LAYER);
    // 下の層の割当 (コンテキスト, キーの列)。`"none"`・`when` 付き・遮蔽されたものも数える (追加だけ、WKEY-2 の (a)(b))
    let lower: Vec<(&'static str, Vec<KeyStroke>)> = keymap
        .layers
        .iter()
        .flat_map(|l| l.bindings())
        .map(|b| (b.context, b.sequence.clone()))
        .collect();
    let mut diags = file.diagnostics.clone();
    let mut layer = Layer::new(WORKSPACE_LAYER);
    let mut entry_diags: Vec<(usize, Diagnostic)> = Vec::new();
    let mut origins: Vec<Origin> = Vec::new();
    // コンテキストとキーの列ごとに、最後に書いた行 (別の書き方の重複、K4。ユーザー層と同じ)
    let mut written: Vec<(&'static str, Vec<KeyStroke>, usize)> = Vec::new();
    // 割当は表のキーの順に届くので、書かれた位置の順に読む (上限の数え方と診断の並び)
    let at = |i: usize| (file.entries[i].line, file.entries[i].column, i);
    let mut order: Vec<usize> = (0..file.entries.len()).collect();
    order.sort_by_key(|&i| at(i));
    for (n, &i) in order.iter().enumerate() {
        let e = &file.entries[i];
        if n >= MAX_WORKSPACE_KEYS {
            entry_diags.push((
                i,
                entry_diagnostic(
                    file,
                    i,
                    Severity::Warning,
                    DiagKind::Workspace(WorkspaceProblem::TooManyKeys),
                    &MAX_WORKSPACE_KEYS.to_string(),
                ),
            ));
            break;
        }
        let mut push = |severity, kind, value: &str| {
            entry_diags.push((i, entry_diagnostic(file, i, severity, kind, value)));
        };
        let warn = |push: &mut dyn FnMut(Severity, DiagKind, &str), p, value: &str| {
            push(Severity::Warning, DiagKind::Keymap(p), value);
        };
        let reject = |push: &mut dyn FnMut(Severity, DiagKind, &str), r| {
            push(
                Severity::Info,
                DiagKind::Workspace(WorkspaceProblem::KeyRejected(r)),
                "",
            );
        };

        let Some(&ctx) = CONTEXTS.iter().find(|c| **c == e.context) else {
            if e.context.starts_with("panel.") {
                push(Severity::Info, DiagKind::Unsupported, "");
            } else {
                warn(&mut push, KeymapProblem::UnknownContext, &e.context);
            }
            continue;
        };
        // (h) 確認のダイアログの選択 (保存・破棄・上書き) を、知らないキーで確定させない (保守的な規則、07 §7.4 の WKEY-2)
        if ctx == "dialog" {
            reject(&mut push, KeyRejectReason::Dialog);
            continue;
        }
        let Ok(seq) = parse_sequence(&e.keys) else {
            warn(&mut push, KeymapProblem::BadKey, &e.keys);
            continue;
        };
        // (c) 文字入力・取消し・確定のキーで始まる (ユーザー層の `PrintableKey` の代わり)
        if takes_text_or_control(seq[0]) {
            reject(&mut push, KeyRejectReason::Text);
            continue;
        }
        // (e) 解除
        let KeyAction::Command {
            command,
            when,
            args,
        } = &e.action
        else {
            reject(&mut push, KeyRejectReason::None);
            continue;
        };
        let Some(spec) = registry.get(&CommandId::new(command)) else {
            warn(&mut push, KeymapProblem::UnknownCommand, command);
            continue;
        };
        // (f) 外部コマンド
        if runs_external(spec.permission) {
            reject(&mut push, KeyRejectReason::Exec);
            continue;
        }
        // (g) 許可リストの外のコマンド
        if !allowed_from_workspace(command, spec.kind) {
            reject(&mut push, KeyRejectReason::Command);
            continue;
        }
        // (d) 引数 (中身を問わない。引数を持たないコマンドだけに付けられる)
        if args.is_some() {
            reject(&mut push, KeyRejectReason::Args);
            continue;
        }
        let when = when.as_deref().unwrap_or("");
        let Ok(expr) = ContextExpr::parse(when) else {
            warn(&mut push, KeymapProblem::BadWhen, when);
            continue;
        };
        if let Some(v) = expr
            .vars()
            .into_iter()
            .find(|v| !crate::command::context::CONTEXT_VARS.contains(&v.as_str()))
        {
            warn(&mut push, KeymapProblem::UnknownWhenVar, &v);
            continue;
        }
        // (a)(b) 下の層の割当と、同時に参照されるコンテキストで出会う
        let meets = lower
            .iter()
            .filter(|(lc, _)| contexts_meet(ctx, lc))
            .map(|(_, ls)| ls);
        let mut shadows = false;
        let mut bound = false;
        for ls in meets {
            if *ls == seq {
                bound = true;
            } else if ls.starts_with(&seq) || seq.starts_with(ls) {
                shadows = true;
            }
        }
        if bound {
            reject(&mut push, KeyRejectReason::Bound);
            continue;
        }
        if shadows {
            reject(&mut push, KeyRejectReason::Shadows);
            continue;
        }
        // 同じコンテキスト・同じキーの列の別の書き方 (情報、K4): 後の行を使う
        match written.iter_mut().find(|(c, s, _)| *c == ctx && *s == seq) {
            Some(w) if file.entries[w.2].keys != e.keys => {
                let other = &file.entries[w.2];
                let dup = DiagKind::Keymap(KeymapProblem::DuplicateKey);
                if (e.line, e.column) > (other.line, other.column) {
                    layer.retain(|b| !(b.context == ctx && b.sequence == seq));
                    push(Severity::Info, dup, &other.keys);
                    w.2 = i;
                } else {
                    entry_diags.push((
                        w.2,
                        entry_diagnostic(file, w.2, Severity::Info, dup, &e.keys),
                    ));
                    continue;
                }
            }
            Some(w) => w.2 = i,
            None => written.push((ctx, seq.clone(), i)),
        }
        layer
            .bind(ctx, &e.keys, command, when)
            .expect("検証済みのキーと when");
        origins.push((ctx, seq, expr, i));
    }
    keymap.set_workspace_layer(layer);
    debug_assert!(
        ["app.quit", "ui.palette_open", "ui.cancel"]
            .iter()
            .all(|c| keymap.escape_lost_focuses(c).is_empty()),
        "追加だけのワークスペース層は脱出キーを奪わない"
    );
    // ワークスペース層の中のプレフィックスの衝突 (ユーザー層と同じ警告。下の層との衝突は (b) で使わない)
    prefix_diagnostics(keymap, WORKSPACE_LAYER, &origins, file, &mut entry_diags);
    entry_diags.sort_by_key(|(i, _)| at(*i));
    diags.extend(entry_diags.into_iter().map(|(_, d)| d));
    // 読込の診断と検証の診断を合わせて、1 つのファイルで上限まで (07 §4.3)
    eve_config::cap_diagnostics(&mut diags);
    diags
}

/// 先頭のストロークが、文字入力 (修飾なし・`shift` だけの印字可能キー。`space` を含む) か、取消し・確定・字の削除の
/// キー (`enter`・`tab`・`backspace`・`delete`・`esc`) (WKEY-2 の (c))。後者も修飾なしと `shift` だけを数える
/// (`shift+enter` などは端末によって修飾の無いキーとして届く)。
fn takes_text_or_control(s: KeyStroke) -> bool {
    let plain = s.mods == Modifiers::NONE || s.mods == Modifiers::SHIFT;
    is_printable(s)
        || (plain
            && matches!(
                s.key,
                KeyCode::Enter | KeyCode::Tab | KeyCode::Backspace | KeyCode::Delete | KeyCode::Esc
            ))
}

/// 外部コマンドを起こす権限 (`perm` が `exec`、WKEY-2 の (f))。v0.5 の権限には無いので常に偽。権限を足したら、ここで
/// 真にする (網羅の `match` で、足したときに見直しを強いる)。
fn runs_external(p: Permission) -> bool {
    match p {
        Permission::None | Permission::Read | Permission::Propose | Permission::Ui => false,
    }
}

/// ワークスペース層から付けられるコマンド (WKEY-2 の (g)): kind が Motion・Ui・Edit で、`app.*` でないもの。
fn allowed_from_workspace(command: &str, kind: CommandKind) -> bool {
    let kind_ok = match kind {
        CommandKind::Motion | CommandKind::Ui | CommandKind::Edit => true,
        CommandKind::File => false,
    };
    kind_ok && !command.starts_with("app.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::parse_stroke;

    fn ks(s: &str) -> KeyStroke {
        parse_stroke(s).expect("試験のキー")
    }

    #[test]
    fn 文字入力と取消し確定のキーで始まるものを見分ける() {
        for k in [
            "a",
            "A",
            "shift+1",
            "space",
            "shift+space",
            "enter",
            "shift+enter",
            "tab",
            "shift+tab",
            "backspace",
            "delete",
            "esc",
        ] {
            assert!(takes_text_or_control(ks(k)), "{k}");
        }
        for k in [
            "ctrl+a",
            "alt+x",
            "ctrl+space",
            "ctrl+enter",
            "alt+backspace",
            "f6",
            "up",
            "ctrl+alt+esc",
        ] {
            assert!(!takes_text_or_control(ks(k)), "{k}");
        }
    }

    #[test]
    fn 許可リストは_motion_ui_edit_で_app_を除く() {
        assert!(allowed_from_workspace("cursor.left", CommandKind::Motion));
        assert!(allowed_from_workspace("ui.palette_open", CommandKind::Ui));
        assert!(allowed_from_workspace(
            "edit.duplicate_line",
            CommandKind::Edit
        ));
        assert!(!allowed_from_workspace("file.save_all", CommandKind::File));
        assert!(!allowed_from_workspace("app.quit", CommandKind::Ui));
        assert!(!runs_external(Permission::Ui));
    }
}
