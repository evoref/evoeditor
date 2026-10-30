//! `Harness`: core をヘッドレスで駆動するシナリオの入口 (docs/09 §3.4、11 §6)。
//!
//! 偽 Port (`MemFs`・`ManualClock`・`ManualJobSpawner`) で `Editor` を組み、入力 (`open` / `keys` / `text` / `paste` /
//! `cmd` / `answer` / `advance_ms` / `resize` / `run_jobs`) と検査 (`doc_text` / `fs_read` / `frame_text` /
//! `frame_contains` / `events` / `effects` / `messages` / `overlay`) を提供する。語彙は v0.2 のシナリオ DSL (11 §6.3)
//! と同じで、DSL はこの Harness の薄い写像として足す。
//!
//! ジョブは `ManualJobSpawner` が積むだけで、`run_jobs` を呼ぶまで進まない (「保存の途中」を観測できる)。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    Args, Clock, CommandInvocation, CommandResult, Editor, InvocationSource, JobPorts, Millis,
    Ports,
};
use eve_view::{
    Answer, Capabilities, DocumentId, EditorEvent, Effect, Frame, InputEvent, KeyEvent, Query,
    QueryError, WindowId,
};

use crate::ascii::{AsciiOptions, render_ascii};
use crate::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};

pub struct Harness {
    pub editor: Editor,
    pub fs: Arc<MemFs>,
    pub clock: ManualClock,
    jobs: Rc<ManualJobSpawner>,
    window: WindowId,
    events: Vec<EditorEvent>,
    effects: Vec<Effect>,
}

impl Default for Harness {
    fn default() -> Harness {
        Harness::new()
    }
}

impl Harness {
    /// 無題の文書が 1 つ開いた状態。保存先の `/w` があり、画面は 80×24。
    pub fn new() -> Harness {
        let clock = ManualClock::new();
        let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
        fs.add_dir("/w");
        let paths = Arc::new(FixedPaths::default());
        let jobs = ManualJobSpawner::new(JobPorts {
            fs: fs.clone(),
            clock: Arc::new(clock.clone()),
            paths: paths.clone(),
        });
        let ports = Ports {
            fs: fs.clone(),
            clock: Arc::new(clock.clone()),
            jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
            paths,
        };
        let mut editor = Editor::new(ports, Capabilities::default());
        editor.open_initial(None, false);
        let window = editor.state().first_window().expect("起動時の Window");
        let mut h = Harness {
            editor,
            fs,
            clock,
            jobs,
            window,
            events: Vec::new(),
            effects: Vec::new(),
        };
        h.resize(80, 24);
        h
    }

    // ---- 初期状態 ----

    /// 偽 FS にファイルを置く (開くのは `open`)。
    pub fn with_file(self, path: &str, data: &[u8]) -> Harness {
        self.fs.add_file(path, data);
        self
    }

    pub fn with_caps(mut self, caps: Capabilities) -> Harness {
        self.editor.set_caps(caps);
        self
    }

    pub fn with_size(mut self, cols: u16, rows: u16) -> Harness {
        self.resize(cols, rows);
        self
    }

    // ---- 入力 ----

    /// `file.open`。ジョブは `run_jobs` で完了する。
    pub fn open(&mut self, path: &str) -> CommandResult {
        self.cmd_with("file.open", Args::new().with_str("path", path))
    }

    /// 07 §7.2 の設定表記 (小文字・`+`・空白区切り) のキー列を押す。`mod` は `ctrl`。不正な表記は panic する。
    pub fn keys(&mut self, seq: &str) {
        for part in seq.split_whitespace() {
            let stroke = eve_core::parse_stroke(part)
                .unwrap_or_else(|e| panic!("キー表記 {part:?} を読めません: {e}"));
            self.input(InputEvent::Key(KeyEvent::press(stroke.key, stroke.mods)));
        }
    }

    /// フォーカスの変化 (`InputEvent::Focus`)。
    pub fn focus(&mut self, gained: bool) {
        self.input(InputEvent::Focus(gained));
    }

    /// 確定済みのテキスト入力 (`InputEvent::Text`)。
    pub fn text(&mut self, s: &str) {
        self.input(InputEvent::Text(s.to_owned()));
    }

    /// `Effect::RequestClipboard` への応答 (`InputEvent::ClipboardContent`)。
    pub fn clipboard_content(&mut self, token: eve_view::ClipboardToken, text: Option<&str>) {
        self.input(InputEvent::ClipboardContent {
            token,
            text: text.map(str::to_owned),
        });
    }

    /// マウス操作 (`InputEvent::Mouse`)。`row` / `col` は編集面の左上を原点とするセル (面の外は負やはみ出しで渡す)。
    pub fn mouse(
        &mut self,
        kind: eve_view::MouseKind,
        mods: eve_view::Modifiers,
        row: i32,
        col: i32,
    ) {
        let view = self
            .editor
            .state()
            .first_window()
            .and_then(|w| self.editor.state().window_view(w))
            .expect("View がある");
        self.input(InputEvent::Mouse(eve_view::MouseEvent {
            kind,
            mods,
            at: eve_view::MouseAt { view, row, col },
        }));
    }

    /// 左クリック (押して離す)。
    pub fn click(&mut self, row: i32, col: i32) {
        use eve_view::{Modifiers, MouseButton, MouseKind};
        self.mouse(
            MouseKind::Down(MouseButton::Left),
            Modifiers::NONE,
            row,
            col,
        );
        self.mouse(MouseKind::Up(MouseButton::Left), Modifiers::NONE, row, col);
    }

    /// ブラケットペースト。
    pub fn paste(&mut self, s: &str) {
        self.input(InputEvent::Paste(s.to_owned()));
    }

    /// コマンドを実行する (`CommandId`、07 §11.6)。
    pub fn cmd(&mut self, id: &str) -> CommandResult {
        self.cmd_with(id, Args::new())
    }

    pub fn cmd_with(&mut self, id: &str, args: Args) -> CommandResult {
        self.editor.tick(self.clock.now());
        let r = self
            .editor
            .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(args));
        self.collect();
        r
    }

    /// 確認ダイアログの選択肢 (0 始まりの並び順) を選ぶ。
    pub fn answer(&mut self, choice: usize) -> CommandResult {
        let n = i64::try_from(choice).expect("選択肢の番号");
        self.cmd_with("ui.dialog_choose", Args::new().with_int("choice", n))
    }

    /// `ManualClock` を進めて `Editor` に時刻を渡す (Undo の結合・保留のキーの期限など)。
    pub fn advance_ms(&mut self, ms: u64) {
        self.clock.advance(Millis(ms));
        self.editor.tick(self.clock.now());
        self.collect();
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.input(InputEvent::Resize { cols, rows });
    }

    /// 積まれたジョブを実行し、結果を `Editor` に渡す。新たに積まれたジョブがなくなるまで続ける。
    pub fn run_jobs(&mut self) {
        loop {
            self.jobs.run_jobs();
            let results = self.jobs.take_results();
            if results.is_empty() {
                break;
            }
            for r in results {
                self.editor.tick(self.clock.now());
                self.editor.on_job_result(r);
            }
        }
        self.collect();
    }

    /// 実行待ちのジョブを 1 周だけ実行し、結果を `Editor` に渡す (結果から再投入されたジョブは走らせない)。
    pub fn run_jobs_once(&mut self) {
        self.jobs.run_jobs();
        for r in self.jobs.take_results() {
            self.editor.tick(self.clock.now());
            self.editor.on_job_result(r);
        }
        self.collect();
    }

    /// 実行待ちのジョブをすべて取り消す (結果は次の `run_jobs` で `Editor` に届く)。
    pub fn cancel_jobs(&mut self) {
        self.jobs.cancel_all();
    }

    /// 実行待ちのジョブの数 (「保存が走っている最中」の検査用)。
    pub fn pending_jobs(&self) -> usize {
        self.jobs.pending()
    }

    fn input(&mut self, ev: InputEvent) {
        self.editor.tick(self.clock.now());
        self.editor.handle_input(self.window, ev);
        self.collect();
    }

    fn collect(&mut self) {
        self.events.extend(self.editor.drain_events());
        self.effects.extend(self.editor.drain_effects());
    }

    // ---- 検査 ----

    /// 読み取り専用の問い合わせ (`Editor::query`、NFR-707 AC-4)。
    pub fn query(&self, q: &Query) -> Result<Answer, QueryError> {
        self.editor.query(q)
    }

    pub fn doc_id(&self) -> DocumentId {
        self.editor
            .state()
            .active_document()
            .expect("今の文書がある")
    }

    pub fn doc_text(&self) -> String {
        self.editor
            .state()
            .document(self.doc_id())
            .expect("文書がある")
            .to_string_lossy()
    }

    pub fn doc_path(&self) -> Option<String> {
        self.editor
            .state()
            .document(self.doc_id())
            .and_then(|d| d.path.as_ref().map(|p| p.display()))
    }

    pub fn is_dirty(&self) -> bool {
        self.editor
            .state()
            .document(self.doc_id())
            .is_some_and(|d| d.is_dirty())
    }

    /// 偽 FS の内容。
    pub fn fs_read(&self, path: &str) -> Option<Vec<u8>> {
        self.fs.read(path)
    }

    /// 出ているオーバーレイの種類 (`""` / `"dialog"` / `"prompt"`)。
    pub fn overlay(&self) -> &'static str {
        self.editor.state().overlay_kind()
    }

    pub fn frame(&mut self) -> Frame {
        self.editor.frame(self.window).expect("Frame が作れる")
    }

    /// ASCII の Frame (クロームと選択とカーソルつき。11 §6.5)。
    pub fn frame_text(&mut self) -> String {
        self.frame_text_with(AsciiOptions {
            show_chrome: true,
            ..AsciiOptions::default()
        })
    }

    pub fn frame_text_with(&mut self, opts: AsciiOptions) -> String {
        render_ascii(&self.frame(), opts)
    }

    pub fn frame_contains(&mut self, needle: &str) -> bool {
        self.frame_text().contains(needle)
    }

    /// これまでの `EditorEvent` (`clear_log` まで蓄える)。
    pub fn events(&self) -> &[EditorEvent] {
        &self.events
    }

    /// これまでの `Effect`。
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    /// `MessageShown` の MessageKey の列。
    pub fn messages(&self) -> Vec<String> {
        self.events
            .iter()
            .filter_map(|ev| match ev {
                EditorEvent::MessageShown { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect()
    }

    /// `Effect::Quit` が出ていればその終了コード。
    pub fn quit_code(&self) -> Option<u8> {
        self.effects.iter().find_map(|e| match e {
            Effect::Quit { code, .. } => Some(*code),
            _ => None,
        })
    }

    /// 蓄えた `EditorEvent` と `Effect` を捨てる (以降の検査を区切る)。
    pub fn clear_log(&mut self) {
        self.events.clear();
        self.effects.clear();
    }
}
