//! キーマップ解決 (docs/02 §7.2 の K1〜K8、docs/07 §7)。frontend ではなく core が行う。
//!
//! spec: 02 §7.2, 07 §7, 07 §11.6 (既定キー), 04 §7.2
//!
//! 範囲: 層の優先、コンテキストの参照順、ストロークの列 (チョード) の保留と取消、`when` による有効判定、`mod` の解決。
//! 層は組込みの既定キーマップとユーザーのキーマップ (`user_keymap`、07 §7.1)。ユーザー設定で生じた曖昧なチョード
//! (完全一致とプレフィックスを兼ねる列) はチョードが勝つ (K3、07 §7.6)。`chord_timeout_ms` による曖昧な割当の受理
//! (K3) と `captures_text` の層 (dialog、K1) は、それを使う層ができるまで使わない。

pub(crate) mod default_keymap;
mod user_keymap;

pub use user_keymap::{CONTEXTS as KEYMAP_CONTEXTS, resolve_keymap};

use std::fmt;

use eve_view::{CommandId, KeyCode, KeyStroke, Modifiers};

use crate::command::context::{ContextExpr, CtxValue};
use crate::command::{Args, CommandInvocation, InvocationSource};

// ---------- ストロークの書式 (07 §7.2) ----------

/// キーの書式 (07 §7.2) の誤り。
#[derive(Debug, PartialEq, Eq)]
pub enum KeyParseError {
    /// キーが空 (`""`・`ctrl+` など)。
    Empty,
    /// 知らないキー名。
    UnknownKey(String),
    /// 知らない修飾 (`ctrl` / `alt` / `shift` / `super` / `mod` 以外)。
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
///
/// # Errors
///
/// キーが空なら `Empty`、知らないキー名・修飾なら `UnknownKey` / `UnknownModifier`、修飾の重複なら `DuplicateModifier`。
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
            Some(c) => {
                let k = char_stroke(c);
                mods = mods | k.mods;
                k.key
            }
            None => return Err(KeyParseError::UnknownKey(key_name.to_owned())),
        },
    };
    Ok(KeyStroke { mods, key })
}

/// 1 文字のキーのストローク。大文字で、小文字が 1 文字に決まるものは `shift+小文字` に正準化する。設定のキー
/// (`parse_stroke`) と、チョードの保留中に届いた `Text` の照合 (02 §7.2 K1) の両方がこれを使う (書いた `A` /
/// `shift+a` と、端末から届いた `Text("A")` が同じストロークになるように)。
pub(crate) fn char_stroke(c: char) -> KeyStroke {
    let mut lower = c.to_lowercase();
    match (c.is_uppercase(), lower.next(), lower.next()) {
        (true, Some(l), None) => KeyStroke {
            mods: Modifiers::SHIFT,
            key: KeyCode::Char(l),
        },
        _ => KeyStroke {
            mods: Modifiers::NONE,
            key: KeyCode::Char(c),
        },
    }
}

/// 空白で区切ったストロークの列 (`ctrl+k s`)。
///
/// # Errors
///
/// 列が空なら `Empty`。ほかは各ストロークの `parse_stroke` の誤り。
pub fn parse_sequence(s: &str) -> Result<Vec<KeyStroke>, KeyParseError> {
    let seq: Result<Vec<_>, _> = s.split_whitespace().map(parse_stroke).collect();
    let seq = seq?;
    if seq.is_empty() {
        Err(KeyParseError::Empty)
    } else {
        Ok(seq)
    }
}

/// ストロークの機械向けの表記 (`Ctrl+S`、`PageDown`、`Up`)。設定の診断のキー (`keys.<context>.<keys>`) と
/// ゴールデンの `default_keymap.tsv` に使う。**画面に出す表記は `eve_view::key_notation`** (04 §7.8。`PgDn`、`↑`)。
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

/// チョードを含むストローク列の機械向けの表記 (`Ctrl+K S`)。画面には `eve_view::key_notation` を使う。
pub fn format_sequence(seq: &[KeyStroke]) -> String {
    seq.iter()
        .map(|k| format_stroke(*k))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------- フォーカスとコンテキスト (07 §7.1) ----------

/// 編集面にフォーカスがあるときのコンテキストの参照順。
pub(crate) const FOCUS_EDITOR: &[&str] = &["editor", "global"];
/// 検索バー。
pub(crate) const FOCUS_SEARCH: &[&str] = &["search", "prompt", "global"];
/// 入力プロンプト。
pub(crate) const FOCUS_PROMPT: &[&str] = &["prompt", "global"];
/// コマンドパレット。
pub(crate) const FOCUS_PALETTE: &[&str] = &["palette", "list", "prompt", "global"];
/// バッファ一覧・設定の診断の一覧 (パレットの割当 `palette` は見ない)。
pub(crate) const FOCUS_LIST: &[&str] = &["list", "prompt", "global"];
/// ヘルプ画面。
pub(crate) const FOCUS_HELP: &[&str] = &["help", "global"];
/// 確認ダイアログ (モーダル。`global` を参照しない)。
pub(crate) const FOCUS_DIALOG: &[&str] = &["dialog"];

/// フォーカスの場所ごとの参照順の全部 (07 §7.1 の表)。
pub(crate) const FOCUSES: [&[&str]; 7] = [
    FOCUS_EDITOR,
    FOCUS_SEARCH,
    FOCUS_PROMPT,
    FOCUS_PALETTE,
    FOCUS_LIST,
    FOCUS_HELP,
    FOCUS_DIALOG,
];

/// コンテキスト `a` と `b` を同時に参照するフォーカスがある (2 つの割当が 1 回の解決で出会い得る)。
pub(crate) fn contexts_meet(a: &str, b: &str) -> bool {
    FOCUSES.iter().any(|f| f.contains(&a) && f.contains(&b))
}

// ---------- 割当と層 ----------

/// 割当の解除を表すコマンド名 (`"none"`、07 §7.1)。レジストリには無く、解決では何も実行しない。
pub const NONE_COMMAND: &str = "none";

/// 1 つの割当。
#[derive(Clone, Debug)]
pub struct Binding {
    /// 割当のコンテキスト (`editor` など。`KEYMAP_CONTEXTS`)。
    pub context: &'static str,
    /// ストロークの列 (2 つ以上ならチョード)。
    pub sequence: Vec<KeyStroke>,
    /// 実行するコマンド (`none` は割当の解除)。
    pub command: CommandId,
    /// コマンドに渡す引数。
    pub args: Args,
    /// 割当が有効な条件。
    pub when: ContextExpr,
    /// `when` の元の文字列 (空は常に真。一覧の出力 `--dump-keymap` に使う)。
    pub when_src: String,
    /// 拡張キー (kitty keyboard protocol など) が無いと区別できない割当 (07 §11.6 の `[E]`)。
    pub enhanced: bool,
}

impl Binding {
    /// `"none"` (下位の層の割当の解除)。
    pub fn is_none(&self) -> bool {
        self.command.as_str() == NONE_COMMAND
    }
}

/// 割当がこの端末で届くか (04 §7.8): 拡張キーが要る割当は拡張キーが有効なときだけ、どのストロークも横取りされないこと。
pub fn reachable(b: &Binding, caps: &eve_view::Capabilities) -> bool {
    (!b.enhanced || caps.enhanced_keys)
        && b.sequence
            .iter()
            .all(|k| !caps.intercepted_keys.contains(k))
}

/// 層 (既定・ユーザー・ワークスペース・モード・overlay 固有)。
#[derive(Clone, Debug, Default)]
pub struct Layer {
    /// 層の名前 (`default` / `user` など)。
    pub name: &'static str,
    bindings: Vec<Binding>,
}

impl Layer {
    /// 空の層を作る。
    pub fn new(name: &'static str) -> Layer {
        Layer {
            name,
            bindings: Vec::new(),
        }
    }

    /// 割当を足す。同じ層・同じコンテキスト・同じキーで `when` が同じ定義は後勝ちで置き換える (K4)。
    /// `when` が違うものは先に足した方が優先 (配列値の先頭から)。
    ///
    /// # Errors
    ///
    /// `keys` がキーの書式に合わない、または `when` が条件式として読めないとき、`keys` を添えた説明の文字列。
    pub fn bind(
        &mut self,
        context: &'static str,
        keys: &str,
        command: &str,
        when: &str,
    ) -> Result<&mut Layer, String> {
        self.bind_full(context, keys, command, when, Args::new())
    }

    /// 引数つきの割当を足す (`when` は常に真)。置き換えの規則は `bind` と同じ。
    ///
    /// # Errors
    ///
    /// `keys` がキーの書式に合わないとき、`keys` を添えた説明の文字列。
    pub fn bind_with_args(
        &mut self,
        context: &'static str,
        keys: &str,
        command: &str,
        args: Args,
    ) -> Result<&mut Layer, String> {
        self.bind_full(context, keys, command, "", args)
    }

    /// 条件と引数つきの割当を足す。置き換えの規則は `bind` と同じ。
    ///
    /// # Errors
    ///
    /// `keys` がキーの書式に合わない、または `when` が条件式として読めないとき、`keys` を添えた説明の文字列。
    pub fn bind_full(
        &mut self,
        context: &'static str,
        keys: &str,
        command: &str,
        when: &str,
        args: Args,
    ) -> Result<&mut Layer, String> {
        let sequence = parse_sequence(keys).map_err(|e| format!("{keys}: {e}"))?;
        let when_src = when.to_owned();
        let when = ContextExpr::parse(when).map_err(|e| format!("{keys}: {e}"))?;
        let b = Binding {
            context,
            sequence,
            command: CommandId::new(command),
            args,
            when,
            when_src,
            enhanced: false,
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

    /// `context` の `keys` の割当を「拡張キーが要る」(`[E]`) にする。
    pub fn mark_enhanced(&mut self, context: &str, keys: &str) {
        let Ok(seq) = parse_sequence(keys) else {
            return;
        };
        for b in self
            .bindings
            .iter_mut()
            .filter(|b| b.context == context && b.sequence == seq)
        {
            b.enhanced = true;
        }
    }

    /// 条件に合う割当だけ残し、外した割当を返す。
    pub(crate) fn retain(&mut self, keep: impl Fn(&Binding) -> bool) -> Vec<Binding> {
        let (kept, dropped): (Vec<_>, Vec<_>) = self.bindings.drain(..).partition(|b| keep(b));
        self.bindings = kept;
        dropped
    }

    /// 層の割当 (足した順)。
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
    /// 保留中に一致しないキーが来て保留を破棄した (「未割当」の通知、今回のキーは捨てる)。チョードを `"none"` で
    /// 解除したキーの列を押し終えたときも、これになる。
    UnassignedInSequence,
    /// 保留なしで一致するものがない。
    Unbound,
}

impl PartialEq for CommandInvocation {
    fn eq(&self, o: &CommandInvocation) -> bool {
        self.id == o.id && self.args == o.args && self.source == o.source
    }
}

/// プレフィックスの衝突 (曖昧なチョード、02 §7.2 K3 / 07 §7.6): 割当 `at` を、チョード `by` が奪う (どちらも
/// `(層の番号, 層の中の番号)`)。ユーザー設定でだけ生じる (既定キーマップには作らない)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Conflict {
    /// 奪われる割当。
    pub at: (usize, usize),
    /// 奪うチョード (層の優先順で最初のもの)。
    pub by: (usize, usize),
    /// `at` のコンテキストを参照するすべてのフォーカスで奪われる (使えない割当)。偽なら一部のフォーカスでだけ奪われ、
    /// ほかのフォーカスでは使える。
    pub everywhere: bool,
}

/// プレフィックスの衝突で使えなくなった割当と、それを無効にしたチョード (一覧の出力とヘルプのキー一覧、07 §7.7)。
#[derive(Clone, Copy, Debug)]
pub struct DisabledBinding<'a> {
    /// 使えなくなった割当のある層の名前 (`default` / `user`)。
    pub layer: &'static str,
    /// 使えなくなった割当。
    pub binding: &'a Binding,
    /// 無効にしたチョードのある層の名前。
    pub by_layer: &'static str,
    /// 無効にしたチョード。
    pub by: &'a Binding,
}

/// キーの列の照合の結果 (`lookup_in`)。
struct Lookup<'a> {
    /// 列にちょうど一致する割当 (上位の層の同じ列に遮蔽されていないもの。`"none"` を含む)。
    exact: Option<&'a Binding>,
    /// 列を真のプレフィックスに持つ、`"none"` でない割当がある (チョードの途中)。
    extends: bool,
}

/// 層を重ねたキーマップと、チョードの保留の状態 (02 §7.2)。
#[derive(Debug)]
pub struct Keymap {
    /// 優先順 (先頭が最優先)。
    layers: Vec<Layer>,
    pending: Vec<KeyStroke>,
    /// プレフィックスの衝突 (層を替えるたびに求め直す)。
    conflicts: Vec<Conflict>,
}

/// コンテキスト `b` を参照するフォーカスは、どれも `o` も参照する (07 §7.1 の参照順の表。`FOCUSES` が正本)。上の層の
/// `o` の割当は、`b` の割当が効くどの場所でも先に照合される。
fn covers(o: &str, b: &str) -> bool {
    FOCUSES
        .iter()
        .filter(|f| f.contains(&b))
        .all(|f| f.contains(&o))
}

/// 上位の層に、同じキーを無条件で奪う割当がある (表示から外す。`"none"` や付け替えで届かなくなった既定の割当)。
/// 奪うのは、`b` のコンテキストを参照するどのフォーカスでも先に照合される割当 (`covers`)。
fn shadowed(layers: &[Layer], layer_index: usize, b: &Binding) -> bool {
    layers[..layer_index].iter().any(|l| {
        l.bindings.iter().any(|o| {
            o.sequence == b.sequence
                && o.when == ContextExpr::always()
                && covers(o.context, b.context)
        })
    })
}

/// プレフィックスの衝突を求める: 同時に参照され得るコンテキストに、割当の列を真のプレフィックスに持つチョード
/// (`"none"` でなく、遮蔽されていないもの) がある割当 (02 §7.2 K3、07 §7.6)。チョードの `when` は見ない (真になり得る
/// なら単体を奪うので、表示と診断は奪われるものとして扱う)。割当のコンテキストを参照するフォーカスのすべてにチョードが
/// あれば「使えない割当」(`everywhere`)。
fn find_conflicts(layers: &[Layer]) -> Vec<Conflict> {
    let all: Vec<((usize, usize), &Binding)> = layers
        .iter()
        .enumerate()
        .flat_map(|(li, l)| {
            l.bindings
                .iter()
                .enumerate()
                .map(move |(bi, b)| ((li, bi), b))
        })
        .collect();
    let mut out = Vec::new();
    for &(at, b) in &all {
        if b.is_none() || shadowed(layers, at.0, b) {
            continue;
        }
        let chords: Vec<((usize, usize), &Binding)> = all
            .iter()
            .copied()
            .filter(|&(cat, c)| {
                !c.is_none()
                    && c.sequence.len() > b.sequence.len()
                    && c.sequence.starts_with(&b.sequence)
                    && contexts_meet(b.context, c.context)
                    && !shadowed(layers, cat.0, c)
            })
            .collect();
        let Some(&(by, _)) = chords.first() else {
            continue;
        };
        let everywhere = FOCUSES
            .iter()
            .filter(|f| f.contains(&b.context))
            .all(|f| chords.iter().any(|(_, c)| f.contains(&c.context)));
        out.push(Conflict { at, by, everywhere });
    }
    out
}

/// `layers` (優先順) を `contexts` の参照順で引いて `seq` を照合する。層の優先が先 (ユーザー層の `global` は既定層の
/// `editor` より優先、07 §7.4)、同じ層の中でコンテキストの参照順。同じキーの列は最初に見つかった割当だけが効く (下は
/// 遮蔽される)。`active` が偽の割当 (`when` が偽など) は無いものとする。
fn lookup_in<'a>(
    layers: impl IntoIterator<Item = &'a Layer>,
    contexts: &[&str],
    seq: &[KeyStroke],
    active: &dyn Fn(&Binding) -> bool,
) -> Lookup<'a> {
    let mut seen: Vec<&[KeyStroke]> = Vec::new();
    let mut found = Lookup {
        exact: None,
        extends: false,
    };
    for layer in layers {
        for ctx in contexts {
            for b in layer.bindings.iter().filter(|b| b.context == *ctx) {
                if !b.sequence.starts_with(seq) || !active(b) {
                    continue;
                }
                if seen.contains(&b.sequence.as_slice()) {
                    continue;
                }
                seen.push(&b.sequence);
                if b.sequence.len() == seq.len() {
                    found.exact = Some(b);
                } else if !b.is_none() {
                    found.extends = true;
                }
            }
        }
    }
    found
}

/// `layers` で、フォーカス `focus` で `seq` を押すと、`when` の値に依らず必ず `command` になる (脱出キーの検査、07 §7.6)。
/// `when` 付きの割当は真になり得るものとして扱う: 同じ列の割当は、`when` の無い割当に行き着くまでのどれもが
/// `command` で、列を先頭にする `"none"` でないチョードが (無条件の割当に遮蔽されずに) 1 つも無いこと。
fn always_runs<'a>(
    layers: impl IntoIterator<Item = &'a Layer>,
    focus: &[&str],
    seq: &[KeyStroke],
    command: &str,
) -> bool {
    let mut decided: Vec<&[KeyStroke]> = Vec::new();
    let mut exact_done = false;
    for layer in layers {
        for ctx in focus {
            for b in layer.bindings.iter().filter(|b| b.context == *ctx) {
                if !b.sequence.starts_with(seq) || decided.contains(&b.sequence.as_slice()) {
                    continue;
                }
                let always = b.when == ContextExpr::always();
                if b.sequence.len() == seq.len() {
                    if exact_done {
                        continue;
                    }
                    if b.command.as_str() != command {
                        return false;
                    }
                    if always {
                        exact_done = true;
                        decided.push(&b.sequence);
                    }
                } else if !b.is_none() {
                    return false;
                } else if always {
                    decided.push(&b.sequence);
                }
            }
        }
    }
    exact_done
}

impl Keymap {
    /// `layers` は優先順 (overlay 固有 > モード > ワークスペース > ユーザー > 既定)。
    /// 同時に参照され得るコンテキストの中で、一方が他方の真のプレフィックスになる割当 (曖昧なチョード) は
    /// `chord_timeout_ms` を使う層が無い間は設計エラー (K3)。ユーザー層は `set_user_layer` で重ね、曖昧さを診断にする。
    ///
    /// # Errors
    ///
    /// 曖昧なチョードがあれば、そのコンテキストと割当を示す説明の文字列。
    pub fn new(layers: Vec<Layer>) -> Result<Keymap, String> {
        if let Some(d) = find_conflicts(&layers).first() {
            let a = &layers[d.at.0].bindings[d.at.1];
            let b = &layers[d.by.0].bindings[d.by.1];
            return Err(format!(
                "曖昧なチョード ({} / {}): {:?} が {:?} のプレフィックス",
                a.context,
                b.context,
                a.command.as_str(),
                b.command.as_str()
            ));
        }
        Ok(Keymap {
            layers,
            pending: Vec::new(),
            conflicts: Vec::new(),
        })
    }

    /// コマンドごとの第一割当 (`{key:<command_id>}` の解決とヒントの逆引き)。
    /// 層は優先順、コンテキストは編集面の参照順 (`editor` → `global`) で、最初に見つかった割当を採る。
    pub fn first_bindings(
        &self,
        contexts: &[&str],
    ) -> std::collections::BTreeMap<String, Vec<KeyStroke>> {
        self.first_bindings_where(contexts, &|_| true)
    }

    /// `first_bindings` のうち、`ok` を満たす割当だけから選ぶ (この端末で届く割当、04 §7.8)。
    /// `contexts` のフォーカスでその割当が実際に効くもの (無条件の割当に遮蔽されず、無条件のチョードに奪われない) だけを
    /// 選び、どこでも使えない割当 (プレフィックスの衝突) も選ばない。
    pub fn first_bindings_where(
        &self,
        contexts: &[&str],
        ok: &dyn Fn(&Binding) -> bool,
    ) -> std::collections::BTreeMap<String, Vec<KeyStroke>> {
        let mut out = std::collections::BTreeMap::new();
        for (li, layer) in self.layers.iter().enumerate() {
            for ctx in contexts {
                for (bi, b) in layer.bindings.iter().enumerate() {
                    if b.context != *ctx || b.is_none() || self.is_disabled((li, bi)) || !ok(b) {
                        continue;
                    }
                    let found = lookup_in(&self.layers, contexts, &b.sequence, &|c| {
                        c.when == ContextExpr::always() || std::ptr::eq(c, b)
                    });
                    if found.extends || !found.exact.is_some_and(|e| std::ptr::eq(e, b)) {
                        continue;
                    }
                    out.entry(b.command.as_str().to_owned())
                        .or_insert_with(|| b.sequence.clone());
                }
            }
        }
        out
    }

    /// すべての割当 (表示から外すもの・プレフィックスの衝突で使えなくなったものを除く)。層の順。ヘルプのキー一覧が使う。
    pub fn all_bindings(&self) -> Vec<&Binding> {
        self.listed_bindings().into_iter().map(|(_, b)| b).collect()
    }

    /// `all_bindings` に、割当のある層の名前 (`default` / `user`) を添えたもの。一覧の出力 (`--dump-keymap`、07 §7.7) が使う。
    pub fn listed_bindings(&self) -> Vec<(&'static str, &Binding)> {
        self.layers
            .iter()
            .enumerate()
            .flat_map(|(li, l)| {
                l.bindings
                    .iter()
                    .enumerate()
                    .map(move |(bi, b)| ((li, bi), l.name, b))
            })
            .filter(|(at, _, b)| {
                !b.is_none() && !shadowed(&self.layers, at.0, b) && !self.is_disabled(*at)
            })
            .map(|(_, name, b)| (name, b))
            .collect()
    }

    /// プレフィックスの衝突で、どのフォーカスでも使えなくなった割当と、それを無効にしたチョード (層の順)。
    /// ヘルプのキー一覧と `--dump-keymap` が、使えない割当として出す (07 §7.7)。一部のフォーカスでだけ奪われる割当は
    /// 含めない (使える割当として出す)。
    pub fn disabled_bindings(&self) -> Vec<DisabledBinding<'_>> {
        self.conflicts
            .iter()
            .filter(|d| d.everywhere)
            .map(|d| DisabledBinding {
                layer: self.layers[d.at.0].name,
                binding: &self.layers[d.at.0].bindings[d.at.1],
                by_layer: self.layers[d.by.0].name,
                by: &self.layers[d.by.0].bindings[d.by.1],
            })
            .collect()
    }

    /// プレフィックスの衝突の全部 (一部のフォーカスでだけ奪われるものを含む。ユーザー層の診断が使う)。
    pub(crate) fn conflicts(&self) -> &[Conflict] {
        &self.conflicts
    }

    fn is_disabled(&self, at: (usize, usize)) -> bool {
        self.conflicts.iter().any(|d| d.at == at && d.everywhere)
    }

    /// 層 `index` (優先順の番号)。
    pub(crate) fn layer(&self, index: usize) -> &Layer {
        &self.layers[index]
    }

    /// 既定の層にある `command` の割当 (コンテキスト, キー列)。
    pub(crate) fn default_sequences(&self, command: &str) -> Vec<(&'static str, Vec<KeyStroke>)> {
        self.layers
            .iter()
            .filter(|l| l.name == "default")
            .flat_map(|l| l.bindings.iter())
            .filter(|b| b.command.as_str() == command)
            .map(|b| (b.context, b.sequence.clone()))
            .collect()
    }

    /// 脱出のコマンド `command` に、既定のキーマップでは届くのに、今のキーマップでは届かなくなったフォーカス (07 §7.1 の
    /// 参照順。07 §7.6 の「脱出キーの消失」)。「届く」は、そのフォーカスで参照する `command` の既定のキーのどれかを押すと、
    /// `when` の値に依らず必ず `command` になること。
    pub(crate) fn escape_lost_focuses(&self, command: &str) -> Vec<&'static [&'static str]> {
        let defaults = self.default_sequences(command);
        let base: Vec<&Layer> = self.layers.iter().filter(|l| l.name == "default").collect();
        FOCUSES
            .iter()
            .copied()
            .filter(|focus| {
                let reaches = |layers: &[&Layer]| {
                    defaults.iter().any(|(ctx, seq)| {
                        focus.contains(ctx)
                            && always_runs(layers.iter().copied(), focus, seq, command)
                    })
                };
                let now: Vec<&Layer> = self.layers.iter().collect();
                reaches(&base) && !reaches(&now)
            })
            .collect()
    }

    /// ユーザー層を置き換える (最優先に置く)。同じ名前 (`user`) の層があれば取り除く。プレフィックスの衝突を求め直す。
    pub fn set_user_layer(&mut self, layer: Layer) {
        self.layers.retain(|l| l.name != layer.name);
        self.layers.insert(0, layer);
        self.pending.clear();
        self.conflicts = find_conflicts(&self.layers);
    }

    /// `first_bindings` の機械向けの表記版 (試験用)。
    pub fn first_labels(&self, contexts: &[&str]) -> std::collections::BTreeMap<String, String> {
        self.first_bindings(contexts)
            .into_iter()
            .map(|(k, v)| (k, format_sequence(&v)))
            .collect()
    }

    /// 層 (試験が既定の割当を列挙する)。
    #[cfg(test)]
    pub(crate) fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// 保留中のストローク (ヒント表示用)。
    pub fn pending(&self) -> &[KeyStroke] {
        &self.pending
    }

    /// 保留中のストロークに続けて押せるキー (which-key、04 §7.4)。`contexts` は参照順。
    /// 戻り値は (次のキー, 割当のコマンド。さらに続くなら `None`)。同じキーは優先する側だけ。`"none"` で解除した
    /// チョードは出さない。同じ次のキーで単体とさらに続くチョードの両方があれば、チョードが勝つ (K3) ので `None`。
    pub fn continuations(
        &self,
        contexts: &[&str],
        lookup: &dyn Fn(&str) -> Option<CtxValue>,
    ) -> Vec<(KeyStroke, Option<String>)> {
        let n = self.pending.len();
        let mut out: Vec<(KeyStroke, Option<String>)> = Vec::new();
        if n == 0 {
            return out;
        }
        let mut seen: Vec<&[KeyStroke]> = Vec::new();
        for layer in &self.layers {
            for ctx in contexts {
                for b in layer.bindings.iter().filter(|b| b.context == *ctx) {
                    if b.sequence.len() <= n
                        || !b.sequence.starts_with(&self.pending)
                        || !b.when.eval(lookup)
                        || seen.contains(&b.sequence.as_slice())
                    {
                        continue;
                    }
                    seen.push(&b.sequence);
                    if b.is_none() {
                        continue;
                    }
                    let next = b.sequence[n];
                    let deeper = b.sequence.len() > n + 1;
                    match out.iter_mut().find(|(k, _)| *k == next) {
                        Some(slot) => {
                            if deeper {
                                slot.1 = None;
                            }
                        }
                        None => out.push((next, (!deeper).then(|| b.command.as_str().to_owned()))),
                    }
                }
            }
        }
        out
    }

    /// チョードの保留を捨てる。
    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    /// `contexts` は参照順 (07 §7.1)。`lookup` は `when` 句の識別子の値。
    ///
    /// 押した列を先頭に持つ有効なチョードがあれば保留する (完全一致の割当があってもチョードが勝つ、K3)。無ければ完全一致の
    /// 割当を実行する。`"none"` の完全一致は何もしない (保留中なら「未割当」と同じ)。
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
        let found = lookup_in(&self.layers, contexts, &seq, &|b| b.when.eval(lookup));
        if found.extends {
            self.pending = seq;
            return Resolution::Pending;
        }
        match found.exact {
            Some(b) if !b.is_none() => Resolution::Run(CommandInvocation {
                id: b.command.clone(),
                args: b.args.clone(),
                source: InvocationSource::Keymap,
            }),
            _ if seq.len() > 1 => Resolution::UnassignedInSequence,
            _ => Resolution::Unbound,
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
        assert_eq!(m.pending(), []);
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
        assert_eq!(m.pending(), []);
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
        // 同時に参照されないコンテキストなら曖昧ではない
        let mut l = Layer::new("default");
        l.bind("editor", "ctrl+k", "a.one", "").unwrap();
        l.bind("search", "ctrl+k s", "a.two", "").unwrap();
        assert!(Keymap::new(vec![l]).is_ok());
        // `global` は `editor` と同時に参照されるので、層やコンテキストが違っても曖昧
        let mut l = Layer::new("default");
        l.bind("editor", "ctrl+k", "a.one", "").unwrap();
        l.bind("global", "ctrl+k s", "a.two", "").unwrap();
        assert!(Keymap::new(vec![l]).is_err());
    }

    #[test]
    fn 同時に参照されるコンテキスト() {
        assert!(contexts_meet("editor", "global"));
        assert!(contexts_meet("palette", "prompt"));
        assert!(contexts_meet("search", "search"));
        assert!(!contexts_meet("editor", "search"));
        assert!(!contexts_meet("palette", "search"));
        assert!(
            !contexts_meet("dialog", "global"),
            "確認ダイアログはモーダル"
        );
    }

    /// 既定の層に、ユーザー層を重ねたキーマップ。
    fn with_user(user: Layer) -> Keymap {
        let mut def = Layer::new("default");
        def.bind("editor", "mod+s", "file.save", "").unwrap();
        def.bind("editor", "ctrl+k s", "file.save_as", "").unwrap();
        def.bind("editor", "ctrl+k c", "x.chord_c", "").unwrap();
        let mut m = Keymap::new(vec![def]).unwrap();
        m.set_user_layer(user);
        m
    }

    #[test]
    fn ユーザーのチョードは単体に勝ち_単体を使えない割当にする() {
        let mut user = Layer::new("user");
        user.bind("global", "ctrl+s x", "u.chord", "").unwrap();
        let mut m = with_user(user);
        assert_eq!(
            run(&mut m, &["editor", "global"], "ctrl+s"),
            Resolution::Pending
        );
        assert_eq!(id(&run(&mut m, &["editor", "global"], "x")), "u.chord");
        let d = m.disabled_bindings();
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(
            (d[0].layer, d[0].binding.command.as_str(), d[0].by_layer),
            ("default", "file.save", "user")
        );
        assert!(
            !m.first_labels(&["editor", "global"])
                .contains_key("file.save"),
            "使えない割当はキーの表記に選ばない"
        );
        assert!(
            m.all_bindings()
                .iter()
                .all(|b| b.command.as_str() != "file.save")
        );
    }

    #[test]
    fn when_が偽のチョードは単体を奪わない() {
        let mut user = Layer::new("user");
        user.bind("editor", "ctrl+s x", "u.chord", "doc.modified")
            .unwrap();
        let mut m = with_user(user);
        assert_eq!(id(&run(&mut m, &["editor"], "ctrl+s")), "file.save");
        let modified = |n: &str| (n == "doc.modified").then_some(CtxValue::Bool(true));
        assert_eq!(
            m.resolve(&["editor"], ks("ctrl+s"), &modified),
            Resolution::Pending
        );
    }

    #[test]
    fn none_のチョードは続きに出さず_押し切っても未割当() {
        let mut user = Layer::new("user");
        user.bind("editor", "ctrl+k s", NONE_COMMAND, "").unwrap();
        let mut m = with_user(user);
        run(&mut m, &["editor"], "ctrl+k");
        let next: Vec<String> = m
            .continuations(&["editor"], &no_ctx)
            .into_iter()
            .map(|(k, c)| format!("{} {c:?}", format_stroke(k)))
            .collect();
        assert_eq!(next, ["C Some(\"x.chord_c\")"]);
        assert_eq!(
            run(&mut m, &["editor"], "s"),
            Resolution::UnassignedInSequence,
            "文字を本文に入れない"
        );
        assert_eq!(m.pending(), []);
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
