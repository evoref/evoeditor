//! `Editor`: eve-core の唯一の入口 (docs/02 §4.6 / §5.4)。Host・frontend・テストのハーネスはこれだけを使う。
//! すべてメインスレッドから呼び、どのメソッドも I/O をせずブロックしない (ADR-0020)。
//!
//! 現在の範囲 (M1 の C2a): 文書・View・Window の集合、変更の唯一の経路 (`EditorState::apply_transaction`)、
//! コマンドのレジストリと `execute`。編集コマンド・キーマップ・移動は後続の PR で足す。

mod command;
mod context;
mod default_keymap;
mod edit;
mod file;
mod keymap;
mod layout;
mod motion;
mod state;

use eve_view::{
    Capabilities, DocumentId, EditorEvent, Effect, InputEvent, KeyCode, KeyKind, KeyStroke,
    Modifiers, ViewId, WindowId,
};

pub use command::{
    ArgValue, Args, CommandCtx, CommandInvocation, CommandKind, CommandRegistry, CommandResult,
    CommandSpec, Handler, InvocationSource, Permission, RejectReason,
};
pub use context::{ContextExpr, CtxValue, ParseError};
pub use edit::normalize_eol;
pub use keymap::{Binding, KeyParseError, Keymap, Layer, Resolution, parse_sequence, parse_stroke};
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
        Editor {
            state: EditorState::new(caps, ports.jobs.clone(), ports.paths.clone()),
            registry,
            keymap: default_keymap::default_keymap(),
            ports,
        }
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
    }

    /// 次に `tick` が要る時刻。`None` なら無期限に待ってよい。
    pub fn next_deadline(&self) -> Option<Tick> {
        None
    }

    /// キーマップを通さずにコマンドを実行する。権限判定フックを通る。
    pub fn execute(&mut self, inv: CommandInvocation) -> CommandResult {
        self.registry.execute(&mut self.state, &inv)
    }

    /// キーマップを差し替える (層を足した設定の反映、テスト)。保留中のストロークは破棄する。
    pub fn set_keymap(&mut self, keymap: Keymap) {
        self.keymap = keymap;
    }

    /// ジョブの結果を受け取る (Host が `HostMsg::Job` で呼ぶ)。
    pub fn on_job_result(&mut self, r: crate::jobs::JobResult) {
        self.state.on_job_result(r);
    }

    /// 起動時の開き方 (`Host::start` が最初の `Resize` の後に呼ぶ)。パスがなければ無題の文書 (FR-124)。
    pub fn open_initial(&mut self, path: Option<&str>, read_only: bool) {
        match path {
            None => {
                let (_, view) = self.state.add_document(Document::new_untitled());
                if self.state.first_window().is_none() {
                    self.state.add_window(view);
                }
            }
            Some(p) => {
                let opts = crate::OpenOptions {
                    user_read_only: read_only,
                    ..Default::default()
                };
                self.state.open_path(p, opts);
            }
        }
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
                    self.on_key(KeyStroke {
                        mods: k.mods,
                        key: k.code,
                    });
                }
            }
            InputEvent::Text(s) => self.on_text(&s),
            InputEvent::Paste(s) => {
                // チョードの保留中の貼り付けは、保留を破棄して本文を入れる
                self.keymap.clear_pending();
                edit::paste_text(&mut self.state, &s);
            }
            InputEvent::Resize { cols, rows } => self.state.set_viewport(cols, rows),
            InputEvent::CapabilitiesChanged(caps) => self.state.set_caps(caps),
            // Focus と ClipboardContent は、それを使う機能 (クリップボード) と同時に扱う
            _ => {}
        }
    }

    /// 編集面にフォーカスがあるときのコンテキストの参照順 (07 §7.1)。
    const EDITOR_CONTEXTS: [&'static str; 2] = ["editor", "global"];

    fn on_key(&mut self, stroke: KeyStroke) {
        let state = &self.state;
        let res = self
            .keymap
            .resolve(&Self::EDITOR_CONTEXTS, stroke, &|n| state.context_value(n));
        match res {
            Resolution::Run(inv) => {
                self.execute(inv);
            }
            Resolution::Unbound => {
                // 修飾のない印字可能文字は Text と同じ扱い。それ以外は無視する (「未割当」の通知はメッセージ基盤と同時)
                if let KeyCode::Char(c) = stroke.key
                    && stroke.mods.is_empty()
                {
                    self.insert_text(&c.to_string());
                }
            }
            Resolution::Pending | Resolution::Cancelled | Resolution::UnassignedInSequence => {}
        }
    }

    fn on_text(&mut self, s: &str) {
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

    pub fn take_snapshot_updates(&mut self) -> Vec<DocumentId> {
        self.state.take_snapshot_updates()
    }

    /// Window の Frame を作る (可視範囲だけ。コストは文書の大きさに依存しない)。
    pub fn frame(&mut self, window: WindowId) -> Option<eve_view::Frame> {
        let f = layout::build_frame(&self.state, window);
        self.state.mark_rendered();
        f
    }

    pub fn needs_render(&self) -> bool {
        self.state.needs_render()
    }
}
