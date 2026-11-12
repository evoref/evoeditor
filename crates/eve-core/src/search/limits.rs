//! 検索の大きさと最悪の入力の上限 (docs/03 §12.3 / §12.3.1 の「実行」と「最悪の入力の上限」)。
//!
//! 値は M24 の (2) で決めた初めの値。基準機での計測 (M24 の (6)) で直してよい (03 §12.3.1 の「具体の値は実装の PR で基準機で
//! 測って決め、定数で書く」)。

/// 行の窓の核の大きさ (バイト。行の境界か char 境界に揃える前の目安、§12.3.1 の「実行」)。
pub const SEARCH_WINDOW_CORE_BYTES: usize = 1 << 20;

/// 重い検索語の行の窓の、始めの核の大きさ (バイト)。目安の 1/4 より速ければ倍にして `SEARCH_WINDOW_CORE_BYTES` まで戻す
/// (§12.3.1 の「取り消しの遅れ」)。
pub const SEARCH_HEAVY_CORE_BYTES: usize = 16 << 10;

/// 全文を連結して探す文書の大きさの上限 (UTF-8 のバイト数)。行の窓の大きさの上限も同じ値 (§12.3.1)。
pub const SEARCH_CONCAT_MAX_BYTES: usize = 16 << 20;

/// 検索のジョブの取り消しの遅れの目安 (ミリ秒)。行の窓の 1 つの処理がこれを超えたら、核を縮める (§12.3.1)。
pub const SEARCH_CANCEL_LATENCY_MS: u64 = 50;

/// 重い検索語の判定: 上限のある繰り返しの上限 (入れ子は掛け合わせる) がこれを超える検索語を重いとする (§12.3.1 の
/// 「重い検索語」)。
pub const SEARCH_HEAVY_REPEAT: u64 = 1000;

/// 正規表現のコンパイル後の大きさの上限 (§12.3)。
pub const SEARCH_REGEX_SIZE_LIMIT: usize = 10 << 20;

/// 遅延 DFA のキャッシュの大きさ (`dfa_size_limit`。`regex` の既定の値 (2 MiB) に頼らず明示する、§12.3.1)。2 MiB では、
/// 有界の繰り返しを含む普通の検索語 (`[^\n\r]{0,1500}zq` など) でキャッシュがあふれて `PikeVM` に落ち、100 倍以上遅くなる
/// (M24 の (2) のレビューでの実測)。
pub const SEARCH_DFA_SIZE_LIMIT: usize = 8 << 20;

/// 処理の速さの下限 (バイト / 秒)。直近 `SEARCH_SLOW_JUDGE_MS` 以上の間に進んだ核の量がこれを下回ったら打ち切る (§12.3.1)。
pub const SEARCH_MIN_THROUGHPUT: u64 = 1_000_000;

/// 処理の速さの下限を判定する間隔 (ミリ秒。この時間がたつごとに、その間の量と時間で判定する)。
pub const SEARCH_SLOW_JUDGE_MS: u64 = 1_000;

/// 書記素の境界・単語単位で捨てた候補のために `regex` が読んだ量の予算 (走査する範囲の何倍か、§12.3.1)。
pub const SEARCH_DISCARD_BUDGET_FACTOR: usize = 8;

/// 捨てた候補の予算の下限 (バイト)。小さな範囲では倍数の予算が小さすぎて、数 KiB の文書でも打ち切ってしまうのを避ける。
pub const SEARCH_DISCARD_BUDGET_MIN: usize = 1 << 20;
