//! ワークスペース (docs/03 §12.7、07 §12.1。v0.5 の M31)。
//!
//! - [`root`]: ルートの検出 (`JobKind::WorkspaceRoot`、03 §12.7.1) と、core が持つワークスペースの状態、起動の引数の
//!   ディレクトリの知らせの保留。
//!
//! spec: 03 §12.7.1, 07 §12.1, 07 §18.2, 07 §18.3

pub(crate) mod root;

pub use root::{RootSource, Workspace};
