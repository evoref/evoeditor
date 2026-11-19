# 設定リファレンス

生成物 (`cargo xtask gen-reference` で作り直す。手で直さない。docs/09 §11.2)。`evoeditor --dump-config --defaults --json` の出力 (公開キー 36 個と既定値) から作る。キー・型・許容値・意味の正本は [07 §5](../07-config-extensibility.md#5-設定項目一覧-スキーマと既定値)。既定値は `config.toml` にそのまま書ける表記 (TOML の値)。今の設定を重ねた結果は `evoeditor --dump-config` で見る。

## `[editor]`

| キー | 既定値 |
|---|---|
| `editor.tab_width` | `4` |
| `editor.indent_unit` | `"space"` |
| `editor.indent_width` | `4` |
| `editor.backspace_unit` | `"grapheme"` |
| `editor.undo.total_max_bytes` | `268435456` |

## `[file]`

| キー | 既定値 |
|---|---|
| `file.default_encoding` | `"utf-8"` |
| `file.default_line_ending` | `"lf"` |
| `file.huge_file_warn_bytes` | `536870912` |
| `file.long_line_threshold` | `1048576` |
| `file.reload_on_external_change` | `"auto_if_unmodified"` |
| `file.swap.enabled` | `true` |
| `file.swap.interval_ms` | `2000` |
| `file.swap.min_interval_ms` | `10000` |
| `file.swap.max_bytes` | `67108864` |
| `file.swap.exclude_secret_files` | `true` |

## `[view]`

| キー | 既定値 |
|---|---|
| `view.ambiguous_width` | `1` |
| `view.wrap` | `"off"` |
| `view.wrap_indent` | `"same"` |
| `view.wrap_kinsoku` | `true` |
| `view.show_whitespace` | `"minimal"` |

## `[theme]`

| キー | 既定値 |
|---|---|
| `theme.color_mode` | `"auto"` |
| `theme.name` | `"default"` |
| `theme.background` | `"auto"` |

## `[ui]`

| キー | 既定値 |
|---|---|
| `ui.locale` | `"auto"` |
| `ui.mouse` | `true` |
| `ui.hint_bar` | `true` |
| `ui.mouse_wheel_lines` | `3` |
| `ui.which_key_delay_ms` | `500` |

## `[search]`

| キー | 既定値 |
|---|---|
| `search.newline_sugar` | `true` |
| `search.cp932_compat_fold` | `true` |
| `search.case` | `"smart"` |
| `search.regex_default` | `false` |
| `search.max_matches` | `10000` |
| `search.count_limit` | `1000000` |

## `[clipboard]`

| キー | 既定値 |
|---|---|
| `clipboard.osc52` | `"auto"` |
| `clipboard.osc52_max_bytes` | `100000` |
