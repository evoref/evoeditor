//! `Frontend` の TUI 実装 (docs/02 §9.2、docs/04 §14)。端末の初期化と復元、入力スレッドの起動、描画。OS ごとの端末の部品は
//! `input::win` (Windows のコンソール) と `input::unix` (`/dev/tty`) で、同じ形 (`ConsoleInput`・`ConsoleOut`・`spawn_input_thread`) をしている。
//!
//! 編集面・クローム・オーバーレイを描画する。復元は冪等で、どの終了経路でも `restore` (または `Drop`) で行う。

use std::io;
use std::thread::JoinHandle;

use eve_host::{
    EffectOutcome, FatalReason, Frontend, FrontendConfig, FrontendError, FrontendErrorKind,
    HostMsg, InputSink, RenderOutcome,
};
use eve_view::{Capabilities, CellSize, Effect, Frame, InputEvent};

use crate::ansi::{ENTER, Emitter, LEAVE};
use crate::buffer::{CHROME_ROWS, bar_rows};
use crate::caps::{CapsConfig, ColorMode, Detected, ProcessEnv, check_usable, detect};
use crate::chrome::{ScreenLayout, render_screen};
use crate::glyphs::Glyphs;
use crate::input::InputOut;
#[cfg(unix)]
use crate::input::unix as term_io;
#[cfg(windows)]
use crate::input::win as term_io;
use term_io::{ConsoleInput, ConsoleOut, spawn_input_thread};

/// `eve_host::InputSink` を入力スレッドの送り先にする。
struct SinkOut(InputSink);

impl InputOut for SinkOut {
    fn event(&self, ev: InputEvent) -> bool {
        let ok = self.0.send(ev);
        crate::trace::input_sent();
        ok
    }

    fn died(&self) {
        self.0
            .send_msg(HostMsg::Fatal(FatalReason::InputThreadDied));
    }
}

pub struct TuiFrontend {
    color_mode: ColorMode,
    detected: Option<Detected>,
    input: Option<ConsoleInput>,
    out: Option<ConsoleOut>,
    sink: Option<InputSink>,
    emitter: Emitter,
    glyphs: Glyphs,
    thread: Option<JoinHandle<()>>,
    /// 描画が入力スレッドへ渡す編集面の位置 (マウスの座標変換、04 §6.8)。
    geom: std::sync::Arc<crate::mouse::SharedGeom>,
    entered: bool,
    /// Unix: 端末タイトルを XTWINOPS で退避した (`CSI 22;0 t`)。復元で 1 回だけ戻す (`CSI 23;0 t`、04 §14.1 / §14.3)。
    /// 外から止められた (SIGCONT だけが来る) 再初期化では退避し直さない (退避と復元を釣り合わせる)。
    #[cfg(unix)]
    title_pushed: bool,
    /// マウスの捕捉 (`SetMouseCapture`)。Unix の SIGCONT の後の再初期化で入れ直す。
    mouse: bool,
    /// 「復元済み」の原子フラグ。`restore` と受信スレッドの緊急の復元が共有し、立てた側だけが復元の出力をする (02 §9.6 手順 4)。
    restored: eve_view::TermRestoredFlag,
}

fn err(kind: FrontendErrorKind, e: &io::Error) -> FrontendError {
    FrontendError::io(kind, e)
}

impl TuiFrontend {
    pub fn new() -> TuiFrontend {
        TuiFrontend::with_color_mode(ColorMode::Auto)
    }

    /// `theme.color_mode` (07 §5.4) を指定して作る。
    pub fn with_color_mode(color_mode: ColorMode) -> TuiFrontend {
        TuiFrontend {
            color_mode,
            detected: None,
            input: None,
            out: None,
            sink: None,
            emitter: Emitter::new(),
            glyphs: Glyphs::unicode(),
            thread: None,
            geom: Default::default(),
            entered: false,
            #[cfg(unix)]
            title_pushed: false,
            mouse: false,
            restored: eve_view::TermRestoredFlag::default(),
        }
    }

    /// 判定した能力 (`init` の後)。
    pub fn detected(&self) -> Option<&Detected> {
        self.detected.as_ref()
    }

    fn write(&mut self, s: &str) {
        if let Some(o) = self.out.as_mut() {
            // 端末を失った (ウィンドウを閉じた等) ときの書込の失敗は無視する (04 §14.2)
            let _ = o.write_str(s);
        }
    }

    /// 代替スクリーンと端末のモードを入れる (初期化と、Unix の SIGCONT の後の再初期化)。
    fn enter_screen(&mut self) {
        // 04 §14.1 の 3: 代替スクリーンの前にタイトルを退避する (非対応の端末は無視する)
        #[cfg(unix)]
        if !self.title_pushed {
            self.title_pushed = true;
            self.write(crate::input::unix::TITLE_PUSH);
        }
        self.write(ENTER);
        #[cfg(unix)]
        self.write(crate::input::unix::ENTER_MODES);
        self.entered = true;
    }

    fn set_mouse(&mut self, on: bool) {
        self.mouse = on;
        if let Some(i) = self.input.as_ref() {
            i.set_mouse(on);
        }
    }

    /// Unix: 外から止められて `fg` で再開した (SIGCONT) なら、raw モード・代替スクリーン・各モードを入れ直し、次の Frame を
    /// 全面の描画にする (05 §11.1 の SIGCONT の行)。
    #[cfg(unix)]
    fn reinit_if_continued(&mut self) {
        let Some(i) = self.input.as_mut() else {
            return;
        };
        if !i.take_reinit() {
            return;
        }
        // 先にフラグを下ろす (raw に戻した直後に異常終了しても、ハンドラが端末を戻せるように)
        self.restored
            .0
            .store(false, std::sync::atomic::Ordering::Release);
        i.reenter_raw();
        self.enter_screen();
        let mouse = self.mouse;
        self.set_mouse(mouse);
        self.emitter.invalidate();
    }

    /// Unix: `app.suspend` (05 §11.3)。端末を復元してから自分のプロセスグループに SIGTSTP を送って止まる。`fg` で再開したら (SIGCONT)、
    /// 再初期化の印を立てる (入力スレッドも SIGCONT で立てる。`wake_after` が描画を起こし、`reinit_if_continued` が入れ直す)。
    #[cfg(unix)]
    fn suspend(&mut self) {
        self.restore();
        crate::input::unix::stop_job();
        if let Some(i) = self.input.as_ref() {
            i.request_reinit();
        }
    }

    fn window(&self) -> io::Result<CellSize> {
        match (&self.out, &self.input) {
            (Some(o), _) => o.window_size(),
            (None, Some(i)) => i.window_size(),
            _ => Err(io::Error::from(io::ErrorKind::NotConnected)),
        }
    }
}

/// 端末を使えるかだけを確かめる (`TERM=dumb` でない、入力・描画の端末を開ける)。モードは変えない。`Frontend::init` の
/// 失敗のうち、端末を変える前に分かるものを先に見る。標準入力 (`evoeditor -`) を移して読み始める前に Host (bin) が呼び、
/// 失敗したら標準入力を読まずに `init` の失敗と同じ経路 (終了コード 5) で終わる (03 §9.2.2 の「端末の初期化の時期」)。
/// 端末を開けても、VT への切替・raw モードの設定は `init` で失敗し得る (そのときは読んだ分を失う。既知の制約)。
///
/// # Errors
///
/// `init` と同じ種類の `FrontendError` (`UnusableTerminal` / `ConsoleOutput` / `ConsoleInput`)。
pub fn probe_terminal() -> Result<(), FrontendError> {
    check_usable(&ProcessEnv)
        .map_err(|_| FrontendError::new(FrontendErrorKind::UnusableTerminal))?;
    #[cfg(unix)]
    crate::input::unix::probe_tty().map_err(|e| err(FrontendErrorKind::ConsoleInput, &e))?;
    #[cfg(windows)]
    crate::input::win::probe_console().map_err(|(input, e)| {
        let kind = if input {
            FrontendErrorKind::ConsoleInput
        } else {
            FrontendErrorKind::ConsoleOutput
        };
        err(kind, &e)
    })?;
    Ok(())
}

impl Default for TuiFrontend {
    fn default() -> TuiFrontend {
        TuiFrontend::new()
    }
}

impl Frontend for TuiFrontend {
    fn init(&mut self, cfg: &FrontendConfig) -> Result<Capabilities, FrontendError> {
        // 0 前提確認: TERM=dumb は起動しない
        check_usable(&ProcessEnv)
            .map_err(|_| FrontendError::new(FrontendErrorKind::UnusableTerminal))?;
        // 1〜2 コンソールモード (出力は VT、入力は自前のループ向け)。VT を拒否するコンソールは起動エラー
        let out = ConsoleOut::open().map_err(|e| err(FrontendErrorKind::ConsoleOutput, &e))?;
        let input = ConsoleInput::open().map_err(|e| err(FrontendErrorKind::ConsoleInput, &e))?;
        let ambiguous = matches!(cfg.ambiguous_width, 1 | 2).then_some(cfg.ambiguous_width);
        let detected = detect(
            CapsConfig {
                color_mode: self.color_mode,
                ambiguous_width: ambiguous,
            },
            &ProcessEnv,
            cfg!(windows),
        );
        self.glyphs = Glyphs::for_ascii_only(detected.caps.ascii_only);
        let caps = detected.caps.clone();
        self.detected = Some(detected);
        self.out = Some(out);
        self.input = Some(input);
        // 3〜4 代替スクリーンへ、続けて自動折り返しを無効化 (タイトルは ConsoleOut が退避済み)。Unix は続けて
        // ブラケットペースト・フォーカス報告・kitty の push を入れる
        self.enter_screen();
        self.set_mouse(cfg.mouse);
        Ok(caps)
    }

    fn start_input(&mut self, tx: InputSink) -> Result<(), FrontendError> {
        let Some(input) = self.input.as_ref() else {
            return Err(FrontendError::new(FrontendErrorKind::InputBeforeInit));
        };
        self.sink = Some(tx.clone());
        let t = spawn_input_thread(input, SinkOut(tx), self.geom.clone())
            .map_err(|e| err(FrontendErrorKind::InputThread, &e))?;
        self.thread = Some(t);
        Ok(())
    }

    fn surface_size(&self) -> CellSize {
        match self.window() {
            Ok(w) => ScreenLayout::compute(w, self.geom.hint_bar(), 0).surface(),
            Err(_) => CellSize::default(),
        }
    }

    fn render(&mut self, frame: &Frame) -> RenderOutcome {
        // 編集面の大きさが Frame と違えば、出力せずに Resize を送る (04 §3.6。次の反復で正しい Frame が来る)
        // Frame の編集面は、検索バーが開いていればその行数だけ端末の編集面より小さい (`ScreenLayout::frame_size`)。
        // ヒントバーの表示が変われば (FR-504) クロームの行数が変わるので、先に覚えてから比べる (変わった直後の 1 回は Resize になる)
        #[cfg(unix)]
        self.reinit_if_continued();
        self.geom.set_hint_bar(frame.hint_bar);
        // 大きさを取れなければ 0×0 (`surface_size` と同じ)
        let window = self.window().unwrap_or_default();
        let layout = ScreenLayout::compute(window, self.geom.hint_bar(), bar_rows(frame));
        let now = layout.surface();
        if layout.frame_size() != frame.size {
            self.emitter.invalidate();
            if let Some(s) = &self.sink {
                s.send(InputEvent::Resize {
                    cols: now.cols,
                    rows: now.rows,
                });
            }
            return RenderOutcome::Done;
        }
        if let Some(p) = frame.panes.first() {
            self.geom.set(p.view, p.rect.rows);
        }
        crate::trace::render_enter();
        let depth = self
            .detected
            .as_ref()
            .map_or(eve_view::ColorDepth::TrueColor, |d| d.caps.color_depth);
        let window = self.window().unwrap_or(frame.size);
        let r = render_screen(frame, window, depth, &self.glyphs);
        let out = self.emitter.frame(r, frame, true);
        crate::trace::render_emitted();
        self.write(&out);
        // 計測用ビルド: 描画の `write` が戻った時刻を記録する。`--bench-exit-after-first-frame` なら、端末を復元して終わる
        let content = cfg!(feature = "bench-trace")
            && frame.panes.first().is_some_and(|p| {
                p.rows
                    .iter()
                    .any(|r| r.spans.iter().any(|s| !s.text.trim().is_empty()))
            });
        let progress = cfg!(feature = "bench-trace")
            && frame
                .status
                .items
                .iter()
                .any(|i| i.kind == eve_view::StatusKind::Loading);
        // 検索で一致へ移った: 計測ハーネスが決めた語が編集面の行に出ている (BM-108)
        let matched = crate::trace::match_text().is_some_and(|m| {
            frame.panes.first().is_some_and(|p| {
                p.rows.iter().any(|r| {
                    r.spans
                        .iter()
                        .map(|s| s.text.as_str())
                        .collect::<String>()
                        .contains(m)
                })
            })
        });
        if crate::trace::render_done(content, progress, matched) {
            self.restore();
            crate::trace::flush();
            std::process::exit(0);
        }
        RenderOutcome::Done
    }

    fn apply_effect(&mut self, eff: &Effect) -> EffectOutcome {
        match eff {
            Effect::Bell => {
                self.write("\x07");
                EffectOutcome::Handled
            }
            Effect::SetMouseCapture { enabled } => {
                self.set_mouse(*enabled);
                EffectOutcome::Handled
            }
            Effect::Redraw => {
                // 次の Frame を全画面の再描画にする (差分の基準を捨てる)
                self.emitter.invalidate();
                EffectOutcome::Handled
            }
            #[cfg(unix)]
            Effect::Suspend => {
                self.suspend();
                EffectOutcome::Handled
            }
            _ => EffectOutcome::Unhandled,
        }
    }

    /// Unix: SIGCONT の後の再初期化が要るときは、すぐに描画を起こす (大きさの変わらない `Resize` だけでは描画が起きないため。
    /// 描画の先頭の `reinit_if_continued` が端末を入れ直して全面を描く)。
    fn wake_after(&self) -> Option<std::time::Duration> {
        #[cfg(unix)]
        if self.input.as_ref().is_some_and(|i| i.reinit_pending()) {
            return Some(std::time::Duration::ZERO);
        }
        None
    }

    /// 標準入力を読み終えたとき (03 §9.2.2): 端末のモードを入れ直して、次の描画を全面にする。Unix は SIGCONT の
    /// 再初期化と同じ印を立てる (`reinit_if_continued` が raw モード・代替スクリーン・各モード・マウスを入れ直す)。
    /// Windows はコンソールの入出力のモードを入れ直し、代替スクリーンとマウスを入れ直す。
    fn reinit_terminal(&mut self) {
        #[cfg(unix)]
        if let Some(i) = self.input.as_ref() {
            i.request_reinit();
        }
        #[cfg(windows)]
        if self.entered {
            if let Some(o) = self.out.as_mut() {
                o.reapply_mode();
            }
            if let Some(i) = self.input.as_mut() {
                i.reapply_mode();
            }
            self.enter_screen();
            let mouse = self.mouse;
            self.set_mouse(mouse);
            self.emitter.invalidate();
        }
    }

    fn write_osc52(&mut self, text: &str) -> bool {
        use crate::caps::Env;
        if !self.detected.as_ref().is_some_and(|d| d.caps.osc52_write) {
            return false;
        }
        let tmux = ProcessEnv.var("TMUX").is_some();
        self.write(&crate::ansi::osc52(text, tmux));
        true
    }

    fn emergency_restore_info(&self) -> Option<eve_host::EmergencyRestoreInfo> {
        let (out, input) = (self.out.as_ref()?, self.input.as_ref()?);
        // 保存したモード。`eve_platform::term::emergency_restore` が解釈する: Windows は出力 (u32 LE) + 入力 (u32 LE)、
        // Unix は起動時の termios
        #[cfg(windows)]
        let mode = {
            let mut m = Vec::with_capacity(8);
            m.extend_from_slice(&out.saved_mode().to_le_bytes());
            m.extend_from_slice(&input.saved_mode().to_le_bytes());
            m
        };
        #[cfg(unix)]
        let mode = input.saved_mode();
        Some(eve_host::EmergencyRestoreInfo {
            handle: eve_view::RawTermHandle(out.raw_handle()),
            bytes: EMERGENCY_BYTES,
            saved_mode: eve_view::SavedTermMode(mode.into_boxed_slice()),
            restored: self.restored.clone(),
        })
    }

    fn restore(&mut self) {
        // 04 §14.3: ESU → SGR リセット → カーソル → 自動折り返し → 代替スクリーン。続けてタイトルとコンソールモード。
        // 何度呼んでも安全で、失敗しても次を試す
        if self.entered {
            self.entered = false;
            // 緊急の復元が先に出力していたら、二重に出さない
            if !self
                .restored
                .0
                .swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                #[cfg(unix)]
                self.write(crate::input::unix::LEAVE_MODES);
                self.write(LEAVE);
            }
        }
        // 04 §14.3 の 8: タイトルを戻す (代替スクリーンを抜けた後。緊急の復元が先に戻していても、pop は退避の数だけ)
        #[cfg(unix)]
        if self.title_pushed {
            self.title_pushed = false;
            self.write(crate::input::unix::TITLE_POP);
        }
        if let Some(o) = self.out.as_mut() {
            o.restore();
        }
        if let Some(i) = self.input.as_mut() {
            i.restore();
        }
    }
}

impl TuiFrontend {
    /// 端末を復元する (`Frontend::restore` と同じ。呼び側が trait を import せずに呼べる)。冪等。
    pub fn restore_terminal(&mut self) {
        <TuiFrontend as Frontend>::restore(self);
    }
}

impl Drop for TuiFrontend {
    fn drop(&mut self) {
        self.restore();
    }
}

/// 緊急の端末復元で書くバイト列: 先頭の CAN (書きかけの VT シーケンスを打ち切る) + 復元の固定の列 (04 §14.3)。
#[cfg(windows)]
const EMERGENCY_BYTES: &[u8] = b"\x18\x1b[?2026l\x1b[0m\x1b[0 q\x1b[?25h\x1b[?7h\x1b[?1049l";
/// 同上 (Unix は先に `unix::LEAVE_MODES` と同じモードを外し、最後に `unix::TITLE_POP` でタイトルを戻す)。
#[cfg(unix)]
const EMERGENCY_BYTES: &[u8] = b"\x18\x1b[>4m\x1b[<u\x1b[?1004l\x1b[?2004l\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?2026l\x1b[0m\x1b[0 q\x1b[?25h\x1b[?7h\x1b[?1049l\x1b[23;0t";

/// 編集面の下のクローム行数 (`surface_size` の計算に使う)。
pub const fn chrome_rows() -> u16 {
    CHROME_ROWS
}

/// 入力スレッドの本体 `body` を走らせ、panic したら `on_panic` を呼ぶ (Windows と Unix の入力スレッドで共有する)。
///
/// 入力スレッドが panic したまま黙って終わると、端末は raw のまま入力を受け付けず、退避も走らない。`on_panic` で
/// Host に `Fatal(InputThreadDied)` を送り、Host が退避と端末の復元をして終える (04 §14.2・05 §11.4。
/// docs/review/v0.3-findings.md の SAF-07)。panic の内容はパニックフック (bin) が記録する。
pub(crate) fn run_guarded(body: impl FnOnce(), on_panic: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).is_err() {
        on_panic();
    }
}

#[cfg(test)]
mod tests {
    use super::run_guarded;
    use std::cell::Cell;

    #[test]
    fn 入力スレッドの本体が_panic_したら知らせる() {
        let died = Cell::new(false);
        run_guarded(|| panic!("入力の解析の不具合"), || died.set(true));
        assert!(died.get());
    }

    #[test]
    fn 本体が普通に終われば知らせない() {
        let died = Cell::new(false);
        run_guarded(|| {}, || died.set(true));
        assert!(!died.get());
    }

    /// 入力スレッドの panic を、本物の送り先 (`SinkOut`) と Host の待ち受けのループまで通す (SAF-07、
    /// docs/review/v0.3-findings.md の §2.7 の 6)。Frontend は `HeadlessFrontend` に任せ、`start_input` だけを
    /// 「panic する入力スレッドの本体」に差し替える (`run_guarded` は入力スレッドの本体そのもの。スレッドを作るのは
    /// R11 で win.rs / unix.rs だけなので、ここでは同じスレッドで走らせる)。Host は `Fatal(InputThreadDied)` を受けて、
    /// 端末を 1 回だけ復元して FATAL で終わる。
    #[test]
    fn 入力スレッドが_panic_すると_host_が受けて端末を復元して終わる() {
        use std::rc::Rc;
        use std::sync::Arc;

        use eve_core::{JobPorts, Ports};
        use eve_host::{
            EffectOutcome, ExitCode, Frontend, FrontendConfig, FrontendError, Host, InputSink,
            RenderOutcome,
        };
        use eve_testkit::{FixedPaths, HeadlessFrontend, ManualClock, ManualJobSpawner, MemFs};
        use eve_view::{Capabilities, CellSize, Effect, Frame};

        struct PanickingInput {
            inner: HeadlessFrontend,
            died: bool,
        }

        impl Frontend for PanickingInput {
            fn init(&mut self, cfg: &FrontendConfig) -> Result<Capabilities, FrontendError> {
                self.inner.init(cfg)
            }
            fn start_input(&mut self, tx: InputSink) -> Result<(), FrontendError> {
                use crate::input::InputOut as _;
                let out = super::SinkOut(tx);
                let mut died = false;
                run_guarded(
                    || panic!("入力の解析の不具合 (試験)"),
                    || {
                        out.died();
                        died = true;
                    },
                );
                self.died = died;
                Ok(())
            }
            fn surface_size(&self) -> CellSize {
                self.inner.surface_size()
            }
            fn render(&mut self, frame: &Frame) -> RenderOutcome {
                self.inner.render(frame)
            }
            fn apply_effect(&mut self, eff: &Effect) -> EffectOutcome {
                self.inner.apply_effect(eff)
            }
            fn restore(&mut self) {
                self.inner.restore();
            }
        }

        let clock = ManualClock::new();
        let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
        fs.add_dir("/w");
        let paths = Arc::new(FixedPaths::default());
        let jobs = ManualJobSpawner::new(JobPorts {
            stdin: None,
            fs: fs.clone(),
            clock: Arc::new(clock.clone()),
            paths: paths.clone(),
        });
        let ports = Ports {
            fs,
            clock: Arc::new(clock),
            jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
            paths,
        };
        let inner = HeadlessFrontend::new(40, 8);
        let probe = inner.probe();
        let mut fe = PanickingInput { inner, died: false };
        let mut host = Host::new(ports);
        let code = host.run(&mut fe, &[], false);
        assert_eq!(code, ExitCode::FATAL);
        assert_eq!(probe.restore_count(), 1, "端末を 1 回だけ復元する");
        assert!(fe.died, "panic は run_guarded の中で捕まえて died を呼ぶ");
    }

    /// 緊急の復元の列は、通常の復元 (`ansi::LEAVE`、Unix は先に `unix::LEAVE_MODES`、最後に `unix::TITLE_POP`) と同じ並びに
    /// CAN を前置したもの (04 §14.3。docs/review/v0.3-findings.md の STR-T9)。片方だけを直すとずれるので、ここで突き合わせる。
    #[test]
    fn 緊急の復元の列は通常の復元と同じ並び() {
        #[cfg(windows)]
        let expected = format!("\x18{}", crate::ansi::LEAVE);
        #[cfg(unix)]
        let expected = format!(
            "\x18{}{}{}",
            crate::input::unix::LEAVE_MODES,
            crate::ansi::LEAVE,
            crate::input::unix::TITLE_POP
        );
        assert_eq!(super::EMERGENCY_BYTES, expected.as_bytes());
    }
}
