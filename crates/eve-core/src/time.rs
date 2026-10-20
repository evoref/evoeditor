//! 時刻の型と `Clock` Port (docs/02-architecture.md §4.4)。eve-core の時刻はすべて `Tick` で、壁時計は持たない。

use std::time::Duration;

/// 単調時刻 (ミリ秒)。起点は任意 (本番はプロセス開始、テストは 0)。
/// テスト用の時計は std の `Instant` を作れないので、独自の型にする。`Tick` 同士の加算は持たない。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(pub u64);

/// `Tick` の期間 (ミリ秒)。定数 (300 ms など) と待ち時間はこの型。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Millis(pub u64);

impl Tick {
    /// `self - earlier`。`earlier` の方が後なら `Millis(0)` (期限切れの待ち時間を 0 にしてパニックしない)。
    pub fn saturating_sub(self, earlier: Tick) -> Millis {
        Millis(self.0.saturating_sub(earlier.0))
    }
}

/// `Tick + Millis`。飽和加算 (`u64` の上限で止まる)。
impl std::ops::Add<Millis> for Tick {
    type Output = Tick;
    fn add(self, d: Millis) -> Tick {
        Tick(self.0.saturating_add(d.0))
    }
}

impl Millis {
    /// eve-host が `recv_timeout` に渡すときだけ使う (eve-core は `Duration` を待ちに使わない)。
    pub fn as_duration(self) -> Duration {
        Duration::from_millis(self.0)
    }
}

/// 単調時計。これ 1 本だけ (壁時計は持たない)。本番は eve-host、テストは `ManualClock`。
pub trait Clock: Send + Sync {
    fn now(&self) -> Tick;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 時刻の加算と差は飽和する() {
        assert_eq!(Tick(100) + Millis(300), Tick(400));
        assert_eq!(Tick(u64::MAX) + Millis(1), Tick(u64::MAX));
        assert_eq!(Tick(500).saturating_sub(Tick(200)), Millis(300));
        assert_eq!(
            Tick(200).saturating_sub(Tick(500)),
            Millis(0),
            "期限切れの待ち時間は 0"
        );
        assert!(Tick(1) < Tick(2));
    }

    #[test]
    fn 期間は_duration_に変換できる() {
        assert_eq!(Millis(1500).as_duration(), Duration::from_millis(1500));
    }
}
