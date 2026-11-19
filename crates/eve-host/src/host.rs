//! `Host`: Composition Root の共通部。単一の受信口からメッセージを取り、`Editor` を駆動し、`Effect` を実行し、
//! 必要なときだけ描画する (docs/02 §9.2)。TUI のように Host がループを回す形 (`run`) と、外のループが駆動する形
//! (`step`) の両方を許す。ブロッキング I/O はジョブ (ワーカー) に任せ、メインスレッドでは行わない (ADR-0020)。

use std::collections::BTreeMap;
use std::fmt;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};

use eve_core::{
    ClipboardFallbackReason, Clock, DocSnapshot, Editor, InitialFile, JobResult, Millis, Ports,
    RescueReason, RescueWriter, Tick,
};
use eve_view::{Capabilities, DocumentId, EditorEvent, Effect, StderrLine, WindowId};

use crate::emergency::EmergencyCtl;
use crate::frontend::{
    EffectOutcome, Frontend, FrontendConfig, FrontendError, InputSink, RenderOutcome,
};
use crate::msg::{ExitCode, HostMsg};

/// システムクリップボードの取得を待つ時間 (05 §8.1)。
const CLIPBOARD_TIMEOUT: Millis = Millis(2000);

/// 1 回の `step` で続けて処理する時間の目安 (超えたら残りは次の `step` へ)。
const STEP_BUDGET: Millis = Millis(8);
/// 進捗・検索の途中経過などによる再描画の最小の間隔 (入力・貼り付けの結果と、ジョブの完了・失敗などは即時に描く、04 §4.4)。
const RENDER_INTERVAL: Millis = Millis(16);
/// 蓄えておく `EditorEvent` の上限 (購読者がいない間に増え続けない)。
const MAX_EVENTS: usize = 4096;
/// 緊急退避の時間予算 (02 §11.2.1 の表)。パニック・Fatal は 5 s、シグナル系 (`CTRL_C` / `CTRL_BREAK` 等) は 2.5 s、`CTRL_CLOSE` は 3.5 s。
/// パニックと `HostMsg::Fatal` の退避の時間予算 (5 s)。
pub const RESCUE_BUDGET_FATAL: std::time::Duration = std::time::Duration::from_secs(5);
/// シグナル系 (Unix の SIGINT / SIGTERM / SIGHUP、Windows の `CTRL_C` / `CTRL_BREAK` 等) の退避の時間予算 (2.5 s)。
pub const RESCUE_BUDGET_SIGNAL: std::time::Duration = std::time::Duration::from_millis(2500);
/// `CTRL_CLOSE` (コンソールの閉鎖) の退避の時間予算 (3.5 s)。
pub const RESCUE_BUDGET_CLOSE: std::time::Duration = std::time::Duration::from_millis(3500);
/// Windows のログオフ・シャットダウンの退避の上限 (対話的なアプリの猶予は既定でおよそ 5 秒。`CTRL_CLOSE` と同じ配分、05 §11.2)。
pub const RESCUE_BUDGET_SESSION: std::time::Duration = std::time::Duration::from_millis(3500);
/// 「保存を中止して終了」の退避は時間予算なし (02 §11.2.1 の表。利用者が選んだ終了で、外から終わらされないので書き終えてから終了する)。
/// 書き手は `受信 + 予算` を期限にするので、`Instant` に足して溢れない大きさの値で「なし」を表す。
pub const RESCUE_BUDGET_UNBOUNDED: std::time::Duration =
    std::time::Duration::from_secs(24 * 60 * 60);

/// `Host::step` の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    /// ループを続ける。
    Continue,
    /// 終了する。値はプロセスの終了コード。
    Exit(ExitCode),
}

/// `Host::start` の失敗。
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
            StartError::NoWindow => write!(f, "could not create the window at startup"),
        }
    }
}

impl StartError {
    /// stderr の理由の文言のキー (bin の表 `stderr.reason.*`) と、OS の誤りの詳細。
    pub fn reason_key(&self) -> (String, Option<String>) {
        match self {
            StartError::Init(e) | StartError::Frontend(e) => {
                (format!("frontend.{}", e.kind.key()), e.detail.clone())
            }
            StartError::NoWindow => ("start.no_window".to_owned(), None),
        }
    }
}

impl std::error::Error for StartError {}

/// 受信口 (送信口と受信口の対)。
pub fn channel() -> (Sender<HostMsg>, Receiver<HostMsg>) {
    mpsc::channel()
}

/// Composition Root の共通部 (docs/02 §9.2)。`Editor` と Port を持ち、受信口のメッセージで `Editor` を駆動する。
pub struct Host {
    editor: Editor,
    clock: Arc<dyn Clock>,
    /// 正常終了のときにスワップを消すために持つ (`FileSystem` Port。ほかの I/O はジョブの中だけ)。
    fs: Arc<dyn eve_core::FileSystem>,
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
    slots: BTreeMap<DocumentId, Arc<DocSnapshot>>,
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
    /// システムクリップボードの提供者 (無ければ内部レジスタだけ)。
    clipboard: Option<Arc<dyn eve_platform::clipboard::ClipboardProvider>>,
    /// 取得の使い捨てスレッドが動いている (放棄したスレッドが戻るまで立ったまま。次の要求は新しいスレッドを立てない、05 §8.1)。
    clip_busy: Arc<std::sync::atomic::AtomicBool>,
    /// 待っている取得の token と期限 (2 秒)。
    clip_wait: Option<(eve_view::ClipboardToken, Tick)>,
    /// 貼り付けの結果を渡した: 次の描画を間引かない (クリップボードからの貼り付けは入力の結果、04 §4.4・02 §9.2)。
    render_now: bool,
    /// 端末が OSC 52 の書込に対応する (`frontend.init` の能力、05 §8.1)。
    osc52_write: bool,
    /// SSH 接続中 (`SSH_TTY` / `SSH_CONNECTION`)。`clipboard.osc52 = auto` で OSC 52 を使う条件。
    ssh_session: bool,
    /// 解決済みの設定 (07 §5.0)。
    config: eve_config::Config,
    /// 設定ファイルの診断 (名前, 診断)。最初の Frame の後にメッセージとして出す (07 §4.3、FR-703)。
    config_report: Vec<(String, Vec<eve_config::Diagnostic>)>,
    /// 言語ファイル・言語の選択の診断 (表示名, 診断)。設定の診断の後に `HostReport::LocaleDiagnostics` で出す (07 §17.4)。
    locale_report: Vec<(String, Vec<eve_config::Diagnostic>)>,
}

/// Host を作るときに渡すもの (docs/02 §9.2 の「Host の組み立て」)。`Default` は何も渡さない形 (既定の設定・内部レジスタだけ・
/// 退避の書き手なし)。`Host::with_channel` が欄を決まった順に Editor へ当てる (順は `Host::with_channel` の注釈)。
#[derive(Default)]
pub struct HostInit {
    /// 起動時に開くファイルの文字コードの指定 (`--encoding`、FR-111)。
    pub initial_encoding: Option<eve_core::Encoding>,
    /// 解決済みの設定 (07 §5.0)。`None` は既定の設定のまま (Editor に当てない)。
    pub config: Option<eve_config::Config>,
    /// 設定の診断 (表示名, 診断)。最初の Frame の後に、この順にメッセージとして出す (07 §4.3、FR-703)。空の診断は捨てる。
    pub config_reports: Vec<(String, Vec<eve_config::Diagnostic>)>,
    /// テーマファイルから読んだテーマ (名前, `None` は読めなかった: default で動く。07 §8.6)。
    pub user_theme: Option<(String, Option<eve_config::theme::Theme>)>,
    /// 設定の読込元 (`config.reload` が同じ元を読み直す、07 §6)。
    pub config_sources: Option<eve_core::ConfigSources>,
    /// ユーザーのキーマップ (表示名, `keymap.toml`)。その診断は `config_reports` の後に積む。
    pub keymap: Option<(String, eve_config::KeymapFile)>,
    /// UI の言語 (選択順は `eve_config::select_locale`、07 §17.1)。
    pub locale: Option<eve_config::Locale>,
    /// 選んだ言語の文言の表 (同梱の表にユーザーの言語ファイルを重ねたもの、07 §17.4)。`None` は `locale` の同梱の表。
    /// `locale` が `Some` のときだけ当てる (言語の後に当てないと、言語を当てたときに同梱の表に戻るため)。
    pub catalog: Option<Arc<eve_config::Catalog>>,
    /// ユーザーの言語ファイルの事実 (候補の言語と、選んだ言語のファイルの起動時の大きさと更新日時。07 §17.4)。
    /// `catalog` と同じく、`locale` が `Some` のときだけ当てる。
    pub locale_files: Option<eve_core::LocaleFiles>,
    /// 言語ファイル・言語の選択の診断 (表示名, 診断)。`config_reports` とキーマップの診断の後に出す (07 §17.4)。
    pub locale_reports: Vec<(String, Vec<eve_config::Diagnostic>)>,
    /// ログファイルの場所 (設定の問題の要約に出す)。
    pub log_path: Option<String>,
    /// OS のロケール由来の文字コードのヒント (`file.default_encoding = "system"`、05 §6)。
    pub system_encoding: Option<eve_core::Encoding>,
    /// システムクリップボードの提供者 (無ければ内部レジスタだけ)。
    pub clipboard: Option<Arc<dyn eve_platform::clipboard::ClipboardProvider>>,
    /// 緊急退避の書き手 (無ければ退避は書かない)。
    pub rescue_writer: Option<Arc<dyn RescueWriter>>,
    /// SSH 接続中か (`clipboard.osc52 = auto` の条件)。`None` は環境変数 `SSH_TTY` / `SSH_CONNECTION` から決める。
    pub ssh_session: Option<bool>,
}

impl Host {
    /// Port から、何も渡さない (`HostInit::default()`) Host を作る。Editor は空で、`start` で frontend の能力と最初の文書を設定する。
    pub fn new(ports: Ports) -> Host {
        Host::with_channel(ports, channel(), HostInit::default())
    }

    /// 受信口を外で作った Host (`ThreadJobSpawner` のように、Host より先に送信口が要る Port のため)。
    /// `init` は Editor に次の順で 1 項目ずつ当てる (v0.2.0 の bin の setter の順と同じ。テーマは設定の `theme.name` の後):
    /// 設定 → テーマ → 設定の読込元 → キーマップ → UI の言語 (と、その文言の表・言語ファイルの事実) → ログの場所 →
    /// OS の文字コード。
    /// 設定の診断は `config_reports` → キーマップの診断 → `locale_reports` の順に積み、`start` の最初の Frame の後に出す。
    pub fn with_channel(
        ports: Ports,
        (tx, inbox): (Sender<HostMsg>, Receiver<HostMsg>),
        init: HostInit,
    ) -> Host {
        use eve_core::StartupConfig;
        let HostInit {
            initial_encoding,
            config,
            config_reports,
            user_theme,
            config_sources,
            keymap,
            locale,
            catalog,
            locale_files,
            locale_reports,
            log_path,
            system_encoding,
            clipboard,
            rescue_writer,
            ssh_session,
        } = init;
        let clock = ports.clock.clone();
        let fs = ports.fs.clone();
        let mut editor = Editor::new(ports, Capabilities::default());
        let mut config_report: Vec<_> = config_reports
            .into_iter()
            .filter(|(_, d)| !d.is_empty())
            .collect();
        // `Editor::configure` は欄の順に当てるので、まとめずに 1 項目ずつ渡す (上の順を保つ)
        if let Some(c) = &config {
            editor.configure(StartupConfig {
                config: Some(c.clone()),
                ..Default::default()
            });
        }
        if let Some(t) = user_theme {
            editor.configure(StartupConfig {
                user_theme: Some(t),
                ..Default::default()
            });
        }
        if let Some(s) = config_sources {
            editor.configure(StartupConfig {
                config_sources: Some(s),
                ..Default::default()
            });
        }
        if let Some((file, keymap)) = keymap {
            let diagnostics = editor.set_user_keymap(&keymap);
            if !diagnostics.is_empty() {
                config_report.push((file, diagnostics));
            }
        }
        debug_assert!(
            locale.is_some() || (catalog.is_none() && locale_files.is_none()),
            "catalog と locale_files は locale と一緒に渡す"
        );
        if let Some(l) = locale {
            editor.configure(StartupConfig {
                locale: Some(l),
                catalog,
                locale_files,
                ..Default::default()
            });
        }
        editor.configure(StartupConfig {
            log_path: Some(log_path),
            system_encoding: Some(system_encoding),
            ..Default::default()
        });
        let emergency = EmergencyCtl::new();
        if let Some(w) = rescue_writer {
            emergency.set_writer(w);
        }
        Host {
            editor,
            clock,
            fs,
            inbox,
            tx,
            window: None,
            last_render: None,
            render_due: None,
            wake_at: None,
            slots: BTreeMap::new(),
            events: Vec::new(),
            exit_stderr: Vec::new(),
            initial_encoding,
            emergency,
            restore_info: Arc::new(std::sync::OnceLock::new()),
            run_finished: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            clipboard,
            clip_busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            clip_wait: None,
            render_now: false,
            osc52_write: false,
            ssh_session: ssh_session.unwrap_or_else(|| {
                std::env::var_os("SSH_TTY").is_some()
                    || std::env::var_os("SSH_CONNECTION").is_some()
            }),
            config: config.unwrap_or_default(),
            config_report,
            locale_report: locale_reports
                .into_iter()
                .filter(|(_, d)| !d.is_empty())
                .collect(),
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

    /// 駆動している `Editor`。
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    /// 駆動している `Editor` (可変)。
    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }

    /// 受信口への送信口 (ワーカー・入力スレッド・テスト用)。
    pub fn sender(&self) -> Sender<HostMsg> {
        self.tx.clone()
    }

    /// 最初の Window。`start` が成功するまでは `None`。
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
    pub fn emergency_slots(&self) -> &BTreeMap<DocumentId, Arc<DocSnapshot>> {
        &self.slots
    }

    /// 起動の後半 (02 §9.5): `frontend.init` → 能力と編集面の大きさ → 最初の文書 → 入力の開始 → 最初の Frame。
    /// `files` が空なら無題の文書。複数なら最初のものを表示し、残りは読込中のバッファとして足す。
    ///
    /// # Errors
    ///
    /// `frontend.init` が失敗したとき `StartError::Init` (端末を変えていない)。最初の Window を作れなかったとき
    /// `StartError::NoWindow`。入力の開始が失敗したとき `StartError::Frontend`。
    pub fn start(
        &mut self,
        frontend: &mut dyn Frontend,
        files: &[InitialFile],
        read_only: bool,
    ) -> Result<(), StartError> {
        let fcfg = FrontendConfig {
            ambiguous_width: self.config.ambiguous_width,
            mouse: self.config.mouse,
        };
        let caps = frontend.init(&fcfg).map_err(StartError::Init)?;
        self.osc52_write = caps.osc52_write;
        self.editor.set_caps(caps);
        if let Some(info) = frontend.emergency_restore_info() {
            // Unix: 異常終了のシグナル (SIGSEGV など) で端末だけを戻すハンドラを付ける (05 §11.1 の表の最後の行)。
            // 「復元済み」のフラグは frontend の `restore` と共有する。付けられなくても起動は続ける
            #[cfg(unix)]
            if let Ok(fd) = i32::try_from(info.handle.0) {
                let _ = eve_platform::term::install_crash_restore(
                    fd,
                    info.bytes,
                    &info.saved_mode.0,
                    Arc::clone(&info.restored.0),
                );
            }
            let _ = self.restore_info.set(info);
        }
        let size = frontend.surface_size();
        self.editor.report(eve_core::HostReport::Viewport {
            cols: size.cols,
            rows: size.rows,
        });
        self.editor.tick(self.clock.now());
        self.editor
            .open_initial_files(files, read_only, self.initial_encoding);
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
        // 設定の診断は最初の Frame の後に出す (起動を妨げない、07 §4.3)
        for (file, diagnostics) in std::mem::take(&mut self.config_report) {
            self.editor.report(eve_core::HostReport::ConfigDiagnostics {
                file: &file,
                diagnostics: &diagnostics,
            });
            self.render(frontend);
        }
        for (file, diagnostics) in std::mem::take(&mut self.locale_report) {
            self.editor.report(eve_core::HostReport::LocaleDiagnostics {
                file: &file,
                diagnostics: &diagnostics,
            });
            self.render(frontend);
        }
        Ok(())
    }

    /// TUI の既定ループ。終了コードを返す。終わると `run_finished` を立てる。
    pub fn run(
        &mut self,
        frontend: &mut dyn Frontend,
        files: &[InitialFile],
        read_only: bool,
    ) -> ExitCode {
        let code = self.run_loop(frontend, files, read_only);
        self.run_finished
            .store(true, std::sync::atomic::Ordering::Release);
        code
    }

    fn run_loop(
        &mut self,
        frontend: &mut dyn Frontend,
        files: &[InitialFile],
        read_only: bool,
    ) -> ExitCode {
        if let Err(e) = self.start(frontend, files, read_only) {
            // 07 §18.4: 端末の初期化失敗は 5 (04 §15.2 stderr.not_a_terminal)、それ以外の起動時の失敗は 1。
            let (key, code) = if matches!(e, StartError::Init(_)) {
                ("stderr.not_a_terminal", ExitCode(5))
            } else {
                frontend.restore();
                ("stderr.fatal", ExitCode(1))
            };
            // 理由は文言のキーと OS の詳細で渡す (bin が UI の言語の文にする。STR-C2)
            let (reason_key, detail) = e.reason_key();
            let mut args = vec![("reason_key".to_owned(), reason_key)];
            args.extend(detail.map(|d| ("detail".to_owned(), d)));
            self.exit_stderr.push(StderrLine {
                key: key.to_owned(),
                args,
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
            self.clip_wait.map(|(_, due)| due),
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
        self.check_clipboard_timeout();
        self.sync_emergency();
        let mut input_driven = false;
        let mut next = Some(first);
        while let Some(msg) = next.take() {
            // 各メッセージの処理の直前: Editor の「今」をこのメッセージの受信時刻にする
            self.editor.tick(self.clock.now());
            match self.handle_msg(frontend, msg) {
                ControlFlow::Continue(driven) => input_driven |= driven,
                ControlFlow::Break(code) => return StepOutcome::Exit(code),
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
        if let Some(code) = self.apply_effects(frontend) {
            return StepOutcome::Exit(code);
        }
        self.publish_events();
        self.render_after_step(frontend, input_driven);
        StepOutcome::Continue
    }

    /// 1 メッセージの処理。続けるなら「入力の結果として即時に描く」かを、終了なら終了コードを返す。
    fn handle_msg(
        &mut self,
        frontend: &mut dyn Frontend,
        msg: HostMsg,
    ) -> ControlFlow<ExitCode, bool> {
        match msg {
            HostMsg::Input(w, ev) => {
                self.editor.handle_input(w, ev);
                return ControlFlow::Continue(true);
            }
            HostMsg::Job(out) => {
                // 完了・失敗・確認などの結果は、待たせずに描く (開く操作の最初の画面を 16 ms 遅らせない)。
                // 進捗と検索の途中経過だけを間引く (04 §4.4)。先行表示の先頭 (`Partial`) は即時に描く (先頭を含む最初の
                // Frame を待たせない。BM-104v、03 §9.2.1 の「計測」)
                let driven = !matches!(
                    out.body,
                    JobResult::Progress { .. } | JobResult::SearchBatch { .. }
                );
                // 標準入力を読み終えた (EOF・上限・取消し・失敗): 端末のモードを入れ直して全面を描き直す (03 §9.2.2 の
                // 「端末の初期化の時期」。書き手が端末のモードを変えたり stderr に書いたりして画面が崩れるため)
                let stdin_done = matches!(out.body, JobResult::StdinDone { .. });
                self.editor.on_job_result(*out);
                if stdin_done {
                    frontend.reinit_terminal();
                }
                return ControlFlow::Continue(driven);
            }
            HostMsg::Clipboard { token, result } => self.on_clipboard(token, result),
            HostMsg::Osc52 {
                text,
                system_failure,
            } => {
                if !frontend.write_osc52(&text)
                    && let Some(e) = system_failure
                {
                    self.report_clipboard_failure(e, ClipboardFallbackReason::WriteFailed);
                }
            }
            HostMsg::ClipboardWriteFailed(e) => {
                self.report_clipboard_failure(e, ClipboardFallbackReason::WriteFailed);
            }
            HostMsg::Signal(kind) => {
                return ControlFlow::Break(self.exit_on_signal(frontend, kind));
            }
            HostMsg::Fatal(_) => return ControlFlow::Break(self.exit_on_fatal(frontend)),
            HostMsg::Tick => {}
        }
        ControlFlow::Continue(false)
    }

    /// `HostMsg::Clipboard`: 待っている token の結果だけ使う (期限後に届いたものは捨てる)。
    fn on_clipboard(
        &mut self,
        token: eve_view::ClipboardToken,
        result: Result<Option<String>, eve_platform::clipboard::ClipboardFailure>,
    ) {
        if !self.clip_wait.is_some_and(|(t, _)| t == token) {
            return;
        }
        self.clip_wait = None;
        let text = match result {
            Ok(t) => t,
            Err(e) => {
                self.report_clipboard_failure(e, ClipboardFallbackReason::ReadFailed);
                None
            }
        };
        self.deliver_clipboard(token, text);
    }

    /// クリップボードの失敗を、内部レジスタに切り替えたこととして core に知らせる。
    fn report_clipboard_failure(
        &mut self,
        e: eve_platform::clipboard::ClipboardFailure,
        otherwise: ClipboardFallbackReason,
    ) {
        let reason = self.clipboard_reason(e, otherwise);
        self.editor
            .report(eve_core::HostReport::ClipboardFallback { reason });
    }

    /// 非対話の終了要求: 退避を書く (受信スレッドが先に書いていれば、その `Done` を待つ) → 端末の復元 → 終了 (02 §11.2.1)。
    fn exit_on_signal(
        &mut self,
        frontend: &mut dyn Frontend,
        kind: crate::msg::SignalKind,
    ) -> ExitCode {
        // test-hooks: メインが止まった状態の再現 (受信スレッドの緊急の端末復元を検査する、09 §3.6)
        #[cfg(feature = "test-hooks")]
        if test_hook("main_stall") {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
        let (reason, budget) = match kind {
            crate::msg::SignalKind::ConsoleClose => {
                (RescueReason::ConsoleClose, RESCUE_BUDGET_CLOSE)
            }
            _ => (RescueReason::Signal, RESCUE_BUDGET_SIGNAL),
        };
        self.emergency.run(reason, budget);
        frontend.restore();
        ExitCode::INTERRUPTED
    }

    /// 入力スレッドの死亡・非メインスレッドのパニック: メインが退避を書く (最大 5 s) → 端末の復元 → 終了。
    fn exit_on_fatal(&mut self, frontend: &mut dyn Frontend) -> ExitCode {
        self.emergency.run(RescueReason::Fatal, RESCUE_BUDGET_FATAL);
        frontend.restore();
        ExitCode::FATAL
    }

    /// 溜まった `Effect` を実行する (frontend が扱わなかったものだけ)。`Effect::Quit` なら終了コードを返す (残りは実行しない)。
    fn apply_effects(&mut self, frontend: &mut dyn Frontend) -> Option<ExitCode> {
        for eff in self.editor.drain_effects() {
            if frontend.apply_effect(&eff) == EffectOutcome::Handled {
                continue;
            }
            match eff {
                // 書込: 内部レジスタには core が書き済み。システムへは使い捨てのスレッドで書く (ブロックし得る)
                Effect::SetClipboard { content, .. } => self.write_clipboard(content),
                // 読込: 使い捨てのスレッドで取得し、2 秒で応答が無ければ内部レジスタからの貼り付けに切り替える (05 §8.1)
                Effect::RequestClipboard { token, .. } => self.read_clipboard(token),
                Effect::Quit {
                    code,
                    stderr,
                    rescue,
                    ..
                } => return Some(self.exit_normally(frontend, code, stderr, rescue)),
                _ => {}
            }
        }
        None
    }

    /// `Effect::Quit`: スワップの後始末・必要なら退避 → 端末の復元 → 終了コード。
    fn exit_normally(
        &mut self,
        frontend: &mut dyn Frontend,
        code: u8,
        stderr: Vec<StderrLine>,
        rescue: eve_view::QuitRescue,
    ) -> ExitCode {
        // 未保存の確認 (FR-107) を経た終了 (`app.quit` の 0 と `app.quit_with_error` の 4、07 §18.4) では、スワップはもう要らない。
        // 「保存を中止して終了」などの退避を伴う終了では残す (復旧の材料)
        if matches!(rescue, eve_view::QuitRescue::None) {
            for (doc, path) in self.editor.swap_leftovers() {
                let _ = self.fs.discard_swap(doc, path.as_ref());
            }
            for entry in self.editor.swap_recovered_leftovers() {
                let _ = self.fs.remove_swap(&entry);
            }
        }
        // 「保存を中止して終了」: dirty な文書を緊急退避と同じ書き手・形式で退避してから終了する (03 §9.7 (f))。
        // 結果の行は終了時に main が出す (`emergency.outcome()`)
        if matches!(rescue, eve_view::QuitRescue::DirtyDocuments) {
            self.sync_emergency();
            self.emergency
                .run(RescueReason::QuitAbortedSave, RESCUE_BUDGET_UNBOUNDED);
        }
        frontend.restore();
        self.exit_stderr = stderr;
        ExitCode(code)
    }

    /// `step` の最後の描画: 変化があり間引きの対象でなければ、または frontend の期限が来ていれば描く。次の frontend の期限を覚える。
    fn render_after_step(&mut self, frontend: &mut dyn Frontend, input_driven: bool) {
        let wake_due = self.wake_at.is_some_and(|t| t <= self.clock.now());
        // 貼り付けの結果 (取得の結果・期限切れで内部レジスタから) も入力の結果として即時に描く
        let input_driven = input_driven || std::mem::take(&mut self.render_now);
        if (self.editor.needs_render() && self.render_allowed(input_driven)) || wake_due {
            self.render(frontend);
        }
        self.wake_at = frontend
            .wake_after()
            .map(|d| self.clock.now() + (Millis(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))));
    }

    /// クリップボードの失敗の理由 (`msg.clipboard.fallback` の `{reason}`)。手段が無いときは提供者が理由を持つ
    /// (Linux の「wl-clipboard または xclip を入れると…」等、05 §8.2)。
    fn clipboard_reason(
        &self,
        e: eve_platform::clipboard::ClipboardFailure,
        otherwise: ClipboardFallbackReason,
    ) -> ClipboardFallbackReason {
        use eve_platform::clipboard::ClipboardFailure::{TooLarge, Unsupported};
        match e {
            Unsupported => self
                .clipboard
                .as_ref()
                .and_then(|p| p.unsupported_reason())
                .unwrap_or(otherwise),
            TooLarge => ClipboardFallbackReason::TooLarge,
            _ => otherwise,
        }
    }

    /// `Effect::SetClipboard`: 提供者があれば、使い捨てのスレッドで文字列化して書く。上限 (64 MiB) を超えたら書かずに知らせる。
    #[allow(clippy::disallowed_methods)] // クリップボード書込の使い捨てスレッド (02 §9.1 の表)
    fn write_clipboard(&mut self, content: eve_view::ClipboardText) {
        use eve_platform::clipboard::SYSTEM_MAX_BYTES;
        let source = match content {
            eve_view::ClipboardText::Handle(id) => self.editor.take_clipboard(id),
            _ => None,
        };
        let provider = self.clipboard.clone();
        // OSC 52 で書く条件: 端末が対応し、`clipboard.osc52` が `never` でない。`always` と SSH 中の `auto` は
        // システムへの書込の成否に関わらず書き、それ以外はシステムに書けなかったときだけ書く (05 §8.1)
        let osc = self
            .osc52_write
            .then_some(self.editor.state().config().osc52)
            .and_then(|p| {
                use eve_config::Osc52Policy::{Always, Auto, Never};
                match p {
                    Never => None,
                    Always => Some(true),
                    Auto => Some(self.ssh_session),
                }
            });
        let max_b64 = self.editor.state().config().osc52_max_bytes as usize;
        if provider.is_none() && osc.is_none() {
            return;
        }
        // 文字列化の前に総バイト数で上限と比べる
        let too_large = match (&content, &source) {
            (eve_view::ClipboardText::Inline(s), _) => s.len() > SYSTEM_MAX_BYTES,
            (_, Some(src)) => src.len_bytes() > SYSTEM_MAX_BYTES,
            _ => false,
        };
        if too_large {
            self.editor.report(eve_core::HostReport::ClipboardFallback {
                reason: ClipboardFallbackReason::TooLarge,
            });
            return;
        }
        let tx = self.tx.clone();
        let _ = std::thread::Builder::new()
            .name("eve-clip-write".to_owned())
            .spawn(move || {
                let text = match (content, source) {
                    (eve_view::ClipboardText::Inline(s), _) => s,
                    (_, Some(src)) => src.to_text(),
                    _ => return,
                };
                let result = match &provider {
                    Some(p) => p.set_text(&text),
                    None => Err(eve_platform::clipboard::ClipboardFailure::Unavailable),
                };
                // base64 は 3 バイトを 4 文字にする。上限を超える本文は OSC 52 では送らない (05 §8.4)
                let fits = text.len().div_ceil(3) * 4 <= max_b64;
                match (osc, result) {
                    (Some(always), r) if fits && (always || r.is_err()) => {
                        let _ = tx.send(HostMsg::Osc52 {
                            text,
                            system_failure: r.err(),
                        });
                    }
                    (_, Err(e)) => {
                        let _ = tx.send(HostMsg::ClipboardWriteFailed(e));
                    }
                    _ => {}
                }
            });
    }

    /// `Effect::RequestClipboard`。提供者が無い・前の取得が戻っていないときは、直ちに「読めなかった」と返す。
    #[allow(clippy::disallowed_methods)] // クリップボード取得の使い捨てスレッド (02 §9.1 の表)
    fn read_clipboard(&mut self, token: eve_view::ClipboardToken) {
        use std::sync::atomic::Ordering;
        let provider = self.clipboard.clone();
        let busy = self.clip_busy.swap(true, Ordering::AcqRel);
        let (Some(provider), false) = (provider, busy) else {
            // 放棄したスレッドが戻るまでは積み増さない (busy を立てたのは自分なら戻す)
            if !busy && self.clipboard.is_none() {
                self.clip_busy.store(false, Ordering::Release);
            }
            self.deliver_clipboard(token, None);
            return;
        };
        let tx = self.tx.clone();
        let flag = Arc::clone(&self.clip_busy);
        let spawned = std::thread::Builder::new()
            .name("eve-clip-read".to_owned())
            .spawn(move || {
                let result = provider.get_text();
                flag.store(false, Ordering::Release);
                let _ = tx.send(HostMsg::Clipboard { token, result });
            });
        if spawned.is_err() {
            self.clip_busy.store(false, Ordering::Release);
            self.deliver_clipboard(token, None);
            return;
        }
        self.clip_wait = Some((token, self.clock.now() + CLIPBOARD_TIMEOUT));
    }

    /// 取得の結果 (または代わりの `None`) を core へ渡す。
    fn deliver_clipboard(&mut self, token: eve_view::ClipboardToken, text: Option<String>) {
        if let Some(w) = self.window {
            self.editor
                .handle_input(w, eve_view::InputEvent::ClipboardContent { token, text });
            self.render_now = true;
        }
    }

    /// 取得の期限 (2 秒) が来たら、内部レジスタからの貼り付けに切り替える。取得のスレッドは放棄する (終了を待たない)。
    fn check_clipboard_timeout(&mut self) {
        if let Some((token, due)) = self.clip_wait
            && self.clock.now() >= due
        {
            self.clip_wait = None;
            self.editor.report(eve_core::HostReport::ClipboardFallback {
                reason: ClipboardFallbackReason::Timeout,
            });
            self.deliver_clipboard(token, None);
        }
    }

    /// 描画の間引き: 入力の結果 (クリップボードからの貼り付けを含む) は即時に描く。それ以外 (ジョブの進捗など) だけの変化は、
    /// 直前の描画から 16 ms 未満なら描かず、期限を待ちの期限に加える。
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
                    // 退避の書き手のスロットと Host の写しで 1 つの公開物を共有する (複製しない、PRF-19)
                    let s = Arc::new(s);
                    self.emergency.publish(doc, Some(Arc::clone(&s)));
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
