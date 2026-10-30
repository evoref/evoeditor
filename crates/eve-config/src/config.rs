//! ユーザー層の設定ファイル (`config.toml`) の読込と検証 (docs/07 §4.2・§5.0、01 FR-701〜703)。
//!
//! 公開キーは v0.1 の 7 個と、v0.2 の M6 で足した `file.swap.*` の 5 個。ファイルが無い・壊れている・値が不正なときも `Config` は必ず返り、
//! 問題は `Diagnostic` の列として返す (最初のエラーで止まらない)。不正な項目は既定値に戻す。

use toml::Spanned;
use toml::de::{DeString, DeTable, DeValue};

/// 診断の重さ。表示の文言は host / core が MessageKey で決める (docs/07 §4.3)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// 診断の種類 (docs/07 §4.2 の表)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagKind {
    /// TOML の構文エラー。ファイル全体を無視する。
    Syntax { message: String },
    /// 未知のキー・セクション。`suggestion` は編集距離 2 以下の公開キー。
    UnknownKey { suggestion: Option<String> },
    /// 後の版で公開する設定 (このバージョンでは未対応)。情報。画面には出さず、件数にも数えない (07 §4.3)。
    Unsupported,
    /// 型が違う。
    TypeMismatch {
        expected: &'static str,
        found: &'static str,
    },
    /// 範囲外 (整数)。
    OutOfRange { min: i64, max: i64 },
    /// 列挙値が不正。許容値を提示する。
    InvalidValue { allowed: &'static [&'static str] },
    /// キーマップ (`keymap.toml`) の問題。`Diagnostic.key` は書かれたキー、`value` は書かれたコマンドなど。
    Keymap(KeymapProblem),
}

/// キーマップの問題の種類 (docs/07 §7.4・§7.6)。いずれも該当の割当だけを無効にする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeymapProblem {
    /// 知らないコンテキスト名。
    UnknownContext,
    /// 知らないコマンド。
    UnknownCommand,
    /// キー名が読めない。
    BadKey,
    /// `when` 句が読めない。
    BadWhen,
    /// 修飾のない文字キー: 文字入力を奪うので割り当てられない (07 §7.6)。
    PrintableKey,
    /// 既定のチョードの先頭と同じ単体のキー: チョード側を優先する (07 §7.6)。
    ChordPrefix,
    /// 脱出キー (終了・パレット・取消) に届かなくなる割当: 既定を残す (07 §7.6)。
    EscapeKey,
}

/// 設定の問題 1 件。`line` / `column` は 1 始まり (構文エラーで位置が無いときは 0)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub kind: DiagKind,
    /// ドット区切りのキー (`editor.tab_width`)。構文エラーでは空。
    pub key: String,
    /// 書かれた値 (文字列はそのまま、整数は書かれた表記。値を持たない診断では空)。
    pub value: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentUnit {
    Tab,
    Space,
}

/// Backspace の削除の単位 (`editor.backspace_unit`、03 §13.1、OQ-45)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackspaceUnit {
    /// 書記素クラスタ単位 (既定)。
    Grapheme,
    /// コードポイント単位 (結合文字・ハングル字母を 1 つずつ消す)。
    Codepoint,
}

/// 空白の可視化 (`view.show_whitespace`、03 §13.2、FR-508)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowWhitespace {
    /// 可視化しない。
    Off,
    /// 全角スペース (U+3000) とタブ文字だけを記号で示す (既定)。
    Minimal,
}

/// 折り返しの方式 (`view.wrap`、03 §14.3、07 §5.3)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapMode {
    /// 折り返さない (水平スクロール、既定)。
    Off,
    /// 幅いっぱいの書記素単位で折る。
    Soft,
    /// 単語境界を優先して折る。
    Word,
}

/// 折り返した継続行のインデント (`view.wrap_indent`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapIndent {
    None,
    /// 論理行のインデント幅で揃える (既定)。
    Same,
}

/// OSC 52 でクリップボードへ書く方針 (`clipboard.osc52`、05 §8.1)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Osc52Policy {
    /// SSH 接続中、またはシステムのクリップボードに書けないとき (既定)。
    Auto,
    /// 端末が対応していれば、いつも書く。
    Always,
    /// 書かない。
    Never,
}

/// 新規ファイル・判定不能時の文字コード (`file.default_encoding`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    ShiftJis,
    EucJp,
    Iso2022Jp,
    /// OS ロケール由来のヒント (docs/05 §6)。
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultLineEnding {
    Lf,
    Crlf,
    Cr,
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    TrueColor,
    Color256,
    Color16,
    None,
}

/// 公開キー (v0.1 の 7 個 (docs/07 §5.0) と、v0.2 の M6 で足した `file.swap.*` の 5 個)。`Default` が既定値。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub tab_width: u8,
    pub indent_unit: IndentUnit,
    pub indent_width: u8,
    pub default_encoding: DefaultEncoding,
    pub default_line_ending: DefaultLineEnding,
    /// East Asian Ambiguous の幅 (1 か 2)。
    pub ambiguous_width: u8,
    pub color_mode: ColorMode,
    /// `file.swap.enabled`: swap-lite を使うか (FR-118、docs/07 §5.2)。
    pub swap_enabled: bool,
    /// `file.swap.interval_ms`: 最後の入力からスワップを書くまでの遅延。
    pub swap_interval_ms: u32,
    /// `file.swap.min_interval_ms`: 同じ文書のスワップの最短の間隔。
    pub swap_min_interval_ms: u32,
    /// `file.swap.max_bytes`: これを超える文書はスワップの対象外。
    pub swap_max_bytes: u64,
    /// `file.swap.exclude_secret_files`: 秘密ファイルの名前に一致する文書をスワップしない (OQ-109)。
    pub swap_exclude_secret_files: bool,
    /// `ui.locale`: UI の言語。`None` は `auto` (環境変数・OS ロケールで決める、docs/07 §17.1)。
    pub ui_locale: Option<crate::Locale>,
    /// `editor.undo.total_max_bytes`: プロセス全体の Undo 履歴の上限 (NFR-203、docs/03 §7.5)。
    pub undo_total_max_bytes: u64,
    /// `editor.backspace_unit`: Backspace の削除の単位 (FR-201)。
    pub backspace_unit: BackspaceUnit,
    /// `view.wrap`: 折り返しの方式 (FR-506)。
    pub wrap: WrapMode,
    /// `view.wrap_indent`: 継続行のインデント。
    pub wrap_indent: WrapIndent,
    /// `view.wrap_kinsoku`: 禁則処理 (OQ-57)。
    pub wrap_kinsoku: bool,
    /// `view.show_whitespace`: 空白の可視化。
    pub show_whitespace: ShowWhitespace,
    /// `ui.mouse`: マウスの入力を使うか (FR-307、04 §6.8)。
    pub mouse: bool,
    /// `ui.mouse_wheel_lines`: ホイール 1 目盛りのスクロール量 (表示行)。
    pub mouse_wheel_lines: u8,
    /// `ui.which_key_delay_ms`: チョードの保留から which-key を出すまでの時間 (04 §7.4)。
    pub which_key_delay_ms: u16,
    /// `clipboard.osc52`: OSC 52 の送出方針。
    pub osc52: Osc52Policy,
    /// `clipboard.osc52_max_bytes`: OSC 52 の本文 (base64) の上限。超えたら送らない (05 §8.4)。
    pub osc52_max_bytes: u32,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            tab_width: 4,
            indent_unit: IndentUnit::Space,
            indent_width: 4,
            default_encoding: DefaultEncoding::Utf8,
            default_line_ending: DefaultLineEnding::Lf,
            ambiguous_width: 1,
            color_mode: ColorMode::Auto,
            swap_enabled: true,
            swap_interval_ms: 2_000,
            swap_min_interval_ms: 10_000,
            swap_max_bytes: 64 * 1024 * 1024,
            swap_exclude_secret_files: true,
            ui_locale: None,
            undo_total_max_bytes: 256 * 1024 * 1024,
            backspace_unit: BackspaceUnit::Grapheme,
            wrap: WrapMode::Off,
            wrap_indent: WrapIndent::Same,
            wrap_kinsoku: true,
            show_whitespace: ShowWhitespace::Minimal,
            mouse: true,
            mouse_wheel_lines: 3,
            which_key_delay_ms: 500,
            osc52: Osc52Policy::Auto,
            osc52_max_bytes: 100_000,
        }
    }
}

/// `a` から `b` で値が変わった公開キー (`config.reload` の要約、07 §6 の規則 4)。並びは `Config` のフィールドの順。
pub fn changed_keys(a: &Config, b: &Config) -> Vec<&'static str> {
    // フィールドを足したら、ここにも足す (下の試験が既定値との差で全フィールドを確かめる)
    [
        ("editor.tab_width", a.tab_width != b.tab_width),
        ("editor.indent_unit", a.indent_unit != b.indent_unit),
        ("editor.indent_width", a.indent_width != b.indent_width),
        (
            "file.default_encoding",
            a.default_encoding != b.default_encoding,
        ),
        (
            "file.default_line_ending",
            a.default_line_ending != b.default_line_ending,
        ),
        (
            "view.ambiguous_width",
            a.ambiguous_width != b.ambiguous_width,
        ),
        ("theme.color_mode", a.color_mode != b.color_mode),
        ("file.swap.enabled", a.swap_enabled != b.swap_enabled),
        (
            "file.swap.interval_ms",
            a.swap_interval_ms != b.swap_interval_ms,
        ),
        (
            "file.swap.min_interval_ms",
            a.swap_min_interval_ms != b.swap_min_interval_ms,
        ),
        ("file.swap.max_bytes", a.swap_max_bytes != b.swap_max_bytes),
        (
            "file.swap.exclude_secret_files",
            a.swap_exclude_secret_files != b.swap_exclude_secret_files,
        ),
        ("ui.locale", a.ui_locale != b.ui_locale),
        (
            "editor.undo.total_max_bytes",
            a.undo_total_max_bytes != b.undo_total_max_bytes,
        ),
        (
            "editor.backspace_unit",
            a.backspace_unit != b.backspace_unit,
        ),
        ("view.wrap", a.wrap != b.wrap),
        ("view.wrap_indent", a.wrap_indent != b.wrap_indent),
        ("view.wrap_kinsoku", a.wrap_kinsoku != b.wrap_kinsoku),
        (
            "view.show_whitespace",
            a.show_whitespace != b.show_whitespace,
        ),
        ("ui.mouse", a.mouse != b.mouse),
        (
            "ui.mouse_wheel_lines",
            a.mouse_wheel_lines != b.mouse_wheel_lines,
        ),
        (
            "ui.which_key_delay_ms",
            a.which_key_delay_ms != b.which_key_delay_ms,
        ),
        ("clipboard.osc52", a.osc52 != b.osc52),
        (
            "clipboard.osc52_max_bytes",
            a.osc52_max_bytes != b.osc52_max_bytes,
        ),
    ]
    .into_iter()
    .filter_map(|(k, changed)| changed.then_some(k))
    .collect()
}

/// 読込の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub config: Config,
    pub diagnostics: Vec<Diagnostic>,
}

const SECTIONS: &[&str] = &["editor", "file", "view", "theme", "ui", "clipboard"];
pub(crate) const KEYS: &[&str] = &[
    "editor.tab_width",
    "editor.indent_unit",
    "editor.indent_width",
    "file.default_encoding",
    "file.default_line_ending",
    "view.ambiguous_width",
    "theme.color_mode",
    "file.swap.enabled",
    "file.swap.interval_ms",
    "file.swap.min_interval_ms",
    "file.swap.max_bytes",
    "file.swap.exclude_secret_files",
    "ui.locale",
    "editor.undo.total_max_bytes",
    "editor.backspace_unit",
    "view.wrap",
    "view.wrap_indent",
    "view.wrap_kinsoku",
    "view.show_whitespace",
    "ui.mouse",
    "ui.mouse_wheel_lines",
    "ui.which_key_delay_ms",
    "clipboard.osc52",
    "clipboard.osc52_max_bytes",
];

const INDENT_UNITS: &[&str] = &["tab", "space"];
const BACKSPACE_UNITS: &[&str] = &["grapheme", "codepoint"];
const SHOW_WHITESPACE: &[&str] = &["off", "minimal"];
const OSC52_POLICIES: &[&str] = &["auto", "always", "never"];
const WRAP_MODES: &[&str] = &["off", "soft", "word"];
const WRAP_INDENTS: &[&str] = &["none", "same"];
const ENCODINGS: &[&str] = &[
    "utf-8",
    "utf-8-bom",
    "utf-16le",
    "utf-16be",
    "shift_jis",
    "euc-jp",
    "iso-2022-jp",
    "system",
];
const LINE_ENDINGS: &[&str] = &["lf", "crlf", "cr", "native"];
const COLOR_MODES: &[&str] = &["auto", "truecolor", "256", "16", "none"];
const LOCALES: &[&str] = &["auto", "ja", "en"];

/// 設定ファイルの本文を読む。`text` が空でも構文エラーでも `Config` を返す。
pub fn load_str(text: &str) -> Loaded {
    load_str_onto(Config::default(), text)
}

/// `load_str` と同じだが、書かれていないキーは `base` の値のまま (層を重ねる、07 §3.3)。
/// 構文エラーなら `base` のまま返す。
pub fn load_str_onto(base: Config, text: &str) -> Loaded {
    let mut cx = Cx {
        text,
        out: Loaded {
            config: base,
            diagnostics: Vec::new(),
        },
    };
    let root = match DeTable::parse(text) {
        Ok(t) => t,
        Err(e) => {
            let (line, column) = e.span().map_or((0, 0), |s| cx.pos(s.start));
            cx.out.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                kind: DiagKind::Syntax {
                    message: e.message().to_owned(),
                },
                key: String::new(),
                value: String::new(),
                line,
                column,
            });
            return cx.out;
        }
    };
    for (name, value) in root.get_ref() {
        cx.section(name, value);
    }
    cx.out
}

struct Cx<'a> {
    text: &'a str,
    out: Loaded,
}

impl Cx<'_> {
    /// バイト位置 → (行, 桁)。どちらも 1 始まり、桁は文字数。
    fn pos(&self, offset: usize) -> (u32, u32) {
        let end = offset.min(self.text.len());
        let head = self.text.get(..end).unwrap_or("");
        let line = head.matches('\n').count() + 1;
        let col = head.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        (line as u32, col as u32)
    }

    fn diag(&mut self, severity: Severity, kind: DiagKind, key: String, at: usize) {
        let (line, column) = self.pos(at);
        self.out.diagnostics.push(Diagnostic {
            severity,
            kind,
            key,
            value: String::new(),
            line,
            column,
        });
    }

    fn section(&mut self, name: &Spanned<DeString<'_>>, value: &Spanned<DeValue<'_>>) {
        let n: &str = name.get_ref();
        if crate::reserved::RESERVED_SECTIONS.contains(&n) {
            self.diag(
                Severity::Info,
                DiagKind::Unsupported,
                n.to_owned(),
                name.span().start,
            );
            return;
        }
        if !SECTIONS.contains(&n) {
            let suggestion = nearest(n, SECTIONS).map(str::to_owned);
            self.diag(
                Severity::Warning,
                DiagKind::UnknownKey { suggestion },
                n.to_owned(),
                name.span().start,
            );
            return;
        }
        let DeValue::Table(table) = value.get_ref() else {
            self.diag(
                Severity::Warning,
                DiagKind::TypeMismatch {
                    expected: "table",
                    found: value.get_ref().type_str(),
                },
                n.to_owned(),
                value.span().start,
            );
            return;
        };
        self.keys(n, table);
    }

    /// テーブルの中のキーを読む。`[file.swap]` や `swap.enabled = ...` のような入れ子は、公開キーの接頭辞なら中へ入る。
    fn keys(&mut self, prefix: &str, table: &DeTable<'_>) {
        for (k, v) in table {
            let full = format!("{prefix}.{}", k.get_ref());
            let nested = format!("{full}.");
            if KEYS.contains(&full.as_str()) {
                self.value(&full, v);
            } else if let (DeValue::Table(inner), true) =
                (v.get_ref(), KEYS.iter().any(|key| key.starts_with(&nested)))
            {
                self.keys(&full, inner);
            } else if is_reserved_key(&full) {
                self.diag(Severity::Info, DiagKind::Unsupported, full, k.span().start);
            } else {
                let all: Vec<&str> = KEYS
                    .iter()
                    .chain(crate::reserved::RESERVED_KEYS)
                    .copied()
                    .collect();
                let suggestion = nearest(&full, &all).map(str::to_owned);
                self.diag(
                    Severity::Warning,
                    DiagKind::UnknownKey { suggestion },
                    full,
                    k.span().start,
                );
            }
        }
    }

    fn value(&mut self, key: &str, v: &Spanned<DeValue<'_>>) {
        match key {
            "editor.tab_width" => {
                if let Some(n) = self.int(key, v, 1, 16) {
                    self.out.config.tab_width = n as u8;
                }
            }
            "editor.indent_width" => {
                if let Some(n) = self.int(key, v, 1, 16) {
                    self.out.config.indent_width = n as u8;
                }
            }
            "file.swap.enabled" => {
                if let Some(b) = self.boolean(key, v) {
                    self.out.config.swap_enabled = b;
                }
            }
            "file.swap.exclude_secret_files" => {
                if let Some(b) = self.boolean(key, v) {
                    self.out.config.swap_exclude_secret_files = b;
                }
            }
            "file.swap.interval_ms" => {
                if let Some(n) = self.int(key, v, 100, 600_000) {
                    self.out.config.swap_interval_ms = n as u32;
                }
            }
            "file.swap.min_interval_ms" => {
                if let Some(n) = self.int(key, v, 100, 3_600_000) {
                    self.out.config.swap_min_interval_ms = n as u32;
                }
            }
            "file.swap.max_bytes" => {
                if let Some(n) = self.int(key, v, 1_048_576, 1 << 40) {
                    self.out.config.swap_max_bytes = n as u64;
                }
            }
            "view.ambiguous_width" => {
                if let Some(n) = self.int(key, v, 1, 2) {
                    self.out.config.ambiguous_width = n as u8;
                }
            }
            "editor.indent_unit" => {
                if let Some(s) = self.string(key, v, INDENT_UNITS) {
                    self.out.config.indent_unit = match s.as_str() {
                        "tab" => IndentUnit::Tab,
                        _ => IndentUnit::Space,
                    };
                }
            }
            "editor.backspace_unit" => {
                if let Some(s) = self.string(key, v, BACKSPACE_UNITS) {
                    self.out.config.backspace_unit = match s.as_str() {
                        "codepoint" => BackspaceUnit::Codepoint,
                        _ => BackspaceUnit::Grapheme,
                    };
                }
            }
            "view.wrap" => {
                if let Some(s) = self.string(key, v, WRAP_MODES) {
                    self.out.config.wrap = match s.as_str() {
                        "soft" => WrapMode::Soft,
                        "word" => WrapMode::Word,
                        _ => WrapMode::Off,
                    };
                }
            }
            "view.wrap_indent" => {
                if let Some(s) = self.string(key, v, WRAP_INDENTS) {
                    self.out.config.wrap_indent = match s.as_str() {
                        "none" => WrapIndent::None,
                        _ => WrapIndent::Same,
                    };
                }
            }
            "clipboard.osc52" => {
                if let Some(s) = self.string(key, v, OSC52_POLICIES) {
                    self.out.config.osc52 = match s.as_str() {
                        "always" => Osc52Policy::Always,
                        "never" => Osc52Policy::Never,
                        _ => Osc52Policy::Auto,
                    };
                }
            }
            "clipboard.osc52_max_bytes" => {
                if let Some(n) = self.int(key, v, 1_000, 16_777_216) {
                    self.out.config.osc52_max_bytes = n as u32;
                }
            }
            "ui.mouse" => {
                if let Some(b) = self.boolean(key, v) {
                    self.out.config.mouse = b;
                }
            }
            "ui.which_key_delay_ms" => {
                if let Some(n) = self.int(key, v, 0, 5_000) {
                    self.out.config.which_key_delay_ms = n as u16;
                }
            }
            "ui.mouse_wheel_lines" => {
                if let Some(n) = self.int(key, v, 1, 20) {
                    self.out.config.mouse_wheel_lines = n as u8;
                }
            }
            "view.show_whitespace" => {
                let Some(s) = self.raw_string(key, v) else {
                    return;
                };
                match s.as_str() {
                    "off" => self.out.config.show_whitespace = ShowWhitespace::Off,
                    "minimal" => self.out.config.show_whitespace = ShowWhitespace::Minimal,
                    // 後の版の値 (v0.4): 未対応の情報診断にして既定の `minimal` を使う (07 §5.3)
                    "ideographic" | "trailing" | "all" => {
                        self.diag(
                            Severity::Info,
                            DiagKind::Unsupported,
                            key.to_owned(),
                            v.span().start,
                        );
                        self.set_value(v);
                    }
                    _ => self.invalid(key, SHOW_WHITESPACE, v),
                }
            }
            "view.wrap_kinsoku" => {
                if let Some(b) = self.boolean(key, v) {
                    self.out.config.wrap_kinsoku = b;
                }
            }
            "file.default_encoding" => {
                let Some(s) = self.raw_string(key, v) else {
                    return;
                };
                match encoding(&s) {
                    Some(e) => self.out.config.default_encoding = e,
                    None => self.invalid(key, ENCODINGS, v),
                }
            }
            "file.default_line_ending" => {
                if let Some(s) = self.string(key, v, LINE_ENDINGS) {
                    self.out.config.default_line_ending = match s.as_str() {
                        "crlf" => DefaultLineEnding::Crlf,
                        "cr" => DefaultLineEnding::Cr,
                        "native" => DefaultLineEnding::Native,
                        _ => DefaultLineEnding::Lf,
                    };
                }
            }
            "editor.undo.total_max_bytes" => {
                if let Some(n) = self.int(key, v, 1_048_576, 1 << 40) {
                    self.out.config.undo_total_max_bytes = n as u64;
                }
            }
            "ui.locale" => {
                let Some(s) = self.raw_string(key, v) else {
                    return;
                };
                if s.trim().eq_ignore_ascii_case("auto") {
                    self.out.config.ui_locale = None;
                } else if let Some(l) = crate::Locale::parse(&s) {
                    self.out.config.ui_locale = Some(l);
                } else {
                    self.invalid(key, LOCALES, v);
                }
            }
            "theme.color_mode" => {
                if let Some(s) = self.string(key, v, COLOR_MODES) {
                    self.out.config.color_mode = match s.as_str() {
                        "truecolor" => ColorMode::TrueColor,
                        "256" => ColorMode::Color256,
                        "16" => ColorMode::Color16,
                        "none" => ColorMode::None,
                        _ => ColorMode::Auto,
                    };
                }
            }
            _ => {}
        }
    }

    fn mismatch(&mut self, key: &str, v: &Spanned<DeValue<'_>>, expected: &'static str) {
        self.diag(
            Severity::Warning,
            DiagKind::TypeMismatch {
                expected,
                found: v.get_ref().type_str(),
            },
            key.to_owned(),
            v.span().start,
        );
        self.set_value(v);
    }

    fn invalid(&mut self, key: &str, allowed: &'static [&'static str], v: &Spanned<DeValue<'_>>) {
        self.diag(
            Severity::Warning,
            DiagKind::InvalidValue { allowed },
            key.to_owned(),
            v.span().start,
        );
        self.set_value(v);
    }

    /// 直前に積んだ診断に、書かれた値を記録する。
    fn set_value(&mut self, v: &Spanned<DeValue<'_>>) {
        let text = match v.get_ref() {
            DeValue::String(s) => s.to_string(),
            DeValue::Integer(i) => i.as_str().to_owned(),
            DeValue::Float(f) => f.as_str().to_owned(),
            DeValue::Boolean(b) => b.to_string(),
            other => other.type_str().to_owned(),
        };
        if let Some(d) = self.out.diagnostics.last_mut() {
            d.value = text;
        }
    }

    /// 範囲 `min..=max` の整数。型違い・範囲外は診断を積んで `None` (呼び出し側は既定値のまま)。
    fn int(&mut self, key: &str, v: &Spanned<DeValue<'_>>, min: i64, max: i64) -> Option<i64> {
        let DeValue::Integer(i) = v.get_ref() else {
            self.mismatch(key, v, "integer");
            return None;
        };
        let Ok(n) = i64::from_str_radix(&i.as_str().replace('_', ""), i.radix()) else {
            self.diag(
                Severity::Warning,
                DiagKind::OutOfRange { min, max },
                key.to_owned(),
                v.span().start,
            );
            self.set_value(v);
            return None;
        };
        if (min..=max).contains(&n) {
            Some(n)
        } else {
            self.diag(
                Severity::Warning,
                DiagKind::OutOfRange { min, max },
                key.to_owned(),
                v.span().start,
            );
            self.set_value(v);
            None
        }
    }

    /// 真偽値。型違いは診断を積んで `None`。
    fn boolean(&mut self, key: &str, v: &Spanned<DeValue<'_>>) -> Option<bool> {
        if let DeValue::Boolean(b) = v.get_ref() {
            Some(*b)
        } else {
            self.mismatch(key, v, "boolean");
            None
        }
    }

    fn raw_string(&mut self, key: &str, v: &Spanned<DeValue<'_>>) -> Option<String> {
        if let DeValue::String(s) = v.get_ref() {
            Some(s.to_string())
        } else {
            self.mismatch(key, v, "string");
            None
        }
    }

    /// 許容値のどれかに一致する文字列。
    fn string(
        &mut self,
        key: &str,
        v: &Spanned<DeValue<'_>>,
        allowed: &'static [&'static str],
    ) -> Option<String> {
        let s = self.raw_string(key, v)?;
        if allowed.contains(&s.as_str()) {
            Some(s)
        } else {
            self.invalid(key, allowed, v);
            None
        }
    }
}

/// `file.default_encoding` の値。WHATWG の別名ラベルは正規名に寄せる (docs/07 §5.2)。
fn encoding(s: &str) -> Option<DefaultEncoding> {
    Some(match s.to_ascii_lowercase().as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" => DefaultEncoding::Utf8,
        "utf-8-bom" => DefaultEncoding::Utf8Bom,
        "utf-16le" | "utf-16" | "ucs-2" | "unicode" | "csunicode" => DefaultEncoding::Utf16Le,
        "utf-16be" | "unicodefffe" => DefaultEncoding::Utf16Be,
        "shift_jis" | "shift-jis" | "sjis" | "windows-31j" | "ms_kanji" | "csshiftjis"
        | "x-sjis" | "ms932" | "cp932" => DefaultEncoding::ShiftJis,
        "euc-jp" | "x-euc-jp" | "cseucpkdfmtjapanese" => DefaultEncoding::EucJp,
        "iso-2022-jp" | "csiso2022jp" => DefaultEncoding::Iso2022Jp,
        "system" => DefaultEncoding::System,
        _ => return None,
    })
}

/// 公開前のキー (その下の入れ子のキーを含む)。
fn is_reserved_key(full: &str) -> bool {
    let prefix = format!("{full}.");
    crate::reserved::RESERVED_KEYS
        .iter()
        .any(|r| *r == full || r.starts_with(&prefix))
}

/// 公開キーの既定値の表記 (診断の「既定値 {default} を使います」に入れる)。未知のキーは空。
pub fn default_display(key: &str) -> &'static str {
    match key {
        "editor.tab_width" | "editor.indent_width" => "4",
        "editor.indent_unit" => "space",
        "editor.backspace_unit" => "grapheme",
        "view.wrap" => "off",
        "view.show_whitespace" => "minimal",
        "view.wrap_indent" => "same",
        "view.wrap_kinsoku" | "ui.mouse" => "true",
        "ui.mouse_wheel_lines" => "3",
        "ui.which_key_delay_ms" => "500",
        "clipboard.osc52" => "auto",
        "clipboard.osc52_max_bytes" => "100000",
        "file.default_encoding" => "utf-8",
        "file.default_line_ending" => "lf",
        "view.ambiguous_width" => "1",
        "file.swap.enabled" | "file.swap.exclude_secret_files" => "true",
        "file.swap.interval_ms" => "2000",
        "file.swap.min_interval_ms" => "10000",
        "file.swap.max_bytes" => "67108864",
        "editor.undo.total_max_bytes" => "268435456",
        "theme.color_mode" | "ui.locale" => "auto",
        _ => "",
    }
}

/// 編集距離 2 以下で最も近い候補 (同点は先頭)。
fn nearest<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .map(|c| (distance(name, c), *c))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests;
