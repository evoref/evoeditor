//! spike (M25 の 6): 折り返し ON の長大行 (03 §14.4 の「長大行の折り返し」の試作) が、同じ行を通常の行の経路で
//! 折り返したときと、Frame の行・カーソル・移動のすべてで一致することを確かめる。閾値を 64 KiB に下げた文書 (長大行の経路) と
//! 既定の閾値の文書 (通常の行の経路) に同じキーを送って比べる (03 §14.4 の「正しさの確かめ」と同じ考え方)。

use eve_testkit::Harness;

fn opened(line: &str, threshold: u64) -> Harness {
    let mut data = line.as_bytes().to_vec();
    data.extend_from_slice(b"\nsecond\nthird\n");
    let mut h = Harness::new()
        .with_file("/w/long.txt", &data)
        .with_size(60, 12);
    h.editor.state_mut().set_config(eve_config::Config {
        long_line_threshold: threshold,
        wrap: eve_config::WrapMode::Word,
        ..eve_config::Config::default()
    });
    h.open("/w/long.txt");
    h.run_jobs();
    h
}

/// 比べる中身: 各行の (出どころ, 文字列) と主カーソルの位置と選択。
fn snapshot(h: &mut Harness) -> String {
    let f = h.frame();
    let mut s = String::new();
    for r in &f.panes[0].rows {
        let text: String = r.spans.iter().map(|s| s.text.to_string()).collect();
        let gutter: String = r.gutter.iter().map(|s| s.text.to_string()).collect();
        s.push_str(&format!("{:?} |{gutter}|{text}|\n", r.source));
    }
    s.push_str(&format!(
        "cursor {:?} sel {:?}\n",
        f.primary_cursor.map(|c| c.pos),
        h.selection_range()
    ));
    s
}

#[test]
fn 折り返した長大行は通常の行の経路と同じに表示し_移動する() {
    let unit = "今日は、いい天気です。The quick (brown) fox jumps over the lazy dog. 「引用」\t😀👨\u{200D}👩\u{200D}👧 e\u{301} ";
    let mut line = String::from("  ");
    while line.len() < 80_000 {
        line.push_str(unit);
    }
    let mut long = opened(&line, 65_536);
    let mut normal = opened(&line, 1 << 20);
    assert_eq!(
        long.editor
            .state()
            .document(long.doc_id())
            .unwrap()
            .format
            .long_lines,
        1
    );
    let first = snapshot(&mut long);
    assert!(first.contains("wrap_idx: 3"), "折り返している: {first}");
    assert_eq!(first, snapshot(&mut normal), "開いた直後");
    let mut max_wrap = 0u32;
    let steps: &[&str] = &[
        "pagedown", "pagedown", "down", "end", "end", "home", "home", "up", "up", "pagedown",
        "pagedown", "pagedown", "pagedown", "pagedown", "right", "right", "end", "down", "end",
        "end", "pageup", "pageup", "ctrl+end", "pageup", "up", "up", "home", "ctrl+home",
        "pagedown", "pagedown", "pagedown", "shift+down", "shift+end", "right", "pagedown",
    ];
    for (i, k) in steps.iter().enumerate() {
        long.keys(k);
        normal.keys(k);
        assert_eq!(snapshot(&mut long), snapshot(&mut normal), "{i}: {k}");
        for r in &long.frame().panes[0].rows {
            if let eve_view::RowSource::Line { wrap_idx, .. } = r.source {
                max_wrap = max_wrap.max(wrap_idx);
            }
        }
        if i % 5 == 4 {
            // 編集も混ぜる (変更の位置の手前の記録点から作り直す)
            long.text("xy 、");
            normal.text("xy 、");
            assert_eq!(snapshot(&mut long), snapshot(&mut normal), "{i}: {k} の後の入力");
        }
    }
    // 行の後ろの方 (数百の表示行の先) まで動いた
    assert!(max_wrap > 500, "{max_wrap}");
}
