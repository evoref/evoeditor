//! ワークスペース層 (`<root>/.eve/config.toml`) の読込と項目の分類 (docs/07 §12.3・§12.5、01 FR-705)。
//!
//! 読込はユーザー層と同じ関数 (`load_str_for`) で行い、値を使った公開キーごとに、宣言の表の分類の列 (`WorkspacePolicy`、
//! 07 §12.3 の「v0.5 の公開キーごとの分類」) で、下の層 (ユーザー層。無ければ既定値) の値と比べて分類する。安全な項目だけを
//! 層 L3 として重ね、ほかは使わずに診断にする (WSC-4)。公開前のキー・予約のセクション (`[keys]`・`[formatter.*]`・
//! `[language-server.*]`・`[plugins]`・`clipboard.provider` など) は、ユーザー層と同じ「このバージョンでは未対応」の情報診断で
//! 使わない (WSC-6、§12.3 の表の下)。ここは純粋な関数で、読むファイルの所有者の検査は呼ぶ側 (eve-core の io のジョブ) が行う。

use crate::config::{
    ConfigValue, Magnitude, assign_key, classify_key, declared_values, load_str_for,
};
use crate::{Config, DiagKind, Diagnostic, LanguageDef, Severity, Written};

/// ルートの検出の目印の既定 (`workspace.root_markers`、07 §5.9)。
pub(crate) fn default_root_markers() -> Vec<String> {
    [".eve", ".git", ".hg", ".svn"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// ファインダ・横断検索の除外の既定 (`workspace.exclude`、07 §5.9)。
pub(crate) fn default_exclude() -> Vec<String> {
    ["**/.git", "**/node_modules", "**/target"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// ワークスペース層に書かれた値の分類 (07 §12.3 の分類の行)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceClass {
    /// 安全: 適用する。
    Safe,
    /// 準安全: 保留する (v0.5 は信頼の仕組みが無いので使わない。情報診断)。
    SemiSafe,
    /// 危険: 保留する (値を示し、ユーザーの設定へ写すことを勧めない情報診断)。
    Dangerous,
    /// 常に無視: 信頼しても使わない (警告)。
    AlwaysIgnored,
    /// 効かない: ワークスペース層では意味を持たない (情報診断)。
    NoEffect,
}

/// 公開キーのワークスペース層の分類の決め方 (宣言の表の分類の列。07 §12.3 の「v0.5 の公開キーごとの分類」の表の 2 列目)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspacePolicy {
    /// 安全。
    Safe,
    /// 安全で、下の層の配列に足す (`workspace.exclude`。置換しない)。
    SafeAppend,
    /// 準安全 (どの値でも)。
    SemiSafe,
    /// 下の層と等しい値は安全、ほかは準安全。
    SemiSafeUnlessEqual,
    /// 下の層以下の値は安全、大きい値は準安全 (閾値のキー)。
    SmallerSafe,
    /// 値 `safe` (TOML の表記) なら安全、ほかの値は `otherwise`。
    SafeValue {
        /// 安全な値の TOML の表記 (`"true"` など)。
        safe: &'static str,
        /// ほかの値の分類。
        otherwise: WorkspaceClass,
    },
    /// `file.reload_on_external_change`: `ask` と下の層と同じ値は安全、`never` と、下の層が `ask` のときの
    /// `auto_if_unmodified` は準安全。
    ExternalChange,
    /// 効かない。
    NoEffect,
    /// 常に無視。
    AlwaysIgnored,
}

impl WorkspacePolicy {
    /// 07 §12.3 の公開キーごとの表の分類の列の書き出し (表との一致の試験が、列の文の先頭と比べる)。
    #[must_use]
    pub fn spec_label(&self) -> &'static str {
        match self {
            WorkspacePolicy::Safe => "安全",
            WorkspacePolicy::SafeAppend => "安全 (ワークスペース層の値は、下の層の配列に **足す**",
            WorkspacePolicy::SemiSafe => "準安全",
            WorkspacePolicy::SemiSafeUnlessEqual => "準安全 (等しい値を除く)",
            WorkspacePolicy::SmallerSafe => "小さく・等しい: 安全 / 大きく: 準安全",
            WorkspacePolicy::SafeValue {
                safe: "true",
                otherwise: WorkspaceClass::SemiSafe,
            } => "`true`: 安全 / `false`: 準安全",
            WorkspacePolicy::SafeValue {
                safe: "true",
                otherwise: WorkspaceClass::Dangerous,
            } => "`true`: 安全 / `false`: 危険",
            WorkspacePolicy::SafeValue {
                safe: "false",
                otherwise: WorkspaceClass::Dangerous,
            } => "`false`: 安全 / `true`: 危険",
            WorkspacePolicy::SafeValue { .. } => "?",
            WorkspacePolicy::ExternalChange => "`ask` と、ユーザー層 (無ければ既定) と同じ値: 安全",
            WorkspacePolicy::NoEffect => "効かない",
            WorkspacePolicy::AlwaysIgnored => "常に無視 + 警告",
        }
    }
}

/// `policy` で、ワークスペース層の値 `ws` を下の層の値 `lower` と比べて分類する (07 §12.3 の表の下の「方向のみ」の箇条)。
pub(crate) fn classify<T: ConfigValue + Magnitude + PartialEq>(
    policy: &WorkspacePolicy,
    ws: &T,
    lower: &T,
) -> WorkspaceClass {
    use WorkspaceClass as C;
    match *policy {
        WorkspacePolicy::Safe | WorkspacePolicy::SafeAppend => C::Safe,
        WorkspacePolicy::SemiSafe => C::SemiSafe,
        WorkspacePolicy::SemiSafeUnlessEqual if ws == lower => C::Safe,
        WorkspacePolicy::SemiSafeUnlessEqual => C::SemiSafe,
        // 比べられない値 (大小を持たない) は保留する
        WorkspacePolicy::SmallerSafe => match ws.at_most(lower) {
            Some(true) => C::Safe,
            _ => C::SemiSafe,
        },
        WorkspacePolicy::SafeValue { safe, otherwise } => {
            if ws.toml() == safe {
                C::Safe
            } else {
                otherwise
            }
        }
        WorkspacePolicy::ExternalChange => {
            let (w, l) = (ws.toml(), lower.toml());
            if w == l || w == "\"ask\"" || (w == "\"auto_if_unmodified\"" && l != "\"ask\"") {
                C::Safe
            } else {
                C::SemiSafe
            }
        }
        WorkspacePolicy::NoEffect => C::NoEffect,
        WorkspacePolicy::AlwaysIgnored => C::AlwaysIgnored,
    }
}

/// ワークスペース層の割当を使わなかった理由 (07 §7.4 の WKEY-2 の (a)〜(g))。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyRejectReason {
    /// (a) 下の層に割当のあるキー (上書きできない)。
    Bound,
    /// (b) 下の層の割当を使えなくするチョード・下の層のチョードの先頭。
    Shadows,
    /// (c) 文字入力・取消し・確定のキーで始まる。
    Text,
    /// (d) `args` を持つ。
    Args,
    /// (e) `"none"` (解除)。
    None,
    /// (f) `perm` が `exec` のコマンド。
    Exec,
    /// (g) 許可リストの外のコマンド (kind が File のコマンドと `app.*`)。
    Command,
}

impl KeyRejectReason {
    /// 理由の語の名前 (`ui.workspace.key_reason.<これ>`)。
    #[must_use]
    pub fn reason_name(self) -> &'static str {
        match self {
            KeyRejectReason::Bound => "bound",
            KeyRejectReason::Shadows => "shadows",
            KeyRejectReason::Text => "text",
            KeyRejectReason::Args => "args",
            KeyRejectReason::None => "none",
            KeyRejectReason::Exec => "exec",
            KeyRejectReason::Command => "command",
        }
    }
}

/// ワークスペース層の診断の種類 (`msg.workspace.*`。07 §12.5 の WSC-4、§7.4 の WKEY-2 / WKEY-3、§12.1 の OWN-7)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceProblem {
    /// 準安全の項目を使わなかった (情報。`msg.workspace.item_pending`)。
    Pending,
    /// 危険の項目を使わなかった (情報。`Diagnostic.value` は書かれた値。`msg.workspace.item_pending_unsafe`)。
    PendingUnsafe,
    /// 常に無視の項目 (警告。`msg.workspace.item_ignored`)。
    Ignored,
    /// 効かない項目 (情報。`msg.workspace.item_no_effect`)。
    NoEffect,
    /// キーの割当を使わなかった (情報。`Diagnostic.key` は書かれたキー。`msg.workspace.key_rejected`)。
    KeyRejected(KeyRejectReason),
    /// キーの割当が上限 (1 ファイル 1,000 件) を超えた (警告。超えた分は読まない。`msg.workspace.too_many_keys`)。
    TooManyKeys,
    /// 所有者が「自分たち」でないので読まなかった (警告。`Diagnostic.value` は読まなかったファイルのパス)。
    NotOwned {
        /// 所有者の名前 (名前に直せなければ SID か uid)。
        owner: String,
    },
    /// 他人が書けるので読まなかった (警告)。
    OthersCanWrite {
        /// 書けるアカウントの名前 (`, ` で区切る)。
        who: String,
    },
    /// 所有者を判定できない FS で読まなかった (警告)。
    OwnerUnknown {
        /// FS の名前 (`exFAT`・`9p`・`remote` など)。
        fs: String,
    },
    /// 安全な場所 (`workspace.safe_directories`) の下で、開いた物を確かめられなかった (警告。開いたファイルが判定した場所と
    /// 違う・識別で開き直せない。対処に安全な場所へ足すことを勧めない)。
    OpenedElsewhere,
    /// ふつうのファイルでない (シンボリックリンク・ジャンクション・FIFO など) ので読まなかった (警告)。
    NotRegular,
    /// 大きすぎる (256 KiB を超える) ので読まなかった (警告)。
    TooLarge,
}

impl WorkspaceProblem {
    /// 文言のキーの名前 (`msg.workspace.<これ>`) と、ログの種類の名前。
    #[must_use]
    pub fn message_name(&self) -> &'static str {
        match self {
            WorkspaceProblem::Pending => "item_pending",
            WorkspaceProblem::PendingUnsafe => "item_pending_unsafe",
            WorkspaceProblem::Ignored => "item_ignored",
            WorkspaceProblem::NoEffect => "item_no_effect",
            WorkspaceProblem::KeyRejected(_) => "key_rejected",
            WorkspaceProblem::TooManyKeys => "too_many_keys",
            WorkspaceProblem::NotOwned { .. } => "not_owned",
            WorkspaceProblem::OthersCanWrite { .. } => "others_can_write",
            WorkspaceProblem::OwnerUnknown { .. } => "owner_unknown",
            WorkspaceProblem::OpenedElsewhere => "opened_elsewhere",
            WorkspaceProblem::NotRegular => "not_regular",
            WorkspaceProblem::TooLarge => "too_large",
        }
    }
}

/// ワークスペース層 (`.eve/config.toml`) を読んで分類した結果 (07 §12.5 の WSC-2)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceLayer {
    /// 使った (安全な) 項目の値。`applied` に無いキーの値は意味を持たない。`workspace.exclude` はワークスペース層の行だけ。
    config: Config,
    /// 使った公開キーと、書いた所 (キーごとに勝った 1 つ。書いた順)。
    pub applied: Vec<Written>,
    /// `[[language]]` の定義 (書いた順。`source` は空で、呼ぶ側がファイルの表示名を入れる)。
    pub languages: Vec<LanguageDef>,
    /// 診断 (読込の診断と、使わなかった項目の診断。書いた所の順)。
    pub diagnostics: Vec<Diagnostic>,
}

impl WorkspaceLayer {
    /// 安全な項目を `base` に重ねる (層 L3、07 §12.5 の WSC-3)。`workspace.exclude` はワークスペース層の行を下の層の行の
    /// **前** に足す (`!` の行がユーザー層・既定の除外を戻さない、07 §12.3)。
    pub fn apply_to(&self, base: &mut Config) {
        for w in &self.applied {
            if w.key == "workspace.exclude" {
                let mut lines = self.config.exclude.clone();
                lines.append(&mut base.exclude);
                base.exclude = lines;
            } else {
                assign_key(w.key, &self.config, base);
            }
        }
    }

    /// エラー (構文エラー・書ける構造の上限) があり、ファイル全体を使わなかったか。
    #[must_use]
    pub fn has_error(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

/// ワークスペース層の本文を読み、項目を分類する (07 §12.3 / §12.5)。`lower` は下の層 (ユーザー層。無ければ既定値) の値で、
/// 「方向のみ」の比較に使う。条件付きセクションは `conditions` で解く (07 §3.4 の COND-1)。構文エラーのファイルは何も使わない。
#[must_use]
pub fn load_workspace_str(
    text: &str,
    conditions: crate::Conditions,
    lower: &Config,
) -> WorkspaceLayer {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    // `workspace.exclude` は下の層に足すので、ワークスペース層の行だけを集める (`workspace.exclude = [...]` も置換しない)
    let mut base = lower.clone();
    base.exclude = Vec::new();
    let loaded = load_str_for(base, text, conditions);
    let mut out = WorkspaceLayer {
        config: loaded.config,
        applied: Vec::new(),
        languages: loaded.languages,
        diagnostics: loaded.diagnostics,
    };
    if out.has_error() {
        out.languages.clear();
        return out;
    }
    // キーごとに勝った (最後に重ねた) 書いた所。並びは最初に書いた順
    let mut last: Vec<Written> = Vec::new();
    for w in loaded.written {
        match last.iter_mut().find(|l| l.key == w.key) {
            Some(l) => *l = w,
            None => last.push(w),
        }
    }
    let values = declared_values(&out.config);
    for w in last {
        let class = classify_key(w.key, &out.config, lower).unwrap_or(WorkspaceClass::Dangerous);
        let problem = match class {
            WorkspaceClass::Safe => {
                out.applied.push(w);
                continue;
            }
            WorkspaceClass::SemiSafe => WorkspaceProblem::Pending,
            WorkspaceClass::Dangerous => WorkspaceProblem::PendingUnsafe,
            WorkspaceClass::AlwaysIgnored => WorkspaceProblem::Ignored,
            WorkspaceClass::NoEffect => WorkspaceProblem::NoEffect,
        };
        let severity = if class == WorkspaceClass::AlwaysIgnored {
            Severity::Warning
        } else {
            Severity::Info
        };
        let value = values
            .iter()
            .find(|(k, _)| *k == w.key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        out.diagnostics.push(Diagnostic {
            severity,
            kind: DiagKind::Workspace(problem),
            key: w.key.to_owned(),
            value,
            line: w.line,
            column: w.column,
            condition: w.condition,
        });
    }
    // 読込の診断と分類の診断を、書いた所の順に並べる (位置の無い診断は先頭)
    out.diagnostics.sort_by_key(|d| (d.line, d.column));
    out
}

#[cfg(test)]
mod tests;
