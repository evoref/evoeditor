//! `Frame`: core が frontend に渡す描画モデル (docs/02-architecture.md §8)。テキスト領域は **セルグリッドにレイアウト済み**、
//! UI 部品 (クローム・オーバーレイ) は意味的モデルにする (ADR-0008)。
//!
//! 現在あるのは編集面 (`PaneFrame`) とカーソル。クローム (ステータス・メッセージ・ヒント) とオーバーレイは、
//! それを生成する側と一緒に足す。
//!
//! **不変条件 INV-F1**: Frame に入るすべての外部由来文字列は無害化済みで、制御文字と不正バイトのエスケープ文字
//! (`U+10FF00..`) を含まない (可視記号に展開済み)。frontend は `Span` をそのまま出力してよい。

use crate::caps::CellSize;
use crate::events::MessageLevel;
use crate::ids::{Revision, ViewId, ViewRevision, WindowId};
use crate::input::KeyStroke;
use crate::query::FocusTarget;

/// 色。core が `Capabilities.color_depth` の段で選んだ値で、frontend は再縮退しない (02 §8.2)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Color {
    /// 端末の既定色 (既定の前景色・背景色)。
    #[default]
    Default,
    /// 24 ビット色 (赤・緑・青)。`ColorDepth::TrueColor` の段で使う。
    Rgb(u8, u8, u8),
    /// 端末のパレットの番号 (0〜15 は 16 色、16〜255 は 256 色の段)。
    Indexed(u8),
}

/// 文字の属性 (ビット集合)。キーの修飾 (`Modifiers`) とは別の型。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StyleMods(u8);

impl StyleMods {
    /// 属性なし (空の集合)。
    pub const NONE: StyleMods = StyleMods(0);
    /// 太字。
    pub const BOLD: StyleMods = StyleMods(1);
    /// 斜体。
    pub const ITALIC: StyleMods = StyleMods(2);
    /// 下線 (色は `Style::underline_color`。下線の種類は無い)。
    pub const UNDERLINE: StyleMods = StyleMods(4);
    /// 前景と背景の反転。
    pub const REVERSE: StyleMods = StyleMods(8);
    /// 淡色。
    pub const DIM: StyleMods = StyleMods(16);

    /// `other` のすべての属性を含むか。
    pub fn contains(self, other: StyleMods) -> bool {
        self.0 & other.0 == other.0
    }

    /// 属性が 1 つも無いか。
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for StyleMods {
    type Output = StyleMods;

    fn bitor(self, rhs: StyleMods) -> StyleMods {
        StyleMods(self.0 | rhs.0)
    }
}

/// 解決済みのスタイル (テーマ・選択・検索一致などを合成した結果)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    /// 前景色。
    pub fg: Color,
    /// 背景色。
    pub bg: Color,
    /// 文字の属性。
    pub mods: StyleMods,
    /// 下線の色。`None` なら端末の既定 (前景色) で引く。
    pub underline_color: Option<Color>,
}

impl Style {
    /// 端末の既定色・属性なし。
    pub const DEFAULT: Style = Style {
        fg: Color::Default,
        bg: Color::Default,
        mods: StyleMods::NONE,
        underline_color: None,
    };

    /// 既定色に属性だけを付けたもの。
    pub fn with_mods(mods: StyleMods) -> Style {
        Style {
            mods,
            ..Style::DEFAULT
        }
    }
}

/// 同じスタイルで描く文字列。幅 (`width`) は core が計算済みで、frontend は幅関数を持たない。
///
/// **契約 (編集面)**: 各 `Span` は 1 書記素、または含まれるすべての書記素の幅が同じもの (ASCII の連続など)。
/// frontend は `width` を書記素数で割ってセル位置を求める。幅の異なる書記素を 1 つの `Span` に混ぜない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// 描く文字列 (無害化済み、INV-F1)。
    pub text: String,
    /// 表示幅 (セル)。core が `view.ambiguous_width` を反映して計算した値。
    pub width: u16,
    /// 解決済みのスタイル。
    pub style: Style,
}

impl Span {
    /// 文字列・幅 (セル)・スタイルから作る。幅と文字列の整合は検査しない。
    pub fn new(text: impl Into<String>, width: u16, style: Style) -> Span {
        Span {
            text: text.into(),
            width,
            style,
        }
    }
}

/// セルの位置 (左上が (0, 0))。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellPos {
    /// 列 (セル、0 始まり)。
    pub col: u16,
    /// 行 (セル、0 始まり)。
    pub row: u16,
}

/// セルの矩形 (`Frame` の編集面全体の左上が原点)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellRect {
    /// 左端の列 (セル)。
    pub col: u16,
    /// 上端の行 (セル)。
    pub row: u16,
    /// 幅 (セル)。
    pub cols: u16,
    /// 高さ (セル)。
    pub rows: u16,
}

/// 行の由来。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowSource {
    /// 文書の行 (0 始まり)。`wrap_idx` は折り返し ON のときの行内の表示行 (v0.2。v0.1 は常に 0)。
    Line {
        /// 論理行の番号 (0 始まり)。
        line: u32,
        /// 論理行の中の表示行の番号 (0 始まり。折り返し OFF では常に 0)。
        wrap_idx: u32,
    },
    /// 文書の最後より後ろの行 (表示は空)。
    Virtual,
}

/// 可視の 1 行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderRow {
    /// この表示行が文書のどこに当たるか。
    pub source: RowSource,
    /// ガター (行番号など) の Span。幅の和は `PaneFrame.gutter_width`。
    pub gutter: Vec<Span>,
    /// 本文の Span (ガターの右から。折り返し・タブ展開・水平スクロールの適用後)。
    pub spans: Vec<Span>,
    /// 水平スクロールで左に続きがある (FR-506)。
    pub clipped_left: bool,
    /// 右に続きがある (FR-506)。
    pub clipped_right: bool,
    /// 幅危険書記素を含む (TUI は行単位で再出力する)。
    pub width_risky: bool,
}

/// 編集面 (View 1 つ分)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneFrame {
    /// この面が表示している View。
    pub view: ViewId,
    /// 面のセル矩形 (core がレイアウトした位置、02 §8.3)。
    pub rect: CellRect,
    /// 生成した時点の文書の `Revision` と View の `ViewRevision`。
    pub revision: (Revision, ViewRevision),
    /// ガターの幅 (セル)。本文はこの列から始まる。
    pub gutter_width: u16,
    /// 可視の表示行 (上から順)。
    pub rows: Vec<RenderRow>,
    /// 可視範囲にあるすべてのカーソルのセル位置 (`rect` の左上が原点)。
    pub cursors: Vec<CellPos>,
    /// 入力の行き先がこの面である (`Frame.focus` が `Pane(view)`)。
    pub focused: bool,
}

/// カーソルの形。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CursorShape {
    /// 縦棒。
    #[default]
    Bar,
    /// セル全体の箱。
    Block,
    /// 下線。
    Underline,
}

/// ハードウェアカーソル・IME 候補窓の位置 (Frame 全体のセル座標)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorPlacement {
    /// 置くセル。
    pub pos: CellPos,
    /// カーソルの形。
    pub shape: CursorShape,
}

/// 切り詰めてよい位置 (`CellText` が幅に収まらないとき)。`Never` 以外は 1 書記素 = 1 `Span` で渡す。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CutPolicy {
    /// 切り詰めない (入らなければ項目ごと落とす)。
    #[default]
    Never,
    /// 末尾を切って `…` を置く。
    End,
    /// 中間を切って `…` を置く (ファイル名の拡張子を残すなど、04 §8.7)。
    Middle,
}

/// クロームの文字列 (幅は core が計算済み。frontend は幅関数を持たない)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellText {
    /// 文言の Span (無害化済み・幅付き)。
    pub spans: Vec<Span>,
    /// `spans` の幅の和 (セル)。
    pub width: u16,
    /// 幅に収まらないときに切ってよい位置。
    pub cut: CutPolicy,
}

/// メッセージ行の 1 件 (NFR-602 の 2 部構成: `text` = 何が起きたか、`hint` = どうすればよいか)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// 重大度 (frontend は色分けに使う)。
    pub level: MessageLevel,
    /// 何が起きたか (解決済みの文言)。
    pub text: CellText,
    /// どうすればよいか (解決済みの文言)。無ければ `None`。
    pub hint: Option<CellText>,
}

/// メッセージ行のモデル (FR-505)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageModel {
    /// 今出しているメッセージ。無ければ `None`。
    pub current: Option<Message>,
    /// 未読の件数 (履歴の閲覧は v0.2)。
    pub unread: u16,
}

/// ステータスバーの項目の種類 (04 §8.7)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StatusKind {
    /// ファイル名 (無題は `無題-1` など)。
    Path,
    /// 変更マーカー。
    Dirty,
    /// 読み取り専用。
    ReadOnly,
    /// 読込中 (進捗)。
    Loading,
    /// 保存中 (保存ジョブの完了を待つ間だけ)。
    Saving,
    /// 外部変更 (外部での変更の確認で「無視」を選んでから、保存か再読込をするまで)。
    ExternalChange,
    /// 機能縮退 (長い行の近似表示など)。
    Degraded,
    /// 保留中のチョード (保留中だけ)。
    Chord,
    /// 選択の情報。
    Selection,
    /// カーソルの位置 (行・桁)。
    Position,
    /// 文字コードと BOM。
    Encoding,
    /// 改行コード。
    Eol,
}

/// ステータスバーの左詰め群・右詰め群のどちらに置くか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StatusAlign {
    /// 左詰めの群 (左端から並べる)。
    Left,
    /// 右詰めの群 (右端に寄せる)。
    Right,
}

/// ステータスバーの項目。長い形と短い形は core が言語ファイルから解決済みで、幅も計算済み。
/// frontend は優先度の低い項目から、短い形にし、それでも足りなければ項目を落とす (優先度 5 は落とさない)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusItem {
    /// 項目の種類。
    pub kind: StatusKind,
    /// 長い形 (通常の表示)。
    pub long: CellText,
    /// 幅が足りないときの短い形。無い項目は `None` (短くできず、落とすだけ)。
    pub short: Option<CellText>,
    /// 大きいほど最後まで残る (値の正本は 04 §8.7)。
    pub priority: u8,
    /// 左群・右群のどちらに置くか。
    pub align: StatusAlign,
}

/// ステータスバー。`items` の順が表示順 (左群は左から、右群は右から寄せる)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusModel {
    /// 項目 (表示順)。
    pub items: Vec<StatusItem>,
}

/// ヒントバーの文脈 (v0.1 は 2 つ)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum HintContext {
    /// 編集面にフォーカスがあり、チョードの保留もない。
    #[default]
    Normal,
    /// オーバーレイ・検索バーにフォーカスがある、またはチョードの保留中。
    Overlay,
}

/// キーの表示用の並び (チョードは複数ストローク)。表記 (`Ctrl+S` / `^S`) は frontend が作る。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyLabel {
    /// 押す順のストローク (`Ctrl+K H` は 2 つ)。
    pub strokes: Vec<KeyStroke>,
}

/// ヒントバーの 1 項目: キーと、その動作の短い名前。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HintItem {
    /// 動作に割り当てたキー (複数あれば並べて出す)。
    pub keys: Vec<KeyLabel>,
    /// 動作の名前 (解決済みのヒント語)。
    pub text: CellText,
    /// 07 §11.6 の「ヒント」列の優先度 (大きいほど残る)。
    pub priority: u8,
}

/// ヒントバーのモデル (FR-504)。frontend は幅が足りなければ優先度の低い項目から落とす。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HintModel {
    /// どの文脈のヒントか。
    pub context: HintContext,
    /// 優先度の降順。
    pub items: Vec<HintItem>,
}

/// which-key の 1 行: 続けて押すキーと、その動作の名前。続きがさらに続く (入れ子のチョード) ときは `more`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhichKeyItem {
    /// 続けて押すキー。
    pub key: KeyStroke,
    /// 動作の名前 (解決済みの文言)。
    pub text: CellText,
    /// このキーの後にさらに続きがある (入れ子のチョード)。
    pub more: bool,
}

/// which-key の一覧 (04 §7.4)。`prefix` は保留中のキー。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhichKeyModel {
    /// 保留中のストローク (押した順)。
    pub prefix: Vec<KeyStroke>,
    /// 続けて押せるキーの一覧。
    pub items: Vec<WhichKeyItem>,
}

/// 確認ダイアログの選択肢の識別子。値はダイアログごとに core が決め、選んだ結果の対応づけに使う。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChoiceId(pub u16);

/// 確認ダイアログの選択肢。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// 選択肢の識別子。
    pub id: ChoiceId,
    /// ボタンの文言 (解決済み)。
    pub label: CellText,
    /// アクセラレータ (`Text` の 1 文字。大小は区別しない)。
    pub key: Option<char>,
    /// 破壊的な選択肢 (既定にしない)。
    pub destructive: bool,
}

/// 1 行の入力欄。カーソルのセル位置 = `cells[..cursor - first]` の幅の和。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputFieldModel {
    /// 1 書記素 = 1 `Span` (無害化済み・幅付き)。カーソルを含む最大 1,024 書記素の窓。
    pub cells: Vec<Span>,
    /// `cells[0]` の欄内の書記素番号。
    pub first: usize,
    /// 欄全体の書記素数。
    pub len: usize,
    /// 欄内の書記素番号。
    pub cursor: usize,
    /// 欄内の選択範囲 (書記素番号の半開区間)。選択が無ければ `None`。
    pub selection: Option<std::ops::Range<usize>>,
    /// 欄が空のときに出す文言 (解決済み)。
    pub placeholder: Option<CellText>,
}

/// オーバーレイ (意味的モデル。位置と見た目は frontend が決める)。v0.1 は確認と入力プロンプト。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OverlayModel {
    /// 入力プロンプト (行番号へ移動・名前を付けて保存のパスなど)。
    Prompt {
        /// 見出し。
        title: CellText,
        /// 入力欄。
        input: InputFieldModel,
        /// 入力の案内。無ければ `None`。
        hint: Option<CellText>,
        /// 入力の検証エラー。
        error: Option<CellText>,
    },
    /// 確認ダイアログ。破壊的な操作の既定は拒否の側。
    Confirm {
        /// 見出し。
        title: CellText,
        /// 本文。
        message: CellText,
        /// 対象の一覧などの補足の行。
        details: Option<Vec<CellText>>,
        /// 選択肢 (表示順)。
        choices: Vec<Choice>,
        /// 既定の選択肢。
        default: ChoiceId,
        /// フォーカスのある選択肢 (Tab・矢印で動く。初期値は `default`)。
        focused: ChoiceId,
        /// 操作の案内 (`ui.confirm.nav_hint`)。
        nav_hint: Option<CellText>,
    },
    /// ヘルプ画面 (全画面、04 §8.6)。節を縦に並べた 1 本のスクロール。`lines` は全行で、`scroll` 行目から描く。
    Help {
        /// 見出し。
        title: CellText,
        /// ヘルプの全行。
        lines: Vec<HelpLine>,
        /// 先頭に描く行の番号。
        scroll: usize,
        /// 操作の案内 (`ui.help.footer`)。
        footer: CellText,
    },
    /// リスト選択 (コマンドパレット。バッファ一覧・ファインダと共用、FR-511)。
    /// 照合と並び順は core が決める。`items` は `first` 番目から始まる表示窓 (最大 12 行)。
    List {
        /// 見出し。
        title: CellText,
        /// 絞り込みの入力欄。
        input: InputFieldModel,
        /// 表示窓の項目 (上から順)。
        items: Vec<ListItem>,
        /// `items[0]` の、一致した全項目の中での番号。
        first: usize,
        /// 選択中の項目の、一致した全項目の中での番号。
        selected: usize,
        /// 件数の文言 (`ui.palette.footer`)。eve-tui は `total` から文言を作らない。
        footer: CellText,
        /// 0 件のときに候補の位置へ出す文言。
        no_match: CellText,
    },
}

/// ヘルプの 1 行 (04 §8.6)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpLine {
    /// 行の種類 (frontend が見出しの装飾・字下げを決める)。
    pub kind: HelpLineKind,
    /// 左の列 (キー一覧のキー、コマンド一覧の `CommandId`)。`Entry` のときだけ。
    pub key: Option<CellText>,
    /// 本文 (解決済みの文言)。
    pub text: CellText,
}

/// ヘルプの行の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpLineKind {
    /// 節の見出し。
    Heading,
    /// 小見出し (キー一覧のカテゴリ)。
    Subheading,
    /// 説明の文。
    Body,
    /// 左の列 (`key`) と本文の組。
    Entry,
}

/// リストの 1 行 (03 §14.6)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    /// 項目の名前 (パレットではコマンドの名前)。
    pub label: CellText,
    /// 右寄せの補足 (パレットでは `CommandId`)。
    pub detail: Option<CellText>,
    /// `label` の書記素番号の半開区間 (照合した箇所)。バイト位置ではない。
    pub match_ranges: Vec<std::ops::Range<usize>>,
    /// 解決済みキーマップからの逆引き。
    pub keys: Vec<KeyLabel>,
    /// 実行できない理由 (解決済みの文言)。あれば淡色で出す。
    pub disabled_reason: Option<CellText>,
}

/// 検索の大文字小文字の扱い (FR-402)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseMode {
    /// 検索語に大文字を含むときだけ区別する。
    Smart,
    /// 常に区別する。
    Sensitive,
    /// 区別しない。
    Insensitive,
}

/// 検索バーのフォーカス (`Tab` / `Shift+Tab` で 検索欄 → 置換欄 → ボタン を巡回する、04 §8.4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarFocus {
    /// 検索欄。
    Search,
    /// 置換欄。
    Replace,
    /// 「すべて置換」のボタン (ハードウェアカーソルはボタンの先頭セルに置く)。
    Button,
}

/// 置換欄の行 (2 行目)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceRowModel {
    /// ラベル (`ui.search.replace_label`)。
    pub label: CellText,
    /// 置換文字列の入力欄。
    pub input: InputFieldModel,
    /// ボタン (`ui.search.replace_all_button`)。
    pub button: CellText,
}

/// 検索バー (クロームとして編集面の下に 1 行描く。置換欄があれば 2 行、04 §8.4)。表示中だけ `Frame.search` が `Some`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchBarModel {
    /// ラベル (`ui.search.label`)。
    pub label: CellText,
    /// 検索語の入力欄。
    pub input: InputFieldModel,
    /// 大文字小文字の扱いの設定。実際に区別しているかは `case_on`。
    pub case: CaseMode,
    /// 大小を実際に区別している (トグルを反転して示す)。
    pub case_on: bool,
    /// 正規表現として探す (トグル `[.*]` を反転して示す、FR-402)。
    pub regex: bool,
    /// 単語単位で探す (トグル `[ab]` を反転して示す、FR-402)。
    pub whole_word: bool,
    /// 選択範囲内だけを探す (トグル `[sel]` を反転して示す、03 §12.5)。
    pub in_selection: bool,
    /// 検索語を使えない理由 (正規表現の構文エラー・改行を含むパターン)。あれば件数の位置に描き、検索しない。
    pub error: Option<CellText>,
    /// 件数の文言 (`ui.search.count.*`)。検索していなければ `None`。
    pub count_text: Option<CellText>,
    /// 置換欄 (開いているときだけ)。
    pub replace: Option<ReplaceRowModel>,
    /// 検索バーの中のフォーカスのある欄。
    pub focus: BarFocus,
}

/// frontend が自分の判断で出す場面の、core が解決済みの文言。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChromeTexts {
    /// 端末が小さすぎるとき (`ui.too_small`)。
    pub too_small: CellText,
}

/// テーマのロールの表 (ロール名 → その色深度の段のスタイル、07 §8.6)。core が解決し、frontend はクロームと
/// オーバーレイを描くときにロール名で引く。テーマか色深度が変わったときだけ作り直す。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoleStyles(Vec<(String, Style)>);

impl RoleStyles {
    /// `(ロール名, スタイル)` の列から作る (名前の順に並べ直す)。
    pub fn new(mut roles: Vec<(String, Style)>) -> RoleStyles {
        roles.sort_by(|a, b| a.0.cmp(&b.0));
        RoleStyles(roles)
    }

    /// ロールのスタイル。無ければ既定 (端末の既定色・属性なし)。
    pub fn get(&self, name: &str) -> Style {
        self.find(name).unwrap_or(Style::DEFAULT)
    }

    /// ロールのスタイル。表に無ければ `None`。
    pub fn find(&self, name: &str) -> Option<Style> {
        self.0
            .binary_search_by(|(n, _)| n.as_str().cmp(name))
            .ok()
            .map(|i| self.0[i].1)
    }

    /// すべての `(ロール名, スタイル)` (名前の順)。
    pub fn iter(&self) -> impl Iterator<Item = (&str, Style)> {
        self.0.iter().map(|(n, s)| (n.as_str(), *s))
    }
}

/// 1 回の描画で frontend に渡すもの。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// 描く先の Window。
    pub window: WindowId,
    /// 編集面全体の大きさ (セル。端末の全体からクロームと検索バーの行を除いたもの)。
    pub size: CellSize,
    /// 編集面 (レイアウト済み。v0.1 は 1 つ)。
    pub panes: Vec<PaneFrame>,
    /// 入力の行き先。
    pub focus: FocusTarget,
    /// 編集面にフォーカスがあるときだけ `Some`。
    /// 入力欄にフォーカスがあるときは `None` で、frontend が欄の配置と `InputFieldModel` から位置を求める。
    pub primary_cursor: Option<CursorPlacement>,
    /// ステータスバー。
    pub status: StatusModel,
    /// ヒントバー。
    pub hints: HintModel,
    /// ヒントバーを出すか (`ui.hint_bar` / `ui.toggle_hint_bar`、FR-504)。偽なら frontend はその行を本文に回す (04 §3.5)。
    pub hint_bar: bool,
    /// 前面順。
    pub overlays: Vec<OverlayModel>,
    /// チョードの保留が 500 ms を超えたときの、続けて押せるキーの一覧 (which-key、04 §7.4)。
    pub which_key: Option<WhichKeyModel>,
    /// 検索バー。
    pub search: Option<SearchBarModel>,
    /// メッセージ行。
    pub messages: MessageModel,
    /// frontend が自分の判断で出す場面の文言。
    pub texts: ChromeTexts,
    /// 端末タイトル (無害化済み。内容の規約は 04 §3.1)。
    pub title: String,
    /// テーマのロールの表 (クロームとオーバーレイを描くとき引く)。
    pub roles: std::sync::Arc<RoleStyles>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn スタイルの属性は集合() {
        let m = StyleMods::BOLD | StyleMods::REVERSE;
        assert!(m.contains(StyleMods::BOLD) && m.contains(StyleMods::REVERSE));
        assert!(!m.contains(StyleMods::DIM));
        assert!(StyleMods::NONE.is_empty() && !m.is_empty());
        assert_eq!(Style::with_mods(m).fg, Color::Default);
    }

    #[test]
    fn スパンの組み立て() {
        let s = Span::new("あ", 2, Style::DEFAULT);
        assert_eq!((s.text.as_str(), s.width), ("あ", 2));
    }
}
