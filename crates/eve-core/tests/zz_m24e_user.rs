//! レビュー用 (利用者の視点、M24 の (5) CP932 互換文字の同一視)。画面のモデルの文字列を出して確かめる。
//! `cargo test -p eve-core --test zz_m24e_user -- --nocapture --test-threads=1`

use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;

fn bar_count(h: &mut Harness) -> String {
    let Some(s) = h.frame().search else {
        return "<no bar>".to_owned();
    };
    let c = s.count_text.map(|t| cell_text(&t)).unwrap_or_default();
    let e = s.error.map(|t| cell_text(&t)).unwrap_or_default();
    format!("count=[{c}] error=[{e}]")
}

fn msg(h: &mut Harness) -> String {
    match h.frame().messages.current {
        Some(m) => format!(
            "{} | hint={}",
            cell_text(&m.text),
            m.hint.map(|t| cell_text(&t)).unwrap_or_default()
        ),
        None => "<no message>".to_owned(),
    }
}

fn open(doc: &str, en: bool) -> Harness {
    let mut h = Harness::new().with_size(80, 12);
    if en {
        h = h.with_locale(eve_config::Locale::En);
    }
    let mut h = h.with_file("/w/a.txt", doc.as_bytes());
    h.open("/w/a.txt");
    h.run_jobs();
    h.keys("ctrl+home");
    h
}

#[test]
fn a_件数の印の状態() {
    let mut h = open("a〜b a～b ¥ \\ ~ ～", false);
    h.keys("ctrl+f");
    h.text("〜");
    println!("[A1] 〜 入力直後 (ジョブ前): {}", bar_count(&mut h));
    h.run_jobs();
    println!("[A2] 〜 ジョブ後: {}", bar_count(&mut h));
    for q in ["¥", "\\", "~", "〜〜"] {
        h.keys("ctrl+a");
        h.text(q);
        h.run_jobs();
        println!("[A3] literal {q:?}: {}", bar_count(&mut h));
    }
    h.keys("alt+r");
    for q in [
        r"(?-u:〜)",
        r"\x{301C}",
        r"[〜",
        r".",
        r"[^a]",
        r"a.b",
        r"[〜～]",
        r"\p{Sm}",
        r"[[:^alpha:]]",
    ] {
        h.keys("ctrl+a");
        h.text(q);
        h.run_jobs();
        println!("[A4] regex {q:?}: {}", bar_count(&mut h));
    }
}

#[test]
fn b_置換欄で素早く_enter() {
    let mut h = open("a～b a〜b", false);
    h.keys("ctrl+r");
    h.text("〜");
    h.keys("tab");
    h.text("x");
    h.keys("enter");
    println!(
        "[B1] ジョブ前に Enter: doc={:?} msg={} bar={}",
        h.doc_text(),
        msg(&mut h),
        bar_count(&mut h)
    );
    h.run_jobs();
    h.keys("enter");
    h.run_jobs();
    println!(
        "[B2] ジョブ後に Enter: doc={:?} msg={} bar={} msgs={:?}",
        h.doc_text(),
        msg(&mut h),
        bar_count(&mut h),
        h.messages()
    );
}

#[test]
fn c_英文の_em_dash_を置換すると和文の_horizontal_bar_も変わる() {
    let mut h = open("He said—no.\n彼は――と言った。\n", false);
    h.keys("ctrl+r");
    h.text("—");
    h.run_jobs();
    println!("[C0] bar: {}", bar_count(&mut h));
    h.keys("tab");
    h.text(" - ");
    h.keys("alt+a");
    h.run_jobs();
    println!("[C1] doc={:?}", h.doc_text());
    println!("[C1] msg={}", msg(&mut h));
}

#[test]
fn d_同じ文字にそろえる置換で内容は変わらないとき() {
    let mut h = open("～ ～ ～", false);
    h.keys("ctrl+r");
    h.text("〜");
    h.keys("tab");
    h.text("～");
    h.keys("alt+a");
    h.run_jobs();
    println!(
        "[D1] doc={:?} dirty={} msg={}",
        h.doc_text(),
        h.is_dirty(),
        msg(&mut h)
    );
}

#[test]
fn e_一件置換は知らせが無い() {
    let mut h = open("a〜b a～b", false);
    h.keys("ctrl+r");
    h.text("〜");
    h.run_jobs();
    h.keys("tab");
    h.text("X");
    h.clear_log();
    h.keys("enter");
    h.run_jobs();
    println!(
        "[E1] 1 件目: doc={:?} msg={} msgs={:?} bar={}",
        h.doc_text(),
        msg(&mut h),
        h.messages(),
        bar_count(&mut h)
    );
    h.keys("enter");
    h.run_jobs();
    println!(
        "[E2] 2 件目 (同一視の一致): doc={:?} msg={} msgs={:?}",
        h.doc_text(),
        msg(&mut h),
        h.messages()
    );
}

#[test]
fn f_英語の文言() {
    let mut h = open("a〜\nb～\nc ～", true);
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(r"〜\n");
    h.run_jobs();
    println!("[F0] en bar: {}", bar_count(&mut h));
    h.keys("tab");
    h.text("-");
    h.keys("alt+a");
    h.run_jobs();
    println!("[F1] en msg={}", msg(&mut h));
    let mut h = open("〜 ～", true);
    h.keys("ctrl+r");
    h.text("〜");
    h.keys("tab");
    h.text("~");
    h.keys("alt+a");
    h.run_jobs();
    println!("[F2] en doc={:?} msg={}", h.doc_text(), msg(&mut h));
}

#[test]
fn g_ヘルプの行() {
    let mut h = Harness::new().with_size(80, 40);
    h.keys("ctrl+k h");
    let f = h.frame();
    let Some(eve_view::OverlayModel::Help { lines, .. }) = f.overlays.into_iter().next() else {
        panic!("ヘルプ")
    };
    for l in &lines {
        let t = cell_text(&l.text);
        if t.contains("同一視") || t.contains("newline_sugar") {
            println!("[G] {t}");
        }
    }
}
