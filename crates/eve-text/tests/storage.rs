//! テキストストレージのプロパティテスト (docs/03-core-design.md §2.5 の不変条件 INV-T1〜T5、§2.4 の行の定義)。
//!
//! 素朴なモデル (`&str` を直接走査する実装) と、`Text` / `Snapshot` の結果を突き合わせる。
//! 文字は CR / LF を多めに、1 バイト・2 バイト・3 バイト・4 バイトの文字を混ぜて作り、
//! ropey のチャンク (1 KiB 前後) をまたぐ長さのテキストも生成する。

use eve_text::{
    ByteOff, ByteRange, LineEnding, LineIdx, Snapshot, Text, TextBuilder, TextRead, Utf16Off,
};
use proptest::prelude::*;

// ---------- 素朴なモデル ----------

/// 行区切り (開始, 終了, 種類) を先頭から順に集める。CRLF は 1 つの区切り。
fn model_breaks(s: &str) -> Vec<(usize, usize, LineEnding)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => {
                out.push((i, i + 2, LineEnding::Crlf));
                i += 2;
            }
            b'\r' => {
                out.push((i, i + 1, LineEnding::Cr));
                i += 1;
            }
            b'\n' => {
                out.push((i, i + 1, LineEnding::Lf));
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// 行頭のバイト位置の一覧 (先頭は 0、区切りの終わりごとに 1 つ)。
fn model_line_starts(s: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(model_breaks(s).into_iter().map(|(_, e, _)| e))
        .collect()
}

// ---------- 生成 ----------

fn any_char() -> impl Strategy<Value = char> {
    prop_oneof![
        3 => Just('\r'),
        3 => Just('\n'),
        6 => proptest::char::range('a', 'z'),
        1 => Just('\u{3042}'),       // あ (3 バイト)
        1 => Just('é'),              // 2 バイト
        1 => Just('\u{1F600}'),      // 😀 (4 バイト、UTF-16 ではサロゲートペア)
        1 => Just('\u{0301}'),       // 結合文字
        1 => Just('\u{2028}'),       // LS (行区切りとして扱わない)
        1 => Just('\u{0085}'),       // NEL (行区切りとして扱わない)
        1 => Just('\u{000B}'),       // VT (行区切りとして扱わない)
        1 => Just('\u{2029}'),       // PS (行区切りとして扱わない)
        1 => Just('\u{000C}'),       // FF (行区切りとして扱わない)
    ]
}

/// 短いテキストから、複数のチャンクにまたがる長さのテキストまで。
fn any_text() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => proptest::collection::vec(any_char(), 0..200),
        2 => proptest::collection::vec(any_char(), 1500..6000),
    ]
    .prop_map(|v| v.into_iter().collect())
}

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 128,
        ..ProptestConfig::default()
    }
}

// ---------- 性質 ----------

proptest! {
    #![proptest_config(config())]

    // spec: 03 §2.4 / §2.5 (INV-T2)
    #[test]
    fn 行数は区切りの数プラス_1(s in any_text()) {
        let t = Text::from(s.as_str());
        prop_assert_eq!(t.len_lines(), model_breaks(&s).len() + 1);
        prop_assert_eq!(t.len_bytes(), s.len());
        prop_assert_eq!(t.len_chars(), s.chars().count());
    }

    // spec: 03 §2.4 / §2.5 (INV-T5)
    #[test]
    fn 行頭と行末と改行の種類はモデルと一致する(s in any_text()) {
        let t = Text::from(s.as_str());
        let starts = model_line_starts(&s);
        let breaks = model_breaks(&s);
        for (l, start) in starts.iter().enumerate() {
            prop_assert_eq!(t.line_to_byte(LineIdx(l)), ByteOff(*start));
            let (end, term) = match breaks.get(l) {
                Some(&(bs, _, kind)) => (bs, Some(kind)),
                None => (s.len(), None),
            };
            prop_assert_eq!(t.line_end_byte(LineIdx(l)), ByteOff(end), "行 {} の末尾", l);
            prop_assert_eq!(t.line_terminator(LineIdx(l)), term, "行 {} の改行", l);
            // INV-T5: line_to_byte(l) <= line_end_byte(l) <= line_to_byte(l+1)
            prop_assert!(t.line_to_byte(LineIdx(l)) <= t.line_end_byte(LineIdx(l)));
            prop_assert!(t.line_end_byte(LineIdx(l)) <= t.line_to_byte(LineIdx(l + 1)));
        }
    }

    // spec: 03 §3.2 (byte_to_line)
    #[test]
    fn バイト位置から行を引ける(s in any_text()) {
        let t = Text::from(s.as_str());
        let starts = model_line_starts(&s);
        // 区切りの途中 (CRLF の \n 側) を含む、すべての char 境界で確かめる
        for (b, _) in s.char_indices().chain(std::iter::once((s.len(), ' '))) {
            let expected = starts.partition_point(|&st| st <= b) - 1;
            prop_assert_eq!(t.byte_to_line(ByteOff(b)), LineIdx(expected), "位置 {}", b);
        }
    }

    // spec: 03 §2.5 (INV-T4)
    #[test]
    fn バイトと_char_の変換は往復する(s in any_text()) {
        let t = Text::from(s.as_str());
        let mut char_idx = 0;
        for (b, _) in s.char_indices() {
            prop_assert_eq!(t.byte_to_char(ByteOff(b)).0, char_idx);
            prop_assert_eq!(t.char_to_byte(eve_text::CharIdx(char_idx)), ByteOff(b));
            char_idx += 1;
        }
        prop_assert_eq!(t.byte_to_char(ByteOff(s.len())).0, char_idx);
        prop_assert_eq!(t.char_to_byte(eve_text::CharIdx(char_idx)), ByteOff(s.len()));
    }

    // spec: 03 §3.2 (UTF-16)
    #[test]
    fn utf16_の変換はモデルと一致する(s in any_text()) {
        let t = Text::from(s.as_str());
        let mut u = 0;
        for (b, c) in s.char_indices() {
            prop_assert_eq!(t.byte_to_utf16(ByteOff(b)), Utf16Off(u));
            prop_assert_eq!(t.utf16_to_byte(Utf16Off(u)), ByteOff(b));
            if c.len_utf16() == 2 {
                // サロゲートペアの間は、その文字の先頭へ丸める
                prop_assert_eq!(t.utf16_to_byte(Utf16Off(u + 1)), ByteOff(b));
            }
            u += c.len_utf16();
        }
        prop_assert_eq!(t.byte_to_utf16(ByteOff(s.len())), Utf16Off(s.encode_utf16().count()));
    }

    // spec: 03 §2.2 (T6: チャンク走査)
    #[test]
    fn チャンクを連結すると元のテキストになる(s in any_text(), a in 0usize..8000, b in 0usize..8000) {
        let t = Text::from(s.as_str());
        let all: String = t.chunks(ByteRange::all(t.len_bytes())).collect();
        prop_assert_eq!(&all, &s);
        // 部分範囲: char 境界へ丸めた範囲と一致する
        let (lo, hi) = (a.min(b), a.max(b));
        let floor = (0..=lo.min(s.len())).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
        let ceil = (hi.min(s.len())..=s.len()).find(|&i| s.is_char_boundary(i)).unwrap_or(s.len());
        let part: String = t.chunks(ByteRange::from(lo..hi)).collect();
        prop_assert_eq!(&part, &s[floor..ceil.max(floor)]);
        prop_assert_eq!(t.slice_to_string(ByteRange::from(lo..hi)), part);
    }

    // spec: 03 §2.3 (CRLF がチャンクの境目で分割されない保証。S-02 で確かめる項目)
    #[test]
    fn crlf_はチャンクの境目で分割されない(s in any_text()) {
        let t = Text::from(s.as_str());
        let chunks: Vec<&str> = t.chunks(ByteRange::all(t.len_bytes())).collect();
        for pair in chunks.windows(2) {
            prop_assert!(
                !(pair[0].ends_with('\r') && pair[1].starts_with('\n')),
                "CRLF がチャンクの境目で分割されている"
            );
        }
    }

    // spec: 03 §2.2 (TextBuilder)。チャンクの境目は任意の位置でよい。CRLF の間で切っても、行の構造は内容だけで決まる
    #[test]
    fn ビルダーはチャンクの切り方によらず同じ内容と行構造になる(s in any_text(), cuts in proptest::collection::vec(0usize..8000, 0..8)) {
        // 切れ目は char 境界まで進める
        let snap = |mut c: usize| {
            c = c.min(s.len());
            while !s.is_char_boundary(c) {
                c += 1;
            }
            c
        };
        let mut points: Vec<usize> = cuts.into_iter().map(snap).collect();
        points.extend([0, s.len()]);
        points.sort_unstable();
        points.dedup();
        let mut builder = TextBuilder::new();
        for w in points.windows(2) {
            builder.append(&s[w[0]..w[1]]);
        }
        let t = builder.finish();
        let whole = Text::from(s.as_str());
        prop_assert_eq!(t.slice_to_string(ByteRange::all(t.len_bytes())), s.clone());
        prop_assert_eq!(t.len_lines(), whole.len_lines());
        for l in 0..whole.len_lines() {
            prop_assert_eq!(t.line_terminator(LineIdx(l)), whole.line_terminator(LineIdx(l)));
            prop_assert_eq!(t.line_to_byte(LineIdx(l)), whole.line_to_byte(LineIdx(l)));
        }
    }

    // spec: 03 §2.5 (INV-T3 の前半)。スナップショットは元と同じ内容を持ち、Clone しても同じ
    #[test]
    fn スナップショットは元と同じ内容を持つ(s in any_text()) {
        let t = Text::from(s.as_str());
        let snap: Snapshot = t.snapshot();
        let copy = snap.clone();
        prop_assert_eq!(snap.slice_to_string(ByteRange::all(snap.len_bytes())), s.clone());
        prop_assert_eq!(copy.len_lines(), t.len_lines());
        prop_assert_eq!(copy.len_bytes(), t.len_bytes());
    }

    // spec: 03 §3.2 の C2 (範囲外の入力でパニックせず末尾に丸める。NFR-305)
    #[test]
    fn 範囲外の入力は末尾に丸められる(s in any_text(), x in huge()) {
        let t = Text::from(s.as_str());
        let end = t.len_bytes();
        if x > end {
            prop_assert_eq!(t.byte_to_char(ByteOff(x)).0, t.len_chars());
            prop_assert_eq!(t.byte_to_line(ByteOff(x)), LineIdx(t.len_lines() - 1));
            prop_assert_eq!(t.byte_to_utf16(ByteOff(x)), Utf16Off(s.encode_utf16().count()));
            prop_assert!(!t.is_char_boundary(ByteOff(x)));
            // 範囲外の空の範囲は、末尾の空の範囲になる
            prop_assert_eq!(t.slice_to_string(ByteRange::new(ByteOff(x), ByteOff(x))), "");
            prop_assert_eq!(t.chunks(ByteRange::new(ByteOff(x), ByteOff(x))).count(), 0);
        }
        if x > t.len_chars() {
            prop_assert_eq!(t.char_to_byte(eve_text::CharIdx(x)), ByteOff(end));
        }
        if x >= t.len_lines() {
            prop_assert_eq!(t.line_to_byte(LineIdx(x)), ByteOff(end));
            prop_assert_eq!(t.line_end_byte(LineIdx(x)), ByteOff(end));
            prop_assert_eq!(t.line_terminator(LineIdx(x)), None);
        }
        if x > s.encode_utf16().count() {
            prop_assert_eq!(t.utf16_to_byte(Utf16Off(x)), ByteOff(end));
        }
        // chunk_at は常に文書の範囲に収まるチャンクを返す
        let (chunk, start) = t.chunk_at(ByteOff(x));
        prop_assert!(start.0 + chunk.len() <= end);
    }

    // spec: 03 §3.2 (is_char_boundary はモデルと一致する)
    #[test]
    fn char_境界の判定はモデルと一致する(s in any_text()) {
        let t = Text::from(s.as_str());
        for b in 0..=s.len() + 3 {
            prop_assert_eq!(t.is_char_boundary(ByteOff(b)), b <= s.len() && s.is_char_boundary(b), "位置 {}", b);
        }
    }

    // spec: 03 §2.2 (chunk_at: 位置を含むチャンクと、その先頭のバイト位置)
    #[test]
    fn chunk_at_は位置を含むチャンクを返す(s in any_text(), x in 0usize..8000) {
        let t = Text::from(s.as_str());
        let b = x.min(s.len());
        let (chunk, start) = t.chunk_at(ByteOff(b));
        prop_assert_eq!(&s[start.0..start.0 + chunk.len()], chunk);
        if !s.is_empty() {
            prop_assert!(
                start.0 <= b && (b < start.0 + chunk.len() || b == s.len()),
                "{} が [{}, {}) に含まれない", b, start.0, start.0 + chunk.len()
            );
        }
    }

    // spec: 03 §3.2 (char 境界でない位置は、その char の先頭に丸める)
    #[test]
    fn 文字の途中の位置はその文字の先頭に丸められる(s in any_text()) {
        let t = Text::from(s.as_str());
        for (i, (b, c)) in s.char_indices().enumerate() {
            for inner in 1..c.len_utf8() {
                let mid = ByteOff(b + inner);
                prop_assert!(!t.is_char_boundary(mid));
                prop_assert_eq!(t.byte_to_char(mid).0, i);
                prop_assert_eq!(t.byte_to_utf16(mid), t.byte_to_utf16(ByteOff(b)));
                prop_assert_eq!(t.byte_to_line(mid), t.byte_to_line(ByteOff(b)));
            }
        }
    }

    // spec: 03 §3.2 の C2。逆順の範囲でもパニックせず、空になる
    #[test]
    fn 逆順の範囲は空になる(s in any_text(), a in 0usize..8000, b in 0usize..8000) {
        let t = Text::from(s.as_str());
        let (lo, hi) = (a.min(b), a.max(b));
        // フィールドが公開なので、構造体リテラルで逆順の範囲を作れる
        let rev = ByteRange { start: ByteOff(hi), end: ByteOff(lo) };
        prop_assert_eq!(rev.len(), 0);
        prop_assert_eq!(t.slice_to_string(rev).len(), 0);
        prop_assert_eq!(t.chunks(rev).count(), 0);
    }
}

/// 範囲外の入力の生成: 小さい値から `usize::MAX` の近くまで。
fn huge() -> impl Strategy<Value = usize> {
    prop_oneof![
        3 => 0usize..10_000,
        1 => Just(usize::MAX),
        1 => (usize::MAX - 100)..=usize::MAX,
    ]
}

// ---------- 個別の例 ----------

#[test]
fn 行の定義の例() {
    // 03 §2.4: 末尾が区切りなら、その後ろに空の最終行がある
    let t = Text::from("a\nb\r\nc\r");
    assert_eq!(t.len_lines(), 4);
    assert_eq!(t.line_terminator(LineIdx(0)), Some(LineEnding::Lf));
    assert_eq!(t.line_terminator(LineIdx(1)), Some(LineEnding::Crlf));
    assert_eq!(t.line_terminator(LineIdx(2)), Some(LineEnding::Cr));
    assert_eq!(t.line_terminator(LineIdx(3)), None);
    assert_eq!(t.line_end_byte(LineIdx(1)), ByteOff(3)); // "b" の直後、\r\n の前
    assert_eq!(t.line_to_byte(LineIdx(3)), ByteOff(7));
    assert!(Text::from("").len_lines() == 1, "空の文書は 1 行");
}

#[test]
fn 行区切りは_lf_crlf_cr_だけ() {
    // U+0085 / U+2028 / U+2029 / VT / FF は行区切りにしない (03 §2.3)
    let t = Text::from("a\u{85}b\u{2028}c\u{2029}d\u{b}e\u{c}f");
    assert_eq!(t.len_lines(), 1);
}

#[test]
fn crlf_の間は_char_境界だが行は_crlf_で終わる() {
    let t = Text::from("x\r\ny");
    assert!(t.is_char_boundary(ByteOff(2))); // \r と \n の間も char 境界 (カーソルは置けない: 03 §4.1)
    assert_eq!(
        t.byte_to_line(ByteOff(2)),
        LineIdx(0),
        "\\n は CRLF で終わる行に属する"
    );
    assert_eq!(t.byte_to_line(ByteOff(3)), LineIdx(1));
}

#[test]
fn ビルダーが_crlf_の間で切れても_1_つの区切りになる() {
    let mut b = TextBuilder::new();
    b.append("a\r");
    b.append("\nb");
    let t = b.finish();
    assert_eq!(t.len_lines(), 2);
    assert_eq!(t.line_terminator(LineIdx(0)), Some(LineEnding::Crlf));
}

#[test]
fn デバッグ出力に本文を含めない() {
    let t = Text::from("秘密の本文");
    let s = format!("{t:?}");
    assert!(!s.contains("秘密"), "{s}");
}

#[test]
fn 型の性質はコンパイル時に確かめる() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Snapshot>();
    // TextRead はオブジェクト安全 (&dyn TextRead で Text と Snapshot を同じに扱える。03 §2.2)
    let t = Text::from("a\nb");
    let snap = t.snapshot();
    let readers: [&dyn TextRead; 2] = [&t, &snap];
    assert!(readers.iter().all(|r| r.len_lines() == 2));
}

#[test]
fn 最大値の行番号でもパニックしない() {
    // レビューで見つかった、l + 1 のあふれ (debug でパニック、release で別の行を指す)
    for text in ["", "a\r\nb", "あ😀\r"] {
        let t = Text::from(text);
        assert_eq!(t.line_terminator(LineIdx(usize::MAX)), None);
        assert_eq!(t.line_end_byte(LineIdx(usize::MAX)), ByteOff(t.len_bytes()));
        assert_eq!(t.line_to_byte(LineIdx(usize::MAX)), ByteOff(t.len_bytes()));
    }
}

#[test]
fn 逆順の範囲の長さは_0() {
    let r = ByteRange {
        start: ByteOff(3),
        end: ByteOff(1),
    };
    assert_eq!(r.len(), 0);
    assert!(r.is_empty());
}

// TODO(1-b): ChangeSet の適用を入れたら、INV-T3 (apply の後も、旧 Snapshot が旧内容を保つ) のテストをここに足す。

// 空の範囲 (逆順を含む) は、char の途中の位置でも空のまま (CI の proptest で見つかった、チャンクをまたぐ長いテキストの再現)
#[test]
fn 空の範囲は_char_の途中でも空のまま() {
    let s = "あ😀éa".repeat(2000);
    let t = Text::from(s.as_str());
    for p in 0..=s.len() {
        let empty = ByteRange {
            start: ByteOff(p),
            end: ByteOff(p),
        };
        assert_eq!(t.chunks(empty).count(), 0, "位置 {p}");
        assert_eq!(t.slice_to_string(empty), "", "位置 {p}");
        if p > 0 {
            let rev = ByteRange {
                start: ByteOff(p),
                end: ByteOff(p - 1),
            };
            assert_eq!(t.chunks(rev).count(), 0, "逆順 {p}");
            assert_eq!(t.slice_to_string(rev), "", "逆順 {p}");
        }
    }
    // 空でない範囲は従来どおり外側の char 境界へ丸める (あ = 0..3 の途中 1..2 → 0..3)
    let r = ByteRange {
        start: ByteOff(1),
        end: ByteOff(2),
    };
    assert_eq!(t.slice_to_string(r), "あ");
}
