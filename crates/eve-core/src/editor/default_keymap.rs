//! 組込みの既定キーマップ (docs/07 §11.6 の「既定キー」列が正本、04 §7.2 が設計入力)。
//! 登録済みのコマンドの割当だけを置く。コマンドを足す PR が、そのコマンドの割当を足す。
//! `mod` は TUI では `ctrl` に解決する。`(alt)` の割当 (チョード・代替キー) はそのコマンドの PR で足す。

use super::keymap::{Keymap, Layer};

/// 既定のキーマップを作る。
pub(super) fn default_keymap() -> Keymap {
    let mut l = Layer::new("default");
    let mut bind = |ctx, keys, cmd| {
        l.bind(ctx, keys, cmd, "")
            .expect("既定キーマップの書式は正しい");
    };
    // 編集
    bind("editor", "enter", "edit.newline");
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
