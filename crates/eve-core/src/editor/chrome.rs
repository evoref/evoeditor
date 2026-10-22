//! クローム (ステータスバー・ヒントバー・小さすぎる端末の文言) のモデル (docs/04 §8.7 / §8.8、docs/02 §8.2)。
//! 文言は core が言語ファイルから解決し、幅も計算して渡す。frontend は配置と省略だけを行う。

use eve_text::{ByteOff, ByteRange, TextRead, grapheme_indices};
use eve_view::{
    Choice, ChoiceId, ChromeTexts, CutPolicy, HintContext, HintItem, HintModel, InputFieldModel,
    KeyCode, KeyLabel, KeyStroke, Modifiers, OverlayModel, StatusAlign, StatusItem, StatusKind,
    StatusModel, ViewId,
};

use crate::document::{Document, LoadState};

use super::layout::cell_text;
use super::state::EditorState;

/// ヒントバーの `normal` 文脈の項目 (07 §11.6 の「ヒント」列。CommandId と優先度)。
const NORMAL_HINTS: [(&str, u8); 7] = [
    ("app.quit", 5),
    ("ui.palette_open", 5),
    ("file.save", 4),
    ("search.open", 3),
    ("file.open", 2),
    ("edit.undo", 2),
    ("cursor.goto_line", 1),
];

fn item(
    kind: StatusKind,
    long: &str,
    short: Option<&str>,
    priority: u8,
    align: StatusAlign,
    cut: CutPolicy,
) -> StatusItem {
    StatusItem {
        kind,
        long: cell_text(long, cut),
        short: short.map(|s| cell_text(s, CutPolicy::Never)),
        priority,
        align,
    }
}

fn render(state: &EditorState, key: &str, args: &[(&str, &str)]) -> String {
    let label = |c: &str| state.key_label(c);
    state.catalog().render(key, args, &label)
}

/// 行頭からの書記素数 + 1 (桁。書記素数、04 の `ui.column_unit = "grapheme"`)。
fn grapheme_column(doc: &Document, head: ByteOff) -> usize {
    let t = doc.text();
    let line = t.byte_to_line(head);
    let ls = t.line_to_byte(line);
    let prefix = t.slice_to_string(ByteRange::new(ls, head));
    grapheme_indices(&prefix).len() + 1
}

pub(super) fn status_model(
    state: &EditorState,
    view_id: ViewId,
    chord: &[KeyStroke],
) -> StatusModel {
    let mut items = Vec::new();
    let Some(view) = state.view(view_id) else {
        return StatusModel::default();
    };
    let Some(doc) = state.document(view.doc) else {
        return StatusModel::default();
    };
    use StatusAlign::{Left, Right};

    // 左群: ファイル名・変更・読み取り専用・読込中・保存中・チョード
    let name = match doc.path.as_ref().and_then(|p| p.file_name()) {
        Some(n) => n,
        None => render(state, "status.untitled", &[("n", "1")]),
    };
    items.push(item(
        StatusKind::Path,
        &name,
        None,
        4,
        Left,
        CutPolicy::Middle,
    ));
    if doc.is_dirty() {
        items.push(item(
            StatusKind::Dirty,
            "[+]",
            None,
            5,
            Left,
            CutPolicy::Never,
        ));
    }
    if !doc.read_only.is_empty() {
        let (long, short) = (
            render(state, "status.read_only.long", &[]),
            render(state, "status.read_only.short", &[]),
        );
        items.push(item(
            StatusKind::ReadOnly,
            &long,
            Some(&short),
            5,
            Left,
            CutPolicy::Never,
        ));
    }
    if doc.load_state == LoadState::Loading {
        let a = [("percent", "0")];
        let (long, short) = (
            render(state, "status.loading.long", &a),
            render(state, "status.loading.short", &a),
        );
        items.push(item(
            StatusKind::Loading,
            &long,
            Some(&short),
            5,
            Left,
            CutPolicy::Never,
        ));
    }
    if state.is_saving(view.doc) {
        let (long, short) = (
            render(state, "status.saving.long", &[]),
            render(state, "status.saving.short", &[]),
        );
        items.push(item(
            StatusKind::Saving,
            &long,
            Some(&short),
            5,
            Left,
            CutPolicy::Never,
        ));
    }
    if !chord.is_empty() {
        let text = format!("{} …", super::keymap::format_sequence(chord));
        items.push(item(
            StatusKind::Chord,
            &text,
            None,
            5,
            Left,
            CutPolicy::Never,
        ));
    }

    // 右群: 選択・位置・文字コード・改行コード
    let r = view.selection.primary();
    if !r.is_empty() {
        let t = doc.text();
        let n = t.byte_to_char(r.end()).0 - t.byte_to_char(r.start()).0;
        let text = render(state, "status.selection", &[("count", &n.to_string())]);
        items.push(item(
            StatusKind::Selection,
            &text,
            None,
            2,
            Right,
            CutPolicy::Never,
        ));
    }
    let line = doc.text().byte_to_line(r.head).0 + 1;
    let a = [
        ("line", line.to_string()),
        ("col", grapheme_column(doc, r.head).to_string()),
    ];
    let a: Vec<(&str, &str)> = a.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let (long, short) = (
        render(state, "status.position.long", &a),
        render(state, "status.position.short", &a),
    );
    items.push(item(
        StatusKind::Position,
        &long,
        Some(&short),
        4,
        Right,
        CutPolicy::Never,
    ));
    let (name, short) = (doc.format.encoding.name(), doc.format.encoding.short_name());
    let with_bom = |n: &str| {
        if doc.format.has_bom {
            render(state, "status.encoding.bom", &[("encoding", n)])
        } else {
            n.to_owned()
        }
    };
    let (enc, enc_short) = (with_bom(name), with_bom(short));
    items.push(item(
        StatusKind::Encoding,
        &enc,
        (enc_short != enc).then_some(enc_short.as_str()),
        3,
        Right,
        CutPolicy::Never,
    ));
    let eol = if doc.format.eol_stats.is_mixed() {
        render(state, "status.eol_mixed", &[])
    } else {
        match doc.format.default_eol {
            eve_text::LineEnding::Lf => "LF",
            eve_text::LineEnding::Crlf => "CRLF",
            eve_text::LineEnding::Cr => "CR",
        }
        .to_owned()
    };
    items.push(item(
        StatusKind::Eol,
        &eol,
        None,
        3,
        Right,
        CutPolicy::Never,
    ));
    StatusModel { items }
}

/// オーバーレイのモデル (確認・入力プロンプト)。文言は core が解決して `CellText` で渡す。
pub(super) fn overlay_models(state: &EditorState) -> Vec<OverlayModel> {
    use super::overlay::Overlay;
    match &state.overlay {
        None => Vec::new(),
        Some(Overlay::Confirm(c)) => {
            let key = |leaf: &str| format!("ui.confirm.{}.{leaf}", c.name);
            let args: Vec<(&str, &str)> = c
                .args
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let mut choices: Vec<Choice> = c
                .choices
                .iter()
                .enumerate()
                .map(|(i, d)| Choice {
                    id: ChoiceId(i as u16),
                    label: cell_text(&render(state, &key(d.id), &args), CutPolicy::Never),
                    key: d.accel,
                    destructive: d.destructive,
                })
                .collect();
            // キャンセルは最後 (`Esc`)
            choices.push(Choice {
                id: ChoiceId(c.cancel_index() as u16),
                label: cell_text(&render(state, "ui.confirm.cancel", &[]), CutPolicy::Never),
                key: None,
                destructive: false,
            });
            vec![OverlayModel::Confirm {
                title: cell_text(&render(state, &key("title"), &[]), CutPolicy::Never),
                message: cell_text(&render(state, &key("message"), &args), CutPolicy::End),
                details: (!c.details.is_empty()).then(|| {
                    c.details
                        .iter()
                        .map(|l| cell_text(l, CutPolicy::End))
                        .collect()
                }),
                choices,
                default: ChoiceId(c.default as u16),
                focused: ChoiceId(c.focused as u16),
                nav_hint: Some(cell_text(
                    &render(state, "ui.confirm.nav_hint", &[]),
                    CutPolicy::End,
                )),
            }]
        }
        Some(Overlay::Palette(p)) => vec![state.palette_model(p)],
        Some(Overlay::Prompt(p)) => {
            let name = p.kind.name();
            let cells = cell_text(&p.text, CutPolicy::End).spans;
            let cursor = cell_text(&p.text[..p.cursor], CutPolicy::End).spans.len();
            let input = InputFieldModel {
                len: cells.len(),
                cells,
                first: 0,
                cursor,
                selection: None,
                placeholder: None,
            };
            let hint_key = format!("ui.prompt.{name}.hint");
            let hint = state
                .catalog()
                .contains(&hint_key)
                .then(|| cell_text(&render(state, &hint_key, &[]), CutPolicy::End));
            let error = p.error.map(|kind| {
                cell_text(
                    &render(state, &format!("ui.prompt.{name}.error.{kind}"), &[]),
                    CutPolicy::End,
                )
            });
            vec![OverlayModel::Prompt {
                title: cell_text(
                    &render(state, &format!("ui.prompt.{name}.title"), &[]),
                    CutPolicy::Never,
                ),
                input,
                hint,
                error,
            }]
        }
    }
}

pub(super) fn hint_model(state: &EditorState, chord: &[KeyStroke]) -> HintModel {
    if state.overlay.is_some() || state.search_bar.is_some() {
        // オーバーレイ: 決定・取消 (確認は次へも)。キーは固定 (オーバーレイの割当は既定のまま)
        let stroke = |key| KeyLabel {
            strokes: vec![KeyStroke {
                mods: Modifiers::NONE,
                key,
            }],
        };
        let word = |cmd: &str| {
            cell_text(
                state
                    .catalog()
                    .get(&format!("hint.{cmd}"))
                    .unwrap_or("hint"),
                CutPolicy::Never,
            )
        };
        let mut items = vec![
            HintItem {
                keys: vec![stroke(KeyCode::Esc)],
                text: word("ui.cancel"),
                priority: 5,
            },
            HintItem {
                keys: vec![stroke(KeyCode::Enter)],
                text: word("ui.confirm"),
                priority: 4,
            },
        ];
        if state.overlay.is_none() {
            // 検索バー: Enter は次の一致、Alt+C は大小の切替
            items[1].text = word("search.next");
            items.push(HintItem {
                keys: vec![KeyLabel {
                    strokes: vec![KeyStroke {
                        mods: Modifiers::ALT,
                        key: KeyCode::Char('c'),
                    }],
                }],
                text: word("search.toggle_case"),
                priority: 2,
            });
        }
        if state.is_dialog() {
            items.push(HintItem {
                keys: vec![stroke(KeyCode::Tab)],
                text: word("ui.list_next"),
                priority: 3,
            });
        }
        return HintModel {
            context: HintContext::Overlay,
            items,
        };
    }
    if !chord.is_empty() {
        // チョード保留中は `Esc 取消` だけを出す (04 §8.8)
        let esc = KeyLabel {
            strokes: vec![KeyStroke {
                mods: Modifiers::NONE,
                key: KeyCode::Esc,
            }],
        };
        let text = cell_text(
            state
                .catalog()
                .get("hint.ui.cancel")
                .unwrap_or("hint.ui.cancel"),
            CutPolicy::Never,
        );
        return HintModel {
            context: HintContext::Overlay,
            items: vec![HintItem {
                keys: vec![esc],
                text,
                priority: 5,
            }],
        };
    }
    let mut items: Vec<HintItem> = NORMAL_HINTS
        .iter()
        .filter_map(|&(cmd, priority)| {
            // 届く割当が無い項目は出さない
            let strokes = state.key_sequence(cmd)?;
            let word = state.catalog().get(&format!("hint.{cmd}"))?;
            Some(HintItem {
                keys: vec![KeyLabel { strokes }],
                text: cell_text(word, CutPolicy::Never),
                priority,
            })
        })
        .collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.priority));
    HintModel {
        context: HintContext::Normal,
        items,
    }
}

pub(super) fn chrome_texts(state: &EditorState) -> ChromeTexts {
    let min = state.caps.min_surface;
    let (cols, rows) = (min.cols.to_string(), min.rows.to_string());
    let text = render(state, "ui.too_small", &[("cols", &cols), ("rows", &rows)]);
    ChromeTexts {
        too_small: cell_text(&text, CutPolicy::Never),
    }
}
