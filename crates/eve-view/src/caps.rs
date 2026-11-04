//! frontend の能力 (docs/02-architecture.md §7.1「Capabilities」)。frontend の差を core に伝える唯一の手段。
//!
//! core が読む項目だけを置く。端末にしか意味のない項目 (同期出力・OSC 52 の書込・ブラケットペースト・フォーカスイベント) は
//! frontend の内部状態にして、ここに置かない (OQ-192 の提案に従う。確定したら 02 を直す)。
//! 境界型の凍結段階は C0 (流動)。struct への `#[non_exhaustive]` は C3 から (§8.4)。

use crate::input::KeyStroke;

/// 色深度。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColorDepth {
    Mono,
    Ansi16,
    Ansi256,
    #[default]
    TrueColor,
}

/// 端末の大きさ (セル単位)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellSize {
    pub cols: u16,
    pub rows: u16,
}

/// frontend の能力。`Editor::new` で初期値を渡し、以降の変化は `InputEvent::CapabilitiesChanged` で届く。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// 色深度。TUI は起動時に環境変数だけで判定し、実行中は変えない。
    pub color_depth: ColorDepth,
    pub mouse: bool,
    /// kitty keyboard protocol などで修飾が区別できる。
    pub enhanced_keys: bool,
    /// OSC 52 でクリップボードへ書ける端末 (05 §8.1)。
    pub osc52_write: bool,
    /// Unix のジョブ制御 (`app.suspend` の when 句 `caps.can_suspend`)。
    pub can_suspend: bool,
    /// 端末背景の明暗のヒント。v0.1 は常に `None`。
    pub dark_mode: Option<bool>,
    /// 罫線・記号を ASCII に限定する。
    pub ascii_only: bool,
    /// 読み上げ配慮モード。
    pub screen_reader: bool,
    /// 端末種別からの曖昧幅の推定 (設定が優先)。
    pub ambiguous_width_hint: Option<u8>,
    /// frontend が描ける最小の端末サイズ。
    pub min_surface: CellSize,
    /// 端末が横取りして届かない既定キー (不明な端末は空)。
    pub intercepted_keys: Vec<KeyStroke>,
}

impl Default for Capabilities {
    /// ヘッドレス (テスト用の frontend) の既定: TrueColor、マウスなし、拡張キーなし。
    fn default() -> Capabilities {
        Capabilities {
            color_depth: ColorDepth::TrueColor,
            mouse: false,
            enhanced_keys: false,
            osc52_write: false,
            can_suspend: false,
            dark_mode: None,
            ascii_only: false,
            screen_reader: false,
            ambiguous_width_hint: None,
            min_surface: CellSize { cols: 20, rows: 4 },
            intercepted_keys: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 既定の能力() {
        let c = Capabilities::default();
        assert_eq!(c.color_depth, ColorDepth::TrueColor);
        assert!(!c.mouse && !c.can_suspend);
        assert!(c.intercepted_keys.is_empty());
        assert_eq!(c.clone(), c);
    }
}
