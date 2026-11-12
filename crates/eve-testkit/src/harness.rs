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

/// `Editor` を偽 Port でヘッドレスに駆動する入口。入力を渡し、`Editor` の状態・`EditorEvent`・`Effect`・Frame を検査する。
pub struct Harness {
    /// 駆動している `Editor`。
    pub editor: Editor,
    /// 偽の `FileSystem` (障害注入や外部変更の再現に直接触れる)。
    pub fs: Arc<MemFs>,
    /// 手動の時計 (`advance_ms` と同じものを直接進めることもできる)。
    pub clock: ManualClock,
    jobs: Rc<ManualJobSpawner>,
    window: WindowId,
    /// 組み立てのときの画面の大きさ (`started` で渡す)。
    size: (u16, u16),
    events: Vec<EditorEvent>,
    effects: Vec<Effect>,
}

/// `Harness::builder()` の組み立て。起動の前に効かせたいもの (端末の能力・設定・言語・偽 FS のファイル) を置く。
///
/// ```ignore
/// let mut h = Harness::builder()
///     .size(100, 30)
///     .file("/w/a.txt", b"aaa\n")
///     .locale(Locale::En)
///     .build();
/// ```
#[derive(Default)]
pub struct HarnessBuilder {
    size: Option<(u16, u16)>,
    caps: Option<Capabilities>,
    config: Option<eve_config::Config>,
    locale: Option<eve_config::Locale>,
    files: Vec<(String, Vec<u8>)>,
    dirs: Vec<String>,
    defer_start: bool,
}

impl HarnessBuilder {
    /// 画面の大きさ (既定は 80×24)。
    pub fn size(mut self, cols: u16, rows: u16) -> Self {
        self.size = Some((cols, rows));
        self
    }

    /// 端末の能力 (`Editor::new` に渡す。既定は `Capabilities::default()`)。
    pub fn caps(mut self, caps: Capabilities) -> Self {
        self.caps = Some(caps);
        self
    }

    /// 設定 (起動の前に `set_config` する)。
    pub fn config(mut self, config: eve_config::Config) -> Self {
        self.config = Some(config);
        self
    }

    /// UI の言語 (既定は `ja`)。
    pub fn locale(mut self, locale: eve_config::Locale) -> Self {
        self.locale = Some(locale);
        self
    }

    /// 偽 FS にファイルを置く (`/w` は常にある)。
    pub fn file(mut self, path: &str, data: &[u8]) -> Self {
        self.files.push((path.to_owned(), data.to_vec()));
        self
    }

    /// 偽 FS にディレクトリを置く。
    pub fn dir(mut self, path: &str) -> Self {
        self.dirs.push(path.to_owned());
        self
    }

    /// 起動 (`open_initial`) をしない。試験の側で `h.editor.open_initial*` を呼び、続けて `h.started()` を呼ぶ
    /// (起動の引数・起動時の走査を確かめる試験)。
    pub fn defer_start(mut self) -> Self {
        self.defer_start = true;
        self
    }

    /// 組み立てる。`defer_start` でなければ、無題の文書を開いて起動を済ませる。
    pub fn build(self) -> Harness {
        Harness::assemble(self)
    }
}

impl Default for Harness {
    fn default() -> Harness {
        Harness::new()
    }
}

impl Harness {
    /// 無題の文書が 1 つ開いた状態。保存先の `/w` があり、画面は 80×24。
    pub fn new() -> Harness {
        Harness::builder().build()
    }

    /// 組み立て方を変えた Harness (起動の前に置く設定・言語・端末の能力・ファイル、起動を遅らせるか)。
    pub fn builder() -> HarnessBuilder {
        HarnessBuilder::default()
    }

    fn assemble(b: HarnessBuilder) -> Harness {
        let clock = ManualClock::new();
        let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
        fs.add_dir("/w");
        for d in &b.dirs {
            fs.add_dir(d);
        }
        for (p, data) in &b.files {
            fs.add_file(p, data);
        }
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
        let mut editor = Editor::new(ports, b.caps.unwrap_or_default());
        if let Some(config) = b.config {
            editor.configure(eve_core::StartupConfig {
                config: Some(config),
                ..Default::default()
            });
        }
        if let Some(locale) = b.locale {
            editor.configure(eve_core::StartupConfig {
                locale: Some(locale),
                ..Default::default()
            });
        }
        let mut h = Harness {
            editor,
            fs,
            clock,
            jobs,
            window: WindowId::default(),
            size: b.size.unwrap_or((80, 24)),
            events: Vec::new(),
            effects: Vec::new(),
        };
        if !b.defer_start {
            h.editor.open_initial(None, false);
            h.started();
        }
        h
    }

    /// 起動 (`Editor::open_initial` / `open_initial_files`) を試験の側で呼んだ後に呼ぶ。最初の Window を覚え、
    /// 組み立てのときの大きさを渡す。`HarnessBuilder::defer_start` のときに使う。
    ///
    /// # Panics
    ///
    /// `Editor` に最初の Window が無いとき (`open_initial` を呼んでいない)。
    pub fn started(&mut self) {
        self.window = self.editor.state().first_window().expect("起動時の Window");
        let (cols, rows) = self.size;
        self.resize(cols, rows);
    }

    /// 入力を渡す Window (起動時の Window)。
    /// `defer_start` で `started` をまだ呼んでいなければ、Editor の最初の Window (起動の後なら有る)。
    pub fn window(&self) -> WindowId {
        if self.window == WindowId::default()
            && let Some(w) = self.editor.state().first_window()
        {
            return w;
        }
        self.window
    }

    // ---- 初期状態 ----

    /// 偽 FS にファイルを置く (開くのは `open`)。
    pub fn with_file(self, path: &str, data: &[u8]) -> Harness {
        self.fs.add_file(path, data);
        self
    }

    /// 端末の能力を差し替える。
    pub fn with_caps(mut self, caps: Capabilities) -> Harness {
        self.editor.set_caps(caps);
        self
    }

    /// 画面の大きさ (列, 行) を変える (`resize` を送る)。
    pub fn with_size(mut self, cols: u16, rows: u16) -> Harness {
        self.resize(cols, rows);
        self
    }

    /// UI の言語を選ぶ (既定は `ja`)。
    pub fn with_locale(mut self, locale: eve_config::Locale) -> Harness {
        self.editor.configure(eve_core::StartupConfig {
            locale: Some(locale),
            ..Default::default()
        });
        self
    }

    // ---- 入力 ----

    /// `file.open`。ジョブは `run_jobs` で完了する。
    pub fn open(&mut self, path: &str) -> CommandResult {
        self.cmd_with("file.open", Args::new().with_str("path", path))
    }

    /// 07 §7.2 の設定表記 (小文字・`+`・空白区切り) のキー列を押す。`mod` は `ctrl`。不正な表記は panic する。
    ///
    /// # Panics
    ///
    /// `seq` に読めないキー表記があるとき。
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
    ///
    /// # Panics
    ///
    /// 最初の Window に View が無いとき。
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

    /// 引数付きでコマンドを実行する。実行の前に時刻を `Editor` に渡し、実行の後に `EditorEvent` と `Effect` を蓄える。
    pub fn cmd_with(&mut self, id: &str, args: Args) -> CommandResult {
        self.editor.tick(self.clock.now());
        let r = self
            .editor
            .execute(CommandInvocation::new(id, InvocationSource::Keymap).with_args(args));
        self.collect();
        r
    }

    /// 確認ダイアログの選択肢 (0 始まりの並び順) を選ぶ。
    ///
    /// # Panics
    ///
    /// `choice` が `i64` に収まらないとき。
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

    /// 画面の大きさ (列, 行) を変える (`InputEvent::Resize`)。
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

    /// `run_jobs` と同じだが、各結果を `Editor` に渡す前に `seen` に見せる (途中経過・先行表示の先頭など、`Editor` の
    /// 状態に残らない結果を確かめる試験用)。
    pub fn run_jobs_with(&mut self, mut seen: impl FnMut(&eve_core::JobOutput)) {
        loop {
            self.jobs.run_jobs();
            let results = self.jobs.take_results();
            if results.is_empty() {
                break;
            }
            for r in results {
                seen(&r);
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
        let w = self.window();
        self.editor.handle_input(w, ev);
        self.collect();
    }

    fn collect(&mut self) {
        self.events.extend(self.editor.drain_events());
        self.effects.extend(self.editor.drain_effects());
    }

    // ---- 検査 ----

    /// 読み取り専用の問い合わせ (`Editor::query`、NFR-707 AC-4)。
    ///
    /// # Errors
    ///
    /// `Editor::query` が返す `QueryError` (存在しない文書・View・Window、読込が終わっていない文書、範囲外など)。
    pub fn query(&self, q: &Query) -> Result<Answer, QueryError> {
        self.editor.query(q)
    }

    /// 今の文書の ID。
    ///
    /// # Panics
    ///
    /// 今の文書が無いとき。
    pub fn doc_id(&self) -> DocumentId {
        self.editor
            .state()
            .active_document()
            .expect("今の文書がある")
    }

    /// 今の文書の本文 (不正な UTF-8 は置き換える)。
    ///
    /// # Panics
    ///
    /// 今の文書が無いとき。
    pub fn doc_text(&self) -> String {
        self.editor
            .state()
            .document(self.doc_id())
            .expect("文書がある")
            .to_string_lossy()
    }

    /// 今の文書のパス (表示の形)。無題なら `None`。
    pub fn doc_path(&self) -> Option<String> {
        self.editor
            .state()
            .document(self.doc_id())
            .and_then(|d| d.path.as_ref().map(|p| p.display()))
    }

    /// 今の文書が未保存の変更を持つか。
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

    /// 出ているオーバーレイの種類 (`EditorState::overlay_kind` の値。11 §6.3 の `overlay`):
    /// `""` (なし) / `"dialog"` / `"prompt"` / `"palette"` / `"buffer_list"` / `"help"`。
    pub fn overlay(&self) -> &'static str {
        self.editor.state().overlay_kind()
    }

    /// 今の Window の `Frame`。
    ///
    /// # Panics
    ///
    /// `Frame` を作れないとき。
    pub fn frame(&mut self) -> Frame {
        self.editor.frame(self.window()).expect("Frame が作れる")
    }

    /// ASCII の Frame (クロームと選択とカーソルつき。11 §6.5)。
    pub fn frame_text(&mut self) -> String {
        self.frame_text_with(AsciiOptions {
            show_chrome: true,
            ..AsciiOptions::default()
        })
    }

    /// `opts` を指定した ASCII の Frame (`frame_text` は既定の選び方にクロームを足したもの)。
    pub fn frame_text_with(&mut self, opts: AsciiOptions) -> String {
        render_ascii(&self.frame(), opts)
    }

    /// ASCII の Frame (`frame_text`) に `needle` が含まれるか。
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

    /// `MessageShown` の `MessageKey` の列。
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
