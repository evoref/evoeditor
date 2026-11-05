//! 意味的 UI モデル: オーバーレイ (確認ダイアログ・入力プロンプト・パレット・ヘルプ)、メッセージ行、ステータスバーとヒントバー、
//! 検索バー、設定の診断の通知、保存中の終了の確認の状態遷移 (docs/02 §4.6 の `ui_model/`)。
//!
//! spec: 04 §8, 04 §15, 03 §9.7

pub(crate) mod chrome;
pub(crate) mod config_report;
pub(crate) mod help;
pub(crate) mod message;
pub(crate) mod overlay;
pub(crate) mod palette;
pub(crate) mod quit_saving;
pub(crate) mod search_bar;
