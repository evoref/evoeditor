//! 計測用ビルド (`bench-trace`) の区間の記録 (docs/09 §3.6 の「計測の記録」、01 §3.0.1)。
//!
//! 配布ビルドには `bench-trace` が無く、この module の関数はすべて何もしない (呼び出しは消える。環境変数名・隠しオプションの文字列も
//! 実行ファイルに入らない。`xtask check-dist` が確かめる)。計測用ビルドでも、環境変数 `EVE_TRACE_FILE=<パス>` があるときだけ記録する。
//!
//! 記録する区間 (JSON lines。1 行 1 span: `{"name","start_ns","end_ns","thread","fields"}`。`start_ns` / `end_ns` は
//! `main` 入口からの単調時計の ns。`fields` には文書の本文・パスを入れない):
//!
//! - `input.to_flush`: 入力スレッドが端末から入力のバイトを読み終えた時刻 → その入力による描画の `write` が戻った時刻
//!   (01 §3.0.1 の (b) プロセス内全体。NFR-103 の判定区間)。複数の入力が溜まったときは、最も古い読み終えの時刻から測る。
//! - `startup.first_frame`: `main` 入口 → 最初の Frame の `write` が戻った時刻 (起動 (a) プロセス内。NFR-101)。
//! - `startup.first_content`: 同じ始点 → 本文が見える最初の Frame の `write` が戻った時刻 (ファイルを開く起動。NFR-102)。
//!   どちらも `fields.wall_main_unix_ns` / `fields.wall_end_unix_ns` に、同じ 2 点の壁時計 (UNIX 紀元からの ns) を入れる
//!   (計測ハーネスの spawn 時刻と比べて、起動 (b) spawn からを求める)。
//!
//! `input.to_flush` の `fields` には、区間の内訳 (`sent_ns` = 入力をホストの受信口へ送り終えた時刻、`render_ns` = ホストが描画を
//! 呼んだ時刻 (core の処理と Frame 生成が済んだ)、`emit_ns` = 差分の出力の文字列ができた時刻。いずれも `start_ns` と同じ時間軸の ns)
//! も入る。ボトルネックの切り分け用で、判定には使わない。
//!
//! - `open.progress`: 直前のキー入力 (`fields.vk`。`Enter` = 0x0D) の読取完了 → 読込の進捗を含む最初の Frame の `write` が戻った時刻
//!   (01 §3.0.1 の「開く → 進捗」。NFR-104 / FR-122)。
//! - `resize.to_flush`: 最後のリサイズイベントの読取完了 → 次の Frame の `write` が戻った時刻 (「リサイズ → 再描画」。FR-512)。
//!
//! 書出しは `flush` (終了の直前) にまとめて行い、計測区間にファイルの I/O を含めない。隠しオプション
//! `--bench-exit-after-first-frame` は、最初の Frame の `write` が戻った直後に端末を復元して終了する。

#[cfg(feature = "bench-trace")]
mod imp {
    use std::fmt::Write as _;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    struct State {
        epoch: Instant,
        wall_main: u128,
        path: String,
    }

    static STATE: OnceLock<State> = OnceLock::new();
    static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    /// 最も古い、まだ描画に使われていない入力の読み終えの時刻 (ns+1。0 は無し)。
    static PENDING: AtomicU64 = AtomicU64::new(0);
    /// その入力のキー (仮想キーコード。計測ハーネスが区間を種類で選ぶための印で、入力した文字ではない)。
    static PENDING_VK: AtomicU64 = AtomicU64::new(0);
    /// 内訳の時刻 (ns+1。0 は無し)。最も古い入力に対する最初の値だけを残す。
    static SENT: AtomicU64 = AtomicU64::new(0);
    static RENDER: AtomicU64 = AtomicU64::new(0);
    static EMIT: AtomicU64 = AtomicU64::new(0);
    /// 直近のキー入力の読取完了 (ns+1) とそのキー。「開く → 進捗」の始点。進捗の Frame を書いたら外す。
    static LAST_READ: AtomicU64 = AtomicU64::new(0);
    static LAST_VK: AtomicU64 = AtomicU64::new(0);
    static PROGRESS_ARMED: AtomicBool = AtomicBool::new(false);
    /// 最後のリサイズイベントの読取完了 (ns+1)。
    static RESIZE: AtomicU64 = AtomicU64::new(0);
    static FIRST_DONE: AtomicBool = AtomicBool::new(false);
    static CONTENT_DONE: AtomicBool = AtomicBool::new(false);
    static EXIT_AFTER_FIRST: AtomicBool = AtomicBool::new(false);

    fn wall_ns() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    }

    pub fn init() {
        if let Ok(path) = std::env::var("EVE_TRACE_FILE") {
            let _ = STATE.set(State {
                epoch: Instant::now(),
                wall_main: wall_ns(),
                path,
            });
        }
    }

    fn now_ns(s: &State) -> u64 {
        s.epoch.elapsed().as_nanos() as u64
    }

    pub fn request_exit_after_first_frame() {
        EXIT_AFTER_FIRST.store(true, Ordering::Release);
    }

    pub fn input_read(first_key_down_vk: Option<u16>) {
        let Some(s) = STATE.get() else { return };
        if let Some(vk) = first_key_down_vk {
            LAST_READ.store(now_ns(s) + 1, Ordering::Release);
            LAST_VK.store(u64::from(vk), Ordering::Release);
            PROGRESS_ARMED.store(true, Ordering::Release);
        }
        if let Some(vk) = first_key_down_vk
            && PENDING
                .compare_exchange(0, now_ns(s) + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            PENDING_VK.store(u64::from(vk), Ordering::Release);
        }
    }

    fn mark(slot: &AtomicU64) {
        let Some(s) = STATE.get() else { return };
        if PENDING.load(Ordering::Acquire) != 0 {
            let _ = slot.compare_exchange(0, now_ns(s) + 1, Ordering::AcqRel, Ordering::Acquire);
        }
    }

    /// 入力をホストの受信口へ送り終えた。
    pub fn input_sent() {
        mark(&SENT);
    }

    /// ホストが描画を呼んだ (core の処理と Frame 生成が済んだ)。
    pub fn render_enter() {
        mark(&RENDER);
    }

    /// 差分の出力の文字列ができた (`write` の直前)。
    pub fn render_emitted() {
        mark(&EMIT);
    }

    fn push(line: String) {
        if let Ok(mut v) = LINES.lock() {
            v.push(line);
        }
    }

    /// リサイズイベントを読み終えた (最後のものだけを残す)。
    pub fn resize_read() {
        if let Some(s) = STATE.get() {
            RESIZE.store(now_ns(s) + 1, Ordering::Release);
        }
    }

    /// 描画の `write` が戻った直後。最初の Frame だけが `true` を返す (`--bench-exit-after-first-frame` が付いているとき)。
    /// `content`: この Frame で本文 (空白でない文字) が見える。`progress`: この Frame に読込の進捗が出ている。
    pub fn render_done(content: bool, progress: bool) -> bool {
        let Some(s) = STATE.get() else { return false };
        let end = now_ns(s);
        let thread = std::thread::current().name().unwrap_or("?").to_owned();
        let first = !FIRST_DONE.swap(true, Ordering::AcqRel);
        let first_content = content && !CONTENT_DONE.swap(true, Ordering::AcqRel);
        for (name, hit) in [
            ("startup.first_frame", first),
            ("startup.first_content", first_content),
        ] {
            if hit {
                let mut l = String::new();
                let _ = write!(
                    l,
                    "{{\"name\":\"{name}\",\"start_ns\":0,\"end_ns\":{end},\"thread\":\"{thread}\",\"fields\":{{\"wall_main_unix_ns\":{},\"wall_end_unix_ns\":{}}}}}",
                    s.wall_main,
                    wall_ns()
                );
                push(l);
            }
        }
        if progress && PROGRESS_ARMED.swap(false, Ordering::AcqRel) {
            let start = LAST_READ.load(Ordering::Acquire).saturating_sub(1);
            let vk = LAST_VK.load(Ordering::Acquire);
            push(format!(
                "{{\"name\":\"open.progress\",\"start_ns\":{start},\"end_ns\":{end},\"thread\":\"{thread}\",\"fields\":{{\"vk\":{vk}}}}}"
            ));
        }
        let resize = RESIZE.swap(0, Ordering::AcqRel);
        if resize != 0 {
            push(format!(
                "{{\"name\":\"resize.to_flush\",\"start_ns\":{},\"end_ns\":{end},\"thread\":\"{thread}\",\"fields\":{{}}}}",
                resize - 1
            ));
        }
        let start = PENDING.swap(0, Ordering::AcqRel);
        if start != 0 {
            let vk = PENDING_VK.load(Ordering::Acquire);
            let t = |slot: &AtomicU64| slot.swap(0, Ordering::AcqRel).saturating_sub(1);
            let (sent, render, emit) = (t(&SENT), t(&RENDER), t(&EMIT));
            push(format!(
                "{{\"name\":\"input.to_flush\",\"start_ns\":{},\"end_ns\":{end},\"thread\":\"{thread}\",\"fields\":{{\"vk\":{vk},\"sent_ns\":{sent},\"render_ns\":{render},\"emit_ns\":{emit}}}}}",
                start - 1
            ));
        }
        first && EXIT_AFTER_FIRST.load(Ordering::Acquire)
    }

    pub fn flush() {
        let Some(s) = STATE.get() else { return };
        let Ok(mut v) = LINES.lock() else { return };
        if v.is_empty() {
            return;
        }
        let mut text = v.join("\n");
        text.push('\n');
        v.clear();
        let _ = std::fs::write(&s.path, text);
    }
}

#[cfg(not(feature = "bench-trace"))]
mod imp {
    #[inline(always)]
    pub fn init() {}
    #[inline(always)]
    pub fn input_read(_first_key_down_vk: Option<u16>) {}
    #[inline(always)]
    pub fn input_sent() {}
    #[inline(always)]
    pub fn render_enter() {}
    #[inline(always)]
    pub fn render_emitted() {}
    #[inline(always)]
    pub fn render_done(_content: bool, _progress: bool) -> bool {
        false
    }
    #[inline(always)]
    pub fn resize_read() {}
    #[inline(always)]
    pub fn flush() {}
}

pub use imp::*;
