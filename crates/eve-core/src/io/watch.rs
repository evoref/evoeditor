//! ファイル監視の Port `FileWatcher` と、その知らせ `FsEvent` (docs/02 §4.4 の「`FileWatcher` (v0.5 の M33)」)。
//!
//! 本番の実装は eve-platform (notify。05 §12.2)、試験の実装は eve-testkit の `FakeWatcher` (09 §3.4)。どのメソッドも
//! 要求を積むだけで OS を呼ばないので、メインスレッドから呼んでよい (02 §4.4 の規則 3)。登録の結果と監視の知らせは、
//! Host の受信口に `HostMsg::Fs(FsEvent)` で届く。
//!
//! spec: 02 §4.4, 05 §12.2, 03 §9.5.1

use std::ffi::OsString;
use std::fmt;

use super::fspath::FsPath;

/// 監視の要求の識別 (`FileWatcher::watch_dir` が振る)。実装の中で単調に増やし、使い回さない。
///
/// 同じディレクトリを 2 回頼まないための数え上げは、呼び出し側 (core) が持つ (03 §9.5.1 の WATCH-1)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WatchId(pub u64);

/// ファイル監視の Port (02 §4.4)。メインスレッドから呼ぶ。どのメソッドも要求を積むだけで、OS を呼ばない。
pub trait FileWatcher {
    /// ディレクトリ `dir` を非再帰で監視する要求を積み、要求の識別を返す。
    ///
    /// 監視できる FS かの判定と OS への登録は、`io` のレーンの Host の作業で行う (05 §12.2 の 9)。結果は
    /// `FsEvent { watch, kind: Started }` か `FsEvent { watch, kind: Failed { .. } }` で 1 回だけ届く。`Started` より先に、
    /// 同じ識別の `Changed` などが届くことがある (登録の直後の変更。登録を終えた作業が `Started` を送るより先に、監視の
    /// スレッドが知らせを送り得る)。
    fn watch_dir(&mut self, dir: &FsPath) -> WatchId;

    /// 監視を外す要求を積む。結果は送らない。外した後に届いた知らせは、呼び出し側が捨てる。
    fn unwatch(&mut self, id: WatchId);
}

/// 監視の知らせ (`HostMsg::Fs` の中身)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsEvent {
    /// どの要求の知らせか。
    pub watch: WatchId,
    /// 何が起きたか。
    pub kind: FsEventKind,
}

/// 知らせの種類 (02 §4.4)。作成・変更・削除・改名を区別しない (core は stamp で照合する、03 §9.5.1 の WATCH-5)。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FsEventKind {
    /// 登録できた。
    Started,
    /// 登録できなかった (この識別では監視していない)。
    Failed {
        /// 理由。
        err: WatchError,
    },
    /// ディレクトリの直下の `name` の項目に何か起きた (作成・変更・削除・改名の元と先。読むだけの開閉は含めない)。
    Changed {
        /// 項目の名前 (ディレクトリの直下の 1 要素)。
        name: OsString,
    },
    /// 取りこぼしの恐れ (イベントのあふれ・監視の誤り)。ディレクトリの全文書を照合する。
    Rescan,
    /// 監視が失効した (監視中のディレクトリそのものの削除・改名・監視の読取の失敗。05 §12.2 の 10)。ディレクトリの
    /// 全文書を照合し、次の確かめで張り直す。
    Lost,
}

/// 登録できなかった理由 (02 §4.4)。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WatchError {
    /// 監視しない FS (05 §12.2 の 4)。
    Unsupported {
        /// FS の名前 (`"network"`・`"9p"`・`"exfat"` など。診断と知らせに使う)。
        fs: String,
    },
    /// OS の上限 (inotify の `max_user_watches` など)。
    Limit,
    /// 権限が無い。
    Denied,
    /// ディレクトリが無い。
    NotFound,
    /// そのほか (監視の仕組みを作れなかった・OS の誤り)。
    Other,
}

impl fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchError::Unsupported { fs } => write!(f, "unsupported file system ({fs})"),
            WatchError::Limit => f.write_str("watch limit reached"),
            WatchError::Denied => f.write_str("permission denied"),
            WatchError::NotFound => f.write_str("directory not found"),
            WatchError::Other => f.write_str("watch failed"),
        }
    }
}

impl std::error::Error for WatchError {}
