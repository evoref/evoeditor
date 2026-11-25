//! 括弧・引用符の自動閉じ (FR-210、docs/03 §15.7 の APR-1〜APR-8、01 §5.5 の FR-210 AC-1 / AC-2) のシナリオと性質
//! (03 §15.9.1 の L0 / L1 / L3。v0.5 の M34 の段 4)。文書と Range の印は `marked` (`multi_cursor.rs` と共有)。

mod marked;

use eve_config::{Conditions, Config, Frontend, Os, load_keymap_str};
use eve_core::{Args, ConfigSources, FsPath, InitialFile, StartupConfig};
use eve_testkit::Harness;
use eve_text::{ByteOff, Range};
use marked::{parse, set_selection, shown};
use proptest::prelude::*;

fn on() -> Config {
    Config {
        auto_pairs: true,
        ..Config::default()
    }
}

/// 設定 `cfg` の無題の文書に印の本文を入れ、印の Range を置いた Harness。
fn scene_with(cfg: Config, marked: &str) -> Harness {
    let (text, ranges, primary) = parse(marked);
    let mut h = Harness::builder().config(cfg).build();
    if !text.is_empty() {
        h.paste(&text);
    }
    assert_eq!(h.doc_text(), text, "本文が入る");
    set_selection(&mut h, ranges, primary);
    h.clear_log();
    h
}

/// 自動閉じを入れた場面。
fn scene(marked: &str) -> Harness {
    scene_with(on(), marked)
}

/// 1 文字ずつ打つ (打鍵の経路の `InputEvent::Text`)。
fn type_each(h: &mut Harness, s: &str) {
    for c in s.chars() {
        h.text(&c.to_string());
    }
}

/// `marked` の場面で `s` を 1 文字ずつ打った様子。
fn typed(marked: &str, s: &str) -> String {
    let mut h = scene(marked);
    type_each(&mut h, s);
    shown(&h)
}

/// Undo のノードの数。
fn nodes(h: &Harness) -> usize {
    h.editor
        .state()
        .document(h.doc_id())
        .expect("文書")
        .history()
        .undo_len()
}

fn revision(h: &Harness) -> eve_view::Revision {
    h.editor
        .state()
        .document(h.doc_id())
        .expect("文書")
        .revision()
}

// ---------- 01 FR-210 AC-1 ----------

// verifies: AC-FR-210-1
#[test]
fn 開き字を打つと閉じ字が足され_カーソルが間に入る() {
    assert_eq!(typed("|", "("), "(|)");
    assert_eq!(typed("|", "["), "[|]");
    assert_eq!(typed("|", "{"), "{|}");
    assert_eq!(typed("|", "\""), "\"|\"");
    assert_eq!(typed("|", "`"), "`|`");
    // `'` と全角の括弧は既定の組に無い (APR-1)
    assert_eq!(typed("|", "'"), "'|");
    assert_eq!(typed("|", "「"), "「|");
    // 1 回の Undo で対ごと戻る
    let mut h = scene("|");
    h.text("(");
    h.cmd("edit.undo");
    assert_eq!(shown(&h), "|");
}

// verifies: AC-FR-210-1
#[test]
fn 対にするのは直後が行末_空白_閉じ字_句読点のときだけ() {
    // APR-2 (c) の各場合 (spec: 03 §15.7 の APR-2)
    let pairs = [
        ("|\nx", "(|)\nx", "行末 (区切りの直前)"),
        ("|", "(|)", "文書の末尾"),
        ("| x", "(|) x", "ASCII の空白"),
        ("|\tx", "(|)\tx", "タブ"),
        ("|\u{3000}x", "(|)\u{3000}x", "全角の空白"),
        ("|)", "(|))", "閉じ字"),
        ("|]", "(|)]", "ほかの組の閉じ字"),
        ("|）", "(|)）", "全角の閉じ括弧"),
        ("|」", "(|)」", "全角の閉じ括弧 (かぎ)"),
        ("|〕", "(|)〕", "全角の閉じ括弧 (亀甲)"),
        ("|,", "(|),", ","),
        ("|;", "(|);", ";"),
        ("|:", "(|):", ":"),
        ("|.", "(|).", "."),
        ("|、", "(|)、", "、"),
        ("|。", "(|)。", "。"),
    ];
    for (start, want, why) in pairs {
        assert_eq!(typed(start, "("), want, "{why}");
    }
    let plain = [
        ("|foo", "(|foo", "語の直前"),
        ("|日本", "(|日本", "漢字の直前"),
        ("|\"", "(|\"", "引用符は閉じ字に数えない"),
        ("|(", "(|(", "開き字の直前"),
        ("|-", "(|-", "ほかの記号"),
    ];
    for (start, want, why) in plain {
        assert_eq!(typed(start, "("), want, "{why}");
    }
}

// verifies: AC-FR-210-1
#[test]
fn 英数字と同じ引用符の直後の引用符は対にしない() {
    // APR-2 (d)
    assert_eq!(typed("don|", "\""), "don\"|", "英字の後");
    assert_eq!(typed("1|", "\""), "1\"|", "数字の後");
    assert_eq!(typed("_|", "\""), "_\"|", "下線は英数字");
    assert_eq!(typed("ａ|", "\""), "ａ\"|", "全角の英数字の後");
    assert_eq!(typed("\"|", "\""), "\"\"|", "同じ引用符の後 (\"\" の後ろ)");
    assert_eq!(typed("x = |", "\""), "x = \"|\"", "空白の後");
    assert_eq!(typed("(|)", "\""), "(\"|\")", "開き字の後");
    assert_eq!(typed("日本|", "\""), "日本\"|\"", "漢字は英数字でない");
    assert_eq!(typed("`|", "\""), "`\"|\"", "ほかの引用符の後");
}

// verifies: AC-FR-210-1
#[test]
fn 自動で足した閉じ字の前で同じ閉じ字を打つと飛び越える() {
    // APR-3
    assert_eq!(typed("|", "()"), "()|");
    assert_eq!(typed("|", "(a)"), "(a)|");
    assert_eq!(typed("|", "([{x}])"), "([{x}])|", "入れ子");
    assert_eq!(typed("|", "\"a\""), "\"a\"|", "引用符");
    assert_eq!(typed("|", "f(\"a\", [1])"), "f(\"a\", [1])|");
    // 違う閉じ字は飛び越えない
    assert_eq!(typed("|", "(]"), "(]|)");
    // 飛び越えた閉じ字の外では、次の閉じ字は入る
    assert_eq!(typed("|", "())"), "())|");
}

// verifies: AC-FR-210-1
#[test]
fn ファイルにあった閉じ字と貼り付けた閉じ字は飛び越えない() {
    // APR-3: 自動で足していない閉じ字
    assert_eq!(typed("foo|)", ")"), "foo)|)");
    assert_eq!(
        typed("|\"", "\""),
        "\"|\"",
        "ファイルにあった引用符: 飛び越えず、直後の引用符は閉じ字に数えないので対にもしない"
    );
    let mut h = scene("|");
    h.paste(")");
    h.keys("left");
    h.text(")");
    assert_eq!(shown(&h), ")|)", "貼り付けた閉じ字");
    // 自動で足した閉じ字の前に貼り付けると、カーソルの直後は自動で足した閉じ字のまま
    let mut h = scene("|");
    h.text("(");
    h.paste(")");
    assert_eq!(shown(&h), "()|)");
    h.text(")");
    assert_eq!(shown(&h), "())|");
}

// verifies: AC-FR-210-1
#[test]
fn カーソルが対の間を離れたら_戻っても忘れる() {
    // APR-3 の覚え方: 対の間の移動は保ち、離れたら忘れる
    let mut h = scene("|");
    type_each(&mut h, "(ab");
    h.keys("left");
    h.keys("right");
    h.text(")");
    assert_eq!(shown(&h), "(ab)|", "対の間の移動では覚えている");
    // 右へ抜けて戻る
    let mut h = scene("|");
    type_each(&mut h, "(a");
    h.keys("end");
    h.keys("left");
    h.text(")");
    assert_eq!(shown(&h), "(a)|)", "離れた後は飛び越えない");
    // 左へ抜けて戻る
    let mut h = scene("|");
    h.text("(");
    h.keys("left");
    h.keys("right");
    h.text(")");
    assert_eq!(shown(&h), "()|)");
    // 改行を入れた (閉じ字が次の行へ移った)
    let mut h = scene("|");
    h.text("(");
    h.keys("enter");
    h.keys("up");
    h.keys("down");
    h.text(")");
    assert_eq!(shown(&h), "(\n)|)");
    // 上の行へ移って戻る
    let mut h = scene("x\n|");
    h.text("(");
    h.keys("up");
    h.keys("down");
    h.keys("end");
    h.keys("left");
    h.text(")");
    assert_eq!(shown(&h), "x\n()|)");
}

// verifies: AC-FR-210-1
#[test]
fn undo_redo_とほかの位置への移動で忘れる() {
    // Undo / Redo
    let mut h = scene("|");
    type_each(&mut h, "(a");
    h.cmd("edit.undo");
    h.cmd("edit.redo");
    h.text(")");
    assert_eq!(shown(&h), "(a)|)", "Redo の後は覚えていない");
    // クリック (同じ位置でも。行番号の幅は 2 桁 + 区切り)
    let mut h = scene("|");
    h.text("(");
    h.click(0, 3 + 1);
    assert_eq!(shown(&h), "(|)");
    h.text(")");
    assert_eq!(shown(&h), "()|)");
    // 行へ移動
    let mut h = scene("|");
    h.text("(");
    h.cmd_with("cursor.goto_line", Args::new().with_int("line", 1));
    h.keys("right");
    h.text(")");
    assert_eq!(shown(&h), "()|)");
}

// verifies: AC-FR-210-1
#[test]
fn 対の間の_backspace_は両方を消し_自動で足していない閉じ字の前では_1_書記素だけ() {
    // APR-4
    let mut h = scene("|");
    h.text("(");
    h.keys("backspace");
    assert_eq!(shown(&h), "|");
    // 中身を消してから対を消す
    let mut h = scene("|");
    type_each(&mut h, "[a");
    h.keys("backspace");
    assert_eq!(shown(&h), "[|]");
    h.keys("backspace");
    assert_eq!(shown(&h), "|");
    // 入れ子は内側から
    let mut h = scene("|");
    type_each(&mut h, "({");
    h.keys("backspace");
    assert_eq!(shown(&h), "(|)");
    h.keys("backspace");
    assert_eq!(shown(&h), "|");
    // ファイルにあった対
    assert_eq!(
        {
            let mut h = scene("(|)");
            h.keys("backspace");
            shown(&h)
        },
        "|)"
    );
    // Delete・単語の削除は変えない
    let mut h = scene("|");
    h.text("(");
    h.keys("delete");
    assert_eq!(shown(&h), "(|");
    let mut h = scene("ab |");
    h.text("(");
    h.cmd("edit.delete_word_backward");
    assert_eq!(shown(&h), "ab |)");
}

// verifies: AC-FR-210-1
#[test]
fn 選択があるときに開き字を打つと選択を囲み_向きを保つ() {
    // APR-5
    assert_eq!(typed("«abc»", "("), "(«abc»)");
    assert_eq!(typed("‹abc›", "["), "[‹abc›]", "後ろ向き");
    assert_eq!(
        typed("«a\nb»", "{"),
        "{«a\nb»}",
        "複数行は全体を 1 回だけ囲む"
    );
    assert_eq!(typed("«ab»c", "\""), "\"«ab»\"c", "引用符も囲む");
    assert_eq!(typed("x«ab»", "\""), "x\"«ab»\"", "英数字の後でも囲む");
    assert_eq!(typed("«ab»", "(["), "([«ab»])", "続けて囲む");
    // 同じ行の囲みの閉じ字は自動で足した閉じ字として覚える (中身を打ち替えて閉じ字を打つと飛び越える)
    assert_eq!(typed("«abc»", "(a)"), "(a)|");
    // 複数行の囲みは覚えない (対の間が同じ行でない)
    assert_eq!(typed("«a\nb»", "(x)"), "(x)|)");
    // 閉じ字・ほかの文字は選択を置き換える
    assert_eq!(typed("«abc»", ")"), ")|");
    assert_eq!(typed("«abc»", "x"), "x|");
    // 囲んだ後に続く文字は中身を置き換える (1 文字ずつでも、1 つの Text でも同じ)
    assert_eq!(typed("«abc»", "(a"), "(a|)");
    let mut h = scene("«abc»");
    h.text("(a");
    assert_eq!(shown(&h), "(a|)");
    // 1 つの Transaction で、新しい Undo のノード (選択の置き換えと同じ)
    let mut h = scene("|abc");
    h.text("x");
    let before = nodes(&h);
    set_selection(&mut h, vec![Range::new(ByteOff(1), ByteOff(4))], 0);
    h.text("(");
    assert_eq!(nodes(&h), before + 1);
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), "xabc");
}

// verifies: AC-FR-210-1
#[test]
fn 複数カーソルでは各カーソルで同じに働く() {
    // APR-6: 対にする・そのまま入れる
    assert_eq!(typed("|\n|x\n«y»", "("), "(|)\n(|x\n(«y»)");
    // 対にする・飛び越える・そのまま入れるが 1 回の打鍵で混ざる
    let mut h = scene("|\n|x");
    h.text("(");
    assert_eq!(shown(&h), "(|)\n(|x");
    h.text(")");
    assert_eq!(shown(&h), "()|\n()|x");
    // 飛び越えだけなら文書を変えない (Transaction を作らない)
    let mut h = scene("|\n|");
    h.text("(");
    let rev = revision(&h);
    h.text(")");
    assert_eq!(shown(&h), "()|\n()|");
    assert_eq!(revision(&h), rev, "Revision が変わらない");
    // 対の間の Backspace も各カーソルで
    let mut h = scene("|\n|");
    h.text("[");
    h.keys("backspace");
    assert_eq!(shown(&h), "|\n|");
    // 1 つの Undo で全カーソルの対が戻る
    let mut h = scene("a|\nb|");
    h.text("(");
    h.cmd("edit.undo");
    assert_eq!(shown(&h), "a|\nb|");
}

// verifies: AC-FR-210-1
#[test]
fn undo_の単位は自動閉じを切ったときと同じ() {
    // APR-7: (a)b を打つと、どちらも 1 つのノードで、1 回の Undo で全部が消える
    for cfg in [on(), Config::default()] {
        let auto = cfg.auto_pairs;
        let mut h = scene_with(cfg, "x|");
        h.advance_ms(5_000);
        let base = nodes(&h);
        type_each(&mut h, "(a)b");
        assert_eq!(h.doc_text(), "x(a)b");
        assert_eq!(nodes(&h), base + 1, "auto_pairs = {auto}");
        h.cmd("edit.undo");
        assert_eq!(h.doc_text(), "x", "auto_pairs = {auto}");
    }
    // Backspace の連打も同じ: 打った後の Backspace は 1 つのノード
    for cfg in [on(), Config::default()] {
        let auto = cfg.auto_pairs;
        let mut h = scene_with(cfg, "|");
        h.advance_ms(5_000);
        type_each(&mut h, "ab(");
        let typed_nodes = nodes(&h);
        for _ in 0..3 {
            h.keys("backspace");
        }
        assert_eq!(h.doc_text(), "", "auto_pairs = {auto}");
        assert_eq!(nodes(&h), typed_nodes + 1, "auto_pairs = {auto}");
    }
    // 飛び越えの後に時間が空けば、続く入力は新しいノード (切ったときの閉じ字の入力と同じ)
    for cfg in [on(), Config::default()] {
        let auto = cfg.auto_pairs;
        let mut h = scene_with(cfg, "|");
        h.advance_ms(5_000);
        let base = nodes(&h);
        h.text("(");
        h.advance_ms(1_500);
        h.text(")");
        h.text("b");
        assert_eq!(h.doc_text(), "()b");
        assert_eq!(nodes(&h), base + 2, "auto_pairs = {auto}");
    }
}

// verifies: AC-FR-210-1
#[test]
fn 読み取り専用の文書は今と同じく編集を断る() {
    // APR-8
    let mut h = Harness::builder()
        .defer_start()
        .config(on())
        .file("/w/a.txt", b"x")
        .build();
    h.editor.open_initial(Some("/w/a.txt"), true);
    h.started();
    h.run_jobs();
    h.clear_log();
    h.text("(");
    assert_eq!(h.doc_text(), "x");
    assert!(
        h.messages().iter().any(|m| m.contains("read_only")),
        "{:?}",
        h.messages()
    );
}

// ---------- 01 FR-210 AC-2 ----------

// verifies: AC-FR-210-2
#[test]
fn 貼り付け_キーマップの_insert_text_パレットでは対にしない() {
    // ブラケットペースト
    let mut h = scene("|");
    h.paste("(");
    assert_eq!(shown(&h), "(|");
    // `edit.paste` (システムのクリップボードの応答)
    let mut h = scene("|");
    h.cmd("edit.paste");
    let token = h
        .effects()
        .iter()
        .rev()
        .find_map(|e| match e {
            eve_view::Effect::RequestClipboard { token, .. } => Some(*token),
            _ => None,
        })
        .expect("RequestClipboard");
    h.clipboard_content(token, Some("("));
    assert_eq!(shown(&h), "(|");
    // キーマップの割当で引数を付けて呼んだ `edit.insert_text`
    let mut h = scene("|");
    let d = h.editor.set_user_keymap(&load_keymap_str(
        "[keys.editor]\n\"f6\" = { command = \"edit.insert_text\", args = { text = \"(\" } }\n",
    ));
    assert!(d.is_empty(), "{d:?}");
    h.keys("f6");
    assert_eq!(shown(&h), "(|");
    // 駆動 API・試験の口からのコマンドの実行 (打鍵の経路でない)
    let mut h = scene("|");
    h.cmd_with("edit.insert_text", Args::new().with_str("text", "("));
    assert_eq!(shown(&h), "(|");
}

// verifies: AC-FR-210-2
#[test]
fn 入力欄では対にしない() {
    // APR-8: 検索バー
    let mut h = scene("|");
    h.keys("ctrl+f");
    h.text("(");
    let input = h.frame().search.expect("検索バー").input;
    let field: String = input.cells.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(field, "(");
    assert_eq!(h.doc_text(), "", "文書は変わらない");
    // パレット
    let mut h = scene("|");
    h.cmd("ui.palette_open");
    h.text("(");
    let f = h.frame();
    let Some(eve_view::OverlayModel::List { input, .. }) = f.overlays.first() else {
        panic!("パレットが出ていない: {:?}", f.overlays)
    };
    assert_eq!(input.len, 1);
    assert_eq!(h.doc_text(), "");
}

// verifies: AC-FR-210-2
#[test]
fn まとめて届いた文字の入力は書記素ごとに当て_打つ速さで結果が変わらない() {
    // APR-2 (a): 1 つの Text で受けても、1 文字ずつ受けても同じ
    for s in ["(a", "()", "f(x)", "\"a\"", "([", "(a)b", "日本(語)"] {
        let mut whole = scene("|");
        whole.text(s);
        assert_eq!(shown(&whole), typed("|", s), "{s:?}");
    }
    // 端末の短い貼り付け (打鍵として届く) の既知の制約 (01 §4.6): 片方だけの括弧は閉じ字が増える
    let mut h = scene("|");
    h.text("(");
    assert_eq!(shown(&h), "(|)");
    // 行をまたぐ貼り付けが打鍵として届くと、改行で飛び越えの覚えが消えるので閉じ字が二重になる (APR-8 (2))
    let mut h = scene("|");
    h.text("{\n  x\n}");
    assert_eq!(shown(&h), "{\n  x\n}|}");
    // `Ctrl+K V` (`edit.paste`) やブラケットペーストで貼れば対にしない
    let mut h = scene("|");
    h.paste("{\n  x\n}");
    assert_eq!(shown(&h), "{\n  x\n}|");
    // IME の確定も同じ経路 (全角の括弧は既定の組に無い)
    let mut h = scene("|");
    h.text("「あ」");
    assert_eq!(shown(&h), "「あ」|");
}

// verifies: AC-FR-210-2
#[test]
fn 既定の_false_では_v0_4_と同じ入力() {
    // 既定の設定の場面 (marked::scene は Config::default())
    let mut h = marked::scene("|");
    type_each(&mut h, "(\"[");
    assert_eq!(shown(&h), "(\"[|");
    h.keys("backspace");
    assert_eq!(shown(&h), "(\"|");
    let mut h = marked::scene("«ab»");
    h.text("(");
    assert_eq!(shown(&h), "(|");
}

// ---------- 組 (APR-1) と設定の重ね方 ----------

const CONFIG: &str = "/evo/config/config.toml";
const LANGS: &str = "/evo/config/languages";

fn linux() -> Conditions {
    Conditions {
        os: Some(Os::Linux),
        frontend: Frontend::Tui,
        ..Conditions::default()
    }
}

/// 設定の読込元を置いて `open` を開いて起動した Harness (language.rs と同じ形)。`ws` はワークスペース層
/// (`/w/repo/.eve/config.toml`)。
fn start(fs: &[(&str, &[u8])], sets: &[&str], ws: Option<&str>, open: &str) -> Harness {
    let mut b = Harness::builder()
        .defer_start()
        .dir("/evo/config")
        .dir("/w/repo/.git");
    for (p, data) in fs {
        b = b.file(p, data);
    }
    if let Some(t) = ws {
        b = b.file("/w/repo/.eve/config.toml", t.as_bytes());
    }
    let mut h = b.build();
    let text = fs
        .iter()
        .find(|(p, _)| *p == CONFIG)
        .map(|(_, d)| String::from_utf8(d.to_vec()).expect("UTF-8"));
    let sets: Vec<String> = sets.iter().map(|s| (*s).to_owned()).collect();
    let r = eve_config::resolve(
        &[eve_config::Source {
            name: "config.toml".to_owned(),
            text: Ok(text),
        }],
        &sets,
        linux(),
    );
    h.editor.configure(StartupConfig {
        config: Some(r.config),
        config_origins: Some(r.origins),
        config_user: Some(r.user),
        config_languages: Some(r.languages),
        config_sources: Some(ConfigSources {
            files: vec![("config.toml".to_owned(), FsPath::new(CONFIG))],
            keymap: None,
            sets,
            themes: None,
            languages: Some(FsPath::new(LANGS)),
            locales: None,
            conditions: linux(),
            no_workspace_config: false,
            home: None,
        }),
        workspace: ws.map(|_| "/w/repo".to_owned()),
        ..Default::default()
    });
    h.editor
        .open_initial_files(&[InitialFile::path(open)], false, None);
    h.started();
    h.run_jobs();
    h
}

/// 文書の末尾で `s` を 1 文字ずつ打った本文 (打つ前の本文を消して比べやすくする)。
fn type_at_end(h: &mut Harness, s: &str) -> String {
    h.keys("ctrl+end");
    let before = h.doc_text();
    type_each(h, s);
    h.doc_text()[before.len()..].to_owned()
}

// verifies: AC-FR-210-1
#[test]
fn 言語定義の_auto_pairs_は既定の組を表ごと置き換え_空の表で止める() {
    let user = b"[editor]\nauto_pairs = true\n";
    // 置き換え: `'` を足して `(` を外した組
    let mut h = start(
        &[
            (CONFIG, user),
            (
                "/evo/config/languages/rust.toml",
                b"[[language]]\nname = \"rust\"\nauto_pairs = { \"'\" = \"'\", \"<\" = \">\" }\n",
            ),
            ("/w/a.rs", b""),
            ("/w/a.txt", b""),
        ],
        &[],
        None,
        "/w/a.rs",
    );
    assert_eq!(type_at_end(&mut h, "("), "(", "組から外した");
    assert_eq!(type_at_end(&mut h, " '"), " ''");
    assert_eq!(type_at_end(&mut h, "<"), "<>");
    // ほかの言語は既定の組のまま
    h.open("/w/a.txt");
    h.run_jobs();
    assert_eq!(type_at_end(&mut h, "("), "()");
    // `{}` でその言語だけ止める
    let mut h = start(
        &[
            (CONFIG, user),
            (
                "/evo/config/languages/md.toml",
                b"[[language]]\nname = \"markdown\"\nauto_pairs = {}\n",
            ),
            ("/w/a.md", b""),
        ],
        &[],
        None,
        "/w/a.md",
    );
    assert_eq!(type_at_end(&mut h, "(\"["), "(\"[");
}

// verifies: AC-FR-210-1
#[test]
fn 言語定義の誤った_auto_pairs_は警告して下の層の値を使う() {
    let mut h = start(
        &[
            (CONFIG, b"[editor]\nauto_pairs = true\n"),
            (
                "/evo/config/languages/a.toml",
                b"[[language]]\nname = \"rust\"\nauto_pairs = { \"((\" = \")\" }\n",
            ),
            ("/w/a.rs", b""),
        ],
        &[],
        None,
        "/w/a.rs",
    );
    assert!(
        h.messages().contains(&"msg.config.problem".to_owned()),
        "{:?}",
        h.messages()
    );
    assert_eq!(type_at_end(&mut h, "("), "()", "既定の組");
    h.cmd("config.show_diagnostics");
    let f = h.frame();
    let eve_view::OverlayModel::List { items, .. } = &f.overlays[0] else {
        panic!("一覧が出ていない")
    };
    let rows: Vec<String> = items
        .iter()
        .map(|i| eve_testkit::helpers::cell_text(&i.label))
        .collect();
    assert!(
        rows.iter()
            .any(|l| l.contains("language.auto_pairs") && l.contains("組が正しくありません")),
        "{rows:?}"
    );
}

// verifies: AC-FR-210-1
#[test]
fn editor_auto_pairs_は設定_ワークスペース層_set_で決まる() {
    let file = ("/w/repo/a.txt", b"".as_slice());
    // 既定は偽
    let mut h = start(&[file], &[], None, "/w/repo/a.txt");
    assert_eq!(type_at_end(&mut h, "("), "(");
    // `--set` で真
    let mut h = start(&[file], &["editor.auto_pairs=true"], None, "/w/repo/a.txt");
    assert_eq!(type_at_end(&mut h, "("), "()");
    // ワークスペース層 (安全な項目、07 §12.3) で真
    let mut h = start(
        &[file],
        &[],
        Some("[editor]\nauto_pairs = true\n"),
        "/w/repo/a.txt",
    );
    assert_eq!(type_at_end(&mut h, "("), "()");
    // ワークスペース層の言語定義の組は、ルートの下の文書に効く
    let mut h = start(
        &[file, (CONFIG, b"[editor]\nauto_pairs = true\n")],
        &[],
        Some("[[language]]\nname = \"rust\"\nfile_types = [\"txt\"]\nauto_pairs = {}\n"),
        "/w/repo/a.txt",
    );
    assert_eq!(type_at_end(&mut h, "("), "(");
    // `config.reload` で即時に効く
    let mut h = start(
        &[file, (CONFIG, b"[editor]\nauto_pairs = false\n")],
        &[],
        None,
        "/w/repo/a.txt",
    );
    assert_eq!(type_at_end(&mut h, "("), "(");
    h.fs.add_file(CONFIG, b"[editor]\nauto_pairs = true\n");
    h.cmd("config.reload");
    h.run_jobs();
    assert_eq!(type_at_end(&mut h, "("), "()");
}

// ---------- レビューで足した試験 (段 4) ----------

// verifies: AC-FR-210-1
#[test]
fn 一万カーソルで対の中を打つときの覚えの写像は覚えと演算の数に比例する() {
    // 1 万行の各行頭にカーソルを置き、`(` で 1 万組を覚えてから中に打つ
    let text = "\n".repeat(9_999);
    let mut h = scene_with(on(), "|");
    h.paste(&text);
    let mut at = Vec::with_capacity(10_000);
    at.push(0);
    at.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let rs: Vec<Range> = at.iter().map(|&p| Range::point(ByteOff(p))).collect();
    set_selection(&mut h, rs, 0);
    h.text("(");
    assert!(h.doc_text().starts_with("()\n()\n"));
    let before = eve_text::map_sorted_steps();
    h.text("a");
    let steps = eve_text::map_sorted_steps() - before;
    // 覚え 1 万 × 4 位置 + 演算 (1 万の挿入の前後で約 2 万) = 約 6 万。覚えごとに演算をたどると 2 億を超える
    assert!(steps > 0, "1 回の走査の写像を使う");
    assert!(steps <= 100_000, "演算と位置の数に比例する: {steps}");
    assert!(h.doc_text().starts_with("(a)\n(a)\n"));
    h.text(")");
    assert!(h.doc_text().starts_with("(a)\n(a)\n"), "どの対も飛び越える");
}

// verifies: AC-FR-210-1
#[test]
fn 開き直しの後には覚えが残らない() {
    let mut h = Harness::builder()
        .defer_start()
        .config(on())
        .file("/w/a.txt", b"")
        .build();
    h.editor.open_initial(Some("/w/a.txt"), false);
    h.started();
    h.run_jobs();
    h.text("(");
    h.cmd("file.save");
    h.run_jobs();
    h.cmd_with(
        "file.reopen_with_encoding",
        Args::new().with_str("encoding", "utf-8"),
    );
    h.run_jobs();
    assert_eq!(h.doc_text(), "()");
    h.keys("ctrl+home right");
    h.text(")");
    assert_eq!(h.doc_text(), "())", "差し替えた本文の閉じ字は飛び越えない");
}

// verifies: AC-FR-210-1
#[test]
fn 対の中で保存してから飛び越えても_dirty_にならない() {
    let mut h = Harness::builder()
        .defer_start()
        .config(on())
        .file("/w/a.txt", b"")
        .build();
    h.editor.open_initial(Some("/w/a.txt"), false);
    h.started();
    h.run_jobs();
    h.text("(a");
    h.cmd("file.save");
    h.run_jobs();
    assert!(!h.is_dirty());
    h.text(")");
    assert_eq!(shown(&h), "(a)|");
    assert!(!h.is_dirty(), "本文を変えない飛び越えは dirty にしない");
}

// verifies: AC-FR-210-1
#[test]
fn 二つのペインで同じ文書を開くと覚えは_view_ごとで_片方の_undo_で両方忘れる() {
    // 覚えは View ごと: B で足した閉じ字を、A の編集の写像で追う
    let mut h = scene("|");
    h.cmd("pane.split_right");
    h.text("(a");
    assert_eq!(shown(&h), "(a|)");
    h.cmd("pane.focus_prev");
    h.keys("ctrl+home");
    h.text("z");
    assert_eq!(h.doc_text(), "z(a)");
    // A は覚えていない (A の中の `)` は入る)
    h.cmd("pane.focus_next");
    h.text(")");
    assert_eq!(h.doc_text(), "z(a)", "B では飛び越える");
    // 片方の Undo で両方の覚えを忘れる
    let mut h = scene("ab|");
    h.cmd("pane.split_right");
    h.text("(");
    assert_eq!(shown(&h), "ab(|)");
    h.cmd("pane.focus_prev");
    h.keys("ctrl+home");
    h.advance_ms(5_000);
    h.text("q");
    h.cmd("edit.undo");
    assert_eq!(h.doc_text(), "ab()");
    h.cmd("pane.focus_next");
    h.text(")");
    assert_eq!(h.doc_text(), "ab())", "Undo の後は B も覚えていない");
}

// verifies: AC-FR-210-1
#[test]
fn 対の中で_tab_を押しても覚えは写像で残る() {
    let mut h = scene("|");
    h.text("(");
    h.keys("tab");
    h.text(")");
    assert_eq!(shown(&h), "(   )|", "次のタブの位置 (4 桁目) まで");
}

// verifies: AC-FR-210-1
#[test]
fn 複数カーソルで選択のある_range_と対の間の_backspace_が混ざる() {
    let mut h = scene("|\n«xy»");
    h.text("(");
    assert_eq!(shown(&h), "(|)\n(«xy»)");
    h.keys("backspace");
    assert_eq!(
        shown(&h),
        "|\n(|)",
        "対の間は両方、選択のある Range は選択を消す"
    );
    h.keys("backspace");
    assert_eq!(shown(&h), "|\n|", "囲んだ閉じ字も覚えている");
}

// verifies: AC-FR-210-1
#[test]
fn 言語の切り替えと設定の読み直しで組と真偽が替わる() {
    let mut h = start(
        &[
            (CONFIG, b"[editor]\nauto_pairs = true\n"),
            (
                "/evo/config/languages/md.toml",
                b"[[language]]\nname = \"markdown\"\nauto_pairs = {}\n",
            ),
            ("/w/a.txt", b""),
        ],
        &[],
        None,
        "/w/a.txt",
    );
    assert_eq!(type_at_end(&mut h, "("), "()");
    h.cmd_with("lang.set", Args::new().with_str("language", "markdown"));
    assert_eq!(type_at_end(&mut h, "["), "[", "markdown では止めてある");
    h.cmd_with("lang.set", Args::new().with_str("language", "plain"));
    // 自動閉じを切ると、覚えていても飛び越えず、対の間の Backspace も 1 文字だけ
    h.keys("ctrl+end");
    h.text("{");
    h.fs.add_file(CONFIG, b"[editor]\nauto_pairs = false\n");
    h.cmd("config.reload");
    h.run_jobs();
    h.text("}");
    assert!(h.doc_text().ends_with("{}}"), "{:?}", h.doc_text());
    h.keys("left backspace");
    assert!(h.doc_text().ends_with("[}}"), "{:?}", h.doc_text());
}

// ---------- 性質 (03 §15.9.1 の L0 / L1) ----------

/// 打鍵の列に使う文字 (組の文字と英数字と空白。改行は含めない)。
fn key_char() -> impl Strategy<Value = char> {
    prop::sample::select(vec![
        '(', ')', '[', ']', '{', '}', '"', '`', 'a', 'b', '1', ' ', '日',
    ])
}

/// 任意の文書 (組の文字・英数字・空白・改行) と、その中の任意の複数の Range。
fn doc_and_ranges() -> impl Strategy<Value = (String, Vec<(usize, usize)>)> {
    let ch = prop::sample::select(vec!['(', ')', '"', 'x', '1', ' ', '\n', '、', '）', 'ａ']);
    prop::collection::vec(ch, 0..24).prop_flat_map(|cs| {
        let text: String = cs.into_iter().collect();
        let n = text.chars().count();
        let ranges = prop::collection::vec((0..=n, 0..=n), 1..4);
        (Just(text), ranges)
    })
}

/// 文字の番号を byte 位置にする。
fn byte_at(text: &str, i: usize) -> usize {
    text.char_indices().nth(i).map_or(text.len(), |(b, _)| b)
}

fn set_ranges(h: &mut Harness, text: &str, ranges: &[(usize, usize)]) {
    let rs: Vec<Range> = ranges
        .iter()
        .map(|(a, b)| Range::new(ByteOff(byte_at(text, *a)), ByteOff(byte_at(text, *b))))
        .collect();
    set_selection(h, rs, 0);
}

/// 釣り合った括弧の文字列 (`(a[b]"c")` など)。
fn balanced() -> impl Strategy<Value = String> {
    let leaf = prop::collection::vec(prop::sample::select(vec!['a', 'b', '1', ' ']), 0..3)
        .prop_map(|v| v.into_iter().collect::<String>());
    leaf.prop_recursive(4, 32, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 1..4).prop_map(|v| v.concat()),
            inner.clone().prop_map(|s| format!("({s})")),
            inner.clone().prop_map(|s| format!("[{s}]")),
            inner.clone().prop_map(|s| format!("{{{s}}}")),
            // 引用符の中身に引用符を入れない。開きの引用符の前は空白にする (英数字の直後の開きの引用符は対にせず (APR-2 (d))、
            // 中身が記号で終わると閉じの引用符のほうを対にするので、切ったときと本文が変わる。§15.9.1 の (b) の例の形)
            inner.prop_map(|s| format!(" \"{}\"", s.replace('"', ""))),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// 03 §15.9.1 の (a): 同じ打鍵の列を任意に区切って `Text` に合体させても、結果の本文と選択が 1 文字ずつ送ったときと同じ。
    // verifies: AC-FR-210-2
    #[test]
    fn 打鍵の列を任意に区切って合体させても結果が同じ(
        (text, ranges) in doc_and_ranges(),
        keys in prop::collection::vec(key_char(), 1..16),
        cuts in prop::collection::vec(any::<bool>(), 16),
    ) {
        let mut one = scene_with(on(), "|");
        one.paste(&text);
        set_ranges(&mut one, &text, &ranges);
        let mut whole = scene_with(on(), "|");
        whole.paste(&text);
        set_ranges(&mut whole, &text, &ranges);
        for k in &keys {
            one.text(&k.to_string());
        }
        let mut chunk = String::new();
        for (i, k) in keys.iter().enumerate() {
            chunk.push(*k);
            if cuts[i] || i + 1 == keys.len() {
                whole.text(&chunk);
                chunk.clear();
            }
        }
        prop_assert_eq!(whole.doc_text(), one.doc_text());
        prop_assert_eq!(shown(&whole), shown(&one));
    }

    /// 03 §15.9.1 の (b): 釣り合った括弧の文字列を打った結果の本文が自動閉じを切って打った結果と同じで、Undo のノードの数も
    /// 同じ (APR-7)。
    // verifies: AC-FR-210-1
    #[test]
    fn 釣り合った括弧を打つと本文と_undo_のノードの数が自動閉じを切ったときと同じ(
        prefix in "[a-z ]{0,4}",
        s in balanced(),
        gaps in prop::collection::vec(prop::sample::select(vec![0u64, 10, 1_000]), 64),
        lines in 1usize..4,
    ) {
        // 各行の末尾に 1 つずつカーソル (複数の Range)
        let marked = vec![format!("{prefix}|"); lines].join("\n");
        let run = |cfg: Config| {
            let mut h = scene_with(cfg, &marked);
            h.advance_ms(5_000);
            for (i, c) in s.chars().enumerate() {
                h.advance_ms(gaps[i % gaps.len()]);
                h.text(&c.to_string());
            }
            (h.doc_text(), nodes(&h))
        };
        let (text_on, nodes_on) = run(on());
        let (text_off, nodes_off) = run(Config::default());
        prop_assert_eq!(&text_on, &text_off);
        prop_assert_eq!(nodes_on, nodes_off, "{:?}", s);
    }

    /// 03 §15.9.1 の (b) の後半: どの打鍵の列 (削除・移動・時間の空きを含む) でも、増えた Undo のノードの数だけ Undo すると
    /// 打つ前の本文に戻る (APR-7。自動閉じの入力がノードを作らずに本文を変えることが無い)。
    // verifies: AC-FR-210-1
    #[test]
    fn 増えたノードの数だけ_undo_すると打つ前の本文に戻る(
        (text, ranges) in doc_and_ranges(),
        keys in prop::collection::vec(
            prop_oneof![
                key_char().prop_map(|c| c.to_string()),
                Just("backspace".to_owned()),
                Just("left".to_owned()),
                Just("right".to_owned()),
                Just("wait".to_owned()),
            ],
            1..24,
        ),
    ) {
        let mut h = scene_with(on(), "|");
        h.paste(&text);
        set_ranges(&mut h, &text, &ranges);
        h.advance_ms(5_000);
        let base = nodes(&h);
        for k in &keys {
            match k.as_str() {
                "backspace" | "left" | "right" => h.keys(k),
                "wait" => h.advance_ms(1_500),
                c => h.text(c),
            }
        }
        for _ in base..nodes(&h) {
            h.cmd("edit.undo");
        }
        prop_assert_eq!(h.doc_text(), text, "{:?}", keys);
    }
}
