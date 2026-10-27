//! 自前の仮想画面モデル (docs/09 §3.5、OQ-68)。端末へ出力するバイト列 (CUP・ED・EL・SGR・DECSET など) を解釈し、
//! **セル幅を設定で切り替えて** 画面を再現する。`vt100` や `TestBackend` は unicode-width 前提で、曖昧幅 2 の端末や
//! 半角カナの濁点 (U+FF9E / U+FF9F。端末が別に 1 セル進める、S-06) を再現できないので、幅の食い違いの検査はこれで行う。
//!
//! 解釈するのは eve-tui が出す列だけ: 未知のエスケープは `panic` にして、出力器が想定外の列を出したことに気づけるようにする。

use unicode_width::UnicodeWidthChar;

/// 端末ごとに食い違う幅の設定 (09 §3.5)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidthConfig {
    /// East Asian Ambiguous の文字の幅 (1 または 2)。
    pub ambiguous: u8,
    /// 絵文字表示を持つ記号 (`⚠` `✖` `ℹ` など。異体字セレクタなし) の幅。
    pub emoji: u8,
    /// 異体字セレクタ 16 (U+FE0F) が直前の文字を 2 セルにするか (2 のとき広げる)。
    pub vs16: u8,
    /// 私用領域の文字の幅。
    pub pua: u8,
}

impl Default for WidthConfig {
    fn default() -> WidthConfig {
        WidthConfig {
            ambiguous: 1,
            emoji: 1,
            vs16: 1,
            pua: 1,
        }
    }
}

impl WidthConfig {
    pub fn ambiguous(width: u8) -> WidthConfig {
        WidthConfig {
            ambiguous: width,
            ..WidthConfig::default()
        }
    }

    fn width(&self, ch: char) -> usize {
        match ch {
            // 半角カナの濁点・半濁点は結合せず、別に 1 セル進む (S-06)
            '\u{FF9E}' | '\u{FF9F}' => 1,
            '\u{FE0F}' => 0,
            '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}' => {
                usize::from(self.pua)
            }
            '\u{26A0}' | '\u{2716}' | '\u{2139}' | '\u{2713}' => usize::from(self.emoji),
            c if self.ambiguous == 2 => c.width_cjk().unwrap_or(0),
            c => c.width().unwrap_or(0),
        }
    }
}

/// セルの属性 (SGR の結果)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attr {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
    /// `38;2;r;g;b` は `rgb(r,g,b)`、`38;5;n` は `idx(n)`。既定色は `None`。
    pub fg: Option<String>,
    pub bg: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Cell {
    /// 空文字は「幅 2 の文字の後ろ半分」。
    text: String,
    attr: Attr,
}

fn blank() -> Cell {
    Cell {
        text: " ".to_owned(),
        attr: Attr::default(),
    }
}

/// 仮想画面。
#[derive(Debug)]
pub struct VirtualScreen {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
    x: usize,
    y: usize,
    attr: Attr,
    cursor_visible: bool,
    cursor_shape: Option<u8>,
    sync_depth: i32,
    hidden_during_sync: bool,
    alt_screen: bool,
    autowrap: bool,
    title: Option<String>,
    width: WidthConfig,
    /// 最後に出力した CUP の位置 (列, 行。0 始まり)。
    last_cup: Option<(usize, usize)>,
    bytes_fed: usize,
}

impl VirtualScreen {
    pub fn new(cols: u16, rows: u16, width: WidthConfig) -> VirtualScreen {
        let (cols, rows) = (cols as usize, rows as usize);
        VirtualScreen {
            cols,
            rows,
            cells: vec![blank(); cols * rows],
            x: 0,
            y: 0,
            attr: Attr::default(),
            cursor_visible: true,
            cursor_shape: None,
            sync_depth: 0,
            hidden_during_sync: false,
            alt_screen: false,
            autowrap: true,
            title: None,
            width,
            last_cup: None,
            bytes_fed: 0,
        }
    }

    /// 出力の 1 まとまりを解釈する。
    pub fn feed(&mut self, s: &str) {
        self.bytes_fed += s.len();
        let cs: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < cs.len() {
            let c = cs[i];
            match c {
                '\x1b' => i = self.escape(&cs, i),
                '\x07' => i += 1,
                '\r' => {
                    self.x = 0;
                    i += 1;
                }
                '\n' => {
                    self.y = (self.y + 1).min(self.rows.saturating_sub(1));
                    i += 1;
                }
                c if (c as u32) < 0x20 => panic!("未知の制御文字: {:?}", c),
                c => {
                    self.put(c);
                    i += 1;
                }
            }
        }
    }

    fn escape(&mut self, cs: &[char], i: usize) -> usize {
        match cs.get(i + 1) {
            Some('[') => {
                let mut j = i + 2;
                let mut params = String::new();
                while j < cs.len() && !('\x40'..='\x7e').contains(&cs[j]) {
                    params.push(cs[j]);
                    j += 1;
                }
                assert!(j < cs.len(), "CSI が閉じていない: {params:?}");
                self.csi(&params, cs[j]);
                j + 1
            }
            Some(']') => {
                // OSC: BEL または ST (ESC \) まで
                let mut j = i + 2;
                let mut body = String::new();
                while j < cs.len()
                    && !(cs[j] == '\x07' || (cs[j] == '\x1b' && cs.get(j + 1) == Some(&'\\')))
                {
                    body.push(cs[j]);
                    j += 1;
                }
                assert!(j < cs.len(), "OSC が閉じていない: {body:?}");
                if let Some(t) = body.strip_prefix("0;") {
                    self.title = Some(t.to_owned());
                }
                if cs[j] == '\x07' { j + 1 } else { j + 2 }
            }
            other => panic!("未知のエスケープ: ESC {other:?}"),
        }
    }

    fn csi(&mut self, params: &str, fin: char) {
        let nums = |p: &str| -> Vec<usize> {
            p.split(';')
                .map(|v| v.parse::<usize>().unwrap_or(0))
                .collect()
        };
        match fin {
            'H' => {
                let n = nums(params);
                let r = n.first().copied().unwrap_or(1).max(1);
                let c = n.get(1).copied().unwrap_or(1).max(1);
                self.y = (r - 1).min(self.rows.saturating_sub(1));
                self.x = (c - 1).min(self.cols.saturating_sub(1));
                self.last_cup = Some((self.x, self.y));
            }
            'J' if params == "2" => self.cells.iter_mut().for_each(|c| *c = blank()),
            'K' if params == "0" || params.is_empty() => {
                for x in self.x..self.cols {
                    self.cells[self.y * self.cols + x] = Cell {
                        text: " ".to_owned(),
                        attr: self.attr.clone(),
                    };
                }
            }
            'm' => self.sgr(params),
            'q' if params.ends_with(' ') => {
                self.cursor_shape = params.trim().parse().ok();
            }
            'h' | 'l' => {
                let on = fin == 'h';
                match params {
                    "?25" => {
                        self.cursor_visible = on;
                        if !on && self.sync_depth > 0 {
                            self.hidden_during_sync = true;
                        }
                    }
                    "?2026" => self.sync_depth += if on { 1 } else { -1 },
                    "?1049" => self.alt_screen = on,
                    "?7" => self.autowrap = on,
                    other => panic!("未知の DEC モード: {other:?}"),
                }
            }
            other => panic!("未知の CSI: {params:?} {other:?}"),
        }
    }

    fn sgr(&mut self, params: &str) {
        let n: Vec<usize> = if params.is_empty() {
            vec![0]
        } else {
            params
                .split(';')
                .map(|v| v.parse::<usize>().unwrap_or(0))
                .collect()
        };
        let mut k = 0;
        while k < n.len() {
            match n[k] {
                0 => self.attr = Attr::default(),
                1 => self.attr.bold = true,
                2 => self.attr.dim = true,
                3 => self.attr.italic = true,
                4 => self.attr.underline = true,
                7 => self.attr.reverse = true,
                39 => self.attr.fg = None,
                49 => self.attr.bg = None,
                c @ (38 | 48) => {
                    let color = match n.get(k + 1) {
                        Some(2) => {
                            let v = (n.get(k + 2), n.get(k + 3), n.get(k + 4));
                            k += 4;
                            match v {
                                (Some(r), Some(g), Some(b)) => format!("rgb({r},{g},{b})"),
                                _ => panic!("SGR の色が足りない"),
                            }
                        }
                        Some(5) => {
                            let v = n.get(k + 2).copied().unwrap_or(0);
                            k += 2;
                            format!("idx({v})")
                        }
                        other => panic!("未知の SGR の色: {other:?}"),
                    };
                    if c == 38 {
                        self.attr.fg = Some(color);
                    } else {
                        self.attr.bg = Some(color);
                    }
                }
                other => panic!("未知の SGR: {other}"),
            }
            k += 1;
        }
    }

    fn put(&mut self, ch: char) {
        if ch == '\u{FE0F}' {
            // 異体字セレクタ: 直前の文字を 2 セルに広げる端末の再現
            if self.width.vs16 == 2 && self.x > 0 && self.x < self.cols {
                let i = self.y * self.cols + self.x - 1;
                if self.cells[i].text.chars().count() == 1 && self.x < self.cols {
                    self.cells[i].text.push(ch);
                    self.cells[self.y * self.cols + self.x] = Cell {
                        text: String::new(),
                        attr: self.attr.clone(),
                    };
                    self.x = (self.x + 1).min(self.cols - 1);
                    return;
                }
            }
            let i = self.y * self.cols + self.x.saturating_sub(1);
            self.cells[i].text.push(ch);
            return;
        }
        let w = self.width.width(ch);
        if w == 0 {
            // 結合文字: 直前のセルに足す
            let i = self.y * self.cols + self.x.saturating_sub(1);
            self.cells[i].text.push(ch);
            return;
        }
        if self.x + w > self.cols {
            // 自動折り返しは無効 (ENTER で ?7l)。はみ出す分は捨てる
            return;
        }
        let i = self.y * self.cols + self.x;
        // 幅 2 の文字の後ろ半分を上書きするときは、前半を空白にする
        if self.cells[i].text.is_empty() && self.x > 0 {
            self.cells[i - 1].text = " ".to_owned();
        }
        self.cells[i] = Cell {
            text: ch.to_string(),
            attr: self.attr.clone(),
        };
        for k in 1..w {
            // 前半を上書きされる後ろ半分 (次の文字の前半) も空にする
            self.cells[i + k] = Cell {
                text: String::new(),
                attr: self.attr.clone(),
            };
        }
        // 幅 2 の文字の後ろ半分が別の文字の前半を壊す場合: その文字の後ろ半分を空白にする
        if let Some(next) = self.cells.get_mut(i + w)
            && next.text.is_empty()
            && i + w < (self.y + 1) * self.cols
        {
            next.text = " ".to_owned();
        }
        self.x += w;
        if self.x >= self.cols {
            self.x = self.cols - 1;
        }
    }

    // ---- 検査用の読み出し ----

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// 各行の文字列 (幅 2 の後ろ半分は出さない。末尾の空白は残す)。
    pub fn lines(&self) -> Vec<String> {
        (0..self.rows)
            .map(|y| {
                self.cells[y * self.cols..(y + 1) * self.cols]
                    .iter()
                    .map(|c| c.text.as_str())
                    .collect()
            })
            .collect()
    }

    /// 1 行の文字列 (末尾の空白を除く)。
    pub fn line(&self, y: usize) -> String {
        self.lines()[y].trim_end().to_owned()
    }

    /// (x, y) のセルの文字 (幅 2 の後ろ半分は空文字)。
    pub fn text_at(&self, x: usize, y: usize) -> &str {
        &self.cells[y * self.cols + x].text
    }

    pub fn attr_at(&self, x: usize, y: usize) -> &Attr {
        &self.cells[y * self.cols + x].attr
    }

    /// カーソルの位置 (列, 行)。
    pub fn cursor(&self) -> (usize, usize) {
        (self.x, self.y)
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// DECSCUSR の最後の値 (2 = ブロック、4 = 下線、6 = バー)。
    pub fn cursor_shape(&self) -> Option<u8> {
        self.cursor_shape
    }

    /// 同期出力 (BSU / ESU) の内側でカーソルを隠したことがあるか (04 §4.6: 隠さない)。
    pub fn hid_cursor_during_sync(&self) -> bool {
        self.hidden_during_sync
    }

    /// 同期出力が閉じているか (BSU と ESU が釣り合っている)。
    pub fn sync_balanced(&self) -> bool {
        self.sync_depth == 0
    }

    pub fn in_alt_screen(&self) -> bool {
        self.alt_screen
    }

    pub fn autowrap(&self) -> bool {
        self.autowrap
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// 最後の CUP の位置。毎フレームの出力の最後の CUP がカーソル位置になっていることの検査に使う。
    pub fn last_cup(&self) -> Option<(usize, usize)> {
        self.last_cup
    }

    /// これまでに解釈した出力のバイト数。
    pub fn bytes_fed(&self) -> usize {
        self.bytes_fed
    }
}
