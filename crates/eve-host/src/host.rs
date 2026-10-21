//! `Host`: Composition Root の共通部。単一の受信口からメッセージを取り、`Editor` を駆動し、`Effect` を実行し、
//! 必要なときだけ描画する (docs/02 §9.2)。TUI のように Host がループを回す形 (`run`) と、外のループが駆動する形
//! (`step`) の両方を許す。ブロッキング I/O はジョブ (ワーカー) に任せ、メインスレッドでは行わない (ADR-0020)。

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};

use eve_core::{Clock, DocSnapshot, Editor, Millis, Ports, RescueReason, RescueWriter, Tick};
use eve_view::{Capabilities, DocumentId, EditorEvent, Effect, StderrLine, WindowId};

use crate::emergency::EmergencyCtl;
use crate::frontend::{
    EffectOutcome, Frontend, FrontendConfig, FrontendError, InputSink, RenderOutcome,
};
use crate::msg::{ExitCode, HostMsg};

/// 1 回の `step` で続けて処理する時間の目安 (超えたら残りは次の `step` へ)。
const STEP_BUDGET: Millis = Millis(8);
/// ジョブの結果などによる再描画の最小の間隔 (入力・貼り付けの結果は即時に描く、04 §4.4)。
const RENDER_INTERVAL: Millis = Millis(16);
/// 蓄えておく `EditorEvent` の上限 (購読者がいない間に増え続けない)。
const MAX_EVENTS: usize = 4096;
/// 緊急退避の時間予算 (02 §11.2.1 の表)。パニック・Fatal は 5 s、シグナル系 (CTRL_C / CTRL_BREAK 等) は 2.5 s、CTRL_CLOSE は 3.5 s。
pub const RESCUE_BUDGET_FATAL: std::time::Duration = std::time::Duration::from_secs(5);
pub const RESCUE_BUDGET_SIGNAL: std::time::Duration = std::time::Duration::from_millis(2500);
pub const RESCUE_BUDGET_CLOSE: std::time::Duration = std::time::Duration::from_millis(3500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    Continue,
    Exit(ExitCode),
}

#[derive(Debug)]
pub enum StartError {
    /// `Frontend::init` の失敗 (端末でない等)。端末を変えていないので復元は要らない。
    Init(FrontendError),
    /// 入力の開始の失敗。
    Frontend(FrontendError),
    /// 起動時に Window を作れなかった (内部エラー)。
    NoWindow,
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::Init(e) | StartError::Frontend(e) => e.fmt(f),
            StartError::NoWindow => write!(f, "起動時に Window を作れませんでした"),
        }
    }
}

impl std::error::Error for StartError {}

/// 受信口 (送信口と受信口の対)。
pub fn channel() -> (Sender<HostMsg>, Receiver<HostMsg>) {
    mpsc::channel()
}

pub struct Host {
    editor: Editor,
    clock: Arc<dyn Clock>,
    inbox: Receiver<HostMsg>,
    tx: Sender<HostMsg>,
    window: Option<WindowId>,
    /// 直前に描いた時刻。
    last_render: Option<Tick>,
    /// 間引いた描画の期限。
    render_due: Option<Tick>,
    /// `Frontend::wake_after` の期限。
    wake_at: Option<Tick>,
    /// 退避用の公開物 (文書ごとのスロット。緊急退避の書き手が読む)。
    slots: BTreeMap<DocumentId, DocSnapshot>,
    events: Vec<EditorEvent>,
    exit_stderr: Vec<StderrLine>,
    /// 起動時に開くファイルの文字コードの指定 (`--encoding`)。
    initial_encoding: Option<eve_core::Encoding>,
    /// 緊急退避の共有状態 (公開物のスロット・1 回だけ実行する CAS)。他のスレッドからも使う。
    emergency: Arc<EmergencyCtl>,
    /// 緊急の端末復元の情報 (`init` の後に 1 回だけ入る。受信スレッドが読む、02 §9.6 手順 4)。
    restore_info: Arc<std::sync::OnceLock<crate::frontend::EmergencyRestoreInfo>>,
    /// `run` が終わった (メインが終了へ向かっている)。受信スレッドが「メインの終了待ち」で見る (02 §9.6 手順 2)。
    run_finished: Arc<std::sync::atomic::AtomicBool>,
}

impl Host {
    /// Port から Host を作る。Editor は空で、`start` で frontend の能力と最初の文書を設定する。
    pub fn new(ports: Ports) -> Host {
        Host::with_channel(ports, channel())
    }

    /// 起動時に開くファイルの文字コードを指定する (`--encoding`、FR-111)。`start` / `run` の前に呼ぶ。
    pub fn set_initial_encoding(&mut self, encoding: Option<eve_core::Encoding>) {
        self.initial_encoding = encoding;
    }

    /// 受信口を外で作った Host。`ThreadJobSpawner` のように、Host より先に送信口が要る Port のため。
    pub fn with_channel(ports: Ports, (tx, inbox): (Sender<HostMsg>, Receiver<HostMsg>)) -> Host {
        let clock = ports.clock.clone();
        Host {
            editor: Editor::new(ports, Capabilities::default()),
            clock,
            inbox,
            tx,
            window: None,
            last_render: None,
            render_due: None,
            wake_at: None,
            slots: BTreeMap::new(),
            events: Vec::new(),
            exit_stderr: Vec::new(),
            initial_encoding: None,
            emergency: EmergencyCtl::new(),
            restore_info: Arc::new(std::sync::OnceLock::new()),
            run_finished: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// 緊急の端末復元の情報の置き場 (受信スレッドに配る。`start` の `init` の後に入る)。
    pub fn restore_info(&self) -> Arc<std::sync::OnceLock<crate::frontend::EmergencyRestoreInfo>> {
        Arc::clone(&self.restore_info)
    }

    /// `run` が終わると立つフラグ。受信スレッドが、シグナルの後にメインが終了へ向かったかを見るのに使う。
    pub fn run_finished(&self) -> Arc<std::sync::atomic::AtomicBool> {
        Arc::clone(&self.run_finished)
    }

    /// 緊急退避の共有状態。コンソールイベントのハンドラ・パニックの後始末が、メインを待たずに退避を呼ぶのに使う (02 §11.2.1)。
    pub fn emergency(&self) -> Arc<EmergencyCtl> {
        Arc::clone(&self.emergency)
    }

    /// 緊急退避の書き手を設定する (`start` / `run` の前に呼ぶ)。
    pub fn set_rescue_writer(&mut self, w: Arc<dyn RescueWriter>) {
        self.emergency.set_writer(w);
    }

    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }

    /// 受信口への送信口 (ワーカー・入力スレッド・テスト用)。
    pub fn sender(&self) -> Sender<HostMsg> {
        self.tx.clone()
    }

    pub fn window(&self) -> Option<WindowId> {
        self.window
    }

    /// 蓄えた `EditorEvent` を取り出す (内部購読者・駆動 API の代わり)。
    pub fn take_events(&mut self) -> Vec<EditorEvent> {
        std::mem::take(&mut self.events)
    }

    /// 終了時に端末の復元の後に出す行 (`Effect::Quit` の `stderr`)。文言の解決は呼び出し側 (bin)。
    pub fn take_exit_stderr(&mut self) -> Vec<StderrLine> {
        std::mem::take(&mut self.exit_stderr)
    }

    /// 退避用の公開物 (dirty な文書だけがある)。
    pub fn emergency_slots(&self) -> &BTreeMap<DocumentId, DocSnapshot> {
        &self.slots
    }

    /// 起動の後半 (02 §9.5): `frontend.init` → 能力と編集面の大きさ → 最初の文書 → 入力の開始 → 最初の Frame。
    /// `path` が `None` なら無題の文書。
    pub fn start(
        &mut self,
        frontend: &mut dyn Frontend,
        path: Option<&str>,
        read_only: bool,
    ) -> Result<(), StartError> {
        let caps = frontend
            .init(&FrontendConfig::default())
            .map_err(StartError::Init)?;
        self.editor.state_mut().set_caps(caps);
        if let Some(info) = frontend.emergency_restore_info() {
            let _ = self.restore_info.set(info);
        }
        let size = frontend.surface_size();
        self.editor.state_mut().set_viewport(size.cols, size.rows);
        self.editor.tick(self.clock.now());
        self.editor
            .open_initial_with(path, read_only, self.initial_encoding);
        let window = self
            .editor
            .state()
            .first_window()
            .ok_or(StartError::NoWindow)?;
        self.window = Some(window);
        self.sync_emergency();
        frontend
            .start_input(InputSink::new(self.tx.clone(), window))
            .map_err(StartError::Frontend)?;
        self.render(frontend);
        Ok(())
    }

    /// TUI の既定ループ。終了コードを返す。終わると `run_finished` を立てる。
    pub fn run(
        &mut self,
        frontend: &mut dyn Frontend,
        path: Option<&str>,
        read_only: bool,
    ) -> ExitCode {
        let code = self.run_loop(frontend, path, read_only);
        self.run_finished
            .store(true, std::sync::atomic::Ordering::Release);
        code
    }

    fn run_loop(
        &mut self,
        frontend: &mut dyn Frontend,
        path: Option<&str>,
        read_only: bool,
    ) -> ExitCode {
        if let Err(e) = self.start(frontend, path, read_only) {
            // 07 §18.4: 端末の初期化失敗は 5 (04 §15.2 stderr.not_a_terminal)、それ以外の起動時の失敗は 1。
            let (key, code) = if matches!(e, StartError::Init(_)) {
                ("stderr.not_a_terminal", ExitCode(5))
            } else {
                frontend.restore();
                ("stderr.fatal", ExitCode(1))
            };
            self.exit_stderr.push(StderrLine {
                key: key.to_owned(),
                args: vec![("reason".to_owned(), e.to_string())],
            });
            return code;
        }
        loop {
            // 待機: 次のメッセージか、次の期限 (Editor の deadline・間引いた描画・frontend の時間)
            let first = match self.wait_deadline(frontend) {
                Some(d) => {
                    let wait = d.saturating_sub(self.clock.now()).as_duration();
                    match self.inbox.recv_timeout(wait) {
                        Ok(m) => m,
                        Err(RecvTimeoutError::Timeout) => HostMsg::Tick,
                        Err(RecvTimeoutError::Disconnected) => {
                            HostMsg::Fatal(crate::msg::FatalReason::InputThreadDied)
                        }
                    }
                }
                None => match self.inbox.recv() {
                    Ok(m) => m,
                    Err(_) => HostMsg::Fatal(crate::msg::FatalReason::InputThreadDied),
                },
            };
            if let StepOutcome::Exit(code) = self.step(frontend, first) {
                return code;
            }
        }
    }

    /// 次に起きる必要がある時刻 (どれも無ければ `None` = 無期限に待ってよい、NFR-204)。
    fn wait_deadline(&self, frontend: &dyn Frontend) -> Option<Tick> {
        let from_frontend = frontend
            .wake_after()
            .map(|d| self.clock.now() + (Millis(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))));
        [
            self.editor.next_deadline(),
            self.render_due,
            self.wake_at,
            from_frontend,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// 1 メッセージ (と溜まっている分) を処理し、`Effect` を実行し、必要なら 1 回描画する (02 §9.2)。
    pub fn step(&mut self, frontend: &mut dyn Frontend, first: HostMsg) -> StepOutcome {
        let budget_end = self.clock.now() + (STEP_BUDGET);
        // 先頭: アイドル中に過ぎた期限を先に処理する
        self.editor.tick(self.clock.now());
        self.sync_emergency();
        let mut input_driven = false;
        let mut next = Some(first);
        while let Some(msg) = next.take() {
            // 各メッセージの処理の直前: Editor の「今」をこのメッセージの受信時刻にする
            self.editor.tick(self.clock.now());
            match msg {
                HostMsg::Input(w, ev) => {
                    input_driven = true;
                    self.editor.handle_input(w, ev);
                }
                HostMsg::Job(result) => self.editor.on_job_result(*result),
                HostMsg::Signal(kind) => {
                    // test-hooks: メインが止まった状態の再現 (受信スレッドの緊急の端末復元を検査する、09 §3.6)
                    #[cfg(feature = "test-hooks")]
                    if test_hook("main_stall") {
                        loop {
                            std::thread::sleep(std::time::Duration::from_secs(3600));
                        }
                    }
                    // 非対話の終了要求: 退避を書く (受信スレッドが先に書いていれば、その `Done` を待つ) → 端末の復元 → 終了 (02 §11.2.1)
                    let (reason, budget) = match kind {
                        crate::msg::SignalKind::ConsoleClose => {
                            (RescueReason::ConsoleClose, RESCUE_BUDGET_CLOSE)
                        }
                        _ => (RescueReason::Signal, RESCUE_BUDGET_SIGNAL),
                    };
                    self.emergency.run(reason, budget);
                    frontend.restore();
                    return StepOutcome::Exit(ExitCode::INTERRUPTED);
                }
                HostMsg::Fatal(_) => {
                    // 入力スレッドの死亡・非メインスレッドのパニック: メインが退避を書く (最大 5 s) → 端末の復元 → 終了
                    self.emergency.run(RescueReason::Fatal, RESCUE_BUDGET_FATAL);
                    frontend.restore();
                    return StepOutcome::Exit(ExitCode::FATAL);
                }
                HostMsg::Tick => {}
            }
            self.sync_emergency();
            // 予算の判定は「次を取り出す前」に行う (取り出したメッセージを捨てない)
            if self.clock.now() >= budget_end {
                break;
            }
            next = self.inbox.try_recv().ok();
        }
        // 最後: 処理中に来た期限を処理する
        self.editor.tick(self.clock.now());
        self.sync_emergency();
        for eff in self.editor.drain_effects() {
            if frontend.apply_effect(&eff) == EffectOutcome::Handled {
                continue;
            }
            // クリップボード・ベルなど: 提供者 (ClipboardProvider) ができる版で扱う
            if let Effect::Quit { code, stderr, .. } = eff {
                frontend.restore();
                self.exit_stderr = stderr;
                return StepOutcome::Exit(ExitCode(code));
            }
        }
        self.publish_events();
        let wake_due = self.wake_at.is_some_and(|t| t <= self.clock.now());
        if (self.editor.needs_render() && self.render_allowed(input_driven)) || wake_due {
            self.render(frontend);
        }
        self.wake_at = frontend
            .wake_after()
            .map(|d| self.clock.now() + (Millis(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))));
        StepOutcome::Continue
    }

    /// 描画の間引き: 入力の結果は即時に描く。それ以外 (ジョブの結果など) だけの変化は、直前の描画から 16 ms 未満なら
    /// 描かず、期限を待ちの期限に加える。
    fn render_allowed(&mut self, input_driven: bool) -> bool {
        let now = self.clock.now();
        match self.last_render {
            Some(last) if !input_driven && now.saturating_sub(last) < RENDER_INTERVAL => {
                self.render_due = Some(last + RENDER_INTERVAL);
                false
            }
            _ => true,
        }
    }

    fn render(&mut self, frontend: &mut dyn Frontend) {
        let Some(w) = self.window else { return };
        let Some(frame) = self.editor.frame(w) else {
            return;
        };
        if let RenderOutcome::SurfaceChanged { cols, rows } = frontend.render(&frame) {
            // 2 パス目 (予約)
            self.editor
                .handle_input(w, eve_view::InputEvent::Resize { cols, rows });
            if let Some(f) = self.editor.frame(w) {
                frontend.render(&f);
            }
        }
        self.last_render = Some(self.clock.now());
        self.render_due = None;
    }

    /// Editor の状態を変える呼出しの直後に毎回呼ぶ: 変わった文書の退避用の公開物をスロットに公開する
    /// (dirty でなくなった文書のスロットは空にする)。
    fn sync_emergency(&mut self) {
        for doc in self.editor.take_snapshot_updates() {
            match self.editor.emergency_snapshot(doc) {
                Some(s) => {
                    self.emergency.publish(doc, Some(s.clone()));
                    self.slots.insert(doc, s);
                }
                None => {
                    self.emergency.publish(doc, None);
                    self.slots.remove(&doc);
                }
            }
        }
    }

    fn publish_events(&mut self) {
        self.events.extend(self.editor.drain_events());
        if self.events.len() > MAX_EVENTS {
            let drop = self.events.len() - MAX_EVENTS;
            self.events.drain(..drop);
        }
    }
}

/// 環境変数 `EVE_TEST_HOOK` (`,` 区切り) に `name` があるか。
#[cfg(feature = "test-hooks")]
fn test_hook(name: &str) -> bool {
    std::env::var("EVE_TEST_HOOK")
        .is_ok_and(|v| v.split(',').any(|h| h.split('=').next() == Some(name)))
}
