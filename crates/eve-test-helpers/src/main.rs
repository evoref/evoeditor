//! テスト用の補助プロセス (docs/02 §4.1、09 §3.6 の保存途中 kill)。`save` は実ファイルシステムで 1 回保存し、環境変数
//! `EVE_TEST_HOOK=<切断点>` を指していれば、その地点でプロセスを後始末なしで強制終了する (`eve-platform` の `test-hooks`)。
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
