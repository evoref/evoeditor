//! View の操作: Selection の移動・選択の拡張 (Motion)、マウス、View の表示属性のコマンド (docs/02 §4.6 の `view/`)。
//! View と Selection の格納そのものは `editor::state` が持つ (STR-B2 で部分構造化するときに見直す)。
//!
//! spec: 03 §4, 03 §5, 01 FR-307, 07 §11.6 (移動・選択・`view.*`)

pub(crate) mod motion;
pub(crate) mod mouse;
pub(crate) mod view_cmds;
