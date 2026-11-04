# 互換性の記録 — Linux (WSL2 の Windows Terminal と tmux) (v0.2.0、M7 の受入)

手順は [`../checklist.md`](../checklist.md) の A〜G と L。基準機の固有情報 (機種名・型番・OS のビルド番号・ウイルス対策ソフトの製品名・ユーザー名を含むパス) は書かない。

この記録は **M7 の受入** (v0.2 の開発中、docs/README §3.9) のもの。v0.2.0 のリリース候補ができたら、その tar.gz で `cargo xtask accept-linux --archive <tar.gz>` と手の項目をやり直して、この表を更新する。

- 確認した人 / 日付: 自動の部分は `cargo xtask accept-linux` (2026-11-03)。手の部分は PO (2026-11-03)
- 判定したコミットと、使った tar.gz の SHA-256 (先頭 12 桁): `1a3cd49` (develop) / `2cc9a76b367e` (`cargo xtask dist --target x86_64-unknown-linux-musl` で作った候補。署名なし。版の表示は workspace の版のまま `0.1.0`)。L7 / L8 の見た目は、#311 の修正を入れたビルドで確かめ直した
- Windows / WSL: Windows 11 / WSL 2 (カーネル 6.18 系)
- ディストリビューション: Ubuntu 26.04 LTS
- 端末: Windows Terminal (自動の部分は tmux の擬似端末)
- tmux: 3.6
- 日本語入力: Microsoft IME と Google 日本語入力の両方

○ = 期待どおり、△ = 動くが気になる点あり、× = 期待どおりでない。「自動 ○」は accept-linux で ○ (IME の 2 列は同じ結果)。手の項目は PO の報告 (「下に書いたもの以外はすべて OK」) による。行ごとの細目は個別には書き取っていない。

- accept-linux の結果 (`target/accept-linux/result.md`) の先頭のコミットと SHA-256: `ff61a068a40d` / `2cc9a76b367e` (`1a3cd49` に、accept-linux の G1 の判定の修正だけを当てた作業中のコミット。squash の後は残らない。tar.gz は `1a3cd49` で作ったものと同じバイト列)

| 項目 | Microsoft IME | Google 日本語入力 | 備考 |
|---|---|---|---|
| A1〜A5 起動と終了 | ○ | ○ | 自動 ○ (tmux) と、Windows Terminal で開いた見た目 (手) |
| B1〜B5 日本語入力 (L6) | ○ | ○ | 手 |
| C1 リサイズ | ○ | ○ | 自動 ○ (tmux の resize-window。120×40〜1×1) と、Windows Terminal のウィンドウの大きさの変更 (手) |
| C2 Ctrl+Shift の区別 | △ | △ | **Windows Terminal は WSL のアプリへ `Ctrl+Shift+Z` を `Ctrl+Z` と同じ 0x1A で送る** (modifyOtherKeys・kitty の要求の後も。`od` で確認)。evoeditor の側では区別できず、tmux の中でも同じ。やり直しは `Ctrl+Y` で効く (手)。既知の制約として README と 05 §9.4 に書いた。tmux が区別できる形で送ったときに区別することは自動 ○ (#308) |
| C3 端末が横取りするキー | ○ | ○ | 手 (`Ctrl+Shift+P` は Windows Terminal のパレット。`F1` / `Ctrl+P` で evoeditor のパレット) |
| C4・C5 色の区別と段階 | ○ | ○ | C5 は自動 ○ (SGR を解析)、C4 の見た目は手 |
| D1〜D6 貼り付けとクリップボード | ○ | ○ | D1・D3・D4・D6 は自動 ○、D2 (右クリック)・D5 (`Shift` + ドラッグの範囲選択) は手 |
| E1〜E3 検索と置換 | ○ | ○ | 自動 ○。最初の Enter は現在の一致を確定し、2 回目から次へ (03 §12.2 のとおり) |
| F1〜F6 ファイルの読み書き | ○ | ○ | 自動 ○ |
| G1〜G3 巨大ファイル | ○ | ○ | 自動 ○ (100 MB の取り消し、512 MiB 超の警告、保存中の入力) |
| L1 README の Linux の導入手順 | ○ | ○ | 自動 ○ (作業用の HOME)。手でも同じ手順で入れた (`~/.local/bin` を作った後はログインし直す、README のとおり)。署名は候補に無いので未検証 |
| L2 タブを閉じたときの退避 (SIGHUP) | ○ | ○ | 自動 ○ (tmux の kill-session) と、Windows Terminal の WSL のタブを閉じる操作 (手) |
| L3 サスペンドと再開 | ○ | ○ | 自動 ○ (Ctrl+K Z → 大きさを変えて fg) |
| L4 Windows のクリップボードとの往復 | ○ | ○ | 自動 ○ (powershell.exe。日本語・絵文字・サロゲートペア・改行) |
| L5 端末の貼り付け (ブラケットペースト) | ○ | ○ | 自動 ○ (tmux の paste-buffer -p) と、Windows Terminal の `Ctrl+Shift+V`・右クリック (手) |
| L7 tmux の中 (推奨設定あり) と kill-session | ○ | ○ | 自動 ○。**ダイアログを閉じた跡に `▮` が残った** (tmux は全角の先頭セルの上書きで後続セルも消えたとみなし、Windows Terminal は後続セルの色を残す) → #311 で直し、Windows Terminal で消えたことを確かめた |
| L8 tmux の推奨設定なし | ○ | ○ | 自動 ○。`▮` は L7 と同じく #311 で解消。「破棄して終了の後、次のキーまで終了しない」が 1 回見えたが、`script` で記録した再実行では `Enter` の 2 ms 後に終了し、再現しなかった |

## 判定

- 合格 / 不合格: **合格** (C2 の △ は Windows Terminal の入力の制約で、既知の制約として書いた。代替の `Ctrl+Y` で全機能に届く)
- 受入で見つけて直したもの: tmux の中で Ctrl+Shift のキーが区別されない (#308)、ダイアログを閉じた跡の `▮` (#311)。受入の自動化は #307 (accept-linux)
- 未確認のまま出荷するもの (05 §9.4): 実機のデスクトップ環境 (GNOME / KDE の Wayland・X11) のクリップボードと IME の候補窓、`TERM=linux` のコンソール

## 自動の部分の結果 (`cargo xtask accept-linux`、2026-11-03)

| # | 項目 | 結果 | 備考 |
|---|---|---|---|
| L1 | L1 README の Linux の導入手順 | ○ | ハッシュ OK、ログインシェルの PATH で `evoeditor 0.1.0`。SHA256SUMS.minisig が無いので署名は未検証 (手元で作った候補) |
| A1 | A1・A2 起動と終了 (端末が元に戻る) | ○ | 無題-1 とヒントバー。Ctrl+Q で 0、代替スクリーンを抜けてカーソルを戻す |
| A3 | A3 --version と --help | ○ | `evoeditor 0.1.0`。--help は終了コード 0 |
| A4 | A4 存在しないパスを新規として保存 | ○ | 新規として保存できる |
| A5 | A5 日本語のファイル名 | ○ | 本文とステータスバーの名前が崩れない |
| C1 | C1 リサイズ (大・小・極小) | ○ | 120×40・30×8・2×1・1×1・80×24 で描き直し、落ちない |
| C2 | C2 Ctrl+Z と Ctrl+Shift+Z の区別 (tmux の中) | ○ | Ctrl+Z で元に戻し、Ctrl+Shift+Z でやり直す |
| C5 | C5 色の段階 (truecolor / 256 / 16 / none / NO_COLOR) | ○ | 5 段階で選択・一致・現在の一致・ステータスバーが本文と区別され、色の指定が段階に収まる (見た目は C4 で目視) |
| D1 | D1・L5 ブラケットペーストの 100 行と 1 回の元に戻す | ○ | インデントのまま 100 行が入り、Ctrl+Z 1 回で消える |
| D3 | D3・L5 貼り付けの制御文字 (paste-injection.txt) | ○ | 保存も終了も起きず、全文が入る (制御文字は置き換えられて入る) |
| D4 | D4・L4 Windows のクリップボードとの往復 (WSL) | ○ | 日本語・絵文字・サロゲートペア・改行が往復で一致 (改行は LF に揃えて比較) |
| D6 | D6 数 MB の貼り付け | ○ | 3.8 MiB を貼り付けて、後に打った文字が 0.1 秒で出る |
| E1 | E1・E3 検索 (次・前・折り返し・大小・日本語) | ○ | 最初の Enter で現在の一致を確定し、次 (Enter)・前 (↑)・末尾で折り返し・Alt+C・日本語。Shift+Enter は C2 と同じく tmux の拡張キーに依存するので見ない |
| E2 | E2 すべて置換と 1 回の元に戻す | ○ | 3 件を置換し、Ctrl+Z 1 回で元に戻る |
| F1 | F1 UTF-8 (CRLF) の保存 | ○ | CRLF のまま保存される |
| F2 | F2 Shift_JIS の表示と保存 | ○ | Shift_JIS と表示し、Shift_JIS のまま保存する |
| F3 | F3 読み取り専用のファイル | ○ | 編集を理由つきで拒否する |
| F4 | F4 外部で変更されたファイルの保存 | ○ | 保存の前に確認が出て、Esc で書き換えない |
| F5 | F5 未保存で終了の確認とキャンセル | ○ | 保存 / 破棄 / キャンセルの確認が出て、Esc で何も変わらない |
| F6 | F6 名前を付けて保存の上書きの確認 | ○ | 既存の名前に上書きの確認が出る |
| G1 | G1 100 MB の進捗と取り消し | ○ | [読込中 n%] が出て、Esc から 110 ms で取り消す |
| G2 | G2 512 MiB 超の警告 | ○ | 開く前に警告が出て、読み取り専用で開くかキャンセルを選べる |
| G3 | G3 100 MB の保存中の入力 | ○ | 保存 (0.5 秒) の途中に打った文字が 9 ms で出る |
| L2 | L2・L7 SIGHUP (kill-session) の退避と起動時の案内 | ○ | kill-session で recovery/<日時>-<pid>/ に退避し、次の起動で案内する (Windows Terminal のタブを閉じる操作は手で) |
| L3 | L3 Ctrl+K Z のサスペンドと fg (大きさの変更) | ○ | 端末を戻してシェルへ。fg で代替スクリーンに戻り、100×30 で描き直す |
| L8 | L8 tmux の推奨設定なしの Esc と Ctrl+Shift | ○ | Esc は 6 ms で効く (tmux の既定の escape-time)。Ctrl+Shift+Z はCtrl+Z と区別されない (C2 を見る) |
