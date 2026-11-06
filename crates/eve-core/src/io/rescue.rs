//! 緊急退避の書き手の契約 (docs/02 §11.2.1、05 §7.5 / §11)。
//!
//! 書き手 (`RescueWriter`) は公開済みの `DocSnapshot` だけを読み、Editor にも `FileSystem` Port にも触れずに、状態ディレクトリの
//! `recovery/<UTC timestamp>-<pid>/` へ本文 (`<n>-<basename>.txt`) と `manifest.toml` を書く。実装は eve-platform (安全な関数、
//! `std::fs` を直接使う)、呼ぶのは eve-host (パニック・Fatal・コンソールイベント・シグナルの経路)。時間予算 (`budget`) を超えたら、
//! 書けた分の manifest を `complete = false` のまま残して打ち切る。書き手は **ブロックするロックを取らない** ことが求められる。

use std::path::PathBuf;
use std::time::Duration;

use crate::DocSnapshot;

/// 退避の契機 (manifest の `reason`、02 §11.2.1)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RescueReason {
    Panic,
    Signal,
    ConsoleClose,
    /// Windows のログオフ・シャットダウン (`WM_QUERYENDSESSION` / `WM_ENDSESSION`、05 §11.2)。
    SessionEnd,
    Fatal,
    /// 「保存を中止して終了」(ui.confirm.quit_saving)。
    QuitAbortedSave,
    /// 「退避先に保存」。
    UserSave,
    /// スワップ (swap-lite、03 §10.2)。
    Swap,
}

impl RescueReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RescueReason::Panic => "panic",
            RescueReason::Signal => "signal",
            RescueReason::ConsoleClose => "console_close",
            RescueReason::SessionEnd => "session_end",
            RescueReason::Fatal => "fatal",
            RescueReason::QuitAbortedSave => "quit_aborted_save",
            RescueReason::UserSave => "user_save",
            RescueReason::Swap => "swap",
        }
    }
}

/// 退避の結果 (終了時の stderr の `stderr.recovery.*` の材料)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RescueOutcome {
    /// 退避先のディレクトリ (何も書けなければ `None`)。
    pub dir: Option<PathBuf>,
    /// 退避の対象にした文書の数。
    pub total: usize,
    /// 本文を書き終えた文書の数。
    pub written: usize,
    /// 全文書を書き終えた (manifest が `complete = true`)。
    pub complete: bool,
    /// 通常の場所に書けず、代替の退避先に書いた (05 §7.5)。
    pub fallback: bool,
    /// 秘密を含み得る文書 (`sensitive`) を退避した。
    pub sensitive: bool,
    /// 退避できなかった理由 (何も書けなかったとき)。
    pub error: Option<RescueError>,
}

/// 退避できなかった理由。stderr の文言は bin が UI の言語に合わせて作る (日本語の文字列を運ばない、01 NFR-603。
/// docs/review/v0.3-findings.md の STR-C2)。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RescueError {
    /// OS の入出力の誤り (文は OS が作ったもの)。
    Io(String),
    /// 時間内に 1 件も書けなかった。
    NotWritten,
    /// 退避の書き手が panic した (内部の誤り)。
    WriterPanicked,
    /// 退避の書き手が設定されていない (内部の誤り)。
    NoWriter,
}

impl std::fmt::Display for RescueError {
    /// ログとエラーの連鎖のための英語の短い文 (利用者への文言は bin の `texts`)。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RescueError::Io(s) => f.write_str(s),
            RescueError::NotWritten => f.write_str("nothing could be written in time"),
            RescueError::WriterPanicked => f.write_str("the rescue writer panicked"),
            RescueError::NoWriter => f.write_str("no rescue writer is set"),
        }
    }
}

impl RescueOutcome {
    /// 退避する文書が無かった (dirty な文書が無い)。
    pub fn nothing_to_save(&self) -> bool {
        self.total == 0
    }
}

/// 緊急退避の書き手。
pub trait RescueWriter: Send + Sync {
    /// `docs` (dirty な文書の公開物) を、小さい文書から順に退避する。`budget` を超えたら打ち切る。パニックしてはならない
    /// (呼び側も `catch_unwind` で包むが、書けた分の manifest を残すのは書き手の責務)。
    fn write(
        &self,
        docs: &[std::sync::Arc<DocSnapshot>],
        reason: RescueReason,
        budget: Duration,
    ) -> RescueOutcome;
}
