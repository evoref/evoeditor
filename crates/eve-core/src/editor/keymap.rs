//! キーマップ解決 (docs/02 §7.2 の K1〜K8、docs/07 §7)。frontend ではなく core が行う。
//!
//! 範囲 (M1): 層の優先、コンテキストの参照順、ストロークの列 (チョード) の保留と取消、`when` による有効判定、`mod` の解決。
//! v0.1 はユーザーのキー割当を読まず、組込みの既定キーマップだけを使う (07 §7.1)。曖昧なチョード
//! (`chord_timeout_ms`、K3) と `captures_text` の層 (dialog、K1) は、それを使うオーバーレイと同時に足す。

use std::collections::HashMap;
use std::fmt;

use eve_view::{CommandId, KeyCode, KeyStroke, Modifiers};

use super::command::{Args, CommandInvocation, InvocationSource};
use super::context::{ContextExpr, CtxValue};

// ---------- ストロークの書式 (07 §7.2) ----------

#[derive(Debug, PartialEq, Eq)]
pub enum KeyParseError {
    Empty,
    UnknownKey(String),
    UnknownModifier(String),
    /// `mod` と `ctrl` / `super` の併記 (解決後に重複する)。
    DuplicateModifier,
}

impl fmt::Display for KeyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyParseError::Empty => write!(f, "キーが空"),
            KeyParseError::UnknownKey(k) => write!(f, "未知のキー名: {k}"),
            KeyParseError::UnknownModifier(m) => write!(f, "未知の修飾: {m}"),
            KeyParseError::DuplicateModifier => write!(f, "修飾が重複している"),
        }
    }
}

impl std::error::Error for KeyParseError {}

fn add_mod(m: Modifiers, mods: &mut Modifiers) -> Result<(), KeyParseError> {
    if mods.contains(m) {
        return Err(KeyParseError::DuplicateModifier);
    }
    *mods = *mods | m;
    Ok(())
}

/// `mod+shift+s` のような 1 ストロークを読む。`mod` は TUI では `ctrl` に解決する。
/// 大文字 1 字は `shift+小文字` に正準化する。
pub fn parse_stroke(s: &str) -> Result<KeyStroke, KeyParseError> {
    if s.is_empty() {
        return Err(KeyParseError::Empty);
    }
    // `+` 自体は `plus` と書くので、`+` で区切ってよい。最後の要素がキー名
    let mut parts: Vec<&str> = s.split('+').collect();
    let key_name = parts
        .pop()
        .filter(|k| !k.is_empty())
        .ok_or(KeyParseError::Empty)?;
    let mut mods = Modifiers::NONE;
    let mut uses_mod = false;
    for p in parts {
        match p.to_ascii_lowercase().as_str() {
            "ctrl" => add_mod(Modifiers::CTRL, &mut mods)?,
            "alt" => add_mod(Modifiers::ALT, &mut mods)?,
            "shift" => add_mod(Modifiers::SHIFT, &mut mods)?,
            "super" | "cmd" | "win" | "meta" => add_mod(Modifiers::SUPER, &mut mods)?,
            "mod" => {
                if uses_mod {
                    return Err(KeyParseError::DuplicateModifier);
                }
                uses_mod = true;
                add_mod(Modifiers::CTRL, &mut mods)?;
            }
            other => return Err(KeyParseError::UnknownModifier(other.to_owned())),
        }
    }
    let mut chars = key_name.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    };
    let key = match key_name.to_ascii_lowercase().as_str() {
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "space" => KeyCode::Char(' '),
        "plus" => KeyCode::Char('+'),
        "menu" => KeyCode::Menu,
        "capslock" => KeyCode::CapsLock,
        "printscreen" => KeyCode::PrintScreen,
        f if f.starts_with('f') && f[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => {
            KeyCode::F(f[1..].parse().unwrap_or(1))
        }
        _ => match single {
            Some(c) if c.is_uppercase() => {
                // 大文字は shift + 小文字
                mods = mods | Modifiers::SHIFT;
                KeyCode::Char(c.to_lowercase().next().unwrap_or(c))
            }
            Some(c) => KeyCode::Char(c),
            None => return Err(KeyParseError::UnknownKey(key_name.to_owned())),
        },
    };
    Ok(KeyStroke { mods, key })
}

/// 空白で区切ったストロークの列 (`ctrl+k s`)。
pub fn parse_sequence(s: &str) -> Result<Vec<KeyStroke>, KeyParseError> {
    let seq: Result<Vec<_>, _> = s.split_whitespace().map(parse_stroke).collect();
    let seq = seq?;
    if seq.is_empty() {
        Err(KeyParseError::Empty)
    } else {
        Ok(seq)
    }
}

/// ストロークの表示表記 (`Ctrl+S`。04 §7.8。TUI は全 OS で `Ctrl`)。言語に依存しない ASCII。
pub fn format_stroke(k: KeyStroke) -> String {
    let mut s = String::new();
    for (m, name) in [
        (Modifiers::CTRL, "Ctrl"),
        (Modifiers::ALT, "Alt"),
        (Modifiers::SHIFT, "Shift"),
        (Modifiers::SUPER, "Super"),
    ] {
        if k.mods.contains(m) {
            s.push_str(name);
            s.push('+');
        }
    }
    match k.key {
        KeyCode::Char(' ') => s.push_str("Space"),
        KeyCode::Char(c) => s.extend(c.to_uppercase()),
        KeyCode::F(n) => s.push_str(&format!("F{n}")),
        KeyCode::Enter => s.push_str("Enter"),
        KeyCode::Tab => s.push_str("Tab"),
        KeyCode::Esc => s.push_str("Esc"),
        KeyCode::Backspace => s.push_str("Backspace"),
        KeyCode::Delete => s.push_str("Delete"),
        KeyCode::Insert => s.push_str("Insert"),
        KeyCode::Home => s.push_str("Home"),
        KeyCode::End => s.push_str("End"),
        KeyCode::PageUp => s.push_str("PageUp"),
        KeyCode::PageDown => s.push_str("PageDown"),
        KeyCode::Up => s.push_str("Up"),
        KeyCode::Down => s.push_str("Down"),
        KeyCode::Left => s.push_str("Left"),
        KeyCode::Right => s.push_str("Right"),
        KeyCode::Menu => s.push_str("Menu"),
        KeyCode::CapsLock => s.push_str("CapsLock"),
        KeyCode::PrintScreen => s.push_str("PrintScreen"),
        _ => s.push('?'),
    }
    s
}

/// チョードを含むストローク列の表記 (`Ctrl+K S`)。
pub fn format_sequence(seq: &[KeyStroke]) -> String {
    seq.iter()
        .map(|k| format_stroke(*k))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------- 割当と層 ----------

/// 1 つの割当。
#[derive(Clone, Debug)]
pub struct Binding {
    pub context: &'static str,
    pub sequence: Vec<KeyStroke>,
    pub command: CommandId,
    pub args: Args,
    pub when: ContextExpr,
}

/// 層 (既定・ユーザー・ワークスペース・モード・overlay 固有)。
#[derive(Clone, Debug, Default)]
pub struct Layer {
    pub name: &'static str,
    bindings: Vec<Binding>,
}

impl Layer {
    pub fn new(name: &'static str) -> Layer {
        Layer {
            name,
            bindings: Vec::new(),
        }
    }

    /// 割当を足す。同じ層・同じコンテキスト・同じキーで `when` が同じ定義は後勝ちで置き換える (K4)。
    /// `when` が違うものは先に足した方が優先 (配列値の先頭から)。
    pub fn bind(
        &mut self,
        context: &'static str,
        keys: &str,
        command: &str,
        when: &str,
    ) -> Result<&mut Layer, String> {
        let sequence = parse_sequence(keys).map_err(|e| format!("{keys}: {e}"))?;
        let when = ContextExpr::parse(when).map_err(|e| format!("{keys}: {e}"))?;
        let b = Binding {
            context,
            sequence,
            command: CommandId::new(command),
            args: Args::new(),
            when,
        };
        if let Some(old) = self
            .bindings
            .iter_mut()
            .find(|o| o.context == context && o.sequence == b.sequence && o.when == b.when)
        {
            *old = b;
        } else {
            self.bindings.push(b);
        }
        Ok(self)
    }

    pub fn bind_with_args(
        &mut self,
        context: &'static str,
        keys: &str,
        command: &str,
        args: Args,
    ) -> Result<&mut Layer, String> {
        self.bind(context, keys, command, "")?;
        if let Some(b) = self.bindings.last_mut() {
            b.args = args;
        }
        Ok(self)
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }
}

/// 解決の結果。
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    /// コマンドを実行する。
    Run(CommandInvocation),
    /// チョードの途中 (続きを待つ)。
    Pending,
    /// 保留中の `Esc` が保留を取り消した (通知しない、K2)。
    Cancelled,
    /// 保留中に一致しないキーが来て保留を破棄した (「未割当」の通知、今回のキーは捨てる)。
    UnassignedInSequence,
    /// 保留なしで一致するものがない。
    Unbound,
}

impl PartialEq for CommandInvocation {
    fn eq(&self, o: &CommandInvocation) -> bool {
        self.id == o.id && self.args == o.args && self.source == o.source
    }
}

#[derive(Debug)]
pub struct Keymap {
    /// 優先順 (先頭が最優先)。
    layers: Vec<Layer>,
    pending: Vec<KeyStroke>,
}

impl Keymap {
    /// `layers` は優先順 (overlay 固有 > モード > ワークスペース > ユーザー > 既定)。
    /// 同じコンテキストの中で、一方が他方の真のプレフィックスになる割当 (曖昧なチョード) は
    /// `chord_timeout_ms` を使う層が無い間は設計エラー (K3)。
    pub fn new(layers: Vec<Layer>) -> Result<Keymap, String> {
        let mut by_ctx: HashMap<&str, Vec<&Binding>> = HashMap::new();
        for b in layers.iter().flat_map(|l| &l.bindings) {
            by_ctx.entry(b.context).or_default().push(b);
        }
        for (ctx, bs) in &by_ctx {
            for a in bs {
                for b in bs {
                    if a.sequence.len() < b.sequence.len() && b.sequence.starts_with(&a.sequence) {
                        return Err(format!(
                            "曖昧なチョード ({ctx}): {:?} が {:?} のプレフィックス",
                            a.command.as_str(),
                            b.command.as_str()
                        ));
                    }
                }
            }
        }
        Ok(Keymap {
            layers,
            pending: Vec::new(),
        })
    }

    /// コマンドごとの第一割当 (`{key:<command_id>}` の解決とヒントの逆引き)。
    /// 層は優先順、コンテキストは編集面の参照順 (`editor` → `global`) で、最初に見つかった割当を採る。
    pub fn first_bindings(
        &self,
        contexts: &[&str],
    ) -> std::collections::BTreeMap<String, Vec<KeyStroke>> {
        let mut out = std::collections::BTreeMap::new();
        for ctx in contexts {
            for layer in &self.layers {
                for b in layer.bindings.iter().filter(|b| b.context == *ctx) {
                    out.entry(b.command.as_str().to_owned())
                        .or_insert_with(|| b.sequence.clone());
                }
            }
        }
        out
    }

    /// すべての割当 (コマンド ID, キー列)。層の順。ヘルプのキー一覧が使う。
    pub fn all_bindings(&self) -> Vec<(String, Vec<KeyStroke>)> {
        self.layers
            .iter()
            .flat_map(|l| l.bindings.iter())
            .map(|b| (b.command.as_str().to_owned(), b.sequence.clone()))
            .collect()
    }

    /// `first_bindings` の表示表記版。
    pub fn first_labels(&self, contexts: &[&str]) -> std::collections::BTreeMap<String, String> {
        self.first_bindings(contexts)
            .into_iter()
            .map(|(k, v)| (k, format_sequence(&v)))
            .collect()
    }

    /// 層 (試験が既定の割当を列挙する)。
    #[cfg(test)]
    pub(super) fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// 保留中のストローク (ヒント表示用)。
    pub fn pending(&self) -> &[KeyStroke] {
        &self.pending
    }

    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    /// `contexts` は参照順 (07 §7.1)。`lookup` は `when` 句の識別子の値。
    pub fn resolve(
        &mut self,
        contexts: &[&str],
        key: KeyStroke,
        lookup: &dyn Fn(&str) -> Option<CtxValue>,
    ) -> Resolution {
        // K2: 保留中の Esc は、他のどの層よりも先に保留だけを取り消す
        if !self.pending.is_empty() && key.key == KeyCode::Esc && key.mods.is_empty() {
            self.pending.clear();
            return Resolution::Cancelled;
        }
        let mut seq = std::mem::take(&mut self.pending);
        seq.push(key);
        let mut prefix_seen = false;
        for ctx in contexts {
            for layer in &self.layers {
                for b in layer.bindings.iter().filter(|b| b.context == *ctx) {
                    if !b.sequence.starts_with(&seq) || !b.when.eval(lookup) {
                        continue;
                    }
                    if b.sequence.len() == seq.len() {
                        return Resolution::Run(CommandInvocation {
                            id: b.command.clone(),
                            args: b.args.clone(),
                            source: InvocationSource::Keymap,
                        });
                    }
                    prefix_seen = true;
                }
            }
        }
        if prefix_seen {
            self.pending = seq;
            return Resolution::Pending;
        }
        if seq.len() > 1 {
            Resolution::UnassignedInSequence
        } else {
            Resolution::Unbound
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(s: &str) -> KeyStroke {
        parse_stroke(s).unwrap()
    }

    fn no_ctx(_: &str) -> Option<CtxValue> {
        None
    }

    #[test]
    fn ストロークの書式() {
        assert_eq!(
            ks("mod+s"),
            KeyStroke {
                mods: Modifiers::CTRL,
                key: KeyCode::Char('s')
            }
        );
        assert_eq!(
            ks("ctrl+shift+left").mods,
            Modifiers::CTRL | Modifiers::SHIFT
        );
        assert_eq!(
            ks("shift+ctrl+left"),
            ks("ctrl+shift+left"),
            "修飾の順序は任意"
        );
        assert_eq!(ks("cmd+x").mods, Modifiers::SUPER);
        assert_eq!(ks("A"), ks("shift+a"), "大文字は shift + 小文字");
        assert_eq!(ks("space").key, KeyCode::Char(' '));
        assert_eq!(ks("ctrl+plus").key, KeyCode::Char('+'));
        assert_eq!(ks("f12").key, KeyCode::F(12));
        assert_eq!(ks("return"), ks("enter"));
        assert_eq!(ks("ctrl+,").key, KeyCode::Char(','));
        assert_eq!(
            parse_stroke("mod+ctrl+s"),
            Err(KeyParseError::DuplicateModifier)
        );
        assert_eq!(
            parse_stroke("ctrl+ctrl+s"),
            Err(KeyParseError::DuplicateModifier)
        );
        assert!(matches!(
            parse_stroke("hyper+s"),
            Err(KeyParseError::UnknownModifier(_))
        ));
        assert!(matches!(
            parse_stroke("ctrl+foo"),
            Err(KeyParseError::UnknownKey(_))
        ));
        assert_eq!(parse_stroke(""), Err(KeyParseError::Empty));
        assert_eq!(parse_stroke("ctrl+"), Err(KeyParseError::Empty));
        assert_eq!(
            parse_sequence("ctrl+k s").unwrap(),
            vec![ks("ctrl+k"), ks("s")]
        );
        assert_eq!(parse_sequence("  "), Err(KeyParseError::Empty));
    }

    fn map() -> Keymap {
        let mut def = Layer::new("default");
        def.bind("editor", "mod+s", "file.save", "!doc.read_only")
            .unwrap();
        def.bind("editor", "ctrl+k s", "file.save_as", "").unwrap();
        def.bind("editor", "ctrl+k c", "x.chord_c", "").unwrap();
        def.bind("editor", "enter", "edit.newline", "").unwrap();
        def.bind("global", "enter", "global.enter", "").unwrap();
        def.bind("global", "mod+q", "app.quit", "").unwrap();
        let mut user = Layer::new("user");
        user.bind("editor", "enter", "user.enter", "").unwrap();
        Keymap::new(vec![user, def]).unwrap()
    }

    fn run(m: &mut Keymap, ctxs: &[&str], k: &str) -> Resolution {
        m.resolve(ctxs, ks(k), &no_ctx)
    }

    fn id(r: &Resolution) -> &str {
        match r {
            Resolution::Run(inv) => inv.id.as_str(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn 層とコンテキストの優先() {
        let mut m = map();
        assert_eq!(
            id(&run(&mut m, &["editor", "global"], "enter")),
            "user.enter",
            "上の層が先"
        );
        assert_eq!(id(&run(&mut m, &["global"], "enter")), "global.enter");
        assert_eq!(
            id(&run(&mut m, &["editor", "global"], "mod+q")),
            "app.quit",
            "editor に無ければ global"
        );
        assert_eq!(
            run(&mut m, &["editor"], "mod+q"),
            Resolution::Unbound,
            "参照しないコンテキストは見ない"
        );
    }

    #[test]
    fn when_が偽の割当は無効() {
        let mut m = map();
        let ro = |n: &str| (n == "doc.read_only").then_some(CtxValue::Bool(true));
        assert_eq!(
            m.resolve(&["editor"], ks("mod+s"), &ro),
            Resolution::Unbound
        );
        assert_eq!(
            id(&m.resolve(&["editor"], ks("mod+s"), &no_ctx)),
            "file.save"
        );
    }

    #[test]
    fn チョードは保留して_続きで実行する() {
        let mut m = map();
        assert_eq!(run(&mut m, &["editor"], "ctrl+k"), Resolution::Pending);
        assert_eq!(m.pending().len(), 1);
        assert_eq!(id(&run(&mut m, &["editor"], "s")), "file.save_as");
        assert!(m.pending().is_empty(), "実行で保留クリア");
        run(&mut m, &["editor"], "ctrl+k");
        assert_eq!(id(&run(&mut m, &["editor"], "c")), "x.chord_c");
    }

    #[test]
    fn 保留中の_esc_は保留だけを取り消す() {
        let mut m = map();
        run(&mut m, &["editor"], "ctrl+k");
        assert_eq!(run(&mut m, &["editor"], "esc"), Resolution::Cancelled);
        assert!(m.pending().is_empty());
        // 保留がなければ Esc は通常のキー
        assert_eq!(run(&mut m, &["editor"], "esc"), Resolution::Unbound);
    }

    #[test]
    fn 保留中の不一致は破棄して未割当() {
        let mut m = map();
        run(&mut m, &["editor"], "ctrl+k");
        assert_eq!(
            run(&mut m, &["editor"], "z"),
            Resolution::UnassignedInSequence
        );
        assert!(m.pending().is_empty());
        assert_eq!(
            id(&run(&mut m, &["editor"], "enter")),
            "user.enter",
            "以後は通常に戻る"
        );
    }

    #[test]
    fn 曖昧なチョードは設計エラー() {
        let mut l = Layer::new("default");
        l.bind("editor", "ctrl+k", "a.one", "").unwrap();
        l.bind("editor", "ctrl+k s", "a.two", "").unwrap();
        assert!(Keymap::new(vec![l]).is_err());
        // 別のコンテキストなら曖昧ではない
        let mut l = Layer::new("default");
        l.bind("editor", "ctrl+k", "a.one", "").unwrap();
        l.bind("search", "ctrl+k s", "a.two", "").unwrap();
        assert!(Keymap::new(vec![l]).is_ok());
    }

    #[test]
    fn 同じ層の同じキーは後勝ち() {
        let mut l = Layer::new("default");
        l.bind("editor", "enter", "first", "").unwrap();
        l.bind("editor", "enter", "second", "").unwrap();
        assert_eq!(l.bindings().len(), 1);
        let mut m = Keymap::new(vec![l]).unwrap();
        assert_eq!(id(&run(&mut m, &["editor"], "enter")), "second");
        // when が違うものは先に足した方が優先 (配列値の先頭から)
        let mut l = Layer::new("default");
        l.bind("editor", "tab", "a", "x").unwrap();
        l.bind("editor", "tab", "b", "").unwrap();
        let mut m = Keymap::new(vec![l]).unwrap();
        let x = |n: &str| (n == "x").then_some(CtxValue::Bool(true));
        assert_eq!(id(&m.resolve(&["editor"], ks("tab"), &x)), "a");
        assert_eq!(id(&m.resolve(&["editor"], ks("tab"), &no_ctx)), "b");
    }

    #[test]
    fn キーの表記() {
        assert_eq!(format_stroke(ks("mod+s")), "Ctrl+S");
        assert_eq!(format_stroke(ks("ctrl+shift+left")), "Ctrl+Shift+Left");
        assert_eq!(format_stroke(ks("f12")), "F12");
        assert_eq!(format_stroke(ks("alt+space")), "Alt+Space");
        assert_eq!(
            format_sequence(&parse_sequence("ctrl+k s").unwrap()),
            "Ctrl+K S"
        );
        let m = map();
        let labels = m.first_labels(&["editor", "global"]);
        assert_eq!(labels["file.save"], "Ctrl+S");
        assert_eq!(labels["app.quit"], "Ctrl+Q");
        assert_eq!(labels["user.enter"], "Enter", "上の層の割当が先");
        assert_eq!(labels["file.save_as"], "Ctrl+K S");
    }
}
