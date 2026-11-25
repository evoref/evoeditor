# 既定キーマップ

生成物 (`cargo xtask gen-reference` で作り直す。手で直さない。docs/09 §11.2)。`evoeditor --dump-keymap --json` (`--no-config`。組込みの既定キーマップ、割当 201 件) の出力から作る。既定キーの正本は [07 §11.6 のカタログ](../07-config-extensibility.md#116-v01-commandid-カタログ-付録) の「既定キー」列で、`cargo xtask gen-reference --check` が突合する。

- コンテキストは 07 §7.1 の順。同じコンテキストの中は優先の順 (層の順・登録の順)。キーを探すときの参照の順 (`search` → `prompt` → `global` など) は 07 §7.1。
- **キー** は画面の表記 (04 §7.8)、**設定の表記** は `keymap.toml` に書く形 (07 §7.2)。**拡張キー** の「要」は拡張キーが無い端末では届かない割当 (07 §11.6 の `[E]`)。

## `global`

| キー | 設定の表記 | コマンド | タイトル | when | 拡張キー |
|---|---|---|---|---|---|
| `Ctrl+Q` | `ctrl+q` | `app.quit` | 終了 | — | — |
| `Ctrl+P` | `ctrl+p` | `ui.palette_open` | コマンドパレット | — | — |
| `F1` | `f1` | `ui.palette_open` | コマンドパレット | — | — |
| `Ctrl+Shift+P` | `ctrl+shift+p` | `ui.palette_open` | コマンドパレット | — | 要 |
| `Ctrl+K R` | `ctrl+k r` | `ui.redraw` | 再描画 | — | — |
| `Ctrl+K H` | `ctrl+k h` | `help.open` | ヘルプを開く | — | — |

## `editor`

| キー | 設定の表記 | コマンド | タイトル | when | 拡張キー |
|---|---|---|---|---|---|
| `Ctrl+K M` | `ctrl+k m` | `ui.message_history` | メッセージ履歴 | — | — |
| `Ctrl+K Z` | `ctrl+k z` | `app.suspend` | 一時停止 (サスペンド) | — | — |
| `Alt+Z` | `alt+z` | `view.toggle_wrap` | 折り返しの切り替え | — | — |
| `Ctrl+K O` | `ctrl+k o` | `view.toggle_wrap` | 折り返しの切り替え | — | — |
| `Ctrl+F` | `ctrl+f` | `search.open` | 検索 | — | — |
| `Esc` | `esc` | `ui.cancel` | 取り消し | — | — |
| `Ctrl+R` | `ctrl+r` | `search.replace_open` | 置換 | — | — |
| `Ctrl+H` | `ctrl+h` | `search.replace_open` | 置換 | — | 要 |
| `Ctrl+S` | `ctrl+s` | `file.save` | 保存 | — | — |
| `Ctrl+Shift+S` | `ctrl+shift+s` | `file.save_as` | 名前を付けて保存 | — | 要 |
| `Ctrl+K S` | `ctrl+k s` | `file.save_as` | 名前を付けて保存 | — | — |
| `Ctrl+K A` | `ctrl+k a` | `file.save_all` | すべて保存 | — | — |
| `Ctrl+O` | `ctrl+o` | `workspace.open_file` | ファイルを探して開く | — | — |
| `Ctrl+Shift+F` | `ctrl+shift+f` | `search.workspace` | ワークスペースを検索 | — | 要 |
| `Ctrl+K F` | `ctrl+k f` | `search.workspace` | ワークスペースを検索 | — | — |
| `Ctrl+N` | `ctrl+n` | `file.new` | 新規作成 | — | — |
| `Ctrl+W` | `ctrl+w` | `buffer.close` | 文書を閉じる | — | — |
| `Ctrl+K W` | `ctrl+k w` | `buffer.close` | 文書を閉じる | — | — |
| `Ctrl+PgDn` | `ctrl+pagedown` | `buffer.next` | 次の文書 | — | — |
| `Ctrl+PgUp` | `ctrl+pageup` | `buffer.prev` | 前の文書 | — | — |
| `Ctrl+K N` | `ctrl+k n` | `buffer.next` | 次の文書 | — | — |
| `Ctrl+E` | `ctrl+e` | `buffer.list` | 一覧 | — | — |
| `Ctrl+K P` | `ctrl+k p` | `buffer.prev` | 前の文書 | — | — |
| `Enter` | `enter` | `edit.newline` | 改行 | — | — |
| `Ctrl+C` | `ctrl+c` | `clipboard.copy` | コピー | — | — |
| `Ctrl+X` | `ctrl+x` | `clipboard.cut` | 切り取り | — | — |
| `Ctrl+V` | `ctrl+v` | `edit.paste` | 貼り付け | — | — |
| `Ctrl+Insert` | `ctrl+insert` | `clipboard.copy` | コピー | — | — |
| `Shift+Delete` | `shift+delete` | `clipboard.cut` | 切り取り | — | — |
| `Shift+Insert` | `shift+insert` | `edit.paste` | 貼り付け | — | — |
| `Ctrl+Z` | `ctrl+z` | `edit.undo` | 元に戻す | — | — |
| `Ctrl+Y` | `ctrl+y` | `edit.redo` | やり直し | — | — |
| `Ctrl+Shift+Z` | `ctrl+shift+z` | `edit.redo` | やり直し | — | 要 |
| `Backspace` | `backspace` | `edit.delete_backward` | 前の文字を削除 | — | — |
| `Delete` | `delete` | `edit.delete_forward` | 後ろの文字を削除 | — | — |
| `Ctrl+Backspace` | `ctrl+backspace` | `edit.delete_word_backward` | 前の単語を削除 | — | — |
| `Ctrl+Delete` | `ctrl+delete` | `edit.delete_word_forward` | 後ろの単語を削除 | — | — |
| `Alt+Backspace` | `alt+backspace` | `edit.delete_word_backward` | 前の単語を削除 | — | — |
| `Tab` | `tab` | `edit.indent` | インデント | — | — |
| `Shift+Tab` | `shift+tab` | `edit.outdent` | インデント解除 | — | — |
| `Ctrl+A` | `ctrl+a` | `select.all` | すべて選択 | — | — |
| `←` | `left` | `cursor.move_left` | 左へ移動 | — | — |
| `Shift+←` | `shift+left` | `select.extend_left` | 左へ選択 | — | — |
| `→` | `right` | `cursor.move_right` | 右へ移動 | — | — |
| `Shift+→` | `shift+right` | `select.extend_right` | 右へ選択 | — | — |
| `↑` | `up` | `cursor.move_up` | 上へ移動 | — | — |
| `Shift+↑` | `shift+up` | `select.extend_up` | 上へ選択 | — | — |
| `↓` | `down` | `cursor.move_down` | 下へ移動 | — | — |
| `Shift+↓` | `shift+down` | `select.extend_down` | 下へ選択 | — | — |
| `Ctrl+←` | `ctrl+left` | `cursor.move_word_left` | 単語の左へ移動 | — | — |
| `Ctrl+Shift+←` | `ctrl+shift+left` | `select.extend_word_left` | 単語の左へ選択 | — | — |
| `Ctrl+→` | `ctrl+right` | `cursor.move_word_right` | 単語の右へ移動 | — | — |
| `Ctrl+Shift+→` | `ctrl+shift+right` | `select.extend_word_right` | 単語の右へ選択 | — | — |
| `Alt+←` | `alt+left` | `cursor.move_word_left` | 単語の左へ移動 | — | — |
| `Alt+Shift+←` | `alt+shift+left` | `select.extend_word_left` | 単語の左へ選択 | — | — |
| `Alt+→` | `alt+right` | `cursor.move_word_right` | 単語の右へ移動 | — | — |
| `Alt+Shift+→` | `alt+shift+right` | `select.extend_word_right` | 単語の右へ選択 | — | — |
| `Home` | `home` | `cursor.home` | 行頭へ移動 | — | — |
| `Shift+Home` | `shift+home` | `select.extend_home` | 行頭まで選択 | — | — |
| `End` | `end` | `cursor.end` | 行末へ移動 | — | — |
| `Shift+End` | `shift+end` | `select.extend_end` | 行末まで選択 | — | — |
| `Ctrl+Home` | `ctrl+home` | `cursor.doc_start` | 文書の先頭へ移動 | — | — |
| `Ctrl+Shift+Home` | `ctrl+shift+home` | `select.extend_to_doc_start` | 文書の先頭まで選択 | — | 要 |
| `Ctrl+End` | `ctrl+end` | `cursor.doc_end` | 文書の末尾へ移動 | — | — |
| `Ctrl+Shift+End` | `ctrl+shift+end` | `select.extend_to_doc_end` | 文書の末尾まで選択 | — | 要 |
| `PgUp` | `pageup` | `cursor.page_up` | 前のページへ移動 | — | — |
| `Shift+PgUp` | `shift+pageup` | `select.extend_page_up` | 前のページまで選択 | — | — |
| `PgDn` | `pagedown` | `cursor.page_down` | 次のページへ移動 | — | — |
| `Shift+PgDn` | `shift+pagedown` | `select.extend_page_down` | 次のページまで選択 | — | — |
| `Ctrl+K D` | `ctrl+k d` | `edit.duplicate_line` | 行を複製 | — | — |
| `Ctrl+K L` | `ctrl+k l` | `edit.delete_line` | 行を削除 | — | — |
| `Ctrl+Shift+K` | `ctrl+shift+k` | `edit.delete_line` | 行を削除 | — | 要 |
| `Alt+↑` | `alt+up` | `edit.move_line_up` | 行を上へ移動 | — | — |
| `Alt+↓` | `alt+down` | `edit.move_line_down` | 行を下へ移動 | — | — |
| `Ctrl+K ↑` | `ctrl+k up` | `edit.move_line_up` | 行を上へ移動 | — | — |
| `Ctrl+K ↓` | `ctrl+k down` | `edit.move_line_down` | 行を下へ移動 | — | — |
| `Alt+B` | `alt+b` | `cursor.move_word_left` | 単語の左へ移動 | — | — |
| `Alt+F` | `alt+f` | `cursor.move_word_right` | 単語の右へ移動 | — | — |
| `Ctrl+G` | `ctrl+g` | `cursor.goto_line` | 行へ移動 | — | — |
| `Ctrl+\` | `ctrl+\` | `pane.split_right` | 右に分割 (左右に並べる) | — | — |
| `Ctrl+K \` | `ctrl+k \` | `pane.split_right` | 右に分割 (左右に並べる) | — | — |
| `Ctrl+K \|` | `ctrl+k \|` | `pane.split_right` | 右に分割 (左右に並べる) | — | — |
| `Ctrl+K -` | `ctrl+k -` | `pane.split_down` | 下に分割 (上下に並べる) | — | — |
| `F6` | `f6` | `pane.focus_next` | 次のペイン | — | — |
| `Shift+F6` | `shift+f6` | `pane.focus_prev` | 前のペイン | — | — |
| `Ctrl+K Tab` | `ctrl+k tab` | `pane.focus_next` | 次のペイン | — | — |
| `Ctrl+K Shift+Tab` | `ctrl+k shift+tab` | `pane.focus_prev` | 前のペイン | — | — |
| `Ctrl+K Q` | `ctrl+k q` | `pane.close` | ペインを閉じる | — | — |
| `Ctrl+K ]` | `ctrl+k ]` | `pane.grow` | ペインを広げる | — | — |
| `Ctrl+K [` | `ctrl+k [` | `pane.shrink` | ペインを狭める | — | — |
| `Ctrl+K =` | `ctrl+k =` | `pane.equalize` | ペインの大きさを揃える | — | — |
| `Ctrl+D` | `ctrl+d` | `select.add_next_match` | 次の一致を選択に追加 | — | — |
| `Ctrl+Shift+L` | `ctrl+shift+l` | `select.all_matches` | 一致をすべて選択 | — | 要 |
| `Ctrl+K E` | `ctrl+k e` | `select.all_matches` | 一致をすべて選択 | — | — |
| `Ctrl+Alt+↑` | `ctrl+alt+up` | `select.add_cursor_above` | 上の行にカーソルを追加 | — | — |
| `Ctrl+Alt+↓` | `ctrl+alt+down` | `select.add_cursor_below` | 下の行にカーソルを追加 | — | — |
| `Ctrl+K Shift+↑` | `ctrl+k shift+up` | `select.add_cursor_above` | 上の行にカーソルを追加 | — | — |
| `Ctrl+K Shift+↓` | `ctrl+k shift+down` | `select.add_cursor_below` | 下の行にカーソルを追加 | — | — |
| `Ctrl+K V` | `ctrl+k v` | `edit.paste` | 貼り付け | — | — |
| `Ctrl+]` | `ctrl+]` | `cursor.goto_bracket` | 対応する括弧へ移動 | — | — |
| `Ctrl+K %` | `ctrl+k %` | `cursor.goto_bracket` | 対応する括弧へ移動 | — | — |
| `Ctrl+/` | `ctrl+/` | `edit.toggle_comment` | 行コメントの切替 | — | — |
| `Ctrl+K /` | `ctrl+k /` | `edit.toggle_comment` | 行コメントの切替 | — | — |
| `Ctrl+L` | `ctrl+l` | `select.line` | 行を選択 | — | — |
| `Alt+U` | `alt+u` | `select.expand` | 選択を広げる | — | — |
| `Alt+I` | `alt+i` | `select.shrink` | 選択を縮める | — | — |
| `Ctrl+K U` | `ctrl+k u` | `select.expand` | 選択を広げる | — | — |
| `Ctrl+K I` | `ctrl+k i` | `select.shrink` | 選択を縮める | — | — |
| `Alt+J` | `alt+j` | `cursor.jump_back` | 前の位置へ戻る | — | — |
| `Alt+K` | `alt+k` | `cursor.jump_forward` | 次の位置へ進む | — | — |
| `Ctrl+K J` | `ctrl+k j` | `cursor.jump_back` | 前の位置へ戻る | — | — |
| `Ctrl+K K` | `ctrl+k k` | `cursor.jump_forward` | 次の位置へ進む | — | — |
| `Ctrl+K B` | `ctrl+k b` | `select.toggle_box` | 矩形選択のモードを切り替え | — | — |
| `Ctrl+K ←` | `ctrl+k left` | `cursor.home` | 行頭へ移動 | — | — |
| `Ctrl+K →` | `ctrl+k right` | `cursor.end` | 行末へ移動 | — | — |
| `Ctrl+K Shift+←` | `ctrl+k shift+left` | `select.extend_home` | 行頭まで選択 | — | — |
| `Ctrl+K Shift+→` | `ctrl+k shift+right` | `select.extend_end` | 行末まで選択 | — | — |
| `Ctrl+K ,` | `ctrl+k ,` | `cursor.page_up` | 前のページへ移動 | — | — |
| `Ctrl+K .` | `ctrl+k .` | `cursor.page_down` | 次のページへ移動 | — | — |
| `Ctrl+K <` | `ctrl+k <` | `select.extend_page_up` | 前のページまで選択 | — | — |
| `Ctrl+K >` | `ctrl+k >` | `select.extend_page_down` | 次のページまで選択 | — | — |

## `search`

| キー | 設定の表記 | コマンド | タイトル | when | 拡張キー |
|---|---|---|---|---|---|
| `Enter` | `enter` | `search.replace` | 置換 | `search.replace_focused` | — |
| `Enter` | `enter` | `search.replace_all` | すべて置換 | `search.replace_all_focused` | — |
| `Enter` | `enter` | `search.next` | 次を検索 | — | — |
| `↓` | `down` | `search.next` | 次を検索 | — | — |
| `F3` | `f3` | `search.next` | 次を検索 | — | — |
| `↑` | `up` | `search.prev` | 前を検索 | — | — |
| `Shift+F3` | `shift+f3` | `search.prev` | 前を検索 | — | 要 |
| `Shift+Enter` | `shift+enter` | `search.prev` | 前を検索 | — | 要 |
| `Alt+C` | `alt+c` | `search.toggle_case` | 大小文字の区別を切替 | — | — |
| `Ctrl+K C` | `ctrl+k c` | `search.toggle_case` | 大小文字の区別を切替 | — | — |
| `Alt+R` | `alt+r` | `search.toggle_regex` | 正規表現を切替 | — | — |
| `Ctrl+K X` | `ctrl+k x` | `search.toggle_regex` | 正規表現を切替 | — | — |
| `Alt+W` | `alt+w` | `search.toggle_word` | 単語単位を切替 | — | — |
| `Ctrl+K B` | `ctrl+k b` | `search.toggle_word` | 単語単位を切替 | — | — |
| `Alt+L` | `alt+l` | `search.toggle_selection` | 選択範囲内を切替 | — | — |
| `Ctrl+K L` | `ctrl+k l` | `search.toggle_selection` | 選択範囲内を切替 | — | — |
| `Alt+A` | `alt+a` | `search.replace_all` | すべて置換 | — | — |
| `Tab` | `tab` | `ui.list_next` | 次の項目 | — | — |
| `Shift+Tab` | `shift+tab` | `ui.list_prev` | 前の項目 | — | — |
| `Ctrl+R` | `ctrl+r` | `search.replace_open` | 置換 | — | — |
| `Ctrl+H` | `ctrl+h` | `search.replace_open` | 置換 | — | 要 |
| `Ctrl+F` | `ctrl+f` | `search.open` | 検索 | — | — |
| `Ctrl+S` | `ctrl+s` | `file.save` | 保存 | — | — |
| `Ctrl+Shift+S` | `ctrl+shift+s` | `file.save_as` | 名前を付けて保存 | — | 要 |
| `Ctrl+O` | `ctrl+o` | `workspace.open_file` | ファイルを探して開く | — | — |
| `Ctrl+N` | `ctrl+n` | `file.new` | 新規作成 | — | — |
| `Ctrl+Shift+F` | `ctrl+shift+f` | `search.workspace` | ワークスペースを検索 | — | 要 |
| `Ctrl+K F` | `ctrl+k f` | `search.workspace` | ワークスペースを検索 | — | — |

## `prompt`

| キー | 設定の表記 | コマンド | タイトル | when | 拡張キー |
|---|---|---|---|---|---|
| `Ctrl+V` | `ctrl+v` | `edit.paste` | 貼り付け | — | — |
| `Shift+Insert` | `shift+insert` | `edit.paste` | 貼り付け | — | — |
| `Enter` | `enter` | `ui.confirm` | 決定 | — | — |
| `Esc` | `esc` | `ui.cancel` | 取り消し | — | — |
| `Backspace` | `backspace` | `edit.delete_backward` | 前の文字を削除 | — | — |
| `Delete` | `delete` | `edit.delete_forward` | 後ろの文字を削除 | — | — |
| `←` | `left` | `cursor.move_left` | 左へ移動 | — | — |
| `→` | `right` | `cursor.move_right` | 右へ移動 | — | — |
| `Home` | `home` | `cursor.home` | 行頭へ移動 | — | — |
| `End` | `end` | `cursor.end` | 行末へ移動 | — | — |
| `Ctrl+Backspace` | `ctrl+backspace` | `edit.delete_word_backward` | 前の単語を削除 | — | — |
| `Ctrl+Delete` | `ctrl+delete` | `edit.delete_word_forward` | 後ろの単語を削除 | — | — |
| `Ctrl+←` | `ctrl+left` | `cursor.move_word_left` | 単語の左へ移動 | — | — |
| `Ctrl+→` | `ctrl+right` | `cursor.move_word_right` | 単語の右へ移動 | — | — |
| `Ctrl+A` | `ctrl+a` | `select.all` | すべて選択 | — | — |
| `Alt+↑` | `alt+up` | `ui.history_prev` | 前の履歴 | `(overlay.active == "" && search.active) \|\| overlay.active == "workspace_search"` | — |
| `Alt+↓` | `alt+down` | `ui.history_next` | 次の履歴 | `(overlay.active == "" && search.active) \|\| overlay.active == "workspace_search"` | — |
| `Ctrl+K ↑` | `ctrl+k up` | `ui.history_prev` | 前の履歴 | `(overlay.active == "" && search.active) \|\| overlay.active == "workspace_search"` | — |
| `Ctrl+K ↓` | `ctrl+k down` | `ui.history_next` | 次の履歴 | `(overlay.active == "" && search.active) \|\| overlay.active == "workspace_search"` | — |

## `list`

| キー | 設定の表記 | コマンド | タイトル | when | 拡張キー |
|---|---|---|---|---|---|
| `↓` | `down` | `ui.list_next` | 次の項目 | — | — |
| `↑` | `up` | `ui.list_prev` | 前の項目 | — | — |
| `Ctrl+N` | `ctrl+n` | `ui.list_next` | 次の項目 | — | — |
| `Ctrl+P` | `ctrl+p` | `ui.list_prev` | 前の項目 | — | — |
| `PgDn` | `pagedown` | `ui.list_page_down` | 一覧の次のページ | — | — |
| `PgUp` | `pageup` | `ui.list_page_up` | 一覧の前のページ | — | — |
| `Ctrl+W` | `ctrl+w` | `buffer.close` | 文書を閉じる | — | — |

## `dialog`

| キー | 設定の表記 | コマンド | タイトル | when | 拡張キー |
|---|---|---|---|---|---|
| `Enter` | `enter` | `ui.confirm` | 決定 | — | — |
| `Esc` | `esc` | `ui.cancel` | 取り消し | — | — |
| `Tab` | `tab` | `ui.list_next` | 次の項目 | — | — |
| `→` | `right` | `ui.list_next` | 次の項目 | — | — |
| `↓` | `down` | `ui.list_next` | 次の項目 | — | — |
| `Shift+Tab` | `shift+tab` | `ui.list_prev` | 前の項目 | — | — |
| `←` | `left` | `ui.list_prev` | 前の項目 | — | — |
| `↑` | `up` | `ui.list_prev` | 前の項目 | — | — |

## `help`

| キー | 設定の表記 | コマンド | タイトル | when | 拡張キー |
|---|---|---|---|---|---|
| `↓` | `down` | `ui.list_next` | 次の項目 | — | — |
| `↑` | `up` | `ui.list_prev` | 前の項目 | — | — |
| `PgDn` | `pagedown` | `ui.list_page_down` | 一覧の次のページ | — | — |
| `PgUp` | `pageup` | `ui.list_page_up` | 一覧の前のページ | — | — |
| `Esc` | `esc` | `ui.cancel` | 取り消し | — | — |
| `Ctrl+C` | `ctrl+c` | `ui.message_history_copy` | メッセージ履歴をコピー | — | — |

## `finder`

| キー | 設定の表記 | コマンド | タイトル | when | 拡張キー |
|---|---|---|---|---|---|
| `Alt+C` | `alt+c` | `search.toggle_case` | 大小文字の区別を切替 | `overlay.active == "workspace_search"` | — |
| `Ctrl+K C` | `ctrl+k c` | `search.toggle_case` | 大小文字の区別を切替 | `overlay.active == "workspace_search"` | — |
| `Alt+R` | `alt+r` | `search.toggle_regex` | 正規表現を切替 | `overlay.active == "workspace_search"` | — |
| `Ctrl+K X` | `ctrl+k x` | `search.toggle_regex` | 正規表現を切替 | `overlay.active == "workspace_search"` | — |
| `Alt+W` | `alt+w` | `search.toggle_word` | 単語単位を切替 | `overlay.active == "workspace_search"` | — |
| `Ctrl+K B` | `ctrl+k b` | `search.toggle_word` | 単語単位を切替 | `overlay.active == "workspace_search"` | — |
