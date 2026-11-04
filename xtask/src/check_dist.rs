//! `cargo xtask check-dist`: 配布ビルドにテスト専用の経路が無いことと、CRT が静的リンクであることの機械的な確認
//! (docs/09-testing-quality.md §6.2 の `build-release-smoke`、docs/05 §16.2 / §16.4)。
//!
//! 検査: (1) 配布ビルドの feature に `test-hooks` / `bench-trace` / `headless` が現れない (`cargo tree -e features`)、
//! (2) exe に test-hooks と bench-trace の目印の文字列が無い、(3) 隠しオプション `--bench-exit-after-first-frame` が
//! 未知のオプションとして終了コード 2 で終わる、(4) 静的リンク: Windows (PE) は取り込む DLL に VC ランタイムと UCRT が無い、
//! Linux (ELF、musl) はプログラムインタプリタ (`PT_INTERP`) と共有ライブラリの依存 (`DT_NEEDED`) が無い、
//! (5) Windows だけ: exe のヘッダの DependentLoadFlags が System32 だけから DLL を読む設定 (0x800) になっている (NFR-502 AC-2 (a))。
//! あわせて BM-205 (exe のサイズ) を `target/dist/size.json` に書く。
//!
//! `--exe <パス>` を付けると、ビルドと (1) を飛ばして、そのファイルを検査する (`release.yml` の展開後の exe 用)。
//! `--target <三つ組>` を付けると、そのターゲットで配布ビルドを作って検査する (Linux の musl 版、05 §16.2)。

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
    let target = args
        .iter()
        .position(|a| a == "--target")
        .map(|i| {
            args.get(i + 1)
                .cloned()
                .ok_or("--target には三つ組が要ります".to_owned())
        })
        .transpose()?;
    let exe = match given {
        Some(p) => p,
        None => {
            let mut build = vec!["build", "--release", "-p", "evoeditor"];
            let mut tree_args = vec![
                "tree",
                "-p",
                "evoeditor",
                "-e",
                "normal,build,features",
                "--prefix",
                "none",
            ];
            if let Some(t) = &target {
                build.extend(["--target", t]);
                tree_args.extend(["--target", t]);
            }
            println!("▶ 配布ビルド (cargo {})", build.join(" "));
            run_capture(Command::new("cargo").args(&build))?;
            println!("▶ (1) 配布ビルドの feature");
            let tree = run_capture(Command::new("cargo").args(&tree_args))?;
            let found = forbidden_features(&tree);
            if !found.is_empty() {
                return Err(format!("配布ビルドに有効な feature があります: {found:?}"));
            }
            release_exe(&root, target.as_deref())
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

    if bytes.starts_with(ELF_MAGIC) {
        println!("▶ (4) 静的リンク (ELF: PT_INTERP と DT_NEEDED が無い)");
        let deps = elf_dynamic_deps(&bytes)
            .ok_or_else(|| format!("{}: ELF (64 ビット LE) として読めません", exe.display()))?;
        if deps.interp || deps.needed > 0 {
            return Err(format!(
                "静的リンクではありません (PT_INTERP: {}、DT_NEEDED: {} 件。musl のターゲットで作ってください)",
                deps.interp, deps.needed
            ));
        }
    } else {
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

/// 配布ビルドの exe の場所 (`target` を指定すれば `target/<三つ組>/release`)。
pub fn release_exe(root: &std::path::Path, target: Option<&str>) -> PathBuf {
    let dir = match target {
        Some(t) => root.join("target").join(t).join("release"),
        None => root.join("target/release"),
    };
    let windows = target.map_or(cfg!(windows), |t| t.contains("windows"));
    dir.join(if windows {
        "evoeditor.exe"
    } else {
        "evoeditor"
    })
}

const ELF_MAGIC: &[u8] = b"\x7fELF";

/// ELF の動的リンクの手掛かり。
#[derive(Debug, PartialEq, Eq)]
struct ElfDynamic {
    /// プログラムインタプリタ (`PT_INTERP`、動的リンカ) がある。
    interp: bool,
    /// 共有ライブラリの依存 (`DT_NEEDED`) の数。static-pie は `PT_DYNAMIC` を持つが、依存は 0。
    needed: usize,
}

/// ELF (64 ビット、リトルエンディアン。x86_64 と aarch64) のプログラムヘッダと動的セクションを読む。読めなければ None。
fn elf_dynamic_deps(b: &[u8]) -> Option<ElfDynamic> {
    const PT_DYNAMIC: u32 = 2;
    const PT_INTERP: u32 = 3;
    const DT_NULL: u64 = 0;
    const DT_NEEDED: u64 = 1;
    /// プログラムヘッダ 1 つの大きさ (ELF64)。
    const PHDR_SIZE: usize = 56;
    // 壊れたファイルでも桁あふれで止まらないよう、位置の計算はすべて checked で行う (読めなければ None)
    let bytes_at = |o: usize, n: usize| b.get(o..o.checked_add(n)?);
    let u16_at = |o: usize| bytes_at(o, 2).map(|x| u16::from_le_bytes([x[0], x[1]]));
    let u32_at = |o: usize| bytes_at(o, 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]));
    let u64_at = |o: usize| {
        bytes_at(o, 8)
            .and_then(|x| <[u8; 8]>::try_from(x).ok())
            .map(u64::from_le_bytes)
    };
    if !b.starts_with(ELF_MAGIC) || b.get(4) != Some(&2) || b.get(5) != Some(&1) {
        return None;
    }
    let phoff = usize::try_from(u64_at(0x20)?).ok()?;
    let phentsize = usize::from(u16_at(0x36)?);
    let phnum = usize::from(u16_at(0x38)?);
    if phentsize < PHDR_SIZE {
        return None;
    }
    let mut out = ElfDynamic {
        interp: false,
        needed: 0,
    };
    for i in 0..phnum {
        let ph = phoff.checked_add(i.checked_mul(phentsize)?)?;
        match u32_at(ph)? {
            PT_INTERP => out.interp = true,
            PT_DYNAMIC => {
                let off = usize::try_from(u64_at(ph.checked_add(8)?)?).ok()?;
                let size = usize::try_from(u64_at(ph.checked_add(32)?)?).ok()?;
                for e in (off..off.checked_add(size)?).step_by(16) {
                    match u64_at(e)? {
                        DT_NULL => break,
                        DT_NEEDED => out.needed += 1,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Some(out)
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

    /// 最小の ELF64 LE: プログラムヘッダ (種類, 中身) を並べ、中身はヘッダの後ろに置く。
    fn elf(phs: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let phoff = 64usize;
        let mut b = vec![0u8; phoff + phs.len() * 56];
        b[..4].copy_from_slice(ELF_MAGIC);
        b[4] = 2;
        b[5] = 1;
        b[0x20..0x28].copy_from_slice(&(phoff as u64).to_le_bytes());
        b[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
        b[0x38..0x3a].copy_from_slice(&(phs.len() as u16).to_le_bytes());
        for (i, (ty, body)) in phs.iter().enumerate() {
            let ph = phoff + i * 56;
            let off = b.len();
            b[ph..ph + 4].copy_from_slice(&ty.to_le_bytes());
            b[ph + 8..ph + 16].copy_from_slice(&(off as u64).to_le_bytes());
            b[ph + 32..ph + 40].copy_from_slice(&(body.len() as u64).to_le_bytes());
            b.extend_from_slice(body);
        }
        b
    }

    fn dyn_entries(tags: &[u64]) -> Vec<u8> {
        tags.iter()
            .flat_map(|t| [t.to_le_bytes(), 0u64.to_le_bytes()].concat())
            .collect()
    }

    #[test]
    fn elf_の動的リンクの手掛かりを読む() {
        // static-pie: PT_DYNAMIC はあるが DT_NEEDED は無い
        let static_pie = elf(&[(2, dyn_entries(&[0x6fff_fffb, 0]))]);
        assert_eq!(
            elf_dynamic_deps(&static_pie),
            Some(ElfDynamic {
                interp: false,
                needed: 0
            })
        );
        // 動的リンク: PT_INTERP と DT_NEEDED 2 件 (DT_NULL の後は数えない)
        let dynamic = elf(&[
            (3, b"/lib64/ld-linux-x86-64.so.2\0".to_vec()),
            (2, dyn_entries(&[1, 1, 0, 1])),
        ]);
        assert_eq!(
            elf_dynamic_deps(&dynamic),
            Some(ElfDynamic {
                interp: true,
                needed: 2
            })
        );
        assert_eq!(elf_dynamic_deps(b"MZ not elf"), None);
        // 壊れたヘッダ: 範囲外の位置・大きさでもパニックせずに None
        let mut broken = static_pie.clone();
        broken[0x20..0x28].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(elf_dynamic_deps(&broken), None);
        let mut broken = static_pie.clone();
        broken[64 + 32..64 + 40].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(elf_dynamic_deps(&broken), None);
        let mut broken = static_pie;
        broken[0x36..0x38].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(elf_dynamic_deps(&broken), None);
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
