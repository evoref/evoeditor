//! swap-lite の書出しと後始末 (docs/03 §10.2、FR-118 / NFR-307)。`MemFs` の `write_swap` / `discard_swap` で、
//! 予約 (最後の入力から 2 秒後・最短 10 秒間隔)・除外・削除・Port 未対応の扱いを確かめる。

use eve_config::Config;
use eve_core::FsErrorKind;
use eve_testkit::{FsOp, Harness};

/// 打った後の `tick` (Host は各 step の最後に呼ぶ)。
fn typed(h: &mut Harness, s: &str) {
    h.text(s);
    h.advance_ms(0);
}

fn swaps(h: &Harness) -> u32 {
    h.fs.swap_writes()
}

fn opened(path: &str, bytes: &[u8]) -> Harness {
    let mut h = Harness::new().with_file(path, bytes);
    h.open(path);
    h.run_jobs();
    h
}

// verifies: FR-118, NFR-307
#[test]
fn 最後の入力から_2_秒後にスワップが書かれる() {
    let mut h = Harness::new();
    typed(&mut h, "こんにちは");
    h.advance_ms(1999);
    h.run_jobs();
    assert_eq!(swaps(&h), 0, "2 秒の手前では書かない");
    h.advance_ms(1);
    h.run_jobs();
    assert_eq!(swaps(&h), 1);
    let doc = h.doc_id();
    assert_eq!(h.fs.swap_of(doc).unwrap(), "こんにちは".as_bytes());
    assert_eq!(h.fs.swap_path_of(doc), Some(None), "無題の文書");
}

#[test]
fn 入力が続く間は_2_秒の空きまで待つ() {
    let mut h = Harness::new();
    typed(&mut h, "a");
    h.advance_ms(1500);
    typed(&mut h, "b");
    h.advance_ms(1500);
    h.run_jobs();
    assert_eq!(swaps(&h), 0, "最後の入力から 2 秒たっていない");
    h.advance_ms(500);
    h.run_jobs();
    assert_eq!(swaps(&h), 1);
    assert_eq!(h.fs.swap_of(h.doc_id()).unwrap(), b"ab");
}

#[test]
fn 打ち続けても_最短_10_秒で書く() {
    let mut h = Harness::new();
    // 1.5 秒ごとに打ち続けるので、2 秒の空きは来ない
    for _ in 0..7 {
        typed(&mut h, "x");
        h.advance_ms(1500);
        h.run_jobs();
    }
    assert_eq!(swaps(&h), 1, "未書出しの変更が 10 秒続いたので 1 回書く");
    // 書いた後に変更があれば、最短 10 秒の間隔を空けて次を書く
    let n = swaps(&h);
    for _ in 0..6 {
        typed(&mut h, "y");
        h.advance_ms(1500);
        h.run_jobs();
    }
    assert_eq!(swaps(&h), n, "前の書出しから 10 秒たっていない");
    typed(&mut h, "z");
    h.advance_ms(1500);
    h.run_jobs();
    assert_eq!(swaps(&h), n + 1);
}

#[test]
fn 書いた後の変更は_2_秒たっても_最短_10_秒の間隔を空ける() {
    let mut h = Harness::new();
    typed(&mut h, "a");
    h.advance_ms(2000);
    h.run_jobs();
    assert_eq!(swaps(&h), 1);
    typed(&mut h, "b");
    h.advance_ms(5000);
    h.run_jobs();
    assert_eq!(swaps(&h), 1, "前の書出しから 5 秒");
    h.advance_ms(3000);
    h.run_jobs();
    assert_eq!(swaps(&h), 1, "前の書出しから 8 秒");
    h.advance_ms(2000);
    h.run_jobs();
    assert_eq!(swaps(&h), 2, "前の書出しから 10 秒");
    assert_eq!(h.fs.swap_of(h.doc_id()).unwrap(), b"ab");
}

#[test]
fn 変更が無ければ書き直さない() {
    let mut h = Harness::new();
    typed(&mut h, "a");
    h.advance_ms(2000);
    h.run_jobs();
    h.advance_ms(60_000);
    h.run_jobs();
    assert_eq!(swaps(&h), 1);
}

#[test]
fn 書込中の変更は完了後に最新で_1_回だけ書く() {
    let mut h = Harness::new();
    h.run_jobs(); // 起動時の退避の走査
    typed(&mut h, "a");
    h.advance_ms(2000);
    assert_eq!(h.pending_jobs(), 1, "書込のジョブが走り出す直前");
    // 書込中に何度も変更して、期限を過ぎる
    typed(&mut h, "b");
    h.advance_ms(15_000);
    typed(&mut h, "c");
    h.advance_ms(15_000);
    assert_eq!(h.pending_jobs(), 1, "書込中は 2 つ目を投入しない");
    h.run_jobs_once();
    h.advance_ms(0); // Host は結果を処理した後の最後にも tick する
    h.run_jobs();
    assert_eq!(swaps(&h), 2, "完了後に最新で 1 回");
    assert_eq!(h.fs.swap_of(h.doc_id()).unwrap(), b"abc");
}

#[test]
fn 保存するとスワップを消す() {
    let mut h = opened("/w/a.txt", b"x");
    typed(&mut h, "y");
    h.advance_ms(2000);
    h.run_jobs();
    let doc = h.doc_id();
    assert!(h.fs.swap_of(doc).is_some());
    assert_eq!(
        h.fs.swap_path_of(doc),
        Some(h.doc_path().map(|p| eve_core::FsPath::from(p.as_str())))
    );
    h.keys("ctrl+s");
    h.run_jobs();
    h.advance_ms(0);
    h.run_jobs();
    assert!(!h.is_dirty());
    assert_eq!(h.fs.swap_count(), 0, "保存したのでスワップは要らない");
}

#[test]
fn 文書を元の内容に戻して_dirty_でなくなったらスワップを消す() {
    let mut h = opened("/w/a.txt", b"x");
    typed(&mut h, "y");
    h.advance_ms(2000);
    h.run_jobs();
    assert_eq!(h.fs.swap_count(), 1);
    h.keys("ctrl+z");
    h.advance_ms(0);
    h.run_jobs();
    assert!(!h.is_dirty());
    assert_eq!(h.fs.swap_count(), 0);
}

#[test]
fn 上限を超える文書は対象外() {
    let mut h = Harness::new();
    h.editor.state_mut().set_config(Config {
        swap_max_bytes: 1_048_576,
        ..Config::default()
    });
    h.paste(&"a".repeat(1_048_577));
    h.advance_ms(0);
    h.advance_ms(60_000);
    h.run_jobs();
    assert_eq!(swaps(&h), 0);
}

// verifies: FR-118, NFR-307
#[test]
fn 上限を超えたら_対象外であることを_1_回だけ知らせる() {
    let mut h = Harness::new();
    h.editor.state_mut().set_config(Config {
        swap_max_bytes: 1_048_576,
        ..Config::default()
    });
    h.paste(&"a".repeat(1_048_577));
    h.advance_ms(0);
    assert_eq!(
        h.messages()
            .iter()
            .filter(|k| k.as_str() == "msg.swap.too_large")
            .count(),
        1
    );
    h.advance_ms(60_000);
    h.advance_ms(60_000);
    assert_eq!(
        h.messages()
            .iter()
            .filter(|k| k.as_str() == "msg.swap.too_large")
            .count(),
        1,
        "1 回だけ"
    );
    // 上限以下なら知らせない
    let mut h = Harness::new();
    typed(&mut h, "small");
    h.advance_ms(60_000);
    assert!(!h.messages().contains(&"msg.swap.too_large".to_owned()));
}

#[test]
fn 設定で止められる() {
    let mut h = Harness::new();
    h.editor.state_mut().set_config(Config {
        swap_enabled: false,
        ..Config::default()
    });
    typed(&mut h, "a");
    h.advance_ms(60_000);
    h.run_jobs();
    assert_eq!(swaps(&h), 0);
    assert!(h.editor.next_deadline().is_none(), "期限も出さない");
}

#[test]
fn 秘密ファイルは既定で除外し_設定で外せる() {
    let mut h = opened("/w/.env", b"K=1\n");
    typed(&mut h, "S=2\n");
    h.advance_ms(60_000);
    h.run_jobs();
    assert_eq!(swaps(&h), 0, "秘密ファイルパターンに一致");

    let mut h = opened("/w/.env", b"K=1\n");
    h.editor.state_mut().set_config(Config {
        swap_exclude_secret_files: false,
        ..Config::default()
    });
    typed(&mut h, "S=2\n");
    h.advance_ms(2000);
    h.run_jobs();
    assert_eq!(swaps(&h), 1, "除外を止めれば書く");
}

#[test]
fn port_が未対応ならそのセッションでは止める() {
    let mut h = Harness::new();
    h.fs.set_swap_unsupported(true);
    typed(&mut h, "a");
    h.advance_ms(2000);
    h.run_jobs();
    assert_eq!(swaps(&h), 0);
    // 以後はジョブを投入しない
    typed(&mut h, "b");
    h.advance_ms(60_000);
    assert_eq!(h.pending_jobs(), 0);
    assert!(h.editor.next_deadline().is_none());
}

#[test]
fn 書込に失敗したら間隔を空けて書き直す() {
    let mut h = Harness::new();
    h.fs.fail_next(FsOp::WriteSwap, FsErrorKind::DiskFull);
    typed(&mut h, "a");
    h.advance_ms(2000);
    h.run_jobs();
    h.advance_ms(0); // Host は結果を処理した後の最後にも tick する
    assert_eq!(swaps(&h), 0, "失敗した");
    h.advance_ms(9_000);
    h.run_jobs();
    assert_eq!(swaps(&h), 0, "前の試みから 10 秒たっていない");
    h.advance_ms(1_000);
    h.run_jobs();
    assert_eq!(swaps(&h), 1);
}

#[test]
fn 消すのに失敗したら後でやり直す() {
    let mut h = Harness::new();
    typed(&mut h, "a");
    h.advance_ms(2000);
    h.run_jobs();
    assert_eq!(h.fs.swap_count(), 1);
    h.fs.fail_next(FsOp::DiscardSwap, FsErrorKind::Busy);
    h.keys("ctrl+z");
    h.advance_ms(0);
    h.run_jobs();
    assert_eq!(h.fs.swap_count(), 1, "消せなかった");
    h.advance_ms(10_000);
    h.run_jobs();
    assert_eq!(h.fs.swap_count(), 0);
}

#[test]
fn 期限は_dirty_な文書があるときだけ出る() {
    let mut h = Harness::new();
    assert!(h.editor.next_deadline().is_none());
    typed(&mut h, "a");
    assert!(h.editor.next_deadline().is_some());
}
