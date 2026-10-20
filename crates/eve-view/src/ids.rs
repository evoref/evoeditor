//! 境界型が使う ID 型 (docs/02-architecture.md §12.2)。いずれもプロセス内で意味を持つ不透明な番号で、
//! 採番は eve-core (`DocumentId` / `ViewId` / `WindowId`) と eve-host (`JobId`) が行う。

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);

        impl $name {
            /// 内部の番号。
            pub fn get(self) -> u64 {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(
    /// 文書 (`Document`) の ID。文書の集合 (表) の添字として、閉じた後も再利用しない。
    DocumentId
);
id_type!(
    /// View (文書の表示と Selection の単位) の ID。
    ViewId
);
id_type!(
    /// Window (frontend が持つ編集面) の ID。
    WindowId
);
id_type!(
    /// ジョブの ID。プロセス内で単調増加し、再利用しない (02 §4.4 の `JobSpawner`)。
    JobId
);
id_type!(
    /// `Effect::SetClipboard` の大きな内容を指すハンドル。ID はプロセスの中でだけ意味を持つ (02 §6.3)。
    ClipboardHandleId
);
id_type!(
    /// `Effect::RequestClipboard` の応答の対応づけ。次の要求を出したときだけ、古い token の応答を捨てる。
    ClipboardToken
);

/// 文書ごとに単調増加する版番号。文書を変える Transaction を適用するたびに 1 つ進む (02 §5.3)。
/// 型は eve-view が定義する。eve-text の `Snapshot` は `Revision` を持たず、対にするのは `Document` (eve-core)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(pub u64);

impl Revision {
    /// 文書を開いた直後の版。
    pub const INITIAL: Revision = Revision(0);

    /// 次の版。上限 (`u64::MAX`) では止まる。
    pub fn next(self) -> Revision {
        Revision(self.0.saturating_add(1))
    }
}

impl std::fmt::Display for Revision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// View の表示状態 (スクロール・折り返し・設定の反映など) の版番号 (02 §8.2)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewRevision(pub u64);

impl ViewRevision {
    pub fn next(self) -> ViewRevision {
        ViewRevision(self.0.saturating_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 版番号は単調に進み_上限で止まる() {
        assert_eq!(Revision::INITIAL.next(), Revision(1));
        assert!(Revision(5) < Revision(6));
        assert_eq!(Revision(u64::MAX).next(), Revision(u64::MAX));
        assert_eq!(ViewRevision(1).next(), ViewRevision(2));
    }

    #[test]
    fn id_は表示できる() {
        assert_eq!(DocumentId(7).to_string(), "7");
        assert_eq!(DocumentId(7).get(), 7);
        assert_ne!(DocumentId(1), DocumentId(2));
    }
}
