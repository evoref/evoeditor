//! ファイルへのログ (docs/05 §7.3、Windows)。
//!
//! 場所は `state_dir/log/evoeditor.log` (+ `.1`, `.2`)。ローテーションは 5 MiB × 3 世代の定数。ディレクトリとファイルは
//! **最初のイベントのときに** 作る (起動経路で `state_dir` を作らない、NFR-101)。ログのレベルは環境変数 `EVE_LOG`
//! (`error` / `warn` / `info` / `debug` / `trace`。既定 `warn`)。
//!
//! 現在は同期で書く (イベントは設定の診断など起動時に少数)。ログスレッドと `tracing` の購読者 (ADR-0012) は、
//! core がイベントを出すようになる版で入れる。

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::inflight::create_dir_all_private;
use crate::rescue::{now_secs, timestamp_rfc3339};

/// ローテーションの大きさと世代 (05 §7.3 の定数)。
const MAX_BYTES: u64 = 5 * 1024 * 1024;
const GENERATIONS: u32 = 3;

/// ログのレベル。小さいほど重い。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Level {
    /// `EVE_LOG` の値。読めなければ `None`。
    pub fn parse(s: &str) -> Option<Level> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "error" => Level::Error,
            "warn" => Level::Warn,
            "info" => Level::Info,
            "debug" => Level::Debug,
            "trace" => Level::Trace,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        }
    }
}

pub struct Logger {
    dir: Option<PathBuf>,
    level: Level,
    /// 書込の排他 (複数のスレッドから呼べる)。
    lock: Mutex<()>,
}

impl Logger {
    /// `state_dir` が `None` (解決できない) ならどこにも書かない。レベルは `EVE_LOG`。
    pub fn new(state_dir: Option<&Path>) -> Logger {
        let level = std::env::var("EVE_LOG")
            .ok()
            .and_then(|v| Level::parse(&v))
            .unwrap_or(Level::Warn);
        Logger::with_level(state_dir, level)
    }

    pub fn with_level(state_dir: Option<&Path>, level: Level) -> Logger {
        Logger {
            dir: state_dir.map(|d| d.join("log")),
            level,
            lock: Mutex::new(()),
        }
    }

    /// ログファイルの場所 (まだ無くてもよい)。
    pub fn path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("evoeditor.log"))
    }

    pub fn enabled(&self, level: Level) -> bool {
        level <= self.level
    }

    /// 1 行を書く。レベルが足りなければ何もしない。書けなくても呼び出し側には何も返さない (ログで本体を止めない)。
    pub fn log(&self, level: Level, message: &str) {
        if !self.enabled(level) {
            return;
        }
        let _ = self.write(level, message);
    }

    fn write(&self, level: Level, message: &str) -> io::Result<()> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        create_dir_all_private(dir)?;
        let file = dir.join("evoeditor.log");
        if fs::metadata(&file).is_ok_and(|m| m.len() >= MAX_BYTES) {
            rotate(&file);
        }
        // 改行は 1 行に収める
        let one_line = message.replace(['\r', '\n'], " ");
        let line = format!(
            "{} {} {one_line}\n",
            timestamp_rfc3339(now_secs()),
            level.name()
        );
        let mut f = crate::sys::private_options()
            .create(true)
            .append(true)
            .open(&file)?;
        f.write_all(line.as_bytes())
    }
}

/// `evoeditor.log` → `.1` → `.2` と送る (最古を捨てる)。失敗は無視する (他のインスタンスが先に改名したなど)。
fn rotate(file: &Path) {
    let gen_path = |n: u32| {
        let mut p = file.as_os_str().to_owned();
        p.push(format!(".{n}"));
        PathBuf::from(p)
    };
    for n in (1..GENERATIONS).rev() {
        let from = if n == 1 {
            file.to_path_buf()
        } else {
            gen_path(n - 1)
        };
        let _ = fs::remove_file(gen_path(n));
        let _ = fs::rename(&from, gen_path(n));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("eve-log-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn レベル() {
        assert_eq!(Level::parse("INFO"), Some(Level::Info));
        assert_eq!(Level::parse(" warn "), Some(Level::Warn));
        assert_eq!(Level::parse("x"), None);
    }

    #[test]
    fn 最初のイベントまでディレクトリを作らず_レベル未満は書かない() {
        let d = tmp("lazy");
        let l = Logger::with_level(Some(&d), Level::Warn);
        l.log(Level::Info, "書かない");
        assert!(!d.exists(), "起動経路で state_dir を作らない");
        l.log(Level::Warn, "一行目\n二行目");
        let text = fs::read_to_string(d.join("log").join("evoeditor.log")).unwrap();
        assert!(text.trim_end().ends_with("WARN 一行目 二行目"), "{text}");
        assert_eq!(text.lines().count(), 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn state_dir_が無ければ何もしない() {
        Logger::with_level(None, Level::Trace).log(Level::Error, "x");
    }

    #[test]
    fn 大きくなったら世代を送る() {
        let d = tmp("rot");
        let l = Logger::with_level(Some(&d), Level::Warn);
        let log = d.join("log");
        fs::create_dir_all(&log).unwrap();
        fs::write(log.join("evoeditor.log"), vec![b'x'; MAX_BYTES as usize]).unwrap();
        fs::write(log.join("evoeditor.log.1"), b"old1").unwrap();
        l.log(Level::Warn, "new");
        assert_eq!(fs::read(log.join("evoeditor.log.2")).unwrap(), b"old1");
        assert_eq!(
            fs::metadata(log.join("evoeditor.log.1")).unwrap().len(),
            MAX_BYTES
        );
        assert!(
            fs::read_to_string(log.join("evoeditor.log"))
                .unwrap()
                .contains("WARN new")
        );
        let _ = fs::remove_dir_all(&d);
    }
}
