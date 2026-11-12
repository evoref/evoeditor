//! 文書内検索の基盤 (docs/03 §12、01 FR-401・FR-402・FR-404)。
//!
//! 検索語は 1 行の入力欄から来るので改行を含まない。一致は行をまたがないため、**行ごと**に走査する
//! (チャンクの境界を気にしなくてよい)。一致は重ならないものを左から順に取り、両端が書記素境界でない候補は捨てる (§12.1)。
//!
//! 大小を区別しない照合の意味は Unicode の単純ケースフォールディング (`regex` の `(?i)` と同じ。`k` と K (U+212A)、
//! `s` と `ſ` (U+017F) も一致する)。ASCII だけで `k` `K` `s` `S` を含まない検索語は ASCII の大小を畳み込んだ memmem で、
//! それ以外は検索語をエスケープした正規表現の `(?i)` で探す (§12.4)。
//!
//! 正規表現は、検索語を AST の段で書き換えてから `regex` に渡す (§12.3.1、`rewrite`)。CRLF を CR と LF に分けて
//! 一致させず、行の終端を明示した書き方だけが行の終端に一致する。
//!
//! spec: 03 §12

mod lines;
mod matcher;
mod replace;
mod rewrite;
mod scan;

pub(crate) use matcher::Prepared;
pub use matcher::{Case, Matcher, QueryError, QueryPlan, SearchQuery};
pub use replace::{build_replace_all, replacement_for, unescape_replacement};
pub use rewrite::LineSpan;
#[cfg(test)]
use scan::scan_lines;
pub use scan::{SearchJob, find_next, run_search, scan_bytes};

pub(crate) mod search_state;

#[cfg(test)]
mod tests;
