//! 個別キーとして届く大きな貼り付けが、受信口に溜まらないこと (01 FR-206 AC-4、docs/02 §9.2 の Text の集約、04 §6.6)。
//! Windows の入力経路はブラケットペーストを使わず、貼り付けが 1 文字ずつの KeyDown/KeyUp で届く。
//! 正規化器 (`Normalizer`) が連続する文字をバーストとして合成し、受信口に出すメッセージ数が桁違いに減ることを確かめる。

use eve_tui::input::{BURST_N, KeyRec, MAX_TEXT_BYTES, Normalizer, Out, Raw};
use eve_view::InputEvent;

fn tap(vk: u16, ch: u16, out: &mut Vec<Raw>) {
    for down in [true, false] {
        out.push(Raw::Key(KeyRec {
            down,
            repeat: 1,
            vk,
            ch,
            state: 0,
        }));
    }
}

/// 受信口に届いたメッセージ数と、本文の合計 UTF-16 単位数。
fn run(text: &str, read_chars: usize) -> (usize, usize, Vec<InputEvent>) {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut n = Normalizer::new();
    let mut msgs = 0;
    let mut body = 0;
    let mut big = Vec::new();
    let mut take = |outs: Vec<Out>, big: &mut Vec<InputEvent>| {
        for o in outs {
            if let Out::Event(e) = o {
                msgs += 1;
                match &e {
                    InputEvent::Text(s) | InputEvent::Paste(s) => {
                        body += s.encode_utf16().count();
                        if s.len() > MAX_TEXT_BYTES * 2 {
                            big.push(e.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    };
    // 1 回の読取 (ReadConsoleInput) で read_chars 文字ぶんの記録が読める
    for chunk in units.chunks(read_chars) {
        let mut batch = Vec::new();
        for &u in chunk {
            let vk = if u == 0x0D { 0x0D } else { 0x41 };
            tap(vk, u, &mut batch);
        }
        take(n.feed(&batch), &mut big);
    }
    // 貼り付けの終わり (T ms 無入力)
    take(n.close_burst(), &mut big);
    (msgs, body, big)
}

// verifies: AC-FR-206-4
#[test]
fn 十メガバイトの貼り付けでも受信口のメッセージは一万件を超えない() {
    let text = "the quick brown fox 0123456789 日本語\t行\n".repeat(10 * 1024 * 1024 / 40);
    assert!(text.len() >= 10 * 1024 * 1024);
    let units = text.encode_utf16().count();
    // 1 回の読取で BURST_N (16) 文字から 4096 文字まで。それより小さい読取が続く貼り付けはバーストにならない
    // (通常の入力と区別できない既知の制約、04 §6.6)
    for read in [BURST_N, 64, 512, 4096] {
        let (msgs, body, _) = run(&text, read);
        assert!(msgs <= 10_000, "読取 {read} 文字: {msgs} 件");
        assert_eq!(body, units, "読取 {read} 文字: 本文が欠けない");
    }
}

// verifies: AC-FR-206-4
#[test]
fn 制御文字を含まない貼り付けは_64_kib_ごとの_text_にまとまる() {
    let text = "abcdefghij".repeat(10 * 1024 * 1024 / 10);
    let (msgs, body, _) = run(&text, 512);
    assert_eq!(body, text.len());
    let expected = text.len().div_ceil(MAX_TEXT_BYTES);
    assert_eq!(msgs, expected, "{} バイト → {expected} 件", text.len());
}
