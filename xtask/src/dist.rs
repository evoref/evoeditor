//! `cargo xtask dist`: 配布物 (Windows x86_64 の zip と `SHA256SUMS`) の作成と検査 (docs/05 §18.2、docs/09 §8.2「配布物」)。
//!
//! - `dist [--exe PATH] [--version V]`: 配布ビルドの exe を `check-dist` で検査し、`THIRD-PARTY-NOTICES.md` の最新性を確かめて、
//!   `target/dist/evoeditor-<版>-x86_64-pc-windows-msvc.zip`・同名の `.pdb`・`SHA256SUMS` を作る。zip は同じ入力から同じバイト列になる
//!   (ファイルの順序と時刻を固定する)。作った zip は `--verify` と同じ検査を通す。署名 (`SHA256SUMS.minisig`) はメンテナの手元で付ける (05 §18.9)。
//! - `dist --verify ZIP`: zip の内容物 (必要な 6 ファイルだけ) を確かめ、展開した exe を `check-dist --exe` に通して `--version` を確かめる。
//!   `release.yml` の「ダウンロード → 展開 → 検査」で使う。

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::util::{read_text, run_capture, workspace_root};

const TARGET: &str = "x86_64-pc-windows-msvc";

/// zip の内容物 (アーカイブ内の名前, リポジトリ内の元)。これ以外のファイルは入れない。
const FILES: &[(&str, &str)] = &[
    ("evoeditor.exe", ""),
    ("LICENSE", "LICENSE"),
    ("NOTICE", "NOTICE"),
    ("THIRD-PARTY-NOTICES.md", "THIRD-PARTY-NOTICES.md"),
    ("README.md", "dist/README.md"),
    ("config.example.toml", "dist/config.example.toml"),
];

pub fn run(args: &[String]) -> Result<(), String> {
    if let Some(i) = args.iter().position(|a| a == "--verify") {
        let zip = args
            .get(i + 1)
            .ok_or("--verify には zip のパスが要ります")?;
        return verify(Path::new(zip));
    }
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let root = workspace_root();
    let version = match value("--version") {
        Some(v) => v.clone(),
        None => workspace_version(&root)?,
    };
    let exe = match value("--exe") {
        Some(p) => PathBuf::from(p),
        None => {
            println!("▶ 配布ビルド");
            run_capture(Command::new("cargo").args(["build", "--release", "-p", "evoeditor"]))?;
            root.join("target/release/evoeditor.exe")
        }
    };
    println!("▶ 配布ビルドの検査 (check-dist)");
    crate::check_dist::run(&["--exe".to_owned(), exe.display().to_string()])?;
    println!("▶ THIRD-PARTY-NOTICES.md の最新性");
    crate::gen_notices::run(&["--check".to_owned()])?;

    let out = root.join("target/dist");
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let base = format!("evoeditor-{version}-{TARGET}");
    let zip_path = out.join(format!("{base}.zip"));
    println!("▶ {}", zip_path.display());
    write_zip(&zip_path, &exe, &root)?;

    let mut assets = vec![zip_path.clone()];
    let pdb = exe.with_extension("pdb");
    if pdb.exists() {
        let dst = out.join(format!("{base}.pdb"));
        fs::copy(&pdb, &dst).map_err(|e| format!("{}: {e}", pdb.display()))?;
        assets.push(dst);
    }
    let mut sums = String::new();
    for a in &assets {
        let name = a
            .file_name()
            .and_then(|n| n.to_str())
            .expect("UTF-8 のファイル名");
        sums.push_str(&format!("{}  {name}\n", sha256_hex(a)?));
    }
    let sums_path = out.join("SHA256SUMS");
    fs::write(&sums_path, &sums).map_err(|e| format!("{}: {e}", sums_path.display()))?;
    print!("{sums}");

    verify(&zip_path)?;
    println!("dist: OK ({})", out.display());
    Ok(())
}

/// workspace の `version`。
fn workspace_version(root: &Path) -> Result<String, String> {
    let text = read_text(&root.join("Cargo.toml"))?;
    let v: toml::Value = toml::from_str(&text).map_err(|e| format!("Cargo.toml: {e}"))?;
    v["workspace"]["package"]["version"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "Cargo.toml に workspace.package.version がありません".to_owned())
}

fn sha256_hex(path: &Path) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// 決定的な zip (順序・時刻・権限を固定) を書く。
fn write_zip(zip_path: &Path, exe: &Path, root: &Path) -> Result<(), String> {
    let f = File::create(zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut z = ZipWriter::new(f);
    let opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644);
    for (name, src) in FILES {
        let path = if src.is_empty() {
            exe.to_path_buf()
        } else {
            root.join(src)
        };
        let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        // テキストは LF に揃える (作業ツリーの改行に依らず同じ zip にする)
        let bytes = if name.ends_with(".exe") {
            bytes
        } else {
            String::from_utf8(bytes)
                .map_err(|e| format!("{name}: UTF-8 ではありません: {e}"))?
                .replace("\r\n", "\n")
                .into_bytes()
        };
        z.start_file(*name, opts).map_err(|e| e.to_string())?;
        z.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    z.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// zip の検査: 内容物がそろい、余分なファイルが無く、exe が配布ビルドの検査と `--version` を通る。
fn verify(zip_path: &Path) -> Result<(), String> {
    println!("▶ zip の内容物の検査: {}", zip_path.display());
    let f = File::open(zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut z = ZipArchive::new(f).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut names = Vec::new();
    for i in 0..z.len() {
        let f = z.by_index(i).map_err(|e| e.to_string())?;
        names.push(f.name().map_err(|e| e.to_string())?.into_owned());
    }
    names.sort();
    let mut want: Vec<String> = FILES.iter().map(|(n, _)| (*n).to_owned()).collect();
    want.sort();
    if names != want {
        return Err(format!(
            "zip の内容物が違います。\n  期待: {want:?}\n  実際: {names:?}"
        ));
    }
    let dir = workspace_root().join("target/dist-verify");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let exe = dir.join("evoeditor.exe");
    let mut bytes = Vec::new();
    z.by_name("evoeditor.exe")
        .map_err(|e| e.to_string())?
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    fs::write(&exe, &bytes).map_err(|e| e.to_string())?;
    crate::check_dist::run(&["--exe".to_owned(), exe.display().to_string()])?;
    let out = Command::new(&exe)
        .arg("--version")
        .output()
        .map_err(|e| format!("{}: 実行できません: {e}", exe.display()))?;
    let ver = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() || !ver.starts_with("evoeditor ") {
        return Err(format!("--version が想定外です: {ver:?} ({})", out.status));
    }
    println!("  --version: {}", ver.trim());
    let _ = fs::remove_dir_all(&dir);
    Ok(())
}
