# コマンド一覧

生成物 (`cargo xtask gen-reference` で作り直す。手で直さない。docs/09 §11.2)。`evoeditor --list-commands --json` の出力 (組込みのコマンド 88 個。CommandId の順) から作る。コマンドと既定キーの正本は [07 §11.6 のカタログ](../07-config-extensibility.md#116-v01-commandid-カタログ-付録) で、`cargo xtask gen-reference --check` が突合する。

- **種類**・**権限** は 07 §11.6 の kind / perm 列の表記 (権限の `-` は外部の主体から呼べない)。**when** の `—` は常に実行できる。
- **引数** は `名前` (`?` は省略可能): 型 (07 §11.2)。
- **既定キー** は組込みの既定キーマップの割当 (コンテキストごと。画面の表記、04 §7.8)。`[E]` は拡張キーが無い端末では届かない割当。利用者のキーマップを重ねた結果は `evoeditor --dump-keymap` で見る。
- **パレット** の「出さない」は `hidden` (オーバーレイ内専用と内部用、07 §11.6)。

| ID | タイトル | 種類 | 引数 | when | 権限 | 既定キー | パレット |
|---|---|---|---|---|---|---|---|
| `app.quit` | 終了 | Ui | — | — | `-` | `global`: `Ctrl+Q` | 出す |
| `app.suspend` | 一時停止 (サスペンド) | Ui | — | `caps.can_suspend` | `-` | `editor`: `Ctrl+K Z` | 出す |
| `buffer.close` | 閉じる | Ui | — | — | `-` | `editor`: `Ctrl+W`, `Ctrl+K W`<br>`list`: `Ctrl+W` | 出す |
| `buffer.list` | 一覧 | Ui | — | — | `ui` | `editor`: `Ctrl+E` | 出す |
| `buffer.next` | 次の文書 | Ui | — | — | `ui` | `editor`: `Ctrl+PgDn`, `Ctrl+K N` | 出す |
| `buffer.prev` | 前の文書 | Ui | — | — | `ui` | `editor`: `Ctrl+PgUp`, `Ctrl+K P` | 出す |
| `clipboard.copy` | コピー | Ui | — | `!selection.empty` | `-` | `editor`: `Ctrl+C`, `Ctrl+Insert` | 出す |
| `clipboard.cut` | 切り取り | Edit | — | `!selection.empty && !doc.read_only` | `-` | `editor`: `Ctrl+X`, `Shift+Delete` | 出す |
| `config.reload` | 設定を読み直す | Ui | — | — | `-` | — | 出す |
| `cursor.doc_end` | 文書の末尾へ移動 | Motion | — | — | `read` | `editor`: `Ctrl+End` | 出す |
| `cursor.doc_start` | 文書の先頭へ移動 | Motion | — | — | `read` | `editor`: `Ctrl+Home` | 出す |
| `cursor.end` | 行末へ移動 | Motion | — | — | `read` | `editor`: `End`, `Ctrl+K →`<br>`prompt`: `End` | 出す |
| `cursor.goto_line` | 行へ移動 | Motion | `line?`: `int\|"$"`<br>`col?`: `int` | — | `read` | `editor`: `Ctrl+G` | 出す |
| `cursor.home` | 行頭へ移動 | Motion | — | — | `read` | `editor`: `Home`, `Ctrl+K ←`<br>`prompt`: `Home` | 出す |
| `cursor.move_down` | 下へ移動 | Motion | `count?`: `int` | — | `read` | `editor`: `↓` | 出す |
| `cursor.move_left` | 左へ移動 | Motion | `count?`: `int` | — | `read` | `editor`: `←`<br>`prompt`: `←` | 出す |
| `cursor.move_right` | 右へ移動 | Motion | `count?`: `int` | — | `read` | `editor`: `→`<br>`prompt`: `→` | 出す |
| `cursor.move_up` | 上へ移動 | Motion | `count?`: `int` | — | `read` | `editor`: `↑` | 出す |
| `cursor.move_word_left` | 単語の左へ移動 | Motion | `count?`: `int` | — | `read` | `editor`: `Ctrl+←`, `Alt+←`, `Alt+B` | 出す |
| `cursor.move_word_right` | 単語の右へ移動 | Motion | `count?`: `int` | — | `read` | `editor`: `Ctrl+→`, `Alt+→`, `Alt+F` | 出す |
| `cursor.page_down` | 次のページへ移動 | Motion | — | — | `read` | `editor`: `PgDn`, `Ctrl+K .` | 出す |
| `cursor.page_up` | 前のページへ移動 | Motion | — | — | `read` | `editor`: `PgUp`, `Ctrl+K ,` | 出す |
| `edit.delete_backward` | 前の文字を削除 | Edit | — | `!doc.read_only` | `propose` | `editor`: `Backspace`<br>`prompt`: `Backspace` | 出す |
| `edit.delete_forward` | 後ろの文字を削除 | Edit | — | `!doc.read_only` | `propose` | `editor`: `Delete`<br>`prompt`: `Delete` | 出す |
| `edit.delete_line` | 行を削除 | Edit | — | `!doc.read_only` | `propose` | `editor`: `Ctrl+K L` | 出す |
| `edit.delete_word_backward` | 前の単語を削除 | Edit | — | `!doc.read_only` | `propose` | `editor`: `Ctrl+Backspace`, `Alt+Backspace` | 出す |
| `edit.delete_word_forward` | 後ろの単語を削除 | Edit | — | `!doc.read_only` | `propose` | `editor`: `Ctrl+Delete` | 出す |
| `edit.duplicate_line` | 行を複製 | Edit | — | `!doc.read_only` | `propose` | `editor`: `Ctrl+K D` | 出す |
| `edit.indent` | インデント | Edit | — | `!doc.read_only` | `propose` | `editor`: `Tab` | 出す |
| `edit.insert_text` | 文字を挿入 | Edit | `text`: `string` | `!doc.read_only` | `propose` | — | 出す |
| `edit.move_line_down` | 行を下へ移動 | Edit | — | `!doc.read_only` | `propose` | `editor`: `Alt+↓`, `Ctrl+K ↓` | 出す |
| `edit.move_line_up` | 行を上へ移動 | Edit | — | `!doc.read_only` | `propose` | `editor`: `Alt+↑`, `Ctrl+K ↑` | 出す |
| `edit.newline` | 改行 | Edit | — | `!doc.read_only` | `propose` | `editor`: `Enter` | 出す |
| `edit.outdent` | インデント解除 | Edit | — | `!doc.read_only` | `propose` | `editor`: `Shift+Tab` | 出す |
| `edit.paste` | 貼り付け | Edit | — | `!doc.read_only` | `-` | `editor`: `Ctrl+V`, `Shift+Insert`<br>`prompt`: `Ctrl+V`, `Shift+Insert` | 出す |
| `edit.redo` | やり直し | Edit | — | `!doc.read_only` | `-` | `editor`: `Ctrl+Y`, `Ctrl+Shift+Z` [E] | 出す |
| `edit.undo` | 元に戻す | Edit | — | `!doc.read_only` | `-` | `editor`: `Ctrl+Z` | 出す |
| `file.check_external_changes` | 外部変更の確認 | File | — | `doc.has_path` | `-` | — | 出す |
| `file.new` | 新規作成 | File | — | — | `-` | `editor`: `Ctrl+N`<br>`search`: `Ctrl+N` | 出す |
| `file.open` | 開く | File | `path?`: `path` | — | `-` | `editor`: `Ctrl+O`<br>`search`: `Ctrl+O` | 出す |
| `file.reload` | 再読込 | File | — | `doc.has_path` | `-` | — | 出す |
| `file.reopen_with_encoding` | 文字コードを指定して開き直す | File | `encoding`: `enum(utf-8\|utf-16le\|utf-16be\|shift_jis\|euc-jp\|iso-2022-jp)` | `doc.has_path` | `-` | — | 出す |
| `file.save` | 保存 | File | — | `!doc.read_only` | `-` | `editor`: `Ctrl+S`<br>`search`: `Ctrl+S` | 出す |
| `file.save_all` | すべて保存 | File | — | — | `-` | `editor`: `Ctrl+K A` | 出す |
| `file.save_as` | 名前を付けて保存 | File | `path?`: `path` | — | `-` | `editor`: `Ctrl+Shift+S` [E], `Ctrl+K S`<br>`search`: `Ctrl+Shift+S` [E] | 出す |
| `file.set_encoding` | 文字コードを変更 | File | `encoding`: `enum(utf-8\|utf-16le\|utf-16be\|shift_jis\|euc-jp\|iso-2022-jp)`<br>`bom?`: `bool` | — | `-` | — | 出す |
| `file.set_line_ending` | 改行コードを変換 | Edit | `eol`: `enum(lf\|crlf\|cr)` | `!doc.read_only` | `propose` | — | 出す |
| `file.toggle_read_only` | 読み取り専用の切替 | File | — | `doc.read_only_releasable \|\| !doc.read_only` | `-` | — | 出す |
| `help.commands` | コマンド一覧を開く | Ui | — | — | `ui` | — | 出す |
| `help.keys` | キー一覧を開く | Ui | — | — | `ui` | — | 出す |
| `help.open` | ヘルプを開く | Ui | — | — | `ui` | `global`: `Ctrl+K H` | 出す |
| `search.next` | 次を検索 | Motion | — | `search.active` | `read` | `search`: `Enter`, `↓`, `F3` | 出す |
| `search.open` | 検索 | Ui | `query?`: `string` | — | `ui` | `editor`: `Ctrl+F`<br>`search`: `Ctrl+F` | 出す |
| `search.prev` | 前を検索 | Motion | — | `search.active` | `read` | `search`: `↑`, `Shift+F3` [E], `Shift+Enter` [E] | 出す |
| `search.replace` | 置換 | Edit | — | `search.replace_active && !doc.read_only` | `propose` | `search`: `Enter` | 出す |
| `search.replace_all` | すべて置換 | Edit | — | `search.replace_active && !doc.read_only` | `propose` | `search`: `Alt+A` | 出す |
| `search.replace_open` | 置換 | Ui | — | `!doc.read_only` | `ui` | `editor`: `Ctrl+R`, `Ctrl+H` [E]<br>`search`: `Ctrl+R`, `Ctrl+H` [E] | 出す |
| `search.toggle_case` | 大小文字の区別を切替 | Ui | — | `search.active` | `ui` | `search`: `Alt+C`, `Ctrl+K C` | 出す |
| `search.toggle_regex` | 正規表現を切替 | Ui | — | `search.active` | `ui` | `search`: `Alt+R`, `Ctrl+K X` | 出す |
| `search.toggle_selection` | 選択範囲内を切替 | Ui | — | `search.active` | `ui` | `search`: `Alt+L`, `Ctrl+K L` | 出す |
| `search.toggle_word` | 単語単位を切替 | Ui | — | `search.active` | `ui` | `search`: `Alt+W`, `Ctrl+K B` | 出す |
| `select.all` | すべて選択 | Motion | — | — | `read` | `editor`: `Ctrl+A` | 出す |
| `select.extend_down` | 下へ選択 | Motion | `count?`: `int` | — | `read` | `editor`: `Shift+↓` | 出す |
| `select.extend_end` | 行末まで選択 | Motion | — | — | `read` | `editor`: `Shift+End`, `Ctrl+K Shift+→` | 出す |
| `select.extend_home` | 行頭まで選択 | Motion | — | — | `read` | `editor`: `Shift+Home`, `Ctrl+K Shift+←` | 出す |
| `select.extend_left` | 左へ選択 | Motion | `count?`: `int` | — | `read` | `editor`: `Shift+←` | 出す |
| `select.extend_page_down` | 次のページまで選択 | Motion | — | — | `read` | `editor`: `Shift+PgDn`, `Ctrl+K >` | 出す |
| `select.extend_page_up` | 前のページまで選択 | Motion | — | — | `read` | `editor`: `Shift+PgUp`, `Ctrl+K <` | 出す |
| `select.extend_right` | 右へ選択 | Motion | `count?`: `int` | — | `read` | `editor`: `Shift+→` | 出す |
| `select.extend_to_doc_end` | 文書の末尾まで選択 | Motion | — | — | `read` | `editor`: `Ctrl+Shift+End` [E] | 出す |
| `select.extend_to_doc_start` | 文書の先頭まで選択 | Motion | — | — | `read` | `editor`: `Ctrl+Shift+Home` [E] | 出す |
| `select.extend_up` | 上へ選択 | Motion | `count?`: `int` | — | `read` | `editor`: `Shift+↑` | 出す |
| `select.extend_word_left` | 単語の左へ選択 | Motion | `count?`: `int` | — | `read` | `editor`: `Ctrl+Shift+←`, `Alt+Shift+←` | 出す |
| `select.extend_word_right` | 単語の右へ選択 | Motion | `count?`: `int` | — | `read` | `editor`: `Ctrl+Shift+→`, `Alt+Shift+→` | 出す |
| `ui.cancel` | 取り消し | Ui | — | — | `-` | `editor`: `Esc`<br>`prompt`: `Esc`<br>`dialog`: `Esc`<br>`help`: `Esc` | 出す |
| `ui.confirm` | 決定 | Ui | — | `overlay.active != "" \|\| search.active` | `-` | `prompt`: `Enter`<br>`dialog`: `Enter` | 出さない |
| `ui.dialog_choose` | 選択肢を選ぶ | Ui | `choice`: `int` | `overlay.active == "dialog"` | `-` | — | 出さない |
| `ui.list_next` | 次の項目 | Ui | — | `overlay.active != "" \|\| search.active` | `-` | `search`: `Tab`<br>`palette`: `↓`, `Ctrl+N`<br>`dialog`: `Tab`, `→`, `↓`<br>`help`: `↓` | 出さない |
| `ui.list_page_down` | 一覧の次のページ | Ui | — | `overlay.active != "" \|\| search.active` | `-` | `palette`: `PgDn`<br>`help`: `PgDn` | 出さない |
| `ui.list_page_up` | 一覧の前のページ | Ui | — | `overlay.active != "" \|\| search.active` | `-` | `palette`: `PgUp`<br>`help`: `PgUp` | 出さない |
| `ui.list_prev` | 前の項目 | Ui | — | `overlay.active != "" \|\| search.active` | `-` | `search`: `Shift+Tab`<br>`palette`: `↑`, `Ctrl+P`<br>`dialog`: `Shift+Tab`, `←`, `↑`<br>`help`: `↑` | 出さない |
| `ui.message_history` | メッセージ履歴 | Ui | — | — | `ui` | `global`: `Ctrl+K M` | 出す |
| `ui.message_history_copy` | メッセージ履歴をコピー | Ui | — | — | `ui` | `help`: `Ctrl+C` | 出す |
| `ui.palette_open` | コマンドパレット | Ui | — | — | `-` | `global`: `Ctrl+P`, `F1`, `Ctrl+Shift+P` [E] | 出す |
| `ui.redraw` | 再描画 | Ui | — | — | `-` | `global`: `Ctrl+K R` | 出す |
| `ui.toggle_hint_bar` | ヒントバーの切り替え | Ui | — | — | `ui` | — | 出す |
| `ui.toggle_mouse` | マウスの切り替え | Ui | — | `caps.mouse` | `-` | — | 出す |
| `view.toggle_wrap` | 折り返しの切り替え | Ui | — | — | `ui` | `editor`: `Alt+Z`, `Ctrl+K O` | 出す |
