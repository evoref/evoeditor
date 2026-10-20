//! `CancelToken`: ジョブのキャンセル (docs/02-architecture.md §4.4)。待たない・ブロックしない。
//!
//! キャンセルを表す型はこれと `JobError::Cancelled` の 2 つだけにする (Port のエラーにキャンセルの変種を持たない)。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 立てる (`cancel`) と読む (`is_cancelled`) だけ。`clone` は同じトークンを指す。`JobSpawner` がジョブごとに作る。
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// 他にも同じトークンを持つ者がいる (ジョブがまだワーカーの手元にある)。
    pub fn is_shared(&self) -> bool {
        Arc::strong_count(&self.0) > 1
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 複製は同じトークンを指す() {
        let a = CancelToken::new();
        let b = a.clone();
        assert!(!a.is_cancelled() && !b.is_cancelled());
        b.cancel();
        assert!(a.is_cancelled(), "複製で立てても、元に見える");
    }

    #[test]
    fn 別のトークンは影響しない() {
        let a = CancelToken::new();
        let b = CancelToken::new();
        a.cancel();
        assert!(!b.is_cancelled());
    }

    #[test]
    fn スレッドをまたいで使える型() {
        // eve-core は std::thread を使わない (R3 / R11)。スレッド間で共有できることは型で確かめる
        fn assert_send_sync<T: Send + Sync + Clone>() {}
        assert_send_sync::<CancelToken>();
    }
}
