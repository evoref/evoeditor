//! 改行コードの変換の ChangeSet を作る (docs/03 §8.4、01 FR-113)。
//!
//! 文書の全区切りのうち `eol` と異なるものを `eol` に置き換える。チャンクをまたぐ CRLF を 1 つの区切りとして扱う。
//! すべての区切りを同じ `eol` にするので、置き換えで隣り合った CR と LF が新しい CRLF に結合することはない。

use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, LineEnding, TextRead};

use crate::cancel::CancelToken;

/// 変換の ChangeSet。取り消されたら `None`。
pub fn build_eol_conversion(
    text: &dyn TextRead,
    eol: LineEnding,
    cancel: &CancelToken,
) -> Option<ChangeSet> {
    let mut edits: Vec<Edit> = Vec::new();
    let insert: std::sync::Arc<str> = std::sync::Arc::from(eol.as_str());
    let mut push = |start: usize, len: usize, kind: LineEnding| {
        if kind != eol {
            edits.push(Edit {
                range: ByteRange::new(ByteOff(start), ByteOff(start + len)),
                insert: insert.clone(),
            });
        }
    };
    let mut pos = 0usize;
    // 直前のチャンクの末尾にあった CR の位置 (次のチャンクの先頭が LF なら CRLF)
    let mut pending_cr: Option<usize> = None;
    let full = ByteRange::new(ByteOff(0), ByteOff(text.len_bytes()));
    for chunk in text.chunks(full) {
        if cancel.is_cancelled() {
            return None;
        }
        let b = chunk.as_bytes();
        let mut i = 0;
        if let Some(cr) = pending_cr.take() {
            if b.first() == Some(&b'\n') {
                push(cr, 2, LineEnding::Crlf);
                i = 1;
            } else {
                push(cr, 1, LineEnding::Cr);
            }
        }
        while i < b.len() {
            match b[i] {
                b'\r' if i + 1 == b.len() => {
                    pending_cr = Some(pos + i);
                    i += 1;
                }
                b'\r' if b[i + 1] == b'\n' => {
                    push(pos + i, 2, LineEnding::Crlf);
                    i += 2;
                }
                b'\r' => {
                    push(pos + i, 1, LineEnding::Cr);
                    i += 1;
                }
                b'\n' => {
                    push(pos + i, 1, LineEnding::Lf);
                    i += 1;
                }
                _ => i += 1,
            }
        }
        pos += b.len();
    }
    if let Some(cr) = pending_cr {
        push(cr, 1, LineEnding::Cr);
    }
    ChangeSet::from_sorted_edits(text, edits).ok()
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // ChangeSet の結果を確かめるために Text へ直接適用する
mod tests {
    use super::*;
    use eve_text::Text;

    fn convert(src: &str, eol: LineEnding) -> String {
        let t = Text::from(src);
        let cs = build_eol_conversion(&t, eol, &CancelToken::new()).unwrap();
        let mut out = Text::from(src);
        out.apply(&cs).unwrap();
        out.slice_to_string(ByteRange::all(out.len_bytes()))
    }

    #[test]
    fn 混在を揃える() {
        let src = "a\nb\r\nc\rd";
        assert_eq!(convert(src, LineEnding::Lf), "a\nb\nc\nd");
        assert_eq!(convert(src, LineEnding::Crlf), "a\r\nb\r\nc\r\nd");
        assert_eq!(convert(src, LineEnding::Cr), "a\rb\rc\rd");
    }

    #[test]
    fn crcr_lf_は_cr_と_crlf() {
        assert_eq!(convert("a\r\r\nb", LineEnding::Lf), "a\n\nb");
    }

    #[test]
    fn 変えるものが無ければ空の_changeset() {
        let t = Text::from("a\nb\n");
        let cs = build_eol_conversion(&t, LineEnding::Lf, &CancelToken::new()).unwrap();
        assert!(cs.is_identity());
    }

    /// チャンクの境界をまたぐ CRLF も 1 つの区切りとして扱う (どの位置で分かれても同じ結果)。
    #[test]
    fn 長い文書でも行ごとの変換と一致する() {
        let src: String = (0..20_000)
            .map(|i| match i % 4 {
                0 => "line\r\n",
                1 => "x\n",
                2 => "y\r",
                _ => "z\r\n",
            })
            .collect();
        let want = src.replace("\r\n", "\n").replace('\r', "\n");
        assert_eq!(convert(&src, LineEnding::Lf), want);
        let crlf = want.replace('\n', "\r\n");
        assert_eq!(convert(&src, LineEnding::Crlf), crlf);
    }
}
