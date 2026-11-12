//! エスケープ文字のつながりの検出 (03 §8.3.1、§22.1 の「不正バイトのつながり」、09 §3.2 の M23 の行) の、保存のジョブの
//! 側の試験 (M23 の 3 本目)。
//!
//! - 性質テスト: 任意の本文 (エスケープ文字・通常の文字・改行・保持方式の記録) × 5 つの文字コード × BOM の有無で、
//!   (a) 局所の検査が 1 件も見つけない ⇔ 書いた BOM を除いた出力の全体を同じ文字コードで復号すると本文に一致する、
//!   (b) 出力の全体の復号の最初の不一致は、最初の 1 件の強調の範囲の中にある、
//!   (c) つながりが無ければ、同じ文字コードを指定して開き直すと本文に戻る (NFR-304 AC-4)、
//!   (d) 区切った確認 (ii) は、記録が正しい限り一致する (`VerifyFailed` を出さない)、
//!   (e) 乾式走査と 2 回目の走査で件数・位置・ダイジェスト・変わる範囲が一致し、本番の書き込み (変わる範囲は求めない) が
//!   件数・先頭 100 件・ダイジェストが乾式走査と同じとして最後まで書く、
//!   (f) 件数・位置・強調の範囲・局所の復号が、並びごとに作り直した復号器で求めた参照と一致し、変わる範囲が参照
//!   (03 §8.3.1 の閉じ方の定義どおりに、区切らない復号器を部品ごとに進める。重なる範囲をまとめる) と一致する。
//! - 具体例: 保存のジョブ (`run_save`) が何も書かずに `EscapeMerge` を返す (原子的保存でも直接上書きでも書込が 0 回)、
//!   承認と結果 (件数・先頭 100 件・全件のダイジェスト) が同じときだけ書く (101 件目より後ろの違いも見る)、表現不能が先、
//!   BOM 付きの不正バイトを含む文書が `VerifyFailed` にならない、`abandon_preserved` が `NotApplicable` を変えない、
//!   検出器が並びを見逃すと区切った確認 (ii) が止める (故障の注入)。

use std::sync::Arc;

use proptest::prelude::*;

use eve_encoding::{
    Decoder, EncodeReport, Encoder, Encoding, EscapeMergeAt, EscapePolicy, PreserveReason,
    escape_byte, preserved_matches,
};
use eve_text::{ByteOff, ByteRange, Text};

use super::preserve_save::{Clock0, OneFile, Paths0, STRATEGIES};
use super::*;
use crate::document::preserved::PreservedSpan;
use crate::jobs::SaveStrategy;

/// 本文の部品。
#[derive(Clone, Copy, Debug)]
enum Tok {
    /// 不正バイトのエスケープ文字。
    E(u8),
    /// 通常の文字。
    C(char),
    /// 保持方式の記録 (範囲の文字列と元のバイト列)。
    R(&'static str, &'static [u8]),
}

const ENCS: [Encoding; 5] = [
    Encoding::Utf8,
    Encoding::Utf16Le,
    Encoding::Utf16Be,
    Encoding::ShiftJis,
    Encoding::EucJp,
];

/// つながりやすいバイト (UTF-8 の多バイト列の頭と続き、`Shift_JIS` / EUC-JP の先頭バイト、BOM、UTF-16 のサロゲート)。
const HOT: &[u8] = &[
    0xE3, 0x81, 0x82, 0xEF, 0xBB, 0xBF, 0xFF, 0xFE, 0x8F, 0xA4, 0xA2, 0x88, 0x9F, 0x80, 0xC3, 0xA9,
    0x41, 0x00, 0xD8, 0xDC, 0x0A, 0x40,
];

fn chars(enc: Encoding) -> &'static [char] {
    match enc {
        Encoding::Utf8 => &[
            'a', '\n', '\r', 'é', 'あ', '😀', '\u{80}', '\u{7FF}', '\u{FEFF}', '@',
        ],
        Encoding::Utf16Le | Encoding::Utf16Be => &[
            'a', '\n', '\r', 'あ', '😀', 'Ø', 'þ', '\u{FFD8}', '\u{FEFF}', '\u{0A00}', '\u{DC}',
            '\u{D7FF}', '\u{E000}',
        ],
        Encoding::ShiftJis => &[
            'a', '@', '\n', '\r', 'あ', '亜', '表', 'ｱ', 'ｿ', 'ｻ', '\u{E000}', '＝', '\u{80}', '~',
        ],
        _ => &['a', '\n', '\r', 'あ', '亜', 'ｱ', '~', '\u{3000}'],
    }
}

fn records(enc: Encoding) -> &'static [(&'static str, &'static [u8])] {
    match enc {
        Encoding::ShiftJis => &[("￢", b"\xFA\x54"), ("≒", b"\x87\x90"), ("∵", b"\xFA\x5B")],
        Encoding::EucJp => &[("丂", b"\x8F\xB0\xA1"), ("～", b"\x8F\xA2\xB7")],
        _ => &[],
    }
}

/// 1 文字を `Restore` で符号化したバイト列。
fn enc_char(enc: Encoding, c: char) -> Vec<u8> {
    let mut e = Encoder::new(enc, false, EscapePolicy::Restore);
    let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
    e.encode(&c.to_string(), &mut out, &mut rep);
    e.finish(&mut out, &mut rep);
    assert!(rep.is_empty(), "{enc:?} {c:?} は表せる");
    out
}

fn tok_text(t: Tok) -> String {
    match t {
        Tok::E(b) => escape_byte(b).to_string(),
        Tok::C(c) => c.to_string(),
        Tok::R(s, _) => s.to_owned(),
    }
}

fn tok_bytes(enc: Encoding, t: Tok) -> Vec<u8> {
    match t {
        Tok::E(b) => vec![b],
        Tok::C(c) => enc_char(enc, c),
        Tok::R(_, b) => b.to_vec(),
    }
}

fn decode(enc: Encoding, bytes: &[u8], skip_bom: bool) -> String {
    let mut d = Decoder::new(enc, skip_bom);
    let mut s = String::new();
    d.decode(bytes, true, &mut s);
    s
}

/// 部品から作った文書 (本文・記録の表・部品ごとの出力)。
struct Doc {
    toks: Vec<Tok>,
    text: String,
    map: PreservedMap,
    parts: Vec<Vec<u8>>,
    /// 部品の先頭の文字の通し番号 (末尾に総数)。
    char_at: Vec<u64>,
}

fn build(enc: Encoding, toks: Vec<Tok>) -> Doc {
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut char_at = vec![0u64];
    for &t in &toks {
        let s = tok_text(t);
        if let Tok::R(_, b) = t {
            spans.push(PreservedSpan {
                range: ByteRange::new(ByteOff(text.len()), ByteOff(text.len() + s.len())),
                bytes: b.to_vec().into(),
                reason: PreserveReason::DuplicateEncoding,
                line: false,
            });
        }
        text.push_str(&s);
        char_at.push(char_at.last().copied().unwrap_or(0) + s.chars().count() as u64);
    }
    let parts = toks.iter().map(|&t| tok_bytes(enc, t)).collect();
    Doc {
        toks,
        text,
        map: PreservedMap::from_spans(&spans),
        parts,
        char_at,
    }
}

fn first_diff(a: &str, b: &str) -> u64 {
    a.chars()
        .zip(b.chars())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.chars().count().min(b.chars().count())) as u64
}

/// 参照: 並びごとに作り直した復号器で、並びと直後の 1 部品を復号して比べる (03 §8.3.1 の局所の検査の定義どおり)。
/// 見つけた並びの (並びの先頭の部品の番号, 1 件)。
fn ref_local(enc: Encoding, d: &Doc) -> Vec<(usize, EscapeMergeAt)> {
    let n = d.toks.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if !matches!(d.toks[i], Tok::E(_)) {
            i += 1;
            continue;
        }
        let s = i;
        while i < n && matches!(d.toks[i], Tok::E(_)) {
            i += 1;
        }
        let e = (i + 1).min(n);
        let bytes: Vec<u8> = d.parts[s..e].concat();
        let want: Vec<char> = d.toks[s..e]
            .iter()
            .flat_map(|&t| tok_text(t).chars().collect::<Vec<_>>())
            .collect();
        let got: Vec<char> = decode(enc, &bytes, false).chars().collect();
        let Some(k) = (0..want.len().max(got.len())).find(|&j| want.get(j) != got.get(j)) else {
            continue;
        };
        let run_len = i - s;
        let p_idx = k.min(run_len - 1);
        out.push((
            s,
            EscapeMergeAt {
                char_index: d.char_at[s] + p_idx as u64,
                len_chars: u32::try_from(want.len() - p_idx).unwrap(),
                decoded: got.iter().skip(p_idx).take(16).collect(),
            },
        ));
    }
    out
}

/// 参照: 変わる範囲 (03 §8.3.1 の「示す位置と変わる範囲」の定義どおり)。つながりの位置から、区切らない復号器を部品
/// ごとに進め、範囲の中の最後のつながりの位置より後ろの部品で、出たものがその部品の文字で終わり持ち越しが無くなったら、
/// その部品の前で閉じる。範囲の中で次のつながりに来たら範囲を延ばす (重なる範囲は 1 つ)。閉じた後のつながりは、
/// そこから新しい復号器で次の範囲を始める (閉じた所では持ち越しが無いので、新しい復号器と同じ)。`flags` は
/// (並びの先頭の部品の番号, つながりの位置) の昇順。
fn ref_ranges(
    enc: Encoding,
    d: &Doc,
    flags: &[(usize, u64)],
    bom_written: bool,
) -> (u64, u64, bool) {
    let n = d.toks.len();
    let total = *d.char_at.last().unwrap();
    let unit_of = |p: u64| d.char_at.iter().position(|&c| c == p).unwrap();
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    let mut to_eof = false;
    let mut resume = 0usize;
    let mut i = 0;
    while i < flags.len() {
        let (s, p) = flags[i];
        i += 1;
        let from = s.max(resume);
        let out_start: usize = d.parts[..from].iter().map(Vec::len).sum();
        let mut c = Decoder::new(enc, out_start == 0 && !bom_written);
        let mut p_last = p;
        let mut closed = None;
        for t in from..n {
            while i < flags.len() && unit_of(flags[i].1) <= t {
                p_last = p_last.max(flags[i].1);
                i += 1;
            }
            let mut buf = String::new();
            c.decode(&d.parts[t], false, &mut buf);
            if d.char_at[t] > p_last && c.is_neutral() && buf.ends_with(&tok_text(d.toks[t])) {
                closed = Some(t);
                break;
            }
        }
        match closed {
            Some(t) => {
                ranges.push((p, d.char_at[t]));
                resume = t + 1;
                // 閉じた所より前の位置のつながりは、この範囲にまとめた
                while i < flags.len() && flags[i].1 < d.char_at[t] {
                    i += 1;
                }
            }
            None => {
                ranges.push((p, total));
                to_eof = !c.is_neutral();
                break;
            }
        }
    }
    // 行の数: 範囲の文字の行 (CRLF の LF は CR の行)
    let mut line = 0u64;
    let mut prev_cr = false;
    let mut lines = std::collections::BTreeSet::new();
    for (i, ch) in d.text.chars().enumerate() {
        let i = i as u64;
        let l = if ch == '\n' && prev_cr {
            line - 1
        } else {
            line
        };
        if ranges.iter().any(|&(a, b)| (a..b).contains(&i)) {
            lines.insert(l);
        }
        match ch {
            '\r' => line += 1,
            '\n' if !prev_cr => line += 1,
            _ => {}
        }
        prev_cr = ch == '\r';
    }
    let chars = ranges.iter().map(|(a, b)| b - a).sum();
    (chars, lines.len() as u64, to_eof)
}

/// 部品の選び方: (エスケープ文字か, 選ぶ値)。
fn toks_strategy() -> impl Strategy<Value = Vec<(u8, u16)>> {
    prop::collection::vec((0u8..10, any::<u16>()), 0..24)
}

fn pick(enc: Encoding, raw: &[(u8, u16)]) -> Vec<Tok> {
    let cs = chars(enc);
    let rs: Vec<(&'static str, &'static [u8])> = records(enc)
        .iter()
        .copied()
        .filter(|(s, b)| preserved_matches(enc, b, s))
        .collect();
    raw.iter()
        .map(|&(kind, v)| {
            let v = usize::from(v);
            match kind {
                // エスケープ文字 (つながりやすいバイトに寄せる)。UTF-8 / Shift_JIS / EUC-JP の不正バイトは 0x80 以上
                0..=2 => {
                    let b = HOT[v % HOT.len()];
                    if enc.is_utf16() || b >= 0x80 {
                        Tok::E(b)
                    } else {
                        Tok::E(0x80 | b)
                    }
                }
                3..=4 => {
                    let b = u8::try_from(v % 256).unwrap();
                    if enc.is_utf16() || b >= 0x80 {
                        Tok::E(b)
                    } else {
                        Tok::E(0x80 | b)
                    }
                }
                5 if !rs.is_empty() => {
                    let (s, b) = rs[v % rs.len()];
                    Tok::R(s, b)
                }
                _ => Tok::C(cs[v % cs.len()]),
            }
        })
        .collect()
}

/// 1 つの文書を、乾式走査・2 回目の走査・本番の書き込みの供給で保存し、(a)〜(f) を確かめる。
fn check(enc: Encoding, bom: bool, toks: Vec<Tok>) -> Result<(), TestCaseError> {
    let d = build(enc, toks);
    let text = Text::from(d.text.as_str());
    let out = save_for_test_bom(&text.snapshot(), enc, bom, EscapePolicy::Restore, &d.map);
    let bom_bytes: &[u8] = if bom {
        enc.bom().unwrap_or_default()
    } else {
        &[]
    };
    let bom_written = !bom_bytes.is_empty();
    let body = d.parts.concat();
    prop_assert_eq!(
        &out.bytes,
        &[bom_bytes, &body].concat(),
        "出力は部品ごとの符号化をつないだもの"
    );
    prop_assert_eq!(out.unmappable, 0);
    // (d) 区切った確認 (ii)・確認 (i)
    prop_assert!(out.verified && out.broken == 0, "{:?}", out);
    let m = out.merges.clone().expect("Restore はつながりを調べる");
    // (e) 乾式走査と 2 回目の走査、本番の書き込みの供給 (乾式走査の結果と同じでなければ止まる)
    prop_assert_eq!(Some(&m), out.merges_again.as_ref());
    prop_assert_eq!(out.source.as_ref(), Some(&out.bytes));
    prop_assert_eq!(out.dry_len, out.bytes.len() as u64);
    prop_assert_eq!(
        out.restored,
        d.toks.iter().filter(|t| matches!(t, Tok::E(_))).count() as u64
    );
    // (a) 局所の検査が 0 件 ⇔ 書いた BOM を除いた出力の全体を、同じ文字コードを指定して復号すると本文に一致する
    let full = decode(enc, &body, false);
    prop_assert_eq!(m.total == 0, full == d.text, "{:?} {:02X?}", d.toks, body);
    // (c) つながりが無ければ、開き直す (BOM を読み飛ばす) と本文に戻る。本文の先頭の U+FEFF (BOM の無い文書) は対象外
    if m.total == 0 && !(d.text.starts_with('\u{FEFF}') && !bom_written) {
        prop_assert_eq!(decode(enc, &out.bytes, true), d.text.clone());
    }
    // (f) 局所の検査の参照
    let r = ref_local(enc, &d);
    if let Some((_, first)) = r.first() {
        // (b) 出力の全体の復号の最初の不一致は、最初の 1 件の強調の範囲の中
        let i = first_diff(&full, &d.text);
        prop_assert!(
            first.char_index <= i && i < first.char_index + u64::from(first.len_chars),
            "{} {:?} {:?}",
            i,
            first,
            d.toks
        );
    }
    let ent: Vec<EscapeMergeAt> = r.iter().map(|(_, a)| a.clone()).collect();
    prop_assert_eq!(m.total, r.len() as u64);
    prop_assert_eq!(&m.recorded, &ent);
    // (f) 変わる範囲 (重なる範囲をまとめる場合を含む)
    let flags: Vec<(usize, u64)> = r.iter().map(|(s, a)| (*s, a.char_index)).collect();
    let want = ref_ranges(enc, &d, &flags, bom_written);
    prop_assert_eq!(
        (m.changed_chars, m.changed_lines, m.to_eof),
        want,
        "{:?}",
        d.toks
    );
    if m.total == 0 {
        prop_assert_eq!((m.changed_chars, m.changed_lines, m.to_eof), (0, 0, false));
    } else {
        prop_assert!(m.changed_chars >= 1, "{:?}", m);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // verifies: NFR-304
    #[test]
    fn つながりの局所の検査は出力の全体の復号と同値で_乾式走査と本番で同じ(
        e in 0usize..5,
        bom in any::<bool>(),
        raw in toks_strategy(),
    ) {
        let enc = ENCS[e];
        check(enc, bom, pick(enc, &raw))?;
    }
}

/// 決定的な擬似乱数 (xorshift)。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// `alpha` の部品の長さ `len` の並びを全部試す。
fn exhaust(enc: Encoding, alpha: &[Tok], len: usize, boms: &[bool]) {
    let n = alpha.len();
    for code in 0..n.pow(u32::try_from(len).unwrap()) {
        let mut c = code;
        let toks: Vec<Tok> = (0..len)
            .map(|_| {
                let t = alpha[c % n];
                c /= n;
                t
            })
            .collect();
        for &bom in boms {
            check(enc, bom, toks.clone()).unwrap_or_else(|e| panic!("{enc:?} {bom} {toks:?}: {e}"));
        }
    }
}

// 総当たり: 長さ 2 までは全部の部品、長さ 3 は部品を絞って (並び・直後の単位・記録の組み合わせ)
#[test]
fn 短い並びの総当たりでも性質が成り立つ() {
    for enc in ENCS {
        let escapes: Vec<Tok> = HOT
            .iter()
            .filter(|&&b| enc.is_utf16() || b >= 0x80)
            .map(|&b| Tok::E(b))
            .collect();
        let recs: Vec<Tok> = records(enc)
            .iter()
            .filter(|(s, b)| preserved_matches(enc, b, s))
            .map(|&(s, b)| Tok::R(s, b))
            .collect();
        let mut alpha = escapes.clone();
        alpha.extend(chars(enc).iter().map(|&c| Tok::C(c)));
        alpha.extend(recs.iter().copied());
        exhaust(enc, &alpha, 1, &[false, true]);
        exhaust(enc, &alpha, 2, &[false, true]);
        let mut small: Vec<Tok> = escapes.iter().step_by(3).copied().collect();
        small.extend(chars(enc).iter().take(5).map(|&c| Tok::C(c)));
        small.extend(recs.iter().take(1).copied());
        exhaust(enc, &small, 3, &[false]);
    }
}

// rope のチャンクの形に依らない: 同じ本文を小さな挿入の積み重ねで作った (断片化した) 文書でも、出力とつながりが同じ
#[test]
fn つながりは_rope_のチャンクの形に依らない() {
    use crate::document::{Document, Origin, TextFormat, Transaction};
    let mut rng = Rng(0x0DDB_1A5E_5BAD_5EED);
    for enc in ENCS {
        for _ in 0..12 {
            let mut raw = Vec::new();
            while raw.len() < 3000 {
                let v = rng.next();
                raw.push((
                    u8::try_from(v % 10).unwrap(),
                    u16::try_from((v >> 8) % 65536).unwrap(),
                ));
            }
            let d = build(enc, pick(enc, &raw));
            let whole_text = Text::from(d.text.as_str());
            let whole = save_for_test(&whole_text.snapshot(), enc, EscapePolicy::Restore, &d.map);
            let mut doc = Document::from_text(Text::new(), TextFormat::new_document());
            let mut at = 0usize;
            let cs: Vec<(usize, char)> = d.text.char_indices().collect();
            let mut i = 0usize;
            while i < cs.len() {
                let n = 1 + usize::try_from(rng.next() % 7).unwrap();
                let end = cs.get(i + n).map_or(d.text.len(), |c| c.0);
                let piece = &d.text[cs[i].0..end];
                let change = eve_text::ChangeSet::from_sorted_edits(
                    doc.text(),
                    [eve_text::Edit {
                        range: ByteRange::new(ByteOff(at), ByteOff(at)),
                        insert: piece.into(),
                    }],
                )
                .unwrap();
                doc.apply(&Transaction::new(
                    change,
                    Origin::Paste,
                    crate::time::Tick(0),
                ))
                .unwrap();
                at += piece.len();
                i += n;
            }
            let frag = save_for_test(&doc.snapshot(), enc, EscapePolicy::Restore, &d.map);
            assert_eq!(frag.bytes, whole.bytes, "{enc:?}");
            assert_eq!(frag.merges, whole.merges, "{enc:?}");
            assert!(frag.verified && whole.verified);
            assert_eq!(frag.source.as_ref(), Some(&frag.bytes));
        }
    }
}

// ---------- 保存のジョブ (`run_save`) ----------

fn run(
    fs: &Arc<OneFile>,
    snapshot: &Snapshot,
    enc: Encoding,
    bom: bool,
    escape: EscapePolicy,
    strategy: SaveStrategy,
    ok: Option<&EscapeMergeApproval>,
) -> JobResult {
    let ports = JobPorts {
        fs: Arc::clone(fs) as Arc<dyn crate::io::fs::FileSystem>,
        clock: Arc::new(Clock0),
        paths: Arc::new(Paths0),
    };
    let path = FsPath::from("/w/a.txt");
    let empty = PreservedMap::new();
    run_save(
        SaveParams {
            path: &path,
            snapshot,
            encoding: enc,
            bom,
            escape,
            expected: Expect::Any,
            strategy,
            clear_readonly: false,
            create_link_target: false,
            preserved: &empty,
            escape_merge_ok: ok,
        },
        &ports,
        &CancelToken::new(),
    )
}

fn merge_of(r: &JobResult) -> Option<EscapeMergeApproval> {
    match r {
        JobResult::EscapeMerge {
            total,
            recorded,
            digest,
            ..
        } => Some(EscapeMergeApproval {
            total: *total,
            recorded: recorded.clone(),
            digest: *digest,
        }),
        _ => None,
    }
}

// verifies: NFR-304
#[test]
fn つながりがあれば原子的保存でも直接上書きでも何も書かずに_escape_merge_を返す() {
    let original = b"original\n".to_vec();
    // UTF-8 で <0xE3><0x81><0x82> (「あ」になる) と、Shift_JIS で <0x81> の直後に @
    let cases = [
        (Encoding::Utf8, "a\u{10FFE3}\u{10FF81}\u{10FF82}b\n"),
        (Encoding::ShiftJis, "x\u{10FF81}@y\n"),
        (Encoding::Utf16Le, "\u{10FF41}abc\ndef"),
    ];
    for (enc, s) in cases {
        let text = Text::from(s);
        for strategy in STRATEGIES {
            let fs = OneFile::new(&original);
            let r = run(
                &fs,
                &text.snapshot(),
                enc,
                false,
                EscapePolicy::Restore,
                strategy,
                None,
            );
            let JobResult::EscapeMerge {
                total,
                ref recorded,
                changed_chars,
                changed_lines,
                to_eof,
                ..
            } = r
            else {
                panic!("{enc:?} {strategy:?}: {r:?}");
            };
            assert_eq!(total, 1);
            assert_eq!(
                fs.writes(),
                0,
                "{enc:?} {strategy:?}: 一時ファイルもバックアップも作らない"
            );
            assert_eq!(fs.content(), original);
            match enc {
                Encoding::Utf8 => {
                    assert_eq!((recorded[0].char_index, recorded[0].len_chars), (1, 4));
                    assert_eq!(recorded[0].decoded, "あb");
                    assert_eq!((changed_chars, changed_lines, to_eof), (3, 1, false));
                }
                Encoding::ShiftJis => {
                    assert_eq!((recorded[0].char_index, recorded[0].len_chars), (1, 2));
                    assert_eq!((changed_chars, to_eof), (2, false));
                }
                _ => {
                    // UTF-16 の文書の先頭の奇数個の並び: ファイルの全部の文字が別の文字になる
                    assert_eq!(recorded[0].char_index, 0);
                    assert_eq!((changed_chars, changed_lines, to_eof), (8, 2, true));
                }
            }
        }
    }
}

// 承認 (「このまま保存」) は結果で照らし合わせる: 件数と位置が同じときだけ書き、違えばもう一度 EscapeMerge
#[test]
fn 承認と件数と位置が同じときだけ書く() {
    let text = Text::from("\u{10FF81}@x\n\u{10FF81}@y\n");
    let snap = text.snapshot();
    for strategy in STRATEGIES {
        let fs = OneFile::new(b"old");
        let first = run(
            &fs,
            &snap,
            Encoding::ShiftJis,
            false,
            EscapePolicy::Restore,
            strategy,
            None,
        );
        let ok = merge_of(&first).expect("つながりがある");
        assert_eq!(ok.total, 2);
        // 同じ結果を承認した保存は書く (書いたバイト列は、エスケープ文字を元のバイトに戻したもの)
        let r = run(
            &fs,
            &snap,
            Encoding::ShiftJis,
            false,
            EscapePolicy::Restore,
            strategy,
            Some(&ok),
        );
        assert!(matches!(r, JobResult::Saved { .. }), "{r:?}");
        assert_eq!(fs.content(), b"\x81@x\n\x81@y\n");
        assert_eq!(fs.writes(), 1);
        // 件数・位置・強調の範囲・局所の復号・全件のダイジェストのどれかが違う承認では書かない
        let mut fewer = ok.clone();
        fewer.total = 1;
        fewer.recorded.truncate(1);
        let mut moved = ok.clone();
        moved.recorded[1].char_index += 1;
        let mut wider = ok.clone();
        wider.recorded[0].len_chars += 1;
        let mut other = ok.clone();
        other.recorded[0].decoded.push('x');
        let mut digest = ok.clone();
        digest.digest ^= 1;
        for bad in [fewer, moved, wider, other, digest] {
            let fs = OneFile::new(b"old");
            let r = run(
                &fs,
                &snap,
                Encoding::ShiftJis,
                false,
                EscapePolicy::Restore,
                strategy,
                Some(&bad),
            );
            assert_eq!(merge_of(&r).as_ref(), Some(&ok), "{r:?}");
            assert_eq!(fs.writes(), 0);
        }
        // 承認がつながりの無い保存に渡っても、ふつうに書く
        let clean = Text::from("\u{10FF81}\nx");
        let fs = OneFile::new(b"old");
        let r = run(
            &fs,
            &clean.snapshot(),
            Encoding::ShiftJis,
            false,
            EscapePolicy::Restore,
            strategy,
            Some(&ok),
        );
        assert!(matches!(r, JobResult::Saved { .. }), "{r:?}");
        assert_eq!(fs.content(), b"\x81\nx");
    }
}

// 見る順序 (03 §8.3.1): 表現不能が先。元のバイトに戻したエスケープ文字の数を添える (E3 の「UTF-8 で保存」を出さない)
#[test]
fn 表現不能とつながりが両方あれば表現不能が先() {
    let text = Text::from("\u{10FF81}@x😀\n\u{10FFA0}");
    for strategy in STRATEGIES {
        let fs = OneFile::new(b"old");
        let r = run(
            &fs,
            &text.snapshot(),
            Encoding::ShiftJis,
            false,
            EscapePolicy::Restore,
            strategy,
            None,
        );
        let JobResult::Unmappable {
            total,
            escapes_unmappable,
            escapes_restored,
            ..
        } = r
        else {
            panic!("{r:?}");
        };
        assert_eq!((total, escapes_unmappable, escapes_restored), (1, 0, 2));
        assert_eq!(fs.writes(), 0);
    }
    // 別の文字コードへの保存 (Reject) は、エスケープ文字が表現不能で、戻したものは 0
    let fs = OneFile::new(b"old");
    let r = run(
        &fs,
        &text.snapshot(),
        Encoding::EucJp,
        false,
        EscapePolicy::Reject,
        STRATEGIES[0],
        None,
    );
    assert!(
        matches!(
            r,
            JobResult::Unmappable {
                escapes_unmappable: 2,
                escapes_restored: 0,
                ..
            }
        ),
        "{r:?}"
    );
}

// 回帰: BOM 付きの不正バイトを含む UTF-8 / UTF-16 の文書は、確認 (ii) が BOM を読み飛ばさずに毎回 VerifyFailed になって
// いた (03 §8.3.1 の「復号する範囲」)。書いた BOM を除いて復号するので、元のファイルとバイト単位で一致して書ける
// verifies: NFR-304
#[test]
fn bom_付きの不正バイトを含む文書は_verify_failed_にならない() {
    let files: [(Encoding, &[u8]); 4] = [
        (Encoding::Utf8, b"\xEF\xBB\xBFa\x80b\n\xFF"),
        (Encoding::Utf16Le, b"\xFF\xFEa\x00\x00\xD8b\x00c"),
        (Encoding::Utf16Be, b"\xFE\xFF\x00a\xDC\x00\x00b\x00"),
        (Encoding::Utf8, b"\xEF\xBB\xBF\xE3\x81\n"),
    ];
    for (enc, file) in files {
        let mut d = Decoder::new(enc, true);
        let mut s = String::new();
        d.decode(file, true, &mut s);
        assert!(d.has_bom() && d.invalid_bytes() > 0, "{enc:?}");
        let text = Text::from(s.as_str());
        for strategy in STRATEGIES {
            let fs = OneFile::new(b"");
            let r = run(
                &fs,
                &text.snapshot(),
                enc,
                true,
                EscapePolicy::Restore,
                strategy,
                None,
            );
            assert!(
                matches!(r, JobResult::Saved { .. }),
                "{enc:?} {strategy:?}: {r:?}"
            );
            assert_eq!(fs.content(), file, "{enc:?}");
        }
    }
}

// 記録の無い文書 (保持の状態が NotApplicable) の VerifyFailed では、保持の状態を変えない (03 §8.3.1)。記録を持つ文書は
// これまでどおり「保持なし (照合に失敗した)」にする
#[test]
fn abandon_preserved_は_not_applicable_を変えない() {
    use crate::document::{Document, TextFormat};
    use eve_encoding::{NoPreserveReason, PreserveStatus};
    let mut doc = Document::from_text(Text::from("a"), TextFormat::new_document());
    assert_eq!(doc.format.roundtrip.preserve, PreserveStatus::NotApplicable);
    doc.abandon_preserved();
    assert_eq!(doc.format.roundtrip.preserve, PreserveStatus::NotApplicable);
    doc.format.roundtrip.preserve = PreserveStatus::Active { unpreserved: 0 };
    doc.abandon_preserved();
    assert_eq!(
        doc.format.roundtrip.preserve,
        PreserveStatus::Unavailable(NoPreserveReason::VerifyFailed)
    );
}

// 乾式走査で見なかったつながりが本番の書き込みで出たら (不変条件の違反)、書くのを止める
#[test]
fn 本番の書き込みのつながりが乾式走査と違えば止める() {
    let text = Text::from("\u{10FF81}@x");
    let snap = text.snapshot();
    let empty = PreservedMap::new();
    let cancel = CancelToken::new();
    for expected in [None, Some(MergeReport::default())] {
        let mut src = SnapshotSource {
            stream: SaveStream::new(
                &snap,
                Encoding::ShiftJis,
                false,
                EscapePolicy::Restore,
                &empty,
                false,
            ),
            buf: Vec::new(),
            cancel: &cancel,
            hasher: None,
            written: 0,
            expected_merges: expected,
            merge_mismatch: false,
        };
        let r = loop {
            match src.next_chunk() {
                Ok(Some(_)) => {}
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            }
        };
        assert!(r.is_err() && src.merge_mismatch);
    }
}

// 承認は全件のダイジェストでも照らし合わせる: 101 件目より後ろでつながる位置が変わった (件数と先頭 100 件は同じ) 文書に、
// 前の結果の承認を渡しても書かない (データの損失のレビューの再現)
#[test]
fn 承認は_101_件目より後ろの違いも見る() {
    let line_m = "\u{10FF81}@\n"; // つながる (0x8140)
    let line_c = "\u{10FF81}\n"; // つながらない
    let a: String = (0..160)
        .map(|i| if i < 150 { line_m } else { line_c })
        .collect();
    // B: 120 行目をつながらない行に、155 行目をつながる行にする (件数は同じ 150、先頭 100 件も同じ)
    let b: String = (0..160)
        .map(|i| {
            if (i < 150 && i != 120) || i == 155 {
                line_m
            } else {
                line_c
            }
        })
        .collect();
    let (snap_a, snap_b) = (
        Text::from(a.as_str()).snapshot(),
        Text::from(b.as_str()).snapshot(),
    );
    for strategy in STRATEGIES {
        let fs = OneFile::new(b"old");
        let ra = run(
            &fs,
            &snap_a,
            Encoding::ShiftJis,
            false,
            EscapePolicy::Restore,
            strategy,
            None,
        );
        let ok = merge_of(&ra).expect("つながりがある");
        assert_eq!(ok.total, 150);
        let rb = run(
            &fs,
            &snap_b,
            Encoding::ShiftJis,
            false,
            EscapePolicy::Restore,
            strategy,
            Some(&ok),
        );
        let got = merge_of(&rb).expect("承認していない位置のつながりがあるので書かない");
        assert_eq!((got.total, &got.recorded), (ok.total, &ok.recorded));
        assert_ne!(got.digest, ok.digest);
        assert_eq!(fs.writes(), 0, "{strategy:?}");
    }
}

// 故障の注入: 検出器がつながる並びを見逃した (区切りが無い) とき、区切った確認 (ii) が不一致で止める。区切りを外から
// 与えて確かめる。区切りがあれば一致する。つながるかが決まらない並びの所からは、段をまたいで出力を持って待つ
#[test]
fn 検出器が並びを見逃すと区切った確認_ii_で止める() {
    use eve_encoding::MergeRegion;
    use std::collections::VecDeque;
    let text = Text::from("x\u{10FF81}@y\n");
    let snap = text.snapshot();
    let out = b"x\x81@y\n";
    let verify = |steps: &[&[u8]], regions: &mut dyn FnMut(usize, &mut VecDeque<MergeRegion>)| {
        let mut v = Verify::new(Encoding::ShiftJis, 0);
        let mut rs = VecDeque::new();
        let mut base = 0u64;
        for (i, s) in steps.iter().enumerate() {
            regions(i, &mut rs);
            v.feed_step(&snap, s, base, i + 1 == steps.len(), Some(&mut rs));
            base += s.len() as u64;
        }
        v.finish(&snap);
        v.ok
    };
    let merged = MergeRegion {
        out_start: 1,
        out_end: Some(3),
        text_start: 1,
        text_end: Some(6),
    };
    // 区切りが無い (検出器の見逃し): 0x81 0x40 が全角の空白になって本文と合わない
    assert!(!verify(&[out], &mut |_, _| {}));
    // 区切りがある: つながる範囲の外は一致する
    assert!(verify(&[out], &mut |i, rs| if i == 0 {
        rs.push_back(merged);
    }));
    // 段をまたぐ: 1 段目の終わりではつながるかが決まらず、2 段目の前に決まる
    let open = MergeRegion {
        out_end: None,
        text_end: None,
        ..merged
    };
    assert!(verify(&[b"x\x81", b"@y\n"], &mut |i, rs| match i {
        0 => rs.push_back(open),
        _ => rs[0] = merged,
    }));
    // 決まらないまま区切りが外れた (つながらないと誤って判断した): 持っていた出力をふつうに突き合わせて止める
    assert!(!verify(&[b"x\x81", b"@y\n"], &mut |i, rs| match i {
        0 => rs.push_back(open),
        _ => {
            rs.pop_back();
        }
    }));
    // 本当につながらない並び: 区切りが外れた後、持っていた出力も含めて一致する
    let text2 = Text::from("x\u{10FF81}\ny");
    let snap2 = text2.snapshot();
    let mut v = Verify::new(Encoding::ShiftJis, 0);
    let mut rs = VecDeque::new();
    rs.push_back(open);
    v.feed_step(&snap2, b"x\x81", 0, false, Some(&mut rs));
    rs.pop_back();
    v.feed_step(&snap2, b"\ny", 2, true, Some(&mut rs));
    v.finish(&snap2);
    assert!(v.ok);
}

// 保存の流れ全体: つながらない並び・つながる並び・末尾の並びが混ざっても、確認 (ii) は一致し、区切りは残らない
#[test]
fn つながる並びとつながらない並びが混ざっても確認_ii_は一致する() {
    let text = Text::from("\u{10FF81}\nx\u{10FF81}@y\u{10FFA0}");
    let snap = text.snapshot();
    let empty = PreservedMap::new();
    let mut s = SaveStream::new(
        &snap,
        Encoding::ShiftJis,
        false,
        EscapePolicy::Restore,
        &empty,
        true,
    );
    let mut out = Vec::new();
    while s.step(&mut out) {}
    s.finish_verify();
    assert!(!s.verify_failed());
    assert_eq!(s.merges().map(|m| m.total), Some(1));
    assert!(s.scan.as_mut().is_some_and(|m| m.regions_mut().is_empty()));
}
