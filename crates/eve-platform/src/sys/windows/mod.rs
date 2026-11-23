//! Windows の API の安全な包み (R5)。保存・直接上書き・保存中フラグ・スワップの走査などのアルゴリズムの側は unsafe を持たず、
//! ここの関数だけを呼ぶ。
//!
//! - `wide`: `…W` 関数に渡す NUL 終端の UTF-16 と、長いパスの拡張長形式 (NFR-404)。
//! - `file`: ファイルの作成・削除・fsync・属性・名前の変更・置換 (`ReplaceFileW`)・領域の確保・ハンドルの識別・空き容量・ドライブの種類。
//! - `security`: 現在のユーザー・トークンの既定の所有者・パスの所有者の SID、所有者のみの DACL、所有者のみのディレクトリ。
//! - `owner`: 所有者の検査 (07 §12.1) のディレクトリの固定・属性と再解析のタグ・所有者と DACL・最終のパス・「自分たち」の SID。
//! - `process`: プロセスの作成時刻・他のプロセスの生存の問い合わせ・自プロセスの強制終了 (テストのフック)。

#![allow(unsafe_code)]

mod file;
pub(crate) mod owner;
mod process;
mod security;
mod wide;

pub(crate) use file::{
    attributes, create_new, delete, file_info, flush, free_bytes, is_char_or_pipe, is_remote,
    is_remote_handle, move_no_replace, rename_no_replace, replace_file_once, set_allocation_size,
    set_attributes, trusted_id_fs, volume_info,
};
#[cfg(feature = "test-hooks")]
pub(crate) use process::kill_self;
pub(crate) use process::{ProcessState, process_start, process_state};
pub(crate) use security::{OwnerOnly, create_dir_private, current_user_sid, owned_by_me};
