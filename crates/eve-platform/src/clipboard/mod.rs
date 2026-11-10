//! システムクリップボード (docs/05 §8、02 §6.3)。`ClipboardProvider` は Host が使う読み書きの口で、OS に依らない。
//! 実装は OS ごとに分ける: Windows は Win32 API を直接呼ぶ `WinClipboard` (`win`、05 §8.1)、Linux・WSL は外部コマンドを
//! 起こす `UnixClipboard` (`cmd`、05 §8.2 / §8.3)。専用ライブラリは使わない (OQ-111)。

/// システムクリップボードの失敗の分類 (`HostReport` / メッセージの理由、02 §5.4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardFailure {
    /// 開けない・他のプロセスが握っている。
    Unavailable,
    /// この環境に手段が無い。
    Unsupported,
    /// 64 MiB を超える。
    TooLarge,
    /// それ以外の失敗。
    Failed,
}

/// システムクリップボードの上限 (`clipboard.system_max_bytes` の既定、05 §8.4。v0.1 は定数)。
pub const SYSTEM_MAX_BYTES: usize = 64 * 1024 * 1024;

/// システムクリップボードの読み書き。どちらもブロックし得るので、メインスレッドから呼ばない。
pub trait ClipboardProvider: Send + Sync {
    /// テキストを書く (`CF_UNICODETEXT`)。
    ///
    /// # Errors
    ///
    /// クリップボードを開けない・他のプロセスが握っている (`Unavailable`)、この環境に手段がない (`Unsupported`)、64 MiB を超える (`TooLarge`)、それ以外の失敗 (`Failed`)。
    fn set_text(&self, text: &str) -> Result<(), ClipboardFailure>;
    /// テキストを読む。テキストが無ければ `Ok(None)`。
    ///
    /// # Errors
    ///
    /// クリップボードを開けない・他のプロセスが握っている (`Unavailable`)、この環境に手段がない (`Unsupported`)、64 MiB を超える (`TooLarge`)、それ以外の失敗 (`Failed`)。
    fn get_text(&self) -> Result<Option<String>, ClipboardFailure>;
    /// `Unsupported` を返した理由 (`msg.clipboard.fallback` の `{reason}`。文言は core が言語に合わせて作る)。
    /// 読み書きが失敗を返した後に、メインスレッドから呼ぶ (I/O をしない)。
    fn unsupported_reason(&self) -> Option<eve_core::ClipboardFallbackReason> {
        None
    }
}

pub mod cmd;
#[cfg(windows)]
mod win;

#[cfg(unix)]
pub use cmd::UnixClipboard;
#[cfg(windows)]
pub use win::WinClipboard;
