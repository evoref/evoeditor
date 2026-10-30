//! `Effect`: core が直接実行できない副作用 (frontend / OS に依存するもの) の **要求** (docs/02-architecture.md §6.3)。
//! `eve-host` が `Frontend::apply_effect` に渡し、frontend が扱わないものは Host が実行する。
//! 結果が必要なものは `InputEvent` / `HostMsg` で core に返る。

use crate::ids::{ClipboardHandleId, ClipboardToken};

/// 副作用の要求。変種は最初の利用者の版で足す (`#[non_exhaustive]`)。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Effect {
    /// 終了。順序は、`rescue` の退避 → `Frontend::restore` → `stderr` の各行を出力 → exit (02 §9.2)。
    Quit {
        /// 終了コード (値の正本は 07 §18.4)。
        code: u8,
        rescue: QuitRescue,
        /// 端末の復元の後に出す行。core は文言を解決せず、MessageKey の名前と引数で渡す。
        stderr: Vec<StderrLine>,
    },
    /// クリップボードへの書込。
    SetClipboard {
        content: ClipboardText,
        target: ClipboardTarget,
    },
    /// クリップボードの読込。応答は `InputEvent::ClipboardContent`。
    RequestClipboard {
        token: ClipboardToken,
        target: ClipboardTarget,
    },
    /// ベル。
    Bell,
    /// マウスの取得を有効 / 無効にする (`ui.toggle_mouse`。無効の間は端末のネイティブ選択・スクロールが使える、04 §6.8)。
    SetMouseCapture { enabled: bool },
    /// 画面の全体を描き直す (`ui.redraw`。04 §4.7 (f)。ずれた表示からの回復)。
    Redraw,
}

/// 終了時に未保存の文書を退避するか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuitRescue {
    /// 通常の終了 (FR-107 の確認を経ている)。
    None,
    /// 「保存を中止して終了」。Host が exit の前に、dirty な文書を緊急退避と同じ書き手・形式で退避する。
    DirtyDocuments,
}

/// 端末の復元の後に stderr へ出す 1 行の指定。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StderrLine {
    /// MessageKey の名前 (`stderr.*` のみ)。
    pub key: String,
    /// プレースホルダの名前と値。
    pub args: Vec<(String, String)>,
}

/// `Effect::SetClipboard` の中身。
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardText {
    /// 小さい選択 (64 KiB 未満) は、core が文字列化して直接入れる。
    Inline(String),
    /// 大きい選択。中身は eve-core の `Editor` が持つ表にあり、Host が `Editor::take_clipboard` で取り出す。
    Handle(ClipboardHandleId),
}

/// クリップボードの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardTarget {
    System,
    Primary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 終了の要求() {
        let e = Effect::Quit {
            code: 0,
            rescue: QuitRescue::None,
            stderr: vec![StderrLine {
                key: "stderr.example".into(),
                args: vec![("path".into(), "a.txt".into())],
            }],
        };
        assert_eq!(e.clone(), e);
    }

    #[test]
    fn クリップボードの内容() {
        let inline = ClipboardText::Inline("abc".into());
        assert_ne!(inline, ClipboardText::Handle(ClipboardHandleId(1)));
    }
}
