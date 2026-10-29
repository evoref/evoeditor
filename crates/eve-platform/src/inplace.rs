//! 直接上書き `write_in_place` の Windows 実装 (docs/05 §4.4 の手順 1〜9、§7.5)。
//!
//! 原子的置換ができない・意味が変わるとき (ハードリンク・共有違反・親に書込権限がない・一時ファイル規則) に、core が確認の後で呼ぶ。
//! **元のファイルに書き始めるのは手順 6 からで、それより前のどの失敗でも元のファイルは変わらない**: 照合 → 保存中フラグ →
//! `OPEN_EXISTING` + `FILE_SHARE_READ` で開く (切り詰めない) → 事前バックアップ (開いたハンドルから読み、`state_dir/backup/` の一時名に
//! 書いて fsync し、最終名に変えてからフラグを `backup_complete = true` に書き直す) → 領域の事前確保 → 先頭から上書き → 長さ合わせと fsync →
//! 成功ならフラグ、次にバックアップを消す。書込途中の失敗は `InPlaceFailed { backup }` で、バックアップとフラグを残す。
//!
//! 範囲外 (後続): 置換中の計数 (`ReplacingGuard`)、領域の確保を使えない FS の細かい分岐の実測 (S-09)。

#![allow(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::ptr;
use std::thread;
use std::time::Duration;

use eve_core::{
    CancelToken, ChunkError, Expect, FileKind, FileMeta, FileStamp, FileSystem, FileTime, FsError,
    FsErrorDetail, FsErrorKind, FsPath, FsyncLevel, InPlaceBackup, ReadChunks, SaveMethod,
    SaveOptions, SaveReport, SaveWarning,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALLOCATION_INFO, FILE_SHARE_READ, FileAllocationInfo, GetDiskFreeSpaceExW,
    SetFileInformationByHandle,
};

use crate::fs::{WinFs, map_io_error, winapi};
use crate::hooks;
use crate::inflight::create_dir_all_private;
use crate::rescue::timestamp_basic;
use crate::save::{
    ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION, FlagGuard, SHARING_RETRY_MS, delete, last_error,
    matches, resolve_target, same_file_state, wide,
};

const ERROR_INVALID_FUNCTION: u32 = 1;
const ERROR_NOT_SUPPORTED: u32 = 50;

/// 事前バックアップを読む・書くチャンク。
const CHUNK: usize = 1024 * 1024;

fn file_time(m: &fs::Metadata) -> FileTime {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(FileTime::default(), |d| FileTime {
            secs: d.as_secs() as i64,
            nanos: d.subsec_nanos(),
        })
}

/// T を `OPEN_EXISTING`・読み書き・`FILE_SHARE_READ` で開く。共有違反は 50〜1000 ms の指数バックオフで再試行する (05 §4.4 手順 3)。
fn open_shared_read(target: &Path) -> io::Result<File> {
    let mut attempt = 0;
    loop {
        let r = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ)
            .open(target);
        match r {
            Ok(f) => return Ok(f),
            Err(e)
                if matches!(
                    e.raw_os_error().map(|c| c as u32),
                    Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
                ) && attempt < SHARING_RETRY_MS.len() =>
            {
                thread::sleep(Duration::from_millis(SHARING_RETRY_MS[attempt]));
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// 領域の事前確保 (`FileAllocationInfo` だけ。EOF は延ばさない)。使えない FS では空き容量の確認にする。
fn preallocate(file: &File, dir: &Path, extra: u64, new_alloc: u64) -> Result<(), FsError> {
    let info = FILE_ALLOCATION_INFO {
        AllocationSize: i64::try_from(new_alloc).unwrap_or(i64::MAX),
    };
    // SAFETY: 開いているファイルのハンドルと、有効な `FILE_ALLOCATION_INFO`。
    let ok = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileAllocationInfo,
            ptr::from_ref(&info).cast(),
            std::mem::size_of::<FILE_ALLOCATION_INFO>() as u32,
        )
    };
    if ok != 0 {
        return Ok(());
    }
    let code = last_error();
    if code == ERROR_INVALID_FUNCTION || code == ERROR_NOT_SUPPORTED {
        // 確保を使えない FS: 呼び出し元の空き容量 (クォータ込み) が増える分を上回ることだけを確かめて進む
        let w = wide(dir);
        let mut avail = 0u64;
        // SAFETY: NUL 終端のパスと、書込先の有効なポインタ。
        let ok = unsafe {
            GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, ptr::null_mut(), ptr::null_mut())
        };
        if ok != 0 && avail >= extra {
            return Ok(());
        }
        return Err(FsError::new(FsErrorKind::DiskFull));
    }
    Err(map_io_error(
        io::Error::from_raw_os_error(code as i32),
        &FsPath::new(dir),
    ))
}

impl WinFs {
    pub(crate) fn write_in_place_impl(
        &self,
        path: &FsPath,
        data: &mut dyn ReadChunks,
        len: u64,
        opts: &SaveOptions,
        cancel: &CancelToken,
    ) -> Result<SaveReport, FsError> {
        let conflict = |current| FsError::new(FsErrorKind::Conflict { current });
        let source_failed = || FsError::new(FsErrorKind::SourceFailed);
        let target = resolve_target(path)?;
        let target_path = FsPath::new(target.clone());
        if cancel.is_cancelled() {
            return Err(source_failed());
        }

        // 1. 書き始める前の照合
        let existing: Option<FileMeta> = match self.metadata(&target_path) {
            Ok(m) => Some(m),
            Err(e) if e.kind == FsErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        let current = existing.as_ref().map(|m| FileStamp::from_meta(m, None));
        match (opts.expected, &current) {
            (Expect::Absent, Some(_)) => return Err(conflict(current)),
            (Expect::Stamp(_), None) => return Err(conflict(None)),
            (Expect::Stamp(want), Some(have)) if !matches(&want, have) => {
                return Err(conflict(current));
            }
            _ => {}
        }
        let Some(orig) = existing else {
            // 直接上書きは元のファイルがあるときだけ (新規ファイルは原子的保存の経路)
            return Err(FsError::new(FsErrorKind::NotFound));
        };
        if orig.kind != FileKind::File {
            return Err(FsError::with_detail(
                FsErrorKind::InvalidPath,
                FsErrorDetail::NotAFile,
            ));
        }

        // 2. 保存中フラグ (書けなければ何も書かずに BackupFailed。SkipConfirmed の再投入だけは警告して続ける)
        let required = opts.in_place_backup == InPlaceBackup::Required;
        let mut warnings = Vec::new();
        let backup_final: Option<PathBuf> = match (self.state_dir(), required) {
            (Some(sd), true) => {
                let key = crate::inflight::flag_key(&target).unwrap_or_default();
                let ts = timestamp_basic(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs() as i64),
                );
                Some(
                    sd.join("backup")
                        .join(format!("{key}-{ts}-{}.bak", std::process::id())),
                )
            }
            _ => None,
        };
        let mut flag = match self.state_dir() {
            Some(sd) => FlagGuard::begin_in_place(sd, &target, backup_final.as_deref()),
            None => None,
        };
        if flag.is_none() {
            if required {
                return Err(FsError::with_detail(
                    FsErrorKind::BackupFailed,
                    FsErrorDetail::InflightFlag,
                ));
            }
            warnings.push(SaveWarning::InflightFlagNotWritten);
        }
        // バックアップを作るには状態ディレクトリが要る (無ければ BackupFailed)
        if required && backup_final.is_none() {
            if let Some(f) = flag.as_mut() {
                f.remove();
            }
            return Err(FsError::new(FsErrorKind::BackupFailed));
        }
        let remove_flag = |flag: &mut Option<FlagGuard>| {
            if let Some(f) = flag.as_mut() {
                f.remove();
            }
        };
        hooks::hit("kill_after_inflight_flag");

        // 3. 開く (切り詰めない。FILE_SHARE_READ のみ)
        let mut file = match open_shared_read(&target) {
            Ok(f) => f,
            Err(e) => {
                remove_flag(&mut flag);
                return Err(match e.raw_os_error().map(|c| c as u32) {
                    Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION) => {
                        FsError::new(FsErrorKind::Busy)
                    }
                    _ => map_io_error(e, path),
                });
            }
        };
        // 開いたハンドルで (mtime, len, FileId) を再照合する
        let handle_meta = file.metadata().ok();
        let handle_id = winapi::file_info(&file).ok().map(|i| eve_core::FileId {
            volume: i.volume,
            index: i.index,
        });
        if opts.expected != Expect::Any
            && let Some(hm) = &handle_meta
        {
            let now = FileMeta {
                len: hm.len(),
                mtime: file_time(hm),
                file_id: handle_id,
                ..orig.clone()
            };
            if !same_file_state(&orig, &now) {
                drop(file);
                remove_flag(&mut flag);
                return Err(conflict(Some(FileStamp::from_meta(&now, None))));
            }
        }
        let orig_len = orig.len;

        // 4. 事前バックアップ (開いたハンドルから読む)
        if let (Some(bak), Some(sd)) = (backup_final.as_ref(), self.state_dir()) {
            let fail = |flag: &mut Option<FlagGuard>, tmp: &Path| {
                delete(tmp);
                if let Some(f) = flag.as_mut() {
                    f.remove();
                }
                FsError::new(FsErrorKind::BackupFailed)
            };
            let bak_tmp = PathBuf::from(format!("{}.tmp", bak.display()));
            if create_dir_all_private(&sd.join("backup")).is_err() {
                return Err(fail(&mut flag, &bak_tmp));
            }
            let mut out = match File::create(&bak_tmp) {
                Ok(f) => f,
                Err(_) => return Err(fail(&mut flag, &bak_tmp)),
            };
            if file.seek(SeekFrom::Start(0)).is_err() {
                drop(out);
                return Err(fail(&mut flag, &bak_tmp));
            }
            let mut buf = vec![0u8; CHUNK];
            let mut first = true;
            loop {
                if cancel.is_cancelled() {
                    drop(out);
                    delete(&bak_tmp);
                    remove_flag(&mut flag);
                    return Err(source_failed());
                }
                let n = match file.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => {
                        drop(out);
                        return Err(fail(&mut flag, &bak_tmp));
                    }
                };
                if out.write_all(&buf[..n]).is_err() {
                    drop(out);
                    return Err(fail(&mut flag, &bak_tmp));
                }
                if first {
                    first = false;
                    let _ = out.flush();
                    hooks::hit("kill_mid_backup");
                }
            }
            if opts.fsync != FsyncLevel::None && out.sync_all().is_err() {
                drop(out);
                return Err(fail(&mut flag, &bak_tmp));
            }
            drop(out);
            // 最終名へ。途中で中断したバックアップが完全なものに見えないようにする
            if fs::rename(&bak_tmp, bak).is_err() {
                return Err(fail(&mut flag, &bak_tmp));
            }
            let marked = flag
                .as_ref()
                .is_none_or(|f| f.mark_backup_complete(&target, bak).is_ok());
            if !marked {
                delete(bak);
                return Err(fail(&mut flag, &bak_tmp));
            }
            hooks::hit("kill_after_backup");
        }

        // 5. 領域の事前確保 (長くなるときだけ。長さ・内容は変えない)
        if len > orig_len {
            let dir = target.parent().unwrap_or(Path::new("."));
            if let Err(e) = preallocate(&file, dir, len - orig_len, len) {
                // まだ書いていないので、バックアップ・フラグを消して元のファイルは変わらない
                if let Some(b) = backup_final.as_ref() {
                    delete(b);
                }
                remove_flag(&mut flag);
                return Err(e);
            }
        }
        hooks::hit("kill_after_inplace_alloc");

        // 6. 上書き (ここから取消しは見ない)。書込の全体 (手順 7 の fsync まで) が元のファイルを壊し得る区間
        let _replacing = self.replacing.guard(&target_path.display());
        let in_place_failed = |e: io::Error| {
            let mut err = FsError::new(FsErrorKind::InPlaceFailed {
                backup: backup_final.as_ref().map(FsPath::new),
            });
            err.source = Some(e);
            err
        };
        if let Err(e) = file.seek(SeekFrom::Start(0)) {
            return Err(in_place_failed(e));
        }
        let kill_at = hooks::value("kill_mid_inplace_write");
        let mut written: u64 = 0;
        loop {
            let chunk = match data.next_chunk() {
                Ok(Some(c)) => c,
                Ok(None) => break,
                // 乾式走査と食い違った (実装の誤り): 途中まで書き換わっている
                Err(ChunkError::Io(e)) => {
                    return Err(FsError {
                        source: e.source,
                        ..FsError::new(FsErrorKind::InPlaceFailed {
                            backup: backup_final.as_ref().map(FsPath::new),
                        })
                    });
                }
                Err(_) => {
                    return Err(in_place_failed(io::Error::from(io::ErrorKind::InvalidData)));
                }
            };
            if let Some(n) = kill_at
                && written + chunk.len() as u64 >= n
            {
                let part = (n - written) as usize;
                let _ = file.write_all(&chunk[..part]);
                let _ = file.flush();
                hooks::hit("kill_mid_inplace_write");
            }
            if let Err(e) = file.write_all(chunk) {
                return Err(in_place_failed(e));
            }
            written += chunk.len() as u64;
        }
        if written != len {
            return Err(in_place_failed(io::Error::from(io::ErrorKind::InvalidData)));
        }

        // 7. 長さ合わせ (縮めるのは書き終えた後だけ) と fsync
        if len < orig_len
            && let Err(e) = file.set_len(len)
        {
            return Err(in_place_failed(e));
        }
        if opts.fsync != FsyncLevel::None
            && let Err(e) = file.sync_all()
        {
            return Err(in_place_failed(e));
        }
        drop(file);

        // 9. 後片付け: 先にフラグ (他の記録も)、その後でバックアップ
        if let Some(f) = flag.as_mut() {
            f.finish_success();
        }
        if let Some(b) = backup_final.as_ref()
            && b.exists()
            && !delete(b)
        {
            warnings.push(SaveWarning::BackupNotRemoved {
                path: FsPath::new(b),
            });
        }

        // 事後: FileId は変わらない。自分の書いたものか確かめる
        let after = self.metadata(&target_path)?;
        if after.file_id != orig.file_id {
            warnings.push(SaveWarning::ChangedAfterSave);
        }
        Ok(SaveReport {
            method: SaveMethod::InPlace,
            new_stamp: FileStamp::from_meta(&after, None),
            warnings,
        })
    }
}

/// `list_orphan_backups`: `state_dir/backup/` の直下の `*.bak` / `*.bak.tmp` (名前は `<hash>-<UTC timestamp>-<pid>.bak`、05 §7.5)。
/// 名前の pid のプロセスが動いているもの (書込中・成功後の削除の前)、ディレクトリ (`fr119/`)、形に合わない名前は含めない。
pub(crate) fn list_orphan_backups(state_dir: &Path) -> Vec<eve_core::OrphanBackup> {
    let Ok(rd) = fs::read_dir(state_dir.join("backup")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        let path = e.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let (stem, partial) = match name.strip_suffix(".bak.tmp") {
            Some(s) => (s, true),
            None => match name.strip_suffix(".bak") {
                Some(s) => (s, false),
                None => continue,
            },
        };
        let Some(pid) = stem.rsplit('-').next().and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        if crate::swap_scan::process_alive(pid, None) {
            continue;
        }
        out.push(eve_core::OrphanBackup {
            path: FsPath::new(path),
            pid,
            partial,
            len: meta.len(),
        });
    }
    out.sort_by(|a, b| a.path.as_path().cmp(b.path.as_path()));
    out
}
