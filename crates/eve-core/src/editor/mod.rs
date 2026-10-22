//! `Editor`: eve-core の唯一の入口 (docs/02 §4.6 / §5.4)。Host・frontend・テストのハーネスはこれだけを使う。
//! すべてメインスレッドから呼び、どのメソッドも I/O をせずブロックしない (ADR-0020)。
//!
//! 現在の範囲 (M1 の C2a): 文書・View・Window の集合、変更の唯一の経路 (`EditorState::apply_transaction`)、
//! コマンドのレジストリと `execute`。編集コマンド・キーマップ・移動は後続の PR で足す。

mod chrome;
mod clipboard;
mod command;
mod config_report;
mod context;
mod default_keymap;
mod edit;
mod file;
mod keymap;
mod layout;
mod message;
mod motion;
mod overlay;
mod palette;
mod query;
mod search_bar;
mod search_state;
mod snapshot;
mod state;

use eve_view::{
    Capabilities, DocumentId, EditorEvent, Effect, InputEvent, KeyCode, KeyKind, KeyStroke,
    Modifiers, ViewId, WindowId,
};

pub use clipboard::ClipboardSource;
pub use command::{
    ArgValue, Args, CommandCtx, CommandInvocation, CommandKind, CommandRegistry, CommandResult,
    CommandSpec, Handler, InvocationSource, Permission, RejectReason,
};
pub use context::{ContextExpr, CtxValue, ParseError};
pub use edit::normalize_eol;
pub use keymap::{
    Binding, KeyParseError, Keymap, Layer, Resolution, format_stroke, parse_sequence, parse_stroke,
};
pub use search_state::{SearchCount, SearchStep};
pub use snapshot::DocSnapshot;
pub use state::{EditorState, ScrollPos, View, Window};

use crate::document::Document;
use crate::jobs::Ports;
use crate::time::Tick;

pub struct Editor {
    state: EditorState,
    registry: CommandRegistry,
    keymap: Keymap,
    #[allow(dead_code)] // C4 (開く・保存) のジョブ投入で使う
    ports: Ports,
}

impl Editor {
    /// 空の EditorState を作る。I/O もジョブ投入もしない。
    pub fn new(ports: Ports, caps: Capabilities) -> Editor {
        let mut registry = CommandRegistry::new();
        edit::register(&mut registry);
        motion::register(&mut registry);
        file::register(&mut registry);
        overlay::register(&mut registry);
        let mut state = EditorState::new(caps, ports.jobs.clone(), ports.paths.clone());
        state.set_palette_entries(
            registry
                .iter()
                .filter_map(|s| palette::PaletteEntry::new(s.id.as_str(), &s.when))
                .collect(),
        );
        let mut editor = Editor {
            state,
            registry,
            keymap: default_keymap::default_keymap(),
            ports,
        };
        editor.refresh_key_labels();
        editor
    }

    /// `{key:<command_id>}` の解決に使うキー表記を、今のキーマップから作り直す。
    fn refresh_key_labels(&mut self) {
        let labels = self.keymap.first_labels(&Self::EDITOR_CONTEXTS);
        let seqs = self.keymap.first_bindings(&Self::EDITOR_CONTEXTS);
        self.state.set_key_bindings(labels, seqs);
    }

    pub fn state(&self) -> &EditorState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut EditorState {
        &mut self.state
    }

    pub fn registry(&self) -> &CommandRegistry {
        &self.registry
    }

    pub fn register(&mut self, spec: CommandSpec) {
        self.registry.register(spec);
    }

    /// 文書を追加して表示する。Window がなければ作る。ヘッドレスのテストと、読込完了後の経路が使う。
    pub fn open_document(&mut self, doc: Document) -> (DocumentId, ViewId, WindowId) {
        let (doc_id, view_id) = self.state.add_document(doc);
        let window = match self.state.first_window() {
            Some(w) => w,
            None => self.state.add_window(view_id),
        };
        (doc_id, view_id, window)
    }

    /// Editor の「今」を更新する (Host が各 step の先頭・各メッセージの処理の直前・最後に呼ぶ)。
    pub fn tick(&mut self, now: Tick) {
        self.state.now = now;
        self.state.search_tick();
    }

    /// 次に `tick` が要る時刻。`None` なら無期限に待ってよい。
    pub fn next_deadline(&self) -> Option<Tick> {
        self.state.search_deadline()
    }

    /// キーマップを通さずにコマンドを実行する。権限判定フックを通る。
    pub fn execute(&mut self, inv: CommandInvocation) -> CommandResult {
        let mut res = self.registry.execute(&mut self.state, &inv);
        // パレットで選ばれたコマンドは、パレットを閉じた後に実行する
        while let Some(id) = self.state.take_deferred_command() {
            res = self.registry.execute(
                &mut self.state,
                &CommandInvocation::new(&id, InvocationSource::Palette),
            );
        }
        res
    }

    /// キーマップを差し替える (層を足した設定の反映、テスト)。保留中のストロークは破棄する。
    pub fn set_keymap(&mut self, keymap: Keymap) {
        self.keymap = keymap;
        self.refresh_key_labels();
    }

    /// ジョブの結果を受け取る (Host が `HostMsg::Job` で呼ぶ)。
    pub fn on_job_result(&mut self, r: crate::jobs::JobResult) {
        self.state.on_job_result(r);
    }

    /// 起動時の開き方 (`Host::start` が最初の `Resize` の後に呼ぶ)。パスがなければ無題の文書 (FR-124)。
    pub fn open_initial(&mut self, path: Option<&str>, read_only: bool) {
        self.open_initial_with(path, read_only, None);
    }

    /// `open_initial` に文字コードの指定 (`--encoding`、FR-111) を加えたもの。
    pub fn open_initial_with(
        &mut self,
        path: Option<&str>,
        read_only: bool,
        forced: Option<crate::Encoding>,
    ) {
        match path {
            None => {
                let doc = self.state.blank_document();
                let (_, view) = self.state.add_document(doc);
                if self.state.first_window().is_none() {
                    self.state.add_window(view);
                }
            }
            Some(p) => {
                let opts = crate::OpenOptions {
                    user_read_only: read_only,
                    forced,
                    ..Default::default()
                };
                self.state.open_path(p, opts);
            }
        }
        // 起動時の退避の案内 (走査は io のジョブ。開く文書のメッセージの後に出る)
        self.state.start_recovery_scan();
    }

    /// 入力を処理する (02 §5.4)。キーはキーマップを通してコマンドを実行し、`Text` は文字として挿入する。
    /// `Paste` の本文は改行を正規化して挿入し、制御文字もコマンドとして解釈しない。
    pub fn handle_input(&mut self, window: WindowId, ev: InputEvent) {
        if self.state.window(window).is_none() {
            return;
        }
        match ev {
            InputEvent::Key(k) => {
                if k.kind != KeyKind::Release {
                    let is_esc = k.code == KeyCode::Esc && k.mods.is_empty();
                    let chord_pending = !self.keymap.pending().is_empty();
                    // 保留中の Esc は保留を取り消す (メッセージは消さない、04 §8.9 の Esc の優先順位)
                    if !(is_esc && (chord_pending || self.state.overlay_kind() != "")) {
                        self.state.dismiss_message_on_key(is_esc);
                    }
                    self.on_key(KeyStroke {
                        mods: k.mods,
                        key: k.code,
                    });
                }
            }
            InputEvent::Text(s) => {
                self.state.dismiss_message_on_key(false);
                self.on_text(&s);
            }
            InputEvent::Paste(s) => {
                self.state.dismiss_message_on_key(false);
                self.state.dismiss_error_on_command();
                // チョードの保留中の貼り付けは、保留を破棄して本文を入れる
                self.keymap.clear_pending();
                if self.state.is_dialog() {
                    // 確認ダイアログでは貼り付けを捨てる (選択肢が実行されないように)
                } else if !self.state.prompt_paste(&s) {
                    edit::paste_text(&mut self.state, &s);
                }
            }
            InputEvent::Resize { cols, rows } => self.state.set_viewport(cols, rows),
            InputEvent::CapabilitiesChanged(caps) => self.state.set_caps(caps),
            // システムクリップボードの読み取りの応答。最後の要求の token と違えば捨てる
            InputEvent::ClipboardContent { token, text } => {
                self.state.on_clipboard_content(token, text);
            }
            _ => {}
        }
    }

    /// 編集面にフォーカスがあるときのコンテキストの参照順 (07 §7.1)。
    const EDITOR_CONTEXTS: [&'static str; 2] = ["editor", "global"];

    /// フォーカスの場所ごとのコンテキストの参照順 (07 §7.1)。確認ダイアログはモーダルで `global` を参照しない。
    fn contexts_for(overlay: &str) -> &'static [&'static str] {
        match overlay {
            "dialog" => &["dialog"],
            "prompt" => &["prompt", "global"],
            "palette" => &["palette", "list", "prompt", "global"],
            "search" => &["search", "prompt", "global"],
            _ => &Self::EDITOR_CONTEXTS,
        }
    }

    fn on_key(&mut self, stroke: KeyStroke) {
        let state = &self.state;
        let contexts = Self::contexts_for(state.input_context());
        let res = self
            .keymap
            .resolve(contexts, stroke, &|n| state.context_value(n));
        match res {
            Resolution::Run(inv) => {
                self.execute(inv);
            }
            Resolution::Unbound => {
                // 修飾のない印字可能文字は Text と同じ扱い。それ以外は無視する (「未割当」の通知はメッセージ基盤と同時)
                if let KeyCode::Char(c) = stroke.key
                    && (stroke.mods.is_empty()
                        || (self.state.is_dialog() && stroke.mods == Modifiers::SHIFT))
                {
                    if self.state.is_dialog() {
                        // 確認ダイアログはアクセラレータとして照合する (一致しなければ捨てる、K1)
                        self.state.dialog_accelerator(c);
                    } else {
                        self.insert_text(&c.to_string());
                    }
                }
            }
            Resolution::Pending | Resolution::Cancelled | Resolution::UnassignedInSequence => {}
        }
    }

    fn on_text(&mut self, s: &str) {
        if self.state.is_dialog() {
            // 確認ダイアログ: 複数書記素の Text はすべて捨て、1 書記素ならアクセラレータとして照合する (K1)
            let mut it = s.chars();
            if let (Some(c), None) = (it.next(), it.next()) {
                self.state.dialog_accelerator(c);
            }
            return;
        }
        if !self.keymap.pending().is_empty() {
            // チョードの保留中: 先頭の書記素を 2 打目として照合し、残りは捨てる (K1)
            let t = eve_text::Text::from(s);
            let end = eve_text::next_grapheme_boundary(&t, eve_text::ByteOff(0)).0;
            let first = s.get(..end).filter(|g| g.chars().count() == 1);
            match first.and_then(|g| g.chars().next()) {
                Some(c) => self.on_key(KeyStroke {
                    mods: Modifiers::NONE,
                    key: KeyCode::Char(c),
                }),
                None => self.keymap.clear_pending(),
            }
            return;
        }
        self.insert_text(s);
    }

    fn insert_text(&mut self, s: &str) {
        let args = Args::new().with_str("text", s);
        self.execute(
            CommandInvocation::new("edit.insert_text", InvocationSource::Keymap).with_args(args),
        );
    }

    pub fn drain_effects(&mut self) -> Vec<Effect> {
        self.state.drain_effects()
    }

    pub fn drain_events(&mut self) -> Vec<EditorEvent> {
        self.state.drain_events()
    }

    /// 退避用の公開物を作る。dirty でない文書・読込中の文書・閉じた文書は `None` (スロットを空にする)。
    pub fn emergency_snapshot(&self, doc: DocumentId) -> Option<DocSnapshot> {
        self.state.rescue_snapshot(doc, true)
    }

    /// システムクリップボードを使えなかったことを知らせる (Host が書込の失敗・取得のタイムアウトで呼ぶ)。
    pub fn notify_clipboard_fallback(&mut self, reason: &str) {
        self.state.notify_clipboard_fallback(reason);
    }

    /// `Effect::SetClipboard` の `Handle` の中身を取り出す (Host が実行時に呼ぶ、02 §4.4)。
    pub fn take_clipboard(&mut self, id: eve_view::ClipboardHandleId) -> Option<ClipboardSource> {
        self.state.take_clipboard(id)
    }

    pub fn take_snapshot_updates(&mut self) -> Vec<DocumentId> {
        self.state.take_snapshot_updates()
    }

    /// Window の Frame を作る (可視範囲だけ。コストは文書の大きさに依存しない)。
    pub fn frame(&mut self, window: WindowId) -> Option<eve_view::Frame> {
        let f = layout::build_frame(&self.state, window, self.keymap.pending());
        self.state.mark_rendered();
        f
    }

    pub fn needs_render(&self) -> bool {
        self.state.needs_render()
    }
}
