//! 本文に入れる規則 (docs/03 §8.3.1 の「本文に入れる規則」、01 §5.5 の NFR-304 AC-4)。
//!
//! 不正バイトのエスケープ文字 (`U+10FF00 + b`) を、黙ってバイトにも別の文字にもしないための規則を 1 か所に置く。
//! 挿入の入口 (貼り付け `paste_text`・`edit.insert_text`・置換の文字列) は、本文に入れる文字列をここで決める
//! (改行の正規化 (§8.4) と同じ段階、挿入するテキストを Transaction にする前)。
//!
//! - **経路 (1) 内部のコピー**: 内部レジスタと照合して一致し、挿入先が同じ文字コードのバイトの文書なら、エスケープ文字のまま入れる。
//! - **経路 (2) 外からの文字列**: 挿入先が `pua_escape != DisabledCollision` なら、`U+10FF00..U+10FFFF` を `<U+10FFNN>` の
//!   文字列にする (衝突の文書 (a) にはそのまま入れる)。
//! - 表記の文字列 `<0xNN>` をバイトに戻すことはしない。

use std::borrow::Cow;

use eve_encoding::{Encoding, PuaEscapeMode};
use eve_text::LineEnding;

use crate::command::clipboard::ClipboardSource;
use crate::document::TextFormat;
use crate::ui_model::keys;

/// 不正バイトのエスケープ文字 (`U+10FF00..=U+10FFFF`、§8.3) か。
pub(crate) fn is_escape(c: char) -> bool {
    ('\u{10FF00}'..='\u{10FFFF}').contains(&c)
}

/// 文書の、この規則に効く 2 つの値 (コピー元・挿入先で共通)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EscapeContext {
    /// `U+10FF00..` を印 (不正バイト) として扱い、表記にするか (§8.3.1 の「表記にするかどうかの条件」:
    /// `pua_escape != DisabledCollision`)。偽は衝突の文書で、私用文字をそのまま扱う。
    pub(crate) notation: bool,
    /// 本文のエスケープ文字がどの文字コードのバイトか (`TextFormat.escape_encoding`)。
    pub(crate) escape_encoding: Option<Encoding>,
}

impl EscapeContext {
    pub(crate) fn of(format: &TextFormat) -> Self {
        EscapeContext {
            notation: format.pua_escape != PuaEscapeMode::DisabledCollision,
            escape_encoding: format.escape_encoding,
        }
    }
}

/// 挿入したときの知らせ (04 §15.2 の `info.paste.*`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InsertNotice {
    /// 内部のコピーを同じ文字コードのバイトの文書に貼り、エスケープ文字のまま入れた (`info.paste.as_bytes`)。
    AsBytes(u64),
    /// 不正バイトを含む文書からのコピーを、バイトとして扱えない文書に `<0xNN>` の文字列で貼った (`info.paste.escape_as_text`)。
    EscapeAsText(u64),
    /// 照合が外れ、`<0xNN>` の形の文字列を文字列のまま入れた (`info.paste.notation_as_text`)。
    NotationAsText,
    /// 外から来た `U+10FF00..` を `<U+10FFNN>` の文字列にした (`info.paste.pua_as_text`)。
    PuaAsText(u64),
}

impl InsertNotice {
    /// 文言キー。
    pub(crate) fn key(self) -> &'static str {
        match self {
            InsertNotice::AsBytes(_) => keys::INFO_PASTE_AS_BYTES,
            InsertNotice::EscapeAsText(_) => keys::INFO_PASTE_ESCAPE_AS_TEXT,
            InsertNotice::NotationAsText => keys::INFO_PASTE_NOTATION_AS_TEXT,
            InsertNotice::PuaAsText(_) => keys::INFO_PASTE_PUA_AS_TEXT,
        }
    }

    /// `{count}` の値 (`NotationAsText` は件数を持たない)。
    pub(crate) fn count(self) -> Option<u64> {
        match self {
            InsertNotice::AsBytes(n)
            | InsertNotice::EscapeAsText(n)
            | InsertNotice::PuaAsText(n) => Some(n),
            InsertNotice::NotationAsText => None,
        }
    }
}

/// エスケープ文字 `c` の表記 `<0xNN>` (表示と同じ、§13.2 の書き分けの「バイト」)。
fn byte_notation(c: char) -> [char; 6] {
    let b = u32::from(c).to_le_bytes()[0]; // U+10FF00 の下位 8 ビットは 0
    let hex = |n: u8| char::from(b"0123456789ABCDEF"[usize::from(n)]);
    ['<', '0', 'x', hex(b >> 4), hex(b & 0xF), '>']
}

/// `s` のエスケープ文字を表記 `<0xNN>` にして `out` に足す (コピーの文字列化、§8.3.1 の「コピー」)。
pub(crate) fn push_notation(out: &mut String, s: &str) {
    // エスケープ文字は UTF-8 の先頭バイトが 0xF4。無ければそのまま足す
    if !s.as_bytes().contains(&0xF4) {
        out.push_str(s);
        return;
    }
    for c in s.chars() {
        if is_escape(c) {
            out.extend(byte_notation(c));
        } else {
            out.push(c);
        }
    }
}

/// 経路 (2) の外からの文字列: 挿入先が表記にする文書なら、`U+10FF00..` の各文字を `<U+10FFNN>` (ASCII の 10 文字。
/// §13.2 の書き分けの「コードポイント」) にする。返すのは (入れる文字列, 置き換えた数)。
pub(crate) fn external_text(text: &str, target: EscapeContext) -> (Cow<'_, str>, u64) {
    if !target.notation || !text.as_bytes().contains(&0xF4) {
        return (Cow::Borrowed(text), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut n = 0u64;
    for c in text.chars() {
        if is_escape(c) {
            n += 1;
            out.push_str(&format!("<U+{:04X}>", c as u32));
        } else {
            out.push(c);
        }
    }
    if n == 0 {
        return (Cow::Borrowed(text), 0);
    }
    (Cow::Owned(out), n)
}

/// `s` に `<0x[0-9A-F]{2}>` の形があるか (照合が外れたときの知らせの条件)。
pub(crate) fn has_byte_notation(s: &str) -> bool {
    s.as_bytes().windows(6).any(|w| {
        w[0] == b'<'
            && w[1] == b'0'
            && w[2] == b'x'
            && matches!(w[3], b'0'..=b'9' | b'A'..=b'F')
            && matches!(w[4], b'0'..=b'9' | b'A'..=b'F')
            && w[5] == b'>'
    })
}

/// 改行をそろえた文字の列 (`\r\n`・単独の `\r` を `\n` にする。照合の両側に同じ正規化をかける)。
struct LfChars<I: Iterator<Item = char>> {
    inner: std::iter::Peekable<I>,
}

impl<I: Iterator<Item = char>> Iterator for LfChars<I> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self.inner.next()? {
            '\r' => {
                if self.inner.peek() == Some(&'\n') {
                    self.inner.next();
                }
                Some('\n')
            }
            c => Some(c),
        }
    }
}

fn lf_chars<I: Iterator<Item = char>>(it: I) -> LfChars<I> {
    LfChars {
        inner: it.peekable(),
    }
}

/// 内部レジスタを `to_text` と同じ規則で文字にした列 (文字列を作らずに比べる)。
fn register_chars(src: &ClipboardSource) -> impl Iterator<Item = char> + '_ {
    use eve_text::TextRead;
    let notation = src.notation;
    src.snapshot
        .chunks(src.range)
        .flat_map(str::chars)
        .flat_map(move |c| {
            let (buf, len) = if notation && is_escape(c) {
                (byte_notation(c), 6)
            } else {
                ([c; 6], 1)
            };
            buf.into_iter().take(len)
        })
}

impl ClipboardSource {
    /// 届いた文字列が、この内部レジスタを `to_text` で文字列にしたものと、両側の改行をそろえて一致するか
    /// (§8.3.1 の「内部レジスタとの照合」)。先頭から比べて最初の違いで打ち切る。
    pub(crate) fn matches_text(&self, arrived: &str) -> bool {
        lf_chars(register_chars(self)).eq(lf_chars(arrived.chars()))
    }

    /// 範囲の中のエスケープ文字の数。
    pub(crate) fn escape_count(&self) -> u64 {
        use eve_text::TextRead;
        self.snapshot
            .chunks(self.range)
            .filter(|c| c.as_bytes().contains(&0xF4))
            .flat_map(str::chars)
            .filter(|&c| is_escape(c))
            .count() as u64
    }
}

/// 貼り付けで本文に入れるもの。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PastePlan {
    /// 入れる文字列 (改行の正規化と置き換えの後)。
    pub(crate) text: String,
    /// 知らせ (出す順)。
    pub(crate) notices: Vec<InsertNotice>,
}

/// 貼り付けの入口 (1) (`paste_text`。`edit.paste` の応答と端末からの貼り付け) の規則。
///
/// - `arrived`: 届いた文字列 (読めなかったときは `None`)。
/// - `register`: 内部レジスタ。`stale` はその「システム側は古い」印 (最後のコピーの書き出しが失敗・上限超過)。
/// - `target`: 挿入先の文書。`eol` はその `default_eol`、`normalize_eol` は `editor.paste_normalize_eol`。
///
/// 入れるものが無い (届かず、内部レジスタも空) ときは `None`。
pub(crate) fn resolve_paste(
    arrived: Option<&str>,
    register: Option<&ClipboardSource>,
    stale: bool,
    target: EscapeContext,
    eol: LineEnding,
    normalize_eol: bool,
) -> Option<PastePlan> {
    let eol_rule = |s: &str| -> String {
        if normalize_eol {
            crate::command::edit::normalize_eol(s, eol)
        } else {
            s.to_owned()
        }
    };
    // 一致とみなす内部レジスタ: システム側が古い・届かなかったときは見ずに一致、ほかは両側の改行をそろえて比べる
    let matched = register.filter(|r| match arrived {
        Some(a) if !stale => r.matches_text(a),
        _ => true,
    });
    let mut notices = Vec::new();
    let text = match matched {
        Some(r) if r.escape_origin.is_some() && r.escape_origin == target.escape_encoding => {
            // 経路 (1): 内部レジスタの内容 (エスケープ文字のまま) を、設定どおりの改行の正規化をかけて入れる
            use eve_text::TextRead;
            let raw = r.snapshot.slice_to_string(r.range);
            let n = r.escape_count();
            if n > 0 {
                notices.push(InsertNotice::AsBytes(n));
            }
            eol_rule(&raw)
        }
        Some(r) => {
            // 一致したが挿入先ではバイトとして扱えない: 届いた文字列 (届かない・古いときは内部レジスタの文字列) を経路 (2) で
            let s = match arrived {
                Some(a) if !stale => eol_rule(a),
                _ => eol_rule(&r.to_text()),
            };
            if r.escape_origin.is_some() {
                let n = r.escape_count();
                if n > 0 {
                    notices.push(InsertNotice::EscapeAsText(n));
                }
            }
            let (s, pua) = external_text(&s, target);
            if pua > 0 {
                notices.push(InsertNotice::PuaAsText(pua));
            }
            s.into_owned()
        }
        None => {
            // 照合が外れた (または内部レジスタが空): 経路 (2)
            let a = arrived?;
            if register.is_some_and(|r| r.escape_origin.is_some()) && has_byte_notation(a) {
                notices.push(InsertNotice::NotationAsText);
            }
            let s = eol_rule(a);
            let (s, pua) = external_text(&s, target);
            if pua > 0 {
                notices.push(InsertNotice::PuaAsText(pua));
            }
            s.into_owned()
        }
    };
    Some(PastePlan { text, notices })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_text::{ByteOff, ByteRange};

    fn src(text: &str, notation: bool, origin: Option<Encoding>) -> ClipboardSource {
        ClipboardSource {
            snapshot: eve_text::Text::from(text).snapshot(),
            range: ByteRange::new(ByteOff(0), ByteOff(text.len())),
            notation,
            escape_origin: origin,
        }
    }

    const BYTES_SJIS: EscapeContext = EscapeContext {
        notation: true,
        escape_encoding: Some(Encoding::ShiftJis),
    };
    const PLAIN: EscapeContext = EscapeContext {
        notation: true,
        escape_encoding: None,
    };
    const COLLISION: EscapeContext = EscapeContext {
        notation: false,
        escape_encoding: None,
    };

    #[test]
    fn 表記は大文字の_16_進_2_桁() {
        let mut s = String::new();
        push_notation(&mut s, "a\u{10FF9A}\u{10FF00}\u{10FFFF}b");
        assert_eq!(s, "a<0x9A><0x00><0xFF>b");
    }

    #[test]
    fn 外からの私用文字はコードポイントの表記で_衝突の文書ではそのまま() {
        let (s, n) = external_text("x\u{10FF80}y\u{10FFFF}", PLAIN);
        assert_eq!((s.as_ref(), n), ("x<U+10FF80>y<U+10FFFF>", 2));
        assert_eq!("<U+10FF80>".len(), 10);
        let (s, n) = external_text("x\u{10FF80}", COLLISION);
        assert_eq!((s.as_ref(), n), ("x\u{10FF80}", 0));
        // 範囲の外の私用文字 (U+10FEFF) は変えない
        let (s, n) = external_text("\u{10FEFF}", PLAIN);
        assert_eq!((s.as_ref(), n), ("\u{10FEFF}", 0));
    }

    #[test]
    fn 表記の形は大文字の_16_進だけ() {
        assert!(has_byte_notation("a<0x9A>b"));
        assert!(!has_byte_notation("<0x9a>"));
        assert!(!has_byte_notation("<0X9A>"));
        assert!(!has_byte_notation("<0x9>"));
        assert!(!has_byte_notation("<U+10FF9A>"));
    }

    #[test]
    fn 照合は両側の改行をそろえて比べる() {
        let r = src("a\r\n\u{10FF9A}\rb", true, Some(Encoding::ShiftJis));
        assert!(r.matches_text("a\n<0x9A>\nb"));
        assert!(r.matches_text("a\r\n<0x9A>\r\nb"));
        assert!(!r.matches_text("a\n<0x9A>\nb "), "空白が足された");
        assert!(!r.matches_text("a\n<0x9A>\n"), "短い");
        assert!(!r.matches_text("a\n\u{10FF9A}\nb"), "表記でない");
    }

    #[test]
    fn 同じ文字コードのバイトの文書にはバイトのまま() {
        let r = src("x\u{10FF9A}\r\ny", true, Some(Encoding::ShiftJis));
        let p = resolve_paste(
            Some("x<0x9A>\r\ny"),
            Some(&r),
            false,
            BYTES_SJIS,
            LineEnding::Lf,
            true,
        )
        .unwrap();
        assert_eq!(p.text, "x\u{10FF9A}\ny");
        assert_eq!(p.notices, vec![InsertNotice::AsBytes(1)]);
        // 正規化しない設定では内部レジスタの改行のまま
        let p = resolve_paste(
            Some("x<0x9A>\ny"),
            Some(&r),
            false,
            BYTES_SJIS,
            LineEnding::Lf,
            false,
        )
        .unwrap();
        assert_eq!(p.text, "x\u{10FF9A}\r\ny");
    }

    #[test]
    fn 違う文字コードの文書には表記の文字列で知らせる() {
        let r = src("x\u{10FF9A}", true, Some(Encoding::ShiftJis));
        let target = EscapeContext {
            notation: true,
            escape_encoding: Some(Encoding::EucJp),
        };
        let p = resolve_paste(
            Some("x<0x9A>"),
            Some(&r),
            false,
            target,
            LineEnding::Lf,
            true,
        )
        .unwrap();
        assert_eq!(p.text, "x<0x9A>");
        assert_eq!(p.notices, vec![InsertNotice::EscapeAsText(1)]);
    }

    #[test]
    fn 照合が外れたら届いた文字列で_表記の形があれば知らせる() {
        let r = src("x\u{10FF9A}", true, Some(Encoding::ShiftJis));
        let p = resolve_paste(
            Some("y<0x9A>\u{10FF80}"),
            Some(&r),
            false,
            BYTES_SJIS,
            LineEnding::Lf,
            true,
        )
        .unwrap();
        assert_eq!(p.text, "y<0x9A><U+10FF80>");
        assert_eq!(
            p.notices,
            vec![InsertNotice::NotationAsText, InsertNotice::PuaAsText(1)]
        );
    }

    #[test]
    fn システム側が古い印があれば届いた文字列を見ない() {
        let r = src("x\u{10FF9A}", true, Some(Encoding::ShiftJis));
        let p = resolve_paste(
            Some("OLD"),
            Some(&r),
            true,
            BYTES_SJIS,
            LineEnding::Lf,
            true,
        )
        .unwrap();
        assert_eq!(p.text, "x\u{10FF9A}");
        let p = resolve_paste(Some("OLD"), Some(&r), true, PLAIN, LineEnding::Lf, true).unwrap();
        assert_eq!(p.text, "x<0x9A>", "古い文字列ではなく内部レジスタの表記");
        assert_eq!(p.notices, vec![InsertNotice::EscapeAsText(1)]);
    }

    #[test]
    fn 衝突の文書からのコピーはバイトの文書へコードポイントの表記で() {
        let r = src("x\u{10FF9A}", false, None);
        assert_eq!(r.to_text(), "x\u{10FF9A}");
        let p = resolve_paste(
            Some("x\u{10FF9A}"),
            Some(&r),
            false,
            BYTES_SJIS,
            LineEnding::Lf,
            true,
        )
        .unwrap();
        assert_eq!(p.text, "x<U+10FF9A>");
        assert_eq!(p.notices, vec![InsertNotice::PuaAsText(1)]);
        let p = resolve_paste(
            Some("x\u{10FF9A}"),
            Some(&r),
            false,
            COLLISION,
            LineEnding::Lf,
            true,
        )
        .unwrap();
        assert_eq!(p.text, "x\u{10FF9A}");
        assert_eq!(p.notices, vec![]);
    }

    #[test]
    fn 届かず内部レジスタも空なら何もしない() {
        assert_eq!(
            resolve_paste(None, None, false, PLAIN, LineEnding::Lf, true),
            None
        );
    }

    // ---------- 性質テスト (03 §22.1 の「貼り付けのエスケープ文字」) ----------
    //
    // 参照モデルは §8.3.1 の規則の表の素朴な実装 (文字列の置き換えと、両側を正規化した文字列の等しさ)。件数は
    // `PROPTEST_CASES` で増やせる。

    use proptest::prelude::*;

    fn lf(s: &str) -> String {
        s.replace("\r\n", "\n").replace('\r', "\n")
    }

    fn naive_escape(s: &str, f: impl Fn(u32) -> String) -> String {
        s.chars()
            .map(|c| {
                if ('\u{10FF00}'..='\u{10FFFF}').contains(&c) {
                    f(c as u32)
                } else {
                    c.to_string()
                }
            })
            .collect()
    }

    fn naive_bytes(s: &str) -> String {
        naive_escape(s, |u| format!("<0x{:02X}>", u - 0x10FF00))
    }

    fn naive_codepoints(s: &str) -> String {
        naive_escape(s, |u| format!("<U+{u:X}>"))
    }

    fn escapes(s: &str) -> u64 {
        s.chars()
            .filter(|c| ('\u{10FF00}'..='\u{10FFFF}').contains(c))
            .count() as u64
    }

    fn naive_has_notation(s: &str) -> bool {
        let c: Vec<char> = s.chars().collect();
        c.windows(6).any(|w| {
            let hex = |x: char| x.is_ascii_digit() || ('A'..='F').contains(&x);
            w[0] == '<' && w[1] == '0' && w[2] == 'x' && hex(w[3]) && hex(w[4]) && w[5] == '>'
        })
    }

    /// 規則の表の素朴な実装。返すのは (入れる文字列, 知らせ)。
    fn model(
        arrived: Option<&str>,
        reg: Option<(&str, bool, Option<Encoding>)>,
        stale: bool,
        target: EscapeContext,
        eol: LineEnding,
        normalize: bool,
    ) -> Option<(String, Vec<InsertNotice>)> {
        let norm = |s: &str| {
            if normalize {
                lf(s).replace('\n', eol.as_str())
            } else {
                s.to_owned()
            }
        };
        let to_ext = |s: String| {
            if target.notation {
                let n = escapes(&s);
                (naive_codepoints(&s), n)
            } else {
                (s, 0)
            }
        };
        let reg_text = reg.map(|(raw, notation, _)| {
            if notation {
                naive_bytes(raw)
            } else {
                raw.to_owned()
            }
        });
        let matched = match (reg, arrived) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(_), Some(a)) => stale || lf(a) == lf(reg_text.as_deref().unwrap()),
        };
        let mut notices = Vec::new();
        if matched {
            let (raw, _, origin) = reg.unwrap();
            if origin.is_some() && origin == target.escape_encoding {
                if escapes(raw) > 0 {
                    notices.push(InsertNotice::AsBytes(escapes(raw)));
                }
                return Some((norm(raw), notices));
            }
            let src = match arrived {
                Some(a) if !stale => a.to_owned(),
                _ => reg_text.unwrap(),
            };
            if origin.is_some() && escapes(raw) > 0 {
                notices.push(InsertNotice::EscapeAsText(escapes(raw)));
            }
            let (s, n) = to_ext(norm(&src));
            if n > 0 {
                notices.push(InsertNotice::PuaAsText(n));
            }
            return Some((s, notices));
        }
        let a = arrived?;
        if reg.is_some_and(|(_, _, o)| o.is_some()) && naive_has_notation(a) {
            notices.push(InsertNotice::NotationAsText);
        }
        let (s, n) = to_ext(norm(a));
        if n > 0 {
            notices.push(InsertNotice::PuaAsText(n));
        }
        Some((s, notices))
    }

    fn piece() -> impl Strategy<Value = &'static str> {
        prop::sample::select(vec![
            "a",
            "あ",
            "<",
            "0",
            "x",
            "A",
            "9",
            ">",
            "<0x9A>",
            "<0xa0>",
            " ",
            "\r",
            "\n",
            "\r\n",
            "\u{10FF9A}",
            "\u{10FF00}",
            "\u{10FFFF}",
            "\u{10FEFF}",
        ])
    }

    fn text() -> impl Strategy<Value = String> {
        prop::collection::vec(piece(), 0..12).prop_map(|v| v.concat())
    }

    fn encoding() -> impl Strategy<Value = Option<Encoding>> {
        prop::sample::select(vec![None, Some(Encoding::ShiftJis), Some(Encoding::EucJp)])
    }

    /// 挿入先 (§8.3.1 の「文書の種類と保存の扱い」の表の 6 種): バイトの文書 (2 つの文字コード)・バイトでない文書・
    /// 状態付きの文書 (表記にする、`None`)・衝突の文書 (表記にしない、`None`)。
    fn target() -> impl Strategy<Value = EscapeContext> {
        prop::sample::select(vec![
            BYTES_SJIS,
            EscapeContext {
                notation: true,
                escape_encoding: Some(Encoding::EucJp),
            },
            PLAIN,
            COLLISION,
        ])
    }

    /// 届く文字列: 読めない・内部レジスタの表記そのもの・改行を変えたもの・空白を足したもの・別の文字列。
    fn arrived_for(reg_text: String) -> impl Strategy<Value = Option<String>> {
        let crlf = reg_text.replace("\r\n", "\n").replace('\n', "\r\n");
        let lf_only = lf(&reg_text);
        let spaced = format!("{reg_text} ");
        prop_oneof![
            Just(None),
            Just(Some(reg_text)),
            Just(Some(crlf)),
            Just(Some(lf_only)),
            Just(Some(spaced)),
            text().prop_map(Some),
        ]
    }

    /// 1 件の入力: (内部レジスタ (本文, `notation`, `escape_origin`), 届く文字列, 古い印, 挿入先, 改行, 正規化するか)。
    type Case = (
        Option<(String, bool, Option<Encoding>)>,
        Option<String>,
        bool,
        EscapeContext,
        LineEnding,
        bool,
    );

    fn case() -> impl Strategy<Value = Case> {
        let reg = prop::option::of((text(), any::<bool>(), encoding()).prop_map(
            |(t, notation, origin)| {
                // コピー元の不変条件: `escape_encoding` が `Some` の文書は表記にする (`Enabled`)
                (t, notation || origin.is_some(), origin)
            },
        ));
        reg.prop_flat_map(|reg| {
            let reg_text =
                reg.as_ref().map_or_else(
                    String::new,
                    |(t, n, _)| {
                        if *n { naive_bytes(t) } else { t.clone() }
                    },
                );
            (
                Just(reg),
                arrived_for(reg_text),
                any::<bool>(),
                target(),
                prop::sample::select(vec![LineEnding::Lf, LineEnding::Crlf, LineEnding::Cr]),
                any::<bool>(),
            )
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        #[test]
        fn 貼り付けの結果は規則の表の素朴な実装と一致する(
            (reg, arrived, stale, target, eol, normalize) in case()
        ) {
            let src = reg.as_ref().map(|(t, n, o)| src(t, *n, *o));
            let got = resolve_paste(arrived.as_deref(), src.as_ref(), stale, target, eol, normalize);
            let want = model(
                arrived.as_deref(),
                reg.as_ref().map(|(t, n, o)| (t.as_str(), *n, *o)),
                stale,
                target,
                eol,
                normalize,
            );
            prop_assert_eq!(
                got.as_ref().map(|p| (p.text.clone(), p.notices.clone())),
                want
            );
            // 本文に入る U+10FF00.. は、経路 (1) (内部レジスタの数と同じ) と衝突の文書への挿入だけ
            if let Some(p) = got {
                let n = escapes(&p.text);
                let as_bytes = p.notices.iter().find_map(|x| match x {
                    InsertNotice::AsBytes(k) => Some(*k),
                    _ => None,
                });
                if let Some(k) = as_bytes {
                    prop_assert_eq!(n, k);
                } else if target.notation {
                    prop_assert_eq!(n, 0, "{:?}", p.text);
                }
            }
        }
    }
}
