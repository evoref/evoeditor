//! 行の窓と全文の連結 (docs/03 §12.3.1 の「実行」・「窓から窓への引き継ぎ」・「最悪の入力の上限」、08 §2.5 の M24 の (2))。
//!
//! 行をまたぐ検索は M24 の (4) まで利用者に開かないので、ここでは eve-core の内部の設定 (`SearchInternal`) で開いて確かめる。
//! 参照との突き合わせは `fuzz_regex_search.rs` (09 §3.3) が受け持ち、ここは固定の例と、打ち切り・探さない場合の理由を見る。

use std::sync::atomic::{AtomicU64, Ordering};

use eve_core::{
    CancelToken, Case, Clock, LineSpan, QueryError, ScanCtx, ScanEnd, SearchInternal, SearchQuery,
    Tick, build_replace_all, scan_range,
};
use eve_text::{ByteOff, ByteRange, Text, TextRead};

fn query(p: &str) -> SearchQuery {
    SearchQuery::literal(p)
        .with_regex(true)
        .with_case(Case::Sensitive)
}

fn open(p: &str, core: usize) -> SearchQuery {
    query(p).with_internal(SearchInternal {
        core_bytes: core,
        ..SearchInternal::multiline()
    })
}

/// `from` から末尾まで走査した (終わり方, 一致)。
fn scan_with(
    t: &Text,
    q: &SearchQuery,
    from: usize,
    clock: Option<&dyn Clock>,
) -> (ScanEnd, Vec<(usize, usize)>) {
    let m = q.matcher().expect("照合器");
    let cancel = CancelToken::new();
    let mut ctx = ScanCtx {
        cancel: &cancel,
        clock,
        resume: None,
    };
    let mut out = Vec::new();
    let end = scan_range(
        t,
        &m,
        ByteRange::new(ByteOff(from), ByteOff(t.len_bytes())),
        &mut ctx,
        &mut |r| {
            out.push((r.start.0, r.end.0));
            true
        },
    );
    (end, out)
}

fn found(q: &SearchQuery, doc: &str) -> Vec<(usize, usize)> {
    let (end, v) = scan_with(&Text::from(doc), q, 0, None);
    assert_eq!(end, ScanEnd::Done, "{q:?} / {doc:?}");
    v
}

/// 読み取りごとに決まった時間だけ進む時計。
struct StepClock(AtomicU64, u64);

impl Clock for StepClock {
    fn now(&self) -> Tick {
        Tick(self.0.fetch_add(self.1, Ordering::Relaxed))
    }
}

#[test]
fn 既定では行をまたぐ検索を開かない() {
    // 行の終端を明示して行をまたぐ検索語は「未対応」のまま (M24 の (4) まで)
    assert_eq!(query(r"a\na").compile().err(), Some(QueryError::Multiline));
    // `(?s)` の `.` は 1 行の中、`\A` は各行の先頭 (v0.3 と同じ)
    assert_eq!(found(&query(r"(?s)a.b"), "a\nb axb"), [(4, 7)]);
    assert_eq!(found(&query(r"\Aa"), "a\na"), [(0, 1), (2, 3)]);
    let m = query(r"(?s)a.b").matcher().unwrap();
    assert!(!m.is_stream());
    // 内部の設定で開くと、文書全体の意味になる
    assert_eq!(found(&open(r"a\na", 1 << 20), "a\na\na"), [(0, 3)]);
    assert_eq!(
        found(&open(r"(?s)a.b", 1 << 20), "a\nb axb"),
        [(0, 3), (4, 7)]
    );
    assert_eq!(found(&open(r"\Aa", 1 << 20), "a\na"), [(0, 1)]);
    assert_eq!(found(&open(r"a\z", 1 << 20), "a\na"), [(2, 3)]);
    assert_eq!(found(&open(r"(?-m)^a", 1 << 20), "a\na"), [(0, 1)]);
    assert!(open(r"\Aa", 1).matcher().unwrap().is_stream());
    // k が 0 で文書の先頭 / 末尾を使わない検索語は、行の経路のまま
    assert!(!open(r"a$", 1).matcher().unwrap().is_stream());
}

#[test]
fn 核をまたぐ一致と_crlf() {
    for core in [1, 2, 3, 4, 7, 1 << 20] {
        // `a\na` を `a\na\na` で探すと、先頭からは 1 件 (03 §12.3.1 の「一致の集合」の例)
        assert_eq!(
            found(&open(r"a\na", core), "a\na\na"),
            [(0, 3)],
            "核 {core}"
        );
        // CRLF は 1 つの行の終端 (`\n\n` は `a\r\nb` で 0 件、`a\r\n\r\nb` で 1 件)
        assert_eq!(found(&open(r"\n\n", core), "a\r\nb"), [], "核 {core}");
        assert_eq!(
            found(&open(r"\n\n", core), "a\r\n\r\nb"),
            [(1, 5)],
            "核 {core}"
        );
        assert_eq!(
            found(&open(r"\n{2,}", core), "a\r\n\r\n\r\nb\n\nc"),
            [(1, 7), (8, 10)],
            "核 {core}"
        );
        // 上限なし (全文の連結)
        assert_eq!(
            found(&open(r"(?s)a.*b", core), "xa\r\n\r\nby b"),
            [(1, 10)],
            "核 {core}"
        );
        assert_eq!(
            found(&open(r"[\s\n]+", core), "a \r\n\tb\r"),
            [(1, 5), (6, 7)],
            "核 {core}"
        );
        // 書記素の境界: CRLF の LF だけには当たらない (糖衣なしの `\n` は LF だけ)
        assert_eq!(
            found(&open(r"\n", core).with_newline_sugar(false), "a\r\nb\nc"),
            [(4, 5)],
            "核 {core}"
        );
    }
}

#[test]
fn 選び始める位置で結果が変わる_入口の約束() {
    let t = Text::from("a\na\na");
    let q = open(r"a\na", 1);
    assert_eq!(scan_with(&t, &q, 0, None).1, [(0, 3)]);
    // 2 行目の行頭から選び始めると `[2, 5)` (正本とは違う。始端は正本の一致の終端・再開の位置にする)
    assert_eq!(scan_with(&t, &q, 2, None).1, [(2, 5)]);
    // 正本の一致の終端からは、正本の続き
    assert_eq!(scan_with(&t, &q, 3, None).1, []);
}

#[test]
fn 再開の位置から走査し直すと正本の続きになる() {
    let doc = "ab\ncd\r\nef\rgh\nij\n".repeat(5);
    let t = Text::from(doc.as_str());
    let q = open(r"\w\n\w", 4);
    let m = q.matcher().unwrap();
    let (cancel, mut resumes) = (CancelToken::new(), Vec::new());
    let mut on_resume = |p: ByteOff| resumes.push(p.0);
    let mut ctx = ScanCtx {
        cancel: &cancel,
        clock: None,
        resume: Some(&mut on_resume),
    };
    let mut all = Vec::new();
    let end = scan_range(
        &t,
        &m,
        ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())),
        &mut ctx,
        &mut |r| {
            all.push((r.start.0, r.end.0));
            true
        },
    );
    assert_eq!(end, ScanEnd::Done);
    assert!(resumes.len() > 5, "{resumes:?}");
    for &r in &resumes {
        let rest: Vec<_> = all.iter().copied().filter(|x| x.0 >= r).collect();
        assert_eq!(scan_with(&t, &q, r, None).1, rest, "再開の位置 {r}");
    }
}

#[test]
fn 単語単位は両端をそれぞれの行で調べる() {
    // 終端が行頭なら境界。始端はその行の単語境界
    let ww = |p: &str, core: usize| open(p, core).with_whole_word(true);
    for core in [1, 3, 1 << 20] {
        assert_eq!(
            found(&ww(r"ab\n", core), "ab\nxab\nab"),
            [(0, 3)],
            "核 {core}"
        );
        assert_eq!(
            found(&ww(r"b\nc", core), "ab\ncd b\nc"),
            [(6, 9)],
            "核 {core}"
        );
    }
    // 行の途中から走査しても、始端の行は行頭から調べる (`ー` は直前の文字の分類に続くので、`aーb` は 1 語)
    let t = Text::from("aーb\nb\n");
    assert_eq!(scan_with(&t, &ww(r"b\n?", 1), 4, None).1, [(6, 8)]);
    assert_eq!(scan_with(&t, &ww(r"b\n?", 1 << 20), 0, None).1, [(6, 8)]);
}

#[test]
fn 置換のキャプチャは窓の上で取る() {
    let doc = "ab\r\ncd x\ny";
    for core in [1, 2, 1 << 20] {
        let q = open(r"(\w+)\n(?P<n>\w+)", core);
        let t = Text::from(doc);
        let (cs, n) = build_replace_all(&t, &q, "${n}-$1", &CancelToken::new()).unwrap();
        assert_eq!(n, 2);
        let mut t2 = Text::from(doc);
        // 置換の ChangeSet の結果だけを見るので、文書テキストへ直接当てる
        #[allow(clippy::disallowed_methods)]
        t2.apply(&cs).unwrap();
        assert_eq!(
            t2.slice_to_string(ByteRange::new(ByteOff(0), ByteOff(t2.len_bytes()))),
            "cd-ab y-x",
            "核 {core}"
        );
    }
}

#[test]
fn 上限なしの検索語は大きな文書で探さない() {
    let q = query(r"(?s)a.*b").with_internal(SearchInternal {
        concat_max: 16,
        ..SearchInternal::multiline()
    });
    let m = q.matcher().unwrap();
    assert_eq!(m.lines(), LineSpan::Unbounded);
    assert!(!m.too_large_for(16));
    assert!(m.too_large_for(17));
    assert_eq!(
        scan_with(&Text::from("a\nb"), &q, 0, None),
        (ScanEnd::Done, vec![(0, 3)])
    );
    // 範囲の大きさではなく文書の大きさで決める (何も渡さない)
    let t = Text::from("a\nb..............");
    assert_eq!(scan_with(&t, &q, 0, None), (ScanEnd::TooLarge, vec![]));
    assert_eq!(scan_with(&t, &q, 17, None), (ScanEnd::TooLarge, vec![]));
}

#[test]
fn 窓が上限を超える検索語は探さない() {
    let small = |p: &str| {
        query(p).with_internal(SearchInternal {
            concat_max: 64,
            core_bytes: 1,
            ..SearchInternal::multiline()
        })
    };
    // 最大の長さが有限 (単語単位でない) なら核を行の途中で切るので、上限より長い行があっても窓は「核 + 最大の長さ」に収まる。
    // k 行では窓が上限を超える文書でも探せる (16 MiB を超える行が 1 本あっても探せなくならない)
    let q = small(r"x\ny");
    assert_eq!(q.matcher().unwrap().max_len(), Some(4));
    assert!(!q.matcher().unwrap().too_large_for(1000));
    let long = format!("x\ny\n{}\nx\ny", "z".repeat(100));
    assert_eq!(
        scan_with(&Text::from(long.as_str()), &q, 0, None),
        (ScanEnd::Done, vec![(0, 3), (105, 108)])
    );
    let doc = format!("x\n{}\n{}\nx\nzz\nq", "z".repeat(40), "w".repeat(40));
    let q = small(r"x\nz{0,3}\nq");
    assert_eq!(q.matcher().unwrap().max_len(), Some(9));
    assert_eq!(
        scan_with(&Text::from(doc.as_str()), &q, 0, None),
        (ScanEnd::Done, vec![(84, 90)])
    );
    // 最大の長さが上限に近すぎる検索語は、始める前に断る (上限を超える文書だけ)
    let q = small(r"x\n[^\n]{0,60}y");
    assert!(q.matcher().unwrap().too_large_for(65));
    assert!(!q.matcher().unwrap().too_large_for(64));
    assert_eq!(
        scan_with(&Text::from(long.as_str()), &q, 0, None),
        (ScanEnd::TooLarge, vec![])
    );
    // 最大の長さが無い k = 2 の検索語と単語単位の検索語は、核を行の境界に揃え、後ろの k 行で延ばすので、窓が上限を超える
    // ことは走査の途中で初めて分かる (それまでの一致は渡してある。呼び出し側が捨てる、03 §12.3.1 の「探さない場合」)
    let doc2 = format!("x\n\nq\n{doc}");
    let q = small(r"x\nz*\nq");
    assert!(!q.matcher().unwrap().too_large_for(doc2.len()));
    assert_eq!(
        scan_with(&Text::from(doc2.as_str()), &q, 0, None),
        (ScanEnd::TooLarge, vec![(0, 4)])
    );
    // 単語単位 (`\n` は CRLF も含めて k = 2): 最初の核の窓が、後ろの 2 行 (長い行を含む) で上限を超える
    let q = small(r"x\ny").with_whole_word(true);
    assert!(!q.matcher().unwrap().too_large_for(long.len()));
    assert_eq!(
        scan_with(&Text::from(long.as_str()), &q, 0, None),
        (ScanEnd::TooLarge, vec![])
    );
}

#[test]
fn 取り消すと止まる() {
    let t = Text::from("a\nb\n".repeat(10).as_str());
    let m = open(r"a\nb", 1).matcher().unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    let mut ctx = ScanCtx::new(&cancel);
    let end = scan_range(
        &t,
        &m,
        ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())),
        &mut ctx,
        &mut |_| true,
    );
    assert_eq!(end, ScanEnd::Cancelled);
}

#[test]
fn 一致を受け取る関数が止めたら_stopped() {
    let t = Text::from("a\nb a\nb");
    let m = open(r"a\nb", 1).matcher().unwrap();
    let cancel = CancelToken::new();
    let mut n = 0;
    let end = scan_range(
        &t,
        &m,
        ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())),
        &mut ScanCtx::new(&cancel),
        &mut |_| {
            n += 1;
            false
        },
    );
    assert_eq!((end, n), (ScanEnd::Stopped, 1));
}

#[test]
fn 核を縮めても結果は同じで_遅すぎると打ち切る() {
    let doc = "ab\ncd\nef\n".repeat(200);
    let t = Text::from(doc.as_str());
    let q = open(r"\w\n\w", 64);
    let (_, want) = scan_with(&t, &q, 0, None);
    assert_eq!(want.len(), 599);
    // 窓ごとに 60 ms かかる時計: 取り消しの遅れ (50 ms) を超えるので核を縮める。核が 1 行まで縮んだ後も 1 MB/s を下回り
    // 続けるので打ち切る (それまでに渡した一致は正本の先頭の部分)
    let slow = StepClock(AtomicU64::new(0), 60);
    let (end, part) = scan_with(&t, &q, 0, Some(&slow));
    assert_eq!(end, ScanEnd::TooHeavy);
    assert!(
        part.len() < want.len() && want.starts_with(&part),
        "{part:?}"
    );
    // 速い時計 (窓ごとに 0 ms) なら、縮めずに最後まで
    let fast = StepClock(AtomicU64::new(0), 0);
    assert_eq!(
        scan_with(&t, &q, 0, Some(&fast)),
        (ScanEnd::Done, want.clone())
    );
    // 目安を少し超える時計 (窓ごとに 51 ms): 核は 1 行まで縮むが、処理の速さは見る時間に届く前に終わる量なら打ち切らない
    let short = "ab\ncd\n".repeat(3);
    let ts = Text::from(short.as_str());
    let (_, want_s) = scan_with(&ts, &q, 0, None);
    let mid = StepClock(AtomicU64::new(0), 51);
    assert_eq!(scan_with(&ts, &q, 0, Some(&mid)), (ScanEnd::Done, want_s));
    // 全文の連結も、遅すぎると打ち切る (核を縮められないので、始めからの量と時間で見る)
    let q = open(r"(?s)a.*?b", 1 << 20);
    let t = Text::from("a\nb\n".repeat(100).as_str());
    let slow = StepClock(AtomicU64::new(0), 200);
    assert_eq!(scan_with(&t, &q, 0, Some(&slow)).0, ScanEnd::TooHeavy);
    assert_eq!(scan_with(&t, &q, 0, None).1.len(), 100);
}

#[test]
fn 捨てた候補の予算を超えたら打ち切る() {
    // NFD の文書の `(?s)de.*e`: 候補の終端が結合文字の前になり、捨てては 1 文字先から探し直す (O(n²))
    let nfd = "de\u{301}".repeat(40_000);
    let t = Text::from(nfd.as_str());
    assert_eq!(
        scan_with(&t, &open(r"(?s)de.*e", 1 << 20), 0, None).0,
        ScanEnd::TooHeavy
    );
    // 単語単位の `foo.*bar` (k = 0 の行の経路でも同じ予算)
    let line = format!("{}barx", "foox ".repeat(60_000));
    let t = Text::from(line.as_str());
    let q = open(r"foo.*bar", 1 << 20).with_whole_word(true);
    assert!(!q.matcher().unwrap().is_stream());
    assert_eq!(scan_with(&t, &q, 0, None).0, ScanEnd::TooHeavy);
    // 小さな文書は、予算の下限 (1 MiB) があるので打ち切らない
    let small = "de\u{301}".repeat(200);
    assert_eq!(
        scan_with(
            &Text::from(small.as_str()),
            &open(r"(?s)de.*e", 1 << 20),
            0,
            None
        ),
        (ScanEnd::Done, vec![])
    );
}

#[test]
fn 重い検索語の判定() {
    // 重いのは、上限のある繰り返しの上限 (入れ子は掛け合わせる) が 1000 を超える検索語 (解析の段の構文の判定)
    let heavy = |p: &str| query(p).analyze().unwrap().map(|plan| plan.heavy());
    for p in [
        "abc",
        r"a\nb",
        r"(?s)a.*b",
        r"\w+\s+\w+",
        r"\w{10}",
        r"\d{100}",
        r"\b\w{5,20}\b",
        r"[\w.+-]+@[\w-]+\.[\w.]+",
        r"[^\n]{0,1000}zq",
        r"(?:a{10}){100}",
        r"(?:\w{1000})*",
    ] {
        assert_eq!(heavy(p), Some(false), "{p}");
    }
    for p in [
        r"[^\n]{0,3000}zq",
        r"(?s).{0,1001}",
        r"(?:a{0,10}){101}",
        r"((?:x{2}){30}){20}",
    ] {
        assert_eq!(heavy(p), Some(true), "{p}");
    }
    // 照合器も同じ判定 (コンパイルは 1 回。閉じた既定でも同じ)
    assert!(
        open(r"[^\n]{0,3000}zq", 1 << 20)
            .matcher()
            .unwrap()
            .is_heavy()
    );
    assert!(query(r"[^\n]{0,3000}zq").matcher().unwrap().is_heavy());
    assert!(!open(r"\w{100}", 1 << 20).matcher().unwrap().is_heavy());
    // 10 MiB の上限も超えるものは、重いのではなく「大きすぎる」
    assert_eq!(
        open(r"\w{1000}", 1 << 20).compile().err(),
        Some(QueryError::TooLarge)
    );
    // k が 0 の重い検索語も、開いたときは行の窓で探す (取り消しの遅れと処理の速さの下限を掛ける。結果は行の経路と同じ)
    let q = open(r"[^\n]{0,3000}zq", 8);
    let m = q.matcher().unwrap();
    assert_eq!(m.lines(), LineSpan::Bounded(0));
    assert!(m.is_stream());
    assert!(!query(r"[^\n]{0,3000}zq").matcher().unwrap().is_stream());
    let doc = "ab zq\r\nzq\nxx\rzqzq\n".repeat(30);
    let t = Text::from(doc.as_str());
    let (end, got) = scan_with(&t, &q, 0, None);
    assert_eq!(end, ScanEnd::Done);
    assert_eq!(got, found(&query(r"[^\n]{0,3000}zq"), &doc));
    let slow = StepClock(AtomicU64::new(0), 300);
    assert_eq!(scan_with(&t, &q, 0, Some(&slow)).0, ScanEnd::TooHeavy);
    // 上限の無い重い検索語は、全文の連結に入れずに打ち切る (1 回の呼び出しが文書の全体に掛かり、取り消せないため)
    let q = open(r"(?s)a.*[^\n]{0,2000}b", 1 << 20);
    let m = q.matcher().unwrap();
    assert!(m.is_heavy() && m.too_heavy());
    assert_eq!(m.lines(), LineSpan::Unbounded);
    assert_eq!(
        scan_with(&Text::from("a\nb"), &q, 0, None),
        (ScanEnd::TooHeavy, vec![])
    );
}

#[test]
fn crlf_でない行頭行末は文書全体の意味() {
    // `(?-R)` の `^` / `$` は CRLF・CR を行の終端と見ない (`$` は LF の前だけ、`^` は LF の後ろだけ)。行ごとに切り出すと行の
    // 終端を文字列の端と見てしまうので、k が 0 でも窓で探す (M24 の (2) のレビューの指摘)
    for (p, doc, want) in [
        (r"a(?-R:$)", "a\r\nb", vec![]),
        (r"a(?-R:$)", "a\nb a", vec![(0, 1), (4, 5)]),
        (r"(?-R:^)b", "a\rb", vec![]),
        (r"(?-R:^)b", "a\nb\rb", vec![(2, 3)]),
        (r"(?-R)^b", "a\r\nb\rb", vec![(3, 4)]),
    ] {
        let q = open(p, 1);
        let m = q.matcher().unwrap();
        assert_eq!(m.lines(), LineSpan::Bounded(0), "{p}");
        assert!(m.is_stream(), "{p}");
        assert_eq!(found(&q, doc), want, "{p} / {doc:?}");
    }
    // 閉じた既定では今までどおり行の経路 (v0.3 と同じ)
    assert!(!query(r"a(?-R:$)").matcher().unwrap().is_stream());
}

#[test]
fn 処理の速さの下限は核の大きさに依らず数える() {
    // 1 つの窓に 30 ms (目安 50 ms の 1/4 より遅く、目安より速い) かかり、核が数行のまま縮みも戻りもしない走査でも、1 秒ごとに
    // 進んだ核の量で判定するので、下限 (1 MB/s) を下回れば打ち切る (レビューの指摘: 核が 1 行の間しか数えないと 4 秒走り切った)
    let doc = "ab\ncd\n".repeat(200);
    let t = Text::from(doc.as_str());
    let q = open(r"\w\n\w", 8);
    let clock = StepClock(AtomicU64::new(0), 30);
    let (end, part) = scan_with(&t, &q, 0, Some(&clock));
    assert_eq!(end, ScanEnd::TooHeavy);
    let elapsed = clock.0.load(Ordering::Relaxed);
    // 時計は読み取りごとに進む (窓ごとに 2 回読むので、窓の処理は 30 ms、窓と窓の間も 30 ms)。窓の処理の合計が 1 秒に
    // 届いた所で判定するので、時計の読みは約 2 秒で止まる (数え直しが続いて走り切ると 1200 / 8 × 60 ms ≒ 9 秒)
    assert!(elapsed < 2500, "{elapsed} ms で打ち切る");
    let (_, want) = scan_with(&t, &q, 0, None);
    assert!(want.starts_with(&part));
    // 速ければ (窓ごとに 0 ms) 最後まで
    let fast = StepClock(AtomicU64::new(0), 0);
    assert_eq!(scan_with(&t, &q, 0, Some(&fast)), (ScanEnd::Done, want));
}

#[test]
fn 取り消しが最後の窓の間に届いたら取り消しとして返す() {
    // 走査の最後の `regex` の呼び出しの間に取り消されても、`Done` ではなく `Cancelled` を返す (全文の連結と行の窓の両方)
    struct CancelOnRead<'a>(&'a CancelToken);
    impl Clock for CancelOnRead<'_> {
        fn now(&self) -> Tick {
            self.0.cancel();
            Tick(0)
        }
    }
    for p in [r"(?s)a.*b", r"a\nb"] {
        let t = Text::from("a\nb");
        let m = open(p, 1 << 20).matcher().unwrap();
        let cancel = CancelToken::new();
        let clock = CancelOnRead(&cancel);
        let mut ctx = ScanCtx {
            cancel: &cancel,
            clock: Some(&clock),
            resume: None,
        };
        let end = scan_range(
            &t,
            &m,
            ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())),
            &mut ctx,
            &mut |_| true,
        );
        assert_eq!(end, ScanEnd::Cancelled, "{p}");
    }
}
