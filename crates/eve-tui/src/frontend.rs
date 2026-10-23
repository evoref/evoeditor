//! `Frontend` の TUI 実装 (docs/02 §9.2、docs/04 §14、Windows)。端末の初期化と復元、入力スレッドの起動、描画。
//!
//! 現在の範囲 (M2 の E5): 編集面・クローム・オーバーレイの描画。マウス・フォーカス・ブラケットペースト・拡張キーは
//! v0.1 では有効にしない (04 §14.1)。復元は冪等で、どの終了経路でも `restore` (または `Drop`) で行う。

use std::io;
use std::thread::JoinHandle;

use eve_host::{
    EffectOutcome, FatalReason, Frontend, FrontendConfig, FrontendError, HostMsg, InputSink,
    RenderOutcome,
};
use eve_view::{Capabilities, CellSize, Effect, Frame, InputEvent};

use crate::ansi::{ENTER, Emitter, LEAVE};
use crate::buffer::CHROME_ROWS;
use crate::caps::{CapsConfig, ColorMode, Detected, ProcessEnv, check_usable, detect};
use crate::chrome::render_screen;
use crate::glyphs::Glyphs;
use crate::win::{ConsoleInput, ConsoleOut, InputOut, spawn_input_thread, surface_resize};

/// `eve_host::InputSink` を入力スレッドの送り先にする。
struct SinkOut(InputSink);

impl InputOut for SinkOut {
    fn event(&self, ev: InputEvent) -> bool {
        self.0.send(ev)
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
    entered: bool,
    /// 「復元済み」の原子フラグ。`restore` と受信スレッドの緊急の復元が共有し、立てた側だけが復元の出力をする (02 §9.6 手順 4)。
    restored: eve_view::TermRestoredFlag,
}

fn err(what: &str, e: &io::Error) -> FrontendError {
    FrontendError(format!("{what}: {e}"))
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
            entered: false,
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

    fn window(&self) -> io::Result<CellSize> {
        match (&self.out, &self.input) {
            (Some(o), _) => o.window_size(),
            (None, Some(i)) => i.window_size(),
            _ => Err(io::Error::from(io::ErrorKind::NotConnected)),
        }
    }
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
            .map_err(|_| FrontendError("TERM=dumb の端末では起動できません".to_owned()))?;
        // 1〜2 コンソールモード (出力は VT、入力は自前のループ向け)。VT を拒否するコンソールは起動エラー
        let out =
            ConsoleOut::open().map_err(|e| err("コンソールの出力を VT モードにできません", &e))?;
        let input =
            ConsoleInput::open().map_err(|e| err("コンソールの入力を設定できません", &e))?;
        let ambiguous = matches!(cfg.ambiguous_width, 1 | 2).then_some(cfg.ambiguous_width);
        let detected = detect(
            CapsConfig {
                color_mode: self.color_mode,
                ambiguous_width: ambiguous,
            },
            &ProcessEnv,
            true,
        );
        self.glyphs = Glyphs::for_ascii_only(detected.caps.ascii_only);
        let caps = detected.caps.clone();
        self.detected = Some(detected);
        self.out = Some(out);
        self.input = Some(input);
        // 3〜4 代替スクリーンへ、続けて自動折り返しを無効化 (タイトルは ConsoleOut が退避済み)
        self.write(ENTER);
        self.entered = true;
        Ok(caps)
    }

    fn start_input(&mut self, tx: InputSink) -> Result<(), FrontendError> {
        let Some(input) = self.input.as_ref() else {
            return Err(FrontendError(
                "init の前に start_input が呼ばれました".to_owned(),
            ));
        };
        self.sink = Some(tx.clone());
        let t = spawn_input_thread(input, SinkOut(tx))
            .map_err(|e| err("入力スレッドを起動できません", &e))?;
        self.thread = Some(t);
        Ok(())
    }

    fn surface_size(&self) -> CellSize {
        match self.window() {
            Ok(w) => match surface_resize(w) {
                InputEvent::Resize { cols, rows } => CellSize { cols, rows },
                _ => CellSize::default(),
            },
            Err(_) => CellSize::default(),
        }
    }

    fn render(&mut self, frame: &Frame) -> RenderOutcome {
        // 編集面の大きさが Frame と違えば、出力せずに Resize を送る (04 §3.6。次の反復で正しい Frame が来る)
        let now = self.surface_size();
        if now != frame.size {
            self.emitter.invalidate();
            if let Some(s) = &self.sink {
                s.send(InputEvent::Resize {
                    cols: now.cols,
                    rows: now.rows,
                });
            }
            return RenderOutcome::Done;
        }
        let depth = self
            .detected
            .as_ref()
            .map_or(eve_view::ColorDepth::TrueColor, |d| d.caps.color_depth);
        let window = self.window().unwrap_or(frame.size);
        let r = render_screen(frame, window, depth, &self.glyphs);
        let out = self.emitter.frame(&r, frame, true);
        self.write(&out);
        // 計測用ビルド: 描画の `write` が戻った時刻を記録する。`--bench-exit-after-first-frame` なら、端末を復元して終わる
        let content = cfg!(feature = "bench-trace")
            && frame.panes.first().is_some_and(|p| {
                p.rows
                    .iter()
                    .any(|r| r.spans.iter().any(|s| !s.text.trim().is_empty()))
            });
        if crate::trace::render_done(content) {
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
            _ => EffectOutcome::Unhandled,
        }
    }

    fn emergency_restore_info(&self) -> Option<eve_host::EmergencyRestoreInfo> {
        let (out, input) = (self.out.as_ref()?, self.input.as_ref()?);
        // 保存したモード: 出力 (u32 LE) + 入力 (u32 LE)。`eve_platform::term::emergency_restore` が解釈する
        let mut mode = Vec::with_capacity(8);
        mode.extend_from_slice(&out.saved_mode().to_le_bytes());
        mode.extend_from_slice(&input.saved_mode().to_le_bytes());
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
                self.write(LEAVE);
            }
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
const EMERGENCY_BYTES: &[u8] = b"\x18\x1b[?2026l\x1b[0m\x1b[0 q\x1b[?25h\x1b[?7h\x1b[?1049l";

/// 編集面の下のクローム行数 (`surface_size` の計算に使う)。
pub const fn chrome_rows() -> u16 {
    CHROME_ROWS
}
