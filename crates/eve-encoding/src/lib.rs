//! 文字コードの判定、チャンク単位のデコード / エンコード、encode→decode 照合 (docs/02 §4.1、docs/03 §8)。
//!
//! 現在は M1 の範囲の UTF-8 だけ: `detect` (BOM・バイナリ・UTF-8 の判定)、`Utf8Decoder` / `Utf8Encoder`
//! (不正バイトの私用文字エスケープと、保存時の復元)。他の文字コード (UTF-16・Shift_JIS・EUC-JP・ISO-2022-JP) は
//! それぞれの版で足す。他の `eve-*` crate に依存しない。I/O をしない。

#![forbid(unsafe_code)]

mod decode;
mod detect;
mod encode;
mod encoding;
#[doc(hidden)]
pub mod fuzzing;
mod roundtrip;
mod utf8;

pub use decode::Decoder;
pub use detect::{
    Confidence, DetectOptions, DetectedKind, Detection, ForcedEncoding, UnsupportedBom, detect,
    find_iso_switch, redetect_window,
};
pub use encode::{
    EncodeReport, Encoder, EscapePolicy, MAX_RECORDED, Unmappable, UnmappableKind, cp932, utf16_len,
};
pub use encoding::Encoding;
pub use roundtrip::{
    MAX_LINE, MAX_POSITIONS, RoundtripChecker, RoundtripSummary, check as check_roundtrip,
};
pub use utf8::{ESCAPE_BASE, PuaEscapeMode, Utf8Decoder, Utf8Encoder, escape_byte, unescape_char};
