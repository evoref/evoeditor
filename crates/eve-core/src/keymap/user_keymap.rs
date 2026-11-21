//! ユーザーのキーマップ (`keymap.toml`) を検証して、キーマップのユーザー層にする (docs/07 §7.1・§7.4・§7.6、01 FR-604 /
//! FR-608)。
//!
//! 単一ストロークとチョード (ストロークの数に上限は無い) の上書き・追加・解除 (`"none"`)・`when`・`args` を解決する。
//! `finder` / `panel.*` コンテキストは「このバージョンでは未対応」の情報診断にして使わない。問題のある割当は、それだけを
//! 無効にして診断を返す (ほかの割当は使う)。プレフィックスの衝突 (曖昧なチョード) はチョードを優先し、使えなくなった
//! 割当を診断で名指しする (02 §7.2 K3、07 §7.6)。
//!
//! spec: 07 §7.1, 07 §7.4, 07 §7.6, 02 §7.2

use eve_config::{
    DiagKind, Diagnostic, KeyAction, KeyArgValue, KeymapFile, KeymapProblem, Severity,
};
use eve_view::{CommandId, KeyCode, KeyStroke, Modifiers};

use super::{Binding, Keymap, Layer, NONE_COMMAND, format_sequence, parse_sequence};
use crate::command::arg_schema::{ArgType, command_args};
use crate::command::context::ContextExpr;
use crate::command::{ArgValue, Args, CommandRegistry, ids};
use crate::editor::Editor;

/// ユーザーが書けるコンテキスト (07 §7.1)。core が公開する固定集合。並びは一覧の出力 (`--dump-keymap`) の順にも使う。
pub const CONTEXTS: [&str; 8] = [
    "global", "editor", "search", "prompt", "palette", "list", "dialog", "help",
];

/// どの設定でも必ず届く脱出のコマンド (07 §7.6)。
const ESCAPE_COMMANDS: [&str; 3] = [ids::APP_QUIT, ids::UI_PALETTE_OPEN, ids::UI_CANCEL];

impl Editor {
    /// `keymap.toml` の内容でユーザー層を置き換える。戻り値は、読込の診断に検証の診断を足したもの。
    pub fn set_user_keymap(&mut self, file: &KeymapFile) -> Vec<Diagnostic> {
        self.state.keys.user_keymap_entries = Some(file.entries.clone());
        let diags = apply_user_keymap(&self.registry, &mut self.keymap, file);
        self.refresh_key_labels();
        diags
    }
}

/// 組込みの既定キーマップに `keymap.toml` の内容をユーザー層として重ねた、解決後のキーマップと診断。`Editor` を作らずに
/// キーマップを出す経路 (`evoeditor --dump-keymap`、`--check-config`、07 §7.7 / §18.3) が使う。検証は
/// `Editor::set_user_keymap` と同じ。
pub fn resolve_keymap(file: &KeymapFile) -> (Keymap, Vec<Diagnostic>) {
    let registry = crate::editor::builtin_registry();
    let mut keymap = super::default_keymap::default_keymap();
    let diags = apply_user_keymap(&registry, &mut keymap, file);
    (keymap, diags)
}

/// 層に入れた割当 (コンテキスト・キーの列・`when`) と、それを書いた行 (`file.entries` の番号)。
type Origin = (&'static str, Vec<KeyStroke>, ContextExpr, usize);

/// `file` を検証してユーザー層にし、`keymap` のユーザー層を置き換える。戻り値は、読込の診断に検証の診断を足したもの。
/// 検証の診断は書かれた行の順に並べ、その後に脱出キーの診断を置く。
fn apply_user_keymap(
    registry: &CommandRegistry,
    keymap: &mut Keymap,
    file: &KeymapFile,
) -> Vec<Diagnostic> {
    let mut diags = file.diagnostics.clone();
    let mut layer = Layer::new("user");
    // 割当ごとの診断 (割当の番号, 診断)。最後に書かれた行の順に並べ直す
    let mut entry_diags: Vec<(usize, Diagnostic)> = Vec::new();
    // 層に入れた割当と、その元の行 (診断に、書かれた位置とキーを出す)
    let mut origins: Vec<Origin> = Vec::new();
    // コンテキストとキーの列ごとに、最後に書いた行 (別の書き方の重複を見つける、K4)
    let mut written: Vec<(&'static str, Vec<KeyStroke>, usize)> = Vec::new();
    for (i, e) in file.entries.iter().enumerate() {
        let mut push = |severity, kind, value: &str| {
            entry_diags.push((i, entry_diagnostic(file, i, severity, kind, value)));
        };
        let warn = |push: &mut dyn FnMut(Severity, DiagKind, &str), p, value: &str| {
            push(Severity::Warning, DiagKind::Keymap(p), value);
        };

        let Some(&ctx) = CONTEXTS.iter().find(|c| **c == e.context) else {
            if e.context == "finder" || e.context.starts_with("panel.") {
                push(Severity::Info, DiagKind::Unsupported, "");
            } else {
                warn(&mut push, KeymapProblem::UnknownContext, &e.context);
            }
            continue;
        };
        let Ok(seq) = parse_sequence(&e.keys) else {
            warn(&mut push, KeymapProblem::BadKey, &e.keys);
            continue;
        };
        // 1 打目が文字キーだと `Text` として届き、キーマップを通らない (2 打目以降は保留中なので照合される、K1)
        if is_printable(seq[0]) {
            warn(&mut push, KeymapProblem::PrintableKey, &e.keys);
            continue;
        }
        let (command, when, args) = match &e.action {
            KeyAction::None => (NONE_COMMAND, "", Args::new()),
            KeyAction::Command {
                command,
                when,
                args,
            } => {
                if registry.get(&CommandId::new(command)).is_none() {
                    warn(&mut push, KeymapProblem::UnknownCommand, command);
                    continue;
                }
                let when = when.as_deref().unwrap_or("");
                let Ok(expr) = ContextExpr::parse(when) else {
                    warn(&mut push, KeymapProblem::BadWhen, when);
                    continue;
                };
                // 知らない変数は評価すると黙って偽になるので、割当を使わずに知らせる (07 §7.3)
                if let Some(v) = expr
                    .vars()
                    .into_iter()
                    .find(|v| !crate::command::context::CONTEXT_VARS.contains(&v.as_str()))
                {
                    warn(&mut push, KeymapProblem::UnknownWhenVar, &v);
                    continue;
                }
                let args = match args.as_deref().map(|a| check_args(command, a)) {
                    None => Args::new(),
                    Some(Ok(args)) => args,
                    Some(Err(name)) => {
                        warn(&mut push, KeymapProblem::BadArgs, &name);
                        continue;
                    }
                };
                (command.as_str(), when, args)
            }
        };
        // 同じコンテキスト・同じキーの列を別の書き方で別の行に書いた: 後の行を使う (情報、K4)。配列値の要素どうしは
        // 同じ行なので重複ではない。割当は表のキーの順に届くので、どちらが後の行かは位置で比べる
        match written.iter_mut().find(|(c, s, _)| *c == ctx && *s == seq) {
            Some(w) if file.entries[w.2].keys != e.keys => {
                let other = &file.entries[w.2];
                let dup = DiagKind::Keymap(KeymapProblem::DuplicateKey);
                if (e.line, e.column) > (other.line, other.column) {
                    layer.retain(|b| !(b.context == ctx && b.sequence == seq));
                    push(Severity::Info, dup, &other.keys);
                    w.2 = i;
                } else {
                    // こちらが先の行: 使わない。診断は後の行に付ける
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
            .bind_full(ctx, &e.keys, command, when, args)
            .expect("検証済みのキーと when");
        let when = ContextExpr::parse(when).expect("検証済みの when");
        origins.push((ctx, seq, when, i));
    }
    keymap.set_user_layer(layer.clone());
    let escape = restore_escape_keys(keymap, &mut layer, &origins, file);
    prefix_diagnostics(keymap, &origins, file, &mut entry_diags);
    entry_diags.sort_by_key(|(i, _)| *i);
    diags.extend(entry_diags.into_iter().map(|(_, d)| d));
    diags.extend(escape);
    diags
}

/// `file.entries[i]` の診断 (キーは書かれたとおり `keys.<コンテキスト>.<キー>`、位置はその行)。
fn entry_diagnostic(
    file: &KeymapFile,
    i: usize,
    severity: Severity,
    kind: DiagKind,
    value: &str,
) -> Diagnostic {
    let e = &file.entries[i];
    Diagnostic {
        severity,
        kind,
        key: format!("keys.{}.{}", e.context, e.keys),
        value: value.to_owned(),
        line: e.line,
        column: e.column,
    }
}

/// `args` をコマンドの引数の形 (07 §11.2、`command_args`) に照らして `Args` にする。
///
/// # Errors
///
/// 知らない名前・型の違い・使えない値の引数があればその名前、必須の引数が無ければその名前。
fn check_args(command: &str, args: &[(String, KeyArgValue)]) -> Result<Args, String> {
    let spec = command_args(command);
    let mut out = Args::new();
    for (name, v) in args {
        let Some(s) = spec.iter().find(|s| s.name == name) else {
            return Err(name.clone());
        };
        let value = match (s.ty, v) {
            (ArgType::Bool, KeyArgValue::Bool(b)) => ArgValue::Bool(*b),
            (ArgType::Int | ArgType::IntOrLast, KeyArgValue::Int(n)) => ArgValue::Int(*n),
            (ArgType::IntOrLast, KeyArgValue::Str(x)) if x == "$" => ArgValue::Str(x.clone()),
            (ArgType::String | ArgType::Path, KeyArgValue::Str(x)) => ArgValue::Str(x.clone()),
            (ArgType::Enum(values), KeyArgValue::Str(x)) if values.iter().any(|(v, _)| v == x) => {
                ArgValue::Str(x.clone())
            }
            _ => return Err(name.clone()),
        };
        out = out.with(name, value);
    }
    if let Some(missing) = spec
        .iter()
        .find(|s| s.required && out.get(s.name).is_none())
    {
        return Err(missing.name.to_owned());
    }
    Ok(out)
}

/// プレフィックスの衝突 (曖昧なチョード、07 §7.6) の警告を、`entry_diags` に書いた行の番号つきで足す。どれも操作を
/// 問わず警告にする (01 FR-608 AC-1 の「警告にする操作の範囲」、M29)。
/// - ユーザーの割当が既定のチョードの先頭と同じ: その割当の行で `ChordPrefix`。
/// - ユーザーの割当がユーザーの別のチョードの先頭と同じ: 短い側の行で `UserChordPrefix` (値はチョードのキー)。
/// - ユーザーのチョードが既定の単体の割当を無効にした: チョードの行で `ChordShadowsDefault` (値は無効になった既定の割当)。
///
/// 割当のコンテキストを参照するフォーカスの一部でだけ奪われる衝突は、ユーザーの割当なら `ChordPrefixPartly` (値は奪う
/// チョードの `コンテキスト: キー`)、既定の割当なら `ChordShadowsDefaultPartly` にする (ほかのフォーカスでは使える)。
fn prefix_diagnostics(
    keymap: &Keymap,
    origins: &[Origin],
    file: &KeymapFile,
    entry_diags: &mut Vec<(usize, Diagnostic)>,
) {
    let warn = |i: usize, p: KeymapProblem, value: &str| {
        (
            i,
            entry_diagnostic(file, i, Severity::Warning, DiagKind::Keymap(p), value),
        )
    };
    let mut out: Vec<(usize, Diagnostic)> = Vec::new();
    for d in keymap.conflicts() {
        let (at_layer, by_layer) = (keymap.layer(d.at.0), keymap.layer(d.by.0));
        let b = &at_layer.bindings()[d.at.1];
        let c = &by_layer.bindings()[d.by.1];
        let chord = format!("{}: {}", c.context, format_sequence(&c.sequence));
        let item = match (
            at_layer.name == "user",
            by_layer.name == "user",
            d.everywhere,
        ) {
            (true, _, false) => {
                origin(origins, b).map(|i| warn(i, KeymapProblem::ChordPrefixPartly, &chord))
            }
            (true, false, true) => origin(origins, b).map(|i| {
                let keys = file.entries[i].keys.clone();
                warn(i, KeymapProblem::ChordPrefix, &keys)
            }),
            (true, true, true) => origin(origins, b)
                .zip(origin(origins, c))
                .map(|(i, j)| warn(i, KeymapProblem::UserChordPrefix, &file.entries[j].keys)),
            (false, true, everywhere) => origin(origins, c).map(|j| {
                let value = format!(
                    "{}: {} = {}",
                    b.context,
                    format_sequence(&b.sequence),
                    b.command.as_str()
                );
                let p = if everywhere {
                    KeymapProblem::ChordShadowsDefault
                } else {
                    KeymapProblem::ChordShadowsDefaultPartly
                };
                warn(j, p, &value)
            }),
            // 既定の層どうしの衝突は作らない (`Keymap::new` が設計エラーにする)
            (false, false, _) => None,
        };
        if let Some(item) = item
            && !out.contains(&item)
        {
            out.push(item);
        }
    }
    entry_diags.extend(out);
}

/// 終了・コマンドパレット・取消が、既定で届くフォーカス (07 §7.1 の参照順の表の行) のどれかで届かなくなったら、
/// その原因の割当を外す (エラー、07 §7.6)。原因は、そのフォーカスで参照するコンテキストにあり、脱出キーと同じキーを
/// 別の用途にした (`when` 付きを含む) か解除した割当と、脱出キーを先頭にするチョード (チョードが勝つ、K3)。外すたびに
/// 調べ直し、どのフォーカスでも届くまで続ける。診断は、外した割当を書いた行の位置と、書かれたキーの表記で出す。
fn restore_escape_keys(
    keymap: &mut Keymap,
    layer: &mut Layer,
    origins: &[Origin],
    file: &KeymapFile,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for cmd in ESCAPE_COMMANDS {
        let defaults = keymap.escape_sequences(cmd);
        while let Some(&focus) = keymap.escape_lost_focuses(cmd).first() {
            let keys: Vec<&Vec<KeyStroke>> = defaults
                .iter()
                .filter(|(ctx, _)| focus.contains(ctx))
                .map(|(_, seq)| seq)
                .collect();
            let dropped = layer.retain(|b| {
                !(focus.contains(&b.context)
                    && keys
                        .iter()
                        .any(|k| b.sequence.starts_with(k) || k.starts_with(&b.sequence)))
            });
            if dropped.is_empty() {
                // ユーザー層の外の原因 (起きないはず)。外せるものが無いので止める
                break;
            }
            for b in dropped {
                let d = match origin(origins, &b) {
                    Some(i) => entry_diagnostic(
                        file,
                        i,
                        Severity::Error,
                        DiagKind::Keymap(KeymapProblem::EscapeKey),
                        cmd,
                    ),
                    None => Diagnostic {
                        severity: Severity::Error,
                        kind: DiagKind::Keymap(KeymapProblem::EscapeKey),
                        key: format!("keys.{}.{}", b.context, format_sequence(&b.sequence)),
                        value: cmd.to_owned(),
                        line: 0,
                        column: 0,
                    },
                };
                diags.push(d);
            }
            keymap.set_user_layer(layer.clone());
        }
    }
    diags
}

/// 層の割当 `b` を書いた行の番号。同じコンテキスト・キー・`when` の割当は後の行が上書きするので、最後の行。
fn origin(origins: &[Origin], b: &Binding) -> Option<usize> {
    origins
        .iter()
        .rev()
        .find(|(ctx, seq, when, _)| *ctx == b.context && *seq == b.sequence && *when == b.when)
        .map(|(_, _, _, i)| *i)
}

/// 修飾のない (または Shift だけの) 文字キー。`Text` として届くので、割り当てても効かず、文字入力を奪う。
fn is_printable(s: KeyStroke) -> bool {
    matches!(s.key, KeyCode::Char(_)) && (s.mods == Modifiers::NONE || s.mods == Modifiers::SHIFT)
}
