//! `SystemClock`: 本番の単調時計 (docs/02 §4.4)。起動時の `Instant` からの経過ミリ秒を `Tick` にする。

use std::time::Instant;

use eve_core::{Clock, Tick};

#[derive(Debug)]
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    pub fn new() -> SystemClock {
        SystemClock {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> SystemClock {
        SystemClock::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Tick {
        Tick(u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 時刻は単調に進む() {
        let c = SystemClock::new();
        let a = c.now();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = c.now();
        assert!(b >= a);
        assert!(b.0 >= a.0 + 4, "{a:?} {b:?}");
    }
}
