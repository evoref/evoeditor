//! `Editor` の土台のテスト (docs/02 §5.2 / §5.4、docs/03 §6.5 の手順 7・10): 変更の経路、View の写像、イベント、
//! コマンドの権限と条件。偽 Port は eve-testkit (R12: eve-core の `src/` の中のテストでは使わない)。

use std::rc::Rc;
use std::sync::Arc;

use eve_core::{
    Args, CommandCtx, CommandInvocation, CommandKind, CommandResult, CommandSpec, Document, Editor,
    InvocationSource, Origin, Permission, Ports, ReadOnlyReason, RejectReason, TextFormat, Tick,
    Transaction,
};
use eve_testkit::{FixedPaths, ManualClock, ManualJobSpawner, MemFs};
use eve_text::{ByteOff, ByteRange, ChangeSet, Edit, Range, Selection, Text};
use eve_view::{Capabilities, EditorEvent, OriginTag};

fn editor() -> Editor {
    let clock = ManualClock::new();
    let fs = Arc::new(MemFs::new(Arc::new(clock.clone())));
    let paths = Arc::new(FixedPaths::default());
    let job_ports = eve_core::JobPorts {
        fs: fs.clone(),
        clock: Arc::new(clock.clone()),
        paths: paths.clone(),
    };
    let jobs = ManualJobSpawner::new(job_ports);
    let ports = Ports {
        fs,
        clock: Arc::new(clock),
        jobs: Rc::clone(&jobs) as Rc<dyn eve_core::JobSpawner>,
        paths,
    };
    Editor::new(ports, Capabilities::default())
}

fn doc(s: &str) -> Document {
    Document::from_text(Text::from(s), TextFormat::new_document())
}

fn insert_cs(d: &Document, at: usize, s: &str) -> ChangeSet {
    ChangeSet::from_sorted_edits(
        &d.snapshot(),
        [Edit {
            range: ByteRange::from(at..at),
            insert: s.into(),
        }],
    )
    .unwrap()
}

fn sel(a: usize, h: usize) -> Selection {
    Selection::single(Range::new(ByteOff(a), ByteOff(h)))
}

/// 試験用コマンド: 先頭に "X" を挿入する。
fn handler_x(ctx: &mut CommandCtx<'_>) -> CommandResult {
    let Some(w) = ctx.state.first_window() else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let Some(view) = ctx.state.window_view(w) else {
        return CommandResult::Rejected(RejectReason::NoTarget);
    };
    let doc_id = ctx.state.view(view).unwrap().doc;
    let d = ctx.state.document(doc_id).unwrap();
    let tx = Transaction::new(insert_cs(d, 0, "X"), Origin::User, ctx.state.now()).with_view(view);
    match ctx.state.apply_transaction(doc_id, tx) {
        Ok(_) => CommandResult::Done,
        Err(_) => CommandResult::Rejected(RejectReason::ReadOnly),
    }
}

fn spec(id: &str, perm: Permission, when: &str) -> CommandSpec {
    CommandSpec::new(id, CommandKind::Edit, perm, when, handler_x).unwrap()
}

#[test]
fn 文書を開くと_window_と_view_ができ_opened_が積まれる() {
    let mut e = editor();
    let (doc_id, view_id, win) = e.open_document(doc("hello"));
    assert_eq!(e.state().window_view(win), Some(view_id));
    assert_eq!(e.state().view(view_id).unwrap().doc, doc_id);
    assert_eq!(
        e.drain_events(),
        [EditorEvent::DocumentOpened { doc: doc_id }]
    );
    assert_eq!(e.take_snapshot_updates(), [doc_id]);
    assert!(e.drain_events().is_empty(), "取り出した後は空");
    // 2 つ目の文書は同じ Window を使う
    let (_, _, win2) = e.open_document(doc("b"));
    assert_eq!(win, win2);
}

#[test]
fn 変更は_selection_を写像し_通知を積む() {
    let mut e = editor();
    let (doc_id, a, _) = e.open_document(doc("0123456789"));
    let b = e.state_mut().add_view(doc_id);
    e.state_mut().set_selection(a, sel(2, 4));
    e.state_mut().set_selection(b, sel(5, 7));
    e.drain_events();
    e.tick(Tick(100));
    // View a が位置 0 に "XY" を入力 (発行 View の Selection は指定どおり)
    let d = e.state().document(doc_id).unwrap();
    let tx = Transaction::new(insert_cs(d, 0, "XY"), Origin::User, e.state().now())
        .with_view(a)
        .with_selections(Some(sel(2, 4)), Some(sel(4, 6)));
    let rev = e.state_mut().apply_transaction(doc_id, tx).unwrap();
    assert_eq!(rev.0, 1);
    let st = e.state();
    assert_eq!(
        st.view(a).unwrap().selection,
        sel(4, 6),
        "発行 View は selection_after"
    );
    assert_eq!(
        st.view(b).unwrap().selection,
        sel(7, 9),
        "他の View は写像される"
    );
    let events = e.drain_events();
    assert_eq!(events.len(), 1);
    match &events[0] {
        EditorEvent::DocumentChanged {
            doc,
            revision,
            origin_kind,
        } => {
            assert_eq!((*doc, revision.0), (doc_id, 1));
            assert_eq!(
                (origin_kind.origin, origin_kind.is_undo),
                (OriginTag::User, false)
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(e.take_snapshot_updates(), [doc_id]);
}

#[test]
fn undo_は発行_view_の_selection_を戻し_元の_origin_で通知する() {
    let mut e = editor();
    let (doc_id, v, _) = e.open_document(doc("abc"));
    let d = e.state().document(doc_id).unwrap();
    let tx = Transaction::new(insert_cs(d, 3, "Z"), Origin::Paste, Tick(0))
        .with_view(v)
        .with_selections(Some(sel(3, 3)), Some(sel(4, 4)));
    e.state_mut().apply_transaction(doc_id, tx).unwrap();
    e.drain_events();
    assert!(e.state_mut().undo(doc_id, Some(v)).unwrap());
    assert_eq!(e.state().view(v).unwrap().selection, sel(3, 3));
    match &e.drain_events()[0] {
        EditorEvent::DocumentChanged { origin_kind, .. } => {
            assert_eq!(
                (origin_kind.origin, origin_kind.is_undo),
                (OriginTag::Paste, true)
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(e.state_mut().redo(doc_id, Some(v)).unwrap());
    assert_eq!(e.state().view(v).unwrap().selection, sel(4, 4));
    assert!(!e.state_mut().undo(doc_id, Some(v)).map(|_| false).unwrap());
}

#[test]
fn スクロール位置は行頭へ丸めて写像する() {
    let mut e = editor();
    let (doc_id, v, _) = e.open_document(doc("aaa\nbbb\nccc"));
    e.state_mut().set_scroll_top(v, ByteOff(4));
    let d = e.state().document(doc_id).unwrap();
    // 他の View (発行 View でない) の編集: 先頭に 2 バイト挿入 → 先頭可視行 (2 行目) の行頭は 6
    let tx = Transaction::new(insert_cs(d, 0, "xx"), Origin::User, Tick(0));
    e.state_mut().apply_transaction(doc_id, tx).unwrap();
    assert_eq!(e.state().view(v).unwrap().scroll.top, ByteOff(6));
    // 2 行目の途中に挿入しても、行頭は動かない
    let d = e.state().document(doc_id).unwrap();
    let tx = Transaction::new(insert_cs(d, 8, "yy"), Origin::User, Tick(1));
    e.state_mut().apply_transaction(doc_id, tx).unwrap();
    assert_eq!(e.state().view(v).unwrap().scroll.top, ByteOff(6));
}

#[test]
fn コマンドの実行と条件と権限() {
    let mut e = editor();
    let (doc_id, _v, _) = e.open_document(doc("abc"));
    e.register(spec("t.edit", Permission::Propose, "!doc.read_only"));
    e.register(spec("t.internal", Permission::None, ""));
    let run = |e: &mut Editor, id: &str, src| e.execute(CommandInvocation::new(id, src));

    assert_eq!(
        run(&mut e, "t.edit", InvocationSource::Keymap),
        CommandResult::Done
    );
    assert_eq!(
        e.state().document(doc_id).unwrap().to_string_lossy(),
        "Xabc"
    );
    assert_eq!(
        run(&mut e, "t.nothing", InvocationSource::Keymap),
        CommandResult::Rejected(RejectReason::UnknownCommand)
    );
    // 外部の主体: Permission::None は呼べない、Propose は呼べる
    assert_eq!(
        run(&mut e, "t.internal", InvocationSource::External),
        CommandResult::Rejected(RejectReason::PermissionDenied)
    );
    assert_eq!(
        run(&mut e, "t.internal", InvocationSource::Palette),
        CommandResult::Done
    );
    assert_eq!(
        run(&mut e, "t.edit", InvocationSource::External),
        CommandResult::Done
    );
    // when が偽 (読み取り専用)
    e.state_mut()
        .document_mut(doc_id)
        .unwrap()
        .read_only
        .insert(ReadOnlyReason::UserRequested);
    assert_eq!(
        run(&mut e, "t.edit", InvocationSource::Keymap),
        CommandResult::Rejected(RejectReason::NotAvailable)
    );
    let _ = Args::new();
}
