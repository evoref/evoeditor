//! コマンドの型とレジストリ (docs/02 §6.1〜§6.3)。すべての操作は `CommandInvocation` に正規化されて実行される。
//! カタログ (ID・when・perm・既定キー) の正本は docs/07 §11.6。
//!
//! spec: 02 §6, 07 §11.6 (カタログ), 03 §15 (編集・クリップボード)

pub(crate) mod arg_schema;
pub(crate) mod clipboard;
pub(crate) mod context;
pub(crate) mod edit;
pub(crate) mod ids;
pub(crate) mod line_ops;

use std::collections::BTreeMap;

use eve_view::{CommandId, JobId};

use crate::command::context::{ContextExpr, ParseError};
use crate::editor::state::EditorState;

/// 引数の値。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgValue {
    /// 整数。
    Int(i64),
    /// 文字列。
    Str(String),
    /// 真偽値。
    Bool(bool),
}

/// 引数の集合。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args(BTreeMap<String, ArgValue>);

impl Args {
    /// 空の引数の集合を作る。
    pub fn new() -> Args {
        Args::default()
    }

    /// 引数 `name` に `value` を入れた集合を返す。同じ名前があれば置き換える。
    pub fn with(mut self, name: &str, value: ArgValue) -> Args {
        self.0.insert(name.to_owned(), value);
        self
    }

    /// 文字列の引数を入れた集合を返す (`with` の略記)。
    pub fn with_str(self, name: &str, value: &str) -> Args {
        self.with(name, ArgValue::Str(value.to_owned()))
    }

    /// 整数の引数を入れた集合を返す (`with` の略記)。
    pub fn with_int(self, name: &str, value: i64) -> Args {
        self.with(name, ArgValue::Int(value))
    }

    /// 引数 `name` の値。無ければ `None`。
    pub fn get(&self, name: &str) -> Option<&ArgValue> {
        self.0.get(name)
    }

    /// 引数 `name` が文字列ならその値。無い・型が違えば `None`。
    pub fn str(&self, name: &str) -> Option<&str> {
        match self.0.get(name) {
            Some(ArgValue::Str(s)) => Some(s),
            _ => None,
        }
    }

    /// 引数 `name` が整数ならその値。無い・型が違えば `None`。
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
    /// コマンドパレット。
    Palette,
    /// コマンド行 (CLI) からの呼び出し。
    Cli,
    /// マクロの再生。
    Macro,
    /// 駆動 API・MCP sidecar など外部クライアント (v0.2 以降。変種のみ予約)。
    External,
}

impl InvocationSource {
    fn is_user(self) -> bool {
        !matches!(self, InvocationSource::External)
    }
}

/// コマンドの呼び出し (02 §6.1)。すべての操作はこの形に正規化されて `CommandRegistry::execute` で実行される。
#[derive(Clone, Debug)]
pub struct CommandInvocation {
    /// 呼ぶコマンドの ID (07 §11.6 のカタログ)。
    pub id: CommandId,
    /// 引数。
    pub args: Args,
    /// 呼び出しの主体 (権限の判定に使う)。
    pub source: InvocationSource,
}

impl CommandInvocation {
    /// 引数なしの呼び出しを作る。
    pub fn new(id: &str, source: InvocationSource) -> CommandInvocation {
        CommandInvocation {
            id: CommandId::new(id),
            args: Args::new(),
            source,
        }
    }

    /// 引数を `args` に置き換えた呼び出しを返す。
    pub fn with_args(mut self, args: Args) -> CommandInvocation {
        self.args = args;
        self
    }
}

/// コマンドの種別 (02 §6.3)。Undo・権限の判定に使う。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    /// 文書を変える (Transaction を 1 つ以上生成する)。
    Edit,
    /// Selection だけを動かす (カーソル移動・選択の拡張)。
    Motion,
    /// UI の状態だけを変える (パレット・オーバーレイ・分割など)。
    Ui,
    /// ファイルの操作 (ジョブを起動し得る。保存・再読込など)。
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
    /// エディタの状態。
    pub state: &'a mut EditorState,
    /// 呼び出しの引数。
    pub args: &'a Args,
}

/// コマンドの処理の本体。
pub type Handler = fn(&mut CommandCtx<'_>) -> CommandResult;

/// コマンドを実行しなかった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// 登録されていない ID。
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

/// コマンドの実行の結果 (02 §6.1)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandResult {
    /// 実行を終えた。
    Done,
    /// ジョブを投入した。結果は `on_job_result` で届く。
    Pending(JobId),
    /// 実行しなかった。
    Rejected(RejectReason),
}

/// 登録するコマンドの定義 (02 §6.1)。
pub struct CommandSpec {
    /// コマンドの ID。
    pub id: CommandId,
    /// 種別。
    pub kind: CommandKind,
    /// User 以外の主体からの呼出し可否 (07 §11.6 の perm 列)。
    pub permission: Permission,
    /// 実行できる条件 (07 §11.6 の when 列を解析したもの)。
    pub when: ContextExpr,
    /// `when` の元の文字列 (07 §11.6 のカタログの書き方のまま。一覧の出力と突合に使う)。
    pub when_src: String,
    /// 処理の本体。
    pub handler: Handler,
}

impl CommandSpec {
    /// `when` は 07 §11.6 の文字列 (空は常に真)。
    ///
    /// # Errors
    ///
    /// `when` が条件式の構文に合わないとき `ParseError` (知らない識別子は誤りではなく偽として評価する)。
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
            when_src: when.to_owned(),
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
    /// 空の登録簿を作る。
    pub fn new() -> CommandRegistry {
        CommandRegistry::default()
    }

    /// 登録する。同じ ID が既にあれば置き換える。
    pub fn register(&mut self, spec: CommandSpec) {
        self.specs.insert(spec.id.clone(), spec);
    }

    /// 組込みのコマンドを登録する (docs/review/v0.3-findings.md の STR-B4 (2))。`when` は 07 §11.6 のカタログの
    /// 文字列。構文の誤りと、同じ ID の二重の登録は組込みの誤りなので panic する (起動の試験で分かる)。
    pub(crate) fn add(
        &mut self,
        id: &str,
        kind: CommandKind,
        permission: Permission,
        when: &str,
        handler: Handler,
    ) {
        let spec = CommandSpec::new(id, kind, permission, when, handler)
            .unwrap_or_else(|e| panic!("組込みのコマンド {id} の when が読めない: {e:?}"));
        let old = self.specs.insert(spec.id.clone(), spec);
        assert!(old.is_none(), "組込みのコマンド {id} を二重に登録した");
    }

    /// ID の定義。登録されていなければ `None`。
    pub fn get(&self, id: &CommandId) -> Option<&CommandSpec> {
        self.specs.get(id)
    }

    /// 登録したコマンドの数。
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// 1 つも登録していない。
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// 登録した定義を ID の順に返す。
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
            // 読み取り専用の文書への編集は、条件で止まるときも理由を示す (FR-116 AC-1)
            if spec.kind == CommandKind::Edit
                && state
                    .context_value("doc.read_only")
                    .is_some_and(|v| v.truthy())
            {
                state.notify_read_only();
            }
            return CommandResult::Rejected(RejectReason::NotAvailable);
        }
        // Error のメッセージは、次の編集・ファイルのコマンド実行で消える (カーソル移動・オーバーレイ内の操作では消えない、04 §8.9)
        if matches!(spec.kind, CommandKind::Edit | CommandKind::File) {
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
        let res = (spec.handler)(&mut ctx);
        // 読み取り専用の文書への編集は、理由つきで拒否したことを示す (FR-116 AC-1)
        if matches!(res, CommandResult::Rejected(RejectReason::ReadOnly)) {
            state.notify_read_only();
        }
        res
    }
}
