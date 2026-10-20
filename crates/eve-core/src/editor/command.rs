//! コマンドの型とレジストリ (docs/02 §6.1〜§6.3)。すべての操作は `CommandInvocation` に正規化されて実行される。
//! カタログ (ID・when・perm・既定キー) の正本は docs/07 §11.6。

use std::collections::BTreeMap;

use eve_view::{CommandId, JobId};

use super::context::{ContextExpr, ParseError};
use super::state::EditorState;

/// 引数の値。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgValue {
    Int(i64),
    Str(String),
    Bool(bool),
}

/// 引数の集合。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args(BTreeMap<String, ArgValue>);

impl Args {
    pub fn new() -> Args {
        Args::default()
    }

    pub fn with(mut self, name: &str, value: ArgValue) -> Args {
        self.0.insert(name.to_owned(), value);
        self
    }

    pub fn with_str(self, name: &str, value: &str) -> Args {
        self.with(name, ArgValue::Str(value.to_owned()))
    }

    pub fn with_int(self, name: &str, value: i64) -> Args {
        self.with(name, ArgValue::Int(value))
    }

    pub fn get(&self, name: &str) -> Option<&ArgValue> {
        self.0.get(name)
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        match self.0.get(name) {
            Some(ArgValue::Str(s)) => Some(s),
            _ => None,
        }
    }

    pub fn int(&self, name: &str) -> Option<i64> {
        match self.0.get(name) {
            Some(ArgValue::Int(n)) => Some(*n),
            _ => None,
        }
    }
}

/// 呼び出しの主体。User (キーマップ・パレット・CLI・マクロ) は常に全権。それ以外は `Permission` で制限する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InvocationSource {
    /// チョードを含む。
    Keymap,
    Palette,
    Cli,
    Macro,
    /// 駆動 API・MCP sidecar など外部クライアント (v0.2 以降。変種のみ予約)。
    External,
}

impl InvocationSource {
    fn is_user(self) -> bool {
        !matches!(self, InvocationSource::External)
    }
}

#[derive(Clone, Debug)]
pub struct CommandInvocation {
    pub id: CommandId,
    pub args: Args,
    pub source: InvocationSource,
}

impl CommandInvocation {
    pub fn new(id: &str, source: InvocationSource) -> CommandInvocation {
        CommandInvocation {
            id: CommandId::new(id),
            args: Args::new(),
            source,
        }
    }

    pub fn with_args(mut self, args: Args) -> CommandInvocation {
        self.args = args;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    Edit,
    Motion,
    Ui,
    File,
}

/// User 以外の主体からの呼出し可否 (07 §11.6 の perm 列と 1 対 1)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    /// `-`: 外部から呼べない。
    None,
    /// `read`: 読み取りだけ。
    Read,
    /// `propose`: 結果が Proposal を経る。
    Propose,
    /// `ui`: 通知・オーバーレイ等の UI 操作。
    Ui,
}

/// コマンドの実行時の文脈。
pub struct CommandCtx<'a> {
    pub state: &'a mut EditorState,
    pub args: &'a Args,
}

pub type Handler = fn(&mut CommandCtx<'_>) -> CommandResult;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    UnknownCommand,
    /// `when` が偽。
    NotAvailable,
    /// 読み取り専用または読込中。
    ReadOnly,
    /// 外部の主体に許されていない。
    PermissionDenied,
    /// 引数の過不足・型の誤り。
    InvalidArgs,
    /// 対象の文書・View がない。
    NoTarget,
    /// 確認または入力 (名前の入力・上書きの確認など) が要る。確認の UI ができるまでは拒否する。
    NeedsInput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandResult {
    Done,
    /// ジョブを投入した。結果は `on_job_result` で届く。
    Pending(JobId),
    Rejected(RejectReason),
}

pub struct CommandSpec {
    pub id: CommandId,
    pub kind: CommandKind,
    pub permission: Permission,
    pub when: ContextExpr,
    pub handler: Handler,
}

impl CommandSpec {
    /// `when` は 07 §11.6 の文字列 (空は常に真)。
    pub fn new(
        id: &str,
        kind: CommandKind,
        permission: Permission,
        when: &str,
        handler: Handler,
    ) -> Result<CommandSpec, ParseError> {
        Ok(CommandSpec {
            id: CommandId::new(id),
            kind,
            permission,
            when: ContextExpr::parse(when)?,
            handler,
        })
    }
}

/// コマンドの登録簿。
#[derive(Default)]
pub struct CommandRegistry {
    specs: BTreeMap<CommandId, CommandSpec>,
}

impl CommandRegistry {
    pub fn new() -> CommandRegistry {
        CommandRegistry::default()
    }

    /// 登録する。同じ ID が既にあれば置き換える。
    pub fn register(&mut self, spec: CommandSpec) {
        self.specs.insert(spec.id.clone(), spec);
    }

    pub fn get(&self, id: &CommandId) -> Option<&CommandSpec> {
        self.specs.get(id)
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &CommandSpec> {
        self.specs.values()
    }

    /// 権限・条件を確かめて実行する (02 §6.1)。
    pub fn execute(&self, state: &mut EditorState, inv: &CommandInvocation) -> CommandResult {
        // オーバーレイが出ている間はモーダル: 入力欄の編集はプロンプトへ、それ以外の文書のコマンドは拒否する
        if let Some(r) = state.overlay_intercept(inv) {
            return r;
        }
        let Some(spec) = self.specs.get(&inv.id) else {
            return CommandResult::Rejected(RejectReason::UnknownCommand);
        };
        // 権限判定フック: User 以外は `Permission::None` を呼べない
        if !inv.source.is_user() && spec.permission == Permission::None {
            return CommandResult::Rejected(RejectReason::PermissionDenied);
        }
        if !spec.when.eval(&|name| state.context_value(name)) {
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        // Error のメッセージは、次のコマンド実行で消える (カーソル移動では消えない、04 §8.9)
        if spec.kind != CommandKind::Motion {
            state.dismiss_error_on_command();
        }
        // Motion の実行はカーソル移動なので、Undo の結合を封印する (03 §7.3 R3)
        if spec.kind == CommandKind::Motion {
            state.seal_active_history();
        }
        let mut ctx = CommandCtx {
            state,
            args: &inv.args,
        };
        (spec.handler)(&mut ctx)
    }
}
