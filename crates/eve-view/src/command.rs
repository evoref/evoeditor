//! `CommandId`: コマンドの識別子 (docs/02-architecture.md §6.1〜§6.2、docs/07 §11.6 のカタログが正本)。
//! `CommandSpec` などの本体は eve-core。ID は設定ファイル・キーマップ・駆動 API でも使うので、eve-view に置く。

use std::fmt;
use std::sync::Arc;

/// `file.save` のような、`<名前空間>.<名前>` の形の識別子。CommandId 自体は翻訳しない。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommandId(Arc<str>);

impl CommandId {
    pub fn new(id: &str) -> CommandId {
        CommandId(Arc::from(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 名前空間 (最初の `.` より前)。`.` が無ければ全体。
    pub fn namespace(&self) -> &str {
        self.0.split('.').next().unwrap_or(&self.0)
    }
}

impl From<&str> for CommandId {
    fn from(id: &str) -> CommandId {
        CommandId::new(id)
    }
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn 識別子の名前空間() {
        let id = CommandId::from("file.save_as");
        assert_eq!(id.as_str(), "file.save_as");
        assert_eq!(id.namespace(), "file");
        assert_eq!(CommandId::new("quit").namespace(), "quit");
        assert_eq!(id.to_string(), "file.save_as");
    }

    #[test]
    fn 識別子は集合のキーにできる() {
        let mut set = HashSet::new();
        set.insert(CommandId::new("edit.undo"));
        assert!(set.contains(&CommandId::from("edit.undo")));
        assert!(!set.contains(&CommandId::from("edit.redo")));
    }
}
