//! リテラル検索の基盤 (docs/03 §12、01 FR-401・FR-402・FR-404)。
//!
//! 検索語は 1 行の入力欄から来るので改行を含まない。一致は行をまたがないため、**行ごと**に走査する
//! (チャンクの境界を気にしなくてよい)。一致は重ならないものを左から順に取り、両端が書記素境界でない候補は捨てる (§12.1)。
//!
//! 大小を区別しない照合は、1 文字ずつ小文字へ畳み込んだ文字列どうしで行う。畳み込めるのは小文字が 1 文字になる
//! 文字だけで、それ以外 (`İ` など) はそのままにする。これは Unicode の単純ケースフォールディングとほぼ同じだが、
//! `ſ` (U+017F) と `s` は同一視しない (§12.4 の実装メモ)。
//!
//! spec: 03 §12

mod matcher;
mod replace;
mod scan;

pub use matcher::{Case, Matcher, QueryError, SearchQuery};
pub use replace::{build_replace_all, replacement_for, unescape_replacement};
#[cfg(test)]
use scan::scan_lines;
pub use scan::{SearchJob, find_next, run_search, scan_bytes};

pub(crate) mod search_state;

#[cfg(test)]
mod tests;
