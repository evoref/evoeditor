//! ユーザーのキーマップ (`keymap.toml`) を検証して、キーマップのユーザー層にする (docs/07 §7.1・§7.4・§7.6、01 FR-604)。
//!
//! v0.2 は単一ストロークの上書き・追加・解除 (`"none"`) を解決する。2 ストローク以上・`args`・`finder` / `panel.*`
//! コンテキストは「このバージョンでは未対応」の情報診断にして使わない。問題のある割当は、それだけを無効にして診断を返す
//! (ほかの割当は使う)。

use eve_config::{DiagKind, Diagnostic, KeyAction, KeymapFile, KeymapProblem, Severity};
use eve_view::{CommandId, KeyCode, KeyStroke, Modifiers};

use super::Editor;
use super::context::ContextExpr;
use super::keymap::{Layer, NONE_COMMAND, format_sequence, parse_sequence};

/// ユーザーが書けるコンテキスト (07 §7.1)。core が公開する固定集合。
const CONTEXTS: [&str; 8] = [
    "global", "editor", "search", "prompt", "palette", "list", "dialog", "help",
];

/// どの設定でも必ず届く脱出のコマンド (07 §7.6)。
const ESCAPE_COMMANDS: [&str; 3] = ["app.quit", "ui.palette_open", "ui.cancel"];

impl Editor {
    /// `keymap.toml` の内容でユーザー層を置き換える。戻り値は、読込の診断に検証の診断を足したもの。
    pub fn set_user_keymap(&mut self, file: &KeymapFile) -> Vec<Diagnostic> {
        let mut diags = file.diagnostics.clone();
        self.state.user_keymap_entries = Some(file.entries.clone());
        let mut layer = Layer::new("user");
        for e in &file.entries {
            let key = format!("keys.{}.{}", e.context, e.keys);
            let diag = |severity, kind, value: &str| Diagnostic {
                severity,
                kind,
                key: key.clone(),
                value: value.to_owned(),
                line: e.line,
                column: e.column,
            };
            let warn = |p, value: &str| diag(Severity::Warning, DiagKind::Keymap(p), value);
            let unsupported = || diag(Severity::Info, DiagKind::Unsupported, "");

            let Some(&ctx) = CONTEXTS.iter().find(|c| **c == e.context) else {
                diags.push(
                    if e.context == "finder" || e.context.starts_with("panel.") {
                        unsupported()
                    } else {
                        warn(KeymapProblem::UnknownContext, &e.context)
                    },
                );
                continue;
            };
            if e.keys.split_whitespace().count() > 1 {
                diags.push(unsupported());
                continue;
            }
            let Ok(seq) = parse_sequence(&e.keys) else {
                diags.push(warn(KeymapProblem::BadKey, &e.keys));
                continue;
            };
            let [stroke] = seq[..] else {
                diags.push(warn(KeymapProblem::BadKey, &e.keys));
                continue;
            };
            if is_printable(stroke) {
                diags.push(warn(KeymapProblem::PrintableKey, &e.keys));
                continue;
            }
            let (command, when) = match &e.action {
                KeyAction::None => (NONE_COMMAND, ""),
                KeyAction::Command {
                    command,
                    when,
                    has_args,
                } => {
                    if *has_args {
                        diags.push(unsupported());
                        continue;
                    }
                    if self.registry.get(&CommandId::new(command)).is_none() {
                        diags.push(warn(KeymapProblem::UnknownCommand, command));
                        continue;
                    }
                    let when = when.as_deref().unwrap_or("");
                    if ContextExpr::parse(when).is_err() {
                        diags.push(warn(KeymapProblem::BadWhen, when));
                        continue;
                    }
                    (command.as_str(), when)
                }
            };
            if self.keymap.has_chord_starting_with(ctx, stroke) {
                diags.push(warn(KeymapProblem::ChordPrefix, &e.keys));
                continue;
            }
            layer
                .bind(ctx, &e.keys, command, when)
                .expect("検証済みのキーと when");
        }
        self.keymap.set_user_layer(layer.clone());
        self.restore_escape_keys(&mut layer, &mut diags);
        self.refresh_key_labels();
        diags
    }

    /// 終了・コマンドパレット・取消に届かなくなったら、その原因の割当を外す (エラー、07 §7.6)。
    fn restore_escape_keys(&mut self, layer: &mut Layer, diags: &mut Vec<Diagnostic>) {
        for cmd in ESCAPE_COMMANDS {
            if self.keymap.escape_reachable(cmd) {
                continue;
            }
            let defaults = self.keymap.default_sequences(cmd);
            // 脱出キーと同じキーを別の用途にした (または解除した) 割当を、コンテキストを問わず外す
            let dropped = layer.retain(|b| !defaults.iter().any(|(_, seq)| *seq == b.sequence));
            for b in dropped {
                diags.push(Diagnostic {
                    severity: Severity::Error,
                    kind: DiagKind::Keymap(KeymapProblem::EscapeKey),
                    key: format!("keys.{}.{}", b.context, format_sequence(&b.sequence)),
                    value: cmd.to_owned(),
                    line: 0,
                    column: 0,
                });
            }
            self.keymap.set_user_layer(layer.clone());
        }
    }
}

/// 修飾のない (または Shift だけの) 文字キー。`Text` として届くので、割り当てても効かず、文字入力を奪う。
fn is_printable(s: KeyStroke) -> bool {
    matches!(s.key, KeyCode::Char(_)) && (s.mods == Modifiers::NONE || s.mods == Modifiers::SHIFT)
}
