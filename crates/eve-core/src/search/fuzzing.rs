//! `fuzz_regex_search` の本体 (docs/09 §3.3、03 §22.1、09 §3.2 の検索の拡張の行)。cargo-fuzz のターゲット
//! (`fuzz/fuzz_targets/fuzz_regex_search.rs`) と、Windows の CI で回す proptest (`crates/eve-core/tests/fuzz_regex_search.rs`)
//! が同じ関数を使う (libFuzzer は MSVC 非対応のため、Linux の夜間ジョブだけで fuzz する)。
//!
//! 入力のバイト列を、検索の条件 (大小区別の 3 値・単語単位・`newline_sugar`・選択範囲)・窓の核の大きさ (1〜40 バイト)・
//! 連結の上限・時計の進み方・S-04 の部品から組んだ検索語・文書・チャンクの分け方 (`Text` を挿入の繰り返しで作る) に読み替え、
//! 行をまたぐ検索を内部の設定で開いた走査の入口 (`scan_range`。行の窓・全文の連結・行の経路) を次の参照と比べる。
//!
//! - **1 つ目の参照**: 全文を 1 本の文字列にして、同じ書き換えの検索語で `regex` を走らせ、03 §12.1 の選び方 (重ならないものを
//!   左から・空の一致は採らない・両端が書記素境界・単語単位は両端のそれぞれの行で) で選んだ列。一致の位置、再開の位置と
//!   正本の一致の終端から走査し直した続き、それを使った前 / 次の一致、すべて置換の展開 (`$0` / `$1` / `${g1}`) を比べる。
//!   k の健全さ (各一致の行の終端のバイトの数 ≤ k、長さ ≤ 最大の長さ) も確かめる。
//! - **呼び出し側** (M24 の (3)、03 §12.3.1 の「呼び出し側」): 全件のジョブ (`run_search`) の一致の集合が 1 つ目の参照と
//!   同じで、行をまたぐ検索の規則 (`Matcher::whole_doc`) では文書の順に届き、`first_after` が起点以降の最初の一致、
//!   再開の位置から走査し直すと正本の続きになる。単発の検索 (`find_next`) で次 / 前を繰り返して得る一致の並びが、全件の
//!   集合 (前は逆順) と同じで、端で折り返す。正本の各一致の 1 件置換 (`replace_one`) が、すべて置換の展開と同じ。
//! - **2 つ目の参照** (書き換えの誤りを見つけるため。1 つ目は参照にも同じ書き換えを掛ける): `\r` も、行の終端に 1 文字ずつ
//!   一致しうる書き方 (`(?s)` の `.`・すべての文字のクラス・要素に `\r` を書いたクラス・`(?-R)`) も含まない検索語について、
//!   CRLF と CR を LF にそろえた文書で `\n` を LF だけ (糖衣なし) として探し、位置を元の文書へ戻した結果と一致する。
//!
//! 最悪の入力の上限 (捨てた候補の予算・処理の速さの下限) で打ち切った入力は、打ち切ったことだけを確かめて比べない。
//! 探さなかった入力 (`ScanEnd::TooLarge`) は、文書が連結の上限を超えることを確かめる。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use eve_text::{
    ByteOff, ByteRange, ChangeSet, Edit, Text, TextRead, is_grapheme_boundary, word_boundaries,
};

use crate::jobs::cancel::CancelToken;
use crate::jobs::{JobOutput, JobResult};
use crate::time::{Clock, Tick};

use super::limits::SEARCH_CONCAT_MAX_BYTES;
use super::{
    Case, FindJob, LineSpan, ScanCtx, ScanEnd, ScanLimit, SearchInternal, SearchJob, SearchQuery,
    build_replace_all, find_next, replace_one, run_search, scan_range,
};

/// 検索語の部品 (書き方, 2 つ目の参照に使えるか)。S-04 と 03 §12.3.1 の表の書き方。
const ATOMS: &[(&str, bool)] = &[
    ("a", true),
    ("b", true),
    ("e", true),
    ("あ", true),
    ("é", true),
    (" ", true),
    ("\"", true),
    ("1", true),
    ("_", true),
    ("σ", true),
    ("\u{301}", true),
    (r"\t", true),
    (".", true),
    ("^", true),
    ("$", true),
    (r"\A", true),
    (r"\z", true),
    (r"\b", true),
    (r"\B", true),
    ("(?i)", true),
    ("(?s)", true),
    ("(?-s)", true),
    ("(?-m)", true),
    ("(?U)", true),
    ("(?-R)", false),
    ("(?-R:^)", false),
    ("(?-R:$)", false),
    (r"\n", true),
    (r"\n", true),
    (r"\x0A", true),
    (r"\r", false),
    (r"\x0D", false),
    (r"\r\n", false),
    (r"\s", true),
    (r"\S", true),
    (r"\w", true),
    (r"\W", true),
    (r"\d", true),
    (r"\pL", true),
    (r#"[^"]"#, true),
    (r"[^a]", true),
    (r"[^\n]", true),
    (r"[\x00-\x1F]", true),
    (r"[\s\S]", false),
    (r"\p{Any}", false),
    (r"[\n ]", true),
    (r"[\s\n]", true),
    (r"[\r ]", false),
    (r"[\r\n]", false),
    (r"\x{10FF0A}", true),
];

/// フラグのグループ `(?flags:…)` のフラグ。
const FLAGS: &[&str] = &["i", "s", "-m", "U", "-i", "is", "-s", "s-i", "-R"];

/// 量指定子。
const QUANTS: &[&str] = &[
    "?", "*", "+", "{2}", "{1,3}", "{0,}", "*?", "+?", "{0}", "??", "{0,3}",
];

/// 文書の部品 (LF・CR・CRLF、結合文字、ZWJ、行の終端でない U+0085・U+2028、エスケープ文字 U+10FF0A、大小の畳み込みの
/// 特別な文字)。
const DOC: &[&str] = &[
    "a",
    "b",
    "e",
    "あ",
    " ",
    "\"",
    "\t",
    "\r",
    "\n",
    "\r\n",
    "\r",
    "\n",
    "\r\n",
    "é",
    "\u{301}",
    "1",
    "_",
    "σ",
    "Σ",
    "ς",
    "ſ",
    "\u{212A}",
    "😀",
    "\u{200D}",
    "\u{85}",
    "\u{2028}",
    "\u{10FF0A}",
    "ab",
    "e\u{301}",
];

/// 置換の文字列 (`$0`・`$1`・名前のグループ。無いグループは空になる)。
const REPLACEMENT: &str = "<$0|$1|${g1}>";

/// 1 件の検査の条件。
#[derive(Clone, Debug)]
pub struct RegexCase {
    /// 検索語 (正規表現)。
    pub pattern: String,
    /// 2 つ目の参照に使える検索語か (`\r`・`(?s)` の `.`・すべての文字のクラス・`\r` を要素に書いたクラス・`(?-R)` を含まない)。
    pub ref2: bool,
    /// 文書。
    pub doc: String,
    /// 大小の区別。
    pub case: Case,
    /// 単語単位。
    pub whole_word: bool,
    /// `\n` の糖衣 (`search.newline_sugar`)。
    pub sugar: bool,
    /// 選択範囲 (バイト。char 境界へ丸め、昇順で重ならない形に直して使う)。
    pub scope: Option<Vec<(usize, usize)>>,
    /// 窓の核の大きさ (バイト)。
    pub core_bytes: usize,
    /// 全文の連結と窓の大きさの上限 (バイト)。
    pub concat_max: usize,
    /// チャンクの分け方 (文書をこの位置で分け、後ろの部分から順に先頭へ挿入して `Text` を作る)。
    pub splits: Vec<usize>,
    /// 時計の 1 回の読み取りで進むミリ秒 (0 は時計なし)。核を縮める経路と処理の速さの下限を通す。
    pub clock_step: u64,
}

/// 検査の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checked {
    /// 参照と比べた。
    Compared,
    /// 比べなかった (検索語が不正・空、探さなかった、打ち切った)。
    Skipped(&'static str),
}

/// 入力のバイト列を検査の条件に読み替えて検査し、参照と食い違えばパニックする (cargo-fuzz のターゲットの本体)。
///
/// # Panics
///
/// 走査の入口の結果が参照と食い違うとき (fuzz が見つける不具合)。
pub fn regex_search(data: &[u8]) {
    if let Err(e) = check(&decode(data)) {
        panic!("{e}");
    }
}

/// 入力のバイト列を検査の条件に読み替える。足りないバイトは 0 として読む。
pub fn decode(data: &[u8]) -> RegexCase {
    let mut r = Reader { data, i: 0 };
    let opt = r.byte();
    let case = [Case::Smart, Case::Sensitive, Case::Insensitive][usize::from(opt % 3)];
    let core = r.byte();
    let core_bytes = if core >= 240 {
        1 << 20
    } else {
        1 + usize::from(core % 40)
    };
    let cmax = r.byte();
    let concat_max = if cmax < 24 {
        usize::from(cmax)
    } else {
        SEARCH_CONCAT_MAX_BYTES
    };
    let clock_step = if opt & 32 != 0 {
        u64::from(r.byte() % 120)
    } else {
        0
    };
    let mut info = PatInfo {
        ref2: true,
        s_flag: false,
        dot: false,
        names: 0,
    };
    let pattern = gen_node(&mut r, 4, &mut info);
    let n = usize::from(r.byte() % 40);
    let mut doc = String::new();
    for _ in 0..n {
        let b = r.byte();
        let part = DOC[usize::from(r.byte()) % DOC.len()];
        if b >= 248 {
            // 長い繰り返し (核をまたぐ長い行と、多くの行)
            doc.push_str(&part.repeat(8 + usize::from(b % 8) * 8));
        } else {
            doc.push_str(part);
        }
    }
    let scope = (opt & 16 != 0).then(|| {
        let k = 1 + usize::from(r.byte() % 3);
        (0..k)
            .map(|_| (usize::from(r.byte()), usize::from(r.byte())))
            .collect()
    });
    let splits = (0..usize::from(r.byte() % 8))
        .map(|_| usize::from(r.byte()))
        .collect();
    RegexCase {
        pattern,
        ref2: info.ref2 && !(info.s_flag && info.dot),
        doc,
        case,
        whole_word: opt & 4 != 0,
        sugar: opt & 8 == 0,
        scope,
        core_bytes,
        concat_max,
        splits,
        clock_step,
    }
}

struct Reader<'a> {
    data: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> u8 {
        let b = self.data.get(self.i).copied().unwrap_or(0);
        self.i = self.i.saturating_add(1);
        b
    }
}

/// 検索語を組みながら覚えること。
struct PatInfo {
    ref2: bool,
    s_flag: bool,
    dot: bool,
    names: u32,
}

/// 検索語の構文木を 1 つ組む (部品・並び・選択・捕獲・名前の捕獲・捕獲しないグループ・フラグのグループ・繰り返し)。
fn gen_node(r: &mut Reader<'_>, depth: u32, info: &mut PatInfo) -> String {
    let b = r.byte();
    if depth == 0 || b < 112 {
        let (a, ok) = ATOMS[usize::from(r.byte()) % ATOMS.len()];
        info.ref2 &= ok;
        info.s_flag |= a == "(?s)";
        info.dot |= a == ".";
        return a.to_owned();
    }
    match b % 7 {
        0 => {
            let n = 1 + r.byte() % 4;
            (0..n).map(|_| gen_node(r, depth - 1, info)).collect()
        }
        1 => {
            let a = gen_node(r, depth - 1, info);
            let c = gen_node(r, depth - 1, info);
            format!("(?:{a}|{c})")
        }
        2 => format!("({})", gen_node(r, depth - 1, info)),
        3 => {
            info.names += 1;
            let n = info.names;
            format!("(?P<g{n}>{})", gen_node(r, depth - 1, info))
        }
        4 => format!("(?:{})", gen_node(r, depth - 1, info)),
        5 => {
            let f = FLAGS[usize::from(r.byte()) % FLAGS.len()];
            info.s_flag |= f.contains('s');
            info.ref2 &= !f.contains('R');
            format!("(?{f}:{})", gen_node(r, depth - 1, info))
        }
        _ => {
            let q = QUANTS[usize::from(r.byte()) % QUANTS.len()];
            format!("(?:{}){q}", gen_node(r, depth - 1, info))
        }
    }
}

/// 時計の読み取りごとに決まった時間だけ進む時計 (核を縮める経路と、処理の速さの下限を通す)。
struct StepClock {
    now: AtomicU64,
    step: u64,
}

impl Clock for StepClock {
    fn now(&self) -> Tick {
        Tick(self.now.fetch_add(self.step, Ordering::Relaxed))
    }
}

/// 参照の一致 (始端, 終端, 置換の展開)。
#[derive(Clone, Debug)]
struct RefMatch {
    s: usize,
    e: usize,
    expanded: String,
}

/// 1 件を検査する。参照と食い違えば `Err` (食い違いの説明)。
///
/// # Errors
///
/// 走査の入口の結果が参照と食い違うとき。
pub fn check(c: &RegexCase) -> Result<Checked, String> {
    let doc = c.doc.as_str();
    let scope = scope_ranges(doc, c.scope.as_deref());
    let internal = SearchInternal {
        multiline: true,
        core_bytes: c.core_bytes.max(1),
        concat_max: c.concat_max,
    };
    let q = SearchQuery::literal(&c.pattern)
        .with_regex(true)
        .with_case(c.case)
        .with_whole_word(c.whole_word)
        .with_newline_sugar(c.sugar)
        .with_scope(scope.clone().map(Arc::from))
        .with_internal(internal);
    let Ok(Some(m)) = q.compile() else {
        return Ok(Checked::Skipped("検索語が不正か空"));
    };
    let Some(pattern) = q
        .analyze()
        .ok()
        .flatten()
        .and_then(|p| p.regex_pattern().map(str::to_owned))
    else {
        return Ok(Checked::Skipped("正規表現でない"));
    };
    let insensitive = !q.is_case_sensitive();
    let Some(re) = reference_regex(&pattern, insensitive) else {
        return Ok(Checked::Skipped("参照を作れない"));
    };
    let t = build_text(doc, &c.splits);
    let got_doc = t.slice_to_string(ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())));
    if got_doc != doc {
        return Err(format!("挿入で作った文書が違う: {got_doc:?} / {doc:?}"));
    }
    let ctx = || {
        format!(
            "検索語 {:?} (書き換え {pattern:?}) 文書 {doc:?} 条件 {c:?}",
            c.pattern
        )
    };

    // 1 つ目の参照
    let reference = select(doc, &re, c.whole_word);
    // k の健全さ
    for r in &reference {
        let ends = doc.as_bytes()[r.s..r.e]
            .iter()
            .filter(|b| matches!(b, b'\r' | b'\n'))
            .count();
        if let LineSpan::Bounded(k) = m.lines()
            && ends > k
        {
            return Err(format!(
                "k = {k} より多い行の終端 ({ends}) の一致 {:?}: {}",
                (r.s, r.e),
                ctx()
            ));
        }
        if let Some(n) = m.max_len()
            && r.e - r.s > n
        {
            return Err(format!(
                "最大の長さ {n} を超える一致 {:?}: {}",
                (r.s, r.e),
                ctx()
            ));
        }
    }
    let want: Vec<&RefMatch> = reference
        .iter()
        .filter(|r| in_scope(scope.as_deref(), r.s, r.e))
        .collect();
    let want_ranges: Vec<(usize, usize)> = want.iter().map(|r| (r.s, r.e)).collect();

    // 走査の入口 (時計なし)。再開の位置を集める
    let mut resumes = Vec::new();
    let (end, got) = scan(&t, &m, 0, None, Some(&mut resumes));
    match end {
        ScanEnd::Done => {}
        ScanEnd::TooHeavy => {
            return Ok(Checked::Skipped(
                "打ち切った (捨てた候補の予算・上限なしの重い検索語)",
            ));
        }
        ScanEnd::TooLarge => {
            if doc.len() <= c.concat_max {
                return Err(format!(
                    "上限 {} 以下の文書で探さなかった: {}",
                    c.concat_max,
                    ctx()
                ));
            }
            return Ok(Checked::Skipped("探さなかった (大きさの上限)"));
        }
        e => return Err(format!("走査が {e:?} で終わった: {}", ctx())),
    }
    if m.too_large_for(doc.len()) {
        return Err(format!("too_large_for なのに探した: {}", ctx()));
    }
    if got != want_ranges {
        return Err(format!(
            "一致が違う: {}\n 実際 {got:?}\n 期待 {want_ranges:?}",
            ctx()
        ));
    }

    // 時計あり (核を縮める経路): 打ち切らなければ同じ結果
    if c.clock_step > 0 {
        let clock = StepClock {
            now: AtomicU64::new(0),
            step: c.clock_step,
        };
        let (end, got2) = scan(&t, &m, 0, Some(&clock), None);
        match end {
            ScanEnd::Done if got2 != want_ranges => {
                return Err(format!(
                    "時計ありで一致が違う: {}\n 実際 {got2:?}\n 期待 {want_ranges:?}",
                    ctx()
                ));
            }
            ScanEnd::Done | ScanEnd::TooHeavy => {}
            e => return Err(format!("時計ありの走査が {e:?} で終わった: {}", ctx())),
        }
    }

    if m.is_stream() {
        // 再開の位置と正本の一致の終端から走査し直すと、正本の続きになる (入口の約束、03 §12.3.1)
        resumes.dedup();
        let mut starts: Vec<usize> = sample(&resumes, 6);
        starts.extend(sample(&want.iter().map(|r| r.e).collect::<Vec<_>>(), 6));
        for &from in &starts {
            let (end, rest) = scan(&t, &m, from, None, None);
            let expect: Vec<(usize, usize)> = want_ranges
                .iter()
                .copied()
                .filter(|r| r.0 >= from)
                .collect();
            if end == ScanEnd::Done && rest != expect {
                return Err(format!(
                    "{from} から走査し直した続きが違う: {}\n 実際 {rest:?}\n 期待 {expect:?}",
                    ctx()
                ));
            }
        }
        // 任意の始端・終端 (char 境界): `a` から選び始めた一致のうち、`b` より前で始まるもの (03 §12.3.1 の「走査の入口の
        // 約束」。始端が正本の位置でなくても、その始端から選んだ結果になる)
        let len = doc.len();
        for pair in c.splits.chunks(2).take(3) {
            let a = floor_char(doc, pair[0].wrapping_mul(7) % (len + 1));
            let b = floor_char(doc, a + pair.get(1).copied().unwrap_or(len) % (len + 1 - a));
            let (end, got) = scan_part(&t, &m, a, b);
            let expect: Vec<(usize, usize)> = select_from(doc, &re, c.whole_word, a)
                .iter()
                .filter(|r| r.s < b && in_scope(scope.as_deref(), r.s, r.e))
                .map(|r| (r.s, r.e))
                .collect();
            if end == ScanEnd::Done && got != expect {
                return Err(format!(
                    "範囲 {a}..{b} の走査が違う: {}\n 実際 {got:?}\n 期待 {expect:?}",
                    ctx()
                ));
            }
        }
        // 前 / 次の一致 (再開の位置から探す。03 §12.3.1 の「次 / 前」の (3))
        for i in 0..=4 {
            let cursor = floor_char(doc, len * i / 4);
            let next = next_from(&t, &m, &resumes, cursor);
            let want_next = want_ranges.iter().copied().find(|r| r.0 >= cursor);
            if next != Some(want_next) {
                return Err(format!(
                    "{cursor} の次の一致が違う: {}\n 実際 {next:?}\n 期待 {want_next:?}",
                    ctx()
                ));
            }
            let prev = prev_from(&t, &m, &resumes, cursor);
            let want_prev = want_ranges.iter().copied().rev().find(|r| r.1 <= cursor);
            if prev != Some(want_prev) {
                return Err(format!(
                    "{cursor} の前の一致が違う: {}\n 実際 {prev:?}\n 期待 {want_prev:?}",
                    ctx()
                ));
            }
        }
    }

    // 呼び出し側: 全件のジョブと、次 / 前の繰り返し (03 §12.3.1 の「呼び出し側」)
    check_callers(c, &t, &q, &m, &want_ranges, &ctx)?;

    // すべて置換の展開 (走査が最後まで進んだので、すべて置換も作れる)
    {
        let Some((cs, n)) = build_replace_all(&t, &q, REPLACEMENT, &CancelToken::new()) else {
            return Err(format!(
                "走査は最後まで進んだのに、すべて置換を作れない: {}",
                ctx()
            ));
        };
        let got = apply(doc, &cs)?;
        let mut expect = String::new();
        let mut last = 0;
        for r in &want {
            expect.push_str(&doc[last..r.s]);
            expect.push_str(&r.expanded);
            last = r.e;
        }
        expect.push_str(&doc[last..]);
        if got != expect || n != want.len() as u64 {
            return Err(format!(
                "すべて置換が違う: {}\n 実際 {got:?} ({n} 件)\n 期待 {expect:?} ({} 件)",
                ctx(),
                want.len()
            ));
        }
    }

    // 1 件置換: 正本の各一致について、確かめとキャプチャの取り直し (`replace_one`) の結果が、すべて置換の展開と同じ
    // (03 §12.3.1 の「置換」。選び始める位置は、行をまたぐ検索の規則では一致の始端、それ以外は一致の始端を含む行の行頭)
    for r in sample_refs(&want, 8) {
        let from = if m.whole_doc() {
            r.s
        } else {
            t.line_to_byte(t.byte_to_line(ByteOff(r.s))).0
        };
        let got = replace_one(
            &t,
            &m,
            ByteOff(from),
            ByteRange::new(ByteOff(r.s), ByteOff(r.e)),
            REPLACEMENT,
            None,
            &CancelToken::new(),
        );
        match got {
            Ok(Some(x)) if x == r.expanded => {}
            Err(ScanEnd::TooHeavy | ScanEnd::TooLarge) => {}
            other => {
                return Err(format!(
                    "1 件置換 {:?} が違う: {}\n 実際 {other:?}\n 期待 {:?}",
                    (r.s, r.e),
                    ctx(),
                    r.expanded
                ));
            }
        }
    }

    // 2 つ目の参照: CRLF と CR を LF にそろえた文書で、糖衣なしの `\n` (LF だけ) で探す
    if c.ref2 && c.sugar && scope.is_none() {
        let q2 = SearchQuery::literal(&c.pattern)
            .with_regex(true)
            .with_case(c.case)
            .with_newline_sugar(false);
        let p2 = q2
            .analyze()
            .map_err(|e| format!("糖衣なしの解析の誤り {e:?}: {}", ctx()))?
            .and_then(|p| p.regex_pattern().map(str::to_owned));
        if let Some(p2) = p2
            && let Some(re2) = reference_regex(&p2, !q2.is_case_sensitive())
        {
            let (norm, map) = normalize(doc);
            let second: Vec<(usize, usize)> = select(&norm, &re2, c.whole_word)
                .iter()
                .map(|r| (map[r.s], map[r.e]))
                .collect();
            let first: Vec<(usize, usize)> = reference.iter().map(|r| (r.s, r.e)).collect();
            if second != first {
                return Err(format!(
                    "2 つ目の参照と違う: {}\n 糖衣なし {p2:?}\n 1 つ目 {first:?}\n 2 つ目 {second:?}",
                    ctx()
                ));
            }
        }
    }
    Ok(Checked::Compared)
}

/// 全件のジョブ (`run_search`) の結果 (届いた一致, `first_after`, 再開の位置, 打ち切りの理由)。
type JobRun = (
    Vec<(usize, usize)>,
    Option<(usize, usize)>,
    Vec<usize>,
    Option<ScanLimit>,
);

/// 全件のジョブを `from` を起点に走らせる。
fn run_job(t: &Text, q: &SearchQuery, from: usize) -> JobRun {
    let clock = StepClock {
        now: AtomicU64::new(0),
        step: 0,
    };
    let mut out: Vec<JobOutput> = Vec::new();
    run_search(
        SearchJob {
            job: eve_view::JobId(1),
            snapshot: t,
            query: q,
            from: ByteOff(from),
            max_matches: usize::MAX,
            count_limit: u64::MAX,
        },
        &clock,
        &CancelToken::new(),
        &mut |r| out.push(r),
    );
    let (mut got, mut first, mut resume, mut limit) = (Vec::new(), None, Vec::new(), None);
    for r in out {
        match r.body {
            JobResult::SearchBatch {
                matches,
                first_after,
            } => {
                got.extend(matches.iter().map(|r| (r.start.0, r.end.0)));
                if let Some(f) = first_after {
                    first = Some((f.start.0, f.end.0));
                }
            }
            JobResult::SearchDone {
                resume: rs,
                limit: l,
                ..
            } => {
                resume = rs.iter().map(|p| p.0).collect();
                limit = l;
            }
            _ => {}
        }
    }
    (got, first, resume, limit)
}

/// 単発の検索 (`find_next`)。取り消し・打ち切りは `None`。
fn find_one(
    t: &Text,
    q: &SearchQuery,
    from: usize,
    forward: bool,
    resume: &[ByteOff],
) -> Option<Option<((usize, usize), bool)>> {
    find_next(
        &FindJob {
            snapshot: t,
            query: q,
            from: ByteOff(from),
            forward,
            resume,
        },
        None,
        &CancelToken::new(),
    )
    .ok()
    .map(|o| o.map(|(r, w)| ((r.start.0, r.end.0), w)))
}

/// 呼び出し側の検査 (全件のジョブと次 / 前の繰り返し)。
fn check_callers(
    c: &RegexCase,
    t: &Text,
    q: &SearchQuery,
    m: &super::Matcher,
    want: &[(usize, usize)],
    ctx: &dyn Fn() -> String,
) -> Result<(), String> {
    let doc = c.doc.as_str();
    let len = doc.len();
    let cursor = floor_char(doc, c.splits.first().copied().unwrap_or(0) % (len + 1));
    let (got, first, resume, limit) = run_job(t, q, cursor);
    if limit.is_some() {
        return Ok(());
    }
    let mut sorted = got.clone();
    sorted.sort_unstable();
    if sorted != want {
        return Err(format!(
            "全件のジョブの一致が違う: {}\n 実際 {got:?}\n 期待 {want:?}",
            ctx()
        ));
    }
    if !m.whole_doc() {
        return Ok(());
    }
    if got != want {
        return Err(format!(
            "全件のジョブが文書の順に届かない: {}\n 実際 {got:?}",
            ctx()
        ));
    }
    let want_first = want.iter().copied().find(|r| r.0 >= cursor);
    if first != want_first {
        return Err(format!(
            "{cursor} からの first_after が違う: {}\n 実際 {first:?}\n 期待 {want_first:?}",
            ctx()
        ));
    }
    if resume.windows(2).any(|w| w[0] >= w[1]) {
        return Err(format!("再開の位置が昇順でない {resume:?}: {}", ctx()));
    }
    for &from in &sample(&resume, 4) {
        let (end, rest) = scan(t, m, from, None, None);
        let expect: Vec<(usize, usize)> = want.iter().copied().filter(|r| r.0 >= from).collect();
        if end == ScanEnd::Done && rest != expect {
            return Err(format!(
                "ジョブの再開の位置 {from} から走査し直した続きが違う: {}\n 実際 {rest:?}\n 期待 {expect:?}",
                ctx()
            ));
        }
    }
    // 次 / 前を繰り返す: 次は今の一致の終端から (正本の位置として渡す)、前は今の一致の始端から
    let points: Vec<ByteOff> = resume.iter().map(|&p| ByteOff(p)).collect();
    let steps = want.len().min(24);
    let mut from = 0;
    for (i, w) in want.iter().take(steps).enumerate() {
        let mut starts = points.clone();
        starts.push(ByteOff(from));
        let got = find_one(t, q, from, true, &starts);
        if got != Some(Some((*w, false))) {
            return Err(format!(
                "次の一致の {i} 番目 ({from} から) が違う: {}\n 実際 {got:?}\n 期待 {w:?}",
                ctx()
            ));
        }
        from = w.1;
    }
    if steps == want.len() {
        let mut starts = points.clone();
        starts.push(ByteOff(from));
        let got = find_one(t, q, from, true, &starts);
        let expect = want.first().map(|w| (*w, true));
        if got != Some(expect) {
            return Err(format!(
                "末尾 ({from}) の次の一致 (折り返し) が違う: {}\n 実際 {got:?}\n 期待 {expect:?}",
                ctx()
            ));
        }
    }
    let mut from = len;
    for (i, w) in want.iter().rev().take(steps).enumerate() {
        let got = find_one(t, q, from, false, &points);
        if got != Some(Some((*w, false))) {
            return Err(format!(
                "前の一致の {i} 番目 ({from} から) が違う: {}\n 実際 {got:?}\n 期待 {w:?}",
                ctx()
            ));
        }
        from = w.0;
    }
    if steps == want.len() {
        let got = find_one(t, q, from, false, &points);
        let expect = want.last().map(|w| (*w, true));
        if got != Some(expect) {
            return Err(format!(
                "先頭 ({from}) の前の一致 (折り返し) が違う: {}\n 実際 {got:?}\n 期待 {expect:?}",
                ctx()
            ));
        }
    }
    Ok(())
}

/// 走査の入口で `from` から末尾まで走査した (終わり方, 一致)。
fn scan(
    t: &Text,
    m: &super::Matcher,
    from: usize,
    clock: Option<&dyn Clock>,
    resumes: Option<&mut Vec<usize>>,
) -> (ScanEnd, Vec<(usize, usize)>) {
    let cancel = CancelToken::new();
    let mut out = Vec::new();
    let mut record: Vec<usize> = Vec::new();
    let mut on_resume = |p: ByteOff| record.push(p.0);
    let mut ctx = ScanCtx {
        cancel: &cancel,
        clock,
        resume: Some(&mut on_resume),
    };
    let end = scan_range(
        t,
        m,
        ByteRange::new(ByteOff(from), ByteOff(t.len_bytes())),
        &mut ctx,
        &mut |r| {
            out.push((r.start.0, r.end.0));
            true
        },
    );
    if let Some(v) = resumes {
        *v = record;
    }
    (end, out)
}

/// 走査の入口で `[from, to)` を走査した (終わり方, 一致)。
fn scan_part(
    t: &Text,
    m: &super::Matcher,
    from: usize,
    to: usize,
) -> (ScanEnd, Vec<(usize, usize)>) {
    let cancel = CancelToken::new();
    let mut out = Vec::new();
    let end = scan_range(
        t,
        m,
        ByteRange::new(ByteOff(from), ByteOff(to)),
        &mut ScanCtx::new(&cancel),
        &mut |r| {
            out.push((r.start.0, r.end.0));
            true
        },
    );
    (end, out)
}

/// 次の一致: カーソル以前で最も近い再開の位置から走査し、カーソル以降に始まる最初の一致。取り消し・打ち切りは `None`。
fn next_from(
    t: &Text,
    m: &super::Matcher,
    resumes: &[usize],
    cursor: usize,
) -> Option<Option<(usize, usize)>> {
    let from = resumes
        .iter()
        .copied()
        .filter(|&r| r <= cursor)
        .max()
        .unwrap_or(0);
    let (end, v) = scan(t, m, from, None, None);
    (end == ScanEnd::Done).then(|| v.into_iter().find(|r| r.0 >= cursor))
}

/// 前の一致: カーソルより前の再開の位置から走査し、カーソル以前に終わる最後の一致。無ければ 1 つ前の再開の位置から探し直す。
fn prev_from(
    t: &Text,
    m: &super::Matcher,
    resumes: &[usize],
    cursor: usize,
) -> Option<Option<(usize, usize)>> {
    let mut points: Vec<usize> = resumes.iter().copied().filter(|&r| r < cursor).collect();
    points.push(0);
    points.sort_unstable();
    points.dedup();
    for &from in points.iter().rev() {
        let (end, v) = scan(t, m, from, None, None);
        if end != ScanEnd::Done {
            return None;
        }
        if let Some(r) = v.into_iter().rev().find(|r| r.1 <= cursor) {
            return Some(Some(r));
        }
    }
    Some(None)
}

/// 参照の一致の並びから最大 `n` 個を均等に選ぶ。
fn sample_refs<'a>(v: &[&'a RefMatch], n: usize) -> Vec<&'a RefMatch> {
    if v.len() <= n {
        return v.to_vec();
    }
    (0..n).map(|i| v[i * v.len() / n]).collect()
}

/// 並びから最大 `n` 個を均等に選ぶ。
fn sample(v: &[usize], n: usize) -> Vec<usize> {
    if v.len() <= n {
        return v.to_vec();
    }
    (0..n).map(|i| v[i * v.len() / n]).collect()
}

/// 参照の照合器 (走査と同じフラグ。大きさの上限は広げる)。
fn reference_regex(pattern: &str, insensitive: bool) -> Option<regex::Regex> {
    regex::RegexBuilder::new(pattern)
        .case_insensitive(insensitive)
        .multi_line(true)
        .crlf(true)
        .size_limit(1 << 26)
        .build()
        .ok()
}

/// 全文を 1 本の文字列として、03 §12.1 の選び方で選んだ一致 (範囲の内外を問わない正本)。
fn select(doc: &str, re: &regex::Regex, whole_word: bool) -> Vec<RefMatch> {
    select_from(doc, re, whole_word, 0)
}

/// 全文を 1 本の文字列として、`from` から 03 §12.1 の選び方で選び始めた一致。
fn select_from(doc: &str, re: &regex::Regex, whole_word: bool, from: usize) -> Vec<RefMatch> {
    let t = Text::from(doc);
    let next_char = |i: usize| i + doc[i..].chars().next().map_or(1, char::len_utf8);
    let mut out = Vec::new();
    let mut pos = from;
    while pos <= doc.len() {
        let Some(c) = re.captures_at(doc, pos) else {
            break;
        };
        let Some(m) = c.get(0) else {
            break;
        };
        let (s, e) = (m.start(), m.end());
        if s == e {
            pos = next_char(e);
            continue;
        }
        if is_grapheme_boundary(&t, ByteOff(s))
            && is_grapheme_boundary(&t, ByteOff(e))
            && words_ok(doc, s, e, whole_word)
        {
            let mut expanded = String::new();
            c.expand(REPLACEMENT, &mut expanded);
            out.push(RefMatch { s, e, expanded });
            pos = e;
        } else {
            pos = next_char(s);
        }
    }
    out
}

/// 単語単位: 始端はそれを含む行の、終端はそれを含む行の単語境界 (終端が行頭なら境界、03 §12.3.1)。
fn words_ok(doc: &str, s: usize, e: usize, on: bool) -> bool {
    if !on {
        return true;
    }
    let b = doc.as_bytes();
    let line = |x: usize| {
        let ls = b[..x]
            .iter()
            .rposition(|c| matches!(c, b'\r' | b'\n'))
            .map_or(0, |i| i + 1);
        let le = b[x..]
            .iter()
            .position(|c| matches!(c, b'\r' | b'\n'))
            .map_or(b.len(), |i| x + i);
        (ls, le)
    };
    let (ls, le) = line(s);
    if !word_boundaries(&doc[ls..le]).contains(&(s - ls)) {
        return false;
    }
    let line_start = b[e - 1] == b'\n' || (b[e - 1] == b'\r' && b.get(e) != Some(&b'\n'));
    if line_start {
        return true;
    }
    let (ls, le) = line(e);
    word_boundaries(&doc[ls..le]).contains(&(e - ls))
}

/// 選択範囲を char 境界へ丸め、昇順で重ならない形にする (空の範囲は除く)。
fn scope_ranges(doc: &str, raw: Option<&[(usize, usize)]>) -> Option<Vec<ByteRange>> {
    let raw = raw?;
    let n = doc.len() + 1;
    let mut v: Vec<(usize, usize)> = raw
        .iter()
        .map(|&(a, b)| {
            let (a, b) = (floor_char(doc, a % n), floor_char(doc, b % n));
            (a.min(b), a.max(b))
        })
        .filter(|(a, b)| a < b)
        .collect();
    v.sort_unstable();
    let mut out: Vec<ByteRange> = Vec::new();
    for (a, b) in v {
        if out.last().is_some_and(|l| a < l.end.0) {
            continue;
        }
        out.push(ByteRange::new(ByteOff(a), ByteOff(b)));
    }
    Some(out)
}

/// 一致 `[s, e)` が範囲のどれか 1 つに収まる (範囲が無ければ真)。
fn in_scope(scope: Option<&[ByteRange]>, s: usize, e: usize) -> bool {
    scope.is_none_or(|sc| sc.iter().any(|r| r.start.0 <= s && e <= r.end.0))
}

/// `x` 以下で最大の char 境界。
fn floor_char(s: &str, x: usize) -> usize {
    let mut x = x.min(s.len());
    while !s.is_char_boundary(x) {
        x -= 1;
    }
    x
}

/// 文書を `splits` の位置 (char 境界へ丸める) で分け、後ろの部分から順に先頭へ挿入して作った `Text` (読込とは違うチャンクの形)。
// 文書テキストの変更は本来 `Document::apply` の仕事。ここでは検査用の木の形を作るためだけに `Text` へ直接当てる
#[allow(clippy::disallowed_methods)]
fn build_text(doc: &str, splits: &[usize]) -> Text {
    let n = doc.len() + 1;
    let mut cuts: Vec<usize> = splits.iter().map(|&p| floor_char(doc, p % n)).collect();
    cuts.push(doc.len());
    cuts.sort_unstable();
    cuts.dedup();
    let mut pieces = Vec::new();
    let mut prev = 0;
    for c in cuts {
        if c > prev {
            pieces.push(&doc[prev..c]);
            prev = c;
        }
    }
    let mut t = Text::from("");
    for p in pieces.iter().rev() {
        let edit = Edit {
            range: ByteRange::new(ByteOff(0), ByteOff(0)),
            insert: Arc::from(*p),
        };
        let applied = ChangeSet::from_sorted_edits(&t, [edit])
            .ok()
            .and_then(|cs| t.apply(&cs).ok());
        if applied.is_none() {
            return Text::from(doc);
        }
    }
    t
}

/// 文書に `ChangeSet` を当てた文字列。
#[allow(clippy::disallowed_methods)]
fn apply(doc: &str, cs: &ChangeSet) -> Result<String, String> {
    let mut t = Text::from(doc);
    t.apply(cs)
        .map_err(|e| format!("置換を当てられない: {e:?}"))?;
    Ok(t.slice_to_string(ByteRange::new(ByteOff(0), ByteOff(t.len_bytes()))))
}

/// CRLF と CR を LF にそろえた文書と、そろえた文書の位置 → 元の文書の位置 (行の終端は元の終端の先頭、その後ろは元の終端の
/// 後ろへ写す)。
fn normalize(doc: &str) -> (String, Vec<usize>) {
    let b = doc.as_bytes();
    let mut out = String::with_capacity(doc.len());
    let mut map = Vec::with_capacity(doc.len() + 1);
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' {
            map.push(i);
            out.push('\n');
            i += if b.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
            continue;
        }
        let n = doc[i..].chars().next().map_or(1, char::len_utf8);
        for k in 0..n {
            map.push(i + k);
        }
        out.push_str(&doc[i..i + n]);
        i += n;
    }
    map.push(doc.len());
    (out, map)
}
