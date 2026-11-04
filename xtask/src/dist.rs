//! `cargo xtask dist`: 配布物 (Windows x86_64 の zip・Linux の musl 静的版の tar.gz と `SHA256SUMS`) の作成と検査
//! (docs/05 §18.2 / §18.6、docs/09 §8.2「配布物」)。
//!
//! - `dist [--exe PATH] [--version V]`: 配布ビルドの exe を `check-dist` で検査し、`THIRD-PARTY-NOTICES.md` の最新性を確かめて、
//!   `target/dist/evoeditor-<版>-x86_64-pc-windows-msvc.zip`・同名の `.pdb`・`SHA256SUMS` を作る。zip は同じ入力から同じバイト列になる
//!   (ファイルの順序と時刻を固定する)。作った zip は `--verify` と同じ検査を通す。署名 (`SHA256SUMS.minisig`) はメンテナの手元で付ける (05 §18.9)。
//! - `dist --target <三つ組> [--exe PATH] [--version V]`: Linux の musl 静的版 (`x86_64-unknown-linux-musl` /
//!   `aarch64-unknown-linux-musl`。Linux の上で作る)。デバッグ情報を残して作り、`objcopy` で分割デバッグ情報
//!   `evoeditor-<版>-<三つ組>.debug` を取り出してから、配る exe は strip して `.gnu_debuglink` を付ける (05 §18.6)。
//!   `evoeditor-<版>-<三つ組>.tar.gz` は GNU tar と gzip で、順序・時刻・所有者を固定して作る (同じ入力から同じバイト列)。
//! - `dist --verify ARCHIVE`: zip / tar.gz の内容物 (必要な 7 ファイルだけ) を確かめ、展開した exe を `check-dist --exe` に通して
//!   `--version` を確かめる。`release.yml` の「ダウンロード → 展開 → 検査」で使う。

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::util::{read_text, run_capture, workspace_root};

const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";

/// 実行ファイル以外の内容物 (アーカイブ内の名前, リポジトリ内の元)。これ以外のファイルは入れない。
const DOCS: &[(&str, &str)] = &[
    ("LICENSE", "LICENSE"),
    ("NOTICE", "NOTICE"),
    ("THIRD-PARTY-NOTICES.md", "THIRD-PARTY-NOTICES.md"),
    ("README.md", "dist/README.md"),
    // 英語版 (v0.2、M10。日本語版が正、09 §11.3)
    ("README.en.md", "dist/README.en.md"),
    ("config.example.toml", "dist/config.example.toml"),
];

pub fn run(args: &[String]) -> Result<(), String> {
    if let Some(i) = args.iter().position(|a| a == "--verify") {
        let archive = args
            .get(i + 1)
            .ok_or("--verify にはアーカイブのパスが要ります")?;
        return verify(Path::new(archive));
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
    let target = value("--target").map_or(WINDOWS_TARGET, String::as_str);
    let linux = target.contains("linux");
    if linux && !cfg!(target_os = "linux") {
        return Err(format!("{target} の配布物は Linux の上で作ります"));
    }
    if !linux && target != WINDOWS_TARGET {
        return Err(format!("対応していないターゲットです: {target}"));
    }
    let exe = match value("--exe") {
        Some(p) => PathBuf::from(p),
        None if linux => {
            println!("▶ 配布ビルド ({target}。分割デバッグ情報のため strip せずに作る)");
            run_capture(
                Command::new("cargo")
                    .args(["build", "--release", "-p", "evoeditor", "--target", target])
                    .env("CARGO_PROFILE_RELEASE_STRIP", "none"),
            )?;
            crate::check_dist::release_exe(&root, Some(target))
        }
        None => {
            println!("▶ 配布ビルド");
            run_capture(Command::new("cargo").args(["build", "--release", "-p", "evoeditor"]))?;
            root.join("target/release/evoeditor.exe")
        }
    };

    let out = root.join("target/dist");
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let base = format!("evoeditor-{version}-{target}");
    let mut assets = Vec::new();
    let archive = if linux {
        let stage = out.join(format!("stage-{target}"));
        let _ = fs::remove_dir_all(&stage);
        fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
        let debug = out.join(format!("{base}.debug"));
        let shipped = stage.join("evoeditor");
        println!("▶ 分割デバッグ情報: {}", debug.display());
        split_debug(&exe, &debug, &shipped)?;
        check_and_notices(&shipped)?;
        let tgz = out.join(format!("{base}.tar.gz"));
        println!("▶ {}", tgz.display());
        write_tar_gz(&tgz, &stage, &root)?;
        assets.push(tgz.clone());
        assets.push(debug);
        tgz
    } else {
        check_and_notices(&exe)?;
        let zip_path = out.join(format!("{base}.zip"));
        println!("▶ {}", zip_path.display());
        write_zip(&zip_path, &exe, &root)?;
        assets.push(zip_path.clone());
        let pdb = exe.with_extension("pdb");
        if pdb.exists() {
            let dst = out.join(format!("{base}.pdb"));
            fs::copy(&pdb, &dst).map_err(|e| format!("{}: {e}", pdb.display()))?;
            assets.push(dst);
        }
        zip_path
    };

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

    verify(&archive)?;
    println!("dist: OK ({})", out.display());
    Ok(())
}

/// 配る exe を `check-dist` に通し、`THIRD-PARTY-NOTICES.md` の最新性を確かめる。
fn check_and_notices(exe: &Path) -> Result<(), String> {
    println!("▶ 配布ビルドの検査 (check-dist)");
    crate::check_dist::run(&["--exe".to_owned(), exe.display().to_string()])?;
    println!("▶ THIRD-PARTY-NOTICES.md の最新性");
    crate::gen_notices::run(&["--check".to_owned()])
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

/// テキストの内容物を LF に揃えて読む (作業ツリーの改行に依らず同じアーカイブにする)。
fn doc_bytes(root: &Path, name: &str, src: &str) -> Result<Vec<u8>, String> {
    let path = root.join(src);
    let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(String::from_utf8(bytes)
        .map_err(|e| format!("{name}: UTF-8 ではありません: {e}"))?
        .replace("\r\n", "\n")
        .into_bytes())
}

/// 決定的な zip (順序・時刻・権限を固定) を書く。
fn write_zip(zip_path: &Path, exe: &Path, root: &Path) -> Result<(), String> {
    let f = File::create(zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut z = ZipWriter::new(f);
    let opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644);
    let exe_bytes = fs::read(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    z.start_file("evoeditor.exe", opts)
        .map_err(|e| e.to_string())?;
    z.write_all(&exe_bytes).map_err(|e| e.to_string())?;
    for (name, src) in DOCS {
        let bytes = doc_bytes(root, name, src)?;
        z.start_file(*name, opts).map_err(|e| e.to_string())?;
        z.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    z.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// `exe` (デバッグ情報つき) から分割デバッグ情報 `debug` を取り出し、strip して `.gnu_debuglink` を付けた exe を `shipped` に書く。
fn split_debug(exe: &Path, debug: &Path, shipped: &Path) -> Result<(), String> {
    run_capture(
        Command::new("objcopy")
            .arg("--only-keep-debug")
            .arg(exe)
            .arg(debug),
    )?;
    run_capture(
        Command::new("objcopy")
            .arg("--strip-all")
            .arg(format!("--add-gnu-debuglink={}", debug.display()))
            .arg(exe)
            .arg(shipped),
    )?;
    Ok(())
}

/// 決定的な tar.gz (順序・時刻・所有者・権限を固定。gzip の名前と時刻は書かない) を書く。`stage` には exe が置いてある。
#[cfg(unix)]
fn write_tar_gz(tgz: &Path, stage: &Path, root: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let set_mode = |p: &Path, mode: u32| {
        fs::set_permissions(p, fs::Permissions::from_mode(mode))
            .map_err(|e| format!("{}: {e}", p.display()))
    };
    set_mode(&stage.join("evoeditor"), 0o755)?;
    for (name, src) in DOCS {
        let p = stage.join(name);
        fs::write(&p, doc_bytes(root, name, src)?).map_err(|e| format!("{}: {e}", p.display()))?;
        set_mode(&p, 0o644)?;
    }
    let tar = tgz.with_extension(""); // `.tar.gz` → `.tar`
    let _ = fs::remove_file(&tar);
    let _ = fs::remove_file(tgz);
    let mut cmd = Command::new("tar");
    cmd.args([
        "--format=gnu",
        "--mtime=@0",
        "--owner=0",
        "--group=0",
        "--numeric-owner",
    ])
    .arg("-cf")
    .arg(&tar)
    .arg("-C")
    .arg(stage)
    .arg("evoeditor")
    .args(DOCS.iter().map(|(n, _)| *n));
    run_capture(&mut cmd)?;
    run_capture(Command::new("gzip").args(["-9", "-n"]).arg(&tar))?;
    if !tgz.exists() {
        return Err(format!("{} を作れませんでした", tgz.display()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn write_tar_gz(_: &Path, _: &Path, _: &Path) -> Result<(), String> {
    Err("tar.gz は Linux の上で作ります".to_owned())
}

/// アーカイブの検査: 内容物がそろい、余分なファイルが無く、exe が配布ビルドの検査と `--version` を通る。
fn verify(archive: &Path) -> Result<(), String> {
    let name = archive.to_string_lossy();
    let tar = name.ends_with(".tar.gz");
    let exe_name = if tar { "evoeditor" } else { "evoeditor.exe" };
    println!("▶ アーカイブの内容物の検査: {}", archive.display());
    let dir = workspace_root().join("target/dist-verify");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut names = if tar {
        let list = run_capture(Command::new("tar").arg("-tzf").arg(archive))?;
        run_capture(
            Command::new("tar")
                .arg("-xzf")
                .arg(archive)
                .arg("-C")
                .arg(&dir),
        )?;
        list.lines().map(str::to_owned).collect::<Vec<_>>()
    } else {
        extract_zip(archive, &dir)?
    };
    names.sort();
    let mut want: Vec<String> = std::iter::once(exe_name)
        .chain(DOCS.iter().map(|(n, _)| *n))
        .map(str::to_owned)
        .collect();
    want.sort();
    if names != want {
        return Err(format!(
            "アーカイブの内容物が違います。\n  期待: {want:?}\n  実際: {names:?}"
        ));
    }
    let exe = dir.join(exe_name);
    #[cfg(unix)]
    if tar {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&exe)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(format!("{exe_name} に実行の権限がありません: {mode:o}"));
        }
    }
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

/// zip の内容物の名前を返し、exe を `dir` に取り出す。
fn extract_zip(zip_path: &Path, dir: &Path) -> Result<Vec<String>, String> {
    let f = File::open(zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut z = ZipArchive::new(f).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut names = Vec::new();
    for i in 0..z.len() {
        let f = z.by_index(i).map_err(|e| e.to_string())?;
        names.push(f.name().map_err(|e| e.to_string())?.into_owned());
    }
    if names.iter().any(|n| n == "evoeditor.exe") {
        let mut bytes = Vec::new();
        z.by_name("evoeditor.exe")
            .map_err(|e| e.to_string())?
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        fs::write(dir.join("evoeditor.exe"), &bytes).map_err(|e| e.to_string())?;
    }
    Ok(names)
}
