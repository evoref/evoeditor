//! 設定ファイルの診断を Message キューへ積む (docs/07 §4.3、04 §15.2 の `msg.config.*`)。

use eve_config::{DiagKind, Diagnostic, default_display};
use eve_view::MessageLevel;

use super::state::EditorState;

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
                "msg.config.problem",
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
                .log_path
                .clone()
                .unwrap_or_else(|| "ログなし".to_owned());
            self.show_message(
                MessageLevel::Warn,
                "msg.config.problems",
                &[("count", &count), ("detail", &detail), ("log", &log)],
            );
        }
    }

    /// 診断 1 件の種別の文言 (本文, 対処)。
    fn diagnostic_texts(&self, d: &Diagnostic) -> (String, String) {
        diagnostic_texts(self.catalog(), d)
    }
}

/// 診断 1 件の種別の文言 (本文, 対処)。`--check-config` (端末を使わない) も使う。
pub fn diagnostic_texts(cat: &eve_config::Catalog, d: &Diagnostic) -> (String, String) {
    let default = default_display(&d.key);
    let mut args: Vec<(&str, String)> = vec![("item", d.key.clone()), ("value", d.value.clone())];
    let kind = match &d.kind {
        DiagKind::Unsupported => "unsupported",
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
            "type_mismatch"
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
                P::PrintableKey => "keymap_printable_key",
                P::ChordPrefix => "keymap_chord_prefix",
                P::EscapeKey => "keymap_escape_key",
            }
        }
        DiagKind::InvalidValue { allowed } => {
            args.push(("allowed", allowed.join(", ")));
            args.push(("default", default.to_owned()));
            "invalid_value"
        }
    };
    let refs: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let key = |part: &str| format!("msg.config.{kind}.{part}");
    (
        cat.render(&key("text"), &refs, &|_| None),
        cat.render(&key("hint"), &refs, &|_| None),
    )
}
