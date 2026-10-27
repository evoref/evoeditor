//! 表現不能の文字の置換 (docs/03 §8.6「E3 の選択肢」の選択肢 2)。保存先の文字コードで表せない文字を、置換先の規則で
//! 置き換える ChangeSet を作る (`JobKind::ReplaceUnmappable`、bg ワーカー)。

use std::collections::HashMap;
use std::sync::Arc;

use eve_encoding::{EncodeReport, Encoder, Encoding, EscapePolicy, PuaEscapeMode};
use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, Snapshot, TextRead};

use crate::cancel::CancelToken;

/// 一覧に載せる対象の上限 (03 §8.6 の「置換の一覧」)。
pub const MAX_TARGETS: usize = 100;

/// 置換の対象 1 件 (一覧の 1 行)。位置は ChangeSet を作った Snapshot の座標。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceTarget {
    pub offset: usize,
    pub ch: char,
    pub replacement: char,
}

/// 置換先ごとの件数。総数は 3 つの合計。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplaceCounts {
    /// 半角カナ → 全角カナ (ISO-2022-JP)。
    pub kana: u64,
    /// 互換文字の表 (Shift_JIS / EUC-JP / ISO-2022-JP)。
    pub compat: u64,
    /// それ以外 → `?`。
    pub question: u64,
}

impl ReplaceCounts {
    pub fn total(&self) -> u64 {
        self.kana + self.compat + self.question
    }
}

/// 半角カナ (U+FF61〜FF9F) の全角への対応 (WHATWG の index ISO-2022-JP katakana)。濁点・半濁点は合成せず `゛` / `゜`。
const KANA: &str = "。「」、・ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン゛゜";

/// 互換文字の表 (03 §8.6 の (a'))。`(対象, 置換後, Shift_JIS / EUC-JP のときだけか)`。
const COMPAT: [(char, char, bool); 10] = [
    ('\u{301C}', '\u{FF5E}', false),
    ('\u{2016}', '\u{2225}', false),
    ('\u{2212}', '\u{FF0D}', false),
    ('\u{00A2}', '\u{FFE0}', false),
    ('\u{00A3}', '\u{FFE1}', false),
    ('\u{00AC}', '\u{FFE2}', false),
    ('\u{00A6}', '\u{FFE4}', false),
    ('\u{2014}', '\u{2015}', false),
    ('\u{00A5}', '\u{FFE5}', true),
    ('\u{203E}', '\u{FFE3}', true),
];

/// 置換先の分類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Kana,
    Compat,
    Question,
}

/// `ch` の置換先 (保存先の文字コード別)。
fn replacement(enc: Encoding, ch: char) -> (char, Kind) {
    let c = ch as u32;
    if enc == Encoding::Iso2022Jp
        && (0xFF61..=0xFF9F).contains(&c)
        && let Some(k) = KANA.chars().nth((c - 0xFF61) as usize)
    {
        return (k, Kind::Kana);
    }
    if matches!(
        enc,
        Encoding::ShiftJis | Encoding::EucJp | Encoding::Iso2022Jp
    ) {
        let sjis_euc = enc != Encoding::Iso2022Jp;
        for (from, to, only_sjis_euc) in COMPAT {
            if from == ch && (!only_sjis_euc || sjis_euc) {
                return (to, Kind::Compat);
            }
        }
    }
    ('?', Kind::Question)
}

/// `ch` 1 文字が保存先の文字コードで表現不能か (黙って別の文字に写される場合を含む、E1)。
fn is_unmappable(enc: Encoding, pua: PuaEscapeMode, ch: char) -> bool {
    let mut e = Encoder::new(enc, false, EscapePolicy::from_mode(pua));
    let mut report = EncodeReport::default();
    let mut sink = Vec::new();
    let mut buf = [0u8; 4];
    e.encode(ch.encode_utf8(&mut buf), &mut sink, &mut report);
    e.finish(&mut sink, &mut report);
    !report.is_empty()
}

/// 表現不能の文字をすべて置換する ChangeSet を作る。対象が無ければ `None`。取消しで `None` を返さず `Err(())`。
pub fn build_replacement(
    snapshot: &Snapshot,
    enc: Encoding,
    pua: PuaEscapeMode,
    cancel: &CancelToken,
) -> Result<Option<(ChangeSet, Vec<ReplaceTarget>, ReplaceCounts)>, ()> {
    let body = snapshot.slice_to_string(ByteRange::all(snapshot.len_bytes()));
    let mut cache: HashMap<char, bool> = HashMap::new();
    let mut edits: Vec<Edit> = Vec::new();
    let mut targets: Vec<ReplaceTarget> = Vec::new();
    let mut counts = ReplaceCounts::default();
    for (i, (off, ch)) in body.char_indices().enumerate() {
        if i % 65_536 == 0 && cancel.is_cancelled() {
            return Err(());
        }
        if ch.is_ascii() {
            continue;
        }
        let bad = *cache
            .entry(ch)
            .or_insert_with(|| is_unmappable(enc, pua, ch));
        if !bad {
            continue;
        }
        let (to, kind) = replacement(enc, ch);
        match kind {
            Kind::Kana => counts.kana += 1,
            Kind::Compat => counts.compat += 1,
            Kind::Question => counts.question += 1,
        }
        if targets.len() < MAX_TARGETS {
            targets.push(ReplaceTarget {
                offset: off,
                ch,
                replacement: to,
            });
        }
        let mut s = String::new();
        s.push(to);
        edits.push(Edit {
            range: ByteRange::new(ByteOff(off), ByteOff(off + ch.len_utf8())),
            insert: Arc::from(s.as_str()),
        });
    }
    if edits.is_empty() {
        return Ok(None);
    }
    let cs = ChangeSet::from_sorted_edits(snapshot, edits).map_err(|_| ())?;
    Ok(Some((cs, targets, counts)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 半角カナの表は_63_文字() {
        assert_eq!(KANA.chars().count(), 63);
        assert_eq!(replacement(Encoding::Iso2022Jp, 'ｱ'), ('ア', Kind::Kana));
        assert_eq!(
            replacement(Encoding::Iso2022Jp, '\u{FF9E}'),
            ('\u{309B}', Kind::Kana)
        );
        assert_eq!(
            replacement(Encoding::Iso2022Jp, '\u{FF9F}'),
            ('\u{309C}', Kind::Kana)
        );
        assert_eq!(replacement(Encoding::Iso2022Jp, '｡'), ('。', Kind::Kana));
    }

    #[test]
    fn 互換文字の置換先() {
        assert_eq!(
            replacement(Encoding::ShiftJis, '\u{301C}'),
            ('\u{FF5E}', Kind::Compat)
        );
        // ¥ は Shift_JIS / EUC-JP だけ対象
        assert_eq!(
            replacement(Encoding::ShiftJis, '\u{00A5}'),
            ('\u{FFE5}', Kind::Compat)
        );
        assert_eq!(
            replacement(Encoding::Iso2022Jp, '\u{00A5}'),
            ('?', Kind::Question)
        );
        assert_eq!(replacement(Encoding::ShiftJis, '😀'), ('?', Kind::Question));
        assert_eq!(replacement(Encoding::EucJp, 'ｱ'), ('?', Kind::Question));
    }

    #[test]
    fn 表現不能の判定() {
        let pua = PuaEscapeMode::Enabled;
        assert!(is_unmappable(Encoding::ShiftJis, pua, '😀'));
        assert!(!is_unmappable(Encoding::ShiftJis, pua, 'あ'));
    }
}
