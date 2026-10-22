//! 組込みの既定キーマップ (docs/07 §11.6 の「既定キー」列が正本、04 §7.2 が設計入力)。
//! 登録済みのコマンドの割当だけを置く。コマンドを足す PR が、そのコマンドの割当を足す。
//! `mod` は TUI では `ctrl` に解決する。`(alt)` の割当 (チョード・代替キー) はそのコマンドの PR で足す。

use super::keymap::{Keymap, Layer};

/// 既定のキーマップを作る。
pub(super) fn default_keymap() -> Keymap {
    let mut l = Layer::new("default");
    let mut bind = |ctx: &'static str, keys: &str, cmd: &str| {
        l.bind(ctx, keys, cmd, "")
            .expect("既定キーマップの書式は正しい");
    };
    // どこでも
    bind("global", "mod+q", "app.quit");
    bind("global", "mod+p", "ui.palette_open");
    bind("global", "f1", "ui.palette_open");
    // 検索バー (search → prompt → global)
    for (keys, cmd) in [
        ("enter", "search.next"),
        ("down", "search.next"),
        ("f3", "search.next"),
        ("up", "search.prev"),
        ("shift+f3", "search.prev"),
        ("alt+c", "search.toggle_case"),
        ("alt+a", "search.replace_all"),
        ("tab", "ui.list_next"),
        ("shift+tab", "ui.list_prev"),
        ("mod+r", "search.replace_open"),
        ("mod+f", "search.open"),
        ("mod+s", "file.save"),
        ("mod+shift+s", "file.save_as"),
        ("mod+o", "file.open"),
        ("mod+n", "file.new"),
    ] {
        bind("search", keys, cmd);
    }
    bind("editor", "mod+f", "search.open");
    bind("editor", "mod+r", "search.replace_open");
    // コマンドパレット (palette → list → prompt → global)
    bind("palette", "down", "ui.list_next");
    bind("palette", "up", "ui.list_prev");
    bind("palette", "pagedown", "ui.list_page_down");
    bind("palette", "pageup", "ui.list_page_up");
    // テスト専用: パニックの注入 (本来キー割当なし。E2E が端末から送れるよう、フックがあるときだけ割り当てる)
    #[cfg(feature = "test-hooks")]
    if super::overlay::test_hook_panic_command() {
        bind("global", "f12", "internal.debug_panic");
    }
    // 確認ダイアログ (モーダル。dialog のみを参照する)
    bind("dialog", "enter", "ui.confirm");
    bind("dialog", "esc", "ui.cancel");
    for (keys, cmd) in [
        ("tab", "ui.list_next"),
        ("right", "ui.list_next"),
        ("down", "ui.list_next"),
        ("shift+tab", "ui.list_prev"),
        ("left", "ui.list_prev"),
        ("up", "ui.list_prev"),
    ] {
        bind("dialog", keys, cmd);
    }
    // 入力プロンプト (prompt → global)
    bind("prompt", "mod+v", "edit.paste");
    bind("prompt", "shift+insert", "edit.paste");
    bind("prompt", "enter", "ui.confirm");
    bind("prompt", "esc", "ui.cancel");
    for (keys, cmd) in [
        ("backspace", "edit.delete_backward"),
        ("delete", "edit.delete_forward"),
        ("left", "cursor.move_left"),
        ("right", "cursor.move_right"),
        ("home", "cursor.home"),
        ("end", "cursor.end"),
    ] {
        bind("prompt", keys, cmd);
    }
    // ファイル
    bind("editor", "mod+s", "file.save");
    bind("editor", "mod+shift+s", "file.save_as");
    bind("editor", "ctrl+k s", "file.save_as");
    bind("editor", "mod+o", "file.open");
    bind("editor", "mod+n", "file.new");
    // 編集
    bind("editor", "enter", "edit.newline");
    // クリップボード
    bind("editor", "mod+c", "clipboard.copy");
    bind("editor", "mod+x", "clipboard.cut");
    bind("editor", "mod+v", "edit.paste");
    bind("editor", "ctrl+insert", "clipboard.copy");
    bind("editor", "shift+delete", "clipboard.cut");
    bind("editor", "shift+insert", "edit.paste");
    bind("editor", "mod+z", "edit.undo");
    bind("editor", "mod+y", "edit.redo");
    bind("editor", "mod+shift+z", "edit.redo");
    bind("editor", "backspace", "edit.delete_backward");
    bind("editor", "delete", "edit.delete_forward");
    bind("editor", "ctrl+backspace", "edit.delete_word_backward");
    bind("editor", "ctrl+delete", "edit.delete_word_forward");
    bind("editor", "alt+backspace", "edit.delete_word_backward");
    bind("editor", "tab", "edit.indent");
    bind("editor", "shift+tab", "edit.outdent");
    bind("editor", "mod+a", "select.all");
    // 移動・選択 (shift 付きは select.extend_*)
    for (key, cmd, ext) in [
        ("left", "cursor.move_left", "select.extend_left"),
        ("right", "cursor.move_right", "select.extend_right"),
        ("up", "cursor.move_up", "select.extend_up"),
        ("down", "cursor.move_down", "select.extend_down"),
        (
            "ctrl+left",
            "cursor.move_word_left",
            "select.extend_word_left",
        ),
        (
            "ctrl+right",
            "cursor.move_word_right",
            "select.extend_word_right",
        ),
        (
            "alt+left",
            "cursor.move_word_left",
            "select.extend_word_left",
        ),
        (
            "alt+right",
            "cursor.move_word_right",
            "select.extend_word_right",
        ),
        ("home", "cursor.home", "select.extend_home"),
        ("end", "cursor.end", "select.extend_end"),
        (
            "ctrl+home",
            "cursor.doc_start",
            "select.extend_to_doc_start",
        ),
        ("ctrl+end", "cursor.doc_end", "select.extend_to_doc_end"),
        ("pageup", "cursor.page_up", "select.extend_page_up"),
        ("pagedown", "cursor.page_down", "select.extend_page_down"),
    ] {
        bind("editor", key, cmd);
        bind("editor", &format!("shift+{key}"), ext);
    }
    bind("editor", "alt+b", "cursor.move_word_left");
    bind("editor", "alt+f", "cursor.move_word_right");
    bind("editor", "mod+g", "cursor.goto_line");
    // チョードの代替キー ((alt))
    for (keys, cmd) in [
        ("ctrl+k left", "cursor.home"),
        ("ctrl+k right", "cursor.end"),
        ("ctrl+k shift+left", "select.extend_home"),
        ("ctrl+k shift+right", "select.extend_end"),
        ("ctrl+k ,", "cursor.page_up"),
        ("ctrl+k .", "cursor.page_down"),
        ("ctrl+k <", "select.extend_page_up"),
        ("ctrl+k >", "select.extend_page_down"),
    ] {
        bind("editor", keys, cmd);
    }
    Keymap::new(vec![l]).expect("既定キーマップに曖昧なチョードはない")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 既定キーマップを作れる() {
        let _ = default_keymap();
    }
}
