//! 保存中の終了操作 (docs/03 §9.7 (f)、04 §15.2 `ui.confirm.quit_saving`)。保存ジョブは `ManualJobSpawner` が
//! `run_jobs` まで止めておくので、「保存が終わらない間」の終了を再現できる。

use eve_testkit::Harness;
use eve_view::{Effect, OverlayModel, QuitRescue};

fn ct(t: &eve_view::CellText) -> String {
    t.spans.iter().map(|s| s.text.as_str()).collect()
}

/// a.txt を開いて編集し、保存ジョブを投入した状態 (ジョブはまだ実行していない)。
fn saving() -> Harness {
    let mut h = Harness::new().with_file("/w/a.txt", b"abc");
    h.open("/w/a.txt");
    h.run_jobs();
    h.text("x");
    h.cmd("file.save");
    assert_eq!(h.pending_jobs(), 1, "保存ジョブが実行待ち");
    h
}

fn quit_effect(h: &Harness) -> Option<(u8, QuitRescue, usize)> {
    h.effects().iter().find_map(|e| match e {
        Effect::Quit {
            code,
            rescue,
            stderr,
        } => Some((*code, *rescue, stderr.len())),
        _ => None,
    })
}

#[test]
fn 保存中の終了は確認を出さずに待ち_五秒以内に終われば終了する() {
    let mut h = saving();
    h.cmd("app.quit");
    assert_eq!(h.overlay(), "", "保存の確認は出さない (待つ)");
    assert!(quit_effect(&h).is_none(), "まだ終わらない");
    h.advance_ms(3000);
    assert_eq!(h.overlay(), "", "5 秒以内は確認を出さない");
    h.run_jobs();
    assert_eq!(h.fs_read("/w/a.txt").as_deref(), Some(&b"xabc"[..]));
    assert_eq!(
        quit_effect(&h),
        Some((0, QuitRescue::None, 0)),
        "保存が成功して clean なので終了する"
    );
}

#[test]
fn 五秒を超えたら待つか中止して終了かを尋ね_待つなら保存の完了で終了する() {
    let mut h = saving();
    h.cmd("app.quit");
    h.advance_ms(5000);
    assert_eq!(h.overlay(), "dialog");
    let f = h.frame();
    let OverlayModel::Confirm {
        title,
        message,
        choices,
        ..
    } = &f.overlays[0]
    else {
        panic!("確認ダイアログではない");
    };
    assert_eq!(ct(title), "保存が終わっていません");
    assert!(ct(message).contains("/w/a.txt"), "{}", ct(message));
    let labels: Vec<String> = choices.iter().map(|c| ct(&c.label)).collect();
    assert!(labels[0].starts_with("[W] 待つ"), "{labels:?}");
    assert!(
        labels[1].starts_with("[Q] 保存を中止して終了"),
        "{labels:?}"
    );
    h.answer(0); // 待つ
    assert_eq!(h.overlay(), "");
    h.advance_ms(10_000);
    assert_eq!(h.overlay(), "", "確認は自動では出し直さない");
    h.run_jobs();
    assert_eq!(quit_effect(&h), Some((0, QuitRescue::None, 0)));
}

#[test]
fn もう一度_app_quit_すると確認をすぐに出す() {
    let mut h = saving();
    h.cmd("app.quit");
    h.advance_ms(5000);
    h.answer(0);
    assert_eq!(h.overlay(), "");
    h.cmd("app.quit");
    assert_eq!(h.overlay(), "dialog");
}

#[test]
fn esc_は待つと同じで確認を閉じるだけ() {
    let mut h = saving();
    h.cmd("app.quit");
    h.advance_ms(5000);
    h.keys("esc");
    assert_eq!(h.overlay(), "");
    assert!(quit_effect(&h).is_none());
    h.run_jobs();
    assert!(
        quit_effect(&h).is_some(),
        "要求は取り下げない。完了で終了する"
    );
}

#[test]
fn 保存を中止して終了は取消しを送り_終わったら退避を伴って終了する() {
    let mut h = saving();
    h.cmd("app.quit");
    h.advance_ms(5000);
    h.answer(1); // 保存を中止して終了
    h.run_jobs();
    let (code, rescue, stderr_lines) = quit_effect(&h).expect("終了する");
    assert_eq!(code, 0);
    assert_eq!(
        rescue,
        QuitRescue::DirtyDocuments,
        "dirty な文書の退避は Host が行う"
    );
    assert_eq!(
        stderr_lines, 0,
        "取り消した保存は何も書いていないので行は出さない"
    );
    assert_eq!(
        h.fs_read("/w/a.txt").as_deref(),
        Some(&b"abc"[..]),
        "取り消せたので元のファイルのまま"
    );
}

#[test]
fn 保存中でなければ従来どおり未保存の確認を出す() {
    let mut h = Harness::new().with_file("/w/a.txt", b"abc");
    h.open("/w/a.txt");
    h.run_jobs();
    h.text("x");
    h.cmd("app.quit");
    assert_eq!(h.overlay(), "dialog");
    let f = h.frame();
    let OverlayModel::Confirm { title, .. } = &f.overlays[0] else {
        panic!()
    };
    assert_eq!(ct(title), "未保存の変更");
}

#[test]
fn 保存中の文書が_2_件以上なら_先頭の_1_件に_ほか_n_件_を足す() {
    let mut h = Harness::new()
        .with_file("/w/a.txt", b"abc")
        .with_file("/w/b.txt", b"def");
    h.open("/w/a.txt");
    h.open("/w/b.txt");
    h.run_jobs();
    for p in ["/w/a.txt", "/w/b.txt"] {
        h.open(p);
        h.text("x");
        h.cmd("file.save");
    }
    assert_eq!(h.pending_jobs(), 2);
    h.cmd("app.quit");
    h.advance_ms(5000);
    assert_eq!(h.overlay(), "dialog");
    let f = h.frame();
    let OverlayModel::Confirm { message, .. } = &f.overlays[0] else {
        panic!("確認ダイアログではない");
    };
    let m = ct(message);
    assert!(m.contains("ほか 1 件"), "{m}");
    assert!(m.contains("の保存が終わっていません"), "{m}");
}
