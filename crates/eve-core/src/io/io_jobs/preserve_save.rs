//! 保存での保持方式の記録の差し替え (03 §8.6 E2、§8.7.1 の「保存」、M22 の 6) の単体試験。
//!
//! - 未編集の文書は、記録を使って書くと元のファイルとバイト単位で一致する。
//! - 表が空なら、v0.3 (5 本目まで) の出力 (記録なしの符号化) とバイト単位で同じ。
//! - 乾式走査と本番の書き込みの供給の出力が一致し、rope のチャンクの形に依らない。
//! - 故障の注入: 表の記録の位置をずらす・元のバイト列をすり替える・ISO-2022-JP の記録を行の途中に置く、などをしても、
//!   誤ったバイト列をファイルに書かない (確認 (i) で記録を使わずに符号化するか、確認 (ii) で保存を止める。どちらになるかを
//!   固定する)。原子的保存と直接上書きの両方で、元のファイルを壊さない。記録を不正バイトのエスケープの直後に置くと、
//!   v0.4 の M23 からは、つながり (`EscapeMerge`、03 §8.3.1) として書く前に止まる (`VerifyFailed` にしない)。

use std::sync::{Arc, Mutex};

use eve_encoding::{Decoder, Encoding, EscapePolicy, PreserveReason, check_roundtrip_with_records};
use eve_text::{ByteOff, ByteRange, Text, TextRead};

use super::*;
use crate::document::preserved::PreservedSpan;
use crate::io::fs::{
    FileKind, FileMeta, FileSystem, FileTime, FsKind, InPlaceBackup, SaveMethod, SaveReport,
};
use crate::jobs::SaveStrategy;
use crate::time::{Clock, Tick};

/// バイト列を読込と同じ手順で本文と表にする (復号・記録を集める・表にして照合する)。
fn load(enc: Encoding, bytes: &[u8]) -> (Text, PreservedMap) {
    let text = Text::from(decode(enc, bytes).as_str());
    let (_, recs) = check_roundtrip_with_records(enc, bytes);
    let map = PreservedMap::from_records(&recs, enc, &text).expect("読込の完了時の照合が通る");
    (text, map)
}

fn decode(enc: Encoding, bytes: &[u8]) -> String {
    let mut d = Decoder::new(enc, false);
    let mut s = String::new();
    d.decode(bytes, true, &mut s);
    s
}

fn all(t: &dyn TextRead) -> String {
    t.slice_to_string(ByteRange::all(t.len_bytes()))
}

/// 元の文字コードで保存するときの不正バイトのエスケープの扱い (読んだ文字コードで書くバイトの文書の
/// `TextFormat::save_escape_policy` と同じ。状態付きは戻さない)。
fn policy(enc: Encoding) -> EscapePolicy {
    if enc.is_stateful() {
        EscapePolicy::Reject
    } else {
        EscapePolicy::Restore
    }
}

/// 記録なしの符号化 (v0.3 の出力): 全文を 1 つの符号化器で。
fn plain(enc: Encoding, text: &str) -> Vec<u8> {
    let mut e = Encoder::new(enc, false, policy(enc));
    let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
    e.encode(text, &mut out, &mut rep);
    e.finish(&mut out, &mut rep);
    out
}

/// 決定的な擬似乱数 (xorshift)。
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// 0 以上 `n` 未満。
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap_or(0)
    }

    fn pick<'a>(&mut self, xs: &[&'a [u8]]) -> &'a [u8] {
        xs[self.below(xs.len())]
    }
}

/// `Shift_JIS` の部品 (区分 B の重複符号化・互換文字・外字・半角カナ・不正バイト)。
const SJIS: &[&[u8]] = &[
    b"a",
    b"Z\\ ",
    b"\x93\xFA",
    b"\xFA\x54",
    b"\xFA\x55",
    b"\xED\x40",
    b"\xEE\xEF",
    b"\x87\x90",
    b"\x81\xCA",
    b"\xF0\x40",
    b"\xB1",
    b"\xFD",
    b"\x81 ",
];

/// EUC-JP の部品 (補助漢字・別のコードに写るもの・重複・不正バイト)。
const EUC: &[&[u8]] = &[
    b"a",
    b"\xC6\xFC",
    b"\x8E\xB1",
    b"\x8F\xB0\xA1",
    b"\x8F\xA2\xB7",
    b"\x8F\xF3\xF3",
    b"\xAD\xA1",
    b"\xFC\xFC",
    b"\xA1\xC1",
    b"\xFF",
];

/// ISO-2022-JP の部品 (旧 JIS・ローマ字・半角カナ・冗長なエスケープ)。行の終わりは呼び出し側が ASCII 状態に戻す。
const ISO: &[&[u8]] = &[
    b"ab",
    b"\x1B$B$\"\x1B(B",
    b"\x1B$@$\"\x1B(B",
    b"\x1B(J\\a\x1B(B",
    b"\x1B(I12\x1B(B",
    b"\x1B(B",
    b"\x1B$BF|K\\\x1B(B",
];

const EOLS: &[&[u8]] = &[b"\n", b"\r\n", b"\r"];

fn random_doc(rng: &mut Rng, enc: Encoding) -> Vec<u8> {
    let atoms = match enc {
        Encoding::ShiftJis => SJIS,
        Encoding::EucJp => EUC,
        _ => ISO,
    };
    let mut b = Vec::new();
    for _ in 0..(rng.next() % 12) {
        for i in 0..(rng.next() % 5) {
            // ISO-2022-JP は、エスケープシーケンスが出力を挟まずに続く (冗長な、不正バイトになる) 並びを減らす
            if enc.is_stateful() && i > 0 && !rng.next().is_multiple_of(4) {
                b.push(b'x');
            }
            b.extend_from_slice(rng.pick(atoms));
        }
        if !rng.next().is_multiple_of(4) {
            b.extend_from_slice(rng.pick(EOLS));
        }
    }
    b
}

const ENCS: [Encoding; 3] = [Encoding::ShiftJis, Encoding::EucJp, Encoding::Iso2022Jp];

// 03 §8.7.1 の「保存」: 未編集の文書は、元のバイト列の記録と区分 A の再符号化で、元のファイルとバイト単位で一致する
#[test]
fn 未編集の文書を記録を使って書くと元のファイルと一致する() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for enc in ENCS {
        let mut tried = 0;
        for _ in 0..600 {
            let bytes = random_doc(&mut rng, enc);
            let mut d = Decoder::new(enc, false);
            d.decode(&bytes, true, &mut String::new());
            if enc.is_stateful() && d.invalid_bytes() > 0 {
                // 不正バイト (冗長なエスケープなど) を含む ISO-2022-JP は、ISO-2022-JP では保存しない (03 §8.3)
                continue;
            }
            tried += 1;
            let (summary, recs) = check_roundtrip_with_records(enc, &bytes);
            let (text, map) = load(enc, &bytes);
            let out = save_for_test(&text.snapshot(), enc, policy(enc), &map);
            assert!(out.verified && out.broken == 0, "{enc:?} {bytes:02X?}");
            assert_eq!(out.dry_len, out.bytes.len() as u64);
            if out.unmappable > 0 {
                // 編集していない ISO-2022-JP の行でも、記録の無い行 (ASCII 状態で始まらない行) の半角カナは表現不能
                assert!(enc.is_stateful() && recs.unpreserved > 0, "{bytes:02X?}");
                continue;
            }
            assert_eq!(
                out.source.as_ref(),
                Some(&out.bytes),
                "本番の書き込みの供給と同じ"
            );
            assert_eq!(decode(enc, &out.bytes), all(&text), "{enc:?} {bytes:02X?}");
            assert_eq!(out.line_unused, 0);
            if recs.unpreserved == 0 && !summary.incomplete {
                assert_eq!(out.bytes, bytes, "{enc:?} 未編集はバイト一致");
            }
        }
        assert!(tried > 100, "{enc:?}: {tried}");
    }
}

// 差分の試験: 表が空なら、5 本目までの出力 (記録なしの符号化) とバイト単位で同じ
#[test]
fn 表が空なら記録なしの符号化と同じ出力() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let empty = PreservedMap::new();
    for enc in ENCS {
        for _ in 0..300 {
            let bytes = random_doc(&mut rng, enc);
            let text = Text::from(decode(enc, &bytes).as_str());
            let out = save_for_test(&text.snapshot(), enc, policy(enc), &empty);
            assert_eq!(out.bytes, plain(enc, &all(&text)), "{enc:?} {bytes:02X?}");
            assert_eq!(out.dry_len, out.bytes.len() as u64);
            // 確認 (ii) は、エスケープ文字を元のバイトに戻す保存でも行う (M23 の 3 本目)。表現不能の文字 (補助漢字など。
            // 表が空なので符号化する) は出力から抜けて突き合わせがずれるが、保存は表現不能 (E3) で先に止まる
            assert!(out.verified || out.unmappable > 0, "{enc:?} {bytes:02X?}");
        }
    }
}

// (b) rope のチャンクの形に依らない: 同じ本文を小さな挿入の積み重ねで作った (断片化した) 文書でも同じ出力
#[test]
fn 出力は_rope_のチャンクの形に依らない() {
    use crate::document::{Document, Origin, TextFormat, Transaction};
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for enc in ENCS {
        for _ in 0..40 {
            let mut bytes = Vec::new();
            // 1 チャンクより長い文書 (記録が rope のチャンクの境をまたぐように)
            while bytes.len() < 6000 {
                bytes.extend(random_doc(&mut rng, enc));
            }
            let (text, map) = load(enc, &bytes);
            let s = all(&text);
            let whole = save_for_test(&text.snapshot(), enc, policy(enc), &map);
            // 前から 1〜7 文字ずつ挿入して作る
            let mut doc = Document::from_text(Text::new(), TextFormat::new_document());
            let mut at = 0usize;
            let chars: Vec<(usize, char)> = s.char_indices().collect();
            let mut i = 0usize;
            while i < chars.len() {
                let n = 1 + rng.below(7);
                let end = chars.get(i + n).map_or(s.len(), |c| c.0);
                let piece = &s[chars[i].0..end];
                let cs = eve_text::ChangeSet::from_sorted_edits(
                    doc.text(),
                    [eve_text::Edit {
                        range: ByteRange::new(ByteOff(at), ByteOff(at)),
                        insert: piece.into(),
                    }],
                )
                .unwrap();
                doc.apply(&Transaction::new(cs, Origin::Paste, Tick(0)))
                    .unwrap();
                at += piece.len();
                i += n;
            }
            assert_eq!(all(doc.text()), s);
            let frag = save_for_test(&doc.snapshot(), enc, policy(enc), &map);
            assert_eq!(frag.bytes, whole.bytes, "{enc:?}");
            assert_eq!(frag.dry_len, whole.dry_len);
            assert_eq!(frag.line_unused, whole.line_unused);
        }
    }
}

// ---------- 故障の注入 ----------
//
// 表の記録が確認 (i) に合わない・表の不変条件に反するとき (実装の誤り) は、どのビルドでも何も書かずに止める
// (`JobError::VerifyFailed`、03 §8.6 E2)。ISO-2022-JP の行の記録が行の内容の全体でない (行を結合した) のは違反ではなく、
// その行を編集した行と同じく正規形で書いて保存は成功する。

/// 表の記録の列を、`f` で書き換えた表にする (試験用の口)。
fn tamper(map: &PreservedMap, f: impl FnOnce(&mut Vec<PreservedSpan>)) -> PreservedMap {
    let mut spans = map.to_spans();
    f(&mut spans);
    PreservedMap::from_spans(&spans)
}

/// `a￢b` + `￢￢c` (￢ は IBM 拡張の重複 0xFA54。代表バイト列は 0x81CA)。
const SJIS_DOC: &[u8] = b"a\xFA\x54b\n\xFA\x54\xFA\x54c\n";

/// 確かめに合わず、乾式走査でも本番の書き込みの供給でも止まる。
fn assert_stopped(out: &SaveTestOut, broken: usize) {
    assert!(!out.verified, "確かめに合わない");
    assert_eq!(out.broken, broken);
    assert!(
        out.source.is_none(),
        "本番の書き込みの供給も最後まで返さない"
    );
}

#[test]
fn 故障の注入_記録の位置を_1_バイトずらすと止める() {
    let (text, map) = load(Encoding::ShiftJis, SJIS_DOC);
    let bad = tamper(&map, |s| {
        s[0].range = ByteRange::new(ByteOff(2), ByteOff(5)); // ￢ の UTF-8 の途中
    });
    let out = save_for_test(
        &text.snapshot(),
        Encoding::ShiftJis,
        EscapePolicy::Restore,
        &bad,
    );
    assert_stopped(&out, 1);
}

#[test]
fn 故障の注入_記録の位置を_1_文字ずらすと確認_i_で止める() {
    let (text, map) = load(Encoding::ShiftJis, SJIS_DOC);
    // ￢ (1..4) を b (4..5) へ。範囲の文字列 "b" は元のバイト列 0xFA54 の復号 (￢) と違う
    let bad = tamper(&map, |s| {
        s[0].range = ByteRange::new(ByteOff(4), ByteOff(5))
    });
    let out = save_for_test(
        &text.snapshot(),
        Encoding::ShiftJis,
        EscapePolicy::Restore,
        &bad,
    );
    assert_stopped(&out, 1);
}

#[test]
fn 故障の注入_元のバイト列をすり替えると確認_i_で止める() {
    let (text, map) = load(Encoding::ShiftJis, SJIS_DOC);
    // ￢ の記録の元のバイト列を、別の文字 (￤ 0xFA55) のものにすり替える
    let bad = tamper(&map, |s| s[0].bytes = (*b"\xFA\x55").into());
    let out = save_for_test(
        &text.snapshot(),
        Encoding::ShiftJis,
        EscapePolicy::Restore,
        &bad,
    );
    assert_stopped(&out, 1);
    // EUC-JP: 補助漢字の記録を別の補助漢字のものにすり替える。表現不能 (E3) より先に、確かめの不一致で止める
    let (text, map) = load(Encoding::EucJp, b"x\x8F\xB0\xA1y\n");
    let bad = tamper(&map, |s| s[0].bytes = (*b"\x8F\xB0\xA2").into());
    let out = save_for_test(
        &text.snapshot(),
        Encoding::EucJp,
        EscapePolicy::Restore,
        &bad,
    );
    assert_stopped(&out, 1);
}

#[test]
fn 故障の注入_記録の重なりと本文の外は止める() {
    let (text, map) = load(Encoding::ShiftJis, SJIS_DOC);
    let bad = tamper(&map, |s| {
        // 2 件目を 1 件目に重ねる、本文の外の記録を足す
        let mut extra = s[0].clone();
        extra.range = ByteRange::new(ByteOff(3), ByteOff(6));
        s.insert(1, extra);
        let mut out = s[0].clone();
        out.range = ByteRange::new(ByteOff(100), ByteOff(103));
        s.push(out);
    });
    let out = save_for_test(
        &text.snapshot(),
        Encoding::ShiftJis,
        EscapePolicy::Restore,
        &bad,
    );
    assert_stopped(&out, 2);
}

/// `あ` (旧 JIS の `ESC $ @`) の行と、半角カナ (`ESC ( I`) の行と、ASCII だけの行。
const ISO_DOC: &[u8] = b"\x1B$@$\"\x1B(B\n\x1B(I1\x1B(B\nabc\n";

#[test]
fn iso_2022_jp_の記録が行の途中にあれば_その行を正規形で書いて保存できる() {
    let (text, map) = load(Encoding::Iso2022Jp, ISO_DOC);
    assert_eq!(all(&text), "あ\nｱ\nabc\n");
    // 1 行目の記録を、本文 "aあb" の行の途中 (あ) に置いた形にする (行を結合したのと同じ形)
    let t2 = Text::from("aあb\nｱ\nabc\n");
    let moved = tamper(&map, |s| {
        s[0].range = ByteRange::new(ByteOff(1), ByteOff(4));
        s[1].range = ByteRange::new(ByteOff(6), ByteOff(9));
    });
    let out = save_for_test(
        &t2.snapshot(),
        Encoding::Iso2022Jp,
        EscapePolicy::Reject,
        &moved,
    );
    assert!(out.verified, "行の途中の記録は不変条件の違反ではない");
    assert_eq!((out.broken, out.line_unused), (0, 1));
    assert_eq!(
        out.bytes, b"a\x1B$B$\"\x1B(Bb\n\x1B(I1\x1B(B\nabc\n",
        "1 行目は正規形、2 行目 (半角カナ) は元のバイト列"
    );
    assert_eq!(out.source.as_ref(), Some(&out.bytes));
}

#[test]
fn 故障の注入_iso_2022_jp_の元のバイト列が_ascii_状態で終わらなければ止める() {
    let (text, map) = load(Encoding::Iso2022Jp, ISO_DOC);
    // 半角カナの状態のまま終わる元のバイト列 (すり替え): 確認 (i) に合わない
    let bad = tamper(&map, |s| s[1].bytes = (*b"\x1B(I1").into());
    let out = save_for_test(
        &text.snapshot(),
        Encoding::Iso2022Jp,
        EscapePolicy::Reject,
        &bad,
    );
    assert_stopped(&out, 1);
}

/// 孤立した先頭バイト 0x81 (エスケープ文字) の直後に ￢ の記録を置いた本文と表 (読込ではこの並びにならない。0x81 0xFA は
/// 組になる)。
fn escape_before_record() -> (Text, PreservedMap) {
    let text = Text::from("\u{10FF81}￢x");
    let map = PreservedMap::from_spans(&[PreservedSpan {
        range: ByteRange::new(ByteOff(4), ByteOff(7)),
        bytes: (*b"\xFA\x54").into(),
        reason: PreserveReason::DuplicateEncoding,
        line: false,
    }]);
    (text, map)
}

// v0.4 の M23 の 2 本目までは、確認 (ii) の不一致 (`VerifyFailed`) で止めていた。並びの直後の記録は、記録の全体を直後の
// 1 単位として扱うので、つながり (03 §8.3.1) として見つけ、確認 (ii) はつながる範囲を突き合わせない
#[test]
fn 不正バイトのエスケープの直後の記録とのつながりは確認_ii_で止めずにつながりとして見つける() {
    let (text, map) = escape_before_record();
    let out = save_for_test(
        &text.snapshot(),
        Encoding::ShiftJis,
        EscapePolicy::Restore,
        &map,
    );
    // 確認 (i) は通る (記録だけを見れば正しい)
    assert!(out.verified && out.broken == 0, "{out:?}");
    let m = out.merges.as_ref().expect("つながりを調べる");
    assert_eq!((m.total, m.recorded[0].char_index), (1, 0));
    assert_eq!(m.recorded[0].len_chars, 2, "並びと記録の全体");
    assert_eq!(out.bytes, b"\x81\xFA\x54x");
}

// ---------- 原子的保存・直接上書き (Port まで) ----------

/// 1 つのファイルだけを持つ偽の FS。書いた回数を数える。
pub(super) struct OneFile {
    content: Mutex<Vec<u8>>,
    writes: Mutex<u32>,
}

impl OneFile {
    pub(super) fn new(bytes: &[u8]) -> Arc<OneFile> {
        Arc::new(OneFile {
            content: Mutex::new(bytes.to_vec()),
            writes: Mutex::new(0),
        })
    }

    pub(super) fn content(&self) -> Vec<u8> {
        self.content.lock().unwrap().clone()
    }

    pub(super) fn writes(&self) -> u32 {
        *self.writes.lock().unwrap()
    }

    fn report(len: usize, method: SaveMethod) -> SaveReport {
        SaveReport {
            method,
            new_stamp: crate::FileStamp {
                mtime: FileTime::default(),
                len: len as u64,
                file_id: None,
                hash: None,
            },
            warnings: Vec::new(),
        }
    }

    /// 供給を最後まで読む。途中で失敗したら `SourceFailed` (原子的保存は一時ファイルを捨てる)。
    fn drain(data: &mut dyn ReadChunks) -> Result<Vec<u8>, FsError> {
        let mut buf = Vec::new();
        loop {
            match data.next_chunk() {
                Ok(Some(c)) => buf.extend_from_slice(c),
                Ok(None) => return Ok(buf),
                Err(_) => return Err(FsError::new(FsErrorKind::SourceFailed)),
            }
        }
    }
}

impl FileSystem for OneFile {
    fn metadata(&self, _: &FsPath) -> Result<FileMeta, FsError> {
        Ok(FileMeta {
            kind: FileKind::File,
            len: self.content().len() as u64,
            mtime: FileTime::default(),
            readonly: false,
            file_id: None,
            nlink: 1,
            link: None,
            fs_kind: FsKind::Local,
            owner_is_me: None,
        })
    }
    fn canonicalize(&self, p: &FsPath) -> Result<FsPath, FsError> {
        Ok(p.clone())
    }
    fn open_read(&self, _: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
        Err(FsError::new(FsErrorKind::Other))
    }
    fn write_atomic(
        &self,
        _: &FsPath,
        data: &mut dyn ReadChunks,
        _: &SaveOptions,
    ) -> Result<SaveReport, FsError> {
        *self.writes.lock().unwrap() += 1;
        let buf = OneFile::drain(data)?;
        let n = buf.len();
        *self.content.lock().unwrap() = buf;
        Ok(OneFile::report(n, SaveMethod::Atomic))
    }
    fn write_in_place(
        &self,
        _: &FsPath,
        data: &mut dyn ReadChunks,
        len: u64,
        _: &SaveOptions,
        _: &CancelToken,
    ) -> Result<SaveReport, FsError> {
        *self.writes.lock().unwrap() += 1;
        // 元のファイルへ書き始めた後の失敗は InPlaceFailed (05 §4.4)。この偽の FS は全部を読んでから置き換える
        let buf = OneFile::drain(data).map_err(|_| {
            FsError::new(FsErrorKind::InPlaceFailed {
                backup: Some(FsPath::from("/s/backup/a.bak")),
            })
        })?;
        assert_eq!(buf.len() as u64, len, "乾式走査の長さのとおりに書く");
        let n = buf.len();
        *self.content.lock().unwrap() = buf;
        Ok(OneFile::report(n, SaveMethod::InPlace))
    }
}

pub(super) struct Paths0;
impl crate::io::paths::Paths for Paths0 {
    fn config_dir(&self) -> FsPath {
        FsPath::from("/c")
    }
    fn state_dir(&self) -> FsPath {
        FsPath::from("/s")
    }
    fn data_dir(&self) -> FsPath {
        FsPath::from("/d")
    }
    fn runtime_dir(&self) -> FsPath {
        FsPath::from("/r")
    }
    fn normalize(&self, raw: &str) -> Result<FsPath, FsError> {
        Ok(FsPath::from(raw))
    }
}

pub(super) struct Clock0;
impl Clock for Clock0 {
    fn now(&self) -> Tick {
        Tick(0)
    }
}

pub(super) const STRATEGIES: [SaveStrategy; 2] = [
    SaveStrategy::Atomic {
        hardlink: crate::io::fs::HardlinkPolicy::Ask,
    },
    SaveStrategy::InPlace {
        backup: InPlaceBackup::Required,
    },
];

/// `run_save` を偽の FS で走らせる。
fn run(
    fs: &Arc<OneFile>,
    snapshot: &Snapshot,
    enc: Encoding,
    map: &PreservedMap,
    strategy: SaveStrategy,
) -> JobResult {
    let ports = JobPorts {
        stdin: None,
        fs: Arc::clone(fs) as Arc<dyn FileSystem>,
        clock: Arc::new(Clock0),
        paths: Arc::new(Paths0),
    };
    let path = FsPath::from("/w/a.txt");
    run_save(
        SaveParams {
            path: &path,
            snapshot,
            encoding: enc,
            bom: false,
            escape: policy(enc),
            expected: Expect::Any,
            strategy,
            clear_readonly: false,
            create_link_target: false,
            preserved: map,
            escape_merge_ok: None,
        },
        &ports,
        &CancelToken::new(),
    )
}

/// `tamper` で作る、確かめに合わない表の例 (どれも書く前に止まる)。
fn broken_maps(map: &PreservedMap) -> Vec<(&'static str, PreservedMap)> {
    vec![
        (
            "1 バイトずらす",
            tamper(map, |s| s[0].range = ByteRange::new(ByteOff(2), ByteOff(5))),
        ),
        (
            "1 文字ずらす",
            tamper(map, |s| s[0].range = ByteRange::new(ByteOff(4), ByteOff(5))),
        ),
        (
            "すり替え",
            tamper(map, |s| s[0].bytes = (*b"\xFA\x55").into()),
        ),
        (
            "重なり",
            tamper(map, |s| {
                let mut extra = s[0].clone();
                extra.range = ByteRange::new(ByteOff(3), ByteOff(6));
                s.insert(1, extra);
            }),
        ),
        (
            "本文の外",
            tamper(map, |s| {
                let mut out = s[0].clone();
                out.range = ByteRange::new(ByteOff(100), ByteOff(103));
                s.push(out);
            }),
        ),
    ]
}

#[test]
fn 確かめに合わない表は原子的保存でも直接上書きでも何も書かない() {
    let original = b"original bytes\n".to_vec();
    // 確認 (i) に合わない・表の不変条件に反する
    let (sjis_text, map) = load(Encoding::ShiftJis, SJIS_DOC);
    let mut cases = Vec::new();
    for (name, bad) in broken_maps(&map) {
        cases.push((name, sjis_text.snapshot(), bad));
    }
    for (name, snap, map) in &cases {
        for strategy in STRATEGIES {
            let fs = OneFile::new(&original);
            let r = run(&fs, snap, Encoding::ShiftJis, map, strategy);
            assert!(
                matches!(
                    r,
                    JobResult::Failed {
                        err: JobError::VerifyFailed
                    }
                ),
                "{name} {strategy:?}: {r:?}"
            );
            assert_eq!(
                fs.writes(),
                0,
                "{name} {strategy:?}: 乾式走査で止めて Port を呼ばない"
            );
            assert_eq!(fs.content(), original, "{name}");
        }
    }
    // 並びの直後の記録とのつながり: 確認 (ii) の不一致ではなく、つながりとして書く前に止まる (03 §8.3.1)
    let (text, joint) = escape_before_record();
    for strategy in STRATEGIES {
        let fs = OneFile::new(&original);
        let r = run(&fs, &text.snapshot(), Encoding::ShiftJis, &joint, strategy);
        assert!(
            matches!(r, JobResult::EscapeMerge { total: 1, .. }),
            "{strategy:?}: {r:?}"
        );
        assert_eq!(fs.writes(), 0, "{strategy:?}");
        assert_eq!(fs.content(), original);
    }
}

#[test]
fn 正しい表なら原子的保存でも直接上書きでも元のファイルと一致する() {
    let (text, map) = load(Encoding::ShiftJis, SJIS_DOC);
    for strategy in STRATEGIES {
        let fs = OneFile::new(b"");
        let r = run(&fs, &text.snapshot(), Encoding::ShiftJis, &map, strategy);
        assert!(matches!(r, JobResult::Saved { .. }), "{r:?}");
        assert_eq!(fs.content(), SJIS_DOC);
    }
}

#[test]
fn 表現不能の文字の位置は元のバイト列を書いた範囲を数えて通し番号がずれない() {
    // 補助漢字 (記録) の後ろの 😀 は、通し番号 (文字の数) が 3 (x・丂・y の後ろ)
    let (text, map) = load(Encoding::EucJp, b"x\x8F\xB0\xA1y");
    let t2 = Text::from(format!("{}😀", all(&text)).as_str());
    let snap = t2.snapshot();
    let path = FsPath::from("/w/a.txt");
    let p = SaveParams {
        path: &path,
        snapshot: &snap,
        encoding: Encoding::EucJp,
        bom: false,
        escape: EscapePolicy::Restore,
        expected: Expect::Any,
        strategy: SaveStrategy::default(),
        clear_readonly: false,
        create_link_target: false,
        preserved: &map,
        escape_merge_ok: None,
    };
    let d = dry_run(&p, &CancelToken::new()).unwrap();
    assert_eq!(d.broken, 0);
    assert_eq!(
        d.report.total, 1,
        "補助漢字は記録で書くので表現不能に数えない"
    );
    assert_eq!(d.report.recorded[0].char_index, 3);
    assert_eq!(d.report.recorded[0].ch, '😀');
}
