//! 入力 (docs/04 §6、05 §9.1)。OS に依らない部品 (送り先 `InputOut`・`Text` の合体・編集面の `Resize`) をここに置き、
//! OS ごとの部品を下に分ける。
//!
//! - `normalize`: Windows のコンソールの記録 (`ReadConsoleInputW`) の正規化とバースト検出 (OS を呼ばない)。
//! - `vt`: VT (xterm 形式) のバイト列の解析 (Unix の入力経路。OS を呼ばない)。
//! - `win` / `unix`: 端末を開く・モードを変える・入力スレッド (OS を呼ぶのはこの 2 つだけ、R5)。同じ形
//!   (`ConsoleInput`・`ConsoleOut`・`spawn_input_thread`) をしていて、`frontend` は OS に依らずこれらを呼ぶ。

use eve_view::{CellSize, InputEvent};

use crate::chrome::ScreenLayout;

pub mod normalize;
#[cfg(unix)]
pub mod unix;
pub mod vt;
#[cfg(windows)]
pub mod win;

pub use normalize::{BURST_N, BURST_T_MS, KeyRec, Normalizer, Out, Raw};

/// 連続する `Text` を 1 メッセージにまとめる上限 (バイト、04 §6.1 の合体)。
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

/// 入力の送り先 (本番は `eve_host::InputSink`、テストや診断は別の実装)。
pub trait InputOut: Send + 'static {
    /// `InputEvent` を送る。受け手が閉じていれば `false`。
    fn event(&self, ev: InputEvent) -> bool;
    /// 入力スレッドが続けられなくなった (入力の失敗)。
    fn died(&self);
}

/// 端末のウィンドウの大きさから、編集面の `Resize` を作る (クローム行を除く、04 §3.1)。`hint_bar` は最後の Frame の
/// ヒントバーの表示 (隠していればその行も編集面に入れる、FR-504)。
pub fn surface_resize(window: CellSize, hint_bar: bool) -> InputEvent {
    let s = ScreenLayout::compute(window, hint_bar, 0).surface();
    InputEvent::Resize {
        cols: s.cols,
        rows: s.rows,
    }
}

/// `Text` を持てる出力 (`normalize::Out` と `vt::VtOut`)。
pub(crate) trait TextItem: Sized {
    /// `Text` ならその本文。
    fn text_mut(&mut self) -> Option<&mut String>;
    fn from_text(s: String) -> Self;
}

/// 文字列を `out` に足す (04 §6.1 の合体)。末尾が `Text` で `MAX_TEXT_BYTES` に収まるならそこへ続け、収まらなければ新しい
/// `Text` にする。`s` 自体が上限を超えるなら、文字の境界で `MAX_TEXT_BYTES` ごとに分ける。
pub(crate) fn push_text<T: TextItem>(out: &mut Vec<T>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut().and_then(T::text_mut)
        && last.len() + s.len() <= MAX_TEXT_BYTES
    {
        last.push_str(s);
        return;
    }
    let mut rest = s;
    while !rest.is_empty() {
        let mut cut = rest.len().min(MAX_TEXT_BYTES);
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        out.push(T::from_text(rest[..cut].to_owned()));
        rest = &rest[cut..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl TextItem for InputEvent {
        fn text_mut(&mut self) -> Option<&mut String> {
            match self {
                InputEvent::Text(t) => Some(t),
                _ => None,
            }
        }
        fn from_text(s: String) -> Self {
            InputEvent::Text(s)
        }
    }

    fn lens(out: &[InputEvent]) -> Vec<usize> {
        out.iter()
            .map(|e| match e {
                InputEvent::Text(t) => t.len(),
                _ => usize::MAX,
            })
            .collect()
    }

    #[test]
    fn 末尾の_text_に続け_上限を超えたら新しい_text() {
        let mut out = Vec::new();
        push_text(&mut out, "a");
        push_text(&mut out, "b");
        assert_eq!(out, vec![InputEvent::Text("ab".to_owned())]);
        // ちょうど上限までは続ける
        push_text(&mut out, &"c".repeat(MAX_TEXT_BYTES - 3));
        push_text(&mut out, "c");
        assert_eq!(lens(&out), vec![MAX_TEXT_BYTES]);
        push_text(&mut out, "c");
        assert_eq!(lens(&out), vec![MAX_TEXT_BYTES, 1]);
        out.push(InputEvent::Focus(true));
        push_text(&mut out, "d");
        assert_eq!(lens(&out), vec![MAX_TEXT_BYTES, 1, usize::MAX, 1]);
        push_text(&mut out, "");
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn 大きな文字列は文字の境界で上限ごとに分ける() {
        let mut out = Vec::new();
        // 3 バイトの文字で上限ちょうどにならない長さ
        let s = "あ".repeat(MAX_TEXT_BYTES / 3 + 10);
        push_text(&mut out, &s);
        let l = lens(&out);
        assert_eq!(l.iter().sum::<usize>(), s.len());
        assert!(
            l.iter().all(|&n| n <= MAX_TEXT_BYTES && n % 3 == 0),
            "{l:?}"
        );
        assert_eq!(l.len(), 2);
    }
}
