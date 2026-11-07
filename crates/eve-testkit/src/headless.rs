//! `HeadlessFrontend`: 端末を持たない `Frontend`。描いた Frame を記録し、入力は `InputSink` 経由で外から送る。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eve_host::{EffectOutcome, Frontend, FrontendConfig, FrontendError, InputSink, RenderOutcome};
use eve_view::{Capabilities, CellSize, Effect, Frame};

use crate::ascii::{AsciiOptions, render_ascii};

#[derive(Debug, Default)]
struct Shared {
    frames: Vec<Frame>,
    restored: u32,
    sink: Option<InputSink>,
    osc52: Vec<String>,
}

/// テストから観察する口 (Host に渡した後も持てる)。
#[derive(Clone, Debug, Default)]
pub struct HeadlessProbe(Arc<Mutex<Shared>>);

impl HeadlessProbe {
    /// 今までに `render` された回数。
    ///
    /// # Panics
    ///
    /// 内部の `Mutex` が汚れている (別スレッドがロック中にパニックした) とき。
    pub fn render_count(&self) -> usize {
        self.0.lock().unwrap().frames.len()
    }

    /// 最後に描かれた `Frame`。まだ描かれていなければ `None`。
    ///
    /// # Panics
    ///
    /// 内部の `Mutex` が汚れているとき。
    pub fn last_frame(&self) -> Option<Frame> {
        self.0.lock().unwrap().frames.last().cloned()
    }

    /// 最後に描かれた `Frame` の ASCII 表現 (`render_ascii`)。まだ描かれていなければ `None`。
    ///
    /// # Panics
    ///
    /// 内部の `Mutex` が汚れているとき。
    pub fn last_ascii(&self, opts: AsciiOptions) -> Option<String> {
        self.last_frame().map(|f| render_ascii(&f, opts))
    }

    /// `start_input` で渡された入力口 (別スレッドから入力を送る用。起動前は `None`)。
    ///
    /// # Panics
    ///
    /// 内部の `Mutex` が汚れているとき。
    pub fn sink(&self) -> Option<InputSink> {
        self.0.lock().unwrap().sink.clone()
    }

    /// `write_osc52` で書かれた本文 (古い順)。
    ///
    /// # Panics
    ///
    /// 内部の `Mutex` が汚れているとき。
    pub fn osc52_writes(&self) -> Vec<String> {
        self.0.lock().unwrap().osc52.clone()
    }

    /// `restore` が呼ばれた回数 (終了経路で 1 回だけ呼ばれるはず)。
    ///
    /// # Panics
    ///
    /// 内部の `Mutex` が汚れているとき。
    pub fn restore_count(&self) -> u32 {
        self.0.lock().unwrap().restored
    }
}

/// 端末を持たない `Frontend`。`render` された `Frame` を記録し、`Effect` は処理しない (`Unhandled`)。
/// 記録は `probe` で取った `HeadlessProbe` から読む。
pub struct HeadlessFrontend {
    size: CellSize,
    caps: Capabilities,
    shared: HeadlessProbe,
    wake: Option<Duration>,
}

impl HeadlessFrontend {
    /// `cols` × `rows` セルの編集面を持つ frontend を作る (能力は `Capabilities::default()`)。
    pub fn new(cols: u16, rows: u16) -> HeadlessFrontend {
        HeadlessFrontend {
            size: CellSize { cols, rows },
            caps: Capabilities::default(),
            shared: HeadlessProbe::default(),
            wake: None,
        }
    }

    /// 観察の口。Host に frontend を渡した後も持てる。
    pub fn probe(&self) -> HeadlessProbe {
        self.shared.clone()
    }

    /// 能力を差し替える (`init` が返す値)。
    pub fn with_caps(mut self, caps: Capabilities) -> HeadlessFrontend {
        self.caps = caps;
        self
    }

    /// `wake_after` が返す値を決める (`None` で要らない)。
    pub fn set_wake_after(&mut self, d: Option<Duration>) {
        self.wake = d;
    }
}

impl Frontend for HeadlessFrontend {
    fn init(&mut self, _cfg: &FrontendConfig) -> Result<Capabilities, FrontendError> {
        Ok(self.caps.clone())
    }

    fn start_input(&mut self, tx: InputSink) -> Result<(), FrontendError> {
        self.shared.0.lock().unwrap().sink = Some(tx);
        Ok(())
    }

    fn surface_size(&self) -> CellSize {
        self.size
    }

    fn render(&mut self, frame: &Frame) -> RenderOutcome {
        self.shared.0.lock().unwrap().frames.push(frame.clone());
        RenderOutcome::Done
    }

    fn apply_effect(&mut self, _eff: &Effect) -> EffectOutcome {
        EffectOutcome::Unhandled
    }

    fn restore(&mut self) {
        self.shared.0.lock().unwrap().restored += 1;
    }

    fn wake_after(&self) -> Option<Duration> {
        self.wake
    }

    fn write_osc52(&mut self, text: &str) -> bool {
        self.shared.0.lock().unwrap().osc52.push(text.to_owned());
        true
    }
}
