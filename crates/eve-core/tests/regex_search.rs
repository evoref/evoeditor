//! 正規表現と単語単位の検索 (docs/03 §12.3 / §12.3.1 / §12.4、01 FR-402)。行をまたがない検索語は行単位で、行の終端を
//! 明示した検索語は文書全体の意味 (行の窓と全文の連結) で探す (v0.4 の M24 の (4))。

use eve_core::{Case, QueryError, SearchCount, SearchQuery};
use eve_testkit::Harness;

/// 文書全体を数えた一致の数 (全件ジョブを走らせて待つ)。
fn count(text: &str, q: SearchQuery) -> u64 {
    let mut h = Harness::new();
    h.text(text);
    h.keys("ctrl+home");
    h.editor.state_mut().search_start(q);
    h.run_jobs();
    match h.editor.state().search_count() {
        SearchCount::Exact { total, .. } => total,
        other => panic!("{other:?}"),
    }
}

fn re(p: &str) -> SearchQuery {
    SearchQuery::literal(p).with_regex(true)
}

// verifies: FR-402
#[test]
fn 正規表現で数える() {
    assert_eq!(count("a1 b22 c333", re(r"\d+")), 3);
    assert_eq!(count("fooxbar", re(r"o.b")), 1);
    assert_eq!(
        count("fooxbar", SearchQuery::literal("o.b")),
        0,
        "リテラルの . は任意の 1 文字ではない"
    );
    assert_eq!(count("foo.bar", SearchQuery::literal(".")), 1);
}

// verifies: FR-402
#[test]
fn 行頭と行末の記号は行ごとに働く() {
    assert_eq!(count("ab\nab\nab", re("^ab$")), 3);
    assert_eq!(
        count("ab\r\nab\r\n", re("^ab$")),
        2,
        "CRLF の行末の CR は行に含めない"
    );
    assert_eq!(count("xab\nab", re("^ab")), 1);
}

#[test]
fn 空の一致は数えない() {
    assert_eq!(count("abc", re("x*")), 0);
    assert_eq!(
        count("aXb", re("x*")),
        1,
        "Smart: 小文字だけなので区別せず X に一致する。a と b の位置の空の一致は数えない"
    );
    assert_eq!(count("aab", re("a*")), 1, "空でない一致だけ");
}

// verifies: FR-402
#[test]
fn 大小は_smart_で切り替わり_sensitive_insensitive_も効く() {
    assert_eq!(
        count("Foo foo FOO", re("foo")),
        3,
        "小文字だけなら区別しない"
    );
    assert_eq!(count("Foo foo FOO", re("Foo")), 1, "大文字を含めば区別する");
    assert_eq!(
        count("Foo foo FOO", re("foo").with_case(Case::Sensitive)),
        1
    );
    assert_eq!(
        count("Foo foo FOO", re("Foo").with_case(Case::Insensitive)),
        3
    );
    // エスケープの大文字 (\W など) は大文字として数えない
    assert_eq!(
        count("a b A", re(r"a\W")),
        1,
        "大文字のエスケープがあっても小文字だけ扱いなので区別しない"
    );
}

#[test]
fn 日本語を含むパターンと書記素の境界() {
    assert_eq!(count("日本語と日本語", re("日本語")), 2);
    assert_eq!(count("あいうえお", re("[あ-う]+")), 1);
    // 書記素の途中で始まる・終わる一致は採らない (e + 結合文字)
    assert_eq!(
        count("e\u{301}e", re("e")),
        1,
        "結合文字つきの e は 1 つの書記素"
    );
}

// verifies: FR-402
#[test]
fn 構文エラーは理由つきで拒否し_改行を含むパターンは探す() {
    assert!(matches!(re("(").compile(), Err(QueryError::Syntax(_))));
    assert!(matches!(re("[").compile(), Err(QueryError::Syntax(_))));
    // v0.3 までの「改行を含むパターンは未対応」は v0.4 の M24 の (4) で無くなった (03 §12.3.1)
    assert!(matches!(re(r"a\nb").compile(), Ok(Some(_))));
    assert!(matches!(re(r"a\rb").compile(), Ok(Some(_))));
    assert!(
        matches!(re(r"a\\n").compile(), Ok(Some(_))),
        "バックスラッシュ + n の文字"
    );
    assert!(matches!(re("").compile(), Ok(None)), "空は何にも一致しない");
    // 後方参照・先読みは提供しない (ReDoS 回避、§12.3)
    assert!(matches!(re(r"(a)\1").compile(), Err(QueryError::Syntax(_))));
    assert!(matches!(
        re(r"a(?=b)").compile(),
        Err(QueryError::Syntax(_))
    ));
}

// verifies: FR-402
#[test]
fn 行の終端を明示した書き方は符号位置で書いても行の終端に一致する() {
    // 03 §12.3.1 の表で行の終端に一致する書き方 (v0.3 では「未対応」、`\x0A` などは v0.3 まで黙って 0 件)
    let doc = "a\nb\r\nc\rd";
    for (p, want) in [
        (r"\x0A", 3),
        (r"\x{A}", 3),
        (r"a\u000Ab", 1),
        (r"\r\n", 1),
        (r"\x{D}", 1),
        (r"[\n]", 3),
        (r"[ \r]", 1),
        (r"[\s\n]+", 3),
        // 要素に `\n` / `\r` を書いた、すべての文字のクラスは、全文を 1 つの一致にする
        (r"[\n\s\S]+", 1),
        (r"[\r\n\s\S]+", 1),
    ] {
        assert_eq!(count(doc, re(p)), want, "{p}");
    }
    assert!(
        matches!(re(r"\n{0}a").compile(), Ok(Some(_))),
        "一致が行の終端を含みえない"
    );
}

// verifies: FR-402
#[test]
fn 行の終端を明示しない書き方は_1_行の中で探す() {
    // 否定・略記のクラスは行の終端を集合から除く。`[^\n]` は CR にも一致しない (03 §12.3.1)
    assert_eq!(count("ab\ncd\r\nef\rgh", re(r"[^\n]+")), 4);
    assert_eq!(count("ab\ncd\r\nef\rgh", re(r"[^x]+")), 4);
    assert_eq!(count("a b\na\tb", re(r"a\sb")), 2);
    assert_eq!(
        count("\"a\nb\" \"c\"", re(r#""[^"]*""#)),
        1,
        "行をまたがない"
    );
    // `(?s)` の `.` と `[\s\S]` は行の終端を明示した書き方で、行をまたぐ (v0.3 までは 1 行の中、M24 の (4))
    assert_eq!(count("axb\na\nb", re(r"(?s)a.b")), 2);
    assert_eq!(count("a b\na\nb", re(r"a[\s\S]b")), 2);
    assert_eq!(count("a\r\nb", re(r"(?s)a.b")), 1, "CRLF は 1 文字に数える");
    // `^` / `$` は行頭と行末 (CRLF・CR も行の終端)
    assert_eq!(count("ab\r\nab\rab", re("^ab$")), 3);
}

// verifies: FR-402
#[test]
fn 文書の先頭と末尾の記号は文書の端にだけ一致する() {
    // v0.3 までは 1 行ずつ探したので各行の先頭 / 末尾に一致していた (03 §12.3.1 の「v0.3 からの振る舞いの変化」)
    assert_eq!(count("ab\nab\nab", re(r"\Aab")), 1);
    assert_eq!(count("ab\nab\nab", re(r"ab\z")), 1);
    assert_eq!(count("ab\nab\nab", re(r"(?-m)^ab")), 1);
    assert_eq!(count("ab\nab\nab", re(r"(?-m)ab$")), 1);
    // `(?-R)` の `^` / `$` は LF の後ろ / 前と文書の端 (CR・CRLF の行の終端の前後では一致しない)
    assert_eq!(count("ab\rab\nab", re(r"(?-R)^ab")), 2);
}

// verifies: FR-402
#[test]
fn 構文エラーのときは何にも一致しない() {
    assert_eq!(count("a(b", re("(")), 0);
}

#[test]
fn 巨大なパターンは拒否する() {
    let big = "(?:[a-z]{1000}){1000}".to_owned();
    assert!(matches!(
        re(&big).compile(),
        Err(QueryError::TooLarge | QueryError::Syntax(_))
    ));
}

#[test]
fn 単語単位は両端が単語境界のものだけ() {
    let w = |p: &str| SearchQuery::literal(p).with_whole_word(true);
    assert_eq!(
        count("foo foobar barfoo foo_bar foo.bar", w("foo")),
        2,
        "foo と foo.bar の foo"
    );
    assert_eq!(
        count("foo foobar", SearchQuery::literal("foo")),
        2,
        "単語単位でなければ部分一致"
    );
    assert_eq!(count("a.b a-b ab", w("a")), 2);
    // 日本語: 文字種が変わるところが境界
    assert_eq!(
        count("東京都と京都", w("京都")),
        1,
        "「東京都」の京都は漢字の連なりの途中"
    );
    assert_eq!(count("カタカナとひらがな", w("カタカナ")), 1);
    assert_eq!(count("カタカナとひらがな", w("カタ")), 0);
    assert_eq!(
        count("ＡＢＣabc", w("abc")),
        1,
        "全角英数と半角英数は別の単位"
    );
    assert_eq!(count("ＡＢＣabc", w("ＡＢＣ")), 1);
}

#[test]
fn 単語単位は正規表現とも組み合わせられる() {
    let q = re(r"fo+").with_whole_word(true);
    assert_eq!(count("fo foo fooo food", q), 3, "food の fo は単語の途中");
}

#[test]
fn 単語単位でも大小の区別は効く() {
    let q = SearchQuery::literal("Foo").with_whole_word(true);
    assert_eq!(count("Foo foo FOO", q), 1);
    let q = SearchQuery::literal("foo").with_whole_word(true);
    assert_eq!(count("Foo foo FOO", q), 3);
}
