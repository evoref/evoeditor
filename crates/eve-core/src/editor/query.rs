//! `Editor::query`: 読み取り専用の問い合わせ (docs/02 §10.1 X18、01 NFR-707 AC-4 / AC-6)。副作用がない。
//! 同じ状態 (同じ Revision・選択・オーバーレイ) に同じ問い合わせをすれば同じ応答を返す。

use eve_text::{ByteOff, ByteRange as TextRange, TextRead};
use eve_view::{
    Answer, ByteRange, DocumentInfo, FocusTarget, LoadStateView, Query, QueryError, SelRange,
    ViewId, WindowId, WindowInfo,
};

use crate::document::{LoadState, ReadOnlyReason};

use super::{Editor, chrome};

impl Editor {
    pub fn query(&self, q: &Query) -> Result<Answer, QueryError> {
        let state = &self.state;
        match q {
            Query::Documents => Ok(Answer::Documents(
                state
                    .documents
                    .iter()
                    .map(|(id, d)| DocumentInfo {
                        doc: *id,
                        path: d.path.as_ref().map(|p| p.display()),
                        untitled_name: d.path.is_none().then(|| {
                            state
                                .catalog()
                                .render("status.untitled", &[("n", "1")], &|_| None)
                        }),
                        revision: d.revision(),
                        dirty: d.is_dirty(),
                        load_state: match d.load_state {
                            LoadState::Loading => LoadStateView::Loading,
                            LoadState::Ready => LoadStateView::Ready,
                            LoadState::Failed => LoadStateView::Failed,
                        },
                        read_only: d.read_only.iter().map(reason_name).collect(),
                        views: state
                            .views
                            .iter()
                            .filter(|(_, v)| v.doc == *id)
                            .map(|(vid, _)| *vid)
                            .collect(),
                    })
                    .collect(),
            )),
            Query::Windows => Ok(Answer::Windows(
                state
                    .windows
                    .iter()
                    .map(|w| WindowInfo {
                        window: w.id,
                        views: vec![w.root],
                        focus: if state.overlay.is_some() {
                            FocusTarget::Overlay(0)
                        } else {
                            FocusTarget::Pane(w.root)
                        },
                    })
                    .collect(),
            )),
            Query::DocumentText { doc, range } => {
                let d = state
                    .document(*doc)
                    .ok_or(QueryError::UnknownDocument(*doc))?;
                if d.load_state != LoadState::Ready {
                    return Err(QueryError::NotLoaded(*doc));
                }
                let t = d.text();
                let len = t.len_bytes() as u64;
                let r = range.unwrap_or(ByteRange::new(0, len));
                if r.start > r.end || r.end > len {
                    return Err(QueryError::RangeOutOfBounds { len });
                }
                let (s, e) = (to_off(r.start), to_off(r.end));
                if !t.is_char_boundary(s) || !t.is_char_boundary(e) {
                    return Err(QueryError::RangeOutOfBounds { len });
                }
                Ok(Answer::DocumentText {
                    doc: *doc,
                    revision: d.revision(),
                    range: r,
                    text: t.slice_to_string(TextRange::new(s, e)),
                })
            }
            Query::Selection { view } => {
                let v = state.view(*view).ok_or(QueryError::UnknownView(*view))?;
                let d = state
                    .document(v.doc)
                    .ok_or(QueryError::UnknownDocument(v.doc))?;
                if d.load_state != LoadState::Ready {
                    return Err(QueryError::NotLoaded(v.doc));
                }
                Ok(Answer::Selection {
                    view: *view,
                    doc: v.doc,
                    revision: d.revision(),
                    ranges: v
                        .selection
                        .ranges()
                        .iter()
                        .map(|r| SelRange {
                            anchor: r.anchor.0 as u64,
                            head: r.head.0 as u64,
                        })
                        .collect(),
                    primary: v.selection.primary_index(),
                })
            }
            Query::Capabilities => Ok(Answer::Capabilities(state.caps.clone())),
            Query::Status { window } => {
                let view = self.window_root(*window)?;
                Ok(Answer::Status(chrome::status_model(
                    state,
                    view,
                    self.keymap.pending(),
                )))
            }
            Query::Overlay { window } => {
                self.window_root(*window)?;
                Ok(Answer::Overlay(chrome::overlay_models(state).pop()))
            }
            _ => Err(QueryError::Unsupported),
        }
    }

    fn window_root(&self, window: WindowId) -> Result<ViewId, QueryError> {
        self.state
            .window_view(window)
            .ok_or(QueryError::UnknownWindow(window))
    }
}

fn to_off(n: u64) -> ByteOff {
    ByteOff(usize::try_from(n).unwrap_or(usize::MAX))
}

fn reason_name(r: ReadOnlyReason) -> String {
    format!("{r:?}")
}
