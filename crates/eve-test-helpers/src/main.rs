//! テスト用の補助プロセス (docs/02 §4.1、09 §3.6 の保存途中 kill)。`save` は実ファイルシステムで 1 回保存し、環境変数
//! `EVE_TEST_HOOK=<切断点>` を指していれば、その地点でプロセスを後始末なしで強制終了する (`eve-platform` の `test-hooks`)。
//!
//! `clip-delay --hold <ms>` はクリップボードを開いたまま保持し、`clip-delay --delay-render <ms> --text <文字列> [--lifetime <ms>]` は
//! 遅延レンダリングで登録して応答を遅らせる (準備ができたら標準出力に `ready` を出す)。`clip-read` は今のクリップボードのテキストを出す
//! (09 §3.4、実クリップボードの失敗と遅延の検査)。
//!
//! `console-event <pid> <ctrl-c|ctrl-break>` は、別プロセスのコンソールにコントロールイベントを送る (09 §3.6 (8))。
//!
//! 使い方: `eve-test-helpers save <state_dir> <path> <atomic|inplace|inplace-skip> <content_file>`
//! 終了コード: 0 = 保存した (`SaveReport` の method を標準出力へ)、3 = 保存の失敗 (`FsErrorKind` を標準エラーへ)、2 = 使い方の誤り、
//! 137 = 切断点で強制終了。

#![forbid(unsafe_code)]
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use eve_core::{
    CancelToken, ChunkError, Expect, FileStamp, FileSystem, FsPath, InPlaceBackup, ReadChunks,
    SaveOptions,
};
use eve_platform::fs::WinFs;

/// 内容を一定の大きさのチャンクで返す。
struct Chunks {
    data: Vec<u8>,
    pos: usize,
    size: usize,
}

impl ReadChunks for Chunks {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        let end = (self.pos + self.size).min(self.data.len());
        let c = &self.data[self.pos..end];
        self.pos = end;
        Ok(Some(c))
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("console-event") => return console_event(&args[1..]),
        Some("clip-delay") => return clip_delay(&args[1..]),
        Some("clip-read") => return clip_read(),
        _ => {}
    }
    let [cmd, state, path, mode, content] = args.as_slice() else {
        eprintln!(
            "使い方: eve-test-helpers save <state_dir> <path> <atomic|inplace|inplace-skip> <content_file>"
        );
        return ExitCode::from(2);
    };
    if cmd != "save" {
        eprintln!("未知のサブコマンド: {cmd}");
        return ExitCode::from(2);
    }
    let data = match std::fs::read(content) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("内容を読めません: {e}");
            return ExitCode::from(2);
        }
    };
    let fs = WinFs::new(state);
    let target = FsPath::from(path.as_str());
    let expected = fs.metadata(&target).map_or(Expect::Absent, |m| {
        Expect::Stamp(FileStamp::from_meta(&m, None))
    });
    let mut opts = SaveOptions::user_file(expected);
    let len = data.len() as u64;
    let mut src = Chunks {
        data,
        pos: 0,
        size: 4096,
    };
    let result = match mode.as_str() {
        "atomic" => fs.write_atomic(&target, &mut src, &opts),
        "inplace" | "inplace-skip" => {
            if mode == "inplace-skip" {
                opts.in_place_backup = InPlaceBackup::SkipConfirmed;
            }
            fs.write_in_place(&target, &mut src, len, &opts, &CancelToken::new())
        }
        other => {
            eprintln!("未知の方式: {other}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(r) => {
            println!("{:?}", r.method);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{:?}", e.kind);
            ExitCode::from(3)
        }
    }
}

/// `console-event <pid> <ctrl-c|ctrl-break>`: 0 = 送った、2 = 使い方の誤り、3 = 送れなかった。
fn console_event(args: &[String]) -> ExitCode {
    use eve_platform::console::{CtrlEvent, send_ctrl_event};
    let [pid, kind] = args else {
        eprintln!("使い方: eve-test-helpers console-event <pid> <ctrl-c|ctrl-break>");
        return ExitCode::from(2);
    };
    let (Ok(pid), ev) = (
        pid.parse::<u32>(),
        match kind.as_str() {
            "ctrl-c" => CtrlEvent::CtrlC,
            "ctrl-break" => CtrlEvent::CtrlBreak,
            _ => return ExitCode::from(2),
        },
    ) else {
        return ExitCode::from(2);
    };
    match send_ctrl_event(pid, ev) {
        Ok(()) => {
            // イベントがコンソールに配られるのを待ってから終わる
            std::thread::sleep(std::time::Duration::from_millis(300));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("送れません: {e}");
            ExitCode::from(3)
        }
    }
}

/// `clip-delay --hold <ms>` / `clip-delay --delay-render <ms> --text <文字列> [--lifetime <ms>]`。
fn clip_delay(args: &[String]) -> ExitCode {
    use eve_platform::clipboard_test::{hold, serve_delayed};
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let ready = || {
        println!("ready");
        use std::io::Write;
        let _ = std::io::stdout().flush();
    };
    if let Some(ms) = flag("--hold").and_then(|v| v.parse::<u64>().ok()) {
        // 開けたら `ready` を出して、開いたまま待つ。開けなければ失敗
        return if hold(ms, ready) {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(3)
        };
    }
    if let Some(delay) = flag("--delay-render").and_then(|v| v.parse::<u64>().ok()) {
        let text = flag("--text").cloned().unwrap_or_default();
        let lifetime = flag("--lifetime")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30_000);
        return if serve_delayed(&text, delay, lifetime, ready) {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(3)
        };
    }
    eprintln!(
        "使い方: eve-test-helpers clip-delay --hold <ms> | --delay-render <ms> --text <文字列> [--lifetime <ms>]"
    );
    ExitCode::from(2)
}

/// `clip-read`: 今のクリップボードのテキストを標準出力へ (無ければ空、終了コード 3)。
fn clip_read() -> ExitCode {
    match eve_platform::clipboard_test::read_text() {
        Some(t) => {
            print!("{t}");
            ExitCode::SUCCESS
        }
        None => ExitCode::from(3),
    }
}
