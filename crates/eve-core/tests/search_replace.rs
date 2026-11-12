//! すべて置換 (docs/03 §12.5、01 FR-403、04 §8.4): 置換欄・1 Undo 単位・大きな文書のジョブ。

use eve_testkit::Harness;
use eve_testkit::helpers::cell_text;
use eve_view::BarFocus;

fn message(h: &mut Harness) -> String {
    h.frame()
        .messages
        .current
        .map(|m| cell_text(&m.text))
        .unwrap_or_default()
}

/// 検索語と置換文字列を入れた状態にする (検索欄 → Tab → 置換欄)。
fn setup(h: &mut Harness, find: &str, with: &str) {
    h.keys("ctrl+r");
    h.text(find);
    h.keys("tab");
    h.text(with);
}

#[test]
fn ctrl_r_で置換欄つきの検索バーが開き_編集面が_2_行縮む() {
    let mut h = Harness::new().with_size(40, 10);
    let rows = h.frame().size.rows;
    h.keys("ctrl+r");
    let f = h.frame();
    assert_eq!(f.size.rows, rows - 2);
    let b = f.search.unwrap();
    assert!(b.replace.is_some());
    assert_eq!(b.focus, BarFocus::Search, "検索語が空なら検索欄");
    h.text("a");
    h.keys("ctrl+r");
    assert_eq!(h.frame().search.unwrap().focus, BarFocus::Replace);
}

#[test]
fn tab_で_検索欄_置換欄_ボタンを巡回する() {
    let mut h = Harness::new().with_size(40, 10);
    h.keys("ctrl+r");
    let focus = |h: &mut Harness| h.frame().search.unwrap().focus;
    h.keys("tab");
    assert_eq!(focus(&mut h), BarFocus::Replace);
    h.keys("tab");
    assert_eq!(focus(&mut h), BarFocus::Button);
    h.keys("tab");
    assert_eq!(focus(&mut h), BarFocus::Search);
    h.keys("shift+tab");
    assert_eq!(focus(&mut h), BarFocus::Button);
}

#[test]
fn すべて置換は一度の_undo_で戻り_件数を知らせる() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("foo bar foo\nfoo");
    setup(&mut h, "foo", "X");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "X bar X\nX");
    assert_eq!(message(&mut h), "3 件を置換しました (元に戻すで戻せます)");
    assert!(h.is_dirty());
    h.keys("esc");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "foo bar foo\nfoo", "1 回の Undo で全部戻る");
}

#[test]
fn ボタンで_enter_でも置換する() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("aa");
    setup(&mut h, "a", "b");
    h.keys("tab enter");
    assert_eq!(h.doc_text(), "bb");
}

#[test]
fn 置換文字列は展開せず_空なら削除し_日本語も扱う() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("日本語 日本");
    setup(&mut h, "日本", "$1\\n");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "$1\\n語 $1\\n");
    let mut h = Harness::new().with_size(60, 10);
    h.text("a-b-c");
    setup(&mut h, "-", "");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "abc");
}

#[test]
fn 大小の区別は検索バーのトグルに従う() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("Foo foo");
    setup(&mut h, "foo", "x");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "x x", "smart + 小文字は区別しない");
    let mut h = Harness::new().with_size(60, 10);
    h.text("Foo foo");
    h.keys("ctrl+r alt+c alt+c");
    h.keys("alt+c");
    h.keys("alt+c");
    h.text("Foo");
    h.keys("tab");
    h.text("x");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "x foo", "smart + 大文字を含めば区別する");
}

#[test]
fn 検索語が空なら案内し_一致が無ければ文書を変えない() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("abc");
    h.keys("ctrl+r tab");
    h.text("x");
    h.keys("alt+a");
    assert_eq!(message(&mut h), "検索する文字列を入力してください");
    assert_eq!(h.doc_text(), "abc");
    let mut h = Harness::new().with_size(60, 10);
    h.text("abc");
    setup(&mut h, "zzz", "x");
    h.keys("alt+a");
    assert_eq!(message(&mut h), "置換する箇所はありません");
    assert_eq!(h.doc_text(), "abc");
}

#[test]
fn 置換欄が開いていなければ_alt_a_は何もしない() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("aa");
    h.keys("ctrl+f");
    h.text("a");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "aa");
}

#[test]
fn 置換の後も検索バーは開いたままで_再検索する() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("ab ab");
    setup(&mut h, "ab", "abab");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "abab abab");
    let b = h.frame().search.unwrap();
    assert!(cell_text(&b.count_text.unwrap()).ends_with("/4"));
}

fn big_doc() -> String {
    // 1 MiB を超える
    "foo bar baz\n".repeat(90_000)
}

#[test]
fn 大きな文書はジョブで置換し_入力は止めない() {
    let mut h = Harness::new().with_size(60, 10);
    h.paste(&big_doc());
    setup(&mut h, "foo", "FOOO");
    h.keys("alt+a");
    assert!(h.doc_text().starts_with("foo bar"), "まだ置換していない");
    h.run_jobs();
    assert!(h.doc_text().starts_with("FOOO bar baz\nFOOO"));
    assert_eq!(
        message(&mut h),
        "90000 件を置換しました (元に戻すで戻せます)"
    );
    h.keys("esc ctrl+z");
    assert!(h.doc_text().starts_with("foo bar baz\nfoo"));
}

#[test]
fn ジョブの間に文書が変わったら適用せず中止を知らせる() {
    use eve_core::{Origin, Transaction};
    use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, TextRead};
    let mut h = Harness::new().with_size(60, 10);
    h.paste(&big_doc());
    setup(&mut h, "foo", "x");
    h.keys("alt+a");
    // ジョブが走る前に、別の経路 (ここでは API) で文書を変える
    let doc = h.doc_id();
    let cs = {
        let st = h.editor.state();
        let t = st.document(doc).unwrap().text();
        assert!(t.len_bytes() > 1 << 20);
        ChangeSet::from_sorted_edits(
            t,
            [Edit {
                range: ByteRange::new(ByteOff(0), ByteOff(0)),
                insert: "Z".into(),
            }],
        )
        .unwrap()
    };
    h.editor
        .state_mut()
        .apply_transaction(doc, Transaction::new(cs, Origin::User, eve_core::Tick(0)))
        .unwrap();
    h.run_jobs();
    assert!(h.doc_text().starts_with("Zfoo bar"), "置換していない");
    assert_eq!(
        message(&mut h),
        "編集が続いたため、すべて置換を中止しました"
    );
}

#[test]
fn バーを閉じると置換のジョブは取り消される() {
    let mut h = Harness::new().with_size(60, 10);
    h.paste(&big_doc());
    setup(&mut h, "foo", "x");
    h.keys("alt+a");
    h.keys("esc");
    h.run_jobs();
    assert!(h.doc_text().starts_with("foo bar"));
}

// ---- v0.2: 正規表現・単語単位の置換、1 件置換 (03 §12.5) ----

#[test]
fn 正規表現の置換は_キャプチャを展開する() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("foo=1 bar=2");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(r"(\w+)=(\d)");
    h.keys("tab");
    h.text("$2:${1}!");
    // 正規表現の照合器は検索のジョブがコンパイルする (03 §12.3.1)。置換の前にジョブを走らせる
    h.run_jobs();
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "1:foo! 2:bar!");
    assert_eq!(message(&mut h), "2 件を置換しました (元に戻すで戻せます)");
    h.keys("esc");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "foo=1 bar=2", "1 回の Undo で戻る");
}

// 照合器ができる前のすべて置換は、小さい文書でもメインスレッドでコンパイルせず、ジョブで作る (03 §12.3.1)
#[test]
fn 照合器ができる前のすべて置換は_小さい文書でもジョブで作る() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("foo=1 bar=2");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(r"(\w+)=(\d)");
    h.keys("tab");
    h.text("$2:$1");
    h.keys("alt+a");
    assert_eq!(
        h.doc_text(),
        "foo=1 bar=2",
        "ジョブの結果が届くまで変えない"
    );
    h.run_jobs();
    assert_eq!(h.doc_text(), "1:foo 2:bar");
    assert_eq!(message(&mut h), "2 件を置換しました (元に戻すで戻せます)");
    h.keys("esc");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "foo=1 bar=2", "1 回の Undo で戻る");
}

// ジョブのコンパイルで分かった誤り (大きさの上限の超過) は、すべて置換でもメッセージ行と検索バーに示す
#[test]
fn 照合器ができる前のすべて置換で大きすぎる正規表現は理由を示す() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("abc");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text("(?:[a-z]{1000}){1000}");
    h.keys("tab");
    h.text("x");
    h.keys("alt+a");
    h.run_jobs();
    assert_eq!(h.doc_text(), "abc");
    assert!(message(&mut h).contains("pattern too large"));
    let e = cell_text(&h.frame().search.unwrap().error.expect("大きすぎる"));
    assert!(e.contains("pattern too large"), "{e}");
}

#[test]
fn 正規表現の置換は_改行と_ドルを扱う() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("a,b,c");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(",");
    h.keys("tab");
    h.text(r"\n");
    // 正規表現の照合器は検索のジョブがコンパイルする (03 §12.3.1)。置換の前にジョブを走らせる
    h.run_jobs();
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "a\nb\nc", "\n は既定の改行 (LF)");

    let mut h = Harness::new().with_size(60, 10);
    h.text("price 5");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(r"\d");
    h.keys("tab");
    h.text("$$ 100");
    // 正規表現の照合器は検索のジョブがコンパイルする (03 §12.3.1)。置換の前にジョブを走らせる
    h.run_jobs();
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "price $ 100", "$$ は $");

    let mut h = Harness::new().with_size(60, 10);
    h.text("a-b");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text("-");
    h.keys("tab");
    h.text(r"\\n"); // バックスラッシュ 2 つと n
    // 正規表現の照合器は検索のジョブがコンパイルする (03 §12.3.1)。置換の前にジョブを走らせる
    h.run_jobs();
    h.keys("alt+a");
    assert_eq!(
        h.doc_text(),
        r"a\nb",
        "バックスラッシュ 2 つは 1 つになり、n は文字のまま"
    );
}

#[test]
fn 単語単位のすべて置換() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("foo foobar barfoo foo");
    h.keys("ctrl+r");
    h.keys("alt+w");
    h.text("foo");
    h.keys("tab");
    h.text("X");
    // 単語単位の大小無視のリテラルは、エスケープした正規表現で探す (03 §12.4 の 3)。照合器は検索のジョブが作る
    h.run_jobs();
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "X foobar barfoo X");
}

#[test]
fn 置換欄の_enter_は_1_件置換で_次の一致へ進み_置換ごとに_undo_できる() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("a a a");
    h.keys("ctrl+home");
    setup(&mut h, "a", "b");
    h.keys("enter");
    assert_eq!(h.doc_text(), "b a a");
    h.keys("enter");
    assert_eq!(h.doc_text(), "b b a");
    h.keys("enter");
    assert_eq!(h.doc_text(), "b b b");
    assert_eq!(h.frame().search.unwrap().focus, BarFocus::Replace);
    h.keys("enter");
    assert_eq!(message(&mut h), "置換する箇所はありません");
    h.keys("esc");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "b b a", "1 件ずつ戻る");
    h.keys("ctrl+z");
    assert_eq!(h.doc_text(), "b a a");
}

#[test]
fn 検索欄の_enter_は従来どおり次の一致へ移るだけ() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("a a");
    h.keys("ctrl+home");
    setup(&mut h, "a", "b");
    h.keys("shift+tab");
    h.keys("enter");
    assert_eq!(h.doc_text(), "a a", "検索欄では置換しない");
}

#[test]
fn 選択した一致を置換し_正規表現ではキャプチャを展開する() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("k1=v1 k2=v2");
    h.keys("ctrl+home");
    h.keys("ctrl+r");
    h.keys("alt+r");
    h.text(r"(\w+)=(\w+)");
    h.keys("tab");
    h.text("$2=$1");
    h.keys("shift+tab");
    // 正規表現の照合器は検索のジョブがコンパイルし、届いてから入力中の現在の一致を強調する (03 §12.3.1)
    h.run_jobs();
    h.keys("enter"); // 1 つ目の一致を選択
    h.keys("enter"); // 2 つ目へ (選択)
    h.keys("tab");
    h.keys("enter"); // 選択している 2 つ目を置換
    assert_eq!(h.doc_text(), "k1=v1 v2=k2");
}

// ---- 検索語をコンパイルできないときの置換 (04 §8.4) ----

/// 今のメッセージの (レベル, 本文, 対処)。
fn message_parts(h: &mut Harness) -> (eve_view::MessageLevel, String, String) {
    let m = h.frame().messages.current.expect("メッセージ");
    (
        m.level,
        cell_text(&m.text),
        m.hint.map(|t| cell_text(&t)).unwrap_or_default(),
    )
}

/// 検索バーの件数の位置に出ている誤り。
fn bar_error(h: &mut Harness) -> String {
    cell_text(&h.frame().search.unwrap().error.expect("誤り"))
}

#[test]
fn 構文エラーの検索語で置換すると_空の案内ではなく誤りを示し_何も置き換えない() {
    for key in ["alt+a", "enter"] {
        let mut h = Harness::new().with_size(80, 10);
        h.text("a(b a(b");
        h.keys("ctrl+r alt+r");
        h.text("a(b");
        h.keys("tab");
        h.text("x");
        let shown = bar_error(&mut h);
        let reason = shown
            .strip_prefix("✖ 正規表現が不正: ")
            .expect("検索バーの誤り")
            .to_owned();
        h.keys(key);
        assert_eq!(h.doc_text(), "a(b a(b", "{key}: 置き換えない");
        let (level, text, hint) = message_parts(&mut h);
        assert_ne!(text, "検索する文字列を入力してください", "{key}");
        assert_eq!(
            text,
            format!("正規表現が不正なため、置換しませんでした: {reason}"),
            "{key}: 検索バーの誤りと同じ理由"
        );
        assert_eq!(hint, "( ) や [ ] の対応などを確認してください");
        assert_eq!(level, eve_view::MessageLevel::Warn);
    }
}

#[test]
fn 改行を含むパターンで置換すると_未対応と示し_何も置き換えない() {
    for key in ["alt+a", "enter"] {
        let mut h = Harness::new().with_size(80, 10);
        h.text("ab");
        h.keys("ctrl+r alt+r");
        h.text(r"a\nb");
        h.keys("tab");
        h.text("x");
        assert_eq!(bar_error(&mut h), "✖ 改行を含むパターンは未対応です");
        h.keys(key);
        assert_eq!(h.doc_text(), "ab", "{key}: 置き換えない");
        let (_, text, hint) = message_parts(&mut h);
        assert_eq!(
            text, "改行を含むパターンは未対応のため、置換しませんでした",
            "{key}"
        );
        assert_eq!(hint, "行ごとに検索します。1 行の中のパターンにしてください");
    }
}

#[test]
fn 検索語が空なら_1_件置換でも空の案内を出す() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("abc");
    h.keys("ctrl+r tab");
    h.text("x");
    h.keys("enter");
    assert_eq!(message(&mut h), "検索する文字列を入力してください");
    assert_eq!(h.doc_text(), "abc");
}

// ---- 大小を区別しない置換 (03 §12.4、OQ-196) ----

#[test]
fn 大小を区別しない置換は_long_s_とケルビン記号にも一致する() {
    let mut h = Harness::new().with_size(60, 10);
    h.text("class CLASS cla\u{17F}s \u{212A}ick");
    setup(&mut h, "class", "x");
    // `s` を含む大小無視のリテラルは、エスケープした正規表現で探す (03 §12.4 の 3)。照合器は検索のジョブが作る
    h.run_jobs();
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "x x x \u{212A}ick");
    assert_eq!(message(&mut h), "3 件を置換しました (元に戻すで戻せます)");
    let mut h = Harness::new().with_size(60, 10);
    h.text("kick KICK \u{212A}ick");
    setup(&mut h, "kick", "y");
    h.run_jobs();
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "y y y");
}

#[test]
fn 大小を区別しないリテラルの置換は_正規表現の経路で探しても置換文字列を展開しない() {
    // `k` を含む大小無視のリテラルは、エスケープした正規表現で探す (03 §12.4 の 3)。置換文字列はリテラルのまま
    let mut h = Harness::new().with_size(60, 10);
    h.text("Kelvin kelvin");
    h.keys("ctrl+home");
    setup(&mut h, "kelvin", "$1${0}");
    // 照合器 (エスケープした正規表現) は検索のジョブが作る。届くまでの置換欄の Enter は何もしない (03 §12.3.1)
    h.keys("enter");
    assert_eq!(
        h.doc_text(),
        "Kelvin kelvin",
        "照合器が届く前は置き換えない"
    );
    h.run_jobs();
    h.keys("enter");
    assert_eq!(h.doc_text(), "$1${0} kelvin", "1 件置換");
    h.keys("alt+a");
    assert_eq!(h.doc_text(), "$1${0} $1${0}", "すべて置換");
}
