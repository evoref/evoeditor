//! 組込みのコマンドの ID (07 §11.6 のカタログの CommandId。docs/review/v0.3-findings.md の STR-B4 (2))。
//! 製品のコードは ID を文字列で書かず、ここの定数を使う (書き間違いをコンパイルで止める)。既定のキーマップ
//! (`keymap/default_keymap.rs`) は 07 §11.6 の表と照らすので文字列のまま。`ALL` が登録と一致することを試験で確かめる。

pub const APP_QUIT: &str = "app.quit";
pub const APP_SUSPEND: &str = "app.suspend";
pub const BUFFER_CLOSE: &str = "buffer.close";
pub const BUFFER_LIST: &str = "buffer.list";
pub const BUFFER_NEXT: &str = "buffer.next";
pub const BUFFER_PREV: &str = "buffer.prev";
pub const CLIPBOARD_COPY: &str = "clipboard.copy";
pub const CLIPBOARD_CUT: &str = "clipboard.cut";
pub const CONFIG_RELOAD: &str = "config.reload";
pub const CURSOR_DOC_END: &str = "cursor.doc_end";
pub const CURSOR_DOC_START: &str = "cursor.doc_start";
pub const CURSOR_END: &str = "cursor.end";
pub const CURSOR_GOTO_LINE: &str = "cursor.goto_line";
pub const CURSOR_HOME: &str = "cursor.home";
pub const CURSOR_MOVE_DOWN: &str = "cursor.move_down";
pub const CURSOR_MOVE_LEFT: &str = "cursor.move_left";
pub const CURSOR_MOVE_RIGHT: &str = "cursor.move_right";
pub const CURSOR_MOVE_UP: &str = "cursor.move_up";
pub const CURSOR_MOVE_WORD_LEFT: &str = "cursor.move_word_left";
pub const CURSOR_MOVE_WORD_RIGHT: &str = "cursor.move_word_right";
pub const CURSOR_PAGE_DOWN: &str = "cursor.page_down";
pub const CURSOR_PAGE_UP: &str = "cursor.page_up";
pub const EDIT_DELETE_BACKWARD: &str = "edit.delete_backward";
pub const EDIT_DELETE_FORWARD: &str = "edit.delete_forward";
pub const EDIT_DELETE_LINE: &str = "edit.delete_line";
pub const EDIT_DELETE_WORD_BACKWARD: &str = "edit.delete_word_backward";
pub const EDIT_DELETE_WORD_FORWARD: &str = "edit.delete_word_forward";
pub const EDIT_DUPLICATE_LINE: &str = "edit.duplicate_line";
pub const EDIT_INDENT: &str = "edit.indent";
pub const EDIT_INSERT_TEXT: &str = "edit.insert_text";
pub const EDIT_MOVE_LINE_DOWN: &str = "edit.move_line_down";
pub const EDIT_MOVE_LINE_UP: &str = "edit.move_line_up";
pub const EDIT_NEWLINE: &str = "edit.newline";
pub const EDIT_OUTDENT: &str = "edit.outdent";
pub const EDIT_PASTE: &str = "edit.paste";
pub const EDIT_REDO: &str = "edit.redo";
pub const EDIT_UNDO: &str = "edit.undo";
pub const FILE_CHECK_EXTERNAL_CHANGES: &str = "file.check_external_changes";
pub const FILE_NEW: &str = "file.new";
pub const FILE_OPEN: &str = "file.open";
pub const FILE_RELOAD: &str = "file.reload";
pub const FILE_REOPEN_WITH_ENCODING: &str = "file.reopen_with_encoding";
pub const FILE_SAVE: &str = "file.save";
pub const FILE_SAVE_ALL: &str = "file.save_all";
pub const FILE_SAVE_AS: &str = "file.save_as";
pub const FILE_SET_ENCODING: &str = "file.set_encoding";
pub const FILE_SET_LINE_ENDING: &str = "file.set_line_ending";
pub const FILE_TOGGLE_READ_ONLY: &str = "file.toggle_read_only";
pub const HELP_COMMANDS: &str = "help.commands";
pub const HELP_KEYS: &str = "help.keys";
pub const HELP_OPEN: &str = "help.open";
pub const SEARCH_NEXT: &str = "search.next";
pub const SEARCH_OPEN: &str = "search.open";
pub const SEARCH_PREV: &str = "search.prev";
pub const SEARCH_REPLACE: &str = "search.replace";
pub const SEARCH_REPLACE_ALL: &str = "search.replace_all";
pub const SEARCH_REPLACE_OPEN: &str = "search.replace_open";
pub const SEARCH_TOGGLE_CASE: &str = "search.toggle_case";
pub const SEARCH_TOGGLE_REGEX: &str = "search.toggle_regex";
pub const SEARCH_TOGGLE_SELECTION: &str = "search.toggle_selection";
pub const SEARCH_TOGGLE_WORD: &str = "search.toggle_word";
pub const SELECT_ALL: &str = "select.all";
pub const SELECT_EXTEND_DOWN: &str = "select.extend_down";
pub const SELECT_EXTEND_END: &str = "select.extend_end";
pub const SELECT_EXTEND_HOME: &str = "select.extend_home";
pub const SELECT_EXTEND_LEFT: &str = "select.extend_left";
pub const SELECT_EXTEND_PAGE_DOWN: &str = "select.extend_page_down";
pub const SELECT_EXTEND_PAGE_UP: &str = "select.extend_page_up";
pub const SELECT_EXTEND_RIGHT: &str = "select.extend_right";
pub const SELECT_EXTEND_TO_DOC_END: &str = "select.extend_to_doc_end";
pub const SELECT_EXTEND_TO_DOC_START: &str = "select.extend_to_doc_start";
pub const SELECT_EXTEND_UP: &str = "select.extend_up";
pub const SELECT_EXTEND_WORD_LEFT: &str = "select.extend_word_left";
pub const SELECT_EXTEND_WORD_RIGHT: &str = "select.extend_word_right";
pub const UI_CANCEL: &str = "ui.cancel";
pub const UI_CONFIRM: &str = "ui.confirm";
pub const UI_DIALOG_CHOOSE: &str = "ui.dialog_choose";
pub const UI_LIST_NEXT: &str = "ui.list_next";
pub const UI_LIST_PAGE_DOWN: &str = "ui.list_page_down";
pub const UI_LIST_PAGE_UP: &str = "ui.list_page_up";
pub const UI_LIST_PREV: &str = "ui.list_prev";
pub const UI_MESSAGE_HISTORY: &str = "ui.message_history";
pub const UI_MESSAGE_HISTORY_COPY: &str = "ui.message_history_copy";
pub const UI_PALETTE_OPEN: &str = "ui.palette_open";
pub const UI_REDRAW: &str = "ui.redraw";
pub const UI_TOGGLE_HINT_BAR: &str = "ui.toggle_hint_bar";
pub const UI_TOGGLE_MOUSE: &str = "ui.toggle_mouse";
pub const VIEW_TOGGLE_WRAP: &str = "view.toggle_wrap";

/// 組込みのコマンドの ID のすべて (ID の順。登録と一致することを確かめる試験に使う)。
#[cfg(test)]
pub const ALL: [&str; 88] = [
    APP_QUIT,
    APP_SUSPEND,
    BUFFER_CLOSE,
    BUFFER_LIST,
    BUFFER_NEXT,
    BUFFER_PREV,
    CLIPBOARD_COPY,
    CLIPBOARD_CUT,
    CONFIG_RELOAD,
    CURSOR_DOC_END,
    CURSOR_DOC_START,
    CURSOR_END,
    CURSOR_GOTO_LINE,
    CURSOR_HOME,
    CURSOR_MOVE_DOWN,
    CURSOR_MOVE_LEFT,
    CURSOR_MOVE_RIGHT,
    CURSOR_MOVE_UP,
    CURSOR_MOVE_WORD_LEFT,
    CURSOR_MOVE_WORD_RIGHT,
    CURSOR_PAGE_DOWN,
    CURSOR_PAGE_UP,
    EDIT_DELETE_BACKWARD,
    EDIT_DELETE_FORWARD,
    EDIT_DELETE_LINE,
    EDIT_DELETE_WORD_BACKWARD,
    EDIT_DELETE_WORD_FORWARD,
    EDIT_DUPLICATE_LINE,
    EDIT_INDENT,
    EDIT_INSERT_TEXT,
    EDIT_MOVE_LINE_DOWN,
    EDIT_MOVE_LINE_UP,
    EDIT_NEWLINE,
    EDIT_OUTDENT,
    EDIT_PASTE,
    EDIT_REDO,
    EDIT_UNDO,
    FILE_CHECK_EXTERNAL_CHANGES,
    FILE_NEW,
    FILE_OPEN,
    FILE_RELOAD,
    FILE_REOPEN_WITH_ENCODING,
    FILE_SAVE,
    FILE_SAVE_ALL,
    FILE_SAVE_AS,
    FILE_SET_ENCODING,
    FILE_SET_LINE_ENDING,
    FILE_TOGGLE_READ_ONLY,
    HELP_COMMANDS,
    HELP_KEYS,
    HELP_OPEN,
    SEARCH_NEXT,
    SEARCH_OPEN,
    SEARCH_PREV,
    SEARCH_REPLACE,
    SEARCH_REPLACE_ALL,
    SEARCH_REPLACE_OPEN,
    SEARCH_TOGGLE_CASE,
    SEARCH_TOGGLE_REGEX,
    SEARCH_TOGGLE_SELECTION,
    SEARCH_TOGGLE_WORD,
    SELECT_ALL,
    SELECT_EXTEND_DOWN,
    SELECT_EXTEND_END,
    SELECT_EXTEND_HOME,
    SELECT_EXTEND_LEFT,
    SELECT_EXTEND_PAGE_DOWN,
    SELECT_EXTEND_PAGE_UP,
    SELECT_EXTEND_RIGHT,
    SELECT_EXTEND_TO_DOC_END,
    SELECT_EXTEND_TO_DOC_START,
    SELECT_EXTEND_UP,
    SELECT_EXTEND_WORD_LEFT,
    SELECT_EXTEND_WORD_RIGHT,
    UI_CANCEL,
    UI_CONFIRM,
    UI_DIALOG_CHOOSE,
    UI_LIST_NEXT,
    UI_LIST_PAGE_DOWN,
    UI_LIST_PAGE_UP,
    UI_LIST_PREV,
    UI_MESSAGE_HISTORY,
    UI_MESSAGE_HISTORY_COPY,
    UI_PALETTE_OPEN,
    UI_REDRAW,
    UI_TOGGLE_HINT_BAR,
    UI_TOGGLE_MOUSE,
    VIEW_TOGGLE_WRAP,
];

#[cfg(test)]
mod tests {
    #[test]
    fn 定数の一覧は組込みの登録と一致する() {
        let reg = crate::editor::builtin_registry();
        let registered: Vec<&str> = reg.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(registered, super::ALL);
    }
}
