//! レビュー (仕様との整合) の確かめ: CP932 互換文字の同一視 (03 §12.4.1)。レビューの後に消してよい。

use eve_core::{CancelToken, SearchQuery, build_replace_all_in, scan_bytes};
use eve_text::{ByteOff, ByteRange, Text, TextRead};
use proptest::prelude::*;

const PAIRS: [(char, char); 8] = [
    ('\u{301C}', '\u{FF5E}'),
    ('\u{2016}', '\u{2225}'),
    ('\u{2212}', '\u{FF0D}'),
    ('\u{00A2}', '\u{FFE0}'),
    ('\u{00A3}', '\u{FFE1}'),
    ('\u{00AC}', '\u{FFE2}'),
    ('\u{00A6}', '\u{FFE4}'),
    ('\u{2014}', '\u{2015}'),
];

fn re(p: &str) -> SearchQuery {
    SearchQuery::literal(p).with_regex(true)
}

fn find_all(doc: &str, q: &SearchQuery) -> Vec<(usize, usize)> {
    let t = Text::from(doc);
    let Some(m) = q.matcher() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    scan_bytes(
        &t,
        &m,
        ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())),
        &CancelToken::new(),
        &mut |r| {
            out.push((r.start.0, r.end.0));
            true
        },
    );
    out
}

/// (置換後の本文, 件数, 同一視で当たった件数)。
fn replace_all(doc: &str, q: &SearchQuery, rep: &str) -> (String, u64, u64) {
    let t = Text::from(doc);
    let b = build_replace_all_in(&t, q, rep, None, &CancelToken::new())
        .unwrap()
        .unwrap();
    let mut t2 = Text::from(doc);
    #[allow(clippy::disallowed_methods)]
    t2.apply(&b.changes).unwrap();
    (
        t2.slice_to_string(ByteRange::new(ByteOff(0), ByteOff(t2.len_bytes()))),
        b.count,
        b.folded,
    )
}

fn marked(q: &SearchQuery) -> bool {
    q.analyze()
        .ok()
        .flatten()
        .is_some_and(|p| p.compat_folded())
}

// ---- 2. すべて置換の folded の数え方 ----

/// 一致 "a" (0..1) は、同一視を切った検索語でも文書の上で同じ範囲に当たる (`b$` は文書の 2 の位置で偽)。
/// 期待: folded = 0。実装は「一致の後ろ 1 文字まで」の文字列の終端を `$` / `\b` の成り立つ端として扱う。
#[test]
fn folded_の数え方_窓の終端で後ろの選択肢が偽って成り立つ() {
    for p in [r"a(?:b$)?|〜", r"a(?:b\b)?|〜", r"a(?:b\z)?|〜"] {
        let unfolded_on_doc = regex::Regex::new(&format!("(?mR){}", p))
            .unwrap()
            .find("abc")
            .map(|m| (m.start(), m.end()));
        assert_eq!(unfolded_on_doc, Some((0, 1)), "{p}: 文書の上の同一視なしの最初の一致");
        let got = replace_all("abc", &re(p), "#");
        assert_eq!(got.0, "#bc", "{p}");
        assert_eq!(got.2, 0, "{p}: folded (仕様の定義では 0)");
    }
}

/// 上と同じ誤りの、ありそうな形: 「— の後ろに数があれば数も」。文書の `—` (同一視していない文字) の一致を数える。
#[test]
fn folded_の数え方_ありそうな形() {
    let got = replace_all("—2nd", &re(r"—(?:\d+\b)?"), "-");
    assert_eq!(got.0, "-2nd");
    assert_eq!(got.2, 0, "folded (文書の `—` は同一視で当たっていない)");
}

/// 仕様の確かめ方は「文書の上で、同一視を切った検索語を一致の始端から走らせた最初の一致が、同じ範囲でなければ数える」。
/// 同一視を切った検索語が文書の上で一致の後ろ 2 文字以上を要る選択肢で当たるとき、前後 1 文字の文字列では食い違う。
#[test]
fn folded_の数え方_窓が短く文書の上と食い違う() {
    let p = "〜|～cc|[〜～]";
    let unfolded_on_doc = regex::Regex::new(&format!("(?mR){}", p))
        .unwrap()
        .find("～cc")
        .map(|m| (m.start(), m.end()));
    // 文書の上では「～cc」(0..5) で、一致 (0..3) と違う → 仕様の確かめ方では数える
    assert_eq!(unfolded_on_doc, Some((0, 5)));
    let got = replace_all("～cc", &re(p), "#");
    assert_eq!(got.0, "#cc");
    eprintln!("{p} on ～cc: folded = {}", got.2);
    assert_eq!(got.2, 1, "仕様の確かめ方 (文書の上で探すのと同じ) なら 1");
}

#[test]
fn folded_の数え方_ふつうの例() {
    // `$1` のキャプチャと番号
    assert_eq!(
        replace_all("〜a ～b", &re(r"(〜)(?P<n>[ab])"), "$2$1"),
        ("a〜 b～".to_owned(), 2, 1)
    );
    // 空にも一致する検索語
    let g = replace_all("～", &re("〜*"), "#");
    eprintln!("〜* on ～: {g:?}");
    // 否定のクラス: 同一視を切ると `[^〜]` は `～` に一致するが、同一視すると一致しない
    assert_eq!(replace_all("〜～a", &re("[^〜]"), "#"), ("〜～#".to_owned(), 1, 0));
    // \P{Sm}: 同一視で外れる文字
    assert_eq!(replace_all("〜‖－x", &re(r"\P{Sm}"), "#"), ("〜‖－#".to_owned(), 1, 0));
}

// ---- 3. 印 ----

#[test]
fn 印の判定() {
    let yes = [
        "〜",
        r"\x{301C}",
        r"\u{FF5E}",
        r"\U0000301C",
        "[〜]",
        "[^〜]",
        r"\p{Sm}",
        r"\P{Sm}",
        r"[^\P{Sm}]",
        r"[\x{2010}-\x{2014}]",
        r"[[〜]&&[～]]",
        r"[〜-～]",
        "(?i)[^a〜]",
        r"[\p{Pd}&&\p{Sm}]",
    ];
    let no = [
        "[〜～]",
        r"\S",
        ".",
        "[^a]",
        r"[[〜]&&a]",
        "(?-u:〜)",
        "¥",
        r"[\p{Sm}\p{Pd}\p{Po}\p{Sc}]",
        r"[^\p{Sm}\p{Pd}\p{Po}\p{Sc}]",
    ];
    for p in yes {
        assert!(marked(&re(p)), "印あり: {p}");
    }
    for p in no {
        let q = re(p);
        if q.analyze().is_err() {
            eprintln!("{p}: 解析の誤り");
            continue;
        }
        assert!(!marked(&q), "印なし: {p}");
    }
    assert!(!marked(&re("〜").with_compat_fold(false)));
    assert!(!marked(&SearchQuery::literal("〜").with_compat_fold(false)));
}

// ---- 1. 組で閉じていること (どの検索語でも、文書の組の文字を相手に替えても、一致の文字の位置は変わらない) ----

const PATS: &[&str] = &[
    "〜",
    "[^〜]",
    r"\P{Sm}",
    r"[\p{Sm}--−]",
    r"[[〜]&&[～]]",
    r"[〜-～]",
    r"[‖-∥]",
    r"[\x{2010}-\x{2014}]+",
    r"(?i)[^a〜]",
    r"[^\P{Sm}]",
    r"[\p{Pd}&&\p{Sm}]",
    r"[\p{Sc}~~[¢]]",
    r"[\p{Sc}~~[￠£]]",
    r"\p{Pd}",
    r"\p{Po}+",
    r"[^\p{Sm}\p{Pd}]",
    r"[[:^alpha:]&&[^〜]]",
    r"\x{FF5E}",
    r"[\u{2015}]",
    "(?x) 〜 # c",
    "[^[^〜]]",
    "(?i:[〜])",
    r"[〜&&\p{Sm}]",
    r"[^\p{Sm}--[^〜]]",
    r"[\p{Sm}--\p{Sm}]",
    r"[^‖--∥]",
    r"[\W&&[^\p{Sm}]]",
    r"\p{Sm}{2}",
    r"[\p{Any}--[〜¢]]",
    r"[\p{Sk}\p{So}]",
    r"[\x{A0}-\x{FF}]",
    r"[\x{FF00}-\x{FFEF}]",
    r"[^\x{FF00}-\x{FFEF}]",
    r"\b〜",
    r"(?u:[^a])",
    // `(?-u)` の後ろは仕様どおり同一視しない (03 §12.4.1) ので、組で閉じない。性質から外す
    // "(?-u)a|〜",
    r"(?i)\p{Sm}|[^\p{Lu}〜]",
    r"[\p{Sm}&&[^\p{Pd}]]|[\x{FF0D}-\x{FF5E}]{2}",
    r"(?:―|—)+",
    r"[^\p{Sm}]{2}",
    r"\p{Sc}",
];

const DOC_CHARS: &[char] = &[
    '〜', '～', '‖', '∥', '−', '－', '¢', '￠', '£', '￡', '¬', '￢', '¦', '￤', '—', '―', '¥',
    '￥', 'a', ' ', '-',
];

fn to_first(s: &str) -> String {
    s.chars()
        .map(|c| PAIRS.iter().find(|p| p.1 == c).map_or(c, |p| p.0))
        .collect()
}

fn to_second(s: &str) -> String {
    s.chars()
        .map(|c| PAIRS.iter().find(|p| p.0 == c).map_or(c, |p| p.1))
        .collect()
}

/// バイト範囲を char の番号の範囲に。
fn char_ranges(doc: &str, v: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let idx = |b: usize| doc[..b].chars().count();
    v.iter().map(|&(s, e)| (idx(s), idx(e))).collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn 同一視した検索は組で閉じている(
        pi in 0..PATS.len(),
        doc in prop::collection::vec(prop::sample::select(DOC_CHARS), 0..16).prop_map(|v| v.into_iter().collect::<String>()),
    ) {
        let p = PATS[pi];
        let q = re(p);
        prop_assume!(q.analyze().is_ok());
        let a = char_ranges(&doc, &find_all(&doc, &q));
        let d1 = to_first(&doc);
        let d2 = to_second(&doc);
        let b = char_ranges(&d1, &find_all(&d1, &q));
        let c = char_ranges(&d2, &find_all(&d2, &q));
        prop_assert_eq!(&a, &b, "{} 文書 {:?} / {:?}", p, doc, d1);
        prop_assert_eq!(&a, &c, "{} 文書 {:?} / {:?}", p, doc, d2);
    }

    /// 同一視は「組のどちらでも一致する」だけを足す: 同一視しない検索で一致する文字の集合は、同一視した検索でも一致する
    /// (1 文字のクラスについて、文書を 1 文字にして調べる)。否定でないクラスでだけ。
    #[test]
    fn 同一視は否定でないクラスの集合を広げるだけ(
        pi in 0..PATS.len(),
        ch in prop::sample::select(DOC_CHARS),
    ) {
        let p = PATS[pi];
        let q = re(p);
        prop_assume!(q.analyze().is_ok());
        let doc = ch.to_string();
        let off = find_all(&doc, &re(p).with_compat_fold(false));
        let on = find_all(&doc, &q);
        // 同一視 OFF で一致し ON で一致しない文字は、否定の側の組の文字だけ (組に無い文字は変わらない)
        if PAIRS.iter().all(|&(x, y)| ch != x && ch != y) {
            prop_assert_eq!(off, on, "{} / {:?}", p, ch);
        }
    }
}

/// 個別の確かめ (結果を表示する)。
#[test]
fn 個別の例を表示() {
    for (p, d) in [
        (r"[[〜]&&[～]]", "〜～"),
        (r"[\p{Sm}--−]", "−－〜～"),
        (r"[^‖--∥]", "‖∥a"),
        (r"[\p{Sc}~~[¢]]", "¢￠£"),
        (r"[〜&&\p{Sm}]", "〜～"),
        (r"[^\p{Sm}--[^〜]]", "〜～a"),
        ("(?-u)a|〜", "〜～"),
        (r"(?-u:[^a])", "〜～"),
    ] {
        let q = re(p);
        eprintln!(
            "{p} on {d}: {:?} 印 {} / OFF {:?}",
            q.analyze().map(|x| x.is_some()),
            marked(&q),
            find_all(d, &q)
        );
        eprintln!("   off: {:?}", find_all(d, &re(p).with_compat_fold(false)));
    }
}

// ---- 互換文字を含むリテラル検索 + 単語単位で、連結の上限より長い行 (行の窓の経路に移ったことの影響) ----

#[test]
fn 単語単位のリテラル検索で長い行() {
    use eve_core::{ScanCtx, SearchTuning, scan_range};
    let tuning = SearchTuning {
        core_bytes: 4,
        concat_max: 64,
    };
    let line = format!("〜 {} ～ あ\n", "x".repeat(200));
    let doc = format!("{line}〜 y\n");
    let t = Text::from(doc.as_str());
    for (name, q) in [
        ("〜 (同一視 ON)", SearchQuery::literal("〜").with_whole_word(true).with_tuning(tuning)),
        ("〜 (同一視 OFF)", SearchQuery::literal("〜").with_whole_word(true).with_compat_fold(false).with_tuning(tuning)),
        ("あ", SearchQuery::literal("あ").with_whole_word(true).with_tuning(tuning)),
        ("〜 (単語単位なし)", SearchQuery::literal("〜").with_tuning(tuning)),
    ] {
        let m = q.matcher().unwrap();
        let mut out = Vec::new();
        let cancel = CancelToken::new();
        let mut ctx = ScanCtx::new(&cancel);
        let end = scan_range(
            &t,
            &m,
            ByteRange::new(ByteOff(0), ByteOff(t.len_bytes())),
            &mut ctx,
            &mut |r| {
                out.push((r.start.0, r.end.0));
                true
            },
        );
        eprintln!("{name}: {end:?} {out:?}");
    }
}
