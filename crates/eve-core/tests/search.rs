//! 文書内検索 (docs/03 §12、01 FR-401・FR-404): ジョブでの全件走査、次へ・前へ、編集後の扱い。

use eve_core::{Case, SearchCount, SearchQuery, SearchStep};
use eve_testkit::Harness;

fn sel(h: &Harness) -> (usize, usize) {
    let id = h.doc_id();
    let _ = id;
    let st = h.editor.state();
    let w = st.first_window().unwrap();
    let v = st.view(st.window_view(w).unwrap()).unwrap();
    let r = v.selection.primary();
    (r.start().0, r.end().0)
}

fn start(h: &mut Harness, q: &str) {
    h.editor.state_mut().search_start(SearchQuery::literal(q));
}

fn step(h: &mut Harness, forward: bool) -> SearchStep {
    h.editor.state_mut().search_step(forward)
}

#[test]
fn 全件を数えて_次へ_前へ_で移り_端で折り返す() {
    let mut h = Harness::new();
    h.text("ab_ab_ab");
    h.keys("ctrl+home");
    start(&mut h, "ab");
    h.run_jobs();
    assert_eq!(
        h.editor.state().search_count(),
        SearchCount::Exact {
            current: None,
            total: 3
        }
    );
    assert_eq!(step(&mut h, true), SearchStep::Moved { wrapped: false });
    assert_eq!(sel(&h), (0, 2));
    assert_eq!(
        h.editor.state().search_count(),
        SearchCount::Exact {
            current: Some(1),
            total: 3
        }
    );
    step(&mut h, true);
    assert_eq!(sel(&h), (3, 5));
    step(&mut h, true);
    assert_eq!(sel(&h), (6, 8));
    assert_eq!(step(&mut h, true), SearchStep::Moved { wrapped: true });
    assert_eq!(sel(&h), (0, 2));
    assert_eq!(step(&mut h, false), SearchStep::Moved { wrapped: true });
    assert_eq!(sel(&h), (6, 8));
    step(&mut h, false);
    assert_eq!(sel(&h), (3, 5));
}

#[test]
fn 走査が終わる前でも_次へ_は同期検索で動く() {
    let mut h = Harness::new();
    h.text("x\nfoo\nx\nfoo");
    h.keys("ctrl+home");
    start(&mut h, "foo");
    assert!(matches!(
        h.editor.state().search_count(),
        SearchCount::Running { .. }
    ));
    assert_eq!(step(&mut h, true), SearchStep::Moved { wrapped: false });
    assert_eq!(sel(&h), (2, 5));
    step(&mut h, true);
    assert_eq!(sel(&h), (8, 11));
    assert_eq!(step(&mut h, true), SearchStep::Moved { wrapped: true });
    assert_eq!(sel(&h), (2, 5));
    assert_eq!(step(&mut h, false), SearchStep::Moved { wrapped: true });
    assert_eq!(sel(&h), (8, 11));
}

#[test]
fn 一致が無ければ_nomatch_で選択を変えない() {
    let mut h = Harness::new();
    h.text("abc");
    start(&mut h, "zzz");
    h.run_jobs();
    let before = sel(&h);
    assert_eq!(step(&mut h, true), SearchStep::NoMatch);
    assert_eq!(sel(&h), before);
    assert_eq!(
        h.editor.state().search_count(),
        SearchCount::Exact {
            current: None,
            total: 0
        }
    );
}

#[test]
fn 検索語が空なら検索しない() {
    let mut h = Harness::new();
    h.text("abc");
    start(&mut h, "");
    assert_eq!(h.editor.state().search_count(), SearchCount::Idle);
    assert_eq!(step(&mut h, true), SearchStep::NoQuery);
}

#[test]
fn 大小の区別は_smart_が既定で_切り替えられる() {
    let mut h = Harness::new();
    h.text("Foo foo");
    start(&mut h, "foo");
    h.run_jobs();
    assert_eq!(
        h.editor.state().search_count(),
        SearchCount::Exact {
            current: None,
            total: 2
        }
    );
    h.editor
        .state_mut()
        .search_start(SearchQuery::literal("foo").with_case(Case::Sensitive));
    h.run_jobs();
    assert_eq!(
        h.editor.state().search_count(),
        SearchCount::Exact {
            current: None,
            total: 1
        }
    );
}

#[test]
fn 編集すると件数は約表示に落ち_次へ_は再検索して動く() {
    let mut h = Harness::new();
    h.text("ab ab");
    h.keys("ctrl+home");
    start(&mut h, "ab");
    h.run_jobs();
    h.keys("end");
    h.text(" ab");
    assert_eq!(
        h.editor.state().search_count(),
        SearchCount::Approx { total: 2 }
    );
    h.keys("ctrl+home");
    assert_eq!(step(&mut h, true), SearchStep::Moved { wrapped: false });
    assert_eq!(sel(&h), (0, 2));
    h.run_jobs();
    assert_eq!(
        h.editor.state().search_count(),
        SearchCount::Exact {
            current: Some(1),
            total: 3
        }
    );
}

#[test]
fn 新しい検索は古い検索の結果を捨てる() {
    let mut h = Harness::new();
    h.text("aa bb");
    start(&mut h, "aa");
    start(&mut h, "bb");
    h.run_jobs();
    assert_eq!(
        h.editor.state().search_count(),
        SearchCount::Exact {
            current: None,
            total: 1
        }
    );
    assert_eq!(h.pending_jobs(), 0);
}

#[test]
fn 検索を止めると状態が消える() {
    let mut h = Harness::new();
    h.text("aa");
    start(&mut h, "aa");
    h.editor.state_mut().search_stop();
    h.run_jobs();
    assert_eq!(h.editor.state().search_count(), SearchCount::Idle);
}

#[test]
fn 日本語と書記素境界() {
    let mut h = Harness::new();
    h.text("日本語e\u{301}e");
    h.keys("ctrl+home");
    start(&mut h, "e");
    h.run_jobs();
    assert_eq!(
        h.editor.state().search_count(),
        SearchCount::Exact {
            current: None,
            total: 1
        }
    );
    assert_eq!(step(&mut h, true), SearchStep::Moved { wrapped: false });
    let (s, e) = sel(&h);
    assert_eq!(e - s, 1);
    assert_eq!(s, 12);
}
