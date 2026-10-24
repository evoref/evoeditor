//! DLL の検索を System32 に限る (docs/05 §16、docs/09 §8.2 の (a)、01 NFR-502 AC-2)。
//!
//! 起動時のインポートは、リンカーオプション `/DEPENDENTLOADFLAG:0x800` (`.cargo/config.toml`) で System32 に限っている。
//! これは、実行時の `LoadLibrary` (依存の crate が遅延で読む DLL を含む) にも同じ制限を掛ける。`main` の最初に呼ぶ。

#![allow(unsafe_code)]

use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
};

/// 以後の DLL の読込の検索先を System32 だけにする。成功なら `true`。
pub fn restrict_search_to_system32() -> bool {
    // SAFETY: 引数は定数のフラグだけで、ポインタを渡さない。プロセス全体の設定を変えるだけ。
    unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) != 0 }
}
