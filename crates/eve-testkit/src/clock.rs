//! `ManualClock`: テストが `advance` で進める単調時計 (docs/02 §4.4)。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use eve_core::{Clock, Millis, Tick};

/// 起点は `Tick(0)`。複製は同じ時計を指す (Host・MemFs・テストで共有する)。
#[derive(Clone, Debug, Default)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    pub fn new() -> ManualClock {
        ManualClock::default()
    }

    pub fn advance(&self, d: Millis) {
        // 飽和加算 (u64 の上限で止まる)。CAS の繰り返し (fetch_update は MSRV と最新版で名前が違う)
        let mut cur = self.0.load(Ordering::Acquire);
        while let Err(actual) = self.0.compare_exchange_weak(
            cur,
            cur.saturating_add(d.0),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            cur = actual;
        }
    }

    pub fn set(&self, t: Tick) {
        self.0.store(t.0, Ordering::Release);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Tick {
        Tick(self.0.load(Ordering::Acquire))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 進めた分だけ進み_複製と共有する() {
        let c = ManualClock::new();
        let d = c.clone();
        assert_eq!(c.now(), Tick(0));
        c.advance(Millis(300));
        assert_eq!(d.now(), Tick(300));
        d.set(Tick(5));
        assert_eq!(c.now(), Tick(5));
        c.advance(Millis(u64::MAX));
        assert_eq!(c.now(), Tick(u64::MAX), "飽和する");
    }
}
