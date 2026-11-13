use eve_text::{ByteOff, ByteRange, Text, TextRead};
use eve_view::JobId;

use super::*;
use super::{Prepared, replace};
use crate::jobs::cancel::CancelToken;
use crate::jobs::{JobError, JobOutput, JobResult};
use crate::time::{Clock, Tick};

struct Fixed;
impl Clock for Fixed {
    fn now(&self) -> Tick {
        Tick(0)
    }
}

fn find_all(doc: &str, q: &SearchQuery) -> Vec<(usize, usize)> {
    let t = Text::from(doc);
    let Some(m) = q.matcher() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    scan_lines(
        &t,
        &m,
        0..t.len_lines(),
        &CancelToken::new(),
        false,
        &mut |r, _| {
            out.push((r.start.0, r.end.0));
            true
        },
    );
    out
}

fn lit(p: &str) -> SearchQuery {
    SearchQuery::literal(p)
}

#[test]
fn 一致は重ならないものを左から取る() {
    assert_eq!(find_all("aaa", &lit("aa")), [(0, 2)]);
    assert_eq!(find_all("abab\nab", &lit("ab")), [(0, 2), (2, 4), (5, 7)]);
}

#[test]
fn コンパイルの結果は複製と共有し_条件を変えると作り直す() {
    let q = lit("abc").with_regex(true).with_case(Case::Sensitive);
    let m = q.matcher().unwrap();
    // 複製 (検索ジョブに渡すもの) は同じ照合器を使う
    assert!(std::sync::Arc::ptr_eq(&m, &q.clone().matcher().unwrap()));
    assert!(std::sync::Arc::ptr_eq(&m, &q.matcher().unwrap()));
    // 条件を変えた問い合わせは作り直す。比べるのは条件だけ
    let insensitive = q.clone().with_case(Case::Insensitive);
    assert!(!std::sync::Arc::ptr_eq(&m, &insensitive.matcher().unwrap()));
    assert_eq!(q, lit("abc").with_regex(true).with_case(Case::Sensitive));
    assert_ne!(q, insensitive);
    assert_eq!(find_all("ABC abc", &insensitive), [(0, 3), (4, 7)]);
    assert_eq!(find_all("ABC abc", &q), [(4, 7)]);
    // 誤りも覚える
    let bad = lit("(").with_regex(true);
    assert!(bad.compile().is_err() && bad.clone().compile().is_err());
}

#[test]
fn 空の検索語は何にも一致しない() {
    assert!(lit("").matcher().is_none());
    let actual = find_all("abc", &lit(""));
    assert!(actual.is_empty(), "{actual:?}");
}

#[test]
fn smart_は大文字を含めば区別し_含まなければ区別しない() {
    assert_eq!(find_all("Foo foo FOO", &lit("foo")).len(), 3);
    assert_eq!(find_all("Foo foo FOO", &lit("Foo")), [(0, 3)]);
}

#[test]
fn sensitive_と_insensitive() {
    let q = lit("foo").with_case(Case::Sensitive);
    assert_eq!(find_all("Foo foo FOO", &q), [(4, 7)]);
    let q = lit("Foo").with_case(Case::Insensitive);
    assert_eq!(find_all("Foo foo FOO", &q).len(), 3);
}

#[test]
fn 日本語と全角英字() {
    assert_eq!(find_all("日本語の日本", &lit("日本")), [(0, 6), (12, 18)]);
    // 全角英字は大小の区別がある文字。半角とは同一視しない
    assert_eq!(find_all("Ａａ", &lit("ａ")).len(), 2);
    let actual = find_all("Ａａ", &lit("a"));
    assert!(actual.is_empty(), "{actual:?}");
}

#[test]
fn 畳み込みで長さが変わる文字を含む行でも位置は元の文書のもの() {
    // KELVIN SIGN (3 バイト) は k (1 バイト) に畳み込まれる
    let doc = "\u{212A}-xyz";
    assert_eq!(find_all(doc, &lit("xyz")), [(4, 7)]);
    assert_eq!(find_all(doc, &lit("k")), [(0, 3)]);
}

#[test]
fn 書記素の途中に当たった候補は捨てる() {
    // e + COMBINING ACUTE: `e` だけには一致しない
    let actual = find_all("e\u{301}", &lit("e"));
    assert!(actual.is_empty(), "{actual:?}");
    assert_eq!(find_all("e\u{301}e", &lit("e")), [(3, 4)]);
    // 絵文字の ZWJ 列の先頭
    let family = "👨\u{200D}👩\u{200D}👧";
    let actual = find_all(family, &lit("👨"));
    assert!(actual.is_empty(), "{actual:?}");
}

// verifies: AC-FR-404-3
#[test]
fn 取り消すと止まる() {
    let t = Text::from("a\na\na\n");
    let m = lit("a").matcher().unwrap();
    let c = CancelToken::new();
    c.cancel();
    assert!(!scan_lines(
        &t,
        &m,
        0..t.len_lines(),
        &c,
        false,
        &mut |_, _| true
    ));
}

fn run(doc: &str, q: &SearchQuery, from: usize, max: usize, limit: u64) -> Vec<JobOutput> {
    let t = Text::from(doc);
    let mut out = Vec::new();
    run_search(
        SearchJob {
            job: JobId(1),
            snapshot: &t,
            query: q,
            from: ByteOff(from),
            max_matches: max,
            count_limit: limit,
        },
        &Fixed,
        &CancelToken::new(),
        &mut |r| out.push(r),
    );
    out
}

fn collect(rs: &[JobOutput]) -> (Vec<usize>, Option<(u64, bool, bool)>) {
    let mut starts = Vec::new();
    let mut done = None;
    for r in rs {
        match &r.body {
            JobResult::SearchBatch { matches, .. } => {
                starts.extend(matches.iter().map(|m| m.start.0));
            }
            JobResult::SearchDone {
                total,
                count_limited,
                truncated,
                ..
            } => done = Some((*total, *count_limited, *truncated)),
            _ => {}
        }
    }
    (starts, done)
}

#[test]
fn ジョブはカーソルの行から末尾まで_続けて先頭からを走査する() {
    let rs = run("a\nb\na\nb\na\n", &lit("a"), 4, 100, 1000);
    let (starts, done) = collect(&rs);
    assert_eq!(starts, [4, 8, 0]);
    assert_eq!(done, Some((3, false, false)));
}

#[test]
fn 保持の上限を超えても数え続ける() {
    let doc = "a\n".repeat(10);
    let (starts, done) = collect(&run(&doc, &lit("a"), 0, 3, 1000));
    assert_eq!(starts.len(), 3);
    assert_eq!(done, Some((10, false, true)));
}

#[test]
fn 数える上限で打ち切る() {
    let doc = "a\n".repeat(10);
    let (_, done) = collect(&run(&doc, &lit("a"), 0, 100, 4));
    assert_eq!(done, Some((4, true, false)));
}

#[test]
fn 一致が多いとバッチに分ける() {
    let doc = "a\n".repeat(600);
    let rs = run(&doc, &lit("a"), 0, 10_000, 1_000_000);
    let batches = rs
        .iter()
        .filter(|r| matches!(r.body, JobResult::SearchBatch { .. }))
        .count();
    assert_eq!(batches, 4, "最初の 1 件 + 256 + 256 + 87");
}

#[test]
fn 取り消されたジョブは_cancelled_で終わる() {
    let t = Text::from("a\n");
    let c = CancelToken::new();
    c.cancel();
    let mut out = Vec::new();
    run_search(
        SearchJob {
            job: JobId(1),
            snapshot: &t,
            query: &lit("a"),
            from: ByteOff(0),
            max_matches: 10,
            count_limit: 10,
        },
        &Fixed,
        &c,
        &mut |r| out.push(r),
    );
    assert!(matches!(
        out[0].body,
        JobResult::Failed {
            err: JobError::Cancelled,
            ..
        }
    ));
}

#[test]
fn 最初の一致はすぐ送る() {
    let doc = "a\n".repeat(10);
    let rs = run(&doc, &lit("a"), 0, 100, 1000);
    let first = rs
        .iter()
        .position(|r| matches!(r.body, JobResult::SearchBatch { .. }))
        .unwrap();
    let JobResult::SearchBatch { matches, .. } = &rs[first].body else {
        unreachable!()
    };
    assert_eq!(matches.len(), 1);
}

fn find(
    t: &Text,
    q: &SearchQuery,
    from: usize,
    forward: bool,
    cancel: &CancelToken,
) -> Result<Option<(ByteRange, bool)>, ScanEnd> {
    find_next(
        &FindJob {
            snapshot: t,
            query: q,
            from: ByteOff(from),
            forward,
            resume: &[],
        },
        None,
        cancel,
    )
}

/// 正規表現 (行の窓の核は `core` バイト)。
fn multi(p: &str, core: usize) -> SearchQuery {
    SearchQuery::literal(p)
        .with_regex(true)
        .with_case(Case::Sensitive)
        .tuned(SearchTuning {
            core_bytes: core,
            ..SearchTuning::default()
        })
}

/// 全件のジョブの (届いた一致の始端の並び, `first_after` の始端, 再開の位置, 打ち切りの理由, 総数)。
#[allow(clippy::type_complexity)]
fn run_whole(
    doc: &str,
    q: &SearchQuery,
    from: usize,
) -> (
    Vec<usize>,
    Option<usize>,
    Vec<usize>,
    Option<ScanLimit>,
    u64,
) {
    let rs = run(doc, q, from, 100, 1000);
    let (mut starts, mut first, mut resume, mut limit, mut total) =
        (Vec::new(), None, Vec::new(), None, 0);
    for r in &rs {
        match &r.body {
            JobResult::SearchBatch {
                matches,
                first_after,
            } => {
                starts.extend(matches.iter().map(|m| m.start.0));
                if let Some(f) = first_after {
                    assert!(first.is_none(), "first_after は 1 回だけ");
                    first = Some(f.start.0);
                }
            }
            JobResult::SearchDone {
                resume: rv,
                limit: l,
                total: n,
                ..
            } => {
                resume = rv.iter().map(|p| p.0).collect();
                limit = *l;
                total = *n;
            }
            _ => {}
        }
    }
    (starts, first, resume, limit, total)
}

#[test]
fn 行をまたぐ規則の全件ジョブは文書の先頭から走査し_first_after_と再開の位置を返す() {
    // `a\na` を `a\na\na\na\na` で探す: 先頭からは [0,3) と [4,7) (カーソルの行 (2) から探すと [2,5) と [6,9))
    let q = multi(r"a\na", 2);
    let (starts, first, resume, limit, total) = run_whole("a\na\na\na\na", &q, 2);
    assert_eq!(starts, [0, 4], "文書の順に届く");
    assert_eq!(first, Some(4), "起点 2 以降の最初の一致");
    assert_eq!((limit, total), (None, 2));
    assert_eq!(resume.first(), Some(&0));
    assert!(
        resume.windows(2).all(|w| w[0] + 2 <= w[1]),
        "核の大きさごとに間引く: {resume:?}"
    );
    // 起点以降に一致が無ければ first_after は送らない
    let (_, first, ..) = run_whole("a\na\nb", &q, 3);
    assert_eq!(first, None);
    // 行をまたがない検索語は今までどおり、カーソルの行から走査し、再開の位置を返さない
    let (starts, first, resume, ..) = run_whole("ab\nab\nab", &lit("ab"), 3);
    assert_eq!(starts, [3, 6, 0]);
    assert_eq!((first, resume.len()), (None, 0));
}

#[test]
fn first_after_は保持の上限に依らず送る() {
    let q = multi(r"a\na", 1 << 20);
    let doc = "a\na\n".repeat(10);
    let rs = run(&doc, &q, 30, 2, 1000);
    let firsts: Vec<usize> = rs
        .iter()
        .filter_map(|r| match &r.body {
            JobResult::SearchBatch {
                first_after: Some(f),
                ..
            } => Some(f.start.0),
            _ => None,
        })
        .collect();
    assert_eq!(
        firsts,
        [32],
        "起点 30 以降の最初の一致 (正本の始端は 4 の倍数)"
    );
    let (_, done) = collect(&rs);
    assert_eq!(done, Some((10, false, true)));
}

#[test]
fn 走査の途中で探さないと分かったら_理由つきの_0_件で終わる() {
    // k = 1 で最大の長さの無い検索語は、核を行の境界に揃える。長い行で窓が上限を超える
    let q = SearchQuery::literal(r"a\n[^\n]*")
        .with_regex(true)
        .tuned(SearchTuning {
            core_bytes: 4,
            concat_max: 32,
        });
    let doc = format!("a\nb\na\nb\n{}\n", "c".repeat(100));
    let (_, _, resume, limit, total) = run_whole(&doc, &q, 0);
    assert_eq!(
        (limit, total, resume.len()),
        (Some(ScanLimit::TooLarge), 0, 0)
    );
}

#[test]
fn 行をまたぐ規則の単発の検索は正本の位置から探す() {
    // 正本は [0,3) と [4,7)。起点 2 から次を探すと [4,7) (行頭 2 から選ぶと [2,5))
    let t = Text::from("a\na\na\na");
    let q = multi(r"a\na", 2);
    let f = |from: usize, forward: bool, resume: &[ByteOff]| {
        find_next(
            &FindJob {
                snapshot: &t,
                query: &q,
                from: ByteOff(from),
                forward,
                resume,
            },
            Some(&Fixed),
            &CancelToken::new(),
        )
        .map(|o| o.map(|(r, w)| (r.start.0, r.end.0, w)))
    };
    assert_eq!(f(2, true, &[]), Ok(Some((4, 7, false))));
    assert_eq!(f(3, true, &[ByteOff(3)]), Ok(Some((4, 7, false))));
    assert_eq!(f(7, true, &[ByteOff(7)]), Ok(Some((0, 3, true))));
    assert_eq!(f(4, false, &[]), Ok(Some((0, 3, false))));
    assert_eq!(f(0, false, &[]), Ok(Some((4, 7, true))));
    // 探さない検索語は理由を返す
    let big = multi(r"a\n+", 2).tuned(SearchTuning {
        core_bytes: 2,
        concat_max: 4,
    });
    let r = find_next(
        &FindJob {
            snapshot: &t,
            query: &big,
            from: ByteOff(0),
            forward: true,
            resume: &[],
        },
        Some(&Fixed),
        &CancelToken::new(),
    );
    assert_eq!(r, Err(ScanEnd::TooLarge));
}

#[test]
fn 一件置換は対象の始端から走査の入口で選んだ最初の一致で確かめる() {
    let t = Text::from("ab\r\ncd\r\nef");
    let q = multi(r"(\w)\n(\w)", 4);
    let m = q.matcher().unwrap();
    let r = |s: usize, e: usize| ByteRange::new(ByteOff(s), ByteOff(e));
    let one = |s: usize, e: usize| {
        replace_one(
            &t,
            &m,
            ByteOff(s),
            r(s, e),
            "$2$1",
            None,
            &CancelToken::new(),
        )
    };
    assert_eq!(one(1, 5), Ok(Some("cb".to_owned())));
    // 一致でない範囲 (同じ始端から選んだ一致と違う) は対象にしない
    assert_eq!(one(1, 4), Ok(None));
    assert_eq!(one(0, 5), Ok(None));
    // 窓の端で `\b` が偽って成り立たない (正本は `a\n` の [0,2))
    let t = Text::from("a\nbc");
    let q = multi(r"a\nb\b|a\n", 4);
    let m = q.matcher().unwrap();
    assert_eq!(
        replace_one(&t, &m, ByteOff(0), r(0, 2), "X", None, &CancelToken::new()),
        Ok(Some("X".to_owned()))
    );
}

fn next(doc: &str, p: &str, from: usize, forward: bool) -> Option<Option<(usize, bool)>> {
    let t = Text::from(doc);
    find(&t, &lit(p), from, forward, &CancelToken::new())
        .ok()
        .map(|o| o.map(|(r, w)| (r.start.0, w)))
}

#[test]
fn 単発の検索_先へ_前へ_折り返し() {
    let doc = "ab\nxx\nab\nxx\nab";
    assert_eq!(next(doc, "ab", 0, true), Some(Some((0, false))));
    assert_eq!(next(doc, "ab", 1, true), Some(Some((6, false))));
    assert_eq!(
        next(doc, "ab", 13, true),
        Some(Some((0, true))),
        "末尾を越えて先頭へ"
    );
    assert_eq!(next(doc, "ab", 14, false), Some(Some((12, false))));
    assert_eq!(next(doc, "ab", 2, false), Some(Some((0, false))));
    assert_eq!(
        next(doc, "ab", 0, false),
        Some(Some((12, true))),
        "先頭を越えて末尾へ"
    );
    assert_eq!(next(doc, "zz", 0, true), Some(None));
    assert_eq!(next(doc, "", 0, false), Some(None));
}

#[test]
fn 単発の検索は取り消せる() {
    let t = Text::from("ab\n".repeat(1000).as_str());
    let c = CancelToken::new();
    c.cancel();
    assert_eq!(find(&t, &lit("zz"), 0, true, &c), Err(ScanEnd::Cancelled));
    assert_eq!(find(&t, &lit("zz"), 0, false, &c), Err(ScanEnd::Cancelled));
}

// ---- 大小を区別しない検索の経路 (03 §12.4、OQ-196): どの経路でも Unicode の単純ケースフォールディングの意味 ----

/// 文書全体を走査の入口 (`scan_bytes`) で探した一致。
fn scan_all(doc: &str, q: &SearchQuery) -> Vec<(usize, usize)> {
    let t = Text::from(doc);
    let Some(m) = q.matcher() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let full = eve_text::ByteRange::new(ByteOff(0), ByteOff(t.len_bytes()));
    scan_bytes(&t, &m, full, &CancelToken::new(), &mut |r| {
        out.push((r.start.0, r.end.0));
        true
    });
    out
}

/// 検索語 `p` をエスケープした正規表現の問い合わせ (大小の扱いと単語単位は `q` と同じ)。
fn as_regex(p: &str, q: &SearchQuery) -> SearchQuery {
    lit(&regex::escape(p))
        .with_regex(true)
        .with_case(q.case())
        .with_whole_word(q.is_whole_word())
}

#[test]
fn 大小を区別しない検索で_s_と_long_s_と_k_とケルビン記号が互いに一致する() {
    // s=0, S=2, ſ=4..6, k=7, K=9, K (U+212A)=11..14
    let doc = "s S \u{17F} k K \u{212A}";
    let s_all = [(0, 1), (2, 3), (4, 6)];
    let k_all = [(7, 8), (9, 10), (11, 14)];
    for p in ["s", "S", "\u{17F}"] {
        let q = lit(p).with_case(Case::Insensitive);
        assert_eq!(scan_all(doc, &q), s_all, "{p:?}");
        assert_eq!(find_all(doc, &q), s_all, "{p:?}");
        assert_eq!(scan_all(doc, &as_regex(p, &q)), s_all, "正規表現 {p:?}");
    }
    for p in ["k", "K", "\u{212A}"] {
        let q = lit(p).with_case(Case::Insensitive);
        assert_eq!(scan_all(doc, &q), k_all, "{p:?}");
        assert_eq!(find_all(doc, &q), k_all, "{p:?}");
        assert_eq!(scan_all(doc, &as_regex(p, &q)), k_all, "正規表現 {p:?}");
    }
    // Smart: 小文字だけ (ſ は小文字) なら区別せず、大文字 (K も大文字) を含めば区別する
    assert_eq!(scan_all(doc, &lit("s")), s_all);
    assert_eq!(scan_all(doc, &lit("\u{17F}")), s_all);
    assert_eq!(scan_all(doc, &lit("k")), k_all);
    assert_eq!(scan_all(doc, &lit("S")), [(2, 3)]);
    assert_eq!(scan_all(doc, &lit("\u{212A}")), [(11, 14)]);
    // 単語単位でも同じ
    let w = |p: &str| lit(p).with_whole_word(true);
    assert_eq!(scan_all(doc, &w("s")), s_all);
    assert_eq!(scan_all(doc, &w("k")), k_all);
    // 語の途中の ſ / K
    let doc = "class CLASS cla\u{17F}s \u{212A}ICK kick";
    let q = lit("class");
    assert_eq!(scan_all(doc, &q), [(0, 5), (6, 11), (12, 18)]);
    assert_eq!(scan_all(doc, &as_regex("class", &q)), scan_all(doc, &q));
    let q = lit("KICK").with_case(Case::Insensitive);
    assert_eq!(scan_all(doc, &q), [(19, 25), (26, 30)]);
}

#[test]
fn ascii_の近道は_k_と_s_を含まない検索語だけで_それ以外は行ごとの経路() {
    let chunked = |q: SearchQuery| q.matcher().unwrap().is_chunked();
    assert!(chunked(lit("ZEBRA").with_case(Case::Insensitive)));
    assert!(chunked(lit("zebra")));
    assert!(chunked(lit("Class").with_case(Case::Sensitive)));
    assert!(chunked(lit("日本語")), "区別の無い文字だけ");
    for p in ["s", "S", "k", "K", "class", "QUICK"] {
        assert!(!chunked(lit(p).with_case(Case::Insensitive)), "{p}");
    }
    assert!(!chunked(lit("ab").with_whole_word(true)));
}

#[test]
fn ascii_以外から_ascii_の英字へ畳み込まれる文字はケルビン記号と_long_s_だけ() {
    // §12.4 の 2 の経路 (ASCII の大小だけを畳み込む) の前提。区別の無い文字は畳み込まれない (下の試験) ので、
    // 区別のある文字だけを調べればよい
    let hay: String = ('\u{80}'..=char::MAX)
        .filter(|&c| matcher::is_cased(c))
        .collect();
    let re = regex::Regex::new("(?i)[a-z]").unwrap();
    let found: Vec<char> = re
        .find_iter(&hay)
        .filter_map(|m| m.as_str().chars().next())
        .collect();
    assert_eq!(found, ['\u{17F}', '\u{212A}']);
}

#[test]
fn 区別の無い文字はどれも大小の畳み込みで区別のある文字に一致しない() {
    // §12.4 の 1 の経路 (区別の無い文字だけの検索語はそのまま memmem) の前提。区別の無い文字の集合を範囲の
    // 文字クラスにして `(?i)` で広げても、区別のある文字には一致しない
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for c in ('\0'..=char::MAX).filter(|&c| !matcher::is_cased(c)) {
        let c = u32::from(c);
        match ranges.last_mut() {
            Some((_, end)) if *end + 1 == c => *end = c,
            _ => ranges.push((c, c)),
        }
    }
    let mut class = String::from("(?i)[");
    for (s, e) in ranges {
        class.push_str(&format!(r"\x{{{s:X}}}-\x{{{e:X}}}"));
    }
    class.push(']');
    let re = regex::RegexBuilder::new(&class)
        .size_limit(1 << 30)
        .build()
        .unwrap();
    let cased: String = ('\0'..=char::MAX)
        .filter(|&c| matcher::is_cased(c))
        .collect();
    let found: Vec<&str> = re.find_iter(&cased).map(|m| m.as_str()).collect();
    assert!(found.is_empty(), "{found:?}");
}

mod 経路の性質 {
    use proptest::prelude::*;

    use super::*;

    /// 検索語の文字: 大小の対応が入り組んだ文字 (`ſ` と `s`、K と `k`、σ / ς / Σ、µ / μ、タイトルケースの ǅ) と、
    /// ASCII の経路に乗る文字・区別の無い文字。
    const PATTERN: &[char] = &[
        's', 'S', '\u{17F}', 'k', 'K', '\u{212A}', 'a', 'A', 'σ', 'ς', 'Σ', '\u{B5}', 'ǅ', 'é', '-',
    ];

    /// 文書の文字: 検索語の文字に、大小の相手 (ß / ẞ、μ / Μ、ǆ / Ǆ、É) と、結合文字・単語の区切り・改行を足したもの。
    const DOC: &[char] = &[
        's', 'S', '\u{17F}', 'k', 'K', '\u{212A}', 'a', 'A', 'é', 'É', 'σ', 'ς', 'Σ', 'ß',
        '\u{1E9E}', '\u{B5}', 'μ', 'Μ', 'ǅ', 'ǆ', 'Ǆ', 'あ', '\u{301}', ' ', '-', '.', '\n',
    ];

    fn text_of(
        chars: &'static [char],
        len: std::ops::Range<usize>,
    ) -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(chars), len)
            .prop_map(|v| v.into_iter().collect())
    }

    proptest! {
        // リテラル (チャンクごと・行ごと) と、エスケープした正規表現の結果は同じ (大小の扱い・単語単位を問わない)
        #[test]
        fn リテラルの経路と正規表現の経路の結果は同じ(
            doc in text_of(DOC, 0..40),
            p in text_of(PATTERN, 1..4),
            case in prop::sample::select(vec![Case::Smart, Case::Sensitive, Case::Insensitive]),
            whole_word: bool,
        ) {
            let q = lit(&p).with_case(case).with_whole_word(whole_word);
            let want = scan_all(&doc, &as_regex(&p, &q));
            prop_assert_eq!(&scan_all(&doc, &q), &want);
            prop_assert_eq!(&find_all(&doc, &q), &want);
        }
    }
}

// ---- 照合器の 2 段 (03 §12.3.1 の「メインスレッドでのコンパイル」) ----

#[test]
fn 正規表現の照合器はメインスレッドで作らず_memmem_の照合器は作る() {
    let q = lit("a+").with_regex(true);
    assert!(q.analyze().unwrap().is_some(), "解析はメインスレッドで行う");
    assert!(matches!(q.prepared(), Prepared::Pending));
    // ジョブ (複製) がコンパイルすると、覚えを共有する元の問い合わせからも使える
    assert!(q.clone().compile().unwrap().is_some());
    assert!(matches!(q.prepared(), Prepared::Ready(_)));
    assert!(matches!(lit("abc").prepared(), Prepared::Ready(_)));
    assert!(
        matches!(lit("class").prepared(), Prepared::Pending),
        "§12.4 の 3 はエスケープした正規表現"
    );
    assert!(matches!(
        lit("(").with_regex(true).prepared(),
        Prepared::Nothing
    ));
    assert!(matches!(lit("").prepared(), Prepared::Nothing));
    // 行をまたぐ検索語も誤りではなく、照合器は同じく `bg` のジョブが作る (v0.3 の「未対応」は M24 の (4) で無くなった)
    let nl = lit(r"a\nb").with_regex(true);
    assert_eq!(nl.known_error(), None);
    assert!(matches!(nl.prepared(), Prepared::Pending));
    assert!(
        nl.analyze().unwrap().is_some(),
        "解析の結果は返す (k と書き換え)"
    );
}

#[test]
fn 置換は走査の入口を通り_範囲と単語単位と選択範囲内に従う() {
    // 置換の候補の走査 (`scan_replacements`) は検索と同じ入口 (`scan_bytes`) を通る (03 §12.3 の「走査の境界」)
    let t = Text::from("ab ab\nxab ab");
    let collect = |q: SearchQuery| {
        let m = q.matcher().unwrap();
        let mut got = Vec::new();
        let full = eve_text::ByteRange::new(ByteOff(0), ByteOff(t.len_bytes()));
        let cancel = CancelToken::new();
        replace::scan_replacements(
            &t,
            &m,
            full,
            &mut ScanCtx::new(&cancel),
            "[$0]",
            &mut |r, s| {
                got.push((r.start.0, s));
                true
            },
        );
        got
    };
    let mut want = Vec::new();
    scan_bytes(
        &t,
        &lit("ab").matcher().unwrap(),
        eve_text::ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())),
        &CancelToken::new(),
        &mut |r| {
            want.push((r.start.0, "[$0]".to_owned()));
            true
        },
    );
    assert_eq!(collect(lit("ab")), want, "リテラルは展開しない");
    let starts = |v: Vec<(usize, String)>| v.into_iter().map(|(s, _)| s).collect::<Vec<_>>();
    assert_eq!(
        collect(lit("ab").with_regex(true)),
        [
            (0, "[ab]".to_owned()),
            (3, "[ab]".to_owned()),
            (7, "[ab]".to_owned()),
            (10, "[ab]".to_owned())
        ]
    );
    assert_eq!(starts(collect(lit("ab").with_whole_word(true))), [0, 3, 10]);
    let scope: std::sync::Arc<[eve_text::ByteRange]> =
        std::sync::Arc::from([eve_text::ByteRange::new(ByteOff(2), ByteOff(9))]);
    assert_eq!(
        starts(collect(lit("ab").with_regex(true).with_scope(Some(scope)))),
        [3, 7]
    );
}
