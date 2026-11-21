//! ユーザー定義のチョード (docs/01 FR-604 AC-1 / AC-2、02 §7.2 K2 / K3、07 §7.6) の性質テスト。
//!
//! ランダムなユーザーのキーマップ (`keymap.toml` の本文を作って読む) を組込みの既定に重ね、解決器 (`Keymap::resolve`) に
//! キーを押して確かめる:
//! - (a) 曖昧でない任意の割当で、割り当てた列を押すと割り当てたコマンド (引数を含む) が実行される
//! - (b) 保留は時間で切れない (`Harness` で時刻を進める)
//! - (c) 保留中の `Esc` は保留だけを取り消す
//! - (d) 脱出の 3 つ (`app.quit`・`ui.palette_open`・`ui.cancel`) には、どの割当でも既定のキーのどれかで届く
//! - (e) 完全一致とプレフィックスを兼ねる割当がユーザー設定で生じたら、チョードが勝ち、単体の割当は無効になる
//!
//! あわせて、ユーザー設定が無いときの解決が v0.4 の解決器 (この試験に写した参照の実装) と同じであることを確かめる。
//! 件数は `PROPTEST_CASES` で増やせる。

#![allow(clippy::disallowed_methods)]

use eve_config::{DiagKind, KeymapFile, KeymapProblem, load_keymap_str};
use eve_core::{
    Binding, CommandInvocation, CtxValue, Keymap, Resolution, parse_sequence, parse_stroke,
    resolve_keymap,
};
use eve_testkit::Harness;
use eve_view::{KeyCode, KeyStroke, Modifiers};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

/// フォーカスの場所ごとのコンテキストの参照順 (07 §7.1 の表)。
const FOCUSES: [&[&str]; 7] = [
    &["editor", "global"],
    &["search", "prompt", "global"],
    &["prompt", "global"],
    &["palette", "list", "prompt", "global"],
    &["list", "prompt", "global"],
    &["help", "global"],
    &["dialog"],
];

/// 編集面の参照順。
const EDITOR: &[&str] = &["editor", "global"];

/// ユーザーの割当の 1 打目に使うキー (文字キーは 1 打目にできない、07 §7.6)。`ctrl+k` は既定のチョードの先頭。
const FIRST: [&str; 8] = [
    "ctrl+j",
    "ctrl+k",
    "ctrl+s",
    "ctrl+z",
    "f6",
    "f7",
    "alt+x",
    "ctrl+shift+u",
];
/// 2 打目以降に使うキー (保留中は文字キーも照合される、K1)。大文字と `shift+文字` は、端末からは大文字の `Text` で届く。
const NEXT: [&str; 9] = [
    "a", "b", "d", "A", "shift+b", "ctrl+j", "f6", "ctrl+s", "up",
];
/// 割り当てるコマンドと、`args` の TOML の表記 (無ければ空)。
const COMMANDS: [(&str, &str); 6] = [
    ("edit.duplicate_line", ""),
    ("edit.delete_line", ""),
    ("select.all", ""),
    ("edit.insert_text", "text = \"z\""),
    ("cursor.goto_line", "line = 3"),
    ("none", ""),
];

fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn ks(s: &str) -> KeyStroke {
    parse_stroke(s).unwrap_or_else(|e| panic!("{s}: {e}"))
}

fn no_ctx(_: &str) -> Option<CtxValue> {
    None
}

/// ユーザーの割当 1 つ (コンテキスト, キーの書き方, コマンドの番号, `when` (空は常に真))。
#[derive(Clone, Debug)]
struct Entry {
    ctx: &'static str,
    keys: String,
    command: usize,
    when: &'static str,
}

fn keys_of(first: usize, rest: &[usize]) -> String {
    std::iter::once(FIRST[first])
        .chain(rest.iter().map(|&i| NEXT[i]))
        .collect::<Vec<_>>()
        .join(" ")
}

fn entry(contexts: &'static [&'static str]) -> impl Strategy<Value = Entry> {
    (
        proptest::sample::select(contexts.to_vec()),
        0..FIRST.len(),
        proptest::collection::vec(0..NEXT.len(), 0..3),
        0..COMMANDS.len(),
    )
        .prop_map(|(ctx, first, rest, command)| Entry {
            ctx,
            keys: keys_of(first, &rest),
            command,
            when: "",
        })
}

/// 割当の列を `keymap.toml` の本文にする。同じコンテキストの同じ書き方のキーは最初の 1 つだけ (TOML の表は同じキーを
/// 2 回書けない)。
fn toml_of(entries: &[Entry]) -> (String, Vec<Entry>) {
    let mut kept: Vec<Entry> = Vec::new();
    for e in entries {
        if !kept.iter().any(|k| k.ctx == e.ctx && k.keys == e.keys) {
            kept.push(e.clone());
        }
    }
    let mut text = String::new();
    for ctx in ["global", "editor", "prompt", "help", "dialog", "search"] {
        let in_ctx: Vec<&Entry> = kept.iter().filter(|e| e.ctx == ctx).collect();
        if in_ctx.is_empty() {
            continue;
        }
        text.push_str(&format!("[keys.{ctx}]\n"));
        for e in in_ctx {
            let (cmd, args) = COMMANDS[e.command];
            let mut parts = vec![format!("command = \"{cmd}\"")];
            if !args.is_empty() {
                parts.push(format!("args = {{ {args} }}"));
            }
            if !e.when.is_empty() {
                parts.push(format!("when = \"{}\"", e.when));
            }
            if parts.len() == 1 {
                text.push_str(&format!("\"{}\" = \"{cmd}\"\n", e.keys));
            } else {
                text.push_str(&format!("\"{}\" = {{ {} }}\n", e.keys, parts.join(", ")));
            }
        }
    }
    (text, kept)
}

/// 列を順に押し、最後の解決の結果と、途中の結果の列。
fn press(km: &mut Keymap, contexts: &[&str], seq: &[KeyStroke]) -> (Vec<Resolution>, Resolution) {
    let mut out: Vec<Resolution> = seq
        .iter()
        .map(|k| km.resolve(contexts, *k, &no_ctx))
        .collect();
    let last = out.pop().expect("1 打以上");
    (out, last)
}

fn run_id(r: &Resolution) -> Option<&str> {
    match r {
        Resolution::Run(inv) => Some(inv.id.as_str()),
        _ => None,
    }
}

fn invocation(r: &Resolution) -> Option<&CommandInvocation> {
    match r {
        Resolution::Run(inv) => Some(inv),
        _ => None,
    }
}

/// 既定の層のチョードのキーの列 (全コンテキスト)。
fn default_chords() -> Vec<(&'static str, Vec<KeyStroke>)> {
    let (km, _) = resolve_keymap(&KeymapFile::default());
    km.listed_bindings()
        .into_iter()
        .filter(|(_, b)| b.sequence.len() > 1)
        .map(|(_, b)| (b.context, b.sequence.clone()))
        .collect()
}

/// どの割当も、ほかの割当 (ユーザーのものと既定のチョード) のプレフィックスでも延長でもない組だけを残す (曖昧でない割当)。
fn unambiguous(entries: &[Entry], chords: &[(&'static str, Vec<KeyStroke>)]) -> Vec<Entry> {
    let mut free: Vec<Entry> = Vec::new();
    for e in entries {
        let seq = parse_sequence(&e.keys).expect("書けるキー");
        let clash = free.iter().any(|f| {
            let other = parse_sequence(&f.keys).expect("書けるキー");
            other.starts_with(&seq) || seq.starts_with(&other)
        }) || chords
            .iter()
            .any(|(_, c)| c.len() > seq.len() && c.starts_with(&seq));
        if !clash {
            free.push(e.clone());
        }
    }
    free
}

// verifies: AC-FR-604-2
#[test]
fn a_曖昧でない割当は列を押すと割り当てたコマンドになる() {
    let chords = default_chords();
    let mut runner = TestRunner::new(Config {
        cases: cases(256),
        failure_persistence: None,
        ..Config::default()
    });
    let strategy = proptest::collection::vec(entry(&["editor", "global"]), 1..10);
    runner
        .run(&strategy, |entries| {
            let (text, kept) = toml_of(&unambiguous(&entries, &chords));
            let (mut km, diags) = resolve_keymap(&load_keymap_str(&text));
            prop_assert!(
                !diags
                    .iter()
                    .any(|d| d.severity == eve_config::Severity::Error),
                "{text}\n{diags:?}"
            );
            for e in &kept {
                let seq = parse_sequence(&e.keys).expect("書けるキー");
                let (middle, last) = press(&mut km, EDITOR, &seq);
                let (cmd, args) = COMMANDS[e.command];
                if cmd == "none" {
                    // 解除した列を押し切っても何も実行しない (途中の打鍵が保留にならなければ、そのキーの別の割当が
                    // 働くので確かめない。`"none"` のチョードは単体を無効にしない)
                    if middle.iter().all(|r| *r == Resolution::Pending) {
                        prop_assert!(run_id(&last).is_none(), "{} は解除: {last:?}", e.keys);
                    }
                    km.clear_pending();
                    continue;
                }
                prop_assert!(
                    middle.iter().all(|r| *r == Resolution::Pending),
                    "{} の途中: {middle:?}\n{text}",
                    e.keys
                );
                let inv = invocation(&last);
                prop_assert_eq!(
                    inv.map(|i| i.id.as_str()),
                    Some(cmd),
                    "{}\n{}",
                    e.keys,
                    text
                );
                let inv = inv.expect("実行");
                match args {
                    "" => prop_assert!(inv.args.is_empty()),
                    "text = \"z\"" => prop_assert_eq!(inv.args.str("text"), Some("z")),
                    _ => prop_assert_eq!(inv.args.int("line"), Some(3)),
                }
                prop_assert!(km.pending().is_empty(), "実行で保留は消える");
            }
            Ok(())
        })
        .unwrap();
}

// verifies: AC-FR-604-2
#[test]
fn b_保留は時間で切れない() {
    let mut runner = TestRunner::new(Config {
        // 1 件ごとに Harness を作るので、件数は 1/50 (既定 64 件)
        cases: (cases(3200) / 50).max(64),
        failure_persistence: None,
        ..Config::default()
    });
    let strategy = (
        proptest::collection::vec(0..NEXT.len(), 1..4),
        proptest::collection::vec(0u64..3_600_000, 1..4),
    );
    runner
        .run(&strategy, |(rest, waits)| {
            let keys = keys_of(0, &rest);
            let text = format!(
                "[keys.editor]\n\"{keys}\" = {{ command = \"edit.insert_text\", args = {{ text = \"Z\" }} }}\n"
            );
            let mut h = Harness::new();
            let d = h.editor.set_user_keymap(&load_keymap_str(&text));
            prop_assert!(d.is_empty(), "{d:?}");
            let seq: Vec<&str> = keys.split(' ').collect();
            let (last, prefix) = seq.split_last().expect("2 打以上");
            for (i, k) in prefix.iter().enumerate() {
                h.keys(k);
                h.advance_ms(waits[i % waits.len()]);
            }
            prop_assert_eq!(h.doc_text(), "", "途中では何も起きない");
            h.keys(last);
            prop_assert_eq!(h.doc_text(), "Z", "{} を待った後でもチョードが成立する", keys);
            Ok(())
        })
        .unwrap();
}

// verifies: AC-FR-604-1, AC-FR-604-2
#[test]
fn a_端末の_text_で届く_2_打目以降の文字でも割り当てたコマンドになる() {
    // (a) を `Editor` の入力の経路で: 2 打目以降の文字キーは、端末と同じく `Text` (大文字・`shift+文字` は大文字) で送る
    let chords = default_chords();
    let mut runner = TestRunner::new(Config {
        // 1 件ごとに Harness を作るので、件数は 1/50 (既定 64 件)
        cases: (cases(3200) / 50).max(64),
        failure_persistence: None,
        ..Config::default()
    });
    let strategy = proptest::collection::vec(entry(&["editor", "global"]), 1..6);
    runner
        .run(&strategy, |entries| {
            let free = unambiguous(&entries, &chords);
            let mut text = String::new();
            for ctx in ["global", "editor"] {
                let in_ctx: Vec<(usize, &Entry)> =
                    free.iter().enumerate().filter(|(_, e)| e.ctx == ctx).collect();
                if in_ctx.is_empty() {
                    continue;
                }
                text.push_str(&format!("[keys.{ctx}]\n"));
                for (i, e) in in_ctx {
                    text.push_str(&format!(
                        "\"{}\" = {{ command = \"edit.insert_text\", args = {{ text = \"[{i}]\" }} }}\n",
                        e.keys
                    ));
                }
            }
            let mut h = Harness::new();
            let d = h.editor.set_user_keymap(&load_keymap_str(&text));
            prop_assert!(
                !d.iter().any(|d| d.severity == eve_config::Severity::Error),
                "{d:?}\n{text}"
            );
            let mut want = String::new();
            for (i, e) in free.iter().enumerate() {
                for (j, part) in e.keys.split(' ').enumerate() {
                    let k = ks(part);
                    match k.key {
                        KeyCode::Char(c) if j > 0 && k.mods == Modifiers::NONE => {
                            h.text(&c.to_string());
                        }
                        KeyCode::Char(c) if j > 0 && k.mods == Modifiers::SHIFT => {
                            h.text(&c.to_uppercase().to_string());
                        }
                        _ => h.keys(part),
                    }
                }
                want.push_str(&format!("[{i}]"));
                prop_assert_eq!(h.doc_text(), want.clone(), "{}\n{}", e.keys, text);
            }
            Ok(())
        })
        .unwrap();
}

/// (c) の 1 件: `seq` の真のプレフィックスまで押して `Esc` を押すと保留だけが消え、続けて `seq` を押し直すと
/// 最初から押したときと同じ結果になる。
fn esc_cancels_only_pending(km: &mut Keymap, seq: &[KeyStroke], cut: usize) -> Result<(), String> {
    let (_, want) = press(km, EDITOR, seq);
    km.clear_pending();
    for k in &seq[..cut] {
        km.resolve(EDITOR, *k, &no_ctx);
    }
    if km.pending().is_empty() {
        return Ok(()); // 保留にならない列 (単体など)
    }
    let esc = KeyStroke {
        mods: Modifiers::NONE,
        key: KeyCode::Esc,
    };
    let r = km.resolve(EDITOR, esc, &no_ctx);
    if r != Resolution::Cancelled || !km.pending().is_empty() {
        return Err(format!("Esc で {r:?}、保留 {:?}", km.pending()));
    }
    let (_, again) = press(km, EDITOR, seq);
    km.clear_pending();
    if again != want {
        return Err(format!("押し直すと {again:?} (最初は {want:?})"));
    }
    Ok(())
}

// verifies: AC-FR-604-2
#[test]
fn c_保留中の_esc_は保留だけを取り消す() {
    let mut runner = TestRunner::new(Config {
        cases: cases(256),
        failure_persistence: None,
        ..Config::default()
    });
    let strategy = (
        proptest::collection::vec(entry(&["editor", "global"]), 0..8),
        0usize..8,
        0usize..3,
    );
    runner
        .run(&strategy, |(entries, pick, cut)| {
            let (text, kept) = toml_of(&entries);
            let (mut km, _) = resolve_keymap(&load_keymap_str(&text));
            // ユーザーの割当と既定のチョードの列から 1 つ選ぶ
            let mut seqs: Vec<Vec<KeyStroke>> = kept
                .iter()
                .map(|e| parse_sequence(&e.keys).expect("書けるキー"))
                .collect();
            seqs.extend(
                default_chords()
                    .into_iter()
                    .filter(|(c, _)| EDITOR.contains(c))
                    .map(|(_, s)| s),
            );
            let seq = &seqs[pick % seqs.len()];
            let cut = 1 + cut % seq.len().max(1);
            if cut >= seq.len() {
                return Ok(());
            }
            esc_cancels_only_pending(&mut km, seq, cut).map_err(|e| {
                TestCaseError::fail(format!("{e}\n{}\n{text}", eve_core::format_stroke(seq[0])))
            })
        })
        .unwrap();
}

/// 脱出のコマンド (07 §7.6)。
const ESCAPES: [&str; 3] = ["app.quit", "ui.palette_open", "ui.cancel"];

/// `when` の値の組 (`doc.modified` の真偽)。どの組でも届くことを確かめる。
fn valuation(modified: bool) -> impl Fn(&str) -> Option<CtxValue> {
    move |name: &str| (name == "doc.modified").then_some(CtxValue::Bool(modified))
}

/// `km` で、フォーカス `focus` から `cmd` に届く既定のキー (`keys` のどれか) があるか。`when` の値は `modified`。
fn reaches(
    km: &mut Keymap,
    focus: &[&str],
    keys: &[Vec<KeyStroke>],
    cmd: &str,
    modified: bool,
) -> bool {
    let v = valuation(modified);
    keys.iter().any(|seq| {
        let mut last = Resolution::Unbound;
        for k in seq {
            last = km.resolve(focus, *k, &v);
        }
        km.clear_pending();
        run_id(&last) == Some(cmd)
    })
}

// verifies: AC-FR-604-2
#[test]
fn d_脱出の_3_つにはどの割当でもフォーカスごとに届く() {
    // 脱出キーを奪いやすい割当を混ぜる: 脱出キーそのものの付け替え・解除 (when 付きを含む) と、脱出キーを先頭にする
    // チョード
    let escape_keys = ["ctrl+q", "ctrl+p", "f1", "ctrl+shift+p", "esc"];
    let contexts = ["global", "editor", "prompt", "search", "help", "dialog"];
    let whens = ["", "doc.modified", "!doc.modified"];
    let strategy = proptest::collection::vec(
        (
            proptest::sample::select(contexts.to_vec()),
            proptest::sample::select(escape_keys.to_vec()),
            proptest::collection::vec(0..NEXT.len(), 0..2),
            0..COMMANDS.len(),
            proptest::sample::select(whens.to_vec()),
        ),
        0..10,
    );
    let mut runner = TestRunner::new(Config {
        cases: cases(256),
        failure_persistence: None,
        ..Config::default()
    });
    runner
        .run(&strategy, |raw| {
            let entries: Vec<Entry> = raw
                .into_iter()
                .map(|(ctx, first, rest, command, when)| Entry {
                    ctx,
                    keys: std::iter::once(first)
                        .chain(rest.iter().map(|&i| NEXT[i]))
                        .collect::<Vec<_>>()
                        .join(" "),
                    command,
                    when: if COMMANDS[command].0 == "none" {
                        ""
                    } else {
                        when
                    },
                })
                .collect();
            let (text, _) = toml_of(&entries);
            let (mut km, _) = resolve_keymap(&load_keymap_str(&text));
            let (mut base, _) = resolve_keymap(&KeymapFile::default());
            for cmd in ESCAPES {
                for focus in FOCUSES {
                    // そのフォーカスで参照する、cmd の既定のキー
                    let keys: Vec<Vec<KeyStroke>> = base
                        .listed_bindings()
                        .into_iter()
                        .filter(|(_, b)| b.command.as_str() == cmd && focus.contains(&b.context))
                        .map(|(_, b)| b.sequence.clone())
                        .collect();
                    for modified in [false, true] {
                        // 既定のキーマップで届くフォーカスだけを見る
                        if !reaches(&mut base, focus, &keys, cmd, modified) {
                            continue;
                        }
                        prop_assert!(
                            reaches(&mut km, focus, &keys, cmd, modified),
                            "{cmd} に {focus:?} から届かない (doc.modified = {modified})\n{text}"
                        );
                    }
                }
            }
            Ok(())
        })
        .unwrap();
}

// verifies: AC-FR-604-2
#[test]
fn e_曖昧ならチョードが勝ち_単体は無効になる() {
    // 単体 (ユーザーか既定) と、それを先頭にするユーザーのチョード。コンテキストは同時に参照されるもの
    let singles = ["ctrl+j", "ctrl+s", "ctrl+z", "f6", "alt+z", "ctrl+o"];
    let strategy = (
        proptest::sample::select(singles.to_vec()),
        proptest::collection::vec(0..NEXT.len(), 1..3),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    );
    let mut runner = TestRunner::new(Config {
        cases: cases(256),
        failure_persistence: None,
        ..Config::default()
    });
    runner
        .run(
            &strategy,
            |(single, rest, user_single, chord_first, global)| {
                let chord = std::iter::once(single)
                    .chain(rest.iter().map(|&i| NEXT[i]))
                    .collect::<Vec<_>>()
                    .join(" ");
                let chord_ctx = if global { "global" } else { "editor" };
                let mut lines = vec![(chord_ctx, format!("\"{chord}\" = \"edit.duplicate_line\""))];
                if user_single {
                    lines.push(("editor", format!("\"{single}\" = \"edit.delete_line\"")));
                }
                if !chord_first {
                    lines.reverse();
                }
                let mut text = String::new();
                for ctx in ["global", "editor"] {
                    let in_ctx: Vec<&String> = lines
                        .iter()
                        .filter(|(c, _)| *c == ctx)
                        .map(|(_, l)| l)
                        .collect();
                    if !in_ctx.is_empty() {
                        text.push_str(&format!("[keys.{ctx}]\n"));
                        for l in in_ctx {
                            text.push_str(l);
                            text.push('\n');
                        }
                    }
                }
                let (mut km, diags) = resolve_keymap(&load_keymap_str(&text));
                let seq = parse_sequence(&chord).expect("書けるキー");
                // 単体を押すと保留になり、続けて押すとチョードが実行される
                let (_, first) = press(&mut km, EDITOR, &seq[..1]);
                prop_assert_eq!(&first, &Resolution::Pending, "{}", text);
                km.clear_pending();
                let (_, last) = press(&mut km, EDITOR, &seq);
                prop_assert_eq!(run_id(&last), Some("edit.duplicate_line"), "{}", text);
                // 無効になった単体は、使えない割当として出て、診断で名指しされる。ユーザーの単体が無く、既定にも単体が無い
                // キー (ctrl+j・f6) なら、無効になる割当は無い
                let has_single = user_single || !matches!(single, "ctrl+j" | "f6");
                let disabled = km.disabled_bindings();
                let named = |b: &Binding| b.sequence == seq[..1];
                prop_assert_eq!(
                    disabled.iter().any(|d| named(d.binding)),
                    has_single,
                    "{}\n{:?}",
                    text,
                    disabled
                );
                prop_assert!(
                    !km.listed_bindings()
                        .iter()
                        .any(|(_, b)| named(b) && EDITOR.contains(&b.context)),
                    "使える割当の一覧には出さない\n{text}"
                );
                let kinds: Vec<KeymapProblem> = diags
                    .iter()
                    .filter_map(|d| match d.kind {
                        DiagKind::Keymap(p) => Some(p),
                        _ => None,
                    })
                    .collect();
                let want = if user_single {
                    KeymapProblem::UserChordPrefix
                } else {
                    KeymapProblem::ChordShadowsDefault
                };
                if has_single {
                    prop_assert!(kinds.contains(&want), "{want:?} が無い: {kinds:?}\n{text}");
                }
                prop_assert!(
                    diags
                        .iter()
                        .filter(|d| matches!(
                            d.kind,
                            DiagKind::Keymap(
                                KeymapProblem::UserChordPrefix | KeymapProblem::ChordShadowsDefault
                            )
                        ))
                        .all(|d| d.severity == eve_config::Severity::Warning),
                    "操作を問わず警告: {diags:?}"
                );
                Ok(())
            },
        )
        .unwrap();
}

// ---------- ユーザー設定が無いときは v0.4 と同じ (AC-1) ----------

/// v0.4 の解決器 (`Keymap::resolve` を v0.5 の M29 で書き換える前の形) を、既定の層の割当の列に当てたもの。
struct Reference<'a> {
    bindings: Vec<&'a Binding>,
    pending: Vec<KeyStroke>,
}

impl Reference<'_> {
    fn resolve(
        &mut self,
        contexts: &[&str],
        key: KeyStroke,
        lookup: &dyn Fn(&str) -> Option<CtxValue>,
    ) -> Resolution {
        if !self.pending.is_empty() && key.key == KeyCode::Esc && key.mods.is_empty() {
            self.pending.clear();
            return Resolution::Cancelled;
        }
        let mut seq = std::mem::take(&mut self.pending);
        seq.push(key);
        let mut prefix_seen = false;
        for ctx in contexts {
            for b in self.bindings.iter().filter(|b| b.context == *ctx) {
                if !b.sequence.starts_with(&seq) || !b.when.eval(lookup) {
                    continue;
                }
                if b.sequence.len() == seq.len() {
                    if b.command.as_str() == "none" {
                        return Resolution::Unbound;
                    }
                    return Resolution::Run(CommandInvocation {
                        id: b.command.clone(),
                        args: b.args.clone(),
                        source: eve_core::InvocationSource::Keymap,
                    });
                }
                prefix_seen = true;
            }
        }
        if prefix_seen {
            self.pending = seq;
            return Resolution::Pending;
        }
        if seq.len() > 1 {
            Resolution::UnassignedInSequence
        } else {
            Resolution::Unbound
        }
    }
}

// verifies: AC-FR-604-1
#[test]
fn ユーザー設定が無いときの解決は_v0_4_と同じ() {
    let (base, _) = resolve_keymap(&KeymapFile::default());
    let listed: Vec<&Binding> = base.listed_bindings().into_iter().map(|(_, b)| b).collect();
    assert!(base.disabled_bindings().is_empty());
    // 押すキー: 既定の割当に出るストロークのすべてと、割当の無いキー
    let mut pool: Vec<KeyStroke> = listed.iter().flat_map(|b| b.sequence.clone()).collect();
    pool.extend(["z", "esc", "f9", "ctrl+j", "shift+a"].map(ks));
    pool.sort_by_key(|k| format!("{k:?}"));
    pool.dedup();
    let strategy = (
        proptest::collection::vec(0..pool.len(), 1..12),
        0..FOCUSES.len(),
        any::<u64>(),
    );
    let mut runner = TestRunner::new(Config {
        cases: cases(512),
        failure_persistence: None,
        ..Config::default()
    });
    runner
        .run(&strategy, |(keys, focus, bits)| {
            let (mut km, _) = resolve_keymap(&KeymapFile::default());
            let mut old = Reference {
                bindings: listed.clone(),
                pending: Vec::new(),
            };
            // `when` の変数は名前ごとに真偽をばらす
            let lookup = |name: &str| {
                let h = name
                    .bytes()
                    .fold(bits, |a, b| a.rotate_left(5) ^ u64::from(b));
                Some(CtxValue::Bool(h & 1 == 1))
            };
            let contexts = FOCUSES[focus];
            for &i in &keys {
                let k = pool[i];
                let want = old.resolve(contexts, k, &lookup);
                let got = km.resolve(contexts, k, &lookup);
                prop_assert_eq!(&got, &want, "{:?} {:?}", contexts, k);
                prop_assert_eq!(km.pending(), old.pending.as_slice());
            }
            Ok(())
        })
        .unwrap();
}
