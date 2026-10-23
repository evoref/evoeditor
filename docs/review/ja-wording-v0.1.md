# 日本語 UI 文言のレビュー一覧 (v0.1)

`cargo xtask review-wording` が `locales/ja.toml` から生成した一覧です。手では直しません (文言を変えるときは `locales/ja.toml` と docs/04 §15 を直し、この一覧は作り直します)。

文言は全部で 434 個です。

**レビューの観点** (docs/11 §8、NFR-602): (1) 失敗のメッセージは「何が起きたか」と「どうすればよいか」が分かる。(2) 用語が統一されている (保存・書き込み、開く・読み込む、退避・バックアップ、など)。(3) 文末と敬体が揃っている (です・ます)。(4) 角括弧のキー表記 (`[S]` など) が実際のキーと合っている。(5) 引数の `{…}` に入る値で文が崩れない (長いパス・空の値)。(6) ヒントバーの表示は 8 セル以内。

**レビュー結果: 2026-10-23 に全 434 個を確認し、すべて `OK` と判定された** (PO)。文言を変えたら、この記録は無効になり、再生成してレビューをやり直す。

## コマンド (パレットの名前と説明)

126 個

| キー | 文言 | 出す場面 | 判定 |
|---|---|---|---|
| `cmd.app.quit.description` | エディタを終了します。 |  | OK |
| `cmd.app.quit.title` | 終了 |  | OK |
| `cmd.clipboard.copy.description` | 選択範囲をクリップボードにコピーします。 |  | OK |
| `cmd.clipboard.copy.title` | コピー |  | OK |
| `cmd.clipboard.cut.description` | 選択範囲を切り取ってクリップボードに入れます。 |  | OK |
| `cmd.clipboard.cut.title` | 切り取り |  | OK |
| `cmd.cursor.doc_end.description` | カーソルを文書の末尾へ動かします。 |  | OK |
| `cmd.cursor.doc_end.title` | 文書の末尾へ移動 |  | OK |
| `cmd.cursor.doc_start.description` | カーソルを文書の先頭へ動かします。 |  | OK |
| `cmd.cursor.doc_start.title` | 文書の先頭へ移動 |  | OK |
| `cmd.cursor.end.description` | カーソルを行の末尾へ動かします。 |  | OK |
| `cmd.cursor.end.title` | 行末へ移動 |  | OK |
| `cmd.cursor.goto_line.description` | 指定した行へカーソルを動かします。 |  | OK |
| `cmd.cursor.goto_line.title` | 行へ移動 |  | OK |
| `cmd.cursor.home.description` | カーソルを行の先頭へ動かします。 |  | OK |
| `cmd.cursor.home.title` | 行頭へ移動 |  | OK |
| `cmd.cursor.move_down.description` | カーソルを下の行へ動かします。 |  | OK |
| `cmd.cursor.move_down.title` | 下へ移動 |  | OK |
| `cmd.cursor.move_left.description` | カーソルを左へ動かします。 |  | OK |
| `cmd.cursor.move_left.title` | 左へ移動 |  | OK |
| `cmd.cursor.move_right.description` | カーソルを右へ動かします。 |  | OK |
| `cmd.cursor.move_right.title` | 右へ移動 |  | OK |
| `cmd.cursor.move_up.description` | カーソルを上の行へ動かします。 |  | OK |
| `cmd.cursor.move_up.title` | 上へ移動 |  | OK |
| `cmd.cursor.move_word_left.description` | カーソルを一つ前の単語の先頭へ動かします。 |  | OK |
| `cmd.cursor.move_word_left.title` | 単語の左へ移動 |  | OK |
| `cmd.cursor.move_word_right.description` | カーソルを次の単語の末尾へ動かします。 |  | OK |
| `cmd.cursor.move_word_right.title` | 単語の右へ移動 |  | OK |
| `cmd.cursor.page_down.description` | カーソルを一画面ぶん下へ動かします。 |  | OK |
| `cmd.cursor.page_down.title` | 次のページへ移動 |  | OK |
| `cmd.cursor.page_up.description` | カーソルを一画面ぶん上へ動かします。 |  | OK |
| `cmd.cursor.page_up.title` | 前のページへ移動 |  | OK |
| `cmd.edit.delete_backward.description` | カーソルの前の文字を削除します。 |  | OK |
| `cmd.edit.delete_backward.title` | 前の文字を削除 |  | OK |
| `cmd.edit.delete_forward.description` | カーソルの後ろの文字を削除します。 |  | OK |
| `cmd.edit.delete_forward.title` | 後ろの文字を削除 |  | OK |
| `cmd.edit.delete_word_backward.description` | カーソルの前の単語を削除します。 |  | OK |
| `cmd.edit.delete_word_backward.title` | 前の単語を削除 |  | OK |
| `cmd.edit.delete_word_forward.description` | カーソルの後ろの単語を削除します。 |  | OK |
| `cmd.edit.delete_word_forward.title` | 後ろの単語を削除 |  | OK |
| `cmd.edit.indent.description` | 行頭にインデントを入れます。 |  | OK |
| `cmd.edit.indent.title` | インデント |  | OK |
| `cmd.edit.insert_text.description` | カーソル位置に文字を挿入します。 |  | OK |
| `cmd.edit.insert_text.title` | 文字を挿入 |  | OK |
| `cmd.edit.newline.description` | カーソル位置で改行します。 |  | OK |
| `cmd.edit.newline.title` | 改行 |  | OK |
| `cmd.edit.outdent.description` | 行頭のインデントを一段浅くします。 |  | OK |
| `cmd.edit.outdent.title` | インデント解除 |  | OK |
| `cmd.edit.paste.description` | クリップボードの内容を貼り付けます。 |  | OK |
| `cmd.edit.paste.title` | 貼り付け |  | OK |
| `cmd.edit.redo.description` | 元に戻した編集をやり直します。 |  | OK |
| `cmd.edit.redo.title` | やり直し |  | OK |
| `cmd.edit.undo.description` | 直前の編集を取り消します。 |  | OK |
| `cmd.edit.undo.title` | 元に戻す |  | OK |
| `cmd.file.new.description` | 無題の新しい文書を作ります。 |  | OK |
| `cmd.file.new.title` | 新規作成 |  | OK |
| `cmd.file.open.description` | ファイルを開きます。 |  | OK |
| `cmd.file.open.title` | 開く |  | OK |
| `cmd.file.reload.description` | ファイルを読み込み直します。 |  | OK |
| `cmd.file.reload.title` | 再読込 |  | OK |
| `cmd.file.reopen_with_encoding.description` | 文字コードを指定して、ファイルを開き直します。 |  | OK |
| `cmd.file.reopen_with_encoding.title` | 文字コードを指定して開き直す |  | OK |
| `cmd.file.save.description` | 現在の文書を上書き保存します。 |  | OK |
| `cmd.file.save.title` | 保存 |  | OK |
| `cmd.file.save_as.description` | 現在の文書を別の名前で保存します。 |  | OK |
| `cmd.file.save_as.title` | 名前を付けて保存 |  | OK |
| `cmd.file.set_encoding.description` | 保存時に使う文字コードを変更します。 |  | OK |
| `cmd.file.set_encoding.title` | 文字コードを変更 |  | OK |
| `cmd.file.set_line_ending.description` | 文書の改行コードを LF・CRLF・CR のいずれかに変換します。 |  | OK |
| `cmd.file.set_line_ending.title` | 改行コードを変換 |  | OK |
| `cmd.search.next.description` | 次の一致へ移動します。 |  | OK |
| `cmd.search.next.title` | 次を検索 |  | OK |
| `cmd.search.open.description` | 検索バーを開きます。 |  | OK |
| `cmd.search.open.title` | 検索 |  | OK |
| `cmd.search.prev.description` | 前の一致へ移動します。 |  | OK |
| `cmd.search.prev.title` | 前を検索 |  | OK |
| `cmd.search.replace_all.description` | 一致した箇所をすべて置換します。 |  | OK |
| `cmd.search.replace_all.title` | すべて置換 |  | OK |
| `cmd.search.replace_open.description` | 置換欄つきで検索バーを開きます。 |  | OK |
| `cmd.search.replace_open.title` | 置換 |  | OK |
| `cmd.search.toggle_case.description` | 検索で大文字と小文字を区別するかを切り替えます。 |  | OK |
| `cmd.search.toggle_case.title` | 大小文字の区別を切替 |  | OK |
| `cmd.select.all.description` | 文書全体を選択します。 |  | OK |
| `cmd.select.all.title` | すべて選択 |  | OK |
| `cmd.select.extend_down.description` | 選択範囲を下の行へ広げます。 |  | OK |
| `cmd.select.extend_down.title` | 下へ選択 |  | OK |
| `cmd.select.extend_end.description` | 選択範囲を行の末尾まで広げます。 |  | OK |
| `cmd.select.extend_end.title` | 行末まで選択 |  | OK |
| `cmd.select.extend_home.description` | 選択範囲を行の先頭まで広げます。 |  | OK |
| `cmd.select.extend_home.title` | 行頭まで選択 |  | OK |
| `cmd.select.extend_left.description` | 選択範囲を左へ広げます。 |  | OK |
| `cmd.select.extend_left.title` | 左へ選択 |  | OK |
| `cmd.select.extend_page_down.description` | 選択範囲を一画面ぶん下へ広げます。 |  | OK |
| `cmd.select.extend_page_down.title` | 次のページまで選択 |  | OK |
| `cmd.select.extend_page_up.description` | 選択範囲を一画面ぶん上へ広げます。 |  | OK |
| `cmd.select.extend_page_up.title` | 前のページまで選択 |  | OK |
| `cmd.select.extend_right.description` | 選択範囲を右へ広げます。 |  | OK |
| `cmd.select.extend_right.title` | 右へ選択 |  | OK |
| `cmd.select.extend_to_doc_end.description` | 選択範囲を文書の末尾まで広げます。 |  | OK |
| `cmd.select.extend_to_doc_end.title` | 文書の末尾まで選択 |  | OK |
| `cmd.select.extend_to_doc_start.description` | 選択範囲を文書の先頭まで広げます。 |  | OK |
| `cmd.select.extend_to_doc_start.title` | 文書の先頭まで選択 |  | OK |
| `cmd.select.extend_up.description` | 選択範囲を上の行へ広げます。 |  | OK |
| `cmd.select.extend_up.title` | 上へ選択 |  | OK |
| `cmd.select.extend_word_left.description` | 選択範囲を一つ前の単語まで広げます。 |  | OK |
| `cmd.select.extend_word_left.title` | 単語の左へ選択 |  | OK |
| `cmd.select.extend_word_right.description` | 選択範囲を次の単語まで広げます。 |  | OK |
| `cmd.select.extend_word_right.title` | 単語の右へ選択 |  | OK |
| `cmd.ui.cancel.description` | 開いている入力欄やダイアログを閉じます。 |  | OK |
| `cmd.ui.cancel.title` | 取り消し |  | OK |
| `cmd.ui.confirm.description` | 入力内容や選択中の項目を決定します。 |  | OK |
| `cmd.ui.confirm.title` | 決定 |  | OK |
| `cmd.ui.dialog_choose.description` | ダイアログの選択肢を指定して選びます。 |  | OK |
| `cmd.ui.dialog_choose.title` | 選択肢を選ぶ |  | OK |
| `cmd.ui.list_next.description` | 一覧やダイアログで次の項目へ移ります。 |  | OK |
| `cmd.ui.list_next.title` | 次の項目 |  | OK |
| `cmd.ui.list_page_down.description` | 一覧を一画面ぶん下へ送ります。 |  | OK |
| `cmd.ui.list_page_down.title` | 一覧の次のページ |  | OK |
| `cmd.ui.list_page_up.description` | 一覧を一画面ぶん上へ戻します。 |  | OK |
| `cmd.ui.list_page_up.title` | 一覧の前のページ |  | OK |
| `cmd.ui.list_prev.description` | 一覧やダイアログで前の項目へ移ります。 |  | OK |
| `cmd.ui.list_prev.title` | 前の項目 |  | OK |
| `cmd.ui.palette_open.description` | コマンドパレットを開きます。 |  | OK |
| `cmd.ui.palette_open.title` | コマンドパレット |  | OK |
| `cmd.ui.redraw.description` | 画面を描き直します。 |  | OK |
| `cmd.ui.redraw.title` | 再描画 |  | OK |

## ヒントバーのキー表示 (8 セル以内)

14 個

| キー | 文言 | 出す場面 | 判定 |
|---|---|---|---|
| `hint.app.quit` | 終了 |  | OK |
| `hint.cursor.goto_line` | 行へ移動 |  | OK |
| `hint.edit.undo` | 元に戻す |  | OK |
| `hint.file.open` | 開く |  | OK |
| `hint.file.save` | 保存 |  | OK |
| `hint.search.next` | 次へ |  | OK |
| `hint.search.open` | 検索 |  | OK |
| `hint.search.prev` | 前へ |  | OK |
| `hint.search.replace_all` | 全置換 |  | OK |
| `hint.search.toggle_case` | 大小 |  | OK |
| `hint.ui.cancel` | 取消 |  | OK |
| `hint.ui.confirm` | 決定 |  | OK |
| `hint.ui.list_next` | 次へ |  | OK |
| `hint.ui.palette_open` | コマンド |  | OK |

## 確認ダイアログ

86 個

| キー | 文言 | 出す場面 | 判定 |
|---|---|---|---|
| `ui.confirm.cancel` | [Esc] キャンセル |  | OK |
| `ui.confirm.external_deleted` | {path} は削除されています — `[K] 新規として保持 (保存で作り直す)` ★ / `[C] 閉じる` | 外部で削除された (03 §9.5。`[C] 閉じる` は複数文書と同時。v0.1 は保存時の `ui.confirm.external_change` で扱う) | OK |
| `ui.confirm.external_modified` | {path} は他のプログラムで変更されています — `[L] 再読込 (自分の変更を破棄)` / `[I] 無視 (保存時に再確認)` ★ | 外部変更の検知 (フォーカス復帰・明示コマンド。dirty のとき、03 §9.5) | OK |
| `ui.confirm.nav_hint` | Tab / ←→ で選択、Enter で決定 |  | OK |
| `ui.confirm.open.binary.encoding` | [E] 文字コードを指定して開く | 同上 (アクセラレータ `E`。選ぶと `ui.prompt.encoding` で文字コードを尋ねる) | OK |
| `ui.confirm.open.binary.message` | {path} はバイナリファイルのようです | 同上 | OK |
| `ui.confirm.open.binary.readonly` | [R] 読み取り専用で開く | 同上 (既定。アクセラレータ `R`) | OK |
| `ui.confirm.open.binary.title` | バイナリファイル | バイナリ (FR-115) | OK |
| `ui.confirm.open.huge.message` | {path} は {size} あります。開くと多くのメモリを使います | 同上 | OK |
| `ui.confirm.open.huge.readonly` | [R] 読み取り専用で開く | 同上 (既定。アクセラレータ `R`) | OK |
| `ui.confirm.open.huge.title` | 巨大なファイル | 巨大ファイル (`file.huge_file_warn_bytes`、FR-127。`ConfirmAsk::HugeFile { len }`。選択肢の正本は 03 §9.2) | OK |
| `ui.confirm.open.invalid_bytes.abort` | [C] 中止 | 同上 (アクセラレータ `C`。`Esc` も同じ中止) | OK |
| `ui.confirm.open.invalid_bytes.encoding` | [E] 文字コードを指定して開く | 同上 (アクセラレータ `E`。選ぶと `ui.prompt.encoding` で文字コードを尋ねる) | OK |
| `ui.confirm.open.invalid_bytes.message` | {path} は {encoding} として不正なバイトが多く含まれます ({ratio})。開くとメモリを約 {factor} 倍使います | 同上 | OK |
| `ui.confirm.open.invalid_bytes.readonly` | [R] 読み取り専用で開く | 同上 (既定。アクセラレータ `R`) | OK |
| `ui.confirm.open.invalid_bytes.title` | 不正なバイトが多いファイル | 不正なバイトが多い (FR-127。`ConfirmAsk::ManyInvalidBytes { ratio, encoding, memory_factor }`。選択肢の正本は 03 §9.2。`{factor}` は `memory_factor` を小数 1 桁で示す) | OK |
| `ui.confirm.open.utf32.encoding` | [E] 文字コードを指定して開く | 同上 (アクセラレータ `E`。選ぶと `ui.prompt.encoding` で文字コードを尋ねる) | OK |
| `ui.confirm.open.utf32.message` | {path} の先頭に UTF-32 の BOM があります。UTF-32 には対応していません | 同上 | OK |
| `ui.confirm.open.utf32.readonly` | [R] 読み取り専用で開く | 同上 (既定。アクセラレータ `R`) | OK |
| `ui.confirm.open.utf32.title` | UTF-32 のファイル | UTF-32 の BOM (`FF FE 00 00` / `00 00 FE FF`) を検出した (03 §8.2 手順 1 / §8.9。`ConfirmAsk::UnsupportedBom { kind }`。選択肢はバイナリと同じ) | OK |
| `ui.confirm.quit_saving` | {path} の保存が終わっていません — `[W] 待つ` ★ / `[Q] 保存を中止して終了 (保存していない内容は退避します)` | 終了の要求 (`app.quit`) の後、実行中の保存ジョブが 5 秒待っても終わらない (遷移の正本は 03 §9.7 (f)、Host の手順は 05 §11.5。5 秒の間は確認を出さずに待つ)。「待つ」は完了まで待ち、成功したら終了する (失敗したら `ui.confirm.save_failed` 等の選択へ)。「保存を中止して終了」は置換の前の保存に cancel を送り、置換中・直接上書きの書込中は完了を待ってから、保存の結果に依らず dirty な文書を緊急退避と同じ形式で退避して終了する (05 §11.5。保存が失敗していたら、失敗のパスを終了時の stderr に `stderr.quit_abort.*` で出す)。`Esc` は「待つ」と同じで、確認を閉じて保存の待ちに戻るだけ (保存は取り消さず、終了の要求も取り下げない。§7.5)。確認の表示中に保存の結果が届いたら確認を閉じ、成功なら終了、失敗なら `ui.confirm.save_failed` 等の確認に置き換える (積まない)。保存中の終了のためのキーはこの 1 つだけ | OK |
| `ui.confirm.reload_discard.message` | {path} を読み込み直すと、保存していない変更が失われます (読み込み直した後に元に戻すで戻せます) | 同上 | OK |
| `ui.confirm.reload_discard.reload` | [L] 再読込 (変更を破棄) | 同上 (破壊的。既定にしない。アクセラレータ `L`。既定は `[Esc] キャンセル`) | OK |
| `ui.confirm.reload_discard.title` | 再読込 | dirty な文書で `file.reload` を実行した (03 §9.5 の「明示の再読込」の 2。既定はキャンセル。読み直した内容で読み取り専用になったときは元に戻せない (03 §7.2、提案、OQ-216) が、読込前には分からないので文言は変えない) | OK |
| `ui.confirm.save.atomic_unsupported.alt` | [A] 別名で保存 | 同上 (既定。アクセラレータ `A`。確定 (ユーザー回答)) | OK |
| `ui.confirm.save.atomic_unsupported.direct` | [O] 直接上書き (バックアップ付き) | 同上 (アクセラレータ `O`) | OK |
| `ui.confirm.save.atomic_unsupported.message` | {path} は安全な方法 (一時ファイルと置換) で保存できません: {reason} | 同上 | OK |
| `ui.confirm.save.atomic_unsupported.title` | 安全に保存できません | `file.atomic_save_fallback = "ask"` (既定) の AtomicUnsupported (ハードリンク・一時ファイル規則 (`InPlacePattern`) 以外: 他人の所有・ディレクトリに書けない・置換が ACL を引き継げない等、05 §4.4。`"fail"` では出さずに `msg.save.failed`。03 §9.7 (e)) | OK |
| `ui.confirm.save.conflict_changed.alt` | [A] 別名で保存 | 同上 (既定。アクセラレータ `A`) | OK |
| `ui.confirm.save.conflict_changed.message` | {path} は開いた後に他のプログラムで変更されています | 同上 | OK |
| `ui.confirm.save.conflict_changed.overwrite` | [O] 上書き | 同上 (アクセラレータ `O`。`[Esc]` = 中止。v0.1 に再読込の選択肢は無く、`file.reload` で読み直す、03 §9.5) | OK |
| `ui.confirm.save.conflict_changed.title` | 外部で変更されています | 保存時の外部変更の競合 (03 §9.3 手順 5a / 5c、§9.5)。開いた後に既存のファイルが変更された | OK |
| `ui.confirm.save.conflict_created.alt` | [A] 別名で保存 | 同上 (既定。アクセラレータ `A`) | OK |
| `ui.confirm.save.conflict_created.message` | {path} は、この文書を作った後に他のプログラムによって作られています | 同上 | OK |
| `ui.confirm.save.conflict_created.overwrite` | [O] 上書き | 同上 (アクセラレータ `O`。`[Esc]` = 中止。v0.1 に再読込の選択肢は無く、`file.reload` で読み直す、03 §9.5) | OK |
| `ui.confirm.save.conflict_created.title` | 同名のファイルがあります | 新規ファイル (存在しないパスを開いた文書・無題の名前を付けて保存) の保存の時点で、同名のファイルがある (`Expect::Absent` の Conflict) | OK |
| `ui.confirm.save.conflict_deleted.alt` | [A] 別名で保存 | 同上 (既定。アクセラレータ `A`) | OK |
| `ui.confirm.save.conflict_deleted.message` | {path} は開いた後に削除されました。上書きするとファイルを作り直します | 同上 | OK |
| `ui.confirm.save.conflict_deleted.overwrite` | [O] 上書き | 同上 (アクセラレータ `O`。`[Esc]` = 中止。v0.1 に再読込の選択肢は無く、`file.reload` で読み直す、03 §9.5) | OK |
| `ui.confirm.save.conflict_deleted.title` | 外部で削除されています | 同上。開いた後に既存のファイルが削除された (「上書き」はファイルを作り直す) | OK |
| `ui.confirm.save.dangling_link` | {path} はリンクで、リンク先 {target} がありません — `[C] リンク先に作成 (リンクを保つ)` ★ | 同上 (リンク先を作成する選択肢を出す版) | OK |
| `ui.confirm.save.escape_normalized.alt` | [A] 別名で保存 | 同上 (アクセラレータ `A`) | OK |
| `ui.confirm.save.escape_normalized.message` | {count} 行でエスケープシーケンスの書き方が変わります (文字は変わりません) | 同上 | OK |
| `ui.confirm.save.escape_normalized.save` | [S] 保存 | 同上 (アクセラレータ `S`。既定) | OK |
| `ui.confirm.save.escape_normalized.title` | エスケープの書き方が変わります | 区分 D (ISO-2022-JP のエスケープの書き方が変わる、03 §8.7)。文字は変わらないので「保存」が既定 | OK |
| `ui.confirm.save.hardlink.break` | [B] このファイルだけ切り離して保存 | 同上 (アクセラレータ `B`) | OK |
| `ui.confirm.save.hardlink.direct` | [K] リンクを保って直接上書き (バックアップ付き) | 同上 (アクセラレータ `K`) | OK |
| `ui.confirm.save.hardlink.message` | {path} には他に {links} 個の名前 (ハードリンク) があります | 同上 | OK |
| `ui.confirm.save.hardlink.title` | ハードリンクのファイル | `nlink > 1` (FR-106、OQ-73。既定はキャンセル) | OK |
| `ui.confirm.save.lossy_roundtrip.alt` | [A] 別名で保存 | 同上 (アクセラレータ `A`。既定) | OK |
| `ui.confirm.save.lossy_roundtrip.message` | {count} 箇所が、同じ文字の別のバイト表現で保存されます (例: 元の先頭から {example} バイト目の行) | 同上 | OK |
| `ui.confirm.save.lossy_roundtrip.save` | [S] 保存 | 同上 (アクセラレータ `S`) | OK |
| `ui.confirm.save.lossy_roundtrip.title` | 別のバイト表現で保存されます | E2' 区分 B の確認 (03 §8.6)。元の文字コードのまま、別のバイト表現で保存される箇所がある文書を編集して保存するとき。`{example}` は最初の該当行の、元のファイルでの先頭からのバイト位置 | OK |
| `ui.confirm.save.overwrite` | {path} は既にあります。上書きしますか? — `[O] 上書き` / `[R] 別の名前を入力` ★ (既定は別の名前を入力。確定 (ユーザー回答)) | 名前を付けて保存で、入力した保存先に既にファイルがある (確認を閉じる `Esc` は名前の入力プロンプトに戻る、§7.5) | OK |
| `ui.confirm.save.overwrite_without_backup.alt` | [A] 別名で保存 | 同上 (アクセラレータ `A`) | OK |
| `ui.confirm.save.overwrite_without_backup.message` | {path} のバックアップを作れませんでした。バックアップなしで上書きしますか? 上書きの途中で失敗・中断すると、元の内容は取り戻せません | 同上 | OK |
| `ui.confirm.save.overwrite_without_backup.overwrite` | [O] バックアップなしで上書き | 同上 (破壊的。既定にしない。アクセラレータ `O`) | OK |
| `ui.confirm.save.overwrite_without_backup.title` | バックアップを作れません | BackupFailed の後 (事前バックアップ、または in-place の保存中フラグを書けなかった。05 §4.4 手順 2 / 4。既定はキャンセル) | OK |
| `ui.confirm.save.read_only` | {path} は読み取り専用です — `[A] 別名で保存` ★ / `[W] 読み取り専用を解除して保存` | 読み取り専用のファイル (FR-116。v0.1 は読み取り専用の文書の編集を拒否するので出ない。v0.1 で読み取り専用の文書に使える保存は名前を付けて保存だけ、03 §9.6) | OK |
| `ui.confirm.save.unmappable.message` | {count} 個の文字は {encoding} で表せません (最初: 行 {line}, 桁 {col}) | 同上 | OK |
| `ui.confirm.save.unmappable.replace` | [R] 置換して保存 | 同上 (アクセラレータ `R`。文書が編集できるときだけ。置換の一覧は `ui.confirm.save.unmappable_replace.*`) | OK |
| `ui.confirm.save.unmappable.title` | 表現できない文字 | E3 表現不能 (03 §8.6 の「E3 の選択肢」。選択肢と出す条件の正本は 03) | OK |
| `ui.confirm.save.unmappable.utf8` | [U] UTF-8 で保存 | 同上 (アクセラレータ `U`。表現不能の文字にエスケープ文字 (03 §8.3) を含むとき・保存先が UTF-8 のときは出さない) | OK |
| `ui.confirm.save.unmappable_replace.item` | 行 {line}, 桁 {col}: {char} ({codepoint}) → {replacement} | 上の一覧の 1 行 (先頭 100 件) | OK |
| `ui.confirm.save.unmappable_replace.message` | 次の {total} 個の文字を置き換えて {encoding} で保存します (全角カナへ {kana} 個、互換文字へ {compat} 個、`?` へ {question} 個) | 同上 (本文。件数は 0 でもこの順で出す。`{total}` = `kana + compat + question`、02 §9.3 の `ReplaceCounts`) | OK |
| `ui.confirm.save.unmappable_replace.more` | ほか {count} 個 | 上の一覧の最後の行 (101 件目以降があるとき) | OK |
| `ui.confirm.save.unmappable_replace.replace` | [R] 置換して保存 | 同上 (アクセラレータ `R`。既定は `[Esc] キャンセル`) | OK |
| `ui.confirm.save.unmappable_replace.title` | 置換の確認 | E3 で「置換して保存」を選んだ後、置換の前の一覧 (03 §8.6 の「置換の一覧」。一覧は `Confirm.details` に 1 件 1 行で入る。行の文言は下の `.item` / `.more`) | OK |
| `ui.confirm.save_failed.alt` | [A] 別名で保存 | 同上 (Busy 以外の既定。アクセラレータ `A`) | OK |
| `ui.confirm.save_failed.message` | {detail} | 同上 (本文。`{detail}` に対応する `msg.save.*` の text を入れる) | OK |
| `ui.confirm.save_failed.recovery` | [R] 退避先に保存 | 同上 (アクセラレータ `R`。結果は `info.save.to_recovery` / `msg.save.recovery_failed`) | OK |
| `ui.confirm.save_failed.retry` | [T] 再試行 | 同上 (Busy のときだけ。既定。アクセラレータ `T`) | OK |
| `ui.confirm.save_failed.title` | 保存できません | 保存の失敗の後の選択。出す場面と選択肢の正本は 03 §9.7 (e) の表で、対象は Busy / PermissionDenied / DiskFull / ReadOnlyFs / InPlaceFailed / ReplacedMovedToBackup / ParentMissing / DanglingLink / その他 (InvalidPath・Other・ジョブのパニック等) の 9 種。本文に対応する `msg.save.*` (`busy` / `permission_denied` / `disk_full` / `read_only_fs` / `in_place_failed` (`backup: None` なら `in_place_failed_no_backup`) / `moved_to_backup` / `parent_missing` / `dangling_link` / `invalid_path` または `failed`) の text を入れて 1 つの確認として出す。**`[T] 再試行` は Busy のときだけ** 出す (何も書いていないので照合は通る)。PermissionDenied (上書き保存) では文書に `NoWritePermission` の理由が付き、読み取り専用になる (03 §9.6)。ReplacedMovedToBackup の後に「元の名前に戻す」選択肢は出さない (v0.1。利用者が {backup} の名前を手で戻す、05 §4.6)。`[R] 退避先に保存` の結果は `info.save.to_recovery` / `msg.save.recovery_failed`。`Esc` は中止 | OK |
| `ui.confirm.save_failed_closed` | 保存できませんでした: (`msg.save.*` の text) — {path} は既に閉じています。`[R] 退避先に保存` ★ / `[D] 破棄` | 閉じた文書 (v0.1 は置き換えで閉じた旧文書) の保存ジョブが失敗した。閉じた文書なので別名で保存・再試行は出さず、保持している `DocSnapshot` から「退避先に保存」か「破棄」の 2 択にする (03 §9.2 の「閉じる手順」(a)。本文には失敗の種類に対応する `msg.save.*` の text を入れる。`Esc` の扱いは 03 の手順に従う。提案、OQ-199。03 の同項と同じ扱い) | OK |
| `ui.confirm.unsaved.discard` | [D] 破棄して終了 | 同上 (破棄する選択肢。既定にしない。アクセラレータ `D`) | OK |
| `ui.confirm.unsaved.message` | {path} には保存されていない変更があります | 同上 | OK |
| `ui.confirm.unsaved.save` | [S] 保存して終了 | 同上 (既定の選択肢。アクセラレータ `S`) | OK |
| `ui.confirm.unsaved.title` | 未保存の変更 | 未保存で終了する (FR-107。v0.2 からは文書を閉じるときも) | OK |
| `ui.confirm.unsaved_reopen.discard` | [D] 破棄して開き直す | 同上 (破棄する選択肢。既定にしない。アクセラレータ `D`) | OK |
| `ui.confirm.unsaved_reopen.message` | {path} には保存されていない変更があります | 同上 | OK |
| `ui.confirm.unsaved_reopen.save` | [S] 保存してから開き直す | 同上 (既定の選択肢。アクセラレータ `S`) | OK |
| `ui.confirm.unsaved_reopen.title` | 未保存の変更 | 未保存の文書を、文字コードを指定して開き直す (01 FR-111 AC-2) | OK |
| `ui.confirm.unsaved_replace.discard` | [D] 破棄して開く | 同上 (破棄する選択肢。既定にしない。アクセラレータ `D`) | OK |
| `ui.confirm.unsaved_replace.message` | {path} には保存されていない変更があります | 同上 | OK |
| `ui.confirm.unsaved_replace.save` | [S] 保存してから開く | 同上 (既定の選択肢。アクセラレータ `S`) | OK |
| `ui.confirm.unsaved_replace.title` | 未保存の変更 | 未保存の文書を、別のファイルを開く・新規作成で置き換える (v0.1 は単一文書、§7.2) | OK |

## 入力プロンプト

11 個

| キー | 文言 | 出す場面 | 判定 |
|---|---|---|---|
| `ui.prompt.arg.title` | {arg} |  | OK |
| `ui.prompt.encoding.error.invalid` | 対応していない文字コードです |  | OK |
| `ui.prompt.encoding.hint` | UTF-8、Shift_JIS、EUC-JP、ISO-2022-JP、UTF-16LE、UTF-16BE |  | OK |
| `ui.prompt.encoding.title` | 文字コードを指定して開く |  | OK |
| `ui.prompt.goto_line.error.invalid` | 行番号を入力してください (例: 12、12:5、$) |  | OK |
| `ui.prompt.goto_line.hint` | 行、行:桁、$ (最終行) |  | OK |
| `ui.prompt.goto_line.title` | 行へ移動 |  | OK |
| `ui.prompt.open.error.empty` | パスを入力してください |  | OK |
| `ui.prompt.open.title` | 開く |  | OK |
| `ui.prompt.save_as.error.empty` | パスを入力してください |  | OK |
| `ui.prompt.save_as.title` | 名前を付けて保存 |  | OK |

## 画面の文言 (検索バー・パレット・その他)

47 個

| キー | 文言 | 出す場面 | 判定 |
|---|---|---|---|
| `ui.config.type.array` | 配列 | `{actual}` / `{expected}` に入れる型の語 | OK |
| `ui.config.type.bool` | 真偽値 (`true` か `false`) | `{actual}` / `{expected}` に入れる型の語 | OK |
| `ui.config.type.float` | 小数 | `{actual}` / `{expected}` に入れる型の語 | OK |
| `ui.config.type.integer` | 整数 | `{actual}` / `{expected}` に入れる型の語 | OK |
| `ui.config.type.string` | 文字列 | `{actual}` / `{expected}` に入れる型の語 | OK |
| `ui.config.type.table` | テーブル | `{actual}` / `{expected}` に入れる型の語 | OK |
| `ui.key.via_palette` | コマンドパレットから実行 |  | OK |
| `ui.load_state.loading` | 読込中 | 読込中の文書への編集操作を拒否したときの `msg.edit.read_only` の `{reason}` (読込中は読み取り専用の理由ではなく `load_state == Loading` から導出する状態で、03 §9.6。`ui.read_only_reason.*` には入れない。ステータスバーの読込中の表示は `status.loading.long` / `.short`、§8.7) | OK |
| `ui.long_line.omitted` | … (以下 {bytes} バイト省略) | 閾値 (1 MiB) を超える行の、先頭 64 KiB より後ろを省略した印 (行末に出す。NFR-106、03 §11.5)。`{bytes}` は省略したバイト数 | OK |
| `ui.palette.arg.encoding` | 文字コード |  | OK |
| `ui.palette.arg.eol` | 改行コード |  | OK |
| `ui.palette.arg_title` | {command}: {arg} |  | OK |
| `ui.palette.category.app` | アプリ |  | OK |
| `ui.palette.category.clipboard` | クリップボード |  | OK |
| `ui.palette.category.cursor` | 移動 |  | OK |
| `ui.palette.category.edit` | 編集 |  | OK |
| `ui.palette.category.file` | ファイル |  | OK |
| `ui.palette.category.search` | 検索 |  | OK |
| `ui.palette.category.select` | 選択 |  | OK |
| `ui.palette.category.ui` | 画面 |  | OK |
| `ui.palette.disabled` | 現在は実行できません: {reason} |  | OK |
| `ui.palette.footer` | {matched}/{total} |  | OK |
| `ui.palette.no_match` | 一致するコマンドはありません |  | OK |
| `ui.palette.placeholder` | コマンドを入力 |  | OK |
| `ui.palette.reason.no_changes` | 未保存の変更がありません |  | OK |
| `ui.palette.reason.no_path` | 文書に保存先のパスがありません |  | OK |
| `ui.palette.reason.no_selection` | 選択範囲がありません |  | OK |
| `ui.palette.reason.read_only` | 文書が読み取り専用です |  | OK |
| `ui.palette.reason.unavailable` | 今の状態では使えません |  | OK |
| `ui.palette.title` | コマンド |  | OK |
| `ui.read_only_reason.binary` | バイナリファイル | 同上。`Binary` | OK |
| `ui.read_only_reason.file_attribute` | 読み取り専用属性のファイル | 上の `{reason}` と、ステータスバーの理由の表示。`ReadOnlyReason::FileAttribute` (03 §9.6。以下の 8 行は 03 §9.6 の `ReadOnlyReason` と 1 対 1 で、並びは 03 §9.6 の表の順 = 表示の優先順。v0.1 で付く理由だけで、v0.1 では付かない `LoadIncomplete` の文言は入れる版で加える) | OK |
| `ui.read_only_reason.invalid_bytes` | 不正なバイトを含むファイル | 同上。`InvalidBytes` | OK |
| `ui.read_only_reason.large_file_viewer` | 巨大なファイル (読み取り専用で開く指定) | 同上。`LargeFileViewer` | OK |
| `ui.read_only_reason.long_line` | 非常に長い行を含むファイル | 同上。`LongLine` | OK |
| `ui.read_only_reason.no_write_permission` | 書き込む権限のないファイル | 同上。`NoWritePermission` | OK |
| `ui.read_only_reason.stateful_encoding_with_invalid_bytes` | 不正なバイトを含む ISO-2022-JP のファイル | 同上。`StatefulEncodingWithInvalidBytes` | OK |
| `ui.read_only_reason.user_requested` | 読み取り専用で開く指定 (`--readonly`) | 同上。`UserRequested` | OK |
| `ui.search.count.approx` | 約 {total} |  | OK |
| `ui.search.count.at_least` | {current}/{total}+ |  | OK |
| `ui.search.count.exact` | {current}/{total} |  | OK |
| `ui.search.count.no_match` | 0 件 |  | OK |
| `ui.search.count.running` | {found}/… |  | OK |
| `ui.search.label` | 検索: |  | OK |
| `ui.search.replace_all_button` | [A] すべて置換 |  | OK |
| `ui.search.replace_label` | 置換: |  | OK |
| `ui.too_small` | 端末が小さすぎます ({cols}x{rows} 以上が必要) |  | OK |

## メッセージ: 失敗・警告 (何が + どうする)

104 個

| キー | 文言 | 出す場面 | 判定 |
|---|---|---|---|
| `msg.cli.directory.hint` | ファイルのパスを指定して起動してください | 位置引数にフォルダを渡した (07 §18.2。v0.1 は無題の文書を開く) | OK |
| `msg.cli.directory.text` | {path} はフォルダなので開けません。無題の文書を開きました | 位置引数にフォルダを渡した (07 §18.2。v0.1 は無題の文書を開く) | OK |
| `msg.clipboard.fallback.hint` | 他のアプリとの間でコピー・貼り付けするには、端末の範囲選択と貼り付けを使ってください | システムのクリップボードを使えず、内部レジスタに切り替えた (01 FR-205 AC-2。同じ起動の中では最初の 1 回だけ出す) | OK |
| `msg.clipboard.fallback.text` | システムのクリップボードを使えないため、エディタ内のクリップボードを使いました: {reason} | システムのクリップボードを使えず、内部レジスタに切り替えた (01 FR-205 AC-2。同じ起動の中では最初の 1 回だけ出す) | OK |
| `msg.config.invalid_value.hint` | 使える値: {allowed}。この項目は既定値 {default} を使います | 列挙値不正 (`file.default_encoding` の許容値外を含む)。`{allowed}` は許容値を `, ` で並べたもの | OK |
| `msg.config.invalid_value.text` | {item} の値 {value} は使えません | 列挙値不正 (`file.default_encoding` の許容値外を含む)。`{allowed}` は許容値を `, ` で並べたもの | OK |
| `msg.config.out_of_range.hint` | {range} で指定してください。この項目は {default} を使います | 範囲外。`{range}` は許容範囲 (例 `1〜16`)。`{default}` は戻した値 (丸める項目 (07 §4.2) は丸めた値) | OK |
| `msg.config.out_of_range.text` | {item} の値 {value} は範囲外です | 範囲外。`{range}` は許容範囲 (例 `1〜16`)。`{default}` は戻した値 (丸める項目 (07 §4.2) は丸めた値) | OK |
| `msg.config.problem.hint` | {hint} | 設定の診断が 1 件 (07 §4.3) | OK |
| `msg.config.problem.text` | 設定の問題 ({file}:{line}): {detail} | 設定の診断が 1 件 (07 §4.3) | OK |
| `msg.config.problems.hint` | 該当の項目は既定値を使っています。全件はログ ({log}) を見てください | 設定の診断が 2 件以上 | OK |
| `msg.config.problems.text` | 設定に {count} 件の問題があります。最初の 1 件: {detail} | 設定の診断が 2 件以上 | OK |
| `msg.config.syntax.hint` | 該当の行を直してください。直すまでは既定値で動きます | TOML 構文エラー。そのファイル全体を使わない。`{reason}` は構文エラーの短い説明 (パーサの出力を分類し無害化したもの) | OK |
| `msg.config.syntax.text` | この設定ファイルは書式が正しくないため使っていません: {reason} | TOML 構文エラー。そのファイル全体を使わない。`{reason}` は構文エラーの短い説明 (パーサの出力を分類し無害化したもの) | OK |
| `msg.config.type_mismatch.hint` | {expected} で書いてください。この項目は既定値 {default} を使います | 型不一致。`{actual}` は書かれた値の型、`{expected}` は期待する型 (どちらも下の `ui.config.type.*`)。該当項目は下位層の値 (v0.1 は既定値) に戻す | OK |
| `msg.config.type_mismatch.text` | {item} の値の型が違います (書かれた型: {actual}) | 型不一致。`{actual}` は書かれた値の型、`{expected}` は期待する型 (どちらも下の `ui.config.type.*`)。該当項目は下位層の値 (v0.1 は既定値) に戻す | OK |
| `msg.config.unknown_key.hint` | 〔{suggestion} ではありませんか?〕設定できる項目は同梱の config.example.toml を見てください | 未知のキー。`{suggestion}` は編集距離 ≤ 2 の候補 (公開キーと公開前のキー (07 §5.13) の両方から探す)。候補が無ければ 〔 〕 の文を省く | OK |
| `msg.config.unknown_key.text` | {item} という設定項目はありません | 未知のキー。`{suggestion}` は編集距離 ≤ 2 の候補 (公開キーと公開前のキー (07 §5.13) の両方から探す)。候補が無ければ 〔 〕 の文を省く | OK |
| `msg.edit.read_only.hint` | 属性・権限が理由なら、名前を付けて保存 ({key:file.save_as}) した先で編集できます。文字コードの誤判定なら、文字コードを指定して開き直す ({key:file.reopen_with_encoding}) で直せます | 読み取り専用の文書への編集操作を拒否した (01 FR-116 AC-1、NFR-106 AC-3。同じ理由の拒否が続くときは 1 回にまとめる) | OK |
| `msg.edit.read_only.text` | 読み取り専用のため編集できません ({reason}) | 読み取り専用の文書への編集操作を拒否した (01 FR-116 AC-1、NFR-106 AC-3。同じ理由の拒否が続くときは 1 回にまとめる) | OK |
| `msg.edit.replacing.hint` | 読込が終わるまでお待ちください。やめるときは {key:ui.cancel} で読込を取り消せます | 別のファイルを開く・新規作成で文書を置き換えている間 (読込中。旧文書は読み取り専用として扱う) の編集操作を拒否した (03 §9.2 の「置き換えの手順」。理由は `ReadOnlyReasons` に入れない。専用のキーで、`msg.edit.read_only` と `{reason}` は流用しない。同じ理由の拒否が続くときは 1 回にまとめる) | OK |
| `msg.edit.replacing.text` | 別のファイルを読み込み中のため、今の文書は編集できません | 別のファイルを開く・新規作成で文書を置き換えている間 (読込中。旧文書は読み取り専用として扱う) の編集操作を拒否した (03 §9.2 の「置き換えの手順」。理由は `ReadOnlyReasons` に入れない。専用のキーで、`msg.edit.read_only` と `{reason}` は流用しない。同じ理由の拒否が続くときは 1 回にまとめる) | OK |
| `msg.eol.convert_stale.hint` | 入力を止めてから、もう一度実行してください | 変換の準備中に文書が編集され続け、作り直しても間に合わなかった (03 §8.4 の (4)。文書は変わらない) | OK |
| `msg.eol.convert_stale.text` | 編集が続いたため、改行コードの変換を中止しました | 変換の準備中に文書が編集され続け、作り直しても間に合わなかった (03 §8.4 の (4)。文書は変わらない) | OK |
| `msg.io.stalled.hint` | しばらく待つか、{key:ui.cancel} で取り消してください。他のファイル操作はその間も続けられます | `HostReport::IoJobStalled { job }` を受けた (02 §5.4 / §9.1。ファイルの読み書きが応答しない状態が監視時間 (5 秒) を超えた。止まったジョブは失敗にせず、終端の結果が届けば通常どおり扱う。提案、OQ-179) | OK |
| `msg.io.stalled.text` | ファイルの操作が応答していません (ネットワーク上のファイルなど) | `HostReport::IoJobStalled { job }` を受けた (02 §5.4 / §9.1。ファイルの読み書きが応答しない状態が監視時間 (5 秒) を超えた。止まったジョブは失敗にせず、終端の結果が届けば通常どおり扱う。提案、OQ-179) | OK |
| `msg.open.busy.hint` | そのプログラムを閉じてから開き直してください | 他のプロセスが共有を許さずに開いている (共有違反) | OK |
| `msg.open.busy.text` | 他のプログラムが {path} を使用中のため読めませんでした | 他のプロセスが共有を許さずに開いている (共有違反) | OK |
| `msg.open.changed_during_load.hint` | 内容を確かめ、必要なら文字コードを指定して開き直す ({key:file.reopen_with_encoding}) で読み直してください | 読み込んでいる間にファイルが変わった (`changed_during_load`、05 §4.5) | OK |
| `msg.open.changed_during_load.text` | 読み込んでいる間に {path} が変更されました | 読み込んでいる間にファイルが変わった (`changed_during_load`、05 §4.5) | OK |
| `msg.open.failed.hint` | パスと権限を確かめてください | 分類できない失敗 (読み始める前) | OK |
| `msg.open.failed.text` | {path} を開けませんでした: {reason} | 分類できない失敗 (読み始める前) | OK |
| `msg.open.interrupted_new.hint` | 新しいファイルとして開きました。内容を入力し直して保存してください。〔書きかけの一時ファイル {tmp} が残っています。不要なら削除してください。〕 | 存在しないパスを開いたとき、そのパスへの新規作成の保存中フラグ (`method = NewFile`) が残っていた (`InterruptedSave { method: NewFile }`。新規作成の保存が完了せず、ファイルは作られていない。写像の正本は 02 §4.5、03 §9.2)。`{tmp}` は `msg.open.interrupted_save` と同じ規則 (`tmp` が None なら 〔 〕 の文を省く) | OK |
| `msg.open.interrupted_new.text` | {path} の前回の保存 (新規作成) は完了していません。ファイルは作成されていません | 存在しないパスを開いたとき、そのパスへの新規作成の保存中フラグ (`method = NewFile`) が残っていた (`InterruptedSave { method: NewFile }`。新規作成の保存が完了せず、ファイルは作られていない。写像の正本は 02 §4.5、03 §9.2)。`{tmp}` は `msg.open.interrupted_save` と同じ規則 (`tmp` が None なら 〔 〕 の文を省く) | OK |
| `msg.open.interrupted_save.hint` | 元の内容が {backup} に残っている可能性があります。確かめてから開き直してください。〔書きかけの一時ファイル {tmp} が残っています。不要なら削除してください。〕 | 開くときに保存中フラグが残っていた (v0.1、05 §4.6。`InterruptedSave { backup: Some, complete: true }`)。(a) T が無い (NotFound): `{backup}` は記録された退避名 (`.eve-bak-*`)。(b) T があり、記録が直接上書き (`method = InPlace`): T は上書きの途中で中断され壊れている可能性があり、`{backup}` は `state_dir/backup/` の完全なバックアップ。`{tmp}` は `InterruptedSave.tmp` (実在する書きかけの一時ファイル) で、`tmp` が None のときは 〔 〕 の文を省く | OK |
| `msg.open.interrupted_save.text` | {path} の前回の保存が中断されていました | 開くときに保存中フラグが残っていた (v0.1、05 §4.6。`InterruptedSave { backup: Some, complete: true }`)。(a) T が無い (NotFound): `{backup}` は記録された退避名 (`.eve-bak-*`)。(b) T があり、記録が直接上書き (`method = InPlace`): T は上書きの途中で中断され壊れている可能性があり、`{backup}` は `state_dir/backup/` の完全なバックアップ。`{tmp}` は `InterruptedSave.tmp` (実在する書きかけの一時ファイル) で、`tmp` が None のときは 〔 〕 の文を省く | OK |
| `msg.open.interrupted_save_incomplete.hint` | ファイル {path} は書き換わる前に中断した見込みです。内容を確かめ、バックアップは確かめてから削除してください | 同上で、バックアップを書き終える前に中断していた (`complete: false`。05 §4.4 手順 10。直接上書きはまだ始まっていない。`{backup}` は実在する一時名 `<最終名>.tmp`、02 §4.5 の「中断の痕跡の写像」) | OK |
| `msg.open.interrupted_save_incomplete.text` | {path} の前回の保存が中断されていました。バックアップ {backup} は途中までしか書かれていません | 同上で、バックアップを書き終える前に中断していた (`complete: false`。05 §4.4 手順 10。直接上書きはまだ始まっていない。`{backup}` は実在する一時名 `<最終名>.tmp`、02 §4.5 の「中断の痕跡の写像」) | OK |
| `msg.open.interrupted_save_no_backup.hint` | 内容を確かめてから編集してください。〔書きかけの一時ファイル {tmp} が残っています。不要なら削除してください。〕 | 同上で、バックアップが無い (`backup: None`)。「バックアップなしで上書き」を承認した直接上書きの中断と、写像の「上書きの可能性あり」の行 (記録にあるバックアップが無い・不正な組合せ・解析できないフラグ。フラグは残す、02 §4.5 / 03 §9.2) の両方で出す。`{tmp}` は `msg.open.interrupted_save` と同じ規則 (`tmp` が None なら 〔 〕 の文を省く) | OK |
| `msg.open.interrupted_save_no_backup.text` | {path} の前回の保存は、上書きの途中で中断した可能性があります。ファイルが壊れているおそれがあり、バックアップは確認できません | 同上で、バックアップが無い (`backup: None`)。「バックアップなしで上書き」を承認した直接上書きの中断と、写像の「上書きの可能性あり」の行 (記録にあるバックアップが無い・不正な組合せ・解析できないフラグ。フラグは残す、02 §4.5 / 03 §9.2) の両方で出す。`{tmp}` は `msg.open.interrupted_save` と同じ規則 (`tmp` が None なら 〔 〕 の文を省く) | OK |
| `msg.open.not_a_file.hint` | ファイルのパスを指定してください | フォルダ・デバイス等を開こうとした (05 §4.5 の `NotAFile`。起動時の引数でフォルダを渡した場合は下の `msg.cli.directory`) | OK |
| `msg.open.not_a_file.text` | {path} はファイルではありません (フォルダ等) | フォルダ・デバイス等を開こうとした (05 §4.5 の `NotAFile`。起動時の引数でフォルダを渡した場合は下の `msg.cli.directory`) | OK |
| `msg.open.out_of_memory.hint` | 他のプログラムを閉じてから開き直してください | 読込用の確保に失敗した (`try_reserve`、03 §9.2。プロセスは続ける) | OK |
| `msg.open.out_of_memory.text` | メモリが足りないため {path} の読込を中止しました | 読込用の確保に失敗した (`try_reserve`、03 §9.2。プロセスは続ける) | OK |
| `msg.open.parent_missing.hint` | フォルダを作ってから保存するか、別名で保存 ({key:file.save_as}) してください | 存在しないパスを開き、そのフォルダも無い (`JobResult::NewFile` の `parent_missing`、03 §9.2 手順 2)。引数は `{path}` (開いたパス) と `{dir}` (`parent_missing` に入った、無いフォルダ) | OK |
| `msg.open.parent_missing.text` | {path} のフォルダ {dir} がありません。新しいファイルとして開きましたが、このままでは保存できません | 存在しないパスを開き、そのフォルダも無い (`JobResult::NewFile` の `parent_missing`、03 §9.2 手順 2)。引数は `{path}` (開いたパス) と `{dir}` (`parent_missing` に入った、無いフォルダ) | OK |
| `msg.open.permission_denied.hint` | ファイルの権限を確かめてください | 読む権限が無い | OK |
| `msg.open.permission_denied.text` | {path} を読む権限がありません | 読む権限が無い | OK |
| `msg.open.read_failed.hint` | もう一度開く ({key:file.open}) か、パスと接続を確かめてください | 読込の途中の I/O エラー (03 §9.2 の「失敗」。読めた部分は捨てる。確認ダイアログ (再試行・読めた部分を読み取り専用で開く) は v0.3 以降の予約で、v0.1 はメッセージだけ) | OK |
| `msg.open.read_failed.text` | {path} の読込の途中で失敗しました: {reason} | 読込の途中の I/O エラー (03 §9.2 の「失敗」。読めた部分は捨てる。確認ダイアログ (再試行・読めた部分を読み取り専用で開く) は v0.3 以降の予約で、v0.1 はメッセージだけ) | OK |
| `msg.open.reserved_name.hint` | 別の名前を指定してください | Windows の予約名 (`CON`・`NUL` 等。05 §4.5 の `ReservedName`) | OK |
| `msg.open.reserved_name.text` | {path} は Windows で予約された名前なので開けません | Windows の予約名 (`CON`・`NUL` 等。05 §4.5 の `ReservedName`) | OK |
| `msg.recovery.found.hint` | ファイルを開いて内容を確かめ、要らなくなったらそのフォルダを削除してください | 緊急退避 (`kind = Emergency`。代替の退避先のものを含む) が 1 件以上ある (起動時に 1 回。退避の中身は読まない。`{dir}` は最も新しい緊急退避のディレクトリ自身、`{count}` は退避の全件数。選び方は 02 §9.3 の `RecoveryScan` の行) | OK |
| `msg.recovery.found.text` | 異常終了の時に保存されていなかった内容を退避しています: {dir} (退避は全部で {count} 件) | 緊急退避 (`kind = Emergency`。代替の退避先のものを含む) が 1 件以上ある (起動時に 1 回。退避の中身は読まない。`{dir}` は最も新しい緊急退避のディレクトリ自身、`{count}` は退避の全件数。選び方は 02 §9.3 の `RecoveryScan` の行) | OK |
| `msg.recovery.found_rescue.hint` | 要らなくなったらそのフォルダを削除してください | 緊急退避は無く、「退避先に保存」(03 §9.7 (e)) の退避 (`kind = Rescue`) だけがある (起動時に 1 回。`{dir}` は最も新しい「退避先に保存」のディレクトリ自身、`{count}` は全件数) | OK |
| `msg.recovery.found_rescue.text` | 「退避先に保存」で書いた内容が残っています: {dir} (退避は全部で {count} 件) | 緊急退避は無く、「退避先に保存」(03 §9.7 (e)) の退避 (`kind = Rescue`) だけがある (起動時に 1 回。`{dir}` は最も新しい「退避先に保存」のディレクトリ自身、`{count}` は全件数) | OK |
| `msg.recovery.unknown_format.hint` | 新しい版の evoeditor で開いてください | 復旧 UI が読んだ manifest の形式が新しい版のもの (ダウングレード、05 §18.11)。v0.1 の起動時の案内は manifest を読まないので出さない (上の 2 つで場所だけを示す) | OK |
| `msg.recovery.unknown_format.text` | {dir} の退避は新しい版の形式なので読みません (移動も削除もしていません) | 復旧 UI が読んだ manifest の形式が新しい版のもの (ダウングレード、05 §18.11)。v0.1 の起動時の案内は manifest を読まないので出さない (上の 2 つで場所だけを示す) | OK |
| `msg.reload.missing.hint` | 今の内容を残すには、上書き保存 ({key:file.save}) でファイルを作り直すか、名前を付けて保存 ({key:file.save_as}) してください | `file.reload` で、ファイルが無かった (開いた後に削除・移動された。文書は変わらない) | OK |
| `msg.reload.missing.text` | {path} が見つからないため読み込み直せません | `file.reload` で、ファイルが無かった (開いた後に削除・移動された。文書は変わらない) | OK |
| `msg.save.acl_not_preserved.hint` | 必要なら保存後に設定し直してください | 警告 (v0.1 は ACL・拡張属性を複製しない。付いていることを検出したとき) | OK |
| `msg.save.acl_not_preserved.text` | {path} の ACL・拡張属性は引き継がれません | 警告 (v0.1 は ACL・拡張属性を複製しない。付いていることを検出したとき) | OK |
| `msg.save.attributes_not_copied.hint` | 必要なら OS で設定し直してください | 警告 (権限・属性の複製に失敗。保存自体は成功) | OK |
| `msg.save.attributes_not_copied.text` | {path} の権限・属性の一部を引き継げませんでした: {reason} | 警告 (権限・属性の複製に失敗。保存自体は成功) | OK |
| `msg.save.backup_failed.hint` | 続く確認で選んでください | BackupFailed (in-place の事前バックアップに失敗。何も書いていない) | OK |
| `msg.save.backup_failed.text` | 直接上書きの前のバックアップを作れませんでした。何も書き込んでいません | BackupFailed (in-place の事前バックアップに失敗。何も書いていない) | OK |
| `msg.save.busy.hint` | そのプログラムを閉じて {key:file.save} で再試行するか、別名で保存してください | Busy (Windows の共有違反。再試行の後) | OK |
| `msg.save.busy.text` | 他のプログラムが {path} を使用中のため保存できませんでした | Busy (Windows の共有違反。再試行の後) | OK |
| `msg.save.changed_after_save.hint` | 次に保存するときに内容を確認します。必要なら再読込 ({key:file.reload}) で確かめてください | 警告 (`SaveWarning`: 置換の後の T が自分の書いたものと確認できず、保存の直後に他のプログラムが変更した可能性がある。05 §4.1 手順 7。保存自体は成功のまま。次の保存は外部変更として確認される) | OK |
| `msg.save.changed_after_save.text` | {path} を保存しましたが、保存の直後に他のプログラムが変更した可能性があります | 警告 (`SaveWarning`: 置換の後の T が自分の書いたものと確認できず、保存の直後に他のプログラムが変更した可能性がある。05 §4.1 手順 7。保存自体は成功のまま。次の保存は外部変更として確認される) | OK |
| `msg.save.check_failed.hint` | 何も書き込んでいません。{key:file.save} でもう一度保存してください | 未編集の上書き保存の照合 (`CheckingUnchanged` の metadata、03 §9.3 手順 0 / §9.7) が `Busy`・`PermissionDenied`・その他で失敗した (`NotFound` は不一致として保存へ進むので出さない)。保存は中止し (後続も捨てる)、確認の選択肢は出さない。`NoWritePermission` は付けない。`{reason}` は失敗の種類の文言 | OK |
| `msg.save.check_failed.text` | {path} の状態を確認できなかったため、保存を中止しました: {reason} | 未編集の上書き保存の照合 (`CheckingUnchanged` の metadata、03 §9.3 手順 0 / §9.7) が `Busy`・`PermissionDenied`・その他で失敗した (`NotFound` は不一致として保存へ進むので出さない)。保存は中止し (後続も捨てる)、確認の選択肢は出さない。`NoWritePermission` は付けない。`{reason}` は失敗の種類の文言 | OK |
| `msg.save.dangling_link.hint` | 別名で保存 ({key:file.save_as}) するか、リンク先のフォルダを作ってから保存してください | シンボリックリンクのリンク先が無い (05 §4.1。v0.1 は作成せずエラーにする) | OK |
| `msg.save.dangling_link.text` | {path} はリンクで、リンク先 {target} がありません。保存していません | シンボリックリンクのリンク先が無い (05 §4.1。v0.1 は作成せずエラーにする) | OK |
| `msg.save.disk_full.hint` | 空きを作って {key:file.save} で再試行するか、別の場所へ保存 ({key:file.save_as}) してください | DiskFull (原子的保存・in-place とも書く前に検出) | OK |
| `msg.save.disk_full.text` | ディスクの空きが足りないため {path} を保存できませんでした。ファイルは変更していません | DiskFull (原子的保存・in-place とも書く前に検出) | OK |
| `msg.save.encoding_unsupported_iso2022.hint` | 文字コードを指定して開き直す ({key:file.reopen_with_encoding}) で別の文字コード (UTF-8 等) として開き直してください | 不正なバイトを含む文書 (§8.3 のエスケープ文字を含む。主に `StatefulEncodingWithInvalidBytes`) を ISO-2022-JP で保存しようとした (上書き保存・名前を付けて保存・文字コードを指定して保存。03 §8.3 の (a) / §9.7 の「連鎖の開始時の拒否」。確認ではなくエラーのメッセージで、選択肢は無い。保存の連鎖と後続 (終了等) は中止。ジョブは投入しない) | OK |
| `msg.save.encoding_unsupported_iso2022.text` | 不正なバイトを含むため、この文書は ISO-2022-JP では保存できません。何も書き込んでいません | 不正なバイトを含む文書 (§8.3 のエスケープ文字を含む。主に `StatefulEncodingWithInvalidBytes`) を ISO-2022-JP で保存しようとした (上書き保存・名前を付けて保存・文字コードを指定して保存。03 §8.3 の (a) / §9.7 の「連鎖の開始時の拒否」。確認ではなくエラーのメッセージで、選択肢は無い。保存の連鎖と後続 (終了等) は中止。ジョブは投入しない) | OK |
| `msg.save.failed.hint` | 別名で保存 ({key:file.save_as}) するか、権限を確認してください | 分類できない失敗 | OK |
| `msg.save.failed.text` | {path} を保存できませんでした: {reason} | 分類できない失敗 | OK |
| `msg.save.in_place_failed.hint` | 別名で保存 ({key:file.save_as}) してから、バックアップと見比べてください | InPlaceFailed (書込の途中で失敗。`backup: Some`) | OK |
| `msg.save.in_place_failed.text` | {path} の直接上書きの途中で失敗しました。ファイルが壊れている可能性があります。書き込む前の内容: {backup} | InPlaceFailed (書込の途中で失敗。`backup: Some`) | OK |
| `msg.save.in_place_failed_no_backup.hint` | 別名で保存 ({key:file.save_as}) して、今の内容を残してください | InPlaceFailed (`backup: None`。「バックアップなしで上書き」を承認した後の失敗) | OK |
| `msg.save.in_place_failed_no_backup.text` | {path} の直接上書きの途中で失敗しました。ファイルが壊れている可能性があり、バックアップはありません | InPlaceFailed (`backup: None`。「バックアップなしで上書き」を承認した後の失敗) | OK |
| `msg.save.invalid_path.hint` | 別のファイル名で保存してください | InvalidPath (循環リンクを含む) | OK |
| `msg.save.invalid_path.text` | {path} には保存できません: {reason} | InvalidPath (循環リンクを含む) | OK |
| `msg.save.moved_to_backup.hint` | 続く確認で別名で保存し、元のファイルは {backup} の名前を元に戻してください | ReplacedMovedToBackup (Windows の置換途中の失敗) | OK |
| `msg.save.moved_to_backup.text` | 保存の途中で失敗し、元のファイルは {backup} に移っています | ReplacedMovedToBackup (Windows の置換途中の失敗) | OK |
| `msg.save.parent_missing.hint` | フォルダを作ってから {key:file.save} で再試行するか、別名で保存 ({key:file.save_as}) してください | 保存先のフォルダが無い (`NotFound` + `FsErrorDetail::ParentMissing { dir }`、02 §4.5 / 05 §4.1。存在しないパスを開いた新規ファイル。01 FR-101 AC-2。フォルダは作らず何も書かない) | OK |
| `msg.save.parent_missing.text` | {dir} が無いため {path} を保存できません | 保存先のフォルダが無い (`NotFound` + `FsErrorDetail::ParentMissing { dir }`、02 §4.5 / 05 §4.1。存在しないパスを開いた新規ファイル。01 FR-101 AC-2。フォルダは作らず何も書かない) | OK |
| `msg.save.permission_denied.hint` | 別名で保存するか、退避先に保存してください | PermissionDenied | OK |
| `msg.save.permission_denied.text` | {path} に書き込む権限がありません | PermissionDenied | OK |
| `msg.save.read_only_fs.hint` | 別名で保存 ({key:file.save_as}) してください | ReadOnlyFs | OK |
| `msg.save.read_only_fs.text` | {path} は読み取り専用の場所にあります | ReadOnlyFs | OK |
| `msg.save.recovery_failed.hint` | 別名で保存 ({key:file.save_as}) してください | 上の確認で「退避先に保存」を選んだが書けなかった (`state_dir` も代替の退避先 (05 §7.5) も使えない、容量不足等。03 §9.7 の `WritingRecovery`。文書は dirty のまま、後続は中止) | OK |
| `msg.save.recovery_failed.text` | 退避先に保存できませんでした: {reason}。元のファイル {path} は変更していません | 上の確認で「退避先に保存」を選んだが書けなかった (`state_dir` も代替の退避先 (05 §7.5) も使えない、容量不足等。03 §9.7 の `WritingRecovery`。文書は dirty のまま、後続は中止) | OK |
| `msg.save_as.not_a_file.hint` | 別のファイル名を入力してください | 名前を付けて保存の保存先がフォルダ・デバイス等 (03 §9.7 (a) の 4。パスの不正は `msg.save.invalid_path`、metadata の失敗 ((a) の 7) は `msg.save.failed` / `msg.save.permission_denied` / `msg.save.busy`) | OK |
| `msg.save_as.not_a_file.text` | {path} はファイルではありません (フォルダ等) | 名前を付けて保存の保存先がフォルダ・デバイス等 (03 §9.7 (a) の 4。パスの不正は `msg.save.invalid_path`、metadata の失敗 ((a) の 7) は `msg.save.failed` / `msg.save.permission_denied` / `msg.save.busy`) | OK |
| `msg.save_as.same_file_read_only.hint` | 別のファイル名を入力してください | 名前を付けて保存で、読み取り専用の文書と同じファイルを選んだ (03 §9.7 (a) の 1。名前の入力プロンプトに戻る) | OK |
| `msg.save_as.same_file_read_only.text` | 読み取り専用の文書は、同じファイル {path} には保存できません | 名前を付けて保存で、読み取り専用の文書と同じファイルを選んだ (03 §9.7 (a) の 1。名前の入力プロンプトに戻る) | OK |
| `msg.save_as.target_read_only.hint` | 別のファイル名を入力してください | 名前を付けて保存の保存先が読み取り専用属性のファイル (03 §9.7 (a) の 5。上書きの確認の前に拒否し、名前の入力プロンプトに戻る) | OK |
| `msg.save_as.target_read_only.text` | {path} は読み取り専用属性のファイルなので上書きできません | 名前を付けて保存の保存先が読み取り専用属性のファイル (03 §9.7 (a) の 5。上書きの確認の前に拒否し、名前の入力プロンプトに戻る) | OK |
| `msg.search.replace_stale.hint` | 入力を止めてから、もう一度実行してください | すべて置換の準備中に文書が編集され続け、作り直しても間に合わなかった (03 §12.5 の 4。文書は変わらない) | OK |
| `msg.search.replace_stale.text` | 編集が続いたため、すべて置換を中止しました | すべて置換の準備中に文書が編集され続け、作り直しても間に合わなかった (03 §12.5 の 4。文書は変わらない) | OK |

## メッセージ: 通知

32 個

| キー | 文言 | 出す場面 | 判定 |
|---|---|---|---|
| `info.app.release_summary` | (版ごとに書く) | 上の `{summary}` に入れる、その版の要点の 1 文 (リリースのたびに同梱する全言語を同時に書き換える。メッセージ行の幅を超える部分は省略する) | OK |
| `info.app.updated` | evoeditor {version} に更新しました: {summary} | 前回起動した版 (`last_version.toml`) より新しい版で起動した (最初の Frame の後に 1 回) | OK |
| `info.cli.unsupported` | {option} はこのバージョンでは使えないため無視しました | 後の版で使う予約のオプション・`+LINE` を渡した (07 §18.3) | OK |
| `info.config.unsupported` | {item} はこのバージョンでは未対応のため無視しました | 公開前のキー・予約のセクション・後の版の値 (07 §5 冒頭の「このバージョンでは未対応」)。v0.1 は画面に出さず、件数にも数えず、ログにだけ書く (07 §4.3)。`config.show_diagnostics` (v0.2) の一覧で使う | OK |
| `info.eol.converted` | {count} 個の改行を {eol} に変換しました (元に戻すで戻せます) | 改行コードを変換した (01 FR-113 AC-4、03 §8.4。`{eol}` は `LF` / `CRLF` / `CR` で翻訳しない) | OK |
| `info.eol.unchanged` | 改行はすでに {eol} です | 変換する改行が無かった (すべて `{eol}` か、改行が無い。新しく挿入する改行だけを `{eol}` にした) | OK |
| `info.goto_line.clamped` | 行 {line} はありません。最後の行 ({last}) へ移動しました | 行へ移動で範囲外の番号を入れた (01 FR-306 AC-1) | OK |
| `info.open.already_open` | {path} は既に開いています | 開こうとしたファイルが今の文書と同じ (03 §9.2 手順 0 / 2b) | OK |
| `info.open.cancelled` | {path} の読込を取り消しました | 読込を取り消した (`Esc`。01 FR-122 AC-1。今の文書はそのまま) | OK |
| `info.open.guess_unsupported` | {path} の文字コードは {guess} と推定されましたが、この版では扱えないため {encoding} として開きました / 正しく見えないときは、文字コードを指定して開き直す ({key:file.reopen_with_encoding}) で選び直してください | 推定結果が v0.1 の 7 種に無い (03 §8.2 手順 7 の (iii)。windows-1252・GBK 等)。`{guess}` は chardetng の推定名 (encoding_rs の名前)、`{encoding}` は代わりに開いた `file.default_encoding` | OK |
| `info.open.long_line` | {path} には非常に長い行があるため、読み取り専用で開きました (長い行は先頭の一部だけを表示します) | 閾値 (1 MiB) を超える行を含む文書を開いた (NFR-106。v0.1 は読み取り専用で開き、その行は先頭 64 KiB だけを表示) | OK |
| `info.open.lossy_roundtrip` | この文字コードでは、編集した箇所の一部の文字が保存時に別のバイト列になることがあります ({count} 箇所) | 区分 B の情報 (03 §8.7) | OK |
| `info.open.new_file` | {path} は無いため、新しいファイルとして開きました (保存すると作成します) | 存在しないパスを開いた (新規ファイル。01 FR-101 AC-2) | OK |
| `info.open.redetected` | 文字コードを {encoding} と判定し直しました | 文字コードの再判定 (03 §8.2.1) | OK |
| `info.paste.first_line_only` | 入力欄には最初の行だけを貼り付けました | 入力欄への複数行の貼り付け (§6.6) | OK |
| `info.recovery.old` | 30 日より古い退避が {count} 件あります: {dir} | 30 日を超えた退避がある | OK |
| `info.reload.busy` | {path} は読み込み中です。読込が終わってから実行してください | 読込中の文書で `file.reload` を実行した (何もしない) | OK |
| `info.reload.done` | {path} を読み込み直しました (元に戻すで戻せます) | 再読込した (`file.reload` (v0.1) とフォーカス復帰時の確認 (v0.2)。Origin::Reload で Undo できる、03 §9.5)。再読込の後に読み取り専用でない文書のとき | OK |
| `info.reload.done_read_only` | {path} を読み込み直しました (読み取り専用のため元に戻せません) | 再読込した文書が、再読込の後に読み取り専用である (`edit.undo` の when が偽で Undo できない。03 §7.2 / §9.5、提案、OQ-216) | OK |
| `info.reopen.history_cleared` | {encoding} で開き直しました。元に戻す履歴は消えました | 文字コードを指定して開き直した (FR-111。Undo できない、03 §8.8) | OK |
| `info.save.aborted` | 保存を中止しました ({path} は書き換えていません) | 保存の連鎖の中止 (03 §9.7 の「中止」: 名前の入力・確認のキャンセル、保存の待ちの取消し (`JobError::Cancelled`)。後続の終了・置き換え・開き直しも行わない) | OK |
| `info.save.done` | {path} を保存しました | 保存の成功 | OK |
| `info.save.in_place` | {path} を直接上書きで保存しました | 直接上書きで保存した (承認後・一時ファイル規則・`file.save_strategy = "in_place"`。`SaveReport.method == InPlace`)。保存方式の表示はこのメッセージだけで、ステータスバーの項目にはしない。事前バックアップは成功後に消すので (05 §4.4 手順 9) 場所を示さない | OK |
| `info.save.in_progress` | 保存中です | 保存の連鎖の途中 (照合・保存ジョブの待ち、確認・名前の入力の表示中) に、同じ文書の保存要求 (`file.save` / `file.save_as` / `file.set_encoding`) を無視した (03 §9.7 の全状態の遷移表の「無視」。単独の保存の `Saving` への `file.save` はキューに積むので出さない)。ステータスバーの保存中の表示 (`status.saving.*`、§8.7) とは別に 1 回示す | OK |
| `info.save.to_recovery` | 退避先 {target} に保存しました。元のファイル {path} は変更していません | 上の確認で「退避先に保存」を選び、書けた (05 §7.5 の `recovery/rescue/`) | OK |
| `info.save.unchanged` | 変更はありません (ファイルは書き換えていません) | 手順 0 (無編集。書き込まない) | OK |
| `info.search.replace_empty_query` | 検索する文字列を入力してください | 検索語が空のまますべて置換を実行した (何もしない) | OK |
| `info.search.replace_no_match` | 置換する箇所はありません | すべて置換で一致が無かった (文書は変わらない) | OK |
| `info.search.replaced` | {count} 件を置換しました (元に戻すで戻せます) | すべて置換した (01 FR-403 AC-2、03 §12.5) | OK |
| `info.search.wrapped_backward` | 文書の先頭に達したので、末尾から検索しました | `search.prev` が文書先頭を越えて末尾から続けた (同上) | OK |
| `info.search.wrapped_forward` | 文書の末尾に達したので、先頭から検索しました | `search.next` が文書末尾を越えて先頭から続けた (03 §12.2 の `search.next` / `search.prev` の行。折り返した旨を示す) | OK |
| `info.undo.history_trimmed` | メモリの上限のため、古い元に戻す履歴を破棄しました | Undo 履歴の予算による破棄 (03 §7.5) | OK |

## ステータスバー

14 個

| キー | 文言 | 出す場面 | 判定 |
|---|---|---|---|
| `status.degraded.long_line.long` | [長い行: 一部表示] |  | OK |
| `status.degraded.long_line.short` | 一部 |  | OK |
| `status.encoding.bom` | {encoding} BOM |  | OK |
| `status.eol_mixed` | 混在 |  | OK |
| `status.loading.long` | [読込中 {percent}%] |  | OK |
| `status.loading.short` | {percent}% |  | OK |
| `status.position.long` | 行 {line}, 桁 {col} |  | OK |
| `status.position.short` | {line}:{col} |  | OK |
| `status.read_only.long` | [RO] |  | OK |
| `status.read_only.short` | RO |  | OK |
| `status.saving.long` | [保存中] |  | OK |
| `status.saving.short` | 保存中 |  | OK |
| `status.selection` | ({count} 文字選択) |  | OK |
| `status.untitled` | 無題-{n} |  | OK |

