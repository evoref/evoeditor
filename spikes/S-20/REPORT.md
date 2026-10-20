# S-20 REPORT (色縮退の品質と色深度の判定)

状態: **判定済み** (2026-10-21)

## 方法

`spikes/S-20` の `env` / `contrast` / `show` を Windows Terminal と conhost で実行した。`show` は提案した `default` テーマの truecolor と mono (属性だけ)、truecolor からの 256 色・16 色の近似を並べる。判別性は人が目視した。

## env (色深度の判定、04 §5.2a)

| | Windows Terminal | conhost |
|---|---|---|
| `WT_SESSION` | あり | なし |
| `NO_COLOR` / `COLORTERM` / `TERM` / `TERM_PROGRAM` / `COLORFGBG` | 未設定 | 未設定 |
| 04 §5.2a の判定 | truecolor (順 4) | truecolor (順 6) |
| 端末種別 (04 §3.7 A2) | Windows Terminal | Windows Terminal 以外 (conhost 等) |

`COLORFGBG` はどちらの端末でも設定されない (v0.1 は使わない。OQ-124 の v0.2 の案は、Windows ではヒントにならない)。

## contrast (提案値のコントラスト比)

全ロールが基準 (組は 4.5:1 以上、前景のみは白と黒の両方に 3.0:1 以上) を満たした。最小は `message.info` の 4.63:1。

| ロール | truecolor | 比 |
|---|---|---|
| selection | #ffffff / #264f78 | 8.50 |
| search.match | #000000 / #f2c94c | 13.23 |
| search.current | #000000 / #ff9e64 | 10.33 |
| status_bar | #ffffff / #3b4252 | 10.06 |
| status_bar.inactive | #c8c8c8 / #3b4252 | 6.01 |
| hint_bar | #d8dee9 / #2e3440 | 9.25 |
| hint_bar.key | #ffffff / #2e3440 | 12.49 |
| message.error | #ffffff / #b3261e | 6.54 |
| message.warning | #000000 / #f2c94c | 13.23 |
| message.info | #ffffff / #1f6feb | 4.63 |
| control_char | #ffffff / #8b3a62 | 7.29 |
| overlay.selected | #ffffff / #45475a | 9.12 |
| line_number | #808080 (前景のみ) | 白 3.95 / 黒 5.32 |
| whitespace | #767676 (前景のみ) | 白 4.54 / 黒 4.62 |

## show の判別性 (目視)

Windows Terminal・conhost のどちらでも、truecolor の段と mono の段で、選択・検索一致と現在の一致・制御文字・メッセージ 3 種 (記号 `✖ ⚠ ℹ` を併用)・一覧の選択行が、隣のロールと区別できた (報告: 区別できた)。

## 結論と注意

- 提案した 2 段 (truecolor と mono) の方式で区別でき、コントラスト比の基準も満たす。07 §8.2 のロール表の方式 (強調ロールは前景と背景の組、補助は前景のみ、mono は属性) はそのまま採れる。
- **注意**: 本スパイクの色の値は 07 §8.2 の表の値と同一ではない (色相が違う)。07 §8.2 の値は、表中の比 (すべて基準以上) と、同じ形のロールの組で区別できたことから採れる見込みだが、表の値そのものを `show` で見たわけではない。`xtask check-themes` を入れるときに、表の値を入力にして同じ検査をする。
- conhost の `crossterm` の色コマンド (`SetBackgroundColor(Rgb)`) は、S-08 の自動測定で、RGB の背景がマゼンタに出なかった (生の `ESC[48;2;…m` では出た)。eve-tui は生のエスケープで色を書くか、conhost で RGB が出ることを M2 で確認する。
