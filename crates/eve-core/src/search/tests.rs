use super::*;
use crate::time::Tick;
use eve_text::Text;

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
    scan_lines(&t, &m, 0..t.len_lines(), &CancelToken::new(), &mut |r| {
        out.push((r.start.0, r.end.0));
        true
    });
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
fn 空の検索語は何にも一致しない() {
    assert!(lit("").matcher().is_none());
    assert!(find_all("abc", &lit("")).is_empty());
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
    assert!(find_all("Ａａ", &lit("a")).is_empty());
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
    assert!(find_all("e\u{301}", &lit("e")).is_empty());
    assert_eq!(find_all("e\u{301}e", &lit("e")), [(3, 4)]);
    // 絵文字の ZWJ 列の先頭
    let family = "👨\u{200D}👩\u{200D}👧";
    assert!(find_all(family, &lit("👨")).is_empty());
}

#[test]
fn 取り消すと止まる() {
    let t = Text::from("a\na\na\n");
    let m = lit("a").matcher().unwrap();
    let c = CancelToken::new();
    c.cancel();
    assert!(!scan_lines(&t, &m, 0..t.len_lines(), &c, &mut |_| true));
}

fn run(doc: &str, q: &SearchQuery, from: usize, max: usize, limit: u64) -> Vec<JobResult> {
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

fn collect(rs: &[JobResult]) -> (Vec<usize>, Option<(u64, bool, bool)>) {
    let mut starts = Vec::new();
    let mut done = None;
    for r in rs {
        match r {
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
        .filter(|r| matches!(r, JobResult::SearchBatch { .. }))
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
        out[0],
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
        .position(|r| matches!(r, JobResult::SearchBatch { .. }))
        .unwrap();
    let JobResult::SearchBatch { matches, .. } = &rs[first] else {
        unreachable!()
    };
    assert_eq!(matches.len(), 1);
}

fn next(doc: &str, p: &str, from: usize, forward: bool) -> Option<Option<(usize, bool)>> {
    let t = Text::from(doc);
    find_next(&t, &lit(p), ByteOff(from), forward, &CancelToken::new())
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
    assert!(find_next(&t, &lit("zz"), ByteOff(0), true, &c).is_none());
    assert!(find_next(&t, &lit("zz"), ByteOff(0), false, &c).is_none());
}
