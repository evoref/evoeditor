//! 行の取り出し (検索・置換の行ごとの経路が使う、docs/03 §12.3)。
//!
//! 行ごとに `line_to_byte`・`line_end_byte`・`slice_to_string` を呼ぶと、1 行につき木の走査が数回と確保が 1 回かかる
//! (docs/review/v0.3-findings.md の PRF-05 / PRF-12)。ここでは範囲のチャンクを 1 回だけ順に読み、行の終端で区切る。
//! 1 つのチャンクに収まる行は、複写せずにそのチャンクから借りて渡す。
//!
//! 行の区切りは `TextRead` の行と同じで、LF・CR・CRLF の 3 つ (03 §2.4。CRLF はチャンクの境目で分割されないが、
//! 分割されていても 1 つの区切りとして読む)。

use std::ops::Range;

use eve_text::{ByteOff, ByteRange, LineIdx, TextRead};
use memchr::memchr2;

/// 行 `lines` (半開区間) を前から順に取り出し、行頭の文書内バイト位置と、行の内容 (終端を除く) を `f` へ渡す。
/// `f` が `false` を返したら止めて `false`。最終行 (終端が無い行。末尾が改行なら空の行) も、範囲に入っていれば渡す。
pub(super) fn for_each_line(
    text: &dyn TextRead,
    lines: Range<usize>,
    f: &mut dyn FnMut(usize, &str) -> bool,
) -> bool {
    let n = text.len_lines();
    let last = lines.end.min(n);
    if lines.start >= last {
        return true;
    }
    let start = text.line_to_byte(LineIdx(lines.start)).0;
    // 範囲が最終行を含むか。含まなければ、範囲の終わりは次の行の行頭 (その行は渡さない)
    let to_end = last >= n;
    let end = if to_end {
        text.len_bytes()
    } else {
        text.line_to_byte(LineIdx(last)).0
    };
    // 今の行の行頭と、その行のうち前のチャンクまでにあった分
    let mut line_start = start;
    let mut pending = String::new();
    // 今のチャンクの文書内の開始位置
    let mut pos = start;
    // 前のチャンクが CR で終わった (次のチャンクの先頭が LF なら、その CRLF の残り)
    let mut skip_lf = false;
    for chunk in text.chunks(ByteRange::new(ByteOff(start), ByteOff(end))) {
        let bytes = chunk.as_bytes();
        let mut at = 0;
        if skip_lf {
            skip_lf = false;
            if bytes.first() == Some(&b'\n') {
                at = 1;
                line_start = pos + 1;
            }
        }
        while let Some(i) = memchr2(b'\n', b'\r', &bytes[at..]) {
            let t = at + i;
            let ok = if pending.is_empty() {
                f(line_start, &chunk[at..t])
            } else {
                pending.push_str(&chunk[at..t]);
                let ok = f(line_start, &pending);
                pending.clear();
                ok
            };
            if !ok {
                return false;
            }
            at = t + 1;
            if bytes[t] == b'\r' {
                match bytes.get(at) {
                    Some(&b'\n') => at += 1,
                    Some(_) => {}
                    None => skip_lf = true,
                }
            }
            line_start = pos + at;
        }
        pending.push_str(&chunk[at..]);
        pos += bytes.len();
    }
    if to_end {
        return f(line_start, &pending);
    }
    true
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // 編集した木の形を作るために Text へ直接適用する
mod tests {
    use eve_text::Text;
    use proptest::prelude::*;

    use super::*;

    /// 行ごとに `TextRead` の行の API で取り出した結果 (基準)。
    fn reference(t: &Text, lines: Range<usize>) -> Vec<(usize, String)> {
        (lines.start..lines.end.min(t.len_lines()))
            .map(|l| {
                let (s, e) = (t.line_to_byte(LineIdx(l)), t.line_end_byte(LineIdx(l)));
                (s.0, t.slice_to_string(ByteRange::new(s, e)))
            })
            .collect()
    }

    fn collect(t: &Text, lines: Range<usize>) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        assert!(for_each_line(t, lines, &mut |s, line| {
            out.push((s, line.to_owned()));
            true
        }));
        out
    }

    #[test]
    fn 行頭と内容を順に渡す() {
        let t = Text::from("ab\r\nc\rd\n\nえ");
        assert_eq!(
            collect(&t, 0..t.len_lines()),
            vec![
                (0, "ab".to_owned()),
                (4, "c".to_owned()),
                (6, "d".to_owned()),
                (8, String::new()),
                (9, "え".to_owned()),
            ]
        );
    }

    #[test]
    fn 末尾が改行なら空の最終行も渡す() {
        let t = Text::from("a\n");
        assert_eq!(
            collect(&t, 0..2),
            vec![(0, "a".to_owned()), (2, String::new())]
        );
        // 最終行を含まない範囲では渡さない
        assert_eq!(collect(&t, 0..1), vec![(0, "a".to_owned())]);
        // 空の文書は空の 1 行
        let t = Text::from("");
        assert_eq!(collect(&t, 0..1), vec![(0, String::new())]);
        assert_eq!(collect(&t, 0..9), vec![(0, String::new())]);
        assert_eq!(collect(&t, 1..1), Vec::new());
    }

    #[test]
    fn 偽を返したら止める() {
        let t = Text::from("a\nb\nc");
        let mut seen = Vec::new();
        assert!(!for_each_line(&t, 0..3, &mut |s, _| {
            seen.push(s);
            s < 2
        }));
        assert_eq!(seen, vec![0, 2]);
    }

    #[test]
    fn チャンクをまたぐ長い行と多くの行() {
        // ropey のチャンク (約 1 KiB) より長い行と短い行を混ぜる
        let mut s = String::new();
        for i in 0..400 {
            s.push_str(&"あa".repeat(i % 7 * 150));
            s.push_str(["\n", "\r\n", "\r"][i % 3]);
        }
        s.push_str(&"終".repeat(3000));
        let t = Text::from(s.as_str());
        let n = t.len_lines();
        assert_eq!(collect(&t, 0..n), reference(&t, 0..n));
        assert_eq!(collect(&t, 17..233), reference(&t, 17..233));
    }

    proptest! {
        // どの文書・どの行の範囲でも、行の API で 1 行ずつ取り出した結果と同じ
        #[test]
        fn 行の_api_で取り出した結果と同じ(
            parts in prop::collection::vec(
                prop_oneof![
                    Just("\n".to_owned()),
                    Just("\r\n".to_owned()),
                    Just("\r".to_owned()),
                    "[a-zあ😀]{0,6}",
                    // チャンクの境目をまたがせる
                    (300usize..1200).prop_map(|n| "x".repeat(n)),
                ],
                0..40,
            ),
            a in 0usize..50,
            b in 0usize..50,
        ) {
            let t = Text::from(parts.concat().as_str());
            let n = t.len_lines();
            prop_assert_eq!(collect(&t, 0..n), reference(&t, 0..n));
            let (a, b) = (a.min(b), a.max(b));
            prop_assert_eq!(collect(&t, a..b), reference(&t, a..b));
        }

        // 編集で作った木 (チャンクの形が読込とは違う) でも同じ
        #[test]
        fn 編集した文書でも同じ(
            base in "[a\r\n]{0,200}",
            edits in prop::collection::vec((0usize..400, "[b\r\n]{0,5}"), 0..30),
        ) {
            let mut t = Text::from(base.as_str());
            for (at, ins) in edits {
                let at = at.min(t.len_bytes());
                // CRLF の間には挿入しない位置へ寄せる必要は無い (分かれた CR と LF は別の区切りになるだけ)
                let edit = eve_text::Edit {
                    range: ByteRange::new(ByteOff(at), ByteOff(at)),
                    insert: ins.as_str().into(),
                };
                let cs = eve_text::ChangeSet::from_sorted_edits(&t, [edit]).unwrap();
                t.apply(&cs).unwrap();
            }
            let n = t.len_lines();
            prop_assert_eq!(collect(&t, 0..n), reference(&t, 0..n));
        }
    }
}
