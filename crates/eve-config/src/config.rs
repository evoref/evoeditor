//! ユーザー層の設定ファイル (`config.toml`) の読込と検証 (docs/07 §4.2・§5.0、01 FR-701〜703)。
//!
//! 公開キーは宣言の表 (`config_keys!`、07 §4.1) で決め、`Config`・既定値・読込・`--dump-config` の表記をそこから生成する。
//! ファイルが無い・壊れている・値が不正なときも `Config` は必ず返り、問題は `Diagnostic` の列として返す
//! (最初のエラーで止まらない)。不正な項目は既定値に戻す。

use std::sync::OnceLock;

use toml::Spanned;
use toml::de::{DeString, DeTable, DeValue};

/// 診断の重さ。表示の文言は host / core が `MessageKey` で決める (docs/07 §4.3)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// 情報。動作に影響しない (後の版の設定を読み飛ばした、など)。
    Info,
    /// 警告。該当の項目だけを既定値 (または下の層の値) に戻す。
    Warning,
    /// エラー。構文エラーや読めないファイルなど、そのファイル全体を使えない。`--check-config` の終了コード 3 と
    /// `config.reload` で前の設定を維持する判定の対象 (07 §4.3・§18.4)。
    Error,
}

/// 診断の種類 (docs/07 §4.2 の表)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagKind {
    /// TOML の構文エラー。ファイル全体を無視する。
    Syntax {
        /// TOML パーサの説明文。
        message: String,
    },
    /// 書ける構造の上限 (インラインの表・配列の入れ子と、キーの段数) を超えた (07 §4.2)。構文を読む前に見つけ、
    /// 構文エラーと同じくファイル全体を無視する (エラー)。文言は構文エラーと同じで、理由の語が違う。
    Structure(crate::StructureProblem),
    /// 未知のキー・セクション。`suggestion` は編集距離 2 以下の公開キー。
    UnknownKey {
        /// 編集距離 2 以下の公開キー。近いものが無ければ `None`。
        suggestion: Option<String>,
    },
    /// 後の版で公開する設定 (このバージョンでは未対応)。情報。画面には出さず、件数にも数えない (07 §4.3)。
    Unsupported,
    /// 公開しない、常に有効の機能のキー (`editor.multi_cursor`。複数カーソルは常に使える、07 §5.1)。情報。値は使わない。
    MultiCursorAlways,
    /// 型が違う。
    TypeMismatch {
        /// 期待した型の名前 (`integer` など)。
        expected: &'static str,
        /// 書かれていた型の名前。
        found: &'static str,
    },
    /// 範囲外 (整数)。
    OutOfRange {
        /// 許容する最小値 (この値を含む)。
        min: i64,
        /// 許容する最大値 (この値を含む)。
        max: i64,
    },
    /// 列挙値が不正。許容値を提示する。
    InvalidValue {
        /// 許容する値 (診断に出す順)。`ui.locale` では、`locales/` にある言語を含む (07 §17.4) ので実行時に作る。
        allowed: Vec<String>,
    },
    /// キーマップ (`keymap.toml`) の問題。`Diagnostic.key` は書かれたキー、`value` は書かれたコマンドなど。
    Keymap(KeymapProblem),
    /// テーマが無い (組込みの名前でも `themes/<name>.toml` でもない。07 §8.6 / §8.7)。エラーで、テーマは組込みの
    /// `default` で描く。`Diagnostic.key` は `theme.name` か `inherits`、`value` は書かれた名前。位置は名前を書いた所。
    ThemeNotFound {
        /// 組込みの名前に近い名前 (`theme::near_builtin_names`)。無ければ空。
        candidates: Vec<&'static str>,
    },
    /// ユーザーの言語ファイル (`locales/<lang>.toml`) の問題 (07 §17.4)。`Diagnostic.key` は平らにした文言のキー
    /// (ファイル全体の問題では空)。値が文字列でない文言は `TypeMismatch` (期待する型は `string`) で表す。
    Locale(crate::LocaleProblem),
    /// 条件付きセクションの書き方の誤り (07 §3.4 の COND-5 / COND-6。警告)。そのセクション (と、その下のキー) を使わない。
    /// `Diagnostic.key` は誤りの所までの名前 (`os.win`・`os.windows.env.ssh`・`os.windows.tab_width` など)。
    Condition(ConditionProblem),
    /// 言語定義 (`[[language]]`・`languages/*.toml`) の誤り (07 §9.6 の LANG-2〜LANG-6。警告)。その項目 (名前の誤りでは
    /// その定義) を使わない。`Diagnostic.key` は `language.<項目>`。
    Language(crate::language::LanguageProblem),
    /// 配列の要素の誤り (`workspace.root_markers`・`workspace.safe_directories`、07 §5.9)。`Diagnostic.key` は公開キー、
    /// `value` は書かれた要素。広すぎる安全な場所 (`TooBroad`) は警告だけで使い、ほかはその要素だけを使わない。
    ListItem(ListItemProblem),
    /// ワークスペース層 (`.eve/config.toml`・`.eve/keymap.toml`) で使わなかった項目・割当と、読まなかったファイル
    /// (07 §12.5 の WSC-4、§7.4 の WKEY-2、§12.1 の OWN-7)。
    Workspace(crate::workspace::WorkspaceProblem),
    /// 1 つのファイルの診断が上限 ([`CONFIG_MAX_DIAGNOSTICS`](crate::CONFIG_MAX_DIAGNOSTICS)) を超えた: 超えた分は残さず、
    /// この 1 件にまとめる (警告。`msg.config.more`)。要約と `--check-config` の件数は `count` 件と数える。
    More {
        /// 省いた件数。
        count: usize,
    },
}

/// 配列の要素の誤りの種類 (07 §5.9)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListItemProblem {
    /// 空の要素。
    Empty,
    /// 名前でない (`/`・`\` を含む・`.`・`..`。`workspace.root_markers`)。
    NotName,
    /// 絶対パスでない (`workspace.safe_directories`。`~` も展開しない)。
    Relative,
    /// `*` を含む (`workspace.safe_directories`。パターンは書けない)。
    Wildcard,
    /// 広すぎる安全な場所 (`workspace.safe_directories`。警告して使う): ファイルシステムのルート・ドライブの直下・UNC の共有の
    /// 直下は読むときにここで見つける。ホームディレクトリそのものは、環境を知る eve-core がワークスペース層を読む前に足す。
    TooBroad,
}

impl ListItemProblem {
    /// 理由の語の名前 (`ui.config.item_reason.<これ>`)。
    #[must_use]
    pub fn reason_name(self) -> &'static str {
        match self {
            ListItemProblem::Empty => "empty",
            ListItemProblem::NotName => "not_name",
            ListItemProblem::Relative => "relative",
            ListItemProblem::Wildcard => "wildcard",
            ListItemProblem::TooBroad => "too_broad",
        }
    }
}

/// 条件付きセクションの誤りの種類 (07 §3.4 の COND-5 / COND-6)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionProblem {
    /// 知らない条件の名前 (`[os.win]`。COND-5 (a))。
    Unknown {
        /// 編集距離 2 以下の条件の名前。近いものが無ければ `None`。
        suggestion: Option<&'static str>,
    },
    /// 条件の入れ子 (`[os.windows.env.ssh]`。COND-5 (b))。
    Nested,
    /// 条件の直下のキー (`[os.windows]` の `tab_width = 2`。COND-5 (c))。
    BareKey,
    /// 条件の中の言語定義 (`[[os.windows.language]]`) と `[keys]` (COND-5 (d))。
    UnsupportedTable,
    /// `keymap.toml` の条件付きセクション (COND-6)。キーマップは条件付きセクションを読まない。
    InKeymap,
}

impl ConditionProblem {
    /// 文言のキーの名前 (`msg.config.<これ>`) と、ログの種類の名前。
    #[must_use]
    pub fn message_name(&self) -> &'static str {
        match self {
            ConditionProblem::Unknown { .. } => "condition_unknown",
            ConditionProblem::Nested => "condition_nested",
            ConditionProblem::BareKey => "condition_bare_key",
            ConditionProblem::UnsupportedTable => "condition_unsupported_table",
            ConditionProblem::InKeymap => "condition_in_keymap",
        }
    }
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
    /// ユーザーの割当が、ユーザーの別のチョードの先頭と同じ (プレフィックスの衝突、07 §7.6): チョード側を優先し、この割当は
    /// 使わない。`Diagnostic.key` は使わない割当、`value` はそれを無効にしたチョードのキー (書かれたとおり)。
    UserChordPrefix,
    /// ユーザーのチョードが、既定の層の単体の割当を無効にした (プレフィックスの衝突、07 §7.6): チョード側を優先する。
    /// `Diagnostic.key` はチョード、`value` は無効になった既定の割当 (`コンテキスト: キー = コマンド`)。
    ChordShadowsDefault,
    /// 同じコンテキストの同じキーを、別の書き方で別の行にも書いた (`ctrl+s` と `mod+s` など。07 §7.6): 後の行を使う
    /// (情報)。`Diagnostic.key` は後の行、`value` は先の行のキー (書かれたとおり)。
    DuplicateKey,
    /// `args` がコマンドの引数に合わない (知らない名前・型の違い・使えない値・必須の引数の不足、07 §7.6): この割当は
    /// 使わない。`Diagnostic.value` は合わない引数の名前 (必須の引数の不足ではその名前)。
    BadArgs,
    /// ユーザーの割当が、一部のフォーカスでだけチョードの先頭と同じ (プレフィックスの衝突、07 §7.6): そのチョードの
    /// コンテキストが効くフォーカスではチョードを優先し、ほかのフォーカスでは使う。`Diagnostic.value` はチョードの
    /// `コンテキスト: キー`。
    ChordPrefixPartly,
    /// ユーザーのチョードが、既定の層の単体の割当を、一部のフォーカスでだけ使えなくした (07 §7.6): ほかのフォーカスでは
    /// 既定の割当が効く。`Diagnostic.key` はチョード、`value` は既定の割当 (`コンテキスト: キー = コマンド`)。
    ChordShadowsDefaultPartly,
}

/// 設定の問題 1 件。`line` / `column` は 1 始まり (構文エラーで位置が無いときは 0)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// 重さ。
    pub severity: Severity,
    /// 種類と、その詳細。
    pub kind: DiagKind,
    /// ドット区切りのキー (`editor.tab_width`)。構文エラーでは空。
    pub key: String,
    /// 書かれた値 (文字列はそのまま、整数は書かれた表記。値を持たない診断では空)。
    pub value: String,
    /// 行 (1 始まり。位置が無いときは 0)。
    pub line: u32,
    /// 桁 (1 始まりの文字数。位置が無いときは 0)。
    pub column: u32,
    /// 条件付きセクション (07 §3.4) の中で見つけた診断の、そのセクションの条件 (偽の条件のものも。COND-4)。
    /// 本文に `[os.linux]` のように添える。
    pub condition: Option<crate::Condition>,
}

/// 公開キーの値を使った所 (1 つのファイルの中。`config.show` の由来、07 §3.5)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// 公開キー。
    pub key: &'static str,
    /// キーを書いた行 (1 始まり)。
    pub line: u32,
    /// キーを書いた桁 (1 始まりの文字数)。
    pub column: u32,
    /// 条件付きセクションの値なら、その条件。
    pub condition: Option<crate::Condition>,
    /// 書かれた文字列 (文字列でない値は空)。
    pub text: String,
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
        format!("\"{}\"", self.as_ref().map_or("auto", crate::Locale::code))
    }
}

impl ConfigValue for Vec<String> {
    fn toml(&self) -> String {
        let items: Vec<String> = self.iter().map(|s| toml_string(s)).collect();
        format!("[{}]", items.join(", "))
    }
}

/// TOML の基本文字列 (`"…"`。JSON の文字列としても読める形)。
pub(crate) fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// ワークスペース層の「小さくする方向のみ」の比較 (07 §12.3)。大小を持つ値 (整数) だけが `Some` を返す。
pub(crate) trait Magnitude {
    /// `self` が `other` 以下か。大小を持たない値は `None`。
    fn at_most(&self, _other: &Self) -> Option<bool> {
        None
    }
}

macro_rules! ordered_magnitude {
    ($($ty:ty),*) => {
        $(impl Magnitude for $ty {
            fn at_most(&self, other: &Self) -> Option<bool> {
                Some(self <= other)
            }
        })*
    };
}
ordered_magnitude!(u8, u16, u32, u64);

macro_rules! unordered_magnitude {
    ($($ty:ty),*) => {
        $(impl Magnitude for $ty {})*
    };
}
unordered_magnitude!(bool, String, Vec<String>, Option<crate::Locale>);

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

            /// 設定ファイルに書く値。
            #[must_use]
            pub fn name(self) -> &'static str {
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

        impl Magnitude for $ty {}
    )*};
}

config_enum! {
    /// インデントの操作で入れる単位 (`editor.indent_unit`、07 §5.1)。
    IndentUnit {
        /// タブ文字。
        Tab = "tab",
        /// スペース (幅は `editor.indent_width`)。
        Space = "space",
    }

    /// 検索の大文字小文字の区別の既定 (`search.case`、03 §12.1、07 §5.5)。
    SearchCase {
        /// 検索語に大文字を含めば区別し、含まなければ区別しない (既定)。
        Smart = "smart",
        /// いつも区別する。
        Sensitive = "sensitive",
        /// いつも区別しない。
        Insensitive = "insensitive",
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
        /// 暗い背景として塗る。
        Dark = "dark",
        /// 明るい背景として塗る。
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
        /// 揃えない (継続行は左端から)。
        None = "none",
        /// 論理行のインデント幅で揃える (既定)。
        Same = "same",
    }

    /// タブラインを出すか (`ui.tab_line`、07 §5.3、03 §9.9 の PANE-12)。
    TabLineMode {
        /// 開いている文書が 2 つ以上のとき (既定)。
        Auto = "auto",
        /// いつも出す (文書が 1 つでも)。
        Always = "always",
        /// 出さない。
        Never = "never",
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
        /// UTF-8 (BOM なし)。
        Utf8 = "utf-8",
        /// UTF-8 (BOM 付き)。
        Utf8Bom = "utf-8-bom",
        /// UTF-16 リトルエンディアン。
        Utf16Le = "utf-16le",
        /// UTF-16 ビッグエンディアン。
        Utf16Be = "utf-16be",
        /// `Shift_JIS`。
        ShiftJis = "shift_jis",
        /// EUC-JP。
        EucJp = "euc-jp",
        /// ISO-2022-JP。
        Iso2022Jp = "iso-2022-jp",
        /// OS ロケール由来のヒント (docs/05 §6)。
        System = "system",
    }

    /// 新規ファイルの改行 (`file.default_line_ending`、07 §5.2)。
    DefaultLineEnding {
        /// LF (改行文字 U+000A)。
        Lf = "lf",
        /// CRLF (U+000D U+000A)。
        Crlf = "crlf",
        /// CR (U+000D)。
        Cr = "cr",
        /// OS の既定 (Windows は CRLF、それ以外は LF)。
        Native = "native",
    }

    /// 色深度の判定の上書き (`theme.color_mode`、07 §5.4、04 §5.2a)。
    ColorMode {
        /// 環境変数と端末の申告から判定する (既定)。
        Auto = "auto",
        /// 24 ビット色。
        TrueColor = "truecolor",
        /// 256 色。
        Color256 = "256",
        /// 16 色。
        Color16 = "16",
        /// 色なし (各ロールの `mono` の段を使う、04 §12.4)。
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
        $cx.int($key, $v, $min, $max).map(|n| {
            <$ty>::try_from(n).expect("config_keys! の int(min, max) は型に収まる範囲で書く")
        })
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

/// 公開キーの宣言の表 (docs/07 §4.1): `フィールド: 型 = 既定値, "キー", 読み方, ワークスペース層の分類;`。
/// ここから `Config` とその `Default`、公開キーの一覧 `KEYS`、`changed_keys`、読込 (`Cx::value`)、
/// TOML の表記 (`declared_values`。`--dump-config` と `default_display` が使う)、ワークスペース層の分類
/// (`workspace_policy`・`classify_key`、07 §12.3 の「v0.5 の公開キーごとの分類」) と値の写し (`assign_key`) を生成する。
/// キーを足すときは、この表に 1 行足す (07 §5 の表・§12.3 の公開キーごとの表と `dist/config.example.toml` も。どれも試験が
/// 突合する)。
macro_rules! config_keys {
    ($(
        $(#[$doc:meta])*
        $field:ident: $ty:ty = $default:expr, $key:literal, $read:ident $(($($arg:expr),*))?,
            $policy:expr;
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

        /// 公開キー `key` のワークスペース層の分類の決め方 (07 §12.3 の「v0.5 の公開キーごとの分類」)。公開キーでなければ
        /// `None`。
        #[must_use]
        pub fn workspace_policy(key: &str) -> Option<crate::WorkspacePolicy> {
            match key {
                $( $key => Some($policy), )*
                _ => None,
            }
        }

        /// ワークスペース層に書かれた `key` の値 (`ws`) を、下の層の値 (`lower`) と比べて分類する (07 §12.3)。公開キーで
        /// なければ `None`。
        pub(crate) fn classify_key(
            key: &str,
            ws: &Config,
            lower: &Config,
        ) -> Option<crate::WorkspaceClass> {
            match key {
                $( $key => Some(crate::workspace::classify(
                    &$policy,
                    &ws.$field,
                    &lower.$field,
                )), )*
                _ => None,
            }
        }

        /// 公開キー `key` の値を `from` から `to` へ写す (ワークスペース層の安全な項目を重ねる、07 §12.5 の WSC-3)。
        pub(crate) fn assign_key(key: &str, from: &Config, to: &mut Config) {
            match key {
                $( $key => to.$field = from.$field.clone(), )*
                _ => {}
            }
        }

        impl Cx<'_> {
            /// 公開キー `key` (書いた所は `at`) の値を読む。誤りは診断を積んで、値を変えない (既定値または下の層の値のまま)。
            /// 検証だけのとき (偽の条件のセクション、07 §3.4 の COND-4) は、読んで診断を積むが値を使わない。
            fn value(&mut self, key: &str, at: usize, v: &Spanned<DeValue<'_>>) {
                match key {
                    $(
                        $key => {
                            if let Some(x) = read_value!(self, key, v, $ty, $read $(($($arg),*))?)
                                && self.apply
                            {
                                self.out.config.$field = x;
                                self.written($key, at, v);
                            }
                        }
                    )*
                    _ => {}
                }
            }
        }
    };
}

use crate::WorkspaceClass as C;
use crate::WorkspacePolicy as P;

config_keys! {
    /// `editor.tab_width`: タブ文字の表示幅。
    tab_width: u8 = 4, "editor.tab_width", int(1, 16),
        P::Safe;
    /// `editor.indent_unit`: インデントの操作で入れる単位。
    indent_unit: IndentUnit = IndentUnit::Space, "editor.indent_unit", choice,
        P::Safe;
    /// `editor.indent_width`: スペースのインデントの幅。
    indent_width: u8 = 4, "editor.indent_width", int(1, 16),
        P::Safe;
    /// `file.default_encoding`: 新規ファイル・判定不能時の文字コード。
    default_encoding: DefaultEncoding = DefaultEncoding::Utf8,
        "file.default_encoding", custom(read_encoding),
        P::Safe;
    /// `file.default_line_ending`: 新規ファイルの改行。
    default_line_ending: DefaultLineEnding = DefaultLineEnding::Lf,
        "file.default_line_ending", choice,
        P::Safe;
    /// `view.ambiguous_width`: East Asian Ambiguous の幅 (1 か 2)。
    ambiguous_width: u8 = 1, "view.ambiguous_width", int(1, 2),
        P::NoEffect;
    /// `theme.color_mode`: 色深度の判定の上書き。
    color_mode: ColorMode = ColorMode::Auto, "theme.color_mode", choice,
        P::NoEffect;
    /// `file.swap.enabled`: swap-lite を使うか (FR-118、docs/07 §5.2)。
    swap_enabled: bool = true, "file.swap.enabled", boolean,
        P::SafeValue { safe: "true", otherwise: C::SemiSafe };
    /// `file.swap.interval_ms`: 最後の入力からスワップを書くまでの遅延。
    swap_interval_ms: u32 = 2_000, "file.swap.interval_ms", int(100, 600_000),
        P::SemiSafeUnlessEqual;
    /// `file.swap.min_interval_ms`: 同じ文書のスワップの最短の間隔。
    swap_min_interval_ms: u32 = 10_000, "file.swap.min_interval_ms", int(100, 3_600_000),
        P::SemiSafeUnlessEqual;
    /// `file.swap.max_bytes`: これを超える文書はスワップの対象外。
    swap_max_bytes: u64 = 64 * 1024 * 1024, "file.swap.max_bytes", int(1_048_576, 1 << 40),
        P::SemiSafeUnlessEqual;
    /// `file.swap.exclude_secret_files`: 秘密ファイルの名前に一致する文書をスワップしない (OQ-109)。
    swap_exclude_secret_files: bool = true, "file.swap.exclude_secret_files", boolean,
        P::SafeValue { safe: "true", otherwise: C::Dangerous };
    /// `ui.locale`: UI の言語。`None` は `auto` (環境変数・OS ロケールで決める、docs/07 §17.1)。
    ui_locale: Option<crate::Locale> = None, "ui.locale", custom(read_locale),
        P::NoEffect;
    /// `editor.undo.total_max_bytes`: プロセス全体の Undo 履歴の上限 (NFR-203、docs/03 §7.5)。
    undo_total_max_bytes: u64 = 256 * 1024 * 1024,
        "editor.undo.total_max_bytes", int(1_048_576, 1 << 40),
        P::SemiSafe;
    /// `editor.backspace_unit`: Backspace の削除の単位 (FR-201)。
    backspace_unit: BackspaceUnit = BackspaceUnit::Grapheme, "editor.backspace_unit", choice,
        P::Safe;
    /// `view.wrap`: 折り返しの方式 (FR-506)。
    wrap: WrapMode = WrapMode::Off, "view.wrap", choice,
        P::Safe;
    /// `view.wrap_indent`: 継続行のインデント。
    wrap_indent: WrapIndent = WrapIndent::Same, "view.wrap_indent", choice,
        P::Safe;
    /// `view.wrap_kinsoku`: 禁則処理 (OQ-57)。
    wrap_kinsoku: bool = true, "view.wrap_kinsoku", boolean,
        P::Safe;
    /// `view.show_whitespace`: 空白の可視化。
    show_whitespace: ShowWhitespace = ShowWhitespace::Minimal,
        "view.show_whitespace", custom(read_show_whitespace),
        P::Safe;
    /// `theme.name`: テーマ。
    theme_name: String = "default".to_owned(), "theme.name", custom(read_theme_name),
        P::SemiSafe;
    /// `theme.background`: 背景を塗るテーマの明暗。
    theme_background: ThemeBackground = ThemeBackground::Auto, "theme.background", choice,
        P::SemiSafe;
    /// `ui.mouse`: マウスの入力を使うか (FR-307、04 §6.8)。
    mouse: bool = true, "ui.mouse", boolean,
        P::Safe;
    /// `ui.hint_bar`: 画面下のヒントバーを出すか (FR-504、04 §8.5)。`ui.toggle_hint_bar` で実行中に切り替えられる。
    hint_bar: bool = true, "ui.hint_bar", boolean,
        P::Safe;
    /// `ui.tab_line`: 画面の最上行のタブライン (FR-510、03 §9.9 の PANE-12)。`config.reload` で即時に効く。
    tab_line: TabLineMode = TabLineMode::Auto, "ui.tab_line", choice,
        P::Safe;
    /// `ui.mouse_wheel_lines`: ホイール 1 目盛りのスクロール量 (表示行)。
    mouse_wheel_lines: u8 = 3, "ui.mouse_wheel_lines", int(1, 20),
        P::Safe;
    /// `ui.which_key_delay_ms`: チョードの保留から which-key を出すまでの時間 (04 §7.4)。
    which_key_delay_ms: u16 = 500, "ui.which_key_delay_ms", int(0, 5_000),
        P::Safe;
    /// `clipboard.osc52`: OSC 52 の送出方針。
    osc52: Osc52Policy = Osc52Policy::Auto, "clipboard.osc52", choice,
        P::SemiSafe;
    /// `clipboard.osc52_max_bytes`: OSC 52 の本文 (base64) の上限。超えたら送らない (05 §8.4)。
    osc52_max_bytes: u32 = 100_000, "clipboard.osc52_max_bytes", int(1_000, 16_777_216),
        P::SemiSafe;
    /// `file.huge_file_warn_bytes`: これを超えるファイルは開く前に警告する (FR-127、docs/03 §11.2)。
    huge_file_warn_bytes: u64 = 512 * 1024 * 1024,
        "file.huge_file_warn_bytes", int(1_048_576, 1 << 40),
        P::SmallerSafe;
    /// `file.long_line_threshold`: これを超える行を長大行として扱う (NFR-106、docs/03 §11.5 / §14.4)。
    /// 下限は長大行で表示する先頭 (64 KiB)。
    long_line_threshold: u64 = 1024 * 1024, "file.long_line_threshold", int(65_536, 1 << 30),
        P::SmallerSafe;
    /// `file.reload_on_external_change`: 外部変更の扱い (FR-117)。
    reload_on_external_change: ExternalChangePolicy = ExternalChangePolicy::AutoIfUnmodified,
        "file.reload_on_external_change", choice,
        P::ExternalChange;
    /// `search.newline_sugar`: 正規表現の `\n` を改行 1 つ (LF・CRLF・単独の CR) に一致させる (docs/03 §12.3.1)。
    /// 検索バーを開いたときの値で効く。
    newline_sugar: bool = true, "search.newline_sugar", boolean,
        P::Safe;
    /// `search.cp932_compat_fold`: CP932 互換文字 (〜/～ など 8 組) を検索で同一視する (docs/03 §12.4.1)。
    /// 検索バーを開いたときの値で効く。
    cp932_compat_fold: bool = true, "search.cp932_compat_fold", boolean,
        P::Safe;
    /// `search.case`: 検索バーを開いたときの大文字小文字の区別 (docs/03 §12.1。`search.toggle_case` で巡回)。
    /// 検索バーを開いたときの値で効く。
    search_case: SearchCase = SearchCase::Smart, "search.case", choice,
        P::Safe;
    /// `search.regex_default`: 検索バーを開いたときに正規表現のモードにするか (docs/03 §12.1。`search.toggle_regex` で切り替え)。
    /// 検索バーを開いたときの値で効く。
    regex_default: bool = false, "search.regex_default", boolean,
        P::Safe;
    /// `search.max_matches`: 保持する一致の上限 (docs/03 §12.2)。検索バーを開いたときの値で効く。
    max_matches: u32 = 10_000, "search.max_matches", int(100, 1_000_000),
        P::Safe;
    /// `search.count_limit`: 一致の件数を数える上限 (FR-404、docs/03 §12.2)。超えたら打ち切って「N+」と出す。
    /// 検索バーを開いたときの値で効く。
    count_limit: u64 = 1_000_000, "search.count_limit", int(1_000, 1_000_000_000),
        P::Safe;
    /// `search.workspace.respect_gitignore`: 横断検索・ファインダで `.gitignore` と `info/exclude` を効かせる
    /// (docs/03 §12.7.3。v0.5 の M33 の段 5 で公開)。次に走査・横断検索を始めたときから効く。
    ws_respect_gitignore: bool = true, "search.workspace.respect_gitignore", boolean,
        P::SafeValue { safe: "true", otherwise: C::SemiSafe };
    /// `search.workspace.follow_symlinks`: 走査がシンボリックリンク (ジャンクションを含む) の先へ降りる (docs/03 §12.7.2)。
    ws_follow_symlinks: bool = false, "search.workspace.follow_symlinks", boolean,
        P::SafeValue { safe: "false", otherwise: C::Dangerous };
    /// `search.workspace.max_file_size`: 横断検索で探すファイルの大きさの上限 (docs/03 §12.7.5)。
    ws_max_file_size: u64 = 2 * 1024 * 1024, "search.workspace.max_file_size", int(1_024, 1 << 30),
        P::SmallerSafe;
    /// `workspace.root_markers`: ルートの検出の目印 (docs/07 §12.1)。起動時にだけ使う。
    root_markers: Vec<String> = crate::workspace::default_root_markers(),
        "workspace.root_markers", custom(read_root_markers),
        P::NoEffect;
    /// `workspace.exclude`: ファインダ・横断検索の除外 (gitignore の行、docs/07 §5.9)。ワークスペース層の値は下の層に足す。
    exclude: Vec<String> = crate::workspace::default_exclude(),
        "workspace.exclude", custom(read_string_list),
        P::SafeAppend;
    /// `workspace.safe_directories`: 所有者の検査を掛けないディレクトリ (絶対パス、docs/07 §12.1 の OWN-5)。ユーザー層と
    /// `--set` だけが読む。
    safe_directories: Vec<String> = Vec::new(),
        "workspace.safe_directories", custom(read_safe_directories),
        P::AlwaysIgnored;
}

/// 読込の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// 読み込んだ設定。不正な項目は既定値 (または下の層の値) のまま。
    pub config: Config,
    /// 見つかった問題 (表を読んだ順。条件付きセクションの中のものも、条件の無い表と同じ順)。
    pub diagnostics: Vec<Diagnostic>,
    /// 値を使った公開キーと、書いた所 (重ねた順。同じキーは後のものが勝った値)。
    pub written: Vec<Written>,
    /// `[[language]]` の定義 (書いた順。誤りの項目を除く。`source` は空で、層を重ねる [`resolve`](crate::resolve) が
    /// ファイルの表示名を入れる。07 §9.6 の LANG-2)。
    pub languages: Vec<crate::LanguageDef>,
}

/// 公開キーを持つセクション (公開キーの先頭の名前。並びは `--dump-config` の出力の順、07 §5 の表の順)。
const SECTIONS: &[&str] = &[
    "editor",
    "file",
    "view",
    "theme",
    "ui",
    "search",
    "clipboard",
    "workspace",
];

/// 配列の公開キー (`<キー>+ = [...]` で下の層の配列に足せる、07 §3.3)。
const LIST_KEYS: &[&str] = &[
    "workspace.root_markers",
    "workspace.exclude",
    "workspace.safe_directories",
];

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
/// 構文エラーなら `base` のまま返す。条件付きセクションは既定の条件 ([`Conditions::default`](crate::Conditions)) で解く。
pub fn load_str_onto(base: Config, text: &str) -> Loaded {
    load_str_for(base, text, crate::Conditions::default())
}

/// `load_str_onto` と同じだが、条件付きセクション (07 §3.4) を `conditions` で解く。1 つのファイルの中で、条件の無い表、
/// 真の条件のセクション (COND-3 の順) の順にキーごとに重ねる。偽の条件のセクションも検証して診断を出す (COND-4)。
pub fn load_str_for(base: Config, text: &str, conditions: crate::Conditions) -> Loaded {
    load_str_in(base, text, conditions, None)
}

/// `load_str_for` と同じだが、ホームディレクトリ `home` を知っている (`workspace.safe_directories` にホームディレクトリ
/// そのものを書いたら、広すぎる場所の警告を出す、07 §12.1 の OWN-5)。
pub(crate) fn load_str_in(
    base: Config,
    text: &str,
    conditions: crate::Conditions,
    home: Option<&str>,
) -> Loaded {
    let mut cx = Cx {
        text,
        out: Loaded {
            config: base,
            diagnostics: Vec::new(),
            written: Vec::new(),
            languages: Vec::new(),
        },
        conditions,
        apply: true,
        condition: None,
        lines: crate::position::LineIndex::new(text),
        dropped: 0,
        stored: false,
        home,
    };
    // 構文を読む前に、深い入れ子を作る書き方を拒む (07 §4.2。パーサがスタックを使い切らないように)
    if let Some(d) = crate::toml_scan::structure_diagnostic(text) {
        cx.out.diagnostics.push(d);
        return cx.out;
    }
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
                condition: None,
            });
            return cx.out;
        }
    };
    // 表を読む順に診断を積み (条件付きセクションは検証だけ)、真の条件のセクションを COND-3 の順に重ねる
    let mut active: Vec<(crate::Condition, &DeTable<'_>)> = Vec::new();
    for (name, value) in root.get_ref() {
        if crate::reserved::CONDITIONAL_SECTIONS.contains(&name.get_ref().as_ref()) {
            cx.conditional(name, value, &mut active);
        } else {
            cx.section(name, value);
        }
    }
    active.sort_by_key(|(c, _)| *c);
    for (c, table) in active {
        // 診断は検証のときに積んだので、重ねるときの分は捨てる
        let (seen, dropped) = (cx.out.diagnostics.len(), cx.dropped);
        cx.scoped(true, Some(c), |cx| cx.under_condition(c.name(), table));
        cx.out.diagnostics.truncate(seen);
        cx.dropped = dropped;
    }
    cx.finish()
}

struct Cx<'a> {
    text: &'a str,
    out: Loaded,
    /// 条件の値 (07 §3.4 の COND-2)。
    conditions: crate::Conditions,
    /// 値を使うか (偽: 検証だけ。偽の条件のセクションと、真の条件のセクションの 1 回目、COND-4)。
    apply: bool,
    /// 読んでいるセクションの条件 (条件付きセクションの中のとき)。診断と由来に記録する。
    condition: Option<crate::Condition>,
    /// 行の始まりの表 (位置を引く)。
    lines: crate::position::LineIndex<'a>,
    /// 上限を超えて残さなかった診断の数。
    dropped: usize,
    /// 直前の診断を残したか (書かれた値を記録する先)。
    stored: bool,
    /// ホームディレクトリ (広すぎる安全な場所の判定。知らなければ `None`)。
    home: Option<&'a str>,
}

impl Cx<'_> {
    /// バイト位置 → (行, 桁)。どちらも 1 始まり、桁は文字数。
    fn pos(&self, offset: usize) -> (u32, u32) {
        self.lines.pos(offset)
    }

    /// 結果: 上限を超えた診断があれば「ほか N 件」を足す。
    fn finish(mut self) -> Loaded {
        if self.dropped > 0 {
            self.out
                .diagnostics
                .push(crate::position::more(self.dropped));
        }
        self.out
    }

    fn diag(&mut self, severity: Severity, kind: DiagKind, key: String, at: usize) {
        // 上限を超えた分は位置も引かずに数えるだけ (診断の多いファイルで io のジョブを止めない)
        self.stored = self.out.diagnostics.len() < crate::CONFIG_MAX_DIAGNOSTICS;
        if !self.stored {
            self.dropped += 1;
            return;
        }
        let (line, column) = self.pos(at);
        self.out.diagnostics.push(Diagnostic {
            severity,
            kind,
            key,
            value: String::new(),
            line,
            column,
            condition: self.condition,
        });
    }

    /// 値を使った所を記録する (`config.show` の由来、07 §3.5)。
    fn written(&mut self, key: &'static str, at: usize, v: &Spanned<DeValue<'_>>) {
        let (line, column) = self.pos(at);
        let text = match v.get_ref() {
            DeValue::String(s) => s.to_string(),
            _ => String::new(),
        };
        self.out.written.push(Written {
            key,
            line,
            column,
            condition: self.condition,
            text,
        });
    }

    /// 条件の種類の表 (`[os.*]`・`[frontend.*]`・`[env.*]`・`[host.*]`) を検証し、真の条件のセクションを `active` に足す
    /// (07 §3.4 の COND-4 / COND-5)。中のキーは、値を使わずに条件の無い表と同じ検証を掛ける。
    fn conditional<'t>(
        &mut self,
        name: &Spanned<DeString<'_>>,
        value: &'t Spanned<DeValue<'t>>,
        active: &mut Vec<(crate::Condition, &'t DeTable<'t>)>,
    ) {
        let kind: &str = name.get_ref();
        // `host.*` は v0.5 では解決しない: v0.4 までと同じく、条件の名前ごとに 1 件の情報診断 (COND-5 (e))
        if !crate::condition::RESOLVED_KINDS.contains(&kind) {
            self.diag(
                Severity::Info,
                DiagKind::Unsupported,
                kind.to_owned(),
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
                kind.to_owned(),
                value.span().start,
            );
            self.set_value(value);
            return;
        };
        for (sub, inner) in table {
            let full = format!("{kind}.{}", sub.get_ref());
            let at = sub.span().start;
            if crate::condition::UNRESOLVED_NAMES.contains(&full.as_str()) {
                self.diag(Severity::Info, DiagKind::Unsupported, full, at);
                continue;
            }
            let Some(c) = crate::Condition::from_parts(kind, sub.get_ref()) else {
                let suggestion = near_condition(&full);
                self.diag(
                    Severity::Warning,
                    DiagKind::Condition(ConditionProblem::Unknown { suggestion }),
                    full,
                    at,
                );
                continue;
            };
            let DeValue::Table(t) = inner.get_ref() else {
                self.diag(
                    Severity::Warning,
                    DiagKind::Condition(ConditionProblem::BareKey),
                    full,
                    at,
                );
                continue;
            };
            // 検証だけ (値は使わない)。真の条件のセクションは、後で COND-3 の順に重ねる
            self.scoped(false, Some(c), |cx| cx.under_condition(&full, t));
            if self.conditions.holds(c) {
                active.push((c, t));
            }
        }
    }

    /// 条件 `cond` (`os.windows` など) の下の表を読む。中はセクション (`[os.windows.editor]`) だけで、入れ子の条件・直下の
    /// キー・言語定義と `[keys]` は誤り (COND-5 (b)〜(d))。誤りのセクションは使わない。
    fn under_condition(&mut self, cond: &str, table: &DeTable<'_>) {
        for (name, value) in table {
            let n: &str = name.get_ref();
            let full = format!("{cond}.{n}");
            let at = name.span().start;
            let problem = if crate::reserved::CONDITIONAL_SECTIONS.contains(&n) {
                Some(ConditionProblem::Nested)
            } else if !matches!(value.get_ref(), DeValue::Table(_)) && !is_table_array(value) {
                Some(ConditionProblem::BareKey)
            } else if matches!(n, "language" | "keys") {
                Some(ConditionProblem::UnsupportedTable)
            } else {
                None
            };
            let Some(p) = problem else {
                self.section(name, value);
                continue;
            };
            // 入れ子は、入れ子の条件の名前ごとに 1 件 (`os.windows.env.ssh` と `os.windows.env.tmux` で 2 件)。位置はその名前
            let places: Vec<(String, usize)> = match (&p, value.get_ref()) {
                (ConditionProblem::Nested, DeValue::Table(t)) if !t.is_empty() => t
                    .iter()
                    .map(|(k, _)| (format!("{full}.{}", k.get_ref()), k.span().start))
                    .collect(),
                _ => vec![(full, at)],
            };
            // 名前に条件を含むので、条件は添えない
            self.scoped(self.apply, None, |cx| {
                for (key, at) in places {
                    cx.diag(Severity::Warning, DiagKind::Condition(p.clone()), key, at);
                }
            });
        }
    }

    /// 値を使うか (`apply`) と読んでいるセクションの条件 (`condition`) を `f` の間だけ替え、後で元に戻す。
    fn scoped(
        &mut self,
        apply: bool,
        condition: Option<crate::Condition>,
        f: impl FnOnce(&mut Self),
    ) {
        let saved = (self.apply, self.condition);
        (self.apply, self.condition) = (apply, condition);
        f(self);
        (self.apply, self.condition) = saved;
    }

    fn section(&mut self, name: &Spanned<DeString<'_>>, value: &Spanned<DeValue<'_>>) {
        let n: &str = name.get_ref();
        // 言語定義 (07 §9.6)。条件の中の `[[os.windows.language]]` は、呼ぶ前に誤り (COND-5 (d)) にしている
        if n == crate::reserved::LANGUAGE_SECTION {
            self.languages(value);
            return;
        }
        // 公開前のセクションは、セクションごとに未対応の情報診断にする
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
                self.value(&full, k.span().start, v);
            } else if let Some(base) = full
                .strip_suffix('+')
                .and_then(|b| LIST_KEYS.iter().find(|l| **l == b))
            {
                self.append(base, k.span().start, v);
            } else if let (DeValue::Table(inner), true) =
                (v.get_ref(), KEYS.iter().any(|key| key.starts_with(&nested)))
            {
                self.keys(&full, inner);
            } else if crate::reserved::ALWAYS_ON_KEYS.contains(&full.as_str()) {
                self.diag(
                    Severity::Info,
                    DiagKind::MultiCursorAlways,
                    full,
                    k.span().start,
                );
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

    fn invalid(&mut self, key: &str, allowed: &[&str], v: &Spanned<DeValue<'_>>) {
        self.diag(
            Severity::Warning,
            DiagKind::InvalidValue {
                allowed: allowed.iter().map(|a| (*a).to_owned()).collect(),
            },
            key.to_owned(),
            v.span().start,
        );
        self.set_value(v);
    }

    /// 値の書かれた範囲の原文 (前後の空白を除く)。範囲が原文に収まらなければ `None`。
    fn source_of(&self, v: &Spanned<DeValue<'_>>) -> Option<&str> {
        self.text.get(v.span()).map(str::trim)
    }

    /// 直前に積んだ診断に、書かれた値を記録する。
    fn set_value(&mut self, v: &Spanned<DeValue<'_>>) {
        let text = match v.get_ref() {
            DeValue::String(s) => s.to_string(),
            // 数は書いたままの表記 (`0x11`・`1_000`・`+5`) を記録する。toml の `as_str` は接頭辞と区切りを落とす
            DeValue::Integer(i) => self.source_of(v).unwrap_or(i.as_str()).to_owned(),
            DeValue::Float(f) => self.source_of(v).unwrap_or(f.as_str()).to_owned(),
            DeValue::Boolean(b) => b.to_string(),
            other => other.type_str().to_owned(),
        };
        if !self.stored {
            return;
        }
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
        cx.invalid(key, crate::theme::BUILTIN_NAMES, v);
        None
    }
}

/// `ui.locale`: 言語タグか `auto` (`Some(None)`。空の値も `auto` と同じ)。ここでは値の形だけを見る (07 §17.4)。使える言語
/// (同梱の言語と `locales/` の候補) との照合は、候補を知っている bin と `config.reload` が [`check_ui_locale`](crate::check_ui_locale)
/// で行う。
fn read_locale(
    cx: &mut Cx<'_>,
    key: &str,
    v: &Spanned<DeValue<'_>>,
) -> Option<Option<crate::Locale>> {
    let s = cx.raw_string(key, v)?;
    if s.trim().is_empty() || s.trim().eq_ignore_ascii_case("auto") {
        Some(None)
    } else if let Some(l) = crate::Locale::from_tag(&s) {
        Some(Some(l))
    } else {
        cx.invalid(key, LOCALES, v);
        None
    }
}

/// 文字列の配列。配列でなければ型の違い (値を使わない)。文字列でない要素は、その要素の型の違いの診断を出して捨てる。
fn read_string_list(cx: &mut Cx<'_>, key: &str, v: &Spanned<DeValue<'_>>) -> Option<Vec<String>> {
    let DeValue::Array(items) = v.get_ref() else {
        cx.mismatch(key, v, "array");
        return None;
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item.get_ref() {
            DeValue::String(s) => out.push(s.to_string()),
            _ => cx.mismatch(key, item, "string"),
        }
    }
    Some(out)
}

impl Cx<'_> {
    /// 配列の要素の誤り (その要素の位置に、書かれた要素を値として)。
    fn list_item(&mut self, key: &str, item: &Spanned<DeValue<'_>>, p: ListItemProblem) {
        self.diag(
            Severity::Warning,
            DiagKind::ListItem(p),
            key.to_owned(),
            item.span().start,
        );
        self.set_value(item);
    }

    /// 文字列の配列を読み、要素ごとに `check` で確かめる (`Err` の要素は診断を出して捨てる。`TooBroad` は診断を出して使う)。
    fn checked_list(
        &mut self,
        key: &str,
        v: &Spanned<DeValue<'_>>,
        check: impl Fn(&Self, &str) -> Result<(), ListItemProblem>,
    ) -> Option<Vec<String>> {
        let DeValue::Array(items) = v.get_ref() else {
            self.mismatch(key, v, "array");
            return None;
        };
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let DeValue::String(s) = item.get_ref() else {
                self.mismatch(key, item, "string");
                continue;
            };
            match check(self, s) {
                Ok(()) => out.push(s.to_string()),
                Err(ListItemProblem::TooBroad) => {
                    self.list_item(key, item, ListItemProblem::TooBroad);
                    out.push(s.to_string());
                }
                Err(p) => self.list_item(key, item, p),
            }
        }
        Some(out)
    }

    /// パスの形の規則に使う OS (07 §3.4 の COND-4): `os.*` のセクションはその OS (`os.unix` は Unix。偽の条件でも確かめる)。
    /// 条件の無い表と、OS の決まらないセクション (`frontend.*`・`env.*`) のうち今の環境で真のものは、この実行ファイルの OS
    /// (そのセクションの値を使うのはこの OS)。今の環境で偽の `frontend.*`・`env.*` は `None` (別の環境 (WSL の中など) で使う
    /// 値なので、今の OS の規則で誤りにしない)。対象の 3 つの OS でない実行ファイルでも `None`。
    fn path_rules(&self) -> Option<PathRules> {
        use crate::Condition as K;
        let current = || match self.conditions.os {
            Some(crate::Os::Windows) => Some(PathRules::Windows),
            Some(_) => Some(PathRules::Unix),
            None => None,
        };
        match self.condition {
            Some(K::OsWindows) => Some(PathRules::Windows),
            Some(K::OsUnix | K::OsMacos | K::OsLinux) => Some(PathRules::Unix),
            Some(c) if !self.conditions.holds(c) => None,
            _ => current(),
        }
    }

    /// 読んでいるセクションが今の環境で使われるか (条件の無い表か、真の条件のセクション)。
    fn condition_is_current(&self) -> bool {
        self.condition.is_none_or(|c| self.conditions.holds(c))
    }

    /// `<キー>+ = [...]`: 下の層 (同じファイルの前の値を含む) の配列に足す (07 §3.3)。
    fn append(&mut self, key: &'static str, at: usize, v: &Spanned<DeValue<'_>>) {
        let items = match key {
            "workspace.root_markers" => read_root_markers(self, key, v),
            "workspace.safe_directories" => read_safe_directories(self, key, v),
            _ => read_string_list(self, key, v),
        };
        let Some(items) = items else {
            return;
        };
        if !self.apply {
            return;
        }
        let list = match key {
            "workspace.root_markers" => &mut self.out.config.root_markers,
            "workspace.safe_directories" => &mut self.out.config.safe_directories,
            _ => &mut self.out.config.exclude,
        };
        list.extend(items);
        self.written(key, at, v);
    }
}

/// パスの形の規則 (`workspace.safe_directories` の絶対パスの判定)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathRules {
    Windows,
    Unix,
}

impl PathRules {
    /// 絶対パスか。Windows はドライブの絶対パス (`C:\`・`C:/`) と UNC (`\\server\share`・`//server/share`)、Unix は `/` で
    /// 始まるもの。
    fn is_absolute(self, s: &str) -> bool {
        match self {
            PathRules::Unix => s.starts_with('/'),
            PathRules::Windows => {
                let b = s.as_bytes();
                let drive = b.len() >= 3
                    && b[0].is_ascii_alphabetic()
                    && b[1] == b':'
                    && matches!(b[2], b'\\' | b'/');
                let unc = b.len() >= 3
                    && matches!(b[0], b'\\' | b'/')
                    && matches!(b[1], b'\\' | b'/')
                    && !matches!(b[2], b'\\' | b'/');
                drive || unc
            }
        }
    }

    /// `a` と `b` が同じパスか (区切りの違い・末尾の区切りを畳む。Windows は `\` と `/` を同じにし、ASCII の大文字小文字を
    /// 区別しない)。
    fn same_path(self, a: &str, b: &str) -> bool {
        let parts = |s: &str| -> Vec<String> {
            let sep = |c: char| c == '/' || (self == PathRules::Windows && c == '\\');
            s.split(sep)
                .filter(|p| !p.is_empty() && *p != ".")
                .map(|p| {
                    if self == PathRules::Windows {
                        p.to_ascii_lowercase()
                    } else {
                        p.to_owned()
                    }
                })
                .collect()
        };
        parts(a) == parts(b)
    }

    /// ファイルシステムのルート・ドライブの直下・UNC の共有の直下か (広すぎる安全な場所、07 §12.1 の OWN-5)。
    fn is_root(self, s: &str) -> bool {
        let sep = |c: char| c == '/' || (self == PathRules::Windows && c == '\\');
        let parts: Vec<&str> = s
            .split(sep)
            .filter(|p| !p.is_empty() && *p != ".")
            .collect();
        match self {
            PathRules::Unix => parts.is_empty(),
            PathRules::Windows if s.as_bytes().get(1) == Some(&b':') => parts.len() <= 1,
            // UNC: サーバと共有の 2 つの要素だけ
            PathRules::Windows => parts.len() <= 2,
        }
    }
}

/// `workspace.root_markers`: 名前の配列。区切り (`/`・`\`) を含む要素・空の要素・`.`・`..` は警告して捨てる (07 §5.9)。
fn read_root_markers(cx: &mut Cx<'_>, key: &str, v: &Spanned<DeValue<'_>>) -> Option<Vec<String>> {
    cx.checked_list(key, v, |_, s| {
        if s.is_empty() {
            Err(ListItemProblem::Empty)
        } else if s.contains(['/', '\\']) || s == "." || s == ".." {
            Err(ListItemProblem::NotName)
        } else {
            Ok(())
        }
    })
}

/// `workspace.safe_directories`: 絶対パスの配列。空の要素・`*` を含む要素・相対パスは警告して捨てる (`~` は展開しないので
/// 相対パス)。ファイルシステムのルート・ドライブの直下は警告して使う (07 §5.9、§12.1 の OWN-5)。パスの形はセクションの条件の
/// OS の規則で確かめる (07 §3.4 の COND-4)。
fn read_safe_directories(
    cx: &mut Cx<'_>,
    key: &str,
    v: &Spanned<DeValue<'_>>,
) -> Option<Vec<String>> {
    cx.checked_list(key, v, |cx, s| {
        if s.trim().is_empty() {
            return Err(ListItemProblem::Empty);
        }
        if s.contains('*') {
            return Err(ListItemProblem::Wildcard);
        }
        match cx.path_rules() {
            Some(r) if !r.is_absolute(s) => Err(ListItemProblem::Relative),
            Some(r) if r.is_root(s) => Err(ListItemProblem::TooBroad),
            // ホームディレクトリそのもの (ホームは今の OS のものなので、今の OS で使う値だけを比べる)
            Some(r)
                if cx.home.is_some_and(|h| {
                    cx.condition_is_current() && r.is_absolute(h) && r.same_path(s, h)
                }) =>
            {
                Err(ListItemProblem::TooBroad)
            }
            _ => Ok(()),
        }
    })
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

/// 知らない条件の名前 (`os.win`) に近い条件の名前 (07 §3.4 の COND-5 (a)): 書いた名前がその名前の接頭辞か、編集距離 2 以下
/// (どちらも大文字小文字を区別しない。テーマの候補 (`theme::near_builtin_names`) と同じ規則)。当たる名前が 2 つ以上なら、
/// 編集距離の小さいもの、同じなら重ねる順の先のもの。
fn near_condition(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    crate::Condition::ALL
        .into_iter()
        .map(crate::Condition::name)
        .map(|c| (distance(&lower, c), c))
        .filter(|(d, c)| c.starts_with(&lower) || *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// 表の配列 (`[[language]]`) か。
fn is_table_array(v: &Spanned<DeValue<'_>>) -> bool {
    matches!(v.get_ref(), DeValue::Array(items)
        if !items.is_empty() && items.iter().all(|i| matches!(i.get_ref(), DeValue::Table(_))))
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

pub(crate) fn distance(a: &str, b: &str) -> usize {
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

mod language;
pub use language::load_language_str;

#[cfg(test)]
mod tests;
