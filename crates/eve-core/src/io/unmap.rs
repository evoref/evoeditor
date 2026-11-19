//! 表現不能の文字の置換 (docs/03 §8.6「E3 の選択肢」の選択肢 2)。保存先の文字コードで表せない文字を、置換先の規則で
//! 置き換える `ChangeSet` を作る (`JobKind::ReplaceUnmappable`、bg ワーカー)。

use std::collections::HashMap;
use std::sync::Arc;

use eve_encoding::{EncodeReport, Encoder, Encoding, EscapePolicy};
use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, Snapshot, TextRead};

use crate::document::PreservedMap;
use crate::io::unmappable::is_escape_char;
use crate::jobs::cancel::CancelToken;

/// 一覧に載せる対象の上限 (03 §8.6 の「置換の一覧」)。
pub const MAX_TARGETS: usize = 100;

/// 置換の対象 1 件 (一覧の 1 行)。位置は `ChangeSet` を作った Snapshot の座標。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceTarget {
    /// 文字の位置 (バイト)。
    pub offset: usize,
    /// 表現できない文字。
    pub ch: char,
    /// 置き換える文字。
    pub replacement: char,
}

/// 置換先ごとの件数。総数は 3 つの合計。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplaceCounts {
    /// 半角カナ → 全角カナ (ISO-2022-JP)。
    pub kana: u64,
    /// 互換文字の表 (`Shift_JIS` / EUC-JP / ISO-2022-JP)。
    pub compat: u64,
    /// それ以外 → `?`。
    pub question: u64,
}

impl ReplaceCounts {
    /// 置換する文字の総数。
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
fn is_unmappable(enc: Encoding, escape: EscapePolicy, ch: char) -> bool {
    let mut e = Encoder::new(enc, false, escape);
    let mut report = EncodeReport::default();
    let mut sink = Vec::new();
    let mut buf = [0u8; 4];
    e.encode(ch.encode_utf8(&mut buf), &mut sink, &mut report);
    e.finish(&mut sink, &mut report);
    !report.is_empty()
}

/// 表現不能の文字をすべて置換する `ChangeSet` を作る。対象が無ければ `None`。取消しで `None` を返さず `Err(())`。
///
/// `preserved` は保持方式の記録の版 (保存と同じ条件で渡す。空なら記録を使わない)。保存と同じ計画 (`save_plan`) で
/// 元のバイト列を書く記録の範囲の文字は、置換の対象にしない (03 §8.6 E2。編集していない範囲の補助漢字・半角カナを
/// `?` や全角カナに置き換えない)。
pub fn build_replacement(
    snapshot: &Snapshot,
    enc: Encoding,
    escape: EscapePolicy,
    preserved: &PreservedMap,
    cancel: &CancelToken,
) -> Result<Option<(ChangeSet, Vec<ReplaceTarget>, ReplaceCounts)>, ()> {
    let raw = if preserved.is_empty() {
        Vec::new()
    } else {
        crate::io::save_plan::raw_ranges(snapshot, enc, preserved)
    };
    let mut raw = raw.into_iter().peekable();
    let body = snapshot.slice_to_string(ByteRange::all(snapshot.len_bytes()));
    let mut cache: HashMap<char, bool> = HashMap::new();
    let mut edits: Vec<Edit> = Vec::new();
    let mut targets: Vec<ReplaceTarget> = Vec::new();
    let mut counts = ReplaceCounts::default();
    for (i, (off, ch)) in body.char_indices().enumerate() {
        if i % 65_536 == 0 && cancel.is_cancelled() {
            return Err(());
        }
        // §8.3 のエスケープ文字 (不正バイト) は置換の対象にしない (03 §8.6 の置換先。`?` にすると元のバイトが黙って
        // 失われる)。エスケープ文字を含む文書には選択肢 2 自体を出さないので、ここは二重の防御
        if ch.is_ascii() || is_escape_char(ch) {
            continue;
        }
        // 元のバイト列を書く範囲の文字は対象にしない (範囲は昇順で重ならない)
        while raw.next_if(|(_, e)| e.0 <= off).is_some() {}
        if raw.peek().is_some_and(|(s, _)| s.0 <= off) {
            continue;
        }
        let bad = *cache
            .entry(ch)
            .or_insert_with(|| is_unmappable(enc, escape, ch));
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
        let escape = EscapePolicy::Reject;
        assert!(is_unmappable(Encoding::ShiftJis, escape, '😀'));
        assert!(!is_unmappable(Encoding::ShiftJis, escape, 'あ'));
        // エスケープ文字は別の文字コードへ変換するとき表現不能 (E4)
        assert!(is_unmappable(Encoding::ShiftJis, escape, '\u{10FF80}'));
    }

    #[test]
    fn エスケープ文字は置換の対象にしない() {
        // 先頭 100 件の内と外の両方にエスケープ文字を置く
        let body = format!(
            "\u{10FF9A}a{}\u{10FF80}😀\u{10FF0A}",
            "😀".repeat(MAX_TARGETS + 1)
        );
        let text = eve_text::Text::from(body.as_str());
        let snap = text.snapshot();
        let (cs, targets, counts) = build_replacement(
            &snap,
            Encoding::ShiftJis,
            EscapePolicy::Reject,
            &PreservedMap::new(),
            &CancelToken::new(),
        )
        .expect("取り消していない")
        .expect("絵文字は対象");
        // 対象は絵文字だけ (一覧にも件数にもエスケープ文字を入れない)
        assert_eq!(
            counts,
            ReplaceCounts {
                kana: 0,
                compat: 0,
                question: MAX_TARGETS as u64 + 2,
            }
        );
        assert_eq!(targets.len(), MAX_TARGETS);
        assert!(targets.iter().all(|t| t.ch == '😀' && t.replacement == '?'));
        // ChangeSet が書き換えるのは絵文字だけ (エスケープ文字の範囲に触れない)
        let changed: Vec<String> = cs
            .changed_ranges()
            .map(|(old, _)| body[old.start.0..old.end.0].to_owned())
            .collect();
        assert!(
            changed.iter().all(|s| s.chars().all(|c| c == '😀')),
            "{changed:?}"
        );
        assert_eq!(
            cs.len_after(),
            body.len() - (MAX_TARGETS + 2) * ('😀'.len_utf8() - 1)
        );
    }

    #[test]
    fn エスケープ文字だけなら対象は無い() {
        let text = eve_text::Text::from("a\u{10FF80}b");
        let r = build_replacement(
            &text.snapshot(),
            Encoding::EucJp,
            EscapePolicy::Reject,
            &PreservedMap::new(),
            &CancelToken::new(),
        )
        .expect("取り消していない");
        assert!(r.is_none());
    }
}
