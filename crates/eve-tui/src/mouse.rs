//! マウスの入力の変換 (docs/04 §6.8)。OS を呼ばない。Windows のコンソールの `MOUSE_EVENT_RECORD` を、押した位置のペインの
//! 本文の左上を原点とする `MouseEvent` にする。タブラインのタブの左の押下は `ChromeMouse` (`ChromeTarget::Tab`)、ペインの
//! 見出しの左の押下は `ChromeMouse` (`ChromeTarget::PaneBar`) にする (03 §9.9 の PANE-7 (d) / (e)、PANE-12 (g))。判定の順は
//! タブライン → 見出し → 本文 (区切りは何もしない。オーバーレイの間の入力は core が捨てる)。区切りの上の操作は v0.5 では渡さない。ペインの外 (クローム) で始まった操作は捨て、ペインで押したボタンの間だけは、
//! 面の外の座標も押したペインの原点のまま渡す (ドラッグの自動スクロールのため、03 §5.8.1 / §9.9 の PANE-7)。
//!
//! 変換は「(幾何, マウスの記録) → 入力」で、幾何 ([`Geometry`]) は描画側が Frame ごとに作って入力スレッドへ差し替える。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use eve_view::{
    CellRect, ChromeMouseEvent, ChromeTarget, DocumentId, Frame, InputEvent, Modifiers, MouseAt,
    MouseButton, MouseEvent, MouseKind, ViewId,
};

use crate::glyphs::Glyphs;

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

/// 1 つのペインの位置 (Frame の `PaneFrame` の写し)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneGeom {
    /// ペインが表示している View (`MouseAt.view`)。
    pub view: ViewId,
    /// ペインの本文 (ガター + テキスト) の矩形。端末の左上が原点 (タブラインを出すときは、Frame の矩形を 1 行下げたもの)。
    pub rect: CellRect,
    /// ペインの見出しの矩形 (`PaneFrame.bar.rect`。端末の左上が原点。見出しが無ければ `None`)。
    pub bar: Option<CellRect>,
}

/// タブラインの 1 つのタブの位置 (`chrome::tab_line::layout` で並べたもの)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabGeom {
    /// タブの文書 (`ChromeTarget::Tab { doc }`)。
    pub doc: DocumentId,
    /// タブの矩形 (`[` から `]` まで。端末の左上が原点で、行は 0)。
    pub rect: CellRect,
}

fn rect_contains(r: CellRect, col: i32, row: i32) -> bool {
    let (c0, r0) = (i32::from(r.col), i32::from(r.row));
    col >= c0 && col < c0 + i32::from(r.cols) && row >= r0 && row < r0 + i32::from(r.rows)
}

impl PaneGeom {
    fn contains(&self, col: i32, row: i32) -> bool {
        rect_contains(self.rect, col, row)
    }

    /// 端末の座標を、このペインの本文の左上を原点とする位置にする (面の外も負や本文の大きさ以上の値のまま)。
    fn at(&self, col: i32, row: i32) -> MouseAt {
        MouseAt {
            view: self.view,
            row: row.saturating_sub(i32::from(self.rect.row)),
            col: col.saturating_sub(i32::from(self.rect.col)),
        }
    }
}

/// マウスの変換に使う画面の幾何 (全ペインの位置とタブラインのタブ)。空は未設定 (最初の Frame の前)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Geometry {
    /// ペイン (Frame の `panes` の順)。
    pub panes: Vec<PaneGeom>,
    /// タブラインに並べたタブ (左から。タブラインが無ければ空)。省いた側の印・タブの間・行の残りは入れない。
    pub tabs: Vec<TabGeom>,
}

impl Geometry {
    /// Frame から作る。タブの位置は描画と同じ記号 (`glyphs`) で並べる (03 §9.9 の PANE-12 (g))。タブラインを出すときは、
    /// ペインの矩形を 1 行下げる (編集面はタブラインの下)。
    pub fn from_frame(frame: &Frame, glyphs: &Glyphs) -> Geometry {
        let top = crate::chrome::layout::tab_rows(frame);
        let down = |r: CellRect| CellRect {
            row: r.row.saturating_add(top),
            ..r
        };
        let tabs = frame
            .tab_line
            .as_ref()
            .map(|m| {
                crate::chrome::tab_line::layout(m, frame.size.cols, glyphs)
                    .tabs
                    .into_iter()
                    .map(|t| TabGeom {
                        doc: t.doc,
                        rect: t.rect,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Geometry {
            panes: frame
                .panes
                .iter()
                .map(|p| PaneGeom {
                    view: p.view,
                    rect: down(p.rect),
                    bar: p.bar.as_ref().map(|b| down(b.rect)),
                })
                .collect(),
            tabs,
        }
    }

    /// 端末の位置 (`col`, `row`) にあるタブ。
    fn hit_tab(&self, col: i32, row: i32) -> Option<TabGeom> {
        self.tabs
            .iter()
            .copied()
            .find(|t| rect_contains(t.rect, col, row))
    }

    /// 端末の位置 (`col`, `row`) にあるペイン。
    fn hit(&self, col: i32, row: i32) -> Option<PaneGeom> {
        self.panes.iter().copied().find(|p| p.contains(col, row))
    }

    /// 端末の位置 (`col`, `row`) にある見出しのペインと、見出しの矩形。
    fn hit_bar(&self, col: i32, row: i32) -> Option<(PaneGeom, CellRect)> {
        self.panes.iter().find_map(|p| {
            let b = p.bar?;
            rect_contains(b, col, row).then_some((*p, b))
        })
    }
}

/// 描画側 (`render`) が入力スレッドへ渡す、画面の幾何。Frame ごとに丸ごと差し替える。
#[derive(Debug, Default)]
pub struct SharedGeom {
    geom: Mutex<Arc<Geometry>>,
    /// 最後の Frame でヒントバーを隠していたか (`Frame.hint_bar` の否定。既定は表示)。端末の大きさから編集面の行数を
    /// 求めるとき (Resize) に使う (FR-504、04 §3.5)。
    hint_bar_hidden: AtomicBool,
}

impl SharedGeom {
    pub fn set_hint_bar(&self, shown: bool) {
        self.hint_bar_hidden.store(!shown, Ordering::Relaxed);
    }

    pub fn hint_bar(&self) -> bool {
        !self.hint_bar_hidden.load(Ordering::Relaxed)
    }

    /// 幾何を差し替える (変わっていなければ何もしない)。
    pub fn set(&self, geom: Geometry) {
        // 片方のスレッドがパニックしても、幾何は差し替えるだけの値なので、毒になった錠のままでも読み書きしてよい
        let mut g = self.geom.lock().unwrap_or_else(PoisonError::into_inner);
        if **g != geom {
            *g = Arc::new(geom);
        }
    }

    /// 今の幾何。
    pub fn get(&self) -> Arc<Geometry> {
        Arc::clone(&self.geom.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// ボタンの状態の差分から Down / Up / Drag を作る。
#[derive(Debug, Default)]
pub struct MouseTracker {
    buttons: u32,
    /// ペインで押して `Down` を出したボタン (面の外でも Drag / Up を届ける。外で押したものには Up を出さない)。
    pressed_in_pane: u32,
    /// `pressed_in_pane` のボタンを押したペイン (Drag / Up はこのペインの原点で渡す、PANE-7 (b))。
    pressed: Option<PaneGeom>,
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

    /// マウスの記録 `r` を、幾何 `geom` で入力 (`Mouse` と、タブ・見出しの `ChromeMouse`) にする (ボタンの状態は自分が覚える)。
    pub fn feed(&mut self, r: MouseRec, geom: &Geometry) -> Vec<InputEvent> {
        // タブ・見出しの左の押下 (ペインで押したボタンが無いとき)。区切り・タブと見出しのほかの操作 (離す・ドラッグ・
        // ホイール・中と右のボタン) は渡さない (PANE-7 (f)、PANE-12 (g))
        if r.flags & (MOVED | WHEELED | HWHEELED) == 0
            && self.pressed_in_pane == 0
            && r.buttons & LEFT != 0
            && self.buttons & LEFT == 0
        {
            let hit = geom
                .hit_tab(r.col, r.row)
                .map(|t| (ChromeTarget::Tab { doc: t.doc }, t.rect))
                .or_else(|| {
                    geom.hit_bar(r.col, r.row)
                        .map(|(p, b)| (ChromeTarget::PaneBar { view: p.view }, b))
                });
            if let Some((target, rect)) = hit {
                self.buttons = r.buttons & (LEFT | RIGHT | MIDDLE);
                return vec![InputEvent::ChromeMouse(ChromeMouseEvent {
                    kind: MouseKind::Down(MouseButton::Left),
                    mods: mods(r.state),
                    target,
                    row: r.row - i32::from(rect.row),
                    col: r.col - i32::from(rect.col),
                })];
            }
        }
        self.feed_body(r, geom)
            .into_iter()
            .map(InputEvent::Mouse)
            .collect()
    }

    /// 編集面 (ペインの本文) のマウス。
    fn feed_body(&mut self, r: MouseRec, geom: &Geometry) -> Vec<MouseEvent> {
        if geom.panes.is_empty() {
            self.buttons = r.buttons & (LEFT | RIGHT | MIDDLE);
            return Vec::new();
        }
        let hit = geom.hit(r.col, r.row);
        let ev = |kind, pane: PaneGeom| MouseEvent {
            kind,
            mods: mods(r.state),
            at: pane.at(r.col, r.row),
        };
        if r.flags & HWHEELED != 0 {
            return Vec::new();
        }
        if r.flags & WHEELED != 0 {
            // ホイールはポインタの下のペインに渡す (PANE-7 (c))。向きは `dwButtonState` の上位 16 ビット (符号付き) の符号
            let delta = ((r.buttons >> 16) as u16).cast_signed();
            return match (delta, hit) {
                (d, Some(p)) if d > 0 => vec![ev(MouseKind::WheelUp, p)],
                (d, Some(p)) if d < 0 => vec![ev(MouseKind::WheelDown, p)],
                _ => Vec::new(),
            };
        }
        let now = r.buttons & (LEFT | RIGHT | MIDDLE);
        let before = self.buttons;
        self.buttons = now;
        // 押す先: ボタンを押している間は、押したペイン (の中で押したときだけ)。何も押していなければポインタの下のペイン。
        // 同じペインかは矩形で比べる (押している間に Frame が替わり、そのペインが別の文書の View を出していても同じペイン)
        let target = match self.pressed {
            Some(p) if self.pressed_in_pane != 0 => hit.filter(|h| h.rect == p.rect),
            _ => hit,
        };
        let mut out = Vec::new();
        for (bit, button) in [
            (LEFT, MouseButton::Left),
            (MIDDLE, MouseButton::Middle),
            (RIGHT, MouseButton::Right),
        ] {
            let (was, is) = (before & bit != 0, now & bit != 0);
            if is && !was {
                if let Some(p) = target {
                    self.pressed_in_pane |= bit;
                    self.pressed = Some(p);
                    out.push(ev(MouseKind::Down(button), p));
                }
            } else if was
                && !is
                && self.pressed_in_pane & bit != 0
                && let Some(p) = self.pressed
            {
                self.pressed_in_pane &= !bit;
                out.push(ev(MouseKind::Up(button), p));
            }
        }
        if r.flags & MOVED != 0
            && now & LEFT != 0
            && self.pressed_in_pane & LEFT != 0
            && out.is_empty()
            && let Some(p) = self.pressed
        {
            out.push(ev(MouseKind::Drag(MouseButton::Left), p));
        }
        if self.pressed_in_pane == 0 {
            self.pressed = None;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(view: u64, col: u16, row: u16, cols: u16, rows: u16) -> PaneGeom {
        PaneGeom {
            view: ViewId(view),
            rect: CellRect {
                col,
                row,
                cols,
                rows,
            },
            bar: None,
        }
    }

    /// 1 ペイン (80 列 × 10 行) の幾何。
    fn geom() -> Geometry {
        Geometry {
            panes: vec![pane(7, 0, 0, 80, 10)],
            tabs: Vec::new(),
        }
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

    /// `MouseTracker::feed` の結果のうち編集面のマウス (見出しの `ChromeMouse` が混じれば失敗)。
    fn feed(t: &mut MouseTracker, r: MouseRec, g: &Geometry) -> Vec<MouseEvent> {
        t.feed(r, g)
            .into_iter()
            .map(|e| match e {
                InputEvent::Mouse(m) => m,
                other => panic!("編集面のマウスでない: {other:?}"),
            })
            .collect()
    }

    fn kinds(v: &[MouseEvent]) -> Vec<MouseKind> {
        v.iter().map(|e| e.kind).collect()
    }

    #[test]
    fn 押す_動かす_離すが_down_drag_up_になる() {
        let (g, mut t) = (geom(), MouseTracker::new());
        let d = feed(&mut t, rec(5, 2, LEFT, 0), &g);
        assert_eq!(kinds(&d), [MouseKind::Down(MouseButton::Left)]);
        assert_eq!((d[0].at.row, d[0].at.col, d[0].at.view), (2, 5, ViewId(7)));
        let m = feed(&mut t, rec(6, 3, LEFT, MOVED), &g);
        assert_eq!(kinds(&m), [MouseKind::Drag(MouseButton::Left)]);
        let u = feed(&mut t, rec(6, 3, 0, 0), &g);
        assert_eq!(kinds(&u), [MouseKind::Up(MouseButton::Left)]);
    }

    #[test]
    fn ボタンなしの移動は捨てる() {
        let (g, mut t) = (geom(), MouseTracker::new());
        let actual = feed(&mut t, rec(1, 1, 0, MOVED), &g);
        assert!(actual.is_empty(), "{actual:?}");
    }

    #[test]
    fn 面の外で押した操作は捨て_面で押したドラッグは外でも届く() {
        let (g, mut t) = (geom(), MouseTracker::new());
        // クローム (行 10 以降) で押す
        let actual = feed(&mut t, rec(1, 10, LEFT, 0), &g);
        assert!(actual.is_empty(), "{actual:?}");
        assert!(
            feed(&mut t, rec(1, 5, LEFT, MOVED), &g).is_empty(),
            "外で始まった"
        );
        let actual = feed(&mut t, rec(1, 5, 0, 0), &g);
        assert!(actual.is_empty(), "{actual:?}");
        // 面で押して、面の下へ引く
        feed(&mut t, rec(1, 5, LEFT, 0), &g);
        let m = feed(&mut t, rec(1, 12, LEFT, MOVED), &g);
        assert_eq!(
            (kinds(&m), m[0].at.row),
            (vec![MouseKind::Drag(MouseButton::Left)], 12)
        );
        let u = feed(&mut t, rec(1, 12, 0, 0), &g);
        assert_eq!(kinds(&u), [MouseKind::Up(MouseButton::Left)]);
    }

    #[test]
    fn ホイールは向きで分け_面の外は捨てる() {
        let (g, mut t) = (geom(), MouseTracker::new());
        let up = rec(1, 1, 120u32 << 16, WHEELED);
        let down = rec(1, 1, u32::from((-120i16).cast_unsigned()) << 16, WHEELED);
        assert_eq!(kinds(&feed(&mut t, up, &g)), [MouseKind::WheelUp]);
        assert_eq!(kinds(&feed(&mut t, down, &g)), [MouseKind::WheelDown]);
        let out = rec(1, 11, 120u32 << 16, WHEELED);
        let actual = feed(&mut t, out, &g);
        assert!(actual.is_empty(), "{actual:?}");
    }

    #[test]
    fn 修飾キーを写し_右と中ボタンも区別する() {
        let (g, mut t) = (geom(), MouseTracker::new());
        let mut r = rec(0, 0, RIGHT, 0);
        r.state = SHIFT_PRESSED;
        let e = feed(&mut t, r, &g);
        assert_eq!(kinds(&e), [MouseKind::Down(MouseButton::Right)]);
        assert_eq!(e[0].mods, Modifiers::SHIFT);
        feed(&mut t, rec(0, 0, 0, 0), &g);
        let e = feed(&mut t, rec(0, 0, MIDDLE, 0), &g);
        assert_eq!(kinds(&e), [MouseKind::Down(MouseButton::Middle)]);
    }

    #[test]
    fn 最初の_frame_の前は何も出さない() {
        let (g, mut t) = (Geometry::default(), MouseTracker::new());
        let actual = feed(&mut t, rec(1, 1, LEFT, 0), &g);
        assert!(actual.is_empty(), "{actual:?}");
    }

    /// 左右に 2 つ (左 = View 1 の 0..40 列、区切り 40 列、右 = View 2 の 41..81 列。本文は 0..9 行で、9 行は見出し)。
    fn two() -> Geometry {
        Geometry {
            panes: vec![pane(1, 0, 0, 40, 9), pane(2, 41, 0, 40, 9)],
            tabs: Vec::new(),
        }
    }

    #[test]
    fn 押した位置のペインの本文の左上を原点にする() {
        let (g, mut t) = (two(), MouseTracker::new());
        let d = feed(&mut t, rec(45, 3, LEFT, 0), &g);
        assert_eq!(kinds(&d), [MouseKind::Down(MouseButton::Left)]);
        assert_eq!((d[0].at.view, d[0].at.col, d[0].at.row), (ViewId(2), 4, 3));
        feed(&mut t, rec(45, 3, 0, 0), &g);
        let d = feed(&mut t, rec(5, 2, LEFT, 0), &g);
        assert_eq!((d[0].at.view, d[0].at.col, d[0].at.row), (ViewId(1), 5, 2));
    }

    #[test]
    fn ドラッグと_up_は押したペインのまま_ほかのペインの上でもその原点で渡す() {
        let (g, mut t) = (two(), MouseTracker::new());
        feed(&mut t, rec(45, 3, LEFT, 0), &g);
        // 区切りを越えて左のペインの上へ引く: 右のペインの原点で負の列になる
        let m = feed(&mut t, rec(10, 5, LEFT, MOVED), &g);
        assert_eq!(kinds(&m), [MouseKind::Drag(MouseButton::Left)]);
        assert_eq!(
            (m[0].at.view, m[0].at.col, m[0].at.row),
            (ViewId(2), -31, 5)
        );
        let u = feed(&mut t, rec(10, 12, 0, 0), &g);
        assert_eq!(kinds(&u), [MouseKind::Up(MouseButton::Left)]);
        assert_eq!(
            (u[0].at.view, u[0].at.col, u[0].at.row),
            (ViewId(2), -31, 12)
        );
    }

    #[test]
    fn 区切りと見出しの行で押した操作は捨てる() {
        let (g, mut t) = (two(), MouseTracker::new());
        for (col, row) in [(40, 3), (5, 9), (45, 9)] {
            let actual = feed(&mut t, rec(col, row, LEFT, 0), &g);
            assert!(actual.is_empty(), "({col}, {row}): {actual:?}");
            let actual = feed(&mut t, rec(col, row, 0, 0), &g);
            assert!(actual.is_empty(), "({col}, {row}): {actual:?}");
        }
    }

    #[test]
    fn ホイールはポインタの下のペインに渡す() {
        let (g, mut t) = (two(), MouseTracker::new());
        let up = |col| rec(col, 1, 120u32 << 16, WHEELED);
        let e = feed(&mut t, up(50), &g);
        assert_eq!(
            (kinds(&e), e[0].at.view),
            (vec![MouseKind::WheelUp], ViewId(2))
        );
        let e = feed(&mut t, up(3), &g);
        assert_eq!(
            (kinds(&e), e[0].at.view),
            (vec![MouseKind::WheelUp], ViewId(1))
        );
        let actual = feed(&mut t, up(40), &g);
        assert!(actual.is_empty(), "区切りの上: {actual:?}");
    }

    #[test]
    fn 押している間はほかのペインで押したボタンを捨てる() {
        let (g, mut t) = (two(), MouseTracker::new());
        feed(&mut t, rec(5, 1, LEFT, 0), &g);
        let actual = feed(&mut t, rec(50, 1, LEFT | RIGHT, 0), &g);
        assert!(actual.is_empty(), "{actual:?}");
        // 左を離す: 押したペインの原点で Up。右は Down を出していないので Up も出さない
        let u = feed(&mut t, rec(50, 1, 0, 0), &g);
        assert_eq!(kinds(&u), [MouseKind::Up(MouseButton::Left)]);
        assert_eq!(u[0].at.view, ViewId(1));
    }

    #[test]
    fn 押している間に_frame_が替わっても同じペインで押した_2_つ目のボタンは届く() {
        let (g, mut t) = (two(), MouseTracker::new());
        feed(&mut t, rec(5, 1, LEFT, 0), &g);
        // 左のペインが別の文書 (View 3) に切り替わった Frame の幾何
        let g2 = Geometry {
            panes: vec![pane(3, 0, 0, 40, 9), pane(2, 41, 0, 40, 9)],
            tabs: Vec::new(),
        };
        let d = feed(&mut t, rec(6, 1, LEFT | RIGHT, 0), &g2);
        assert_eq!(kinds(&d), [MouseKind::Down(MouseButton::Right)]);
        assert_eq!(d[0].at.view, ViewId(3));
    }

    /// `two` に見出し (9 行目、本文と同じ列) を足したもの。
    fn two_with_bars() -> Geometry {
        let mut g = two();
        for p in &mut g.panes {
            p.bar = Some(CellRect {
                row: 9,
                rows: 1,
                ..p.rect
            });
        }
        g
    }

    // verifies: AC-FR-509-4
    #[test]
    fn 見出しの左の押下は_chrome_mouse_で_見出しのペインの_view_を指す() {
        let (g, mut t) = (two_with_bars(), MouseTracker::new());
        let e = t.feed(rec(45, 9, LEFT, 0), &g);
        assert_eq!(
            e,
            vec![InputEvent::ChromeMouse(ChromeMouseEvent {
                kind: MouseKind::Down(MouseButton::Left),
                mods: Modifiers::NONE,
                target: ChromeTarget::PaneBar { view: ViewId(2) },
                row: 0,
                col: 4,
            })]
        );
        // 離す・見出しの上のホイール・右ボタンは渡さない (PANE-7 (f))
        let actual = t.feed(rec(45, 9, 0, 0), &g);
        assert!(actual.is_empty(), "{actual:?}");
        let actual = t.feed(rec(45, 9, 120u32 << 16, WHEELED), &g);
        assert!(actual.is_empty(), "{actual:?}");
        let actual = t.feed(rec(5, 9, RIGHT, 0), &g);
        assert!(actual.is_empty(), "{actual:?}");
        t.feed(rec(5, 9, 0, 0), &g);
        // 区切りの上は何も渡さない
        let actual = t.feed(rec(40, 9, LEFT, 0), &g);
        assert!(actual.is_empty(), "{actual:?}");
    }

    #[test]
    fn 本文で押したドラッグは見出しの上でも押したペインのまま() {
        let (g, mut t) = (two_with_bars(), MouseTracker::new());
        feed(&mut t, rec(5, 3, LEFT, 0), &g);
        let m = feed(&mut t, rec(5, 9, LEFT, MOVED), &g);
        assert_eq!(
            (kinds(&m), m[0].at.view, m[0].at.row),
            (vec![MouseKind::Drag(MouseButton::Left)], ViewId(1), 9)
        );
        let u = feed(&mut t, rec(5, 9, 0, 0), &g);
        assert_eq!(kinds(&u), [MouseKind::Up(MouseButton::Left)]);
    }

    #[test]
    fn 共有の幾何は丸ごと差し替える() {
        let s = SharedGeom::default();
        assert_eq!(*s.get(), Geometry::default());
        s.set(two());
        assert_eq!(*s.get(), two());
        s.set(geom());
        assert_eq!(*s.get(), geom());
    }

    /// 2 つの文書を開いてタブラインを出した 40 × 6 の Frame の幾何 (`glyphs` で並べる)。
    fn with_tab_line(glyphs: &Glyphs) -> (Geometry, eve_view::Frame) {
        let mut h = eve_testkit::Harness::new()
            .with_size(40, 6)
            .with_file("/w/a.txt", b"a\n")
            .with_file("/w/bb.txt", b"b\n");
        h.open("/w/a.txt");
        h.run_jobs();
        h.open("/w/bb.txt");
        h.run_jobs();
        let f = h.frame();
        (Geometry::from_frame(&f, glyphs), f)
    }

    // verifies: AC-FR-510-2
    #[test]
    fn タブの左の押下は_chrome_mouse_でタブの文書を指し_間と行の残りは何もしない() {
        let (g, f) = with_tab_line(&Glyphs::unicode());
        let docs: Vec<DocumentId> = f
            .tab_line
            .as_ref()
            .expect("タブライン")
            .tabs
            .iter()
            .map(|t| t.doc)
            .collect();
        // `[ a.txt ] [▶bb.txt ]`: 1 つ目は 0..9 列、2 つ目は 10..20 列
        assert_eq!(
            g.tabs
                .iter()
                .map(|t| (t.doc, t.rect.col, t.rect.cols))
                .collect::<Vec<_>>(),
            vec![(docs[0], 0, 9), (docs[1], 10, 10)]
        );
        let mut t = MouseTracker::new();
        for (col, doc, at) in [
            (0, docs[0], 0),
            (8, docs[0], 8),
            (10, docs[1], 0),
            (19, docs[1], 9),
        ] {
            let e = t.feed(rec(col, 0, LEFT, 0), &g);
            assert_eq!(
                e,
                vec![InputEvent::ChromeMouse(ChromeMouseEvent {
                    kind: MouseKind::Down(MouseButton::Left),
                    mods: Modifiers::NONE,
                    target: ChromeTarget::Tab { doc },
                    row: 0,
                    col: at,
                })],
                "{col} 列"
            );
            // 離す・押したままのドラッグは渡さない
            let actual = t.feed(rec(col + 1, 0, LEFT, MOVED), &g);
            assert!(actual.is_empty(), "{actual:?}");
            let actual = t.feed(rec(col, 0, 0, 0), &g);
            assert!(actual.is_empty(), "{actual:?}");
        }
        // タブの間・行の残り・タブの上のホイールと右ボタンは何もしない
        for r in [
            rec(9, 0, LEFT, 0),
            rec(30, 0, LEFT, 0),
            rec(3, 0, 120u32 << 16, WHEELED),
            rec(3, 0, RIGHT, 0),
        ] {
            let actual = t.feed(r, &g);
            assert!(actual.is_empty(), "{r:?}: {actual:?}");
            t.feed(rec(r.col, 0, 0, 0), &g);
        }
    }

    // verifies: AC-FR-510-2
    #[test]
    fn タブラインを出すとペインの本文は_1_行下で_本文の左上が原点のまま() {
        let (g, f) = with_tab_line(&Glyphs::unicode());
        assert_eq!(f.panes[0].rect.row, 0, "Frame は編集面の座標");
        assert_eq!(g.panes[0].rect.row, 1, "端末の座標は 1 行下");
        let mut t = MouseTracker::new();
        let d = feed(&mut t, rec(5, 1, LEFT, 0), &g);
        assert_eq!(kinds(&d), [MouseKind::Down(MouseButton::Left)]);
        assert_eq!((d[0].at.row, d[0].at.col), (0, 5));
        // 本文で押したドラッグはタブラインの上でもペインのまま (面の外の負の行)
        let m = feed(&mut t, rec(5, 0, LEFT, MOVED), &g);
        assert_eq!(
            (kinds(&m), m[0].at.row),
            (vec![MouseKind::Drag(MouseButton::Left)], -1)
        );
        feed(&mut t, rec(5, 0, 0, 0), &g);
    }
}
