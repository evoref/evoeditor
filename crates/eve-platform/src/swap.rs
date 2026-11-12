//! スワップ (swap-lite) の書き手と削除 (docs/03 §10.2、05 §7.5、02 §11.2.1)。
//!
//! 配置は `state_dir/swap/<名前>.swp/` で、中身は緊急退避と同じ形式 (本文の `.txt` と `manifest.toml`、`reason = "swap"`)。
//! 名前は、パスのある文書なら `<hash(path)[..16]>-<pid>`、無題なら `untitled-<DocumentId>-<pid>`。pid を含むので、同じファイルを
//! 開いた複数のインスタンスのスワップは別になる。manifest に書く `pid` と `started_at_*` が、復旧の「他のインスタンスが開いている」と
//! 残骸の判定の材料 (03 §10.3)。
//!
//! 書き方 (強制終了されても、manifest が指す本文は常に完全): (1) 本文を新しい名前 (`<n>-<名前>.r<Revision>.txt`) の一時ファイルに書いて
//! fsync し、名前を変える。(2) manifest を一時ファイル + fsync + rename で置き換える。(3) 古い本文を消す。(2) の前に止まれば前の
//! manifest と前の本文が残り、(2) の後に止まれば新しいものが残る。ディレクトリは所有者のみの DACL で作る (05 §5.6)。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use eve_core::{DocSnapshot, FsError, FsPath, RescueReason};
use eve_text::TextRead as _;

use crate::inflight::{create_dir_all_private, normalized, process_start};
use crate::rescue::{
    ManifestDoc, Owner, SyncFn, eol_name, manifest_text, now_secs, safe_basename, sync_file,
    timestamp_rfc3339, write_body, write_manifest,
};
use crate::sys::map_io_error;

/// `swap/` の下の名前 (拡張子 `.swp` を含む)。
///
/// パスのある文書は、正規形 (05 §7.5 の「`hash(path)` の入力」) の XXH3-64 の先頭 16 桁。親ディレクトリが無くなっていて正規形を
/// 作れないときは、パスの文字列をそのまま (大文字に畳んで) 使う。
pub(crate) fn swap_name(path: Option<&Path>, doc: u64, pid: u32) -> String {
    match path {
        Some(p) => {
            let key = normalized(p).unwrap_or_else(|| p.to_string_lossy().to_uppercase());
            let bytes: Vec<u8> = key.encode_utf16().flat_map(u16::to_le_bytes).collect();
            format!("{:016x}-{pid}.swp", xxhash_rust::xxh3::xxh3_64(&bytes))
        }
        None => format!("untitled-{doc}-{pid}.swp"),
    }
}

fn swap_dir(state_dir: &Path, name: &str) -> PathBuf {
    state_dir.join("swap").join(name)
}

/// スワップを書く。`state_dir` の下に所有者のみで作り、本文と manifest を原子的に置き換える。
pub(crate) fn write_swap(state_dir: &Path, d: &DocSnapshot) -> Result<(), FsError> {
    let dir_path = FsPath::new(state_dir.join("swap"));
    write(state_dir, d, &sync_file).map_err(|e| map_io_error(e, &dir_path))
}

/// `sync` は fsync する関数 (本番は `sync_file`。試験は呼ばれた順を記録する)。スワップは緊急退避の 2 段の書き方 (02 §11.2.1)
/// を使わず、本文と manifest をそれぞれ fsync してから rename する。
fn write(state_dir: &Path, d: &DocSnapshot, sync: SyncFn<'_>) -> io::Result<()> {
    let pid = std::process::id();
    let name = swap_name(d.path.as_ref().map(FsPath::as_path), d.doc.0, pid);
    let dir = swap_dir(state_dir, &name);
    create_dir_all_private(&dir)?;

    let base = d
        .path
        .as_ref()
        .and_then(|p| p.as_path().file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .or_else(|| d.untitled_name.clone())
        .unwrap_or_default();
    let file = format!("1-{}.r{}.txt", safe_basename(&base), d.revision.0);

    // (1) 本文: 一時ファイルに書いて fsync し、名前を変える
    let tmp = dir.join(format!("{file}.tmp"));
    let last = write_body(&tmp, d, &|| false, sync)?.unwrap_or(0);
    fs::rename(&tmp, dir.join(&file))?;

    // (2) manifest: 新しい本文を指すものに置き換える
    let entry = ManifestDoc {
        file: file.clone(),
        path: d.path.as_ref().map(FsPath::display),
        untitled_name: d.untitled_name.clone(),
        encoding: d.format.encoding.name(),
        bom: d.format.has_bom,
        default_eol: eol_name(d.format.default_eol),
        eol_mixed: d.format.eol_stats.is_mixed(),
        final_newline: matches!(last, b'\n' | b'\r'),
        // 本文のエスケープ文字が `encoding` のバイトか (03 §8.3.1。v0.4 の M23 から。形式は変えない)。不変条件
        // (`None` か `Some(encoding)`) が破れていても、別の文字コードのバイトを「バイト」と書かない
        pua_escaped: d.format.escape_encoding == Some(d.format.encoding),
        revision: d.revision.0,
        len: d.snapshot.len_bytes(),
        written: true,
        stored: false,
        xxh3: None,
        sensitive: d.sensitive,
    };
    let owner = Owner {
        pid,
        started_at: process_start(),
    };
    let text = manifest_text(
        RescueReason::Swap,
        &timestamp_rfc3339(now_secs()),
        true,
        &[entry],
        Some(&owner),
    );
    write_manifest(&dir, &text, Some(sync))?;

    // (3) 古い本文と、前回の書込の残りの一時ファイルを消す (消せなくても次の書込で消える)
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if n != file && n != "manifest.toml" && (n.ends_with(".txt") || n.ends_with(".tmp")) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    Ok(())
}

/// 自分 (この pid) のスワップを消す。無ければ何もしない。
pub(crate) fn discard_swap(
    state_dir: &Path,
    doc: u64,
    path: Option<&FsPath>,
) -> Result<(), FsError> {
    let name = swap_name(path.map(FsPath::as_path), doc, std::process::id());
    let dir = swap_dir(state_dir, &name);
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(map_io_error(e, &FsPath::new(dir))),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use eve_core::{DocumentId, TextFormat};
    use eve_text::Text;
    use eve_view::Revision;

    use super::*;
    use crate::rescue::SyncPoint;

    // スワップは緊急退避の 2 段の書き方 (02 §11.2.1) を使わない: 本文を fsync してから rename し、manifest を fsync してから
    // rename する (今までどおり)。manifest に `stored` / `xxh3` を書かない
    #[test]
    fn スワップは本文と_manifest_をそれぞれ_fsync_してから置き換える() {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let state = std::env::temp_dir().join(format!("eve-swap-sync-{}-{n}", std::process::id()));
        let d = DocSnapshot {
            doc: DocumentId(7),
            snapshot: Text::from("swap text").snapshot(),
            format: TextFormat::new_document(),
            revision: Revision(3),
            saved_node_exists: false,
            sensitive: false,
            path: None,
            untitled_name: Some("無題-7".into()),
        };
        let events: RefCell<Vec<(SyncPoint, Vec<String>)>> = RefCell::new(Vec::new());
        let record = |p: SyncPoint, f: &fs::File| {
            // fsync の時点のディレクトリの中身 (rename の前であることを確かめる)
            let dir = swap_dir(&state, &swap_name(None, 7, std::process::id()));
            let mut names: Vec<String> = fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            events.borrow_mut().push((p, names));
            f.sync_all()
        };
        write(&state, &d, &record).unwrap();
        let ev = events.borrow();
        assert_eq!(
            ev.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
            [SyncPoint::Body(1), SyncPoint::Manifest],
            "{ev:?}"
        );
        assert_eq!(
            ev[0].1,
            ["1-無題-7.r3.txt.tmp"],
            "本文は rename の前に fsync する"
        );
        assert_eq!(
            ev[1].1,
            ["1-無題-7.r3.txt", "manifest.toml.tmp"],
            "manifest は rename の前に fsync する"
        );
        let dir = swap_dir(&state, &swap_name(None, 7, std::process::id()));
        let m = fs::read_to_string(dir.join("manifest.toml")).unwrap();
        assert!(m.contains("written = true"), "{m}");
        assert!(!m.contains("stored") && !m.contains("xxh3"), "{m}");
        let _ = fs::remove_dir_all(&state);
    }
}
