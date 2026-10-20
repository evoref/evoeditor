//! `Editor`: eve-core の唯一の入口 (docs/02 §4.6 / §5.4)。Host・frontend・テストのハーネスはこれだけを使う。
//! すべてメインスレッドから呼び、どのメソッドも I/O をせずブロックしない (ADR-0020)。
//!
//! 現在の範囲 (M1 の C2a): 文書・View・Window の集合、変更の唯一の経路 (`EditorState::apply_transaction`)、
//! コマンドのレジストリと `execute`。編集コマンド・キーマップ・移動は後続の PR で足す。

mod command;
mod context;
mod state;

use eve_view::{Capabilities, DocumentId, EditorEvent, Effect, ViewId, WindowId};

pub use command::{
    ArgValue, Args, CommandCtx, CommandInvocation, CommandKind, CommandRegistry, CommandResult,
    CommandSpec, Handler, InvocationSource, Permission, RejectReason,
};
pub use context::{ContextExpr, CtxValue, ParseError};
pub use state::{EditorState, ScrollPos, View, Window};

use crate::document::Document;
use crate::jobs::Ports;
use crate::time::Tick;

pub struct Editor {
    state: EditorState,
    registry: CommandRegistry,
    #[allow(dead_code)] // C4 (開く・保存) のジョブ投入で使う
    ports: Ports,
}

impl Editor {
    /// 空の EditorState を作る。I/O もジョブ投入もしない。
    pub fn new(ports: Ports, caps: Capabilities) -> Editor {
        Editor {
            state: EditorState::new(caps),
            registry: CommandRegistry::new(),
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

    pub fn drain_effects(&mut self) -> Vec<Effect> {
        self.state.drain_effects()
    }

    pub fn drain_events(&mut self) -> Vec<EditorEvent> {
        self.state.drain_events()
    }

    pub fn take_snapshot_updates(&mut self) -> Vec<DocumentId> {
        self.state.take_snapshot_updates()
    }

    pub fn needs_render(&self) -> bool {
        self.state.needs_render()
    }
}
