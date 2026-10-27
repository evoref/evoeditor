//! `cargo xtask check-dist`: 配布ビルドにテスト専用の経路が無いことと、CRT が静的リンクであることの機械的な確認
//! (docs/09-testing-quality.md §6.2 の `build-release-smoke`、docs/05 §16.4)。
//!
//! 検査: (1) 配布ビルドの feature に `test-hooks` / `bench-trace` / `headless` が現れない (`cargo tree -e features`)、
//! (2) exe に test-hooks と bench-trace の目印の文字列が無い、(3) 隠しオプション `--bench-exit-after-first-frame` が
//! 未知のオプションとして終了コード 2 で終わる、(4) exe の取り込む DLL に VC ランタイムと UCRT が無い、
//! (5) exe のヘッダの DependentLoadFlags が System32 だけから DLL を読む設定 (0x800) になっている (NFR-502 AC-2 (a))。
//! あわせて BM-205 (exe のサイズ) を `target/dist/size.json` に書く。
//!
//! `--exe <パス>` を付けると、ビルドと (1) を飛ばして、そのファイルを検査する (`release.yml` の展開後の exe 用)。

use std::path::PathBuf;
use std::process::Command;

use crate::util::{run_capture, workspace_root};

/// 配布ビルドに現れてはならない feature。
const FORBIDDEN_FEATURES: &[&str] = &["test-hooks", "bench-trace", "headless"];

/// 配布ビルドの exe に現れてはならない文字列 (UTF-8 と UTF-16LE の両方で探す)。
const FORBIDDEN_STRINGS: &[&str] = &[
    "EVE_TEST_HOOK",
    "internal.debug_panic",
    "--bench-",
    "EVE_TRACE_FILE",
    "EVE_TRACE_FORMAT",
];

/// exe の取り込む DLL の名前に現れてはならない接頭辞 (小文字)。`+crt-static` なら取り込まない。
const FORBIDDEN_DLLS: &[&str] = &["vcruntime", "msvcp", "api-ms-win-crt", "ucrtbase", "msvcr"];

pub fn run(args: &[String]) -> Result<(), String> {
    let root = workspace_root();
    let given = args
        .iter()
        .position(|a| a == "--exe")
        .map(|i| {
            args.get(i + 1)
                .map(PathBuf::from)
                .ok_or("--exe にはパスが要ります".to_owned())
        })
        .transpose()?;
    let exe = match given {
        Some(p) => p,
        None => {
            println!("▶ 配布ビルド (cargo build --release -p evoeditor)");
            run_capture(Command::new("cargo").args(["build", "--release", "-p", "evoeditor"]))?;
            println!("▶ (1) 配布ビルドの feature");
            let tree = run_capture(Command::new("cargo").args([
                "tree",
                "-p",
                "evoeditor",
                "-e",
                "normal,build,features",
                "--prefix",
                "none",
            ]))?;
            let found = forbidden_features(&tree);
            if !found.is_empty() {
                return Err(format!("配布ビルドに有効な feature があります: {found:?}"));
            }
            root.join("target/release").join(if cfg!(windows) {
                "evoeditor.exe"
            } else {
                "evoeditor"
            })
        }
    };
    let bytes = std::fs::read(&exe).map_err(|e| format!("{}: {e}", exe.display()))?;

    println!("▶ (2) 目印の文字列");
    let found = forbidden_strings(&bytes);
    if !found.is_empty() {
        return Err(format!(
            "{} に目印の文字列があります: {found:?}",
            exe.display()
        ));
    }

    println!("▶ (3) 隠しオプションが未知のオプション (終了コード 2)");
    let out = Command::new(&exe)
        .arg("--bench-exit-after-first-frame")
        .output()
        .map_err(|e| format!("{}: 実行できません: {e}", exe.display()))?;
    if out.status.code() != Some(2) {
        return Err(format!(
            "--bench-exit-after-first-frame の終了コードが 2 ではありません: {:?}",
            out.status.code()
        ));
    }

    println!("▶ (4) CRT の静的リンク");
    let dlls = forbidden_dlls(&bytes);
    if !dlls.is_empty() {
        return Err(format!(
            "CRT を動的に取り込んでいます (+crt-static が効いていない): {dlls:?}"
        ));
    }

    println!("▶ (5) DLL の検索を System32 に限る (DependentLoadFlags)");
    match dependent_load_flags(&bytes) {
        Some(f) if f & LOAD_LIBRARY_SEARCH_SYSTEM32 != 0 => {}
        other => {
            return Err(format!(
                "DependentLoadFlags に LOAD_LIBRARY_SEARCH_SYSTEM32 (0x800) がありません: {other:?} \
                 (.cargo/config.toml の /DEPENDENTLOADFLAG:0x800)"
            ));
        }
    }

    let size = bytes.len();
    let dir = root.join("target/dist");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("size.json");
    std::fs::write(&path, format!("{{\"id\":\"BM-205\",\"bytes\":{size}}}\n"))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    println!("check-dist: OK (exe {size} バイト → {})", path.display());
    Ok(())
}

/// `cargo tree -e features` の出力から、禁止された feature を探す。
/// LOAD_LIBRARY_SEARCH_SYSTEM32: 起動時のインポートの DLL を System32 だけから探す。
const LOAD_LIBRARY_SEARCH_SYSTEM32: u16 = 0x800;

/// PE (64 ビット) のロード構成ディレクトリの DependentLoadFlags (オフセット 78)。読めなければ None。
fn dependent_load_flags(pe: &[u8]) -> Option<u16> {
    let u16_at = |o: usize| pe.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| {
        pe.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let e_lfanew = u32_at(0x3c)? as usize;
    if pe.get(e_lfanew..e_lfanew + 4)? != [b'P', b'E', 0, 0] {
        return None;
    }
    let nsec = u16_at(e_lfanew + 6)? as usize;
    let opt_size = u16_at(e_lfanew + 20)? as usize;
    let opt = e_lfanew + 24;
    if u16_at(opt)? != 0x20b {
        return None; // PE32+ (x86_64) だけ
    }
    // データディレクトリ 10 = ロード構成
    let lc_rva = u32_at(opt + 112 + 10 * 8)? as usize;
    if lc_rva == 0 {
        return None;
    }
    let sections = opt + opt_size;
    let to_file = |rva: usize| -> Option<usize> {
        (0..nsec).find_map(|i| {
            let o = sections + i * 40;
            let (vsize, va, rsize, raw) = (
                u32_at(o + 8)? as usize,
                u32_at(o + 12)? as usize,
                u32_at(o + 16)? as usize,
                u32_at(o + 20)? as usize,
            );
            (va <= rva && rva < va + vsize.max(rsize)).then(|| rva - va + raw)
        })
    };
    u16_at(to_file(lc_rva)? + 78)
}

fn forbidden_features(tree: &str) -> Vec<&'static str> {
    FORBIDDEN_FEATURES
        .iter()
        .copied()
        .filter(|f| tree.contains(&format!("feature \"{f}\"")))
        .collect()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// 禁止された文字列 (UTF-8 または UTF-16LE) が入っているもの。
fn forbidden_strings(bytes: &[u8]) -> Vec<&'static str> {
    FORBIDDEN_STRINGS
        .iter()
        .copied()
        .filter(|s| {
            let wide: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
            contains(bytes, s.as_bytes()) || contains(bytes, &wide)
        })
        .collect()
}

/// 取り込む DLL の名前 (`<名前>.dll`) のうち、禁止された接頭辞で始まるもの。
/// インポートテーブルの DLL 名は ASCII で置かれるので、`.dll` で終わる ASCII の並びを拾えば足りる。
fn forbidden_dlls(bytes: &[u8]) -> Vec<String> {
    let mut found = Vec::new();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..i + 4].eq_ignore_ascii_case(b".dll") {
            let mut start = i;
            while start > 0 && is_dll_char(bytes[start - 1]) {
                start -= 1;
            }
            let name = String::from_utf8_lossy(&bytes[start..i + 4]).to_ascii_lowercase();
            if FORBIDDEN_DLLS.iter().any(|p| name.starts_with(p)) && !found.contains(&name) {
                found.push(name);
            }
            i += 4;
        } else {
            i += 1;
        }
    }
    found
}

fn is_dll_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_は_tree_の行から探す() {
        let tree = "evoeditor v0.0.0\neve-core v0.0.0\neve-core feature \"default\"\n";
        assert!(forbidden_features(tree).is_empty());
        let tree = "evoeditor feature \"test-hooks\"\n";
        assert_eq!(forbidden_features(tree), ["test-hooks"]);
    }

    #[test]
    fn 目印の文字列は_utf8_でも_utf16_でも見つける() {
        assert!(forbidden_strings(b"abc EVE_TEST_HOOK xyz").contains(&"EVE_TEST_HOOK"));
        let wide: Vec<u8> = "EVE_TRACE_FILE"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(forbidden_strings(&wide), ["EVE_TRACE_FILE"]);
        assert!(forbidden_strings(b"nothing here").is_empty());
    }

    // verifies: AC-NFR-502-2
    #[test]
    fn 配布ビルドの_dll_の検索は_system32_に限る() {
        // リポジトリの設定で作った配布ビルドの exe を見る
        let exe = crate::util::workspace_root().join("target/release/evoeditor.exe");
        let Ok(bytes) = std::fs::read(&exe) else {
            return; // 配布ビルドがまだ無いときは、check-dist (CI の build-release-smoke) が見る
        };
        let f = dependent_load_flags(&bytes).expect("ロード構成を読める");
        assert!(f & LOAD_LIBRARY_SEARCH_SYSTEM32 != 0, "{f:#x}");
    }

    #[test]
    fn crt_の_dll_だけを拾う() {
        let bytes = b"\0KERNEL32.dll\0VCRUNTIME140.dll\0api-ms-win-crt-runtime-l1-1-0.dll\0api-ms-win-core-synch-l1-2-0.dll\0";
        assert_eq!(
            forbidden_dlls(bytes),
            ["vcruntime140.dll", "api-ms-win-crt-runtime-l1-1-0.dll"]
        );
    }
}
