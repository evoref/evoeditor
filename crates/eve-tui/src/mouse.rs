//! マウスの入力の変換 (docs/04 §6.8)。OS を呼ばない。Windows のコンソールの `MOUSE_EVENT_RECORD` を、編集面の左上を原点とする
//! `MouseEvent` にする。編集面の外 (クローム) で始まった操作は捨て、編集面で押したボタンの間だけは面の外の座標もそのまま渡す
//! (ドラッグの自動スクロールのため、03 §5.8.1)。

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use eve_view::{Modifiers, MouseAt, MouseButton, MouseEvent, MouseKind, ViewId};

/// `MouseRec` のビット (Windows の `MOUSE_EVENT_RECORD` の値。VT の SGR マウスもこの形にして渡す、`vt_input`)。
pub mod button_bits {
    /// `dwButtonState` のビット。
    pub const LEFT: u32 = 0x0001;
    pub const RIGHT: u32 = 0x0002;
    pub const MIDDLE: u32 = 0x0004;
    /// `dwEventFlags`。
    pub const MOVED: u32 = 0x0001;
    pub const WHEELED: u32 = 0x0004;
    pub const HWHEELED: u32 = 0x0008;
    /// `dwControlKeyState` (Ctrl・Alt は左右のどちらか)。
    pub const SHIFT: u32 = 0x0010;
    pub const CTRL: u32 = 0x0008 | 0x0004;
    pub const ALT: u32 = 0x0002 | 0x0001;
}

use button_bits::{
    ALT as ALT_PRESSED, CTRL as CTRL_PRESSED, HWHEELED, LEFT, MIDDLE, MOVED, RIGHT,
    SHIFT as SHIFT_PRESSED, WHEELED,
};

/// コンソールのマウスの記録 1 件 (座標は端末のウィンドウの左上を原点とするセル)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseRec {
    pub col: i32,
    pub row: i32,
    pub buttons: u32,
    pub state: u32,
    pub flags: u32,
}

/// 描画側 (`render`) が入力スレッドへ渡す、編集面の位置。`view == 0` は未設定 (最初の Frame の前)。
#[derive(Debug, Default)]
pub struct SharedGeom {
    view: AtomicU64,
    rows: AtomicU32,
    /// 最後の Frame でヒントバーを隠していたか (`Frame.hint_bar` の否定。既定は表示)。端末の大きさから編集面の行数を
    /// 求めるとき (Resize) に使う (FR-504、04 §3.5)。
    hint_bar_hidden: std::sync::atomic::AtomicBool,
}

impl SharedGeom {
    pub fn set_hint_bar(&self, shown: bool) {
        self.hint_bar_hidden.store(!shown, Ordering::Relaxed);
    }

    pub fn hint_bar(&self) -> bool {
        !self.hint_bar_hidden.load(Ordering::Relaxed)
    }

    pub fn set(&self, view: ViewId, rows: u16) {
        self.view.store(view.0, Ordering::Relaxed);
        self.rows.store(u32::from(rows), Ordering::Relaxed);
    }

    fn get(&self) -> Option<(ViewId, i32)> {
        let v = self.view.load(Ordering::Relaxed);
        (v != 0).then(|| (ViewId(v), self.rows.load(Ordering::Relaxed) as i32))
    }
}

/// ボタンの状態の差分から Down / Up / Drag を作る。
#[derive(Debug, Default)]
pub struct MouseTracker {
    buttons: u32,
    /// 編集面で押して `Down` を出したボタン (面の外でも Drag / Up を届ける。外で押したものには Up を出さない)。
    pressed_in_pane: u32,
}

fn mods(state: u32) -> Modifiers {
    let mut m = Modifiers::NONE;
    if state & SHIFT_PRESSED != 0 {
        m = m | Modifiers::SHIFT;
    }
    if state & CTRL_PRESSED != 0 {
        m = m | Modifiers::CTRL;
    }
    if state & ALT_PRESSED != 0 {
        m = m | Modifiers::ALT;
    }
    m
}

impl MouseTracker {
    pub fn new() -> MouseTracker {
        MouseTracker::default()
    }

    pub fn feed(&mut self, r: MouseRec, geom: &SharedGeom) -> Vec<MouseEvent> {
        let Some((view, rows)) = geom.get() else {
            self.buttons = r.buttons & (LEFT | RIGHT | MIDDLE);
            return Vec::new();
        };
        let inside = r.row >= 0 && r.row < rows && r.col >= 0;
        let ev = |kind| MouseEvent {
            kind,
            mods: mods(r.state),
            at: MouseAt {
                view,
                row: r.row,
                col: r.col,
            },
        };
        if r.flags & HWHEELED != 0 {
            return Vec::new();
        }
        if r.flags & WHEELED != 0 {
            // ホイールの向きは `dwButtonState` の上位 16 ビット (符号付き) の符号
            let delta = (r.buttons >> 16) as u16 as i16;
            return match delta {
                d if d > 0 && inside => vec![ev(MouseKind::WheelUp)],
                d if d < 0 && inside => vec![ev(MouseKind::WheelDown)],
                _ => Vec::new(),
            };
        }
        let now = r.buttons & (LEFT | RIGHT | MIDDLE);
        let before = self.buttons;
        self.buttons = now;
        let mut out = Vec::new();
        for (bit, button) in [
            (LEFT, MouseButton::Left),
            (MIDDLE, MouseButton::Middle),
            (RIGHT, MouseButton::Right),
        ] {
            let (was, is) = (before & bit != 0, now & bit != 0);
            if is && !was && inside {
                self.pressed_in_pane |= bit;
                out.push(ev(MouseKind::Down(button)));
            } else if was && !is && self.pressed_in_pane & bit != 0 {
                self.pressed_in_pane &= !bit;
                out.push(ev(MouseKind::Up(button)));
            }
        }
        if r.flags & MOVED != 0
            && now & LEFT != 0
            && self.pressed_in_pane & LEFT != 0
            && out.is_empty()
        {
            out.push(ev(MouseKind::Drag(MouseButton::Left)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom() -> SharedGeom {
        let g = SharedGeom::default();
        g.set(ViewId(7), 10);
        g
    }

    fn rec(col: i32, row: i32, buttons: u32, flags: u32) -> MouseRec {
        MouseRec {
            col,
            row,
            buttons,
            state: 0,
            flags,
        }
    }

    fn kinds(v: &[MouseEvent]) -> Vec<MouseKind> {
        v.iter().map(|e| e.kind).collect()
    }

    #[test]
    fn 押す_動かす_離すが_down_drag_up_になる() {
        let (g, mut t) = (geom(), MouseTracker::new());
        let d = t.feed(rec(5, 2, LEFT, 0), &g);
        assert_eq!(kinds(&d), [MouseKind::Down(MouseButton::Left)]);
        assert_eq!((d[0].at.row, d[0].at.col, d[0].at.view), (2, 5, ViewId(7)));
        let m = t.feed(rec(6, 3, LEFT, MOVED), &g);
        assert_eq!(kinds(&m), [MouseKind::Drag(MouseButton::Left)]);
        let u = t.feed(rec(6, 3, 0, 0), &g);
        assert_eq!(kinds(&u), [MouseKind::Up(MouseButton::Left)]);
    }

    #[test]
    fn ボタンなしの移動は捨てる() {
        let (g, mut t) = (geom(), MouseTracker::new());
        assert!(t.feed(rec(1, 1, 0, MOVED), &g).is_empty());
    }

    #[test]
    fn 面の外で押した操作は捨て_面で押したドラッグは外でも届く() {
        let (g, mut t) = (geom(), MouseTracker::new());
        // クローム (行 10 以降) で押す
        assert!(t.feed(rec(1, 10, LEFT, 0), &g).is_empty());
        assert!(
            t.feed(rec(1, 5, LEFT, MOVED), &g).is_empty(),
            "外で始まった"
        );
        assert!(t.feed(rec(1, 5, 0, 0), &g).is_empty());
        // 面で押して、面の下へ引く
        t.feed(rec(1, 5, LEFT, 0), &g);
        let m = t.feed(rec(1, 12, LEFT, MOVED), &g);
        assert_eq!(
            (kinds(&m), m[0].at.row),
            (vec![MouseKind::Drag(MouseButton::Left)], 12)
        );
        let u = t.feed(rec(1, 12, 0, 0), &g);
        assert_eq!(kinds(&u), [MouseKind::Up(MouseButton::Left)]);
    }

    #[test]
    fn ホイールは向きで分け_面の外は捨てる() {
        let (g, mut t) = (geom(), MouseTracker::new());
        let up = rec(1, 1, 120u32 << 16, WHEELED);
        let down = rec(1, 1, (-120i16 as u16 as u32) << 16, WHEELED);
        assert_eq!(kinds(&t.feed(up, &g)), [MouseKind::WheelUp]);
        assert_eq!(kinds(&t.feed(down, &g)), [MouseKind::WheelDown]);
        let out = rec(1, 11, 120u32 << 16, WHEELED);
        assert!(t.feed(out, &g).is_empty());
    }

    #[test]
    fn 修飾キーを写し_右と中ボタンも区別する() {
        let (g, mut t) = (geom(), MouseTracker::new());
        let mut r = rec(0, 0, RIGHT, 0);
        r.state = SHIFT_PRESSED;
        let e = t.feed(r, &g);
        assert_eq!(kinds(&e), [MouseKind::Down(MouseButton::Right)]);
        assert_eq!(e[0].mods, Modifiers::SHIFT);
        t.feed(rec(0, 0, 0, 0), &g);
        let e = t.feed(rec(0, 0, MIDDLE, 0), &g);
        assert_eq!(kinds(&e), [MouseKind::Down(MouseButton::Middle)]);
    }

    #[test]
    fn 最初の_frame_の前は何も出さない() {
        let (g, mut t) = (SharedGeom::default(), MouseTracker::new());
        assert!(t.feed(rec(1, 1, LEFT, 0), &g).is_empty());
    }
}
