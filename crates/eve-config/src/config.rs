//! ユーザー層の設定ファイル (`config.toml`) の読込と検証 (docs/07 §4.2・§5.0、01 FR-701〜703)。
//!
//! 公開キーは宣言の表 (`config_keys!`、07 §4.1) で決め、`Config`・既定値・読込・`--dump-config` の表記をそこから生成する。
//! ファイルが無い・壊れている・値が不正なときも `Config` は必ず返り、問題は `Diagnostic` の列として返す
//! (最初のエラーで止まらない)。不正な項目は既定値に戻す。

use std::sync::OnceLock;

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
    /// `when` 句に知らない変数がある (07 §7.3)。`Diagnostic.value` はその変数名。
    UnknownWhenVar,
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

/// 公開キーの値の TOML の表記 (`--dump-config` の出力と、診断の既定値の表記)。
pub(crate) trait ConfigValue {
    fn toml(&self) -> String;
}

macro_rules! plain_value {
    ($($ty:ty),*) => {
        $(impl ConfigValue for $ty {
            fn toml(&self) -> String {
                self.to_string()
            }
        })*
    };
}
plain_value!(u8, u16, u32, u64, bool);

impl ConfigValue for String {
    fn toml(&self) -> String {
        format!("\"{self}\"")
    }
}

impl ConfigValue for Option<crate::Locale> {
    fn toml(&self) -> String {
        format!("\"{}\"", self.map_or("auto", |l| l.code()))
    }
}

/// 列挙値の宣言: `型 { 変種 = "設定ファイルの値", ... }`。許容値の一覧 (`NAMES`、並びは診断に出す順)・
/// 値からの変換・TOML の表記を生成する。
macro_rules! config_enum {
    ($(
        $(#[$meta:meta])*
        $ty:ident { $( $(#[$vmeta:meta])* $var:ident = $name:literal, )* }
    )*) => {$(
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $ty {
            $( $(#[$vmeta])* $var, )*
        }

        impl $ty {
            /// 設定ファイルに書ける値 (許容値)。
            pub(crate) const NAMES: &'static [&'static str] = &[$($name),*];

            pub(crate) fn from_name(s: &str) -> Option<$ty> {
                match s {
                    $($name => Some($ty::$var),)*
                    _ => None,
                }
            }

            pub(crate) fn name(self) -> &'static str {
                match self {
                    $($ty::$var => $name,)*
                }
            }
        }

        impl ConfigValue for $ty {
            fn toml(&self) -> String {
                format!("\"{}\"", self.name())
            }
        }
    )*};
}

config_enum! {
    IndentUnit {
        Tab = "tab",
        Space = "space",
    }

    /// Backspace の削除の単位 (`editor.backspace_unit`、03 §13.1、OQ-45)。
    BackspaceUnit {
        /// 書記素クラスタ単位 (既定)。
        Grapheme = "grapheme",
        /// コードポイント単位 (結合文字・ハングル字母を 1 つずつ消す)。
        Codepoint = "codepoint",
    }

    /// 背景を塗るテーマの明暗 (`theme.background`、07 §5.4・§8.6)。
    ThemeBackground {
        /// テーマの名前どおり (既定)。
        Auto = "auto",
        Dark = "dark",
        Light = "light",
    }

    /// 空白の可視化 (`view.show_whitespace`、03 §13.2、FR-508)。
    ShowWhitespace {
        /// 可視化しない。
        Off = "off",
        /// 全角スペース (U+3000) とタブ文字だけを記号で示す (既定)。
        Minimal = "minimal",
    }

    /// 折り返しの方式 (`view.wrap`、03 §14.3、07 §5.3)。
    WrapMode {
        /// 折り返さない (水平スクロール、既定)。
        Off = "off",
        /// 幅いっぱいの書記素単位で折る。
        Soft = "soft",
        /// 単語境界を優先して折る。
        Word = "word",
    }

    /// 折り返した継続行のインデント (`view.wrap_indent`)。
    WrapIndent {
        None = "none",
        /// 論理行のインデント幅で揃える (既定)。
        Same = "same",
    }

    /// OSC 52 でクリップボードへ書く方針 (`clipboard.osc52`、05 §8.1)。
    Osc52Policy {
        /// SSH 接続中、またはシステムのクリップボードに書けないとき (既定)。
        Auto = "auto",
        /// 端末が対応していれば、いつも書く。
        Always = "always",
        /// 書かない。
        Never = "never",
    }

    /// 外部変更の扱い (`file.reload_on_external_change`、03 §9.5)。
    ExternalChangePolicy {
        /// いつも確認する。
        Ask = "ask",
        /// dirty でなければ確認なしで再読込する (既定)。
        AutoIfUnmodified = "auto_if_unmodified",
        /// フォーカス復帰で確かめない (明示のコマンドと保存時の照合は行う)。
        Never = "never",
    }

    /// 新規ファイル・判定不能時の文字コード (`file.default_encoding`)。別名は `encoding` で正規名に寄せる。
    DefaultEncoding {
        Utf8 = "utf-8",
        Utf8Bom = "utf-8-bom",
        Utf16Le = "utf-16le",
        Utf16Be = "utf-16be",
        ShiftJis = "shift_jis",
        EucJp = "euc-jp",
        Iso2022Jp = "iso-2022-jp",
        /// OS ロケール由来のヒント (docs/05 §6)。
        System = "system",
    }

    DefaultLineEnding {
        Lf = "lf",
        Crlf = "crlf",
        Cr = "cr",
        Native = "native",
    }

    ColorMode {
        Auto = "auto",
        TrueColor = "truecolor",
        Color256 = "256",
        Color16 = "16",
        None = "none",
    }
}

/// テーマの名前として使える文字列か (組込みの名前、またはテーマファイル `themes/<name>.toml` の名前。
/// 英数字・`-`・`_` だけで 1〜64 文字。パスの区切りや `..` を入れさせない)。
pub fn is_theme_name(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// 公開キー 1 つの値の読み方 (`config_keys!` の 4 列目)。
/// - `int(min, max)`: 範囲 `min..=max` の整数 (範囲外は警告して既定値)。
/// - `boolean`: 真偽値。
/// - `choice`: `config_enum!` で宣言した列挙値 (許容値の外は警告して既定値)。
/// - `custom(f)`: 関数 `f(cx, key, v) -> Option<型>` で読む (別名・後の版の値など、表で書けないもの)。
macro_rules! read_value {
    ($cx:expr, $key:expr, $v:expr, $ty:ty, int($min:expr, $max:expr)) => {
        $cx.int($key, $v, $min, $max).map(|n| n as $ty)
    };
    ($cx:expr, $key:expr, $v:expr, $ty:ty, boolean) => {
        $cx.boolean($key, $v)
    };
    ($cx:expr, $key:expr, $v:expr, $ty:ty, choice) => {
        $cx.string($key, $v, <$ty>::NAMES)
            .and_then(|s| <$ty>::from_name(&s))
    };
    ($cx:expr, $key:expr, $v:expr, $ty:ty, custom($f:expr)) => {
        $f($cx, $key, $v)
    };
}

/// 公開キーの宣言の表 (docs/07 §4.1): `フィールド: 型 = 既定値, "キー", 読み方;`。
/// ここから `Config` とその `Default`、公開キーの一覧 `KEYS`、`changed_keys`、読込 (`Cx::value`)、
/// TOML の表記 (`declared_values`。`--dump-config` と `default_display` が使う) を生成する。
/// キーを足すときは、この表に 1 行足す (07 §5 の表と `dist/config.example.toml` も。どちらも試験が突合する)。
macro_rules! config_keys {
    ($(
        $(#[$doc:meta])*
        $field:ident: $ty:ty = $default:expr, $key:literal, $read:ident $(($($arg:expr),*))?;
    )*) => {
        /// 公開キーの値 (宣言の表 `config_keys!` から生成)。`Default` が既定値。
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct Config {
            $( $(#[$doc])* pub $field: $ty, )*
        }

        impl Default for Config {
            fn default() -> Config {
                Config {
                    $( $field: $default, )*
                }
            }
        }

        /// 公開キー (宣言の順)。
        pub(crate) const KEYS: &[&str] = &[$($key),*];

        /// `a` から `b` で値が変わった公開キー (`config.reload` の要約、07 §6 の規則 4)。並びは宣言の順。
        pub fn changed_keys(a: &Config, b: &Config) -> Vec<&'static str> {
            let mut out = Vec::new();
            $(
                if a.$field != b.$field {
                    out.push($key);
                }
            )*
            out
        }

        /// 公開キーの (キー, TOML の値の表記)。宣言の順。
        pub(crate) fn declared_values(c: &Config) -> Vec<(&'static str, String)> {
            vec![$( ($key, ConfigValue::toml(&c.$field)), )*]
        }

        impl Cx<'_> {
            /// 公開キー `key` の値を読む。誤りは診断を積んで、値を変えない (既定値または下の層の値のまま)。
            fn value(&mut self, key: &str, v: &Spanned<DeValue<'_>>) {
                match key {
                    $(
                        $key => {
                            if let Some(x) = read_value!(self, key, v, $ty, $read $(($($arg),*))?) {
                                self.out.config.$field = x;
                            }
                        }
                    )*
                    _ => {}
                }
            }
        }
    };
}

config_keys! {
    /// `editor.tab_width`: タブ文字の表示幅。
    tab_width: u8 = 4, "editor.tab_width", int(1, 16);
    /// `editor.indent_unit`: インデントの操作で入れる単位。
    indent_unit: IndentUnit = IndentUnit::Space, "editor.indent_unit", choice;
    /// `editor.indent_width`: スペースのインデントの幅。
    indent_width: u8 = 4, "editor.indent_width", int(1, 16);
    /// `file.default_encoding`: 新規ファイル・判定不能時の文字コード。
    default_encoding: DefaultEncoding = DefaultEncoding::Utf8,
        "file.default_encoding", custom(read_encoding);
    /// `file.default_line_ending`: 新規ファイルの改行。
    default_line_ending: DefaultLineEnding = DefaultLineEnding::Lf,
        "file.default_line_ending", choice;
    /// `view.ambiguous_width`: East Asian Ambiguous の幅 (1 か 2)。
    ambiguous_width: u8 = 1, "view.ambiguous_width", int(1, 2);
    /// `theme.color_mode`: 色深度の判定の上書き。
    color_mode: ColorMode = ColorMode::Auto, "theme.color_mode", choice;
    /// `file.swap.enabled`: swap-lite を使うか (FR-118、docs/07 §5.2)。
    swap_enabled: bool = true, "file.swap.enabled", boolean;
    /// `file.swap.interval_ms`: 最後の入力からスワップを書くまでの遅延。
    swap_interval_ms: u32 = 2_000, "file.swap.interval_ms", int(100, 600_000);
    /// `file.swap.min_interval_ms`: 同じ文書のスワップの最短の間隔。
    swap_min_interval_ms: u32 = 10_000, "file.swap.min_interval_ms", int(100, 3_600_000);
    /// `file.swap.max_bytes`: これを超える文書はスワップの対象外。
    swap_max_bytes: u64 = 64 * 1024 * 1024, "file.swap.max_bytes", int(1_048_576, 1 << 40);
    /// `file.swap.exclude_secret_files`: 秘密ファイルの名前に一致する文書をスワップしない (OQ-109)。
    swap_exclude_secret_files: bool = true, "file.swap.exclude_secret_files", boolean;
    /// `ui.locale`: UI の言語。`None` は `auto` (環境変数・OS ロケールで決める、docs/07 §17.1)。
    ui_locale: Option<crate::Locale> = None, "ui.locale", custom(read_locale);
    /// `editor.undo.total_max_bytes`: プロセス全体の Undo 履歴の上限 (NFR-203、docs/03 §7.5)。
    undo_total_max_bytes: u64 = 256 * 1024 * 1024,
        "editor.undo.total_max_bytes", int(1_048_576, 1 << 40);
    /// `editor.backspace_unit`: Backspace の削除の単位 (FR-201)。
    backspace_unit: BackspaceUnit = BackspaceUnit::Grapheme, "editor.backspace_unit", choice;
    /// `view.wrap`: 折り返しの方式 (FR-506)。
    wrap: WrapMode = WrapMode::Off, "view.wrap", choice;
    /// `view.wrap_indent`: 継続行のインデント。
    wrap_indent: WrapIndent = WrapIndent::Same, "view.wrap_indent", choice;
    /// `view.wrap_kinsoku`: 禁則処理 (OQ-57)。
    wrap_kinsoku: bool = true, "view.wrap_kinsoku", boolean;
    /// `view.show_whitespace`: 空白の可視化。
    show_whitespace: ShowWhitespace = ShowWhitespace::Minimal,
        "view.show_whitespace", custom(read_show_whitespace);
    /// `theme.name`: テーマ。
    theme_name: String = "default".to_owned(), "theme.name", custom(read_theme_name);
    /// `theme.background`: 背景を塗るテーマの明暗。
    theme_background: ThemeBackground = ThemeBackground::Auto, "theme.background", choice;
    /// `ui.mouse`: マウスの入力を使うか (FR-307、04 §6.8)。
    mouse: bool = true, "ui.mouse", boolean;
    /// `ui.hint_bar`: 画面下のヒントバーを出すか (FR-504、04 §8.5)。`ui.toggle_hint_bar` で実行中に切り替えられる。
    hint_bar: bool = true, "ui.hint_bar", boolean;
    /// `ui.mouse_wheel_lines`: ホイール 1 目盛りのスクロール量 (表示行)。
    mouse_wheel_lines: u8 = 3, "ui.mouse_wheel_lines", int(1, 20);
    /// `ui.which_key_delay_ms`: チョードの保留から which-key を出すまでの時間 (04 §7.4)。
    which_key_delay_ms: u16 = 500, "ui.which_key_delay_ms", int(0, 5_000);
    /// `clipboard.osc52`: OSC 52 の送出方針。
    osc52: Osc52Policy = Osc52Policy::Auto, "clipboard.osc52", choice;
    /// `clipboard.osc52_max_bytes`: OSC 52 の本文 (base64) の上限。超えたら送らない (05 §8.4)。
    osc52_max_bytes: u32 = 100_000, "clipboard.osc52_max_bytes", int(1_000, 16_777_216);
    /// `file.huge_file_warn_bytes`: これを超えるファイルは開く前に警告する (FR-127、docs/03 §11.2)。
    huge_file_warn_bytes: u64 = 512 * 1024 * 1024,
        "file.huge_file_warn_bytes", int(1_048_576, 1 << 40);
    /// `file.long_line_threshold`: これを超える行を長大行として扱う (NFR-106、docs/03 §11.5)。
    /// 下限は長大行で表示する先頭 (64 KiB)。
    long_line_threshold: u64 = 1024 * 1024, "file.long_line_threshold", int(65_536, 1 << 30);
    /// `file.reload_on_external_change`: 外部変更の扱い (FR-117)。
    reload_on_external_change: ExternalChangePolicy = ExternalChangePolicy::AutoIfUnmodified,
        "file.reload_on_external_change", choice;
}

/// 読込の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub config: Config,
    pub diagnostics: Vec<Diagnostic>,
}

/// 公開キーを持つセクション (公開キーの先頭の名前。並びは `--dump-config` の出力の順、07 §5 の表の順)。
const SECTIONS: &[&str] = &["editor", "file", "view", "theme", "ui", "clipboard"];

const THEME_NAMES: &[&str] = &["default", "default-dark", "default-light", "ansi"];
const LOCALES: &[&str] = &["auto", "ja", "en"];

/// `--dump-config` の並びの順位: セクション (`SECTIONS` の順)、セクションの直下のキー、入れ子のキー (`file.swap.*`)。
pub(crate) fn dump_rank(key: &str) -> (usize, bool) {
    let section = key.split('.').next().unwrap_or("");
    let rank = SECTIONS
        .iter()
        .position(|s| *s == section)
        .unwrap_or(SECTIONS.len());
    (rank, key.matches('.').count() > 1)
}

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

/// `file.default_encoding`: 正規名か WHATWG の別名ラベル (docs/07 §5.2)。
fn read_encoding(cx: &mut Cx<'_>, key: &str, v: &Spanned<DeValue<'_>>) -> Option<DefaultEncoding> {
    let s = cx.raw_string(key, v)?;
    let e = encoding(&s);
    if e.is_none() {
        cx.invalid(key, DefaultEncoding::NAMES, v);
    }
    e
}

/// `view.show_whitespace`: 後の版の値 (v0.5) は、未対応の情報診断にして既定の `minimal` を使う (07 §5.3)。
fn read_show_whitespace(
    cx: &mut Cx<'_>,
    key: &str,
    v: &Spanned<DeValue<'_>>,
) -> Option<ShowWhitespace> {
    let s = cx.raw_string(key, v)?;
    if matches!(s.as_str(), "ideographic" | "trailing" | "all") {
        cx.diag(
            Severity::Info,
            DiagKind::Unsupported,
            key.to_owned(),
            v.span().start,
        );
        cx.set_value(v);
        return None;
    }
    let w = ShowWhitespace::from_name(&s);
    if w.is_none() {
        cx.invalid(key, ShowWhitespace::NAMES, v);
    }
    w
}

/// `theme.name`: 組込みの名前か、テーマファイル (`<設定ディレクトリ>/themes/<name>.toml`) の名前。
/// ファイルが有るかは読むときに見る。
fn read_theme_name(cx: &mut Cx<'_>, key: &str, v: &Spanned<DeValue<'_>>) -> Option<String> {
    let s = cx.raw_string(key, v)?;
    if is_theme_name(&s) {
        Some(s)
    } else {
        cx.invalid(key, THEME_NAMES, v);
        None
    }
}

/// `ui.locale`: 言語タグか `auto` (`Some(None)`)。
fn read_locale(
    cx: &mut Cx<'_>,
    key: &str,
    v: &Spanned<DeValue<'_>>,
) -> Option<Option<crate::Locale>> {
    let s = cx.raw_string(key, v)?;
    if s.trim().eq_ignore_ascii_case("auto") {
        Some(None)
    } else if let Some(l) = crate::Locale::parse(&s) {
        Some(Some(l))
    } else {
        cx.invalid(key, LOCALES, v);
        None
    }
}

/// `file.default_encoding` の値。正規名 (`DefaultEncoding::NAMES`) のほか、WHATWG の別名ラベルを正規名に寄せる
/// (docs/07 §5.2)。大文字小文字は区別しない。
fn encoding(s: &str) -> Option<DefaultEncoding> {
    let s = s.to_ascii_lowercase();
    DefaultEncoding::from_name(&s).or(match s.as_str() {
        "utf8" | "unicode-1-1-utf-8" => Some(DefaultEncoding::Utf8),
        "utf-16" | "ucs-2" | "unicode" | "csunicode" => Some(DefaultEncoding::Utf16Le),
        "unicodefffe" => Some(DefaultEncoding::Utf16Be),
        "shift-jis" | "sjis" | "windows-31j" | "ms_kanji" | "csshiftjis" | "x-sjis" | "ms932"
        | "cp932" => Some(DefaultEncoding::ShiftJis),
        "x-euc-jp" | "cseucpkdfmtjapanese" => Some(DefaultEncoding::EucJp),
        "csiso2022jp" => Some(DefaultEncoding::Iso2022Jp),
        _ => None,
    })
}

/// 公開前のキー (その下の入れ子のキーを含む)。
fn is_reserved_key(full: &str) -> bool {
    let prefix = format!("{full}.");
    crate::reserved::RESERVED_KEYS
        .iter()
        .any(|r| *r == full || r.starts_with(&prefix))
}

/// 公開キーの既定値の表記 (診断の「既定値 {default} を使います」に入れる)。宣言の表の既定値の TOML の表記から
/// 引用符を除いたもの。未知のキーは空。
pub fn default_display(key: &str) -> &'static str {
    static DEFAULTS: OnceLock<Vec<(&'static str, String)>> = OnceLock::new();
    DEFAULTS
        .get_or_init(|| declared_values(&Config::default()))
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("", |(_, v)| v.trim_matches('"'))
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
