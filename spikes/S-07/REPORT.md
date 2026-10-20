# S-07 REPORT (Windows の入力経路)

状態: **判定済み** (2026-10-21)。未確認: Google 日本語入力・サロゲートペア・AltGr (欧州配列)・conhost の Ctrl ショートカット・WT の貼り付け確認ダイアログ・WT_SESSION の誤判定・conhost の writew

## 環境

| 項目 | Windows Terminal | conhost |
|---|---|---|
| OS / 版 | | |
| 端末の版 | | |
| IME (MS-IME / Google 日本語入力) | | |
| キーボード配列 | | |

## (a) バースト検出: N と T

貼り付けの届き方 (`probe`。N・T の判定なし。テキスト = `gen-paste` の複数行・タブ・0x08・ESC・日本語):

| 端末 | 貼り付け | 文字を生成する KeyDown | 記録 | バッチ | 所要 |
|---|---|---|---|---|---|
| Windows Terminal | 1 KB | 508 | 1,157 | 1,156 件の 1 バッチ + 1 | 20〜26 ms |
| Windows Terminal | 100 KB | 52,008 | 117,637 | 1 バッチ | 60〜74 ms |
| Windows Terminal | 10 MB | 5,459,068 | 約 1,228 万 | 数バッチ (最大 1,078 万件) | **約 5.5〜5.9 秒** |
| conhost | 36 文字 | 36 | 72 | 1 バッチ | 0.2 ms |
| conhost | 1 KB | 508 | 1,324 | 1 バッチ | 0.4 ms |
| conhost | 100 KB | 52,008 | 133,980 | 1 バッチ | 6.8 ms |
| conhost | 10 MB | 5,459,068 | 13,907,012 | **1 バッチ** | **708 ms** |

- 貼り付けは **どちらの端末でも、待ちなしで読める分を読み切る方式で 1 つのバッチ (またはほぼ 1 つ) になる**。通常の 1 文字ずつの入力は 1 バッチ 1〜2 件。**N = 16 は 36 文字の貼り付けで超え、通常の打鍵とは明確に区別できる**。
- Windows Terminal の 10 MB は、端末側の書込が遅く (約 220 万記録/秒)、1 回の `ReadConsoleInputW` の連続読取が約 5 秒続く。読取ループは「件数が 0 になるまで続ける」ので 1 つのバーストになり、バッチ間の間隔は 8.4 ms 以下 (T = 16 ms 未満)。conhost は一括で届く。
- **読取は 1 回あたりの件数に上限を付ける必要がある**: 10 MB で 1 回の `GetNumberOfConsoleInputEvents` が 1,078 万〜1,391 万件 (`INPUT_RECORD` 20 B で約 215〜280 MB) を返した。そのまま 1 本のバッファに読むとメモリが膨らむ。実装は 1 回 4,096〜8,192 件ずつ読み、本文の String へ直接連結する。
- **NFR-111 (10 MB の貼り付け 1 秒、Should・記録)**: 入力経路だけで Windows Terminal は約 5.5 秒、conhost は約 0.7 秒。Windows Terminal では入力の転送が支配的で、アプリ側では縮められない。参考値として記録する。
- 暫定値 **N = 16、T = 16 ms は据え置き**。15 文字以下の貼り付けは通常入力に混ざる既知の制約 (04 §6.6)。

制御文字の届き方 (UnicodeChar と VK):

| 端末 | Tab | 改行 | 0x08 (BS) | ESC |
|---|---|---|---|---|
| Windows Terminal | `U+0009 vk=0x09` | `U+000D vk=0x0D` (CRLF は CR 1 つ) | **届かない** (端末が除去) | **届かない** (端末が除去) |
| conhost | `U+0009 vk=0x09` | `U+000D vk=0x0D` | `U+0008 vk=0x08` | `U+001B vk=0x1B` |

- conhost では 0x08・ESC が **`VK_BACK` / `VK_ESCAPE` のキー** として届く。仮想キーコード優先の写像だと本文に入らず、削除や Esc に化ける → 04 §6.6 の「バースト中は UnicodeChar を本文にする」規則が必須であることを確認。
- Windows Terminal は貼り付けから ESC・BS を除くので、ESC を含む本文による入力の偽装は端末側で防がれる (多層防御として core 側の規則も維持)。
- conhost では `Ctrl+V` のキー (`U+0016 vk=0x56`) 自体もアプリに届く (「Ctrl キーショートカットを有効にする」が OFF のとき)。`Ctrl+V` のキーを貼り付けコマンドに割り当てると、貼り付け本文と二重にならないか注意。

## 自動操作 (`spikes/auto/run-s07.ps1`)

`SendInput` で Windows Terminal・conhost の新しいウィンドウにキーを送り、`events` のログを要約した (どちらの端末も同じ結果。差は Ctrl+V と Alt+A の KeyUp の文字のみ)。IME は OFF にして測った (IMM の `IMC_SETOPENSTATUS`)。D = KeyDown、U = KeyUp、数字は仮想キー (16 進)、`/U+XXXX` は `UnicodeChar`。

| 操作 | 届いたイベント |
|---|---|
| `a` | D41/U+0061 U41/U+0061 |
| `Ctrl+H` | D11[LCTRL] D48/U+0008[LCTRL] U48/U+0008 U11 |
| `Backspace` | D08/U+0008 U08/U+0008 |
| `Ctrl+I` / `Tab` | D49/U+0009[LCTRL] … / D09/U+0009 … |
| `Ctrl+M` / `Enter` | D4D/U+000D[LCTRL] … / D0D/U+000D … |
| `Ctrl+[` / `Esc` | DDB/U+001B[LCTRL] … / D1B/U+001B … |
| `Ctrl+J` | D4A/U+000A[LCTRL] … |
| `Ctrl+Shift+S` / `Z` / `B` | D10[LCTRL\|SHIFT] D53/U+0013 … (S/Z/B とも KeyDown が届く) |
| 最初の `Ctrl+Shift+A` | D10 の後 **KeyDown が欠け**、U41/U+0001 だけが届いた (両端末。再現条件不明) |
| `Alt+A` | D12[LALT] D41/U+0061[LALT] U41 … |
| `Ctrl+Alt+A` | D11 D12 D41/(UnicodeChar なし) … (日本語配列に AltGr は無い) |
| `Shift+A` | D10[SHIFT] D41/U+0041 … |
| `Ctrl+V` | conhost: D56/U+0016[LCTRL] が届く。Windows Terminal: KeyDown は届かず、貼り付け本文が個別のキーで届く |
| キーリピート (`x` を 6 回 DOWN) | D58/U+0078 が 6 回 (すべて `wRepeatCount` = 1、KeyUp なし) の後 U58 |
| 半角/全角 (`VK_KANJI`) | **何も届かない** (KeyDown・KeyUp とも IME が消費) |
| 変換 / 無変換 | **KeyUp だけ** (U1C / U1D) |
| `Ctrl+Break` | `CTRL_EVENT=BREAK` が届き、U03/U+0003[LCTRL] だけ (KeyDown なし)。ループは詰まらない |
| フォーカスの移動 | FOCUS out / FOCUS in (ループは詰まらない) |

IME の確定 (ひらがな、conhost): 確定した文字は `vk=0x00`・`scan=0x00`・`UnicodeChar` = その文字 の KeyDown と KeyUp の対で届く (例 `わ` `た` `し`、全角英数 `ｎ`)。確定の `Enter` は KeyUp (`vk=0x0D`) だけが届いた。Windows Terminal で IME が ON のときも同じ形 (`あ`、`ア`、`ａ` が `vk=0x00` で届いた)。サロゲートペア・変換候補の選択後の確定は未確認。

## (b)〜(i) の確認状況

| 項目 | 状況 |
|---|---|
| (a) N・T | 確定 (N = 16、T = 16 ms) |
| (b) IME の確定が `Text` になり確定の Enter が漏れない | 確認 (上の表。Google 日本語入力とサロゲートペアは未確認) |
| (c) AltGr | 未確認 (試験機は日本語配列。`Ctrl+Alt` では文字が付かない) |
| (d) 日本語キー | 確認 (KeyUp だけ届く。詰まらない) |
| (e) Ctrl+Break | 確認 |
| (f) 写像 | 確認 (上の表) |
| (g) WT の貼り付け確認ダイアログ | 未確認 (自動操作の貼り付けでは出なかった) |
| (h) conhost の「Ctrl キーショートカットを有効にする」 | 未確認 (試験機の既定では `Ctrl+V` がアプリに届く設定) |
| (i) `WT_SESSION` の誤判定 | 未確認 |
| CONOUT$ + `WriteConsoleW` | Windows Terminal で文字化けなし。conhost は未確認 |

## 結論

1. N = 16・T = 16 ms を確定。読取は 1 回 4,096〜8,192 件に上限を付ける (04 §6.6 に反映)。
2. conhost の貼り付けは 0x08 と ESC を `VK_BACK` / `VK_ESCAPE` のキーで届けるので、バースト中は `UnicodeChar` から本文を作る規則が必須。Windows Terminal は貼り付けから除く。
3. キーの写像は 04 §6.2 の見込みどおり (反映済み)。IME キーは KeyUp だけ、IME の確定文字は `vk=0` の対、確定の Enter は KeyDown が届かない。
4. `Capabilities.enhanced_keys` は Windows では `Ctrl+Shift+文字` を区別できるので不要 (v0.1 は常に false のまま)。
