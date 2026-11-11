//! 文書の形式情報 (docs/02 §5.1、docs/03 §8.1 / §9.6): 改行の統計、`TextFormat`、読み取り専用の理由、読込状態。

use eve_encoding::{Encoding, EscapePolicy, PreserveStatus, PuaEscapeMode, RoundtripSummary};
use eve_text::{ByteOff, LineEnding, TextRead};

/// 改行の種類別の数。`CRLF` は 1 つと数える (03 §2.4)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EolStats {
    /// LF の数。
    pub lf: usize,
    /// CRLF の数。
    pub crlf: usize,
    /// 単独の CR の数。
    pub cr: usize,
}

/// 長大行の閾値の既定 (`file.long_line_threshold`)。これを超える行は char 近似モードで扱う (NFR-106、03 §11.5.1)。
pub const LONG_LINE_THRESHOLD: usize = 1 << 20;

/// 閾値 `threshold` (バイト) を超える行の数。チャンクを 1 回走査する (読込ジョブと `Document::from_text` で使う)。
pub fn count_long_lines(text: &dyn TextRead, threshold: usize) -> usize {
    let (mut run, mut n) = (0usize, 0usize);
    for chunk in text.chunks(eve_text::ByteRange::all(text.len_bytes())) {
        let b = chunk.as_bytes();
        let mut at = 0;
        while let Some(i) = memchr::memchr2(b'\n', b'\r', &b[at..]) {
            n += usize::from(run + i > threshold);
            run = 0;
            at += i + 1;
        }
        run += b.len() - at;
    }
    n + usize::from(run > threshold)
}

/// 改行の統計と長大行の数を、文書を前から順に渡して 1 回の走査で数える (読込ジョブの復号のループで使う。
/// docs/review/v0.3-findings.md の PRF-17)。結果は [`EolStats::count`] と [`count_long_lines`] に同じ。
#[derive(Debug)]
pub struct LineStatsScanner {
    stats: EolStats,
    long_lines: usize,
    threshold: usize,
    /// 今の行の、ここまでのバイト数。
    run: usize,
    /// 直前に渡された分が CR で終わった (次の先頭が LF なら CRLF)。
    prev_cr: bool,
}

impl LineStatsScanner {
    /// `threshold` (バイト) を超える行を長大行として数える。
    pub fn new(threshold: usize) -> LineStatsScanner {
        LineStatsScanner {
            stats: EolStats::default(),
            long_lines: 0,
            threshold,
            run: 0,
            prev_cr: false,
        }
    }

    /// 文書の続きを渡す (分け方は任意。空でもよい)。
    pub fn feed(&mut self, chunk: &str) {
        let b = chunk.as_bytes();
        let mut i = 0;
        if self.prev_cr && !b.is_empty() {
            self.prev_cr = false;
            if b[0] == b'\n' {
                self.stats.crlf += 1;
                i = 1;
            } else {
                self.stats.cr += 1;
            }
        }
        while let Some(off) = memchr::memchr2(b'\r', b'\n', &b[i..]) {
            let p = i + off;
            self.long_lines += usize::from(self.run + off > self.threshold);
            self.run = 0;
            i = p + 1;
            if b[p] == b'\n' {
                self.stats.lf += 1;
            } else {
                match b.get(i) {
                    Some(b'\n') => {
                        self.stats.crlf += 1;
                        i += 1;
                    }
                    Some(_) => self.stats.cr += 1,
                    None => self.prev_cr = true,
                }
            }
        }
        self.run += b.len() - i;
    }

    /// 改行の統計と、長大行の数。
    pub fn finish(mut self) -> (EolStats, usize) {
        if self.prev_cr {
            self.stats.cr += 1;
        }
        (
            self.stats,
            self.long_lines + usize::from(self.run > self.threshold),
        )
    }
}

/// 位置の範囲 `ranges` (`[s, e]`。昇順。文書の範囲に切り詰める) が触れる行のうち、閾値を超える行の数
/// (同じ行は 1 回だけ数える)。編集の前後の文書で数え、長大行の数を差分で更新するのに使う。
pub(crate) fn long_lines_touched(
    text: &dyn TextRead,
    ranges: impl Iterator<Item = (usize, usize)>,
    threshold: usize,
) -> usize {
    let len = text.len_bytes();
    let (mut next, mut n) = (0usize, 0usize);
    for (s, e) in ranges {
        let a = text.byte_to_line(ByteOff(s.min(len))).0.max(next);
        let b = text.byte_to_line(ByteOff(e.min(len))).0;
        for l in a..=b {
            let l = eve_text::LineIdx(l);
            n += usize::from(text.line_end_byte(l).0 - text.line_to_byte(l).0 > threshold);
        }
        next = next.max(b + 1);
    }
    n
}

impl EolStats {
    /// 文書全体を数える (読込時)。
    pub fn count(text: &dyn TextRead) -> EolStats {
        count_range(text, 0, text.len_bytes())
    }

    /// 改行の総数。
    pub fn total(&self) -> usize {
        self.lf + self.crlf + self.cr
    }

    /// 2 種類以上があるか (ステータスバーの「混在」)。
    pub fn is_mixed(&self) -> bool {
        [self.lf, self.crlf, self.cr]
            .iter()
            .filter(|&&n| n > 0)
            .count()
            >= 2
    }

    /// 多数派の改行。同数なら LF。改行が全くなければ `fallback` (設定の既定、03 §8.4)。
    pub fn majority(&self, fallback: LineEnding) -> LineEnding {
        if self.total() == 0 {
            return fallback;
        }
        if self.crlf > self.lf && self.crlf >= self.cr {
            LineEnding::Crlf
        } else if self.cr > self.lf && self.cr > self.crlf {
            LineEnding::Cr
        } else {
            LineEnding::Lf
        }
    }

    /// `self - removed + added` (各成分を 0 で止める)。
    pub(crate) fn replaced(self, removed: EolStats, added: EolStats) -> EolStats {
        let f = |cur: usize, rem: usize, add: usize| {
            let v = cur.saturating_add(add);
            debug_assert!(v >= rem, "eol_stats が負になった");
            v.saturating_sub(rem)
        };
        EolStats {
            lf: f(self.lf, removed.lf, added.lf),
            crlf: f(self.crlf, removed.crlf, added.crlf),
            cr: f(self.cr, removed.cr, added.cr),
        }
    }
}

fn byte_at(text: &dyn TextRead, i: usize) -> Option<u8> {
    if i >= text.len_bytes() {
        return None;
    }
    let (chunk, start) = text.chunk_at(ByteOff(i));
    chunk.as_bytes().get(i - start.0).copied()
}

/// `[start, end)` の改行を数える。範囲の端が CRLF の `\r` と `\n` の間に当たるときは 1 バイト外へ広げる
/// (03 §6.5「`eol_stats` の更新」)。
pub(crate) fn count_range(text: &dyn TextRead, start: usize, end: usize) -> EolStats {
    let len = text.len_bytes();
    let (mut s, mut e) = (start.min(len), end.min(len));
    if s > 0 && byte_at(text, s) == Some(b'\n') && byte_at(text, s - 1) == Some(b'\r') {
        s -= 1;
    }
    if e > 0 && e < len && byte_at(text, e - 1) == Some(b'\r') && byte_at(text, e) == Some(b'\n') {
        e += 1;
    }
    let mut st = EolStats::default();
    // CR / LF だけを探して飛ぶ (本文の大半は改行でないので、1 バイトずつ見ない。10 MB の長大行で数 ms 縮む)
    let mut prev_cr = false;
    for chunk in text.chunks(eve_text::ByteRange::new(ByteOff(s), ByteOff(e))) {
        let bytes = chunk.as_bytes();
        let mut i = 0;
        if prev_cr {
            // 前のチャンクの末尾の CR: このチャンクの先頭が LF なら CRLF
            prev_cr = false;
            if bytes.first() == Some(&b'\n') {
                st.crlf += 1;
                i = 1;
            } else {
                st.cr += 1;
            }
        }
        while let Some(off) = memchr::memchr2(b'\r', b'\n', &bytes[i..]) {
            let p = i + off;
            if bytes[p] == b'\n' {
                st.lf += 1;
                i = p + 1;
            } else {
                match bytes.get(p + 1) {
                    Some(b'\n') => {
                        st.crlf += 1;
                        i = p + 2;
                    }
                    Some(_) => {
                        st.cr += 1;
                        i = p + 1;
                    }
                    None => {
                        prev_cr = true;
                        i = p + 1;
                    }
                }
            }
        }
    }
    if prev_cr {
        st.cr += 1;
    }
    st
}

/// 書式と読み取り専用の理由の組。再読込 (`Origin::Reload`) が本文と一緒に差し替え、Undo / Redo が本文と一緒に戻す (03 §7.2)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatState {
    pub format: TextFormat,
    pub read_only: ReadOnlyReasons,
    /// 保持方式の記録の表 (03 §8.7.1。v0.4)。再読込は表を新しい読込の結果に入れ替え、Undo / Redo で前後の表に戻す
    /// (状態は `format.roundtrip.preserve`)。表の複製は O(1)。
    pub(crate) preserved: super::preserved::PreservedMap,
}

/// 保持方式の記録の表と状態の組 (03 §8.7.1)。復旧 (`Origin::Recovery`) の前後の値として Undo のノードが持つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreservedState {
    /// 記録の表 (複製は O(1))。
    pub(crate) map: super::preserved::PreservedMap,
    /// 状態 (`TextFormat.roundtrip.preserve` に戻す)。
    pub status: PreserveStatus,
}

/// Undo ノードに持つ書式の変更 (前後の値)。Undo は `before`、Redo は `after` へ戻す。戻す範囲は種類で決まる (03 §7.2)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatChange {
    /// 再読込 (`Origin::Reload`、03 §9.5): `TextFormat` 全体と読み取り専用の理由。
    Reload {
        before: FormatState,
        after: FormatState,
    },
    /// 改行コードの変換 (`file.set_line_ending`、03 §8.4): `default_eol` だけ。
    /// 文字コード・BOM・往復の情報・読み取り専用の理由は戻さない (`eol_stats` は本文の適用で数え直す)。
    DefaultEol {
        before: LineEnding,
        after: LineEnding,
    },
    /// 復旧 (`Origin::Recovery`、03 §10.3 / §8.7.1): 保持方式の記録の表と状態だけ。`Document::apply` が作る
    /// (復旧の後は表が空で「保持なし (復旧した)」、Undo で復旧の前の表と状態に戻る)。
    Recovery {
        before: PreservedState,
        after: PreservedState,
    },
}

/// Undo / Redo で本文と一緒に戻す書式 (`FormatChange` の片側)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatRestore {
    State(FormatState),
    DefaultEol(LineEnding),
    Preserved(PreservedState),
}

impl FormatChange {
    /// Undo なら `before`、Redo (と最初の適用) なら `after` の側。
    pub fn side(&self, after: bool) -> FormatRestore {
        match self {
            FormatChange::Reload { before, after: a } => {
                FormatRestore::State(if after { a.clone() } else { before.clone() })
            }
            FormatChange::DefaultEol { before, after: a } => {
                FormatRestore::DefaultEol(if after { *a } else { *before })
            }
            FormatChange::Recovery { before, after: a } => {
                FormatRestore::Preserved(if after { a.clone() } else { before.clone() })
            }
        }
    }

    /// 前後の保持方式の記録の表の元のバイト列のバイト数 (Undo の予算に数える、03 §7.5)。
    pub(crate) fn preserved_bytes(&self) -> usize {
        match self {
            FormatChange::Reload { before, after } => {
                before.preserved.total_bytes() + after.preserved.total_bytes()
            }
            FormatChange::Recovery { before, after } => {
                before.map.total_bytes() + after.map.total_bytes()
            }
            FormatChange::DefaultEol { .. } => 0,
        }
    }
}

/// 文書の形式 (03 §8.1)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextFormat {
    /// 文字コード (保存先の既定)。
    pub encoding: Encoding,
    /// BOM の種類は `encoding` から決まる (UTF-8 / UTF-16LE / UTF-16BE)。保存時に同じものを書く。
    pub has_bom: bool,
    /// 新規に挿入する改行 (文書全体の改行コードではない)。
    pub default_eol: LineEnding,
    /// 文書の中の改行の種類別の数 (編集で差分を更新する)。
    pub eol_stats: EolStats,
    /// 不正バイトの保持マーカーの数。0 でなければ読み取り専用 (NFR-304)。
    pub invalid_bytes: u32,
    /// 不正バイトの PUA エスケープの方式 (03 §8.3)。
    pub pua_escape: PuaEscapeMode,
    /// 読込時の往復の検査 (03 §8.7。v0.1 は件数と先頭の位置だけ)。
    pub roundtrip: RoundtripSummary,
    /// 開いたときの長大行の閾値 (`file.long_line_threshold`)。設定を変えても開いている文書には遡らない (07 §6)。
    pub long_line_threshold: usize,
    /// 閾値を超える行の数 (`eol_stats` と同じく文書から数え、編集で差分を更新する)。0 でなければ折り返さない (03 §11.5.1)。
    pub long_lines: usize,
}

impl TextFormat {
    /// 新規文書 (無題・存在しないパス)。`default_eol` は全 OS で LF。
    pub fn new_document() -> TextFormat {
        TextFormat {
            encoding: Encoding::Utf8,
            has_bom: false,
            default_eol: LineEnding::Lf,
            eol_stats: EolStats::default(),
            invalid_bytes: 0,
            pua_escape: PuaEscapeMode::Enabled,
            roundtrip: RoundtripSummary::default(),
            long_line_threshold: LONG_LINE_THRESHOLD,
            long_lines: 0,
        }
    }

    /// 文字コード `out` で保存するときの、不正バイトのエスケープ文字の扱い (03 §8.3、E4)。
    ///
    /// - 元の文字コードのまま保存し、文書が不正バイトを持つとき: エスケープ文字を元のバイトに戻す (読込時の方式のまま)。
    /// - 別の文字コードへ保存するとき: エスケープ文字は元の文字コードのバイトなので、別の文字コードの中には書けない
    ///   (書くと前後のバイトとずれる・結合する)。表現不能として報告する。元のテキストが真にその範囲の文字を
    ///   含む (`DisabledCollision`) ときは通常の文字として書く。
    /// - 不正バイトの無い文書: エスケープ文字は読込で作ったものではない (他の文書から貼り付けた等) ので、
    ///   元のバイトに戻さず表現不能として報告する (docs/review/v0.3-findings.md の SAF-02b。貼り付けの時点で
    ///   `<0xNN>` の表記に変える 03 §8.3 の「編集」の行の実装までの暫定)。
    pub fn save_escape_policy(&self, out: Encoding) -> EscapePolicy {
        match self.pua_escape {
            PuaEscapeMode::DisabledCollision => EscapePolicy::AsText,
            PuaEscapeMode::Enabled if out == self.encoding && self.invalid_bytes > 0 => {
                EscapePolicy::Restore
            }
            _ => EscapePolicy::Reject,
        }
    }
}

/// 読み取り専用の理由 (03 §9.6)。宣言順が表示の優先順。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ReadOnlyReason {
    /// ファイルの読み取り専用属性 (`FileMeta.readonly`)。`file.toggle_read_only` で解除できる。
    FileAttribute = 0,
    /// 利用者が読み取り専用を求めた (`--readonly` など)。`file.toggle_read_only` で解除できる。
    UserRequested,
    /// 保存で書込の権限が無いと分かった (`PermissionDenied`)。
    NoWritePermission,
    /// 不正バイトを含む (`invalid_bytes > 0`、NFR-304)。
    InvalidBytes,
    /// 状態付き符号化 (ISO-2022-JP) で不正バイトを含む (03 §8.3)。
    StatefulEncodingWithInvalidBytes,
    /// バイナリの疑いを確認して開いた。
    Binary,
    /// 巨大ファイルを確認して開いた (FR-127)。
    LargeFileViewer,
    /// 読込が途中で終わった部分だけを開いた (予約。v0.1 では付けない)。
    LoadIncomplete,
}

const ALL_REASONS: [ReadOnlyReason; 8] = [
    ReadOnlyReason::FileAttribute,
    ReadOnlyReason::UserRequested,
    ReadOnlyReason::NoWritePermission,
    ReadOnlyReason::InvalidBytes,
    ReadOnlyReason::StatefulEncodingWithInvalidBytes,
    ReadOnlyReason::Binary,
    ReadOnlyReason::LargeFileViewer,
    ReadOnlyReason::LoadIncomplete,
];

/// 理由の集合 (ビットセット)。「読込中」は理由にしない (`LoadState` から導出する)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadOnlyReasons(u16);

impl ReadOnlyReasons {
    /// 理由を加える。
    pub fn insert(&mut self, r: ReadOnlyReason) {
        self.0 |= 1 << (r as u8);
    }

    /// 理由を外す。
    pub fn remove(&mut self, r: ReadOnlyReason) {
        self.0 &= !(1 << (r as u8));
    }

    /// 理由を含む。
    pub fn contains(&self, r: ReadOnlyReason) -> bool {
        self.0 & (1 << (r as u8)) != 0
    }

    /// 理由が 1 つも無い。
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// 優先順に返す。
    pub fn iter(&self) -> impl Iterator<Item = ReadOnlyReason> + '_ {
        ALL_REASONS.into_iter().filter(|r| self.contains(*r))
    }

    /// `file.toggle_read_only` で解除できるか (理由があり、すべてが `FileAttribute` か `UserRequested`。03 §9.6 の表)。
    pub fn releasable(&self) -> bool {
        !self.is_empty()
            && self.iter().all(|r| {
                matches!(
                    r,
                    ReadOnlyReason::FileAttribute | ReadOnlyReason::UserRequested
                )
            })
    }

    /// 表示に出す 1 つ (優先順で先頭)。理由が無ければ `None`。
    pub fn primary(&self) -> Option<ReadOnlyReason> {
        self.iter().next()
    }
}

/// 文書の読込の状態 (03 §9.2)。`Ready` 以外は編集できない。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoadState {
    /// 読込中 (再読込中は旧本文を表示し続ける)。
    Loading,
    /// 読込を終えた (新規・無題の文書を含む)。
    #[default]
    Ready,
    /// 読込に失敗した。
    Failed,
}

#[cfg(test)]
mod tests {
    use eve_text::Text;

    use super::*;

    #[test]
    fn 改行を数える() {
        let t = Text::from("a\nb\r\nc\rd\r\n\n");
        assert_eq!(
            EolStats::count(&t),
            EolStats {
                lf: 2,
                crlf: 2,
                cr: 1
            }
        );
        assert!(EolStats::count(&t).is_mixed());
        assert_eq!(EolStats::count(&Text::from("abc")), EolStats::default());
        assert_eq!(
            EolStats::count(&Text::from("\r\r\n\n\r")),
            EolStats {
                lf: 1,
                crlf: 1,
                cr: 2
            }
        );
    }

    #[test]
    fn 多数派() {
        let s = |lf, crlf, cr| EolStats { lf, crlf, cr };
        assert_eq!(
            s(0, 0, 0).majority(LineEnding::Crlf),
            LineEnding::Crlf,
            "改行がなければ設定"
        );
        assert_eq!(
            s(2, 2, 0).majority(LineEnding::Crlf),
            LineEnding::Lf,
            "同数なら LF"
        );
        assert_eq!(s(1, 3, 0).majority(LineEnding::Lf), LineEnding::Crlf);
        assert_eq!(s(1, 0, 3).majority(LineEnding::Lf), LineEnding::Cr);
        assert_eq!(
            s(0, 2, 2).majority(LineEnding::Lf),
            LineEnding::Crlf,
            "CRLF と CR が同数なら CRLF"
        );
    }

    #[test]
    fn 範囲の端が_crlf_を割るときは広げる() {
        let t = Text::from("a\r\nb");
        // [2, 4) は '\n' から始まる → '\r' を含めて CRLF 1 つ
        assert_eq!(
            count_range(&t, 2, 4),
            EolStats {
                lf: 0,
                crlf: 1,
                cr: 0
            }
        );
        // [0, 2) は '\r' で終わる → '\n' を含めて CRLF 1 つ
        assert_eq!(
            count_range(&t, 0, 2),
            EolStats {
                lf: 0,
                crlf: 1,
                cr: 0
            }
        );
    }

    #[test]
    fn 読み取り専用の理由は優先順() {
        let mut r = ReadOnlyReasons::default();
        assert!(r.is_empty() && r.primary().is_none());
        r.insert(ReadOnlyReason::LoadIncomplete);
        r.insert(ReadOnlyReason::InvalidBytes);
        r.insert(ReadOnlyReason::FileAttribute);
        assert_eq!(r.primary(), Some(ReadOnlyReason::FileAttribute));
        assert_eq!(
            r.iter().collect::<Vec<_>>(),
            [
                ReadOnlyReason::FileAttribute,
                ReadOnlyReason::InvalidBytes,
                ReadOnlyReason::LoadIncomplete
            ]
        );
        r.remove(ReadOnlyReason::FileAttribute);
        assert_eq!(r.primary(), Some(ReadOnlyReason::InvalidBytes));
        assert!(!r.contains(ReadOnlyReason::FileAttribute));
    }

    /// 基準: 1 バイトずつ数える。
    fn count_naive(bytes: &[u8]) -> EolStats {
        let mut st = EolStats::default();
        let mut prev_cr = false;
        for &b in bytes {
            if prev_cr {
                prev_cr = false;
                if b == b'\n' {
                    st.crlf += 1;
                    continue;
                }
                st.cr += 1;
            }
            match b {
                b'\r' => prev_cr = true,
                b'\n' => st.lf += 1,
                _ => {}
            }
        }
        if prev_cr {
            st.cr += 1;
        }
        st
    }

    #[test]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "試験の乱数を小さい剰余で添字にする"
    )]
    fn 改行の数え方は_1_バイトずつ数えた結果と同じ() {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let alphabet = ["a", "\r", "\n", "日", "\r\n", "b"];
        for case in 0..60 {
            // 長さを変える (ropey のチャンクをまたぐ長さを含む)。CR / LF の多い列と少ない列
            let len = [0usize, 1, 7, 100, 3000, 40_000][case % 6];
            let dense = case % 2 == 0;
            let mut s = String::new();
            for _ in 0..len {
                let k = if dense {
                    next() % 6
                } else {
                    [0, 0, 0, 0, 3, 5][(next() % 6) as usize]
                };
                s.push_str(alphabet[k as usize % 6]);
                if !dense && next() % 50 == 0 {
                    s.push_str(alphabet[1 + (next() % 3) as usize % 3]);
                }
            }
            let t = Text::from(s.as_str());
            assert_eq!(
                EolStats::count(&t),
                count_naive(s.as_bytes()),
                "case {case}"
            );
            // 範囲の途中から (CR と LF の間で切る場合を含む)
            for _ in 0..10 {
                if s.is_empty() {
                    break;
                }
                let a = (next() as usize) % s.len();
                let b = a + (next() as usize) % (s.len() - a + 1);
                if !s.is_char_boundary(a) || !s.is_char_boundary(b) {
                    continue;
                }
                let got = count_range(&t, a, b);
                // 基準: 範囲が CR と LF の間で切れていれば広げる (count_range と同じ約束)
                let (mut ra, mut rb) = (a, b);
                let bytes = s.as_bytes();
                if ra > 0 && bytes.get(ra) == Some(&b'\n') && bytes[ra - 1] == b'\r' {
                    ra -= 1;
                }
                if rb > 0 && rb < bytes.len() && bytes[rb - 1] == b'\r' && bytes[rb] == b'\n' {
                    rb += 1;
                }
                assert_eq!(got, count_naive(&bytes[ra..rb]), "case {case} {a}..{b}");
            }
        }
    }

    /// 文書を `cuts` の位置 (char 境界へ寄せる) で分けて渡した結果。
    fn scan_split(doc: &str, cuts: &[usize], threshold: usize) -> (EolStats, usize) {
        let mut sc = LineStatsScanner::new(threshold);
        let mut at = 0;
        for &c in cuts {
            let mut c = c.min(doc.len()).max(at);
            while !doc.is_char_boundary(c) {
                c += 1;
            }
            sc.feed(&doc[at..c]);
            at = c;
        }
        sc.feed(&doc[at..]);
        sc.finish()
    }

    #[test]
    fn 順に渡して数えた結果は_全文を数えた結果と同じ() {
        // CRLF の間・CR の直後・行の途中で分ける。空の文字列を渡しても変わらない
        let doc = "ab\r\ncd\re\n\n\r\r\nlong-long-line\r";
        let t = Text::from(doc);
        for threshold in [0, 1, 2, 5, 13, 14, 100] {
            let want = (EolStats::count(&t), count_long_lines(&t, threshold));
            assert_eq!(
                scan_split(doc, &[], threshold),
                want,
                "分けない {threshold}"
            );
            for a in 0..=doc.len() {
                for b in a..=doc.len() {
                    assert_eq!(
                        scan_split(doc, &[a, a, b], threshold),
                        want,
                        "{a} {b} {threshold}"
                    );
                }
            }
        }
        // 空の文書
        assert_eq!(scan_split("", &[], 0), (EolStats::default(), 0));
    }

    proptest::proptest! {
        #[test]
        fn 順に渡して数えた結果は_どの分け方でも同じ(
            parts in proptest::collection::vec(
                proptest::prop_oneof![
                    proptest::strategy::Just("\n".to_owned()),
                    proptest::strategy::Just("\r\n".to_owned()),
                    proptest::strategy::Just("\r".to_owned()),
                    "[a-zあ😀]{0,12}",
                ],
                0..60,
            ),
            mut cuts in proptest::collection::vec(0usize..400, 0..12),
            threshold in 0usize..24,
        ) {
            let doc = parts.concat();
            let t = Text::from(doc.as_str());
            cuts.sort_unstable();
            proptest::prop_assert_eq!(
                scan_split(&doc, &cuts, threshold),
                (EolStats::count(&t), count_long_lines(&t, threshold))
            );
        }
    }
}
