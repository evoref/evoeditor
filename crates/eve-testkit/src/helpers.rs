//! 試験で繰り返し書いていた小さな補助 (docs/review/v0.3-findings.md の STR-F1)。

use eve_view::CellText;

use crate::Harness;

/// `CellText` の文字だけをつなげる (スタイルは捨てる)。
pub fn cell_text(t: &CellText) -> String {
    t.spans.iter().map(|s| s.text.to_string()).collect()
}

/// `path` にファイルを置き、開いて読込を終えた Harness (80×24)。
pub fn opened(path: &str, bytes: &[u8]) -> Harness {
    let mut h = Harness::new().with_file(path, bytes);
    h.open(path);
    h.run_jobs();
    h
}

impl Harness {
    /// 最初の Window の今の View の主選択の範囲 (バイト位置の `(開始, 終了)`。向きは捨てる)。
    pub fn selection_range(&self) -> (usize, usize) {
        let st = self.editor.state();
        let w = st.first_window().expect("Window がある");
        let r = *st
            .view(st.window_view(w).expect("View がある"))
            .expect("View がある")
            .selection
            .primary();
        (r.start().0, r.end().0)
    }

    /// 文書 `doc` の本文 (今の文書は `doc_text`)。
    pub fn text_of(&self, doc: eve_view::DocumentId) -> String {
        self.editor
            .state()
            .document(doc)
            .expect("文書がある")
            .to_string_lossy()
    }
}
