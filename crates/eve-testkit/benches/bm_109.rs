//! BM-109 / BM-109r / BM-109m / BM-109-compat: 文書の検索の全件の走査 (docs/09-testing-quality.md §5.2、01 NFR-109。
//! 参考値として記録する。docs/review/v0.3-findings.md の §3.3 の 5)。ヘッドレス (`Harness`・`MemFs`) で、検索を始めてから
//! 全件を数え終える (`SearchCount::Exact`) までの時間を測る。
//!
//! **BM-109** (100 MB のリテラル検索。チャンクごとの memmem の経路)。検索語は 09 §5.2 のとおり:
//!
//! - `ascii`: ASCII の語 (`k` `K` `s` `S` を含まない) を大小の区別あり (`Sensitive`) で。
//! - `ascii-i`: 同じ語を大小の区別なし (`Insensitive`) で (ASCII の大小無視は memmem の経路のまま)。
//! - `j3` / `j10`: 日本語の 3 文字・10 文字の語 (BM-109j。区別の無い文字だけなので `Smart` でも memmem)。
//! - `many`: 一致の多い語 (文書の各行にある語)。
//!
//! **BM-109r** (行単位の方式。1 行で済む正規表現と、行ごとの経路になるリテラル。v0.4 の S-04 で、複数行の正規表現の
//! 候補の方式と比べる相手、08 §3)。100 MB (平均 80 B の行) と、10 MB の 1 行 (`-1line`) の 2 つの文書で測る:
//!
//! - `digits`: 文字クラスで始まる正規表現 (`\d+ a`)。一致が多い。
//! - `anchor`: 行頭に固定した正規表現 (`^末尾`)。ほとんどの行は一致しない (行ごとの固定の費用が出る)。
//! - `ci`: 大小無視の正規表現 (`(?i)zebra`)。
//! - `ks`: `k` を含む ASCII の語の大小無視のリテラル (`quick`。行ごとの経路になる、03 §12.1 の経路 3)。
//!
//! 一致の数が数える上限 (`search.count_limit` = 100 万) に届くと走査がそこで止まるので、どの検索語も上限より
//! 少ない数だけ一致するものにしてある (文書の作り方を変えたら、検索語も見直す)。
//!
//! **BM-109m** (v0.4 の M24。行をまたぐ正規表現を行の窓 (03 §12.3.1) で)。S-04 の 6 つのパターンを、100 MB と 10 MB の
//! 1 行 (`-1line`) で測る。`unbounded` (上限の無いパターン。全文の連結の経路) は 16 MiB 以下の `-1line` だけ (100 MB は
//! 探さない)。S-04 の教訓 (開発機はクロックの状態で絶対値が 1.7 倍まで揺れた) に倣い、**比べる相手 (全文を 1 本に連結して
//! `regex` で探す参照。連結の確保と複写・コンパイル・03 §12.1 の一致の選び方を含む) と交互に 5 巡回し**、製品の時間の中央値と、
//! 同じ巡の中の比 (製品 / 参照) の中央値を出す。件数は参照と突き合わせる。
//!
//! **BM-109-compat** (v0.4 の M24): CP932 互換文字を含むリテラル (`〜終わり`) を同一視 ON で (行の窓の経路、03 §12.4.1)。
//! 同一視 OFF (memmem の経路) と交互に 5 巡回する。NFR-109 の目標 (300 ms、許容 1 s) で記録する。
//!
//! **M24 の参考の計測** (09 §5.2):
//!
//! - `BM-109m-longline-1m`: 1 MB で行が 8〜32 KiB の文書。全件の走査 (参照と交互) と、可視範囲の同期検索 (03 §12.3.1 の
//!   「可視範囲の強調」の 2。窓が 64 KiB 以下のときだけメインスレッドで走る) と同じ範囲の `scan_range` の時間の分布。
//! - `BM-109m-heavy`: 重めの検索語 `[^\n]{0,1000}zq` (繰り返しの上限が判定の値 1000 ちょうど。重くない) のコンパイルと
//!   100 MB の走査。重い側の `[^\n]{0,1001}zq` (行の窓で核を小さく始める) と交互に回し、比を出す。行をまたぐ形
//!   `[^\n]{0,1000}\nzq` の 64 KiB 以下の同期検索の時間も測る (メインスレッドで許す最も重い形。比べるため
//!   `[^\n]{0,100}\nzq` も)。
//! - `BM-109m-nextprev`: 100 MB・k ≥ 1 (`abcdef\n\d+ 日本語`。一致は保持の上限 1 万件を超える) で、全件の走査の後に保持に
//!   無い位置へカーソルを移し、`search.next` / `search.prev` の 1 回と、`search.next` の 5 回の連打の遅れ (単発のジョブを
//!   含む。ヘッドレスなので描画は含まない)。
//! - `BM-109m-unbounded-16m`: 16 MiB 以下で行の多い文書の上限の無いパターン (`(?s)目印.*終わり`。連結の経路の上限)。
//! - `BM-109m-bigk`: k の大きい検索語 `(?:[^\n]*\n){100}zq` (100 MB)。
//!
//! BM-108 (最初の一致までの時間) は、ヘッドレスのジョブ (`ManualJobSpawner`) が 1 回で最後まで走るので分けて測れない。
//! 本物のワーカーで測る計測 (`bm_pty` の計測用ビルド) で測る。
//!
//! ```text
//! cargo bench -p eve-testkit --bench bm_109                    # 100 MB (と 10 MB の 1 行)
//! cargo bench -p eve-testkit --bench bm_109 -- --quick         # 10 MB (と 1 MB の 1 行。動作確認。記録には使わない)
//! cargo bench -p eve-testkit --bench bm_109 -- --json          # 1 行ずつの JSON (xtask bench が集める)
//! cargo bench -p eve-testkit --bench bm_109 -- --only 109m,ref # 一部だけ (109 = BM-109 / 109r、109m、compat、ref = M24 の参考)
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]
// 検索を始める・止めるのに EditorState を直接使う (試験と計測だけに許す Editor::state_mut、STR-B3)
#![allow(clippy::disallowed_methods)]

use std::time::{Duration, Instant};

use eve_core::{
    Args, CancelToken, Case, LineSpan, ScanCtx, ScanEnd, SearchCount, SearchQuery, SearchStep,
    scan_range,
};
use eve_testkit::Harness;
use eve_text::{ByteOff, ByteRange, LineIdx, Text, TextRead, is_grapheme_boundary};

/// 数える一致の上限 (`search.count_limit`、07 §5.5)。届くと走査が止まるので、全件の走査の計測にならない。
const COUNT_LIMIT: u64 = 1_000_000;

/// 方式を交互に回す巡の数 (S-04 の教訓、09 §5.2)。
const ROUNDS: usize = 5;

/// 可視範囲の同期検索の窓の上限 (eve-core の `SEARCH_SYNC_MULTILINE_MAX`、03 §12.3.1。crate の外からは見えないので写す)。
const SYNC_MAX: usize = 64 << 10;

/// 平均 80 B の行 (ASCII と日本語が混ざる) を `bytes` 以上になるまで並べる。決定的。末尾にだけ目印を置く。
/// `sep` は行の区切り (`\n`。1 行の文書を作るときは空白)。
fn corpus(bytes: usize, sep: char) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes + 256);
    let mut i = 0u64;
    while out.len() < bytes {
        out.extend_from_slice(segment(i, sep).as_bytes());
        i += 1;
    }
    out.extend_from_slice(format!("末尾の目印 zebra 終わりの十文字の日本語です{sep}").as_bytes());
    out
}

/// `corpus` の `i` 番目の行 (区切り `sep` を含む)。
fn segment(i: u64, sep: char) -> String {
    if i.is_multiple_of(3) {
        format!("{i:08} 日本語の行です。テキストエディタの試験データ。0123456789{sep}")
    } else {
        format!("{i:08} the quick brown fox jumps over the lazy dog 0123456789 abcdef{sep}")
    }
}

/// `corpus` と同じ部品を空白でつなぎ、行の長さを 8〜32 KiB (決定的な疑似乱数) にした文書 (`BM-109m-longline-1m`)。
fn longline_corpus(bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes + (64 << 10));
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    let next_len = |rng: &mut u64| {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 7;
        *rng ^= *rng << 17;
        let span = (32usize << 10) - (8 << 10);
        (8 << 10) + usize::try_from(*rng % span as u64).unwrap_or(0)
    };
    let mut i = 0u64;
    let mut line_start = 0usize;
    let mut target = next_len(&mut rng);
    while out.len() < bytes {
        out.extend_from_slice(segment(i, ' ').as_bytes());
        i += 1;
        if out.len() - line_start >= target {
            // 区切りの空白を改行に替える
            let last = out.len() - 1;
            out[last] = b'\n';
            line_start = out.len();
            target = next_len(&mut rng);
        }
    }
    out.extend_from_slice("末尾の目印 zebra 終わりの十文字の日本語です\n".as_bytes());
    out
}

/// 1 つの検索語の計測。
struct Case1 {
    /// 記録の ID (`BM-109-ascii` など)。
    id: String,
    query: SearchQuery,
}

fn literal(id: &str, pattern: &str, case: Case) -> Case1 {
    Case1 {
        id: id.to_owned(),
        query: SearchQuery::literal(pattern).with_case(case),
    }
}

/// 大小を区別する正規表現の問い合わせ (毎回作り直すと、照合器のコンパイルも測る範囲に入る)。
fn regex_query(pattern: &str) -> SearchQuery {
    SearchQuery::literal(pattern)
        .with_regex(true)
        .with_case(Case::Sensitive)
}

fn regex(id: &str, pattern: &str) -> Case1 {
    Case1 {
        id: id.to_owned(),
        query: regex_query(pattern),
    }
}

/// 行単位の方式の検索語 (BM-109r)。`suffix` は ID の末尾 (1 行の文書は `-1line`)。
fn line_route_cases(suffix: &str) -> Vec<Case1> {
    vec![
        regex(&format!("BM-109r-digits{suffix}"), r"\d+ a"),
        regex(&format!("BM-109r-anchor{suffix}"), "^末尾"),
        regex(&format!("BM-109r-ci{suffix}"), "(?i)zebra"),
        literal(&format!("BM-109r-ks{suffix}"), "QUICK", Case::Insensitive),
    ]
}

/// `data` を開いた `Harness`。
fn open(data: &[u8]) -> Harness {
    let mut h = Harness::new()
        .with_size(120, 40)
        .with_file("/w/big.txt", data);
    h.open("/w/big.txt");
    h.run_jobs();
    h
}

/// `data` を開いて、`cases` を順に測る。
fn measure(data: &[u8], mb: usize, cases: &[Case1], quick: bool, json: bool) {
    let mut h = open(data);
    for c in cases {
        let id = &c.id;
        let runs = if quick { 1 } else { 3 };
        let mut times: Vec<Duration> = Vec::with_capacity(runs);
        let mut total = 0;
        for _ in 0..runs {
            let (d, r) = job_scan(&mut h, c.query.clone());
            total = r.unwrap_or_else(|e| panic!("{id}: 全件を数え終えていない: {e}"));
            assert!(
                total < COUNT_LIMIT,
                "{id}: 一致が数える上限に届いた (走査が途中で止まる)。検索語を見直す"
            );
            times.push(d);
        }
        times.sort();
        let med = times[times.len() / 2];
        if json {
            println!(
                "{{\"id\":\"{id}\",\"n\":{runs},\"p50_ms\":{:.1},\"matches\":{total},\"mb\":{mb},\"quick\":{quick}}}",
                med.as_secs_f64() * 1e3
            );
        } else {
            println!(
                "{id} ({mb} MB, {:?}): 全件の走査 {med:?} (一致 {total} 件、{runs} 回の中央値){}",
                c.query.pattern(),
                if quick { " [--quick]" } else { "" }
            );
        }
    }
}

/// 検索を始めてから全件を数え終えるまで (ヘッドレスのジョブ。照合器のコンパイルを含む)。探さなかった・打ち切った
/// (03 §12.3.1 の「探さない場合」「最悪の入力の上限」) ときは `Err` (件数の状態)。
fn job_scan(h: &mut Harness, q: SearchQuery) -> (Duration, Result<u64, String>) {
    let t = Instant::now();
    h.editor.state_mut().search_start(q);
    h.run_jobs();
    let d = t.elapsed();
    let r = match h.editor.state().search_count() {
        SearchCount::Exact { total, .. } | SearchCount::AtLeast { total, .. } => Ok(total),
        other => Err(format!("{other:?}")),
    };
    h.editor.state_mut().search_stop();
    (d, r)
}

/// 比べる相手: 全文を 1 本の文字列に連結し、書き換えた検索語 (製品と同じ) を `regex` でコンパイルして、03 §12.1 の一致の
/// 選び方 (空の一致は採らずに 1 文字進める・両端が書記素境界でない候補は捨てて次の char 境界から) で数える。S-04 の (c)。
fn concat_scan(text: &dyn TextRead, q: &SearchQuery) -> (Duration, u64) {
    let t = Instant::now();
    let plan = q.analyze().expect("検索語").expect("空でない");
    let pattern = plan.regex_pattern().expect("正規表現の経路");
    let re = regex::RegexBuilder::new(pattern)
        .case_insensitive(!q.is_case_sensitive())
        .multi_line(true)
        .crlf(true)
        .size_limit(10 << 20)
        .dfa_size_limit(8 << 20)
        .build()
        .expect("参照の照合器");
    let doc = text.slice_to_string(ByteRange::new(ByteOff(0), ByteOff(text.len_bytes())));
    let next_char = |i: usize| i + doc[i..].chars().next().map_or(1, char::len_utf8);
    let mut n = 0u64;
    let mut pos = 0;
    while pos <= doc.len() {
        let Some(m) = re.find_at(&doc, pos) else {
            break;
        };
        let (s, e) = (m.start(), m.end());
        if s == e {
            pos = next_char(e);
            continue;
        }
        if is_grapheme_boundary(text, ByteOff(s)) && is_grapheme_boundary(text, ByteOff(e)) {
            n += 1;
            pos = e;
        } else {
            pos = next_char(s);
        }
    }
    (t.elapsed(), n)
}

/// 交互に回した結果。
struct Alt {
    /// 製品 (a) の時間 (巡の順)。
    a: Vec<Duration>,
    /// 比べる相手 (b) の時間。
    b: Vec<Duration>,
    /// 同じ巡の中の比 a / b。
    ratio: Vec<f64>,
}

fn median_d(v: &[Duration]) -> Duration {
    let mut s = v.to_vec();
    s.sort();
    s[s.len() / 2]
}

fn median_f(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[s.len() / 2]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

impl Alt {
    /// `(製品の中央値, 製品の最小〜最大, 相手の中央値, 比の中央値, 比の最小〜最大)` の JSON の項目 (`,"k":v` の形)。
    fn json(&self) -> String {
        let (amin, amax) = (self.a.iter().min(), self.a.iter().max());
        let (rmin, rmax) = (
            self.ratio.iter().copied().fold(f64::MAX, f64::min),
            self.ratio.iter().copied().fold(f64::MIN, f64::max),
        );
        format!(
            ",\"p50_ms\":{:.1},\"min_ms\":{:.1},\"max_ms\":{:.1},\"ref_p50_ms\":{:.1},\"ratio\":{:.2},\"ratio_min\":{:.2},\"ratio_max\":{:.2}",
            ms(median_d(&self.a)),
            amin.map_or(0.0, |d| ms(*d)),
            amax.map_or(0.0, |d| ms(*d)),
            ms(median_d(&self.b)),
            median_f(&self.ratio),
            rmin,
            rmax
        )
    }

    fn text(&self, what_b: &str) -> String {
        let (rmin, rmax) = (
            self.ratio.iter().copied().fold(f64::MAX, f64::min),
            self.ratio.iter().copied().fold(f64::MIN, f64::max),
        );
        format!(
            "{:.1} ms (巡の値 {}) / {what_b} {:.1} ms、比の中央値 {:.2} ({rmin:.2}〜{rmax:.2})",
            ms(median_d(&self.a)),
            self.a
                .iter()
                .map(|d| format!("{:.1}", ms(*d)))
                .collect::<Vec<_>>()
                .join(" "),
            ms(median_d(&self.b)),
            median_f(&self.ratio)
        )
    }
}

/// 製品 (`run(true)`) と比べる相手 (`run(false)`) を交互に `rounds` 巡回す (1 巡目は製品が先、2 巡目は相手が先、…。
/// 順の偏りを消す)。
fn alternate(rounds: usize, mut run: impl FnMut(bool) -> Duration) -> Alt {
    let mut out = Alt {
        a: Vec::new(),
        b: Vec::new(),
        ratio: Vec::new(),
    };
    for i in 0..rounds {
        let (da, db) = if i % 2 == 0 {
            let x = run(true);
            (x, run(false))
        } else {
            let y = run(false);
            (run(true), y)
        };
        out.ratio
            .push(da.as_secs_f64() / db.as_secs_f64().max(1e-9));
        out.a.push(da);
        out.b.push(db);
    }
    out
}

/// 1 行の結果を出す。`extra` は JSON の項目 (`,"k":v` の形)、`text` は文の出力。
fn emit(json: bool, quick: bool, id: &str, extra: &str, text: &str) {
    if json {
        println!("{{\"id\":\"{id}\"{extra},\"quick\":{quick}}}");
    } else {
        println!("{id}: {text}{}", if quick { " [--quick]" } else { "" });
    }
}

/// BM-109m の 6 つのパターン (S-04)。`(名前, パターン, 100 MB でも測るか)`。
const MULTILINE: [(&str, &str, bool); 6] = [
    ("nl-ja", r"abcdef\n\d+ 日本語", true),
    ("nl-digits", r"。0123456789\n\d+ the", true),
    ("nl-none", r"abcdef\n末尾", true),
    ("nl-anchor", r"^\d+ the quick[^\r\n]*\n\d+ the", true),
    ("blank", r"\n\n", true),
    // 上限の無いパターン (全文の連結の経路)。100 MB は 16 MiB を超えるので探さない
    ("unbounded", r"(?s)目印.*終わり", false),
];

/// 製品 (行の窓・全文の連結のジョブ) と全文の連結の参照を交互に回し、件数を突き合わせる。
fn product_vs_concat(h: &mut Harness, id: &str, pattern: &str, rounds: usize) -> (Alt, u64) {
    let mut product_n = None;
    let mut ref_n = None;
    let text = h
        .editor
        .state()
        .document(h.doc_id())
        .expect("文書")
        .text()
        .snapshot();
    let alt = alternate(rounds, |product| {
        if product {
            let (d, r) = job_scan(h, regex_query(pattern));
            let n = r.unwrap_or_else(|e| panic!("{id}: 全件を数え終えていない: {e}"));
            assert!(n < COUNT_LIMIT, "{id}: 一致が数える上限に届いた");
            product_n = Some(n);
            d
        } else {
            let (d, n) = concat_scan(&text, &regex_query(pattern));
            ref_n = Some(n);
            d
        }
    });
    assert_eq!(
        product_n, ref_n,
        "{id}: 件数が全文の連結の参照と違う (製品 / 参照)"
    );
    (alt, product_n.unwrap_or(0))
}

/// BM-109m (100 MB と 10 MB の 1 行)。
fn bm_109m(big: &[u8], mb: usize, line: &[u8], line_mb: usize, quick: bool, json: bool) {
    let rounds = if quick { 1 } else { ROUNDS };
    let mut h = open(big);
    for (name, pattern, on_big) in MULTILINE {
        if !on_big {
            continue;
        }
        let id = format!("BM-109m-{name}");
        let (alt, n) = product_vs_concat(&mut h, &id, pattern, rounds);
        emit(
            json,
            quick,
            &id,
            &format!(",\"n\":{rounds},\"matches\":{n},\"mb\":{mb}{}", alt.json()),
            &format!(
                "{mb} MB {pattern:?}: 一致 {n} 件、{}",
                alt.text("全文の連結")
            ),
        );
    }
    drop(h);
    let mut h = open(line);
    for (name, pattern, _) in MULTILINE {
        let id = format!("BM-109m-{name}-1line");
        let (alt, n) = product_vs_concat(&mut h, &id, pattern, rounds);
        emit(
            json,
            quick,
            &id,
            &format!(
                ",\"n\":{rounds},\"matches\":{n},\"mb\":{line_mb}{}",
                alt.json()
            ),
            &format!(
                "{line_mb} MB の 1 行 {pattern:?}: 一致 {n} 件、{}",
                alt.text("全文の連結")
            ),
        );
    }
}

/// BM-109-compat: CP932 互換文字を含むリテラルを同一視 ON (行の窓) で。OFF (memmem) と交互に回す。
fn bm_109_compat(h: &mut Harness, mb: usize, quick: bool, json: bool) {
    let rounds = if quick { 1 } else { ROUNDS };
    let pattern = "〜終わり";
    let mut on_n = 0;
    let alt = alternate(rounds, |on| {
        let (d, r) = job_scan(h, SearchQuery::literal(pattern).with_compat_fold(on));
        let n = r.unwrap_or_else(|e| panic!("BM-109-compat ({on}): 全件を数え終えていない: {e}"));
        if on {
            on_n = n;
        }
        d
    });
    emit(
        json,
        quick,
        "BM-109-compat",
        &format!(
            ",\"n\":{rounds},\"matches\":{on_n},\"mb\":{mb}{}",
            alt.json()
        ),
        &format!(
            "{mb} MB {pattern:?} (同一視 ON): 一致 {on_n} 件、{}",
            alt.text("同一視 OFF (memmem)")
        ),
    );
}

/// 分布 (p50 / p99 / 最大) の JSON の項目と文。
fn dist(prefix: &str, v: &mut [Duration]) -> (String, String) {
    v.sort();
    // 百分位 (千分率 `permille`) の添字を、(len - 1) × p を四捨五入して求める (bm_pty の `percentile` と同じ)
    let p = |permille: usize| v[((v.len() - 1) * permille + 500) / 1000];
    let (p50, p99, max) = (p(500), p(990), v[v.len() - 1]);
    (
        format!(
            ",\"{prefix}n\":{},\"{prefix}p50_ms\":{:.3},\"{prefix}p99_ms\":{:.3},\"{prefix}max_ms\":{:.3}",
            v.len(),
            ms(p50),
            ms(p99),
            ms(max)
        ),
        format!(
            "n={} p50 {:.3} ms / p99 {:.3} ms / 最大 {:.3} ms",
            v.len(),
            ms(p50),
            ms(p99),
            ms(max)
        ),
    )
}

/// 可視範囲の同期検索 (03 §12.3.1 の「可視範囲の強調」の 2) と同じ範囲を、文書の各行を先頭に置いて `scan_range` で探した時間。
/// 窓 (先頭の k 行前の行頭から、最後の可視行の k 行後ろの行頭まで) が 64 KiB 以下になる最も多くの可視行を採り、1 行も
/// 収まらない位置は数えない (製品は同期検索をしない)。`(時間, 測った位置の数, 全体の位置の数)`。
fn sync_windows(text: &dyn TextRead, pattern: &str, reps: usize) -> (Vec<Duration>, usize, usize) {
    let q = regex_query(pattern);
    let m = q.compile().expect("検索語").expect("空でない");
    let LineSpan::Bounded(k) = m.lines() else {
        panic!("{pattern}: 行の数に上限が無い");
    };
    let n = text.len_lines();
    let line_at = |l: usize| {
        if l >= n {
            text.len_bytes()
        } else {
            text.line_to_byte(LineIdx(l)).0
        }
    };
    let mut out = Vec::new();
    let mut used = 0;
    for top in 0..n {
        let core_s = line_at(top.saturating_sub(k));
        let mut last = top;
        while last < n && line_at(last + 1 + k).saturating_sub(core_s) <= SYNC_MAX {
            last += 1;
        }
        if last == top {
            continue;
        }
        used += 1;
        let range = ByteRange::new(ByteOff(core_s), ByteOff(line_at(last)));
        for _ in 0..reps {
            let cancel = CancelToken::new();
            let t = Instant::now();
            let mut found = 0usize;
            let end = scan_range(text, &m, range, &mut ScanCtx::new(&cancel), &mut |_| {
                found += 1;
                true
            });
            out.push(t.elapsed());
            assert_eq!(end, ScanEnd::Done, "{pattern}: 同期検索が打ち切られた");
        }
    }
    (out, used, n)
}

/// M24 の参考の計測 (09 §5.2)。
#[expect(clippy::too_many_lines, reason = "計測の手順を順に並べる")]
fn m24_reference(big: &[u8], mb: usize, quick: bool, json: bool) {
    let rounds = if quick { 1 } else { ROUNDS };

    // ---- BM-109m-longline-1m ----
    let long = longline_corpus(1 << 20);
    {
        let mut h = open(&long);
        let id = "BM-109m-longline-1m";
        let pattern = MULTILINE[0].1;
        let (alt, n) = product_vs_concat(&mut h, id, pattern, rounds);
        let text = h
            .editor
            .state()
            .document(h.doc_id())
            .expect("文書")
            .text()
            .snapshot();
        let (mut v, used, lines) = sync_windows(&text, pattern, if quick { 1 } else { 20 });
        let (dj, dt) = dist("sync_", &mut v);
        emit(
            json,
            quick,
            id,
            &format!(
                ",\"n\":{rounds},\"matches\":{n},\"mb\":1,\"lines\":{lines},\"sync_positions\":{used}{}{dj}",
                alt.json()
            ),
            &format!(
                "1 MB (行 8〜32 KiB、{lines} 行) {pattern:?}: 全件の走査 {}。同期検索 (64 KiB 以下の窓、{used} 位置): {dt}",
                alt.text("全文の連結")
            ),
        );
    }

    // ---- BM-109m-heavy ----
    {
        let id = "BM-109m-heavy";
        let light = r"[^\n]{0,1000}zq";
        let heavy = r"[^\n]{0,1001}zq";
        let mut compile = Vec::new();
        for _ in 0..rounds {
            let q = regex_query(light);
            let t = Instant::now();
            let m = q.compile().expect("検索語").expect("空でない");
            compile.push(t.elapsed());
            assert!(!m.is_heavy(), "{light} は重くない (上限 1000 ちょうど)");
        }
        assert!(
            regex_query(heavy)
                .compile()
                .expect("検索語")
                .expect("空でない")
                .is_heavy(),
            "{heavy} は重い"
        );
        let mut h = open(big);
        let mut n = 0;
        let alt = alternate(rounds, |is_light| {
            let p = if is_light { light } else { heavy };
            let (d, r) = job_scan(&mut h, regex_query(p));
            let c = r.unwrap_or_else(|e| panic!("{id} ({p}): 全件を数え終えていない: {e}"));
            if is_light {
                n = c;
            }
            d
        });
        drop(h);
        let long_text = Text::from(std::str::from_utf8(&long).expect("UTF-8"));
        let (mut v, used, _) =
            sync_windows(&long_text, r"[^\n]{0,1000}\nzq", if quick { 1 } else { 20 });
        let (dj, dt) = dist("sync_", &mut v);
        // 比べるため、繰り返しの上限が 1 桁小さい形 (判定の値を下げたときの材料)
        let (mut v100, _, _) =
            sync_windows(&long_text, r"[^\n]{0,100}\nzq", if quick { 1 } else { 20 });
        let (dj100, dt100) = dist("sync100_", &mut v100);
        emit(
            json,
            quick,
            id,
            &format!(
                ",\"n\":{rounds},\"matches\":{n},\"mb\":{mb},\"compile_ms\":{:.2}{},\"sync_positions\":{used}{dj}{dj100}",
                ms(median_d(&compile)),
                alt.json()
            ),
            &format!(
                "{light:?} のコンパイル {:.2} ms。{mb} MB の走査 {}。1 MB (行 8〜32 KiB) の 64 KiB 以下の窓の同期検索 ({used} 位置): `[^\\n]{{0,1000}}\\nzq` {dt}、`[^\\n]{{0,100}}\\nzq` {dt100}",
                ms(median_d(&compile)),
                alt.text("重い側の [^\\n]{0,1001}zq")
            ),
        );
    }

    // ---- BM-109m-nextprev ----
    {
        let id = "BM-109m-nextprev";
        let pattern = MULTILINE[0].1;
        let mut h = open(big);
        let (d, r) = {
            let t = Instant::now();
            h.editor.state_mut().search_start(regex_query(pattern));
            h.run_jobs();
            let d = t.elapsed();
            let r = h.editor.state().search_count();
            (d, r)
        };
        let SearchCount::Exact { total, .. } = r else {
            panic!("{id}: 全件を数え終えていない: {r:?}");
        };
        assert!(total > 10_000, "{id}: 一致が保持の上限を超えない");
        let lines = h
            .editor
            .state()
            .document(h.doc_id())
            .expect("文書")
            .text()
            .len_lines();
        let positions = if quick { 2 } else { 10 };
        let goto = |h: &mut Harness, i: usize| {
            // 文書の 30〜90 % の行 (保持した最初の 1 万件より後ろ)
            let line = lines * (30 + 60 * i / positions) / 100;
            h.cmd_with(
                "cursor.goto_line",
                Args::new().with_int("line", i64::try_from(line + 1).unwrap_or(i64::MAX)),
            );
            h.selection_range().0
        };
        let step = |h: &mut Harness, forward: bool, presses: usize| {
            let t = Instant::now();
            for _ in 0..presses {
                let s = h.editor.state_mut().search_step(forward);
                assert!(
                    matches!(s, SearchStep::NotYet | SearchStep::Moved { .. }),
                    "{id}: {s:?}"
                );
            }
            h.run_jobs();
            t.elapsed()
        };
        let (mut next, mut prev, mut burst) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..positions {
            let origin = goto(&mut h, i);
            next.push(step(&mut h, true, 1));
            assert!(h.selection_range().0 >= origin, "{id}: 次が起点より前");
            let origin = goto(&mut h, i);
            prev.push(step(&mut h, false, 1));
            assert!(h.selection_range().1 <= origin, "{id}: 前が起点より後ろ");
            let origin = goto(&mut h, i);
            burst.push(step(&mut h, true, 5));
            assert!(h.selection_range().0 > origin, "{id}: 5 回の連打で進まない");
        }
        let (nj, nt) = dist("next_", &mut next);
        let (pj, pt) = dist("prev_", &mut prev);
        let (bj, bt) = dist("burst5_", &mut burst);
        emit(
            json,
            quick,
            id,
            &format!(
                ",\"n\":{positions},\"matches\":{total},\"mb\":{mb},\"p50_ms\":{:.3},\"scan_ms\":{:.1}{nj}{pj}{bj}",
                ms(median_d(&next)),
                ms(d)
            ),
            &format!(
                "{mb} MB {pattern:?} (一致 {total} 件、全件の走査 {:.1} ms)。保持に無い位置から 次: {nt}。前: {pt}。次の 5 回の連打: {bt}",
                ms(d)
            ),
        );
    }

    // ---- BM-109m-unbounded-16m ----
    {
        let id = "BM-109m-unbounded-16m";
        let pattern = MULTILINE[5].1;
        let limit = 16 << 20;
        let mut data = corpus(limit - (64 << 10), '\n');
        assert!(data.len() <= limit, "{id}: 16 MiB を超えた");
        data.truncate(limit);
        let mut h = open(&data);
        let (alt, n) = product_vs_concat(&mut h, id, pattern, rounds);
        emit(
            json,
            quick,
            id,
            &format!(
                ",\"n\":{rounds},\"matches\":{n},\"bytes\":{}{}",
                data.len(),
                alt.json()
            ),
            &format!(
                "{} バイト {pattern:?}: 一致 {n} 件、{}",
                data.len(),
                alt.text("全文の連結 (参照)")
            ),
        );
    }

    // ---- BM-109m-bigk ----
    {
        let id = "BM-109m-bigk";
        let pattern = r"(?:[^\n]*\n){100}zq";
        let mut h = open(big);
        let (alt, n) = product_vs_concat(&mut h, id, pattern, rounds);
        emit(
            json,
            quick,
            id,
            &format!(",\"n\":{rounds},\"matches\":{n},\"mb\":{mb}{}", alt.json()),
            &format!(
                "{mb} MB {pattern:?}: 一致 {n} 件、{}",
                alt.text("全文の連結")
            ),
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|a| a == "--quick");
    let json = args.iter().any(|a| a == "--json");
    let only: Option<Vec<String>> = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|i| args.get(i + 1))
        .map(|v| v.split(',').map(str::to_owned).collect());
    let want = |g: &str| only.as_ref().is_none_or(|o| o.iter().any(|x| x == g));

    // 行に分かれた文書 (100 MB)
    let mb = if quick { 10 } else { 100 };
    let data = corpus(mb << 20, '\n');
    if want("109") {
        // `k` `s` を含む ASCII の語の大小無視は行ごとの経路になるので、BM-109 では使わない (09 §5.2)
        let mut cases = vec![
            literal("BM-109-ascii", "zebra", Case::Sensitive),
            literal("BM-109-ascii-i", "ZEBRA", Case::Insensitive),
            literal("BM-109-j3", "終わり", Case::Smart),
            literal("BM-109-j10", "終わりの十文字の日本語", Case::Smart),
            literal("BM-109-many", "fox", Case::Sensitive),
        ];
        cases.extend(line_route_cases(""));
        measure(&data, mb, &cases, quick, json);
    }
    if want("compat") {
        let mut h = open(&data);
        bm_109_compat(&mut h, mb, quick, json);
    }

    // 1 行の文書 (10 MB)
    let line_mb = if quick { 1 } else { 10 };
    let line = corpus(line_mb << 20, ' ');
    if want("109") {
        measure(&line, line_mb, &line_route_cases("-1line"), quick, json);
    }
    if want("109m") {
        bm_109m(&data, mb, &line, line_mb, quick, json);
    }
    drop(line);
    if want("ref") {
        m24_reference(&data, mb, quick, json);
    }
}
