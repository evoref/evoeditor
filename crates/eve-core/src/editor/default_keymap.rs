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
    bind("global", "mod+shift+p", "ui.palette_open"); // [E][横]
    bind("global", "ctrl+k r", "ui.redraw");
    // 検索バー (search → prompt → global)
    for (keys, cmd) in [
        ("enter", "search.next"),
        ("down", "search.next"),
        ("f3", "search.next"),
        ("up", "search.prev"),
        ("shift+f3", "search.prev"),
        ("shift+enter", "search.prev"), // [E]
        ("alt+c", "search.toggle_case"),
        ("ctrl+k c", "search.toggle_case"),
        ("alt+a", "search.replace_all"),
        ("tab", "ui.list_next"),
        ("shift+tab", "ui.list_prev"),
        ("mod+r", "search.replace_open"),
        ("ctrl+h", "search.replace_open"), // [E]
        ("mod+f", "search.open"),
        ("mod+s", "file.save"),
        ("mod+shift+s", "file.save_as"),
        ("mod+o", "file.open"),
        ("mod+n", "file.new"),
    ] {
        bind("search", keys, cmd);
    }
    bind("editor", "mod+f", "search.open");
    // 読込中の取消し (読込中でなければ何もしない、FR-122)
    bind("editor", "esc", "ui.cancel");
    bind("editor", "mod+r", "search.replace_open");
    bind("editor", "ctrl+h", "search.replace_open"); // [E]
    // コマンドパレット (palette → list → prompt → global)
    bind("palette", "down", "ui.list_next");
    bind("palette", "up", "ui.list_prev");
    bind("palette", "ctrl+n", "ui.list_next");
    bind("palette", "ctrl+p", "ui.list_prev");
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

    /// 既定キー列 (07 §11.6 の表記) を、割当の組 (`a` / `b` の並び。`—` は `None`) と、直後の括弧書きの注記に分ける。
    /// 属性 `[E]` `[横]` `(alt)` は読み飛ばし、`ctx`: は文脈の見出しとして読み飛ばす。それ以外の `(…)` は注記。
    fn groups(cell: &str) -> Vec<(Vec<Option<&str>>, String)> {
        let mut out: Vec<(Vec<Option<&str>>, String)> = Vec::new();
        let mut cur: Vec<Option<&str>> = Vec::new();
        let mut rest = cell.trim();
        while !rest.is_empty() {
            if let Some(r) = rest.strip_prefix('`') {
                let end = r.find('`').expect("バッククォートが閉じている");
                let (k, after) = (&r[..end], &r[end + 1..]);
                if let Some(a) = after.strip_prefix(':') {
                    rest = a.trim_start(); // `ctx`: の見出し
                    continue;
                }
                cur.push(Some(k));
                rest = after;
            } else if let Some(r) = rest.strip_prefix('—') {
                cur.push(None);
                rest = r;
            } else {
                rest = rest.trim_start_matches([',', '、', ' ']);
                if rest.starts_with('(') || rest.starts_with('*') {
                    // 組の外の括弧書き (直前の組の注記)
                    let (note, after) = take_paren(rest);
                    if let Some(last) = out.last_mut() {
                        last.1.push_str(&note);
                    }
                    rest = after;
                } else if !rest.is_empty() && !rest.starts_with('`') && !rest.starts_with('—') {
                    rest = &rest[rest.chars().next().map_or(0, char::len_utf8)..];
                }
                continue;
            }
            // 属性: `[E]` `[横]` `(alt)`
            loop {
                let t = rest.trim_start();
                if t.starts_with('[') {
                    rest = &t[t.find(']').expect("] が閉じている") + 1..];
                } else if let Some(r) = t.strip_prefix("(alt)") {
                    rest = r;
                } else {
                    break;
                }
            }
            if let Some(r) = rest.trim_start().strip_prefix("/ ") {
                rest = r; // 同じ組の続き
            } else {
                out.push((std::mem::take(&mut cur), String::new()));
            }
        }
        out
    }

    /// `(` から対応する `)` まで (入れ子を数える)。中身と残りを返す。
    fn take_paren(s: &str) -> (String, &str) {
        if !s.starts_with('(') {
            return (
                String::new(),
                &s[s.chars().next().map_or(0, char::len_utf8)..],
            );
        }
        let mut depth = 0;
        for (i, c) in s.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return (s[1..i].to_owned(), &s[i + 1..]);
                    }
                }
                _ => {}
            }
        }
        (s[1..].to_owned(), "")
    }

    // verifies: AC-FR-603-1
    #[test]
    fn 既定キーマップは_07_のカタログの既定キー列と一致する() {
        use super::super::keymap::{format_sequence, parse_sequence};
        use eve_docs_tables::{identifiers, read_tables, strip_parentheticals};
        use std::collections::BTreeSet;

        let doc = include_str!("../../../../docs/07-config-extensibility.md");
        let tables = read_tables(
            doc,
            "### 11.6 v0.1 CommandId カタログ (付録)",
            &["ID", "既定キー"],
        )
        .expect("カタログを読める");
        // 書式: 割当を `, ` で並べる。対のコマンドは `a` / `b` の形で ID と同じ個数を並べ、無い側は `—`。
        // 割当には `[E]` `[横]` `(alt)` の注記が付く。`ctx`: は割当の文脈の見出し (比較には使わない)
        let mut want: BTreeSet<(String, String)> = BTreeSet::new();
        for t in &tables {
            for row in &t.rows {
                let id_cell = t.cell(row, "ID").unwrap_or_default();
                if id_cell.contains("(v") {
                    continue;
                }
                let ids = identifiers(&strip_parentheticals(id_cell));
                let keys = t.cell(row, "既定キー").unwrap_or_default();
                for (elems, note) in groups(keys) {
                    // 1 つだけの割当が複数 ID の行にあるときは、注記の「<ID の最後の語> のみ」で対象を決める
                    let only: Option<&String> = if elems.len() == 1 && ids.len() > 1 {
                        let word = note.split(" のみ").next().unwrap_or("").trim();
                        Some(
                            ids.iter()
                                .find(|id| id.ends_with(word) && !word.is_empty())
                                .unwrap_or_else(|| {
                                    panic!("行 {}: 割当 1 個の対象が決まらない: {note}", row.line)
                                }),
                        )
                    } else {
                        None
                    };
                    assert!(
                        only.is_some() || elems.len() == ids.len() || ids.len() == 1,
                        "行 {}: ID {} 個に割当 {} 個: {elems:?}",
                        row.line,
                        ids.len(),
                        elems.len()
                    );
                    for (i, e) in elems.iter().enumerate() {
                        let Some(k) = e else {
                            continue; // `—` (無い側)
                        };
                        let id = only.unwrap_or(&ids[if ids.len() == 1 { 0 } else { i }]);
                        let seq = parse_sequence(k).unwrap_or_else(|e| panic!("{k}: {e:?}"));
                        want.insert((id.clone(), format_sequence(&seq)));
                    }
                }
            }
        }
        let km = default_keymap();
        let mut have: BTreeSet<(String, String)> = BTreeSet::new();
        for l in km.layers() {
            for b in l.bindings() {
                have.insert((b.command.as_str().to_owned(), format_sequence(&b.sequence)));
            }
        }
        let only_doc: Vec<_> = want.difference(&have).collect();
        let only_impl: Vec<_> = have.difference(&want).collect();
        assert!(
            only_doc.is_empty() && only_impl.is_empty(),
            "カタログだけ: {only_doc:?}\n実装だけ: {only_impl:?}"
        );
    }
}
