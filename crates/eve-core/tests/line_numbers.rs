//! 行番号 (FR-502、docs/03 §14.7 の LNO-1〜LNO-6、01 §5.5 の FR-502 AC-2) のシナリオ (03 §15.9.1 の L3)。
//! `view.line_numbers` の 3 値 (`absolute` / `relative` / `off`)・相対の数え方と寄せ方・ガターの幅・ペインごとの数え方・
//! 折り返し・`config.reload` での即時の反映・ASCII の Frame のゴールデン (`tests/golden/line_numbers_*.frame.txt`)。

use eve_config::{Config, LineNumberMode};
use eve_testkit::Harness;
use eve_view::{Answer, FocusTarget, Modifiers, MouseKind, PaneFrame, Query, RowSource, ViewId};

/// `1`〜`n` の番号の行を `n` 行。`trailing` なら最後の行の後ろにも改行を置く (論理行は `n + 1`)。
fn numbered(n: usize, trailing: bool) -> Vec<u8> {
    let mut s: String = (1..=n)
        .map(|i| format!("row {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    if trailing {
        s.push('\n');
    }
    s.into_bytes()
}

fn config(mode: LineNumberMode) -> Config {
    Config {
        line_numbers: mode,
        ..Config::default()
    }
}

/// `data` を `/w/a.txt` に置いて開いた `cols` × `rows` の Harness (`view.line_numbers = mode`)。
fn opened(mode: LineNumberMode, data: &[u8], cols: u16, rows: u16) -> Harness {
    let mut h = Harness::builder()
        .size(cols, rows)
        .config(config(mode))
        .file("/w/a.txt", data)
        .build();
    h.open("/w/a.txt");
    h.run_jobs();
    h
}

fn set_mode(h: &mut Harness, mode: LineNumberMode) {
    let c = Config {
        line_numbers: mode,
        ..h.editor.state().config().clone()
    };
    h.editor.state_mut().set_config(c);
}

/// ペインの各行の (論理行と表示行の番号, ガターの文字列)。`Virtual` の行は除く。
fn gutters(p: &PaneFrame) -> Vec<(u32, u32, String)> {
    p.rows
        .iter()
        .filter_map(|r| match r.source {
            RowSource::Line { line, wrap_idx } => Some((
                line,
                wrap_idx,
                r.gutter.iter().map(|s| s.text.as_str()).collect(),
            )),
            _ => None,
        })
        .collect()
}

/// 表示行の番号 0 の行のガターの文字列を、論理行の順に。
fn first_rows(p: &PaneFrame) -> Vec<String> {
    gutters(p)
        .into_iter()
        .filter(|(_, w, _)| *w == 0)
        .map(|(_, _, g)| g)
        .collect()
}

/// 論理行 `line` の最初の表示行のガターが太字か。
fn bold(p: &PaneFrame, line: u32) -> bool {
    p.rows
        .iter()
        .find(|r| matches!(r.source, RowSource::Line { line: l, wrap_idx: 0 } if l == line))
        .and_then(|r| r.gutter.first())
        .is_some_and(|s| s.style.mods.contains(eve_view::StyleMods::BOLD))
}

/// 各行のガターの Span の幅と文字の数が、ペインのガターの幅と一致する (寄せ方で幅が変わらない、LNO-4)。
fn assert_gutter_cells(p: &PaneFrame) {
    for r in &p.rows {
        if matches!(r.source, RowSource::Virtual) {
            continue;
        }
        let w: u16 = r.gutter.iter().map(|s| s.width).sum();
        let chars: usize = r.gutter.iter().map(|s| s.text.chars().count()).sum();
        assert_eq!(w, p.gutter_width, "{:?}", r.source);
        assert_eq!(chars, usize::from(p.gutter_width), "{:?}", r.source);
    }
}

fn focused(h: &Harness) -> ViewId {
    let Ok(Answer::Windows(ws)) = h.query(&Query::Windows) else {
        panic!("Window の一覧");
    };
    let FocusTarget::Pane(f) = ws[0].focus else {
        panic!("フォーカスはペイン");
    };
    f
}

fn down(h: &mut Harness, n: usize) {
    for _ in 0..n {
        h.keys("down");
    }
}

// ---------- LNO-2: 相対の数え方と寄せ方 ----------

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-2)
#[test]
fn relative_は主のカーソルの行に絶対の番号を左寄せで_ほかの行に差を右寄せで出す() {
    let mut h = opened(LineNumberMode::Relative, &numbered(12, true), 40, 16);
    down(&mut h, 4);
    let f = h.frame();
    let p = &f.panes[0];
    // 13 行 (最後の行の区切りの後ろの空の行を含む) なので 2 桁 + 区切り
    assert_eq!(p.gutter_width, 3);
    assert_eq!(
        first_rows(p),
        [
            " 4 ", " 3 ", " 2 ", " 1 ", "5  ", " 1 ", " 2 ", " 3 ", " 4 ", " 5 ", " 6 ", " 7 ",
            " 8 "
        ]
    );
    assert_gutter_cells(p);
    // 現在の行の太字は今のまま (ロールは絶対と同じ)
    assert!(bold(p, 4));
    assert!(!bold(p, 3) && !bold(p, 5));
}

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-2, LNO-5)
#[test]
fn 主のカーソルの行が変わると_見えている全行のガターが変わる() {
    let mut h = opened(LineNumberMode::Relative, &numbered(6, false), 40, 10);
    let before = first_rows(&h.frame().panes[0]);
    assert_eq!(before, ["1  ", " 1 ", " 2 ", " 3 ", " 4 ", " 5 "]);
    h.keys("down");
    let after = first_rows(&h.frame().panes[0]);
    assert_eq!(after, [" 1 ", "2  ", " 1 ", " 2 ", " 3 ", " 4 "]);
    for (i, (a, b)) in before.iter().zip(&after).enumerate() {
        assert_ne!(a, b, "{i} 行目");
    }
    // 本文と Frame の型は変わらない (Span の文字列だけ。LNO-5)
    h.keys("end");
    let f = h.frame();
    assert_eq!(
        f.primary_cursor.expect("カーソル").pos.col,
        3 + 5,
        "本文はガターの後ろから"
    );
}

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-2)
#[test]
fn 複数の_range_では主の_range_の_head_から数える() {
    let mut h = opened(LineNumberMode::Relative, &numbered(8, false), 40, 10);
    // 主のカーソル (最後に足したもの) は 6 行目、ほかのカーソルは 2 行目
    let text = h.doc_text();
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let sel = eve_text::Selection::new(
        vec![
            eve_text::Range::point(eve_text::ByteOff(starts[1])),
            eve_text::Range::point(eve_text::ByteOff(starts[5])),
        ],
        1,
    )
    .expect("2 つの Range");
    h.set_selection(sel);
    let p = &h.frame().panes[0];
    assert_eq!(
        first_rows(p),
        [" 5 ", " 4 ", " 3 ", " 2 ", " 1 ", "6  ", " 1 ", " 2 "]
    );
}

// ---------- LNO-3: どのカーソルから・画面の外 ----------

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-3)
#[test]
fn 主のカーソルが画面の外にあっても_その行からの差を出す() {
    let mut h = opened(LineNumberMode::Relative, &numbered(60, false), 40, 8);
    down(&mut h, 30);
    let v = focused(&h);
    // ホイールで上へ戻す (カーソルは 31 行目のまま、画面は先頭)
    for _ in 0..20 {
        h.mouse_in(v, MouseKind::WheelUp, Modifiers::NONE, 0, 5);
    }
    let f = h.frame();
    let p = &f.panes[0];
    assert!(f.primary_cursor.is_none(), "カーソルは画面の外");
    assert_eq!(
        first_rows(p),
        ["30 ", "29 ", "28 ", "27 ", "26 ", "25 ", "24 ", "23 "]
    );
    assert_gutter_cells(p);
}

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-3), 03 §9.9 (PANE-10)
#[test]
fn ペインごとにそのペインの主のカーソルから数え_フォーカスの無いペインは太字にしない() {
    let mut h = opened(LineNumberMode::Relative, &numbered(10, false), 80, 14);
    down(&mut h, 2);
    let left = focused(&h);
    assert_eq!(
        h.cmd("pane.split_right"),
        eve_core::CommandResult::Done,
        "split"
    );
    let right = focused(&h);
    assert_ne!(left, right);
    down(&mut h, 5);
    let f = h.frame();
    let (l, r) = (&f.panes[0], &f.panes[1]);
    assert_eq!((l.view, r.view), (left, right));
    // 左 (フォーカスの無いペイン) は 3 行目から、右は 8 行目から
    assert_eq!(
        first_rows(l),
        [
            " 2 ", " 1 ", "3  ", " 1 ", " 2 ", " 3 ", " 4 ", " 5 ", " 6 ", " 7 "
        ]
    );
    assert_eq!(
        first_rows(r),
        [
            " 7 ", " 6 ", " 5 ", " 4 ", " 3 ", " 2 ", " 1 ", "8  ", " 1 ", " 2 "
        ]
    );
    // 現在の行の太字はフォーカスのペインだけ。左寄せはどちらにも出る (色と太字に頼らない)
    assert!(bold(r, 7));
    assert!(!bold(l, 2));
    assert_gutter_cells(l);
    assert_gutter_cells(r);
}

// ---------- LNO-4: ガターの幅 ----------

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-4)
#[test]
fn ガターの幅は文書の行数の桁数で決まり_相対の値とカーソルの位置で変わらない() {
    // (行の数, 最後の行の区切り, ガターの幅): 9 → 10 行と 99 → 100 行の境
    for (n, trailing, want) in [
        (9, false, 3),
        (9, true, 3),
        (10, false, 3),
        (99, false, 3),
        (99, true, 4),
        (100, false, 4),
    ] {
        let data = numbered(n, trailing);
        let mut abs = opened(LineNumberMode::Absolute, &data, 60, 120);
        let abs_w = abs.frame().panes[0].gutter_width;
        assert_eq!(abs_w, want, "absolute: {n} {trailing}");
        let mut h = opened(LineNumberMode::Relative, &data, 60, 120);
        let lines = n + usize::from(trailing);
        for k in [0, lines / 2, lines - 1] {
            h.keys("ctrl+home");
            down(&mut h, k);
            let f = h.frame();
            let p = &f.panes[0];
            assert_eq!(
                p.gutter_width, want,
                "relative: {n} {trailing} カーソル {k}"
            );
            assert_gutter_cells(p);
            let g = first_rows(p);
            // 現在の行は左寄せ、いちばん遠い行の差は右寄せ (どちらも幅の中)
            let cur = format!("{:<w$} ", k + 1, w = usize::from(want) - 1);
            assert_eq!(g[k], cur, "{n} {trailing} {k}");
            let far = if k == 0 { lines - 1 } else { 0 };
            let diff = format!("{:>w$} ", far.abs_diff(k), w = usize::from(want) - 1);
            assert_eq!(g[far], diff, "{n} {trailing} {k}");
        }
    }
}

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-4)
#[test]
fn 行が増えて桁が上がると_absolute_と同じく幅が広がる() {
    let mut h = opened(LineNumberMode::Relative, &numbered(9, false), 40, 14);
    assert_eq!(h.frame().panes[0].gutter_width, 3);
    h.keys("ctrl+end");
    h.keys("enter");
    // 10 行は 2 桁のまま (最小の 2 桁)
    assert_eq!(h.frame().panes[0].gutter_width, 3);
    let mut h = opened(LineNumberMode::Relative, &numbered(99, false), 40, 14);
    h.keys("ctrl+end");
    h.keys("enter");
    let f = h.frame();
    assert_eq!(f.panes[0].gutter_width, 4);
    let g = first_rows(&f.panes[0]);
    assert_eq!(g.last().map(String::as_str), Some("100 "));
    assert_gutter_cells(&f.panes[0]);
}

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-2, LNO-4)
#[test]
fn 空の文書と最後の行の区切りの有無() {
    // 空の文書: 1 行だけで、その行が現在の行
    let mut h = opened(LineNumberMode::Relative, b"", 40, 6);
    let p = &h.frame().panes[0];
    assert_eq!((p.gutter_width, first_rows(p)), (3, vec!["1  ".to_owned()]));
    // 区切りで終わる文書の末尾 (空の最後の行) にカーソル
    let mut h = opened(LineNumberMode::Relative, b"a\nb\n", 40, 6);
    h.keys("ctrl+end");
    let p = &h.frame().panes[0];
    assert_eq!(first_rows(p), [" 2 ", " 1 ", "3  "]);
    // 区切りで終わらない文書の末尾は最後の行
    let mut h = opened(LineNumberMode::Relative, b"a\nb", 40, 6);
    h.keys("ctrl+end");
    let p = &h.frame().panes[0];
    assert_eq!(first_rows(p), [" 1 ", "2  "]);
    // off の空の文書はガターが無い
    let mut h = opened(LineNumberMode::Off, b"", 40, 6);
    let p = &h.frame().panes[0];
    assert_eq!(p.gutter_width, 0);
    assert_eq!(p.rows[0].gutter, Vec::new());
}

// ---------- 折り返し ON ----------

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-2), 03 §14.3.1
#[test]
fn 折り返し_on_でも論理行で数え_継続行のガターは空白() {
    let long = "x".repeat(50);
    let data = format!("a\n{long}\nb\nc\n");
    let mut h = Harness::builder()
        .size(30, 10)
        .config(Config {
            line_numbers: LineNumberMode::Relative,
            wrap: eve_config::WrapMode::Soft,
            ..Config::default()
        })
        .file("/w/a.txt", data.as_bytes())
        .build();
    h.open("/w/a.txt");
    h.run_jobs();
    // 4 行目 (`c`) の行頭に置く (折り返し ON の `down` は表示行で動くので、位置を直接置く)
    let c = data.find("\nc").expect("c の行") + 1;
    h.set_selection(eve_text::Selection::point(eve_text::ByteOff(c)));
    let f = h.frame();
    let p = &f.panes[0];
    let g = gutters(p);
    // 2 行目は 2 つ以上の表示行に折れる。差は論理行で数える (継続行を数えない)
    assert_eq!(g[0], (0, 0, " 3 ".to_owned()));
    assert_eq!(g[1], (1, 0, " 2 ".to_owned()));
    assert_eq!(g[2], (1, 1, "   ".to_owned()), "継続行はガターの幅の空白");
    let rest: Vec<&(u32, u32, String)> = g.iter().filter(|(l, _, _)| *l >= 2).collect();
    assert_eq!(rest[0], &(2, 0, " 1 ".to_owned()));
    assert_eq!(rest[1], &(3, 0, "4  ".to_owned()));
    assert_gutter_cells(p);
}

// ---------- LNO-1: off ----------

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-1)
#[test]
fn off_はガターの幅が_0_で_折り返しの継続行にもガターを出さない() {
    let long = "y".repeat(50);
    let data = format!("a\n{long}\n");
    let mut h = Harness::builder()
        .size(30, 10)
        .config(Config {
            line_numbers: LineNumberMode::Off,
            wrap: eve_config::WrapMode::Soft,
            ..Config::default()
        })
        .file("/w/a.txt", data.as_bytes())
        .build();
    h.open("/w/a.txt");
    h.run_jobs();
    let f = h.frame();
    let p = &f.panes[0];
    assert_eq!(p.gutter_width, 0);
    let g = gutters(p);
    assert!(g.iter().any(|(_, w, _)| *w > 0), "継続行がある: {g:?}");
    assert!(g.iter().all(|(_, _, s)| s.is_empty()), "{g:?}");
    // 本文は画面の左端から (折り返し幅は 30 − 右余白 1)
    assert_eq!(p.rows[1].spans.iter().map(|s| s.width).sum::<u16>(), 29);
    // クリックの位置も左端から数える
    h.click(1, 3);
    let f = h.frame();
    assert_eq!(f.primary_cursor.expect("カーソル").pos.col, 3);
}

// ---------- LNO-1: 設定の読み直しで即時に効く ----------

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-1), 07 §6
#[test]
fn config_reload_で即時に効き_表示中の全ペインが描き直る() {
    let mut h = opened(LineNumberMode::Absolute, &numbered(5, false), 80, 10);
    down(&mut h, 1);
    assert_eq!(h.cmd("pane.split_right"), eve_core::CommandResult::Done);
    let abs = h.frame();
    assert_eq!(first_rows(&abs.panes[0])[0], " 1 ");
    h.fs.add_file(
        "/evo/config/config.toml",
        b"[view]\nline_numbers = \"relative\"\n",
    );
    h.clear_log();
    h.cmd("config.reload");
    h.run_jobs();
    assert!(
        h.messages().iter().any(|m| m == "info.config.reloaded"),
        "{:?}",
        h.messages()
    );
    assert_eq!(
        h.editor.state().config().line_numbers,
        LineNumberMode::Relative
    );
    let f = h.frame();
    for p in &f.panes {
        assert_eq!(first_rows(p), [" 1 ", "2  ", " 1 ", " 2 ", " 3 "]);
    }
    h.fs.add_file(
        "/evo/config/config.toml",
        b"[view]\nline_numbers = \"off\"\n",
    );
    h.cmd("config.reload");
    h.run_jobs();
    let f = h.frame();
    for p in &f.panes {
        assert_eq!(p.gutter_width, 0);
        assert!(p.rows.iter().all(|r| r.gutter.is_empty()));
    }
    // 不正な値は警告して absolute
    h.fs.add_file(
        "/evo/config/config.toml",
        b"[view]\nline_numbers = \"number\"\n",
    );
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(
        h.editor.state().config().line_numbers,
        LineNumberMode::Absolute
    );
    let f = h.frame();
    assert_eq!(first_rows(&f.panes[0])[1], " 2 ");
}

// spec: 03 §14.7 (LNO-1), 03 §14.3.1
#[test]
fn 本文の桁数が変わる切り替えでだけ折り返しの表示行を戻し_カーソルを見える範囲に保つ() {
    let long = "z".repeat(200);
    let data = format!("{long}\nb\n");
    let mut h = Harness::builder()
        .size(30, 4)
        .config(Config {
            wrap: eve_config::WrapMode::Soft,
            ..Config::default()
        })
        .file("/w/a.txt", data.as_bytes())
        .build();
    h.open("/w/a.txt");
    h.run_jobs();
    let v = focused(&h);
    let scroll = |h: &Harness| h.editor.state().view(v).expect("View").scroll.wrap_idx;
    // 1 行目の途中の表示行から見せる
    for _ in 0..12 {
        h.keys("down");
    }
    let _ = h.frame();
    let w0 = scroll(&h);
    assert!(w0 > 0, "表示行の途中から: {w0}");
    // absolute → relative は幅が変わらないので保つ
    set_mode(&mut h, LineNumberMode::Relative);
    assert_eq!(scroll(&h), w0);
    // relative → off は本文の桁数が変わるので戻す。カーソルは見えている
    set_mode(&mut h, LineNumberMode::Off);
    let f = h.frame();
    assert_eq!(f.panes[0].gutter_width, 0);
    assert!(f.primary_cursor.is_some(), "カーソルは見える");
    // off → absolute も同じ
    set_mode(&mut h, LineNumberMode::Absolute);
    let f = h.frame();
    assert_eq!(f.panes[0].gutter_width, 3);
    assert!(f.primary_cursor.is_some(), "カーソルは見える");
}

// spec: 03 §14.7 (LNO-1)
#[test]
fn 折り返し_off_の横スクロールで_ガターが戻るとカーソルが見える範囲に保たれる() {
    let long = "w".repeat(100);
    let mut h = opened(LineNumberMode::Off, long.as_bytes(), 30, 4);
    h.keys("end");
    let f = h.frame();
    assert_eq!(f.primary_cursor.expect("カーソル").pos.col, 29);
    set_mode(&mut h, LineNumberMode::Absolute);
    let f = h.frame();
    let c = f.primary_cursor.expect("カーソルは見える").pos;
    assert!(c.col < 30 && c.col >= f.panes[0].gutter_width, "{c:?}");
}

// ---------- LNO-6: ASCII の Frame のゴールデン ----------

fn golden_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.frame.txt"))
}

/// ガター・クローム・カーソルつきの ASCII の Frame を、ゴールデン `name` と比べる。
fn check_frame(h: &mut Harness, name: &str) {
    let text = h.frame_text_with(eve_testkit::AsciiOptions {
        show_gutter: true,
        show_chrome: true,
        ..eve_testkit::AsciiOptions::default()
    });
    eve_testkit::golden::check(&golden_path(name), &text);
}

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-6)
#[test]
fn ascii_の_frame_relative_の_80_桁の画面() {
    let mut h = opened(LineNumberMode::Relative, &numbered(120, true), 80, 14);
    down(&mut h, 5);
    h.keys("end");
    check_frame(&mut h, "line_numbers_relative");
    // 主のカーソルが画面の外 (ホイールで下へ)
    let v = focused(&h);
    for _ in 0..10 {
        h.mouse_in(v, MouseKind::WheelDown, Modifiers::NONE, 0, 5);
    }
    check_frame(&mut h, "line_numbers_relative_offscreen");
}

// verifies: AC-FR-502-2
// spec: 03 §14.7 (LNO-6, LNO-1)
#[test]
fn ascii_の_frame_off_の画面() {
    let long = "long line ".repeat(10);
    let data = format!("first\n{long}\nlast\n");
    let mut h = Harness::builder()
        .size(40, 8)
        .config(Config {
            line_numbers: LineNumberMode::Off,
            wrap: eve_config::WrapMode::Word,
            ..Config::default()
        })
        .file("/w/a.txt", data.as_bytes())
        .build();
    h.open("/w/a.txt");
    h.run_jobs();
    h.keys("down");
    check_frame(&mut h, "line_numbers_off");
}
