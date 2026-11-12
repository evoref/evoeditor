//! すべての文字コードのチャンク単位のエンコード (docs/03-core-design.md §8.6)。
//!
//! - **E1 照合**: UTF-8 以外 (UTF-16 を除く) では、出力したバイト列を同じ文字コードで復号して元の文字列と照合し、
//!   `Unmappable` と、黙示の置換 (`¥` → 0x5C、`−` → 0x817C、半角カナ → 全角カナなど) を同じ「表現不能」として
//!   位置つきで報告する (黙って置換しない、FR-112)。最大 100 件を記録し、総数は数える。
//! - **CP932 の外字** (U+E000〜U+E757): `Shift_JIS` では自前の逆算で 2 バイトを出す (`encoding_rs` は復号専用、§8.7)。
//! - **UTF-16**: 自前の書き手 (`encoding_rs` は UTF-16 のエンコードを提供しない。作ると黙って UTF-8 になる)。
//! - **ISO-2022-JP (E6)**: 行ごとにエンコーダを閉じ、各行を ASCII 状態で終える。行は次の改行までためてから符号化する
//!   (改行を含まない巨大な 1 行は、その行の長さだけメモリを使う)。
//! - **不正バイトのエスケープ** (`U+10FF00 + b`): `EscapePolicy::Restore` なら元のバイトに戻して書く (§8.3)。
//! - **保持方式の記録** (v0.4、§8.6 E2・§8.7.1): `Encoder::write_preserved` が、記録の元のバイト列をそのまま書く。
//!   確認 (i) (`preserved_matches`) と ISO-2022-JP の行頭の条件を口の中で確かめ、文字の通し番号を範囲の文字数だけ進める
//!   (範囲の文字は E1 の照合の対象にせず、表現不能に数えない)。保存の計画が確認 (i) を済ませた記録は、確認を通った
//!   ことを型で表す `PreservedBytes` を `Encoder::write_checked` に渡す (口では復号をやり直さず、文字コードと行頭の
//!   条件だけを見る。`PreservedBytes` は確認 (i) を経ずには作れない)。

use encoding_rs::EncoderResult;

use crate::decode::Decoder;
use crate::encoding::Encoding;
use crate::preserve::{IsoState, decodes_to};
use crate::utf8::{PuaEscapeMode, Utf8Encoder, unescape_char};

/// 記録する表現不能の最大件数 (総数は数える)。
pub const MAX_RECORDED: usize = 100;

/// 不正バイトのエスケープ文字の扱い (E4)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapePolicy {
    /// 元のバイトに戻して書く (元の文字コードのまま保存するとき、無編集保存でバイト一致)。
    Restore,
    /// 通常の文字として書く (`DisabledCollision`: 元のテキストが真にこの範囲の文字を含む)。表せない文字コードでは表現不能。
    AsText,
    /// 表現不能として報告する (別の文字コードで保存するとき・状態付き符号化、E4)。
    Reject,
}

impl EscapePolicy {
    /// `PuaEscapeMode` から (同じ文字コードで保存するとき)。
    pub fn from_mode(mode: PuaEscapeMode) -> EscapePolicy {
        match mode {
            PuaEscapeMode::Enabled => EscapePolicy::Restore,
            PuaEscapeMode::DisabledCollision => EscapePolicy::AsText,
            _ => EscapePolicy::Reject,
        }
    }
}

/// 表現不能の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnmappableKind {
    /// エンコーダが `Unmappable` を返した (またはエスケープ文字を書けない)。
    Unmappable,
    /// 別の文字として黙って写された (照合で不一致)。
    Substituted,
}

/// 表現不能の 1 件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unmappable {
    /// これまでに `encode` へ渡した文字の通し番号 (0 始まり、Unicode スカラー値の数)。
    pub char_index: u64,
    pub ch: char,
    pub kind: UnmappableKind,
}

/// 保存中に見つけた表現不能の報告。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EncodeReport {
    /// 先頭 `MAX_RECORDED` 件。
    pub recorded: Vec<Unmappable>,
    /// 総数。
    pub total: u64,
    /// 総数のうち、§8.3 の不正バイトのエスケープ文字 (`U+10FF00 + b`) の数。先頭 `MAX_RECORDED` 件に限らず全件で
    /// 数える (03 §8.6 の「E3 の選択肢」の条件は、記録した先頭の件だけでなく全件で判定する)。
    pub escapes: u64,
}

impl EncodeReport {
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    fn push(&mut self, u: Unmappable) {
        self.total += 1;
        if unescape_char(u.ch).is_some() {
            self.escapes += 1;
        }
        if self.recorded.len() < MAX_RECORDED {
            self.recorded.push(u);
        }
    }
}

/// CP932 の外字 (ユーザー定義領域 0xF040〜0xF9FC) ↔ U+E000〜U+E757 (§8.7、OQ-206)。
pub mod cp932 {
    pub const GAIJI_FIRST: u32 = 0xE000;
    pub const GAIJI_LAST: u32 = 0xE757;

    /// 外字の符号位置 → `Shift_JIS` の 2 バイト。
    pub fn gaiji_to_sjis(ch: char) -> Option<[u8; 2]> {
        let v = ch as u32;
        if !(GAIJI_FIRST..=GAIJI_LAST).contains(&v) {
            return None;
        }
        let n = v - GAIJI_FIRST;
        // 外字の範囲 (0xE000..=0xE757、1880 字) なので n / 188 < 10、n % 188 < 188
        let lead = 0xF0 + u8::try_from(n / 188).ok()?;
        let t = u8::try_from(n % 188).ok()?;
        let trail = if t < 0x3F { 0x40 + t } else { 0x41 + t };
        Some([lead, trail])
    }
}

/// 保持方式の記録の元のバイト列 `bytes` を、`enc` の初期状態 (ISO-2022-JP は ASCII 状態) から復号すると、不正バイト
/// 無しにちょうど `text` になり、初期状態で終わるか (03 §8.6 E2 の確認 (i)、§8.7.1 の不変条件)。記録を使う文字コード
/// (`Shift_JIS` / EUC-JP / ISO-2022-JP) でなければ偽。`bytes` が空なら偽 (記録の元のバイト列は空でない)。
///
/// この真偽だけで、元のバイト列を前後の符号化した出力とつないでも復号の結果が変わらないことが決まる: 状態を持たない
/// 符号化では、`bytes` は文字の境界で始まり文字の境界で終わる。ISO-2022-JP では、ASCII 状態で始まり ASCII 状態で終わる
/// (前後の出力も、行ごとに閉じるので ASCII 状態で接する)。
#[must_use]
pub fn preserved_matches(enc: Encoding, bytes: &[u8], text: &str) -> bool {
    if bytes.is_empty() {
        return false;
    }
    match enc {
        Encoding::ShiftJis | Encoding::EucJp => decodes_to(enc, bytes, text),
        // 復号が不正バイト無しに通れば、エスケープシーケンスはどれも受け付けるもの (状態を正しく追える)
        Encoding::Iso2022Jp => {
            decodes_to(enc, bytes, text) && IsoState::Ascii.after(bytes) == IsoState::Ascii
        }
        Encoding::Utf8 | Encoding::Utf16Le | Encoding::Utf16Be => false,
    }
}

/// 確認 (i) (`preserved_matches`) を通った、保持方式の記録の元のバイト列 (03 §8.6 E2)。[`PreservedBytes::check`] だけが
/// 作る (確認を経ずには作れない) ので、[`Encoder::write_checked`] は受け取ったバイト列の復号をやり直さずに書ける
/// (保存の計画が確かめた結果を、符号化器の口へ型で運ぶ。記録ごとに同じ復号を 2 回しない)。`Debug` は元のバイト列の
/// 中身を出さない (本文由来のバイト列を log やパニックに出さない)。
#[derive(Clone, Copy)]
pub struct PreservedBytes<'a> {
    enc: Encoding,
    bytes: &'a [u8],
    /// 確かめた文字列の文字数 (符号化器の文字の通し番号を進める数)。
    chars: u64,
    /// 確かめた文字列の UTF-8 のバイト数 (書く側が、今書く範囲の長さと合うことを確かめる)。
    text_len: usize,
}

impl std::fmt::Debug for PreservedBytes<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreservedBytes")
            .field("enc", &self.enc)
            .field("bytes", &self.bytes.len())
            .field("chars", &self.chars)
            .field("text_len", &self.text_len)
            .finish()
    }
}

impl<'a> PreservedBytes<'a> {
    /// 確認 (i): `bytes` を `enc` の初期状態から復号すると、不正バイト無しにちょうど `text` になり、初期状態で終わるとき
    /// だけ作る (`preserved_matches` と同じ条件)。
    #[must_use]
    pub fn check(enc: Encoding, bytes: &'a [u8], text: &str) -> Option<PreservedBytes<'a>> {
        preserved_matches(enc, bytes, text).then(|| PreservedBytes {
            enc,
            bytes,
            chars: text.chars().count() as u64,
            text_len: text.len(),
        })
    }

    /// 確かめた文字列の UTF-8 のバイト数。この型が保証するのは「元のバイト列を復号すると、確かめた文字列になる」
    /// までで、その文字列が「今書く範囲の文字列」であることは呼び出し側が保つ。書く側は、範囲の長さがこの値と合う
    /// ことを確かめる (確かめた結果を別の範囲に使い回す誤りへの備え)。
    #[must_use]
    pub fn text_len(&self) -> usize {
        self.text_len
    }

    /// 元のバイト列。
    #[must_use]
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

fn reject(report: &mut EncodeReport, idx: u64, ch: char, kind: UnmappableKind) {
    report.push(Unmappable {
        char_index: idx,
        ch,
        kind,
    });
}

/// 文字コードごとのエンコーダ。`encode` をチャンクごとに呼び、最後に `finish` を呼ぶ。
pub struct Encoder {
    enc: Encoding,
    policy: EscapePolicy,
    pending_bom: bool,
    /// これまでに渡した文字数。
    chars_done: u64,
    utf8: Option<Utf8Encoder>,
    /// ISO-2022-JP: 改行までためている行と、その先頭の文字の通し番号。
    line: String,
    line_start: u64,
}

fn rs_encode(enc: Encoding, text: &str, last: bool, out: &mut Vec<u8>) -> Vec<(usize, char)> {
    // `Unmappable` になった文字の (text 内のバイト位置、文字) を返す。出力はその文字を飛ばして続ける
    let mut e = enc.rs().new_encoder();
    rs_encode_with(&mut e, text, last, out)
}

fn rs_encode_with(
    e: &mut encoding_rs::Encoder,
    text: &str,
    last: bool,
    out: &mut Vec<u8>,
) -> Vec<(usize, char)> {
    let mut bad = Vec::new();
    let mut pos = 0;
    // 書いた末尾。`out` は `end` より後ろを作業の領域として確保しておき、最後に切り詰める。表現不能の文字のたびに
    // 残り全体の大きさで確保し直す (ゼロで埋める) と、表現不能の多い行で O(n²) になる (SAF-22)
    let mut end = out.len();
    let mut grow = true;
    loop {
        let rest = &text[pos..];
        if grow {
            let want = e
                .max_buffer_length_from_utf8_if_no_unmappables(rest.len())
                .unwrap_or(rest.len() * 4 + 16);
            if out.len() < end + want {
                out.resize(end + want, 0);
            }
            grow = false;
        }
        let (res, read, written) =
            e.encode_from_utf8_without_replacement(rest, &mut out[end..], last);
        end += written;
        pos += read;
        match res {
            EncoderResult::InputEmpty => {
                out.truncate(end);
                return bad;
            }
            EncoderResult::OutputFull => grow = true,
            EncoderResult::Unmappable(ch) => {
                // ISO-2022-JP のエンコーダは U+000E / U+000F / U+001B を、元の文字ではなく U+FFFD として返す。
                // 位置と文字は、読み終えた位置の直前の文字から求める
                let actual = text[..pos].chars().next_back().unwrap_or(ch);
                bad.push((pos - actual.len_utf8(), actual));
            }
        }
    }
}

impl Encoder {
    /// `bom`: 先頭に BOM を書く (UTF-8 / UTF-16)。
    pub fn new(enc: Encoding, bom: bool, policy: EscapePolicy) -> Encoder {
        let utf8 = (enc == Encoding::Utf8).then(|| {
            Utf8Encoder::new(
                bom,
                match policy {
                    EscapePolicy::Restore => PuaEscapeMode::Enabled,
                    _ => PuaEscapeMode::DisabledCollision,
                },
            )
        });
        Encoder {
            enc,
            policy,
            pending_bom: bom && enc.is_utf16(),
            chars_done: 0,
            utf8,
            line: String::new(),
            line_start: 0,
        }
    }

    pub fn encoding(&self) -> Encoding {
        self.enc
    }

    /// `text` を符号化して `out` に追記する。表現不能は `report` に記録する (その文字は出力しない)。
    pub fn encode(&mut self, text: &str, out: &mut Vec<u8>, report: &mut EncodeReport) {
        let n_chars = text.chars().count() as u64;
        match self.enc {
            Encoding::Utf8 => self.encode_utf8(text, out, report),
            Encoding::Utf16Le | Encoding::Utf16Be => self.encode_utf16(text, out, report),
            Encoding::Iso2022Jp => self.encode_iso(text, out, report),
            Encoding::ShiftJis | Encoding::EucJp => self.encode_rs(text, out, report),
        }
        self.chars_done += n_chars;
    }

    /// 保持方式の記録の元のバイト列 `bytes` を、そのまま `out` に追記する (03 §8.6 E2)。`text` は記録の範囲の今の文字列。
    ///
    /// 書くのは、確認 (i) (`preserved_matches`) を満たし、ISO-2022-JP では符号化器が行頭 (ためている行が空。行ごとに
    /// 閉じるので ASCII 状態) にあるときだけ。書いたら文字の通し番号を `text` の文字数だけ進めて真を返す (範囲の文字は
    /// E1 の照合の対象にせず、表現不能に数えない)。書かないときは何もせずに偽を返す (呼び出し側は `text` を `encode`
    /// で符号化する)。ISO-2022-JP の「範囲が行の内容の全体である」ことは、行の終わりを見られる呼び出し側が確かめる。
    pub fn write_preserved(&mut self, text: &str, bytes: &[u8], out: &mut Vec<u8>) -> bool {
        PreservedBytes::check(self.enc, bytes, text).is_some_and(|p| self.write_checked(&p, out))
    }

    /// 確認 (i) を通った元のバイト列 (`PreservedBytes::check` で作ったもの) を、そのまま `out` に追記する
    /// (`write_preserved` から確認 (i) の復号を除いたもの。確認は型が保証する)。書くのは、確かめた文字コードがこの
    /// 符号化器の文字コードと同じで、ISO-2022-JP では符号化器が行頭にあるときだけ。書いたら文字の通し番号を確かめた
    /// 文字列の文字数だけ進めて真を返す。書かないときは何もせずに偽を返す (呼び出し側は文字列を `encode` で符号化する)。
    pub fn write_checked(&mut self, p: &PreservedBytes<'_>, out: &mut Vec<u8>) -> bool {
        let at_line_start = self.enc != Encoding::Iso2022Jp || self.line.is_empty();
        if p.enc != self.enc || !at_line_start {
            return false;
        }
        out.extend_from_slice(p.bytes);
        self.chars_done += p.chars;
        true
    }

    /// これまでに渡した文字の数 (`encode` と `write_preserved` の分。表現不能の位置の通し番号と同じ数え方)。
    pub fn chars_done(&self) -> u64 {
        self.chars_done
    }

    /// 最後に呼ぶ (ISO-2022-JP の最終行を ASCII 状態で閉じる)。
    pub fn finish(&mut self, out: &mut Vec<u8>, report: &mut EncodeReport) {
        if self.enc == Encoding::Iso2022Jp && !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            let start = self.line_start;
            self.flush_iso_line(&line, start, out, report);
        }
    }

    fn encode_utf8(&mut self, text: &str, out: &mut Vec<u8>, report: &mut EncodeReport) {
        let Some(u) = self.utf8.as_mut() else { return };
        if self.policy == EscapePolicy::Reject {
            // 別の文字コード由来のエスケープ文字は UTF-8 でも表現不能 (E4)
            let mut start = 0;
            for (i, (bi, c)) in text.char_indices().enumerate() {
                if unescape_char(c).is_some() {
                    u.encode(&text[start..bi], out);
                    reject(
                        report,
                        self.chars_done + i as u64,
                        c,
                        UnmappableKind::Unmappable,
                    );
                    start = bi + c.len_utf8();
                }
            }
            u.encode(&text[start..], out);
        } else {
            u.encode(text, out);
        }
    }

    fn encode_utf16(&mut self, text: &str, out: &mut Vec<u8>, report: &mut EncodeReport) {
        if std::mem::take(&mut self.pending_bom) {
            out.extend_from_slice(self.enc.bom().unwrap_or_default());
        }
        let le = self.enc == Encoding::Utf16Le;
        let mut buf = [0u16; 2];
        for (i, c) in text.chars().enumerate() {
            if let Some(b) = unescape_char(c) {
                match self.policy {
                    EscapePolicy::Restore => {
                        out.push(b);
                        continue;
                    }
                    EscapePolicy::Reject => {
                        reject(
                            report,
                            self.chars_done + i as u64,
                            c,
                            UnmappableKind::Unmappable,
                        );
                        continue;
                    }
                    EscapePolicy::AsText => {}
                }
            }
            for u in c.encode_utf16(&mut buf) {
                out.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() });
            }
        }
    }

    /// `Shift_JIS` / EUC-JP (状態を持たない)。
    fn encode_rs(&mut self, text: &str, out: &mut Vec<u8>, report: &mut EncodeReport) {
        let base = self.chars_done;
        // 特別な文字 (U+10FF00.. のエスケープは UTF-8 の先頭バイトが 0xF4、Shift_JIS の外字 U+E000..U+E757 は 0xEE) が
        // 1 つも無ければ、文字ごとの走査をせず全体を 1 区間として符号化する (100 MB の読込の往復検査が支配的だったため)
        let has_special = text
            .bytes()
            .any(|b| b == 0xF4 || (self.enc == Encoding::ShiftJis && b == 0xEE));
        if !has_special {
            self.run_rs(text, base, out, report);
            return;
        }
        // 特別な文字 (エスケープ・外字) で区切り、間を encoding_rs で符号化して照合する
        let mut run_start = 0;
        let mut chars_before_run = 0u64;
        for (ci, (bi, c)) in text.char_indices().enumerate() {
            let ci = ci as u64;
            let special = unescape_char(c).is_some()
                || (self.enc == Encoding::ShiftJis && cp932::gaiji_to_sjis(c).is_some());
            if special {
                self.run_rs(&text[run_start..bi], base + chars_before_run, out, report);
                if let Some(b) = unescape_char(c) {
                    match self.policy {
                        EscapePolicy::Restore => out.push(b),
                        _ => reject(report, base + ci, c, UnmappableKind::Unmappable),
                    }
                } else if let Some(b) = cp932::gaiji_to_sjis(c) {
                    out.extend_from_slice(&b);
                }
                run_start = bi + c.len_utf8();
                chars_before_run = ci + 1;
            }
        }
        self.run_rs(&text[run_start..], base + chars_before_run, out, report);
    }

    /// 特別な文字を含まない 1 区間を符号化して照合する。`first_idx` は区間の先頭の文字の通し番号。
    fn run_rs(&self, run: &str, first_idx: u64, out: &mut Vec<u8>, report: &mut EncodeReport) {
        if run.is_empty() {
            return;
        }
        let start = out.len();
        let bad = rs_encode(self.enc, run, true, out);
        // 表現不能が無く、出力を復号すると元に戻る区間 (ほとんどがこれ) は、照合の期待値と復号の結果の文字列を作らない
        // (区分 B の箇所が密な文書の保存は、記録の間の短い区間ごとにここを通る。報告するものが無いので結果は同じ)
        if bad.is_empty() && decodes_to(self.enc, &out[start..], run) {
            return;
        }
        // 表現不能 (encoding_rs が返したもの) と、照合で見つけた黙示の置換を、本文の順に合わせて報告する
        // (報告の先頭は「最初の表現不能の文字」として UI が使う。docs/review/v0.3-findings.md の SAF-47)
        // 文字の通し番号は、前の表現不能の文字からの差分だけを数える (表現不能ごとに先頭から数えない、SAF-22)
        let (mut counted_to, mut chars) = (0usize, 0u64);
        let unmappable: Vec<Unmappable> = bad
            .iter()
            .map(|&(bi, ch)| {
                chars += run[counted_to..bi].chars().count() as u64;
                counted_to = bi;
                Unmappable {
                    char_index: first_idx + chars,
                    ch,
                    kind: UnmappableKind::Unmappable,
                }
            })
            .collect();
        // 表現できた文字だけの文字列 (照合の期待値)
        let expected: String = if bad.is_empty() {
            run.to_owned()
        } else {
            let mut s = String::with_capacity(run.len());
            let mut last = 0;
            for &(bi, ch) in &bad {
                s.push_str(&run[last..bi]);
                last = bi + ch.len_utf8();
            }
            s.push_str(&run[last..]);
            s
        };
        let substituted = self.verify(&out[start..], &expected, run, &bad, first_idx);
        // どちらも本文の順に並んでいるので、位置の順に合わせながら積む
        let (mut a, mut b) = (
            unmappable.into_iter().peekable(),
            substituted.into_iter().peekable(),
        );
        loop {
            let next = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) if y.char_index < x.char_index => b.next(),
                (Some(_), _) => a.next(),
                (None, Some(_)) => b.next(),
                (None, None) => break,
            };
            if let Some(u) = next {
                report.push(u);
            }
        }
    }

    /// `bytes` を復号して `expected` と照合し、一致しない文字を `Substituted` として返す (本文の順)。
    fn verify(
        &self,
        bytes: &[u8],
        expected: &str,
        original: &str,
        bad: &[(usize, char)],
        first_idx: u64,
    ) -> Vec<Unmappable> {
        let mut found = Vec::new();
        // 復号すると期待値に戻る (黙示の置換が無い) なら、復号の結果の文字列を作らない
        if decodes_to(self.enc, bytes, expected) {
            return found;
        }
        let mut d = Decoder::new(self.enc, false);
        let mut got = String::new();
        d.decode(bytes, true, &mut got);
        if got == expected {
            return found;
        }
        // 文字ごとに対応づける (黙示の置換は 1 文字を 1 文字に写す)。表現不能で飛ばした文字は原文の側で飛ばす。
        // `bad` は位置の昇順なので、先頭から順に突き合わせる (文字ごとに全件を引かない、SAF-22)
        let mut bad_it = bad.iter().map(|b| b.0).peekable();
        let mut got_it = got.chars();
        let mut idx = first_idx;
        for (bi, c) in original.char_indices() {
            if bad_it.next_if_eq(&bi).is_some() {
                idx += 1;
                continue;
            }
            match got_it.next() {
                Some(g) if g == c => {}
                _ => found.push(Unmappable {
                    char_index: idx,
                    ch: c,
                    kind: UnmappableKind::Substituted,
                }),
            }
            idx += 1;
        }
        found
    }

    /// ISO-2022-JP: 行末までためて、行ごとに符号化する (E6)。
    fn encode_iso(&mut self, text: &str, out: &mut Vec<u8>, report: &mut EncodeReport) {
        let mut rest = text;
        let mut consumed_chars = 0u64;
        while !rest.is_empty() {
            if self.line.is_empty() {
                self.line_start = self.chars_done + consumed_chars;
            }
            match rest.find(['\n', '\r']) {
                Some(i) => {
                    self.line.push_str(&rest[..i]);
                    let eol = rest.as_bytes()[i];
                    let line = std::mem::take(&mut self.line);
                    let start = self.line_start;
                    self.flush_iso_line(&line, start, out, report);
                    out.push(eol);
                    consumed_chars += rest[..i].chars().count() as u64 + 1;
                    rest = &rest[i + 1..];
                }
                None => {
                    self.line.push_str(rest);
                    return;
                }
            }
        }
    }

    fn flush_iso_line(
        &mut self,
        line: &str,
        start: u64,
        out: &mut Vec<u8>,
        report: &mut EncodeReport,
    ) {
        // エスケープ文字 (U+10FF00..。UTF-8 の先頭バイトが 0xF4) を含まない行で、表現不能が無く、出力を復号すると元に
        // 戻るとき (ほとんどの行) は、下の照合のための文字列を作らない (報告するものが無いので結果は同じ)。そうでなければ
        // 出力を戻して、下の手順でやり直す
        if !line.as_bytes().contains(&0xF4) {
            let bytes_start = out.len();
            let bad = rs_encode(Encoding::Iso2022Jp, line, true, out);
            if bad.is_empty() && decodes_to(Encoding::Iso2022Jp, &out[bytes_start..], line) {
                return;
            }
            out.truncate(bytes_start);
        }
        // エスケープ文字 (不正バイト) は状態付き符号化では元のバイトに戻せない: 表現不能
        let mut clean = String::with_capacity(line.len());
        let mut kept: Vec<(usize, char)> = Vec::new(); // clean の各文字の元の位置
        for (i, c) in line.chars().enumerate() {
            if unescape_char(c).is_some() {
                reject(report, start + i as u64, c, UnmappableKind::Unmappable);
            } else {
                clean.push(c);
                kept.push((i, c));
            }
        }
        let bytes_start = out.len();
        let bad = rs_encode(Encoding::Iso2022Jp, &clean, true, out);
        let mut bad_idx: Vec<usize> = Vec::new();
        for &(bi, ch) in &bad {
            let ci = clean[..bi].chars().count();
            bad_idx.push(ci);
            reject(
                report,
                start + kept[ci].0 as u64,
                ch,
                UnmappableKind::Unmappable,
            );
        }
        // 照合: 表現できた文字だけを復号結果と比べる
        let mut d = Decoder::new(Encoding::Iso2022Jp, false);
        let mut got = String::new();
        d.decode(&out[bytes_start..], true, &mut got);
        let expected: String = clean
            .chars()
            .enumerate()
            .filter(|(i, _)| !bad_idx.contains(i))
            .map(|(_, c)| c)
            .collect();
        if got != expected {
            let mut g = got.chars();
            for (i, c) in clean.chars().enumerate() {
                if bad_idx.contains(&i) {
                    continue;
                }
                match g.next() {
                    Some(x) if x == c => {}
                    _ => reject(
                        report,
                        start + kept[i].0 as u64,
                        c,
                        UnmappableKind::Substituted,
                    ),
                }
            }
        }
    }
}

/// 照合なしで、この文字コードの出力長を数える必要がある呼び側のための補助 (乾式走査の出力長)。UTF-16 と UTF-8 だけ厳密。
pub fn utf16_len(text: &str, policy: EscapePolicy) -> usize {
    text.chars()
        .map(|c| match (unescape_char(c), policy) {
            (Some(_), EscapePolicy::Restore) => 1,
            _ => c.len_utf16() * 2,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utf8::escape_byte;

    fn enc(e: Encoding, bom: bool, p: EscapePolicy, chunks: &[&str]) -> (Vec<u8>, EncodeReport) {
        let mut en = Encoder::new(e, bom, p);
        let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
        for c in chunks {
            en.encode(c, &mut out, &mut rep);
        }
        en.finish(&mut out, &mut rep);
        (out, rep)
    }

    // 表現不能と黙示の置換が 1 行に多数あっても、通し番号と種類は本文の順 (SAF-22 で数え方を差分にした)。
    // 出力は表現できた文字だけで、表現不能の文字は飛ばす
    #[test]
    fn 表現不能と黙示の置換が多数並んでも位置は本文の順() {
        let text = "x\u{a5}\u{1F600}".repeat(40);
        let (out, rep) = enc(Encoding::ShiftJis, false, EscapePolicy::Reject, &[&text]);
        assert_eq!(rep.total, 80);
        let got: Vec<(u64, UnmappableKind)> = rep
            .recorded
            .iter()
            .map(|u| (u.char_index, u.kind))
            .collect();
        let want: Vec<(u64, UnmappableKind)> = (0..40u64)
            .flat_map(|i| {
                [
                    (3 * i + 1, UnmappableKind::Substituted),
                    (3 * i + 2, UnmappableKind::Unmappable),
                ]
            })
            .collect();
        assert_eq!(got, want);
        assert_eq!(out, b"x\\".repeat(40));
    }

    // fuzz_encode (#366) が CI で見つけた: 1 区間の中で、照合で見つけた黙示の置換 (Shift_JIS の ¥ は 0x5C に写り、
    // 復号すると \ になる) が、後ろの表現不能の文字より後に報告されていた。報告は本文の順に並べる (先頭は UI が
    // 「最初の表現不能の文字」として使う)。docs/review/v0.3-findings.md の SAF-47
    #[test]
    fn 表現不能と黙示の置換は本文の順に報告する() {
        for e in [Encoding::ShiftJis, Encoding::EucJp] {
            let (_, rep) = enc(e, false, EscapePolicy::Reject, &["a\u{a5}b\u{652}c"]);
            let got: Vec<(u64, UnmappableKind)> = rep
                .recorded
                .iter()
                .map(|u| (u.char_index, u.kind))
                .collect();
            assert_eq!(
                got,
                [
                    (1, UnmappableKind::Substituted),
                    (3, UnmappableKind::Unmappable)
                ],
                "{e:?}"
            );
        }
    }

    fn all(e: Encoding, text: &str) -> (Vec<u8>, EncodeReport) {
        enc(e, false, EscapePolicy::Restore, &[text])
    }

    #[test]
    fn shift_jis_と_euc_jp_に書く() {
        let (b, r) = all(Encoding::ShiftJis, "日本語abc");
        assert_eq!(b, b"\x93\xFA\x96\x7B\x8C\xEAabc");
        assert!(r.is_empty());
        let (b, r) = all(Encoding::EucJp, "日本");
        assert_eq!(b, b"\xC6\xFC\xCB\xDC");
        assert!(r.is_empty());
    }

    #[test]
    fn 表現不能と黙示の置換を位置つきで報告する() {
        // 絵文字は Unmappable、¥ は 0x5C に黙って写される、− は 0x817C に写される
        let (_, r) = all(Encoding::ShiftJis, "a😀b¥c−d");
        let kinds: Vec<(u64, char, UnmappableKind)> = r
            .recorded
            .iter()
            .map(|u| (u.char_index, u.ch, u.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (1, '😀', UnmappableKind::Unmappable),
                (3, '¥', UnmappableKind::Substituted),
                (5, '−', UnmappableKind::Substituted),
            ]
        );
        assert_eq!(r.total, 3);
    }

    #[test]
    fn iso_2022_jp_の_so_si_esc_はパニックせず表現不能として報告する() {
        // encoding_rs は U+000E / U+000F / U+001B を U+FFFD として返す (読み進めた位置から元の文字を求める)
        for c in ['\u{e}', '\u{f}', '\u{1b}'] {
            let (_, r) = all(Encoding::Iso2022Jp, &format!("a{c}b"));
            assert_eq!(r.total, 1, "{c:?}");
            assert_eq!((r.recorded[0].char_index, r.recorded[0].ch), (1, c));
        }
    }

    #[test]
    fn 位置はチャンクをまたいで通し番号() {
        let (_, r) = enc(
            Encoding::ShiftJis,
            false,
            EscapePolicy::Restore,
            &["ab", "c😀"],
        );
        assert_eq!(r.recorded[0].char_index, 3);
    }

    #[test]
    fn 互換文字は_unmappable() {
        for c in ['〜', '‖', '¢', '£', '¬', '¦', '—'] {
            let (_, r) = all(Encoding::ShiftJis, &c.to_string());
            assert_eq!(r.total, 1, "{c} は Shift_JIS で表現不能");
        }
    }

    #[test]
    fn cp932_の外字は逆算して書く() {
        // U+E000 → 0xF040、U+E757 → 0xF9FC、U+E03F → 0xF07E、U+E040 → 0xF080
        assert_eq!(cp932::gaiji_to_sjis('\u{E000}'), Some([0xF0, 0x40]));
        assert_eq!(cp932::gaiji_to_sjis('\u{E757}'), Some([0xF9, 0xFC]));
        assert_eq!(cp932::gaiji_to_sjis('\u{E03E}'), Some([0xF0, 0x7E]));
        assert_eq!(cp932::gaiji_to_sjis('\u{E03F}'), Some([0xF0, 0x80]));
        assert_eq!(cp932::gaiji_to_sjis('\u{E758}'), None);
        let (b, r) = all(Encoding::ShiftJis, "a\u{E000}b");
        assert_eq!(b, b"a\xF0\x40b");
        assert!(r.is_empty());
        // 復号して同じ文字に戻る
        let mut d = Decoder::new(Encoding::ShiftJis, false);
        let mut s = String::new();
        d.decode(&b, true, &mut s);
        assert_eq!(s, "a\u{E000}b");
        // EUC-JP では外字は表現不能
        let (_, r) = all(Encoding::EucJp, "\u{E000}");
        assert_eq!(r.total, 1);
    }

    #[test]
    fn 外字は全域で往復する() {
        for v in cp932::GAIJI_FIRST..=cp932::GAIJI_LAST {
            let c = char::from_u32(v).unwrap();
            let [l, t] = cp932::gaiji_to_sjis(c).unwrap();
            let mut d = Decoder::new(Encoding::ShiftJis, false);
            let mut s = String::new();
            d.decode(&[l, t], true, &mut s);
            assert_eq!(s, c.to_string(), "{v:#X}");
        }
    }

    #[test]
    fn 不正バイトのエスケープは元のバイトに戻す() {
        let t = format!("a{}b{}", escape_byte(0xA0), escape_byte(0x81));
        let (b, r) = all(Encoding::ShiftJis, &t);
        assert_eq!(b, b"a\xA0b\x81");
        assert!(r.is_empty());
        let (b, r) = enc(Encoding::ShiftJis, false, EscapePolicy::Reject, &[&t]);
        assert_eq!(b, b"ab");
        assert_eq!(r.total, 2, "別の文字コードで保存するときは表現不能");
    }

    #[test]
    fn utf16_は自前で書き_bom_とサロゲートを扱う() {
        let (b, r) = enc(Encoding::Utf16Le, true, EscapePolicy::Restore, &["aあ😀"]);
        assert_eq!(b, b"\xFF\xFEa\x00\x42\x30\x3D\xD8\x00\xDE");
        assert!(r.is_empty());
        let (b, _) = enc(Encoding::Utf16Be, true, EscapePolicy::Restore, &["a"]);
        assert_eq!(b, b"\xFE\xFF\x00a");
        // 不正バイトは元のバイトに戻る (孤立サロゲート・奇数長の再現)
        let t = format!("{}{}", escape_byte(0x00), escape_byte(0xD8));
        let (b, _) = enc(Encoding::Utf16Le, false, EscapePolicy::Restore, &[&t]);
        assert_eq!(b, b"\x00\xD8");
    }

    #[test]
    fn utf16_のエンコーダは_encoding_rs_を使わない() {
        // encoding_rs の UTF-16 の output_encoding は UTF-8 (黙って UTF-8 で書かれるのを防ぐ、§8.6)
        assert_eq!(encoding_rs::UTF_16LE.output_encoding(), encoding_rs::UTF_8);
        let (b, _) = enc(Encoding::Utf16Le, false, EscapePolicy::Restore, &["a"]);
        assert_eq!(b, b"a\x00");
    }

    #[test]
    fn iso_2022_jp_は行ごとに_ascii_状態で閉じる() {
        let (b, r) = all(Encoding::Iso2022Jp, "日本\nabc\n語");
        assert_eq!(b, b"\x1B$BF|K\\\x1B(B\nabc\n\x1B$B8l\x1B(B".to_vec());
        assert!(r.is_empty());
    }

    #[test]
    fn iso_2022_jp_のローマ字状態は行末で閉じる() {
        // `"¥\nabc"` → 行ごとに閉じるので `ESC ( J 5C ESC ( B 0A abc`
        let (b, r) = all(Encoding::Iso2022Jp, "¥\nabc");
        assert_eq!(b, b"\x1B(J\x5C\x1B(B\nabc".to_vec());
        assert!(r.is_empty());
    }

    #[test]
    fn iso_2022_jp_はチャンクの境目をまたいでも同じ() {
        let whole = all(Encoding::Iso2022Jp, "日本語abc\n日本語\r\nx").0;
        let t = "日本語abc\n日本語\r\nx";
        for cut in t.char_indices().map(|(i, _)| i) {
            let (a, b) = t.split_at(cut);
            let (got, _) = enc(Encoding::Iso2022Jp, false, EscapePolicy::Restore, &[a, b]);
            assert_eq!(got, whole, "cut {cut}");
        }
    }

    #[test]
    fn iso_2022_jp_の半角カナは黙って全角にせず表現不能() {
        let (_, r) = all(Encoding::Iso2022Jp, "ｱb");
        assert_eq!(r.total, 1);
        assert_eq!(r.recorded[0].ch, 'ｱ');
        assert_eq!(r.recorded[0].kind, UnmappableKind::Substituted);
    }

    #[test]
    fn iso_2022_jp_はエスケープ文字を元のバイトに戻さない() {
        let t = format!("a{}", escape_byte(0x0A));
        let (_, r) = all(Encoding::Iso2022Jp, &t);
        assert_eq!(r.total, 1);
    }

    #[test]
    fn utf8_は_reject_でエスケープ文字だけ表現不能() {
        let t = format!("あ{}", escape_byte(0x80));
        let (b, r) = enc(Encoding::Utf8, false, EscapePolicy::Reject, &[&t]);
        assert_eq!(b, "あ".as_bytes());
        assert_eq!(r.total, 1);
        let (b, r) = enc(Encoding::Utf8, false, EscapePolicy::Restore, &[&t]);
        assert_eq!(b, b"\xE3\x81\x82\x80");
        assert!(r.is_empty());
    }

    // 保持方式の記録の元のバイト列を書く口 (03 §8.6 E2): 確認 (i) を口の中で行い、通し番号を範囲の文字数だけ進める
    #[test]
    fn 記録の元のバイト列をそのまま書き_通し番号を進める() {
        let mut e = Encoder::new(Encoding::ShiftJis, false, EscapePolicy::Restore);
        let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
        e.encode("a", &mut out, &mut rep);
        // ￢ の重複符号化 (IBM 拡張 0xFA54。符号化すると 0x81CA)
        assert!(e.write_preserved("￢￢", b"\xFA\x54\xFA\x54", &mut out));
        e.encode("😀", &mut out, &mut rep);
        e.finish(&mut out, &mut rep);
        assert_eq!(out, b"a\xFA\x54\xFA\x54");
        assert_eq!(rep.total, 1);
        assert_eq!(rep.recorded[0].char_index, 3, "記録の 2 文字を数える");
        assert_eq!(e.chars_done(), 4);
        // EUC-JP の補助漢字 (復号専用): 記録で書けば表現不能にならない
        let mut e = Encoder::new(Encoding::EucJp, false, EscapePolicy::Restore);
        let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
        assert!(e.write_preserved("丂", b"\x8F\xB0\xA1", &mut out));
        e.encode("丂", &mut out, &mut rep);
        assert_eq!(out, b"\x8F\xB0\xA1");
        assert_eq!((rep.total, rep.recorded[0].char_index), (1, 1));
    }

    #[test]
    fn 確認_i_に合わない記録は書かない() {
        let mut e = Encoder::new(Encoding::ShiftJis, false, EscapePolicy::Restore);
        let mut out = Vec::new();
        // 別の文字のバイト列・途中で切れたバイト列・空のバイト列
        assert!(!e.write_preserved("￢", b"\xFA\x55", &mut out));
        assert!(!e.write_preserved("￢", b"\xFA", &mut out));
        assert!(!e.write_preserved("", b"", &mut out));
        assert_eq!(out, b"");
        assert_eq!(e.chars_done(), 0);
        // UTF-8 / UTF-16 は記録を使わない
        let mut e = Encoder::new(Encoding::Utf8, false, EscapePolicy::Restore);
        assert!(!e.write_preserved("a", b"a", &mut out));
        assert!(!preserved_matches(Encoding::Utf16Le, b"a\0", "a"));
    }

    #[test]
    fn iso_2022_jp_の記録は行頭で_ascii_状態で終わるときだけ書く() {
        let mut e = Encoder::new(Encoding::Iso2022Jp, false, EscapePolicy::Reject);
        let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
        // 旧 JIS の「あ」の行 (ASCII 状態で終わる)
        assert!(e.write_preserved("あ", b"\x1B$@$\"\x1B(B", &mut out));
        e.encode("\nx", &mut out, &mut rep);
        // 行の途中 (ためている行が空でない) では書かない
        assert!(!e.write_preserved("あ", b"\x1B$@$\"\x1B(B", &mut out));
        e.encode("\n", &mut out, &mut rep);
        // ASCII 状態で終わらない元のバイト列は書かない (半角カナの状態のまま)
        assert!(!e.write_preserved("ｱ", b"\x1B(I1", &mut out));
        assert!(e.write_preserved("ｱ", b"\x1B(I1\x1B(B", &mut out));
        e.finish(&mut out, &mut rep);
        assert_eq!(out, b"\x1B$@$\"\x1B(B\nx\n\x1B(I1\x1B(B");
        assert!(rep.is_empty());
        // 冗長なエスケープだけの行 (本文では空の行) も書ける
        let mut e = Encoder::new(Encoding::Iso2022Jp, false, EscapePolicy::Reject);
        let mut out = Vec::new();
        assert!(e.write_preserved("", b"\x1B(B", &mut out));
        assert_eq!(out, b"\x1B(B");
    }

    // 確認 (i) を通ったことを型で運ぶ口 (`PreservedBytes` / `write_checked`): 確認 (i) に合わないバイト列からは作れず、
    // 作ったものは別の文字コードの符号化器と、行の途中の ISO-2022-JP の符号化器には書けない
    #[test]
    fn 確認を通った記録だけを書き_文字コードと行頭を見る() {
        assert!(PreservedBytes::check(Encoding::ShiftJis, b"\xFA\x55", "￢").is_none());
        assert!(PreservedBytes::check(Encoding::ShiftJis, b"\xFA", "￢").is_none());
        assert!(PreservedBytes::check(Encoding::ShiftJis, b"", "").is_none());
        assert!(PreservedBytes::check(Encoding::Utf8, b"a", "a").is_none());
        assert!(PreservedBytes::check(Encoding::Iso2022Jp, b"\x1B(I1", "ｱ").is_none());
        let p = PreservedBytes::check(Encoding::ShiftJis, b"\xFA\x54\xFA\x54", "￢￢")
            .expect("確認 (i) に合う");
        assert_eq!(p.bytes(), b"\xFA\x54\xFA\x54");
        assert!(!format!("{p:?}").contains("250"), "Debug は中身を出さない");
        // 別の文字コードの符号化器には書かない (EUC-JP の符号化器に Shift_JIS で確かめたもの)
        let mut e = Encoder::new(Encoding::EucJp, false, EscapePolicy::Restore);
        let mut out = Vec::new();
        assert!(!e.write_checked(&p, &mut out));
        assert_eq!((out.len(), e.chars_done()), (0, 0));
        // 同じ文字コードなら、そのまま書いて通し番号を文字数だけ進める (`write_preserved` と同じ出力)
        let mut e = Encoder::new(Encoding::ShiftJis, false, EscapePolicy::Restore);
        let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
        e.encode("a", &mut out, &mut rep);
        assert!(e.write_checked(&p, &mut out));
        e.encode("😀", &mut out, &mut rep);
        e.finish(&mut out, &mut rep);
        assert_eq!(out, b"a\xFA\x54\xFA\x54");
        assert_eq!(rep.recorded[0].char_index, 3);
        // ISO-2022-JP: 行の途中では書かない
        let q = PreservedBytes::check(Encoding::Iso2022Jp, b"\x1B$@$\"\x1B(B", "あ")
            .expect("確認 (i) に合う");
        let mut e = Encoder::new(Encoding::Iso2022Jp, false, EscapePolicy::Reject);
        let (mut out, mut rep) = (Vec::new(), EncodeReport::default());
        e.encode("x", &mut out, &mut rep);
        assert!(!e.write_checked(&q, &mut out));
        e.encode("\n", &mut out, &mut rep);
        assert!(e.write_checked(&q, &mut out));
        e.finish(&mut out, &mut rep);
        assert_eq!(out, b"x\n\x1B$@$\"\x1B(B");
    }

    // 照合の速い経路 (表現不能が無く、出力を復号すると戻る区間は文字列を作らない) が入っても、出力と報告は区切り方に
    // 依らない (黙示の置換・表現不能・エスケープ・外字が混ざる文字列を、一括と 1 文字ずつで符号化して比べる。短い区間ほど
    // 速い経路を多く通る)。黙示の置換が速い経路で落ちないこと (報告の中身) は、上の個別の試験が見る
    #[test]
    fn 照合の速い経路でも報告は同じ() {
        let pieces = [
            "a",
            "日本",
            "￢",
            "纊",
            "−",
            "¥",
            "ｱ",
            "丂",
            "😀",
            "\u{E000}",
            "\n",
            "\u{10FF81}",
        ];
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for enc in [Encoding::ShiftJis, Encoding::EucJp, Encoding::Iso2022Jp] {
            let policy = if enc.is_stateful() {
                EscapePolicy::Reject
            } else {
                EscapePolicy::Restore
            };
            for _ in 0..300 {
                let n = 1 + (next() % 20) as usize;
                let text: String = (0..n).map(|_| pieces[(next() % 12) as usize]).collect();
                let (whole, rw) = {
                    let mut e = Encoder::new(enc, false, policy);
                    let (mut o, mut r) = (Vec::new(), EncodeReport::default());
                    e.encode(&text, &mut o, &mut r);
                    e.finish(&mut o, &mut r);
                    (o, r)
                };
                // 文字ごとに区切って渡しても同じ (区間が短いほど速い経路を多く通る)
                let mut e = Encoder::new(enc, false, policy);
                let (mut o, mut r) = (Vec::new(), EncodeReport::default());
                for (i, c) in text.char_indices() {
                    e.encode(&text[i..i + c.len_utf8()], &mut o, &mut r);
                }
                e.finish(&mut o, &mut r);
                assert_eq!((&o, &r), (&whole, &rw), "{enc:?} {text:?}");
            }
        }
    }

    #[test]
    fn 記録は百件まで_総数は数える() {
        let t = "😀".repeat(150);
        let (_, r) = all(Encoding::ShiftJis, &t);
        assert_eq!((r.recorded.len(), r.total), (100, 150));
        assert_eq!(r.escapes, 0);
    }

    #[test]
    fn エスケープ文字の数は記録した百件より後ろも数える() {
        // 絵文字 101 個の後ろにエスケープ文字が 2 つ (記録した先頭 100 件には入らない)
        let t = format!("{}\u{10FF80}x\u{10FF9A}", "😀".repeat(101));
        let mut e = Encoder::new(Encoding::ShiftJis, false, EscapePolicy::Reject);
        let (mut out, mut r) = (Vec::new(), EncodeReport::default());
        e.encode(&t, &mut out, &mut r);
        e.finish(&mut out, &mut r);
        assert_eq!((r.recorded.len(), r.total, r.escapes), (100, 103, 2));
        assert!(r.recorded.iter().all(|u| unescape_char(u.ch).is_none()));
    }
}
