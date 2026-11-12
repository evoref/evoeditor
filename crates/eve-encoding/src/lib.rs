//! 文字コードの判定、チャンク単位のデコード / エンコード、encode→decode 照合 (docs/02 §4.1、docs/03 §8)。
//!
//! `detect` (BOM・バイナリ・文字コードの判定)、`Decoder` / `Encoder` (UTF-8・UTF-16・Shift_JIS・EUC-JP・ISO-2022-JP)、
//! `Utf8Decoder` / `Utf8Encoder` (不正バイトの私用文字エスケープと、保存時の復元)、往復の検査と保持方式の記録
//! (`RoundtripChecker`、`PreservedRecords`)。
//! 他の `eve-*` crate に依存しない。I/O をしない。

#![forbid(unsafe_code)]

mod decode;
mod detect;
mod encode;
mod encoding;
#[doc(hidden)]
pub mod fuzzing;
mod preserve;
mod roundtrip;
mod utf8;

pub use decode::Decoder;
pub use detect::{
    Confidence, DetectOptions, DetectedKind, Detection, ForcedEncoding, UnsupportedBom, detect,
    find_iso_switch, redetect_window,
};
pub use encode::{
    EncodeReport, Encoder, EscapePolicy, MAX_RECORDED, PreservedBytes, Unmappable, UnmappableKind,
    cp932, preserved_matches, utf16_len,
};
pub use encoding::Encoding;
pub use preserve::{
    MAX_PRESERVED_BYTES, MAX_PRESERVED_RECORDS, PreserveLimits, PreserveReason, PreserveState,
    PreservedRecord, PreservedRecords,
};
pub use roundtrip::{
    MAX_LINE, MAX_POSITIONS, NoPreserveReason, PreserveStatus, RoundtripChecker, RoundtripSummary,
    check as check_roundtrip, check_with_records as check_roundtrip_with_records,
};
pub use utf8::{ESCAPE_BASE, PuaEscapeMode, Utf8Decoder, Utf8Encoder, escape_byte, unescape_char};
