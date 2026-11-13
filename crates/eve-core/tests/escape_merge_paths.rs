//! エスケープ文字のつながり (docs/03 §8.3.1) を、公開の経路で確かめる (M23 の 3 本目。データの損失の視点のレビューの
//! 確かめを取り込んだもの)。
//!
//! - 保存のジョブ (`run_job` の `JobKind::Save` と `MemFs`) で、エスケープ文字を元のバイトに戻す保存を乱数で試し、
//!   出力の全体を同じ文字コードで復号した結果を正として突き合わせる (見逃し・誤検出・書いたのに違う出力・一時ファイルの
//!   残り・チャンクの形で違う結果・承認しても書けない、が無いこと)。
//! - 記録 (保持方式) を持つ文書を、製品の経路 (開く → 編集 → `file.save`) で保存し、黙って違う文字で書かない・
//!   `VerifyFailed` にならないこと。
//! - 不正バイトの無い本文の保存は、エスケープの方針に依らず、全体を 1 回で符号化したものと同じ (つながりの検出を
//!   足しても出力が変わらない)。
//!
//! 回数は環境変数 `EVE_ESCAPE_MERGE_CASES` (既定 150)。

use std::sync::Arc;

use eve_core::{
    Args, CancelToken, Document, EscapeMergeApproval, Expect, FsPath, HardlinkPolicy,
    InPlaceBackup, JobKind, JobPorts, JobRequest, JobResult, OpenOptions, Origin, PreservedMap,
    SaveStrategy, TextFormat, Tick, Transaction, run_job,
};
use eve_encoding::{
    Decoder, EncodeReport, Encoder, Encoding, EscapePolicy, NoPreserveReason, PreserveStatus,
    escape_byte,
};
use eve_testkit::{FixedPaths, Harness, ManualClock, MemFs};
use eve_text::{ByteOff, ByteRange, Snapshot, Text, TextBuilder};
use eve_view::{JobId, Revision};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap()
    }
}

#[derive(Clone, Copy, Debug)]
enum Tok {
    E(u8),
    C(char),
}

const ENCS: [Encoding; 5] = [
    Encoding::Utf8,
    Encoding::Utf16Le,
    Encoding::Utf16Be,
    Encoding::ShiftJis,
    Encoding::EucJp,
];

fn n_cases() -> u64 {
    std::env::var("EVE_ESCAPE_MERGE_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(150)
}

fn enc_one(enc: Encoding, s: &str, policy: EscapePolicy) -> (Vec<u8>, EncodeReport) {
    let mut e = Encoder::new(enc, false, policy);
    let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
    e.encode(s, &mut out, &mut rep);
    e.finish(&mut out, &mut rep);
    (out, rep)
}

fn decode(enc: Encoding, bytes: &[u8], skip_bom: bool) -> String {
    let mut d = Decoder::new(enc, skip_bom);
    let mut s = String::new();
    d.decode(bytes, true, &mut s);
    s
}

/// その文字コードで 1 文字ずつ表せて、復号すると戻る文字。
fn alphabet(enc: Encoding) -> Vec<char> {
    let mut cand: Vec<char> = Vec::new();
    cand.extend((0x00u8..0x80).map(char::from));
    cand.extend([
        '\u{80}',
        '\u{A5}',
        '\u{D8}',
        '\u{DC}',
        '\u{FE}',
        '\u{FF}',
        'é',
        'Ø',
        'þ',
        'ÿ',
        '\u{7FF}',
        '\u{800}',
        'あ',
        'い',
        'ア',
        '亜',
        '表',
        '能',
        'ｱ',
        'ｿ',
        'ｻ',
        'ﾟ',
        '＝',
        '　',
        '①',
        '纊',
        '￢',
        '∵',
        '≒',
        '丂',
        '～',
        '\u{E000}',
        '\u{E757}',
        '\u{D7FF}',
        '\u{FEFF}',
        '\u{FFFD}',
        '\u{FFFE}',
        '\u{FFFF}',
        '\u{0A00}',
        '\u{0D00}',
        '😀',
        '\u{10000}',
        '\u{10FEFF}',
        '\u{100000}',
        '\u{FF01}',
        '\u{EFBB}',
        '\u{BBBF}',
        '\u{FFEF}',
        '\u{FEBB}',
    ]);
    // Shift_JIS / EUC-JP の 2 バイト文字を広く (先頭バイトごとにいくつか)
    if matches!(enc, Encoding::ShiftJis | Encoding::EucJp) {
        for lead in 0x81u8..=0xFE {
            for trail in [0x40u8, 0x41, 0x7E, 0x80, 0x9F, 0xA1, 0xBF, 0xEF, 0xFC, 0xFE] {
                let s = decode(enc, &[lead, trail], false);
                let mut it = s.chars();
                if let (Some(c), None) = (it.next(), it.next())
                    && eve_encoding::unescape_char(c).is_none()
                {
                    cand.push(c);
                }
            }
        }
    }
    cand.sort_unstable();
    cand.dedup();
    cand.into_iter()
        .filter(|&c| {
            let (b, rep) = enc_one(enc, &c.to_string(), EscapePolicy::Restore);
            rep.is_empty() && !b.is_empty() && decode(enc, &b, false) == c.to_string()
        })
        .collect()
}

fn tok_text(t: Tok) -> String {
    match t {
        Tok::E(b) => escape_byte(b).to_string(),
        Tok::C(c) => c.to_string(),
    }
}

fn tok_bytes(enc: Encoding, t: Tok) -> Vec<u8> {
    match t {
        Tok::E(b) => vec![b],
        Tok::C(c) => enc_one(enc, &c.to_string(), EscapePolicy::Restore).0,
    }
}

/// 同じ本文を、いくつかのチャンクの形で作る。
fn snapshot_shapes(text: &str, rng: &mut Rng) -> Vec<Snapshot> {
    let mut v = vec![Text::from(text).snapshot()];
    let cs: Vec<usize> = text
        .char_indices()
        .map(|c| c.0)
        .chain([text.len()])
        .collect();
    // TextBuilder に小さく分けて足す
    let mut b = TextBuilder::new();
    let mut i = 0;
    while i + 1 < cs.len() {
        let j = (i + 1 + rng.below(5)).min(cs.len() - 1);
        b.append(&text[cs[i]..cs[j]]);
        i = j;
    }
    v.push(b.finish().snapshot());
    // 文書に少しずつ挿入する (断片化した rope)
    let mut doc = Document::from_text(Text::new(), TextFormat::new_document());
    let (mut i, mut at) = (0, 0);
    while i + 1 < cs.len() {
        let j = (i + 1 + rng.below(4)).min(cs.len() - 1);
        let piece = &text[cs[i]..cs[j]];
        let change = eve_text::ChangeSet::from_sorted_edits(
            doc.text(),
            [eve_text::Edit {
                range: ByteRange::new(ByteOff(at), ByteOff(at)),
                insert: piece.into(),
            }],
        )
        .unwrap();
        doc.apply(&Transaction::new(change, Origin::Paste, Tick(0)))
            .unwrap();
        at += piece.len();
        i = j;
    }
    v.push(doc.snapshot());
    v
}

const ORIG: &[u8] = b"ORIGINAL-CONTENT\n";

struct Env {
    fs: Arc<MemFs>,
    ports: JobPorts,
}

fn env() -> Env {
    let clock = Arc::new(ManualClock::new());
    let fs = Arc::new(MemFs::new(clock.clone()));
    fs.add_dir("/w");
    fs.add_file("/w/a.txt", ORIG);
    let ports = JobPorts {
        stdin: None,
        fs: fs.clone(),
        clock,
        paths: Arc::new(FixedPaths::default()),
    };
    Env { fs, ports }
}

fn save(
    e: &Env,
    snap: &Snapshot,
    enc: Encoding,
    bom: bool,
    escape: EscapePolicy,
    strategy: SaveStrategy,
    ok: Option<EscapeMergeApproval>,
) -> JobResult {
    let req = JobRequest {
        id: JobId(1),
        doc: None,
        base: Revision(0),
        kind: JobKind::Save {
            path: FsPath::from("/w/a.txt"),
            snapshot: snap.clone(),
            encoding: enc,
            bom,
            escape,
            expected: Expect::Any,
            strategy,
            clear_readonly: false,
            create_link_target: false,
            preserved: PreservedMap::default(),
            escape_merge_ok: ok,
        },
    };
    let mut out = Vec::new();
    run_job(&req, &e.ports, &CancelToken::new(), &mut |r| out.push(r));
    out.into_iter()
        .map(|o| o.body)
        .rfind(|b| !matches!(b, JobResult::Progress { .. }))
        .expect("結果がある")
}

const STRATS: [SaveStrategy; 2] = [
    SaveStrategy::Atomic {
        hardlink: HardlinkPolicy::Ask,
    },
    SaveStrategy::InPlace {
        backup: InPlaceBackup::SkipConfirmed,
    },
];

fn bom_bytes(enc: Encoding, bom: bool) -> &'static [u8] {
    match enc {
        _ if !bom => &[],
        Encoding::Utf8 => b"\xEF\xBB\xBF",
        Encoding::Utf16Le => b"\xFF\xFE",
        Encoding::Utf16Be => b"\xFE\xFF",
        _ => &[],
    }
}

/// 1 件を確かめる。失敗は文字列で返す。
fn check(enc: Encoding, bom: bool, toks: &[Tok], rng: &mut Rng) -> Result<(), String> {
    let text: String = toks.iter().map(|&t| tok_text(t)).collect();
    let body: Vec<u8> = toks.iter().flat_map(|&t| tok_bytes(enc, t)).collect();
    let bb = bom_bytes(enc, bom);
    let bom_written = !bb.is_empty();
    let written = [bb, &body].concat();
    // 正: 書いた BOM を除いた出力の全体を、同じ文字コードの初期状態の復号器で復号すると本文に一致するか
    let truth = decode(enc, &body, false) != text;
    let mut first_merge: Option<(u64, Vec<eve_core::EscapeMergeAt>, u64)> = None;
    for (si, snap) in snapshot_shapes(&text, rng).iter().enumerate() {
        let strategy = STRATS[rng.below(2)];
        let e = env();
        let r = save(&e, snap, enc, bom, EscapePolicy::Restore, strategy, None);
        let ctx = || format!("{enc:?} bom={bom} shape={si} {strategy:?} toks={toks:?}");
        match &r {
            JobResult::Saved { .. } => {
                if truth {
                    return Err(format!("見逃し (書いた): {}", ctx()));
                }
                let got = e.fs.read("/w/a.txt").unwrap();
                if got != written {
                    return Err(format!("出力が違う: got={got:02X?} {}", ctx()));
                }
                if decode(enc, &got, true) != text
                    && !(text.starts_with('\u{FEFF}') && !bom_written)
                {
                    return Err(format!("開き直すと違う: {}", ctx()));
                }
            }
            JobResult::EscapeMerge {
                total,
                recorded,
                digest,
                ..
            } => {
                if !truth {
                    return Err(format!("誤検出: {r:?} {}", ctx()));
                }
                if *total == 0 || recorded.is_empty() {
                    return Err(format!("件数 0 の EscapeMerge: {}", ctx()));
                }
                if e.fs.read("/w/a.txt").unwrap() != ORIG {
                    return Err(format!("EscapeMerge なのに書いた: {}", ctx()));
                }
                if e.fs.paths() != ["/w/a.txt".to_owned()] {
                    return Err(format!("一時ファイルが残る: {:?} {}", e.fs.paths(), ctx()));
                }
                // チャンクの形に依らない
                let now = (*total, recorded.clone(), *digest);
                match &first_merge {
                    None => first_merge = Some(now),
                    Some(f) if *f != now => {
                        return Err(format!("チャンクの形で結果が違う: {}", ctx()));
                    }
                    Some(_) => {}
                }
                // 承認して保存し直すと、同じバイト列を書く (区切った確認 (ii) が誤って止めない)
                let ok = EscapeMergeApproval {
                    total: *total,
                    recorded: recorded.clone(),
                    digest: *digest,
                };
                let e2 = env();
                let r2 = save(
                    &e2,
                    snap,
                    enc,
                    bom,
                    EscapePolicy::Restore,
                    strategy,
                    Some(ok),
                );
                if !matches!(r2, JobResult::Saved { .. }) {
                    return Err(format!("承認しても書けない: {r2:?} {}", ctx()));
                }
                if e2.fs.read("/w/a.txt").unwrap() != written {
                    return Err(format!("承認した出力が違う: {}", ctx()));
                }
            }
            other => return Err(format!("想定外: {other:?} {}", ctx())),
        }
    }
    Ok(())
}

fn gen_toks(enc: Encoding, rng: &mut Rng, alpha: &[char], len: usize) -> Vec<Tok> {
    let hot: &[u8] = match enc {
        Encoding::Utf8 => &[
            0xE3, 0x81, 0x82, 0xEF, 0xBB, 0xBF, 0xC3, 0xA9, 0xF0, 0x9F, 0x98, 0x80, 0xFF, 0xFE,
            0xED, 0xA0,
        ],
        Encoding::ShiftJis => &[
            0x81, 0x40, 0x88, 0x9F, 0xEF, 0xBB, 0xBF, 0xFF, 0xFE, 0xFA, 0x80, 0xA0, 0xFD, 0xE0,
            0xFC,
        ],
        Encoding::EucJp => &[
            0x8F, 0x8E, 0xA4, 0xA2, 0xB0, 0xA1, 0xFE, 0xFF, 0xEF, 0xBB, 0xBF, 0x80, 0xF5,
        ],
        _ => &[
            0x00, 0xD8, 0xDC, 0xDB, 0xDF, 0xFF, 0xFE, 0x41, 0x0A, 0x0D, 0x3D, 0xEF, 0xBB, 0xBF,
        ],
    };
    let esc: Vec<u8> = if enc.is_utf16() {
        (0..=255).collect()
    } else {
        (0x80..=255).collect()
    };
    let crit: &[char] = &['\n', '\r', '@', 'a', '\u{FEFF}', 'あ', '\0'];
    (0..len)
        .map(|_| match rng.below(10) {
            0..=2 => Tok::E(hot[rng.below(hot.len())]),
            3..=4 => Tok::E(esc[rng.below(esc.len())]),
            5 => {
                let c = crit[rng.below(crit.len())];
                Tok::C(if alpha.contains(&c) { c } else { 'a' })
            }
            _ => Tok::C(alpha[rng.below(alpha.len())]),
        })
        .collect()
}

// verifies: NFR-304
#[test]
fn 乱数の本文の保存を出力の全体の復号と突き合わせる() {
    let n = n_cases();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut fails = Vec::new();
    for enc in ENCS {
        let alpha = alphabet(enc);
        for i in 0..n {
            let len = 1 + rng.below(if i % 10 == 0 { 40 } else { 8 });
            let mut toks = gen_toks(enc, &mut rng, &alpha, len);
            // 時々、長い ASCII の前置き・後置き (チャンクの境界をまたがせる)
            if i % 7 == 0 {
                let pad = rng.below(2100);
                let mut p: Vec<Tok> = (0..pad)
                    .map(|j| Tok::C(if j % 61 == 60 { '\n' } else { 'p' }))
                    .collect();
                p.extend(toks);
                toks = p;
                toks.extend(gen_toks(enc, &mut rng, &alpha, 6));
            }
            let bom = rng.below(2) == 0;
            if let Err(e) = check(enc, bom, &toks, &mut rng)
                && fails.len() < 30
            {
                fails.push(e);
            }
        }
    }
    assert!(
        fails.is_empty(),
        "{} 件の失敗:\n{}",
        fails.len(),
        fails.join("\n")
    );
}

/// バイト列の部品の一覧。
type Pieces = Vec<&'static [u8]>;

/// 記録を作るバイト列 (重複符号化) と、不正バイトになるバイト列と、通常の文字のバイト列。
fn rec_pieces(enc: Encoding) -> (Pieces, Pieces, Pieces) {
    match enc {
        Encoding::ShiftJis => (
            vec![
                b"\xFA\x54",
                b"\x87\x90",
                b"\xFA\x5B",
                b"\xED\x40",
                b"\xEE\xEF",
            ],
            vec![
                b"\x80", b"\xA0", b"\xFD", b"\xFF", b"\x81", b"\xEF", b"\xFA", b"\x87", b"\xE0",
            ],
            vec![
                b"a",
                b"@",
                b"\n",
                b"\r\n",
                b"\x82\xA0",
                b"\xB1",
                b"\xBF",
                b"\x88\x9F",
                b"T",
                b"\x90",
                b"Z",
            ],
        ),
        _ => (
            vec![
                b"\x8F\xB0\xA1",
                b"\x8F\xA2\xB7",
                b"\xAD\xA1",
                b"\x8F\xF3\xF3",
            ],
            vec![
                b"\x8F", b"\x8E", b"\xA4", b"\xFF", b"\x80", b"\xA1", b"\xB0", b"\xF3", b"\xEF",
            ],
            vec![
                b"a",
                b"\n",
                b"\r\n",
                b"\xA4\xA2",
                b"\x8E\xB1",
                b"\xB0\xA1",
                b"z",
                b"\xA2\xB7",
            ],
        ),
    }
}

const PAD: &str = "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789\n";

// 記録を持つ文書を開いて (「編集を許可」で解除して) 一部を消して保存する: 書いたら開き直すと本文と同じで clean、
// 書かなかったら dirty のまま、`VerifyFailed` (保持なし) にはならない
// verifies: NFR-304
#[test]
fn 記録を持つ文書を開いて編集して保存する() {
    let n = (n_cases() / 10).max(30);
    let mut rng = Rng(0xC0FF_EE12_3456_789B);
    let mut fails = Vec::new();
    for enc in [Encoding::ShiftJis, Encoding::EucJp] {
        let (recs, bad, ok) = rec_pieces(enc);
        for _ in 0..n {
            let mut file = Vec::new();
            for _ in 0..2 + rng.below(14) {
                let p: &[u8] = match rng.below(10) {
                    0..=2 => recs[rng.below(recs.len())],
                    3..=5 => bad[rng.below(bad.len())],
                    _ => ok[rng.below(ok.len())],
                };
                file.extend_from_slice(p);
            }
            let body_end = file.len();
            for _ in 0..6 {
                file.extend_from_slice(PAD.as_bytes());
            }
            let mut h = Harness::builder().defer_start().build();
            h.fs.add_file("/w/a.txt", &file);
            h.editor.state_mut().open_path(
                "/w/a.txt",
                OpenOptions {
                    forced: Some(enc),
                    ..OpenOptions::default()
                },
            );
            h.run_jobs();
            // 「編集を許可」(`file.toggle_read_only` と確認の `[A]`)。不正バイトの無い文書は初めから編集できる
            let id = h.doc_id();
            if !h.editor.state().document(id).unwrap().is_editable() {
                if !h
                    .editor
                    .state()
                    .document(id)
                    .unwrap()
                    .read_only_releasable()
                {
                    continue;
                }
                h.cmd("file.toggle_read_only");
                h.text("a");
                assert!(h.editor.state().document(id).unwrap().is_editable());
            }
            // 本文の先頭部分 (詰め物の前) で、1〜3 文字を 1〜3 回消す
            let text0 = h.doc_text();
            let head_chars = decode(enc, &file[..body_end], false).chars().count();
            for _ in 0..1 + rng.below(3) {
                let cur = h.doc_text();
                let cs: Vec<usize> = cur.char_indices().map(|c| c.0).collect();
                let lim = head_chars.min(cs.len().saturating_sub(1)).max(1);
                let a = rng.below(lim);
                let b = (a + 1 + rng.below(3)).min(cs.len());
                let (ba, bb) = (cs[a], cs.get(b).copied().unwrap_or(cur.len()));
                let d = h.editor.state_mut().document_mut(id).unwrap();
                let change = eve_text::ChangeSet::from_sorted_edits(
                    d.text(),
                    [eve_text::Edit {
                        range: ByteRange::new(ByteOff(ba), ByteOff(bb)),
                        insert: "".into(),
                    }],
                )
                .unwrap();
                d.apply(&Transaction::new(change, Origin::Paste, Tick(0)))
                    .unwrap();
            }
            let text = h.doc_text();
            if text == text0 {
                continue;
            }
            let _ = h.cmd_with("file.save", Args::new());
            h.run_jobs();
            let got = h.fs.read("/w/a.txt").unwrap();
            let d = h.editor.state().document(id).unwrap();
            let head = format!("{enc:?} file={:02X?}", &file[..body_end]);
            if got == file {
                if !d.is_dirty() {
                    fails.push(format!("書いていないのに clean: {head}"));
                }
                if d.format.roundtrip.preserve
                    == PreserveStatus::Unavailable(NoPreserveReason::VerifyFailed)
                {
                    fails.push(format!("VerifyFailed: {head}"));
                }
            } else {
                if decode(enc, &got, true) != text {
                    fails.push(format!("黙って違う文字で書いた: {head}"));
                }
                if d.is_dirty() {
                    fails.push(format!("書いたのに dirty: {head}"));
                }
            }
        }
    }
    assert!(
        fails.is_empty(),
        "{} 件:\n{}",
        fails.len(),
        fails
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// 不正バイトの無い本文の保存は、エスケープの方針に依らず全体を 1 回で符号化したものと同じ (つながりの検出と区切った
// 確認 (ii) を足しても出力が変わらない)。ISO-2022-JP の Restore はエスケープ文字を書かない
#[test]
fn エスケープ文字の無い保存は全体を_1_回で符号化したものと同じ() {
    let mut rng = Rng(0xABCD_EF01_2345_6789);
    let mut fails = Vec::new();
    for enc in ENCS.into_iter().chain([Encoding::Iso2022Jp]) {
        let alpha = alphabet(enc);
        for _ in 0..(n_cases() / 10).max(10) {
            let text: String = (0..rng.below(60))
                .map(|_| alpha[rng.below(alpha.len())])
                .collect();
            let bom = rng.below(2) == 0;
            for policy in [
                EscapePolicy::Reject,
                EscapePolicy::AsText,
                EscapePolicy::Restore,
            ] {
                let mut e = Encoder::new(enc, bom, policy);
                let (mut want, mut rep) = (Vec::new(), EncodeReport::default());
                e.encode(&text, &mut want, &mut rep);
                e.finish(&mut want, &mut rep);
                for snap in &snapshot_shapes(&text, &mut rng) {
                    for strategy in STRATS {
                        let env_ = env();
                        match save(&env_, snap, enc, bom, policy, strategy, None) {
                            JobResult::Saved { .. } if rep.is_empty() => {
                                if env_.fs.read("/w/a.txt").unwrap() != want {
                                    fails.push(format!("{enc:?} {policy:?} {bom} {text:?}"));
                                }
                            }
                            JobResult::Unmappable { .. } if !rep.is_empty() => {}
                            other => {
                                fails.push(format!("{enc:?} {policy:?} {bom} {text:?}: {other:?}"));
                            }
                        }
                    }
                }
            }
        }
    }
    for strategy in STRATS {
        let env_ = env();
        let snap = Text::from("a\u{10FF81}b\n").snapshot();
        let r = save(
            &env_,
            &snap,
            Encoding::Iso2022Jp,
            false,
            EscapePolicy::Restore,
            strategy,
            None,
        );
        if !matches!(r, JobResult::Unmappable { .. }) || env_.fs.read("/w/a.txt").unwrap() != ORIG {
            fails.push(format!("ISO-2022-JP Restore: {r:?}"));
        }
    }
    assert!(
        fails.is_empty(),
        "{}",
        fails
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
