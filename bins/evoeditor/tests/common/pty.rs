//! 実バイナリを擬似端末 (portable-pty。Windows は ConPTY、Linux は openpty) で起こす部品。L5 の E2E (`tests/e2e.rs`) と
//! 計測 (`benches/bm_pty.rs`、`#[path]` で取り込む) が共有する (docs/09 §3.6)。画面の解釈と待ち方は使う側が持つ。

// 使う側ごとに使わない項目がある (E2E は spawn の時刻を見ない)
#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Instant, SystemTime};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// 起こした子と、その端末の口。
pub struct Pty {
    pub child: Box<dyn Child + Send + Sync>,
    /// 子への入力 (読み取りスレッドも DSR の応答に使う)。
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// pty を閉じないよう保持する (手放すと端末が閉じる。Windows は `CTRL_CLOSE_EVENT` が起きる)。
    pub master: Box<dyn MasterPty + Send>,
    /// spawn の直前の時刻 (計測の始点)。
    pub spawned_at: Instant,
    /// 同じく壁時計。
    pub spawned_wall: SystemTime,
}

/// 子のコマンド。端末の種類の判定を揺らす環境変数を外し、状態ディレクトリ `state` と UI の言語 (日本語) を固定する
/// (利用者の状態ディレクトリを汚さず、OS の言語に依らない)。設定ディレクトリと他の環境変数は使う側が足す。
pub fn command(exe: &str, args: &[&str], state: &Path) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(exe);
    cmd.args(args);
    cmd.env_remove("NO_COLOR");
    cmd.env_remove("WT_SESSION");
    cmd.env_remove("TERM_PROGRAM");
    cmd.env("EVE_STATE_DIR", state);
    cmd.env("EVE_LANG", "ja");
    cmd
}

/// `cols` × `rows` の擬似端末で `cmd` を起こし、出力を読み取りスレッドで `on_chunk` に渡す。
/// `ConPTY` は起動時にカーソル位置を尋ねる (DSR) ので、`on_chunk` の前に端末として答える。
pub fn spawn(
    cmd: CommandBuilder,
    cols: u16,
    rows: u16,
    buf_size: usize,
    mut on_chunk: impl FnMut(&[u8]) + Send + 'static,
) -> Pty {
    let pty = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("擬似端末を開けません");
    let mut reader = pty.master.try_clone_reader().expect("reader");
    let writer: Arc<Mutex<Box<dyn Write + Send>>> =
        Arc::new(Mutex::new(pty.master.take_writer().expect("writer")));
    let spawned_at = Instant::now();
    let spawned_wall = SystemTime::now();
    let child = pty.slave.spawn_command(cmd).expect("起動できません");
    drop(pty.slave);
    let answer = writer.clone();
    // テストの駆動側の読み取りスレッド (製品のスレッドではない。R11 の対象外)
    #[allow(clippy::disallowed_methods)]
    thread::spawn(move || {
        let mut buf = vec![0u8; buf_size];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            let chunk = &buf[..n];
            if chunk.windows(4).any(|w| w == b"\x1b[6n")
                && let Ok(mut w) = answer.lock()
            {
                let _ = w.write_all(b"\x1b[1;1R");
                let _ = w.flush();
            }
            on_chunk(chunk);
        }
    });
    Pty {
        child,
        writer,
        master: pty.master,
        spawned_at,
        spawned_wall,
    }
}

/// 端末の画面の解釈 (vt100)。
pub fn parser(cols: u16, rows: u16) -> Arc<Mutex<vt100::Parser>> {
    Arc::new(Mutex::new(vt100::Parser::new(
        std::num::NonZeroU16::new(rows).expect("rows は 1 以上"),
        std::num::NonZeroU16::new(cols).expect("cols は 1 以上"),
        0,
    )))
}
