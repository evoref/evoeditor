//! 置換中フラグ (docs/02 §4.4、§9.6 手順 3)。利用者のファイルの置換 (`ReplaceFileW` / rename) と、直接上書きの書込の間だけ
//! 上がる計数。プロセスをここで終わらせると元のファイルが壊れる (退避名に移ったまま、または書きかけ) ので、終了の経路はこれが
//! 0 に戻るまで待つ。`Host` が起動時に 1 つ作り、本番の `FileSystem` (eve-platform) と、終了の待ち (main・受信スレッド) に配る。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct Inner {
    count: AtomicUsize,
    /// 置換中のファイルの表示用パス (終了時の `stderr.quit.waiting_save` の材料)。
    paths: Mutex<Vec<String>>,
}

/// 置換中の計数。clone は同じ計数を指す。待つ側はブロックしない読みだけをする。
#[derive(Clone, Debug, Default)]
pub struct ReplacingCount(Arc<Inner>);

impl ReplacingCount {
    /// 置換を始める。返したガードが drop されると下ろす。`path` は表示用のパス。
    pub fn guard(&self, path: &str) -> ReplacingGuard {
        self.0.count.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut p) = self.0.paths.lock() {
            p.push(path.to_owned());
        }
        ReplacingGuard {
            inner: Arc::clone(&self.0),
            path: path.to_owned(),
        }
    }

    /// 下りている (置換中のものが無い)。
    pub fn is_zero(&self) -> bool {
        self.0.count.load(Ordering::Acquire) == 0
    }

    /// 置換中のファイルの表示用パス。
    pub fn paths(&self) -> Vec<String> {
        self.0.paths.lock().map(|p| p.clone()).unwrap_or_default()
    }
}

/// 置換中であることを示す RAII ガード (作ると +1、drop で -1)。
#[derive(Debug)]
pub struct ReplacingGuard {
    inner: Arc<Inner>,
    path: String,
}

impl Drop for ReplacingGuard {
    fn drop(&mut self) {
        if let Ok(mut p) = self.inner.paths.lock()
            && let Some(i) = p.iter().position(|x| x == &self.path)
        {
            p.remove(i);
        }
        self.inner.count.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ガードの間だけ上がる() {
        let c = ReplacingCount::default();
        assert!(c.is_zero());
        let g = c.guard("a.txt");
        let c2 = c.clone();
        assert!(!c2.is_zero(), "clone は同じ計数");
        assert_eq!(c.paths(), vec!["a.txt".to_owned()]);
        drop(g);
        assert!(c.is_zero() && c.paths().is_empty());
    }

    #[test]
    fn 複数のガード() {
        let c = ReplacingCount::default();
        let (a, b) = (c.guard("a"), c.guard("b"));
        drop(a);
        assert!(!c.is_zero());
        assert_eq!(c.paths(), vec!["b".to_owned()]);
        drop(b);
        assert!(c.is_zero());
    }
}
