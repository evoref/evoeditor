//! ファイルの API (作成・削除・fsync・属性・名前の変更・置換・領域の確保・ハンドルの識別・空き容量・ドライブの種類)。

use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{GENERIC_WRITE, GetLastError, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateFileW, DeleteFileW, FILE_ALLOCATION_INFO,
    FILE_ATTRIBUTE_NORMAL, FILE_ID_INFO, FILE_REMOTE_PROTOCOL_INFO, FILE_SHARE_READ,
    FILE_TYPE_CHAR, FILE_TYPE_PIPE, FileAllocationInfo, FileIdInfo, FileRemoteProtocolInfo,
    FlushFileBuffers, GetDiskFreeSpaceExW, GetDriveTypeW, GetFileAttributesW,
    GetFileInformationByHandle, GetFileInformationByHandleEx, GetFileType,
    GetVolumeInformationByHandleW, INVALID_FILE_ATTRIBUTES, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    ReplaceFileW, SetFileAttributesW, SetFileInformationByHandle,
};

use super::security::OwnerOnly;
use super::wide::wide;

/// `GetDriveTypeW` の戻り値 `DRIVE_REMOTE`。
const DRIVE_REMOTE: u32 = 4;

pub(crate) fn last_error() -> u32 {
    // SAFETY: スレッドの最後のエラーを読むだけ。
    unsafe { GetLastError() }
}

/// 一時ファイルを `CREATE_NEW`・`FILE_SHARE_READ` で作る (所有者のみの DACL を渡せる)。
pub(crate) fn create_new(path: &Path, owner_only: Option<&OwnerOnly>) -> io::Result<File> {
    let w = wide(path);
    let sa = owner_only.map_or(ptr::null(), OwnerOnly::as_ptr);
    // SAFETY: `w` は NUL 終端、`sa` は null または有効な `SECURITY_ATTRIBUTES`。
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ,
            sa,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `h` は有効なファイルハンドルで、`File` が所有する (閉じるのは `File` の Drop)。
    Ok(unsafe { File::from_raw_handle(h) })
}

pub(crate) fn delete(path: &Path) -> bool {
    let w = wide(path);
    // SAFETY: NUL 終端のパス。
    unsafe { DeleteFileW(w.as_ptr()) != 0 }
}

pub(crate) fn flush(f: &File) -> bool {
    // SAFETY: 開いているファイルのハンドル。
    unsafe { FlushFileBuffers(f.as_raw_handle()) != 0 }
}

pub(crate) fn attributes(path: &Path) -> Option<u32> {
    let w = wide(path);
    // SAFETY: NUL 終端のパス。
    let a = unsafe { GetFileAttributesW(w.as_ptr()) };
    (a != INVALID_FILE_ATTRIBUTES).then_some(a)
}

pub(crate) fn set_attributes(path: &Path, attrs: u32) -> bool {
    let w = wide(path);
    // SAFETY: NUL 終端のパス。
    unsafe { SetFileAttributesW(w.as_ptr(), attrs) != 0 }
}

/// 既存のファイルを置き換えない名前の変更 (`MoveFileExW(MOVEFILE_WRITE_THROUGH)`。`MOVEFILE_REPLACE_EXISTING` なし。
/// `to` が在れば失敗する)。失敗は `GetLastError` のコード。
pub(crate) fn move_no_replace(from: &Path, to: &Path) -> Result<(), u32> {
    let (wf, wt) = (wide(from), wide(to));
    // SAFETY: NUL 終端のパス 2 つ。
    let ok = unsafe { MoveFileExW(wf.as_ptr(), wt.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if ok == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// `move_no_replace` の `io::Error` 版。
pub(crate) fn rename_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    // Win32 のエラー番号 (DWORD) を、io::Error の i32 にビットのまま入れる
    move_no_replace(from, to).map_err(|code| io::Error::from_raw_os_error(code.cast_signed()))
}

/// `ReplaceFileW(T, tmp, backup, 0)` を 1 回だけ呼ぶ (再試行は呼び側)。失敗は `GetLastError` のコード。
pub(crate) fn replace_file_once(target: &Path, tmp: &Path, backup: &Path) -> Result<(), u32> {
    let (wt, wr, wb) = (wide(target), wide(tmp), wide(backup));
    // SAFETY: NUL 終端のパス 3 つ。フラグ 0 (`IGNORE_MERGE_ERRORS` / `IGNORE_ACL_ERRORS` は付けない)。
    let ok = unsafe {
        ReplaceFileW(
            wt.as_ptr(),
            wr.as_ptr(),
            wb.as_ptr(),
            0,
            ptr::null(),
            ptr::null(),
        )
    };
    if ok != 0 {
        return Ok(());
    }
    Err(last_error())
}

/// 領域の事前確保 (`SetFileInformationByHandle(FileAllocationInfo)`。EOF は延ばさない)。失敗は `GetLastError` のコード。
pub(crate) fn set_allocation_size(file: &File, size: i64) -> Result<(), u32> {
    let info = FILE_ALLOCATION_INFO {
        AllocationSize: size,
    };
    // SAFETY: 開いているファイルのハンドルと、有効な `FILE_ALLOCATION_INFO`。
    let ok = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileAllocationInfo,
            ptr::from_ref(&info).cast(),
            u32::try_from(std::mem::size_of::<FILE_ALLOCATION_INFO>()).unwrap_or(u32::MAX),
        )
    };
    if ok != 0 {
        return Ok(());
    }
    Err(last_error())
}

/// `path` のあるボリュームの、呼び出し元が使える空き容量 (バイト)。調べられなければ `None`。
pub(crate) fn free_bytes(path: &Path) -> Option<u64> {
    let w = wide(path);
    let mut avail = 0u64;
    // SAFETY: NUL 終端のパスと、書込先の有効なポインタ (他の出力は不要なので null)。
    let ok = unsafe {
        GetDiskFreeSpaceExW(w.as_ptr(), &raw mut avail, ptr::null_mut(), ptr::null_mut())
    };
    (ok != 0).then_some(avail)
}

/// ボリューム番号・ファイルインデックス・ハードリンク数と、識別を同じファイルの判定に使ってよいか。
pub(crate) struct HandleInfo {
    pub volume: u64,
    pub index: u128,
    pub nlink: u32,
    /// `volume` と `index` で同じファイルを判定してよい (05 §5.4 の (1)): NTFS / `ReFS` のローカルのボリュームで、
    /// `index` が 0 でない。
    pub unique: bool,
}

/// ハンドルの識別 (05 §5.4)。`FileIdInfo` (128 bit。`ReFS` の識別は 64 bit に収まらない) が取れればそれを、取れない FS
/// (FAT など) では `GetFileInformationByHandle` の 64 bit の値を使う。どの経路 (`metadata`・直接上書きのハンドル) も
/// この関数で取るので、比べる値の作り方は揃う。
pub(crate) fn file_info(f: &File) -> io::Result<HandleInfo> {
    // SAFETY: 全ビット 0 の `BY_HANDLE_FILE_INFORMATION` は有効な値 (すべて整数)。
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `f` は開いているファイルのハンドル、`info` は書込先の有効な構造体。
    if unsafe { GetFileInformationByHandle(f.as_raw_handle(), &raw mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let (volume, index) = file_id_info(f).unwrap_or((
        u64::from(info.dwVolumeSerialNumber),
        (u128::from(info.nFileIndexHigh) << 32) | u128::from(info.nFileIndexLow),
    ));
    let unique = index != 0 && trusted_id_fs(f) && !is_remote_handle(f);
    Ok(HandleInfo {
        volume,
        index,
        nlink: info.nNumberOfLinks,
        unique,
    })
}

/// `GetFileInformationByHandleEx(FileIdInfo)`: (ボリュームのシリアル番号 (64 bit), ファイルの識別 (128 bit))。
fn file_id_info(f: &File) -> Option<(u64, u128)> {
    let mut info = FILE_ID_INFO::default();
    // SAFETY: `f` は開いているファイルのハンドル。書込先は `FILE_ID_INFO` で、渡す大きさはその大きさ。
    let ok = unsafe {
        GetFileInformationByHandleEx(
            f.as_raw_handle(),
            FileIdInfo,
            (&raw mut info).cast(),
            u32::try_from(std::mem::size_of::<FILE_ID_INFO>()).unwrap_or(u32::MAX),
        )
    };
    (ok != 0).then(|| {
        (
            info.VolumeSerialNumber,
            u128::from_le_bytes(info.FileId.Identifier),
        )
    })
}

/// ハンドルのボリュームの FS の名前と旗 (`GetVolumeInformationByHandleW`)。問い合わせの失敗は `None`。
pub(crate) fn volume_info(f: &File) -> Option<(String, u32)> {
    /// FS の名前の欄の長さ (`MAX_PATH + 1` 文字。API の説明が求める上限)。
    const NAME_LEN: usize = 261;
    let mut name = [0u16; NAME_LEN];
    let mut flags = 0u32;
    // SAFETY: `f` は開いているファイルのハンドル。FS の旗の書込先は `flags`、FS の名前の書込先は `name` (長さ `NAME_LEN`)。
    // 要らない出力は null と 0。
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            f.as_raw_handle(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut flags,
            name.as_mut_ptr(),
            u32::try_from(NAME_LEN).unwrap_or(0),
        )
    };
    if ok == 0 {
        return None;
    }
    let len = name.iter().position(|&c| c == 0).unwrap_or(NAME_LEN);
    Some((String::from_utf16_lossy(&name[..len]), flags))
}

/// ハンドルのボリュームの FS が NTFS か `ReFS` で、識別で開ける FS か (`GetVolumeInformationByHandleW` の FS の名前と旗)。問い合わせの失敗は
/// `false` (識別を信頼しない側)。ファイル監視の FS の判定 (05 §12.2 の 4) も同じ関数で、ユーザーモードの FS を見分ける。
pub(crate) fn trusted_id_fs(f: &File) -> bool {
    /// FS の旗 `FILE_SUPPORTS_OPEN_BY_FILE_ID` (winnt.h。windows-sys では `Win32_System_SystemServices` の feature にあり、
    /// この 1 つのために feature を足さない)。
    const FILE_SUPPORTS_OPEN_BY_FILE_ID: u32 = 0x0100_0000;
    let Some((fs, flags)) = volume_info(f) else {
        return false;
    };
    // WinFsp・Dokan などで作る FS が NTFS と名乗ることがあるので、識別で開ける FS であることも求める (05 §5.4)
    (fs.eq_ignore_ascii_case("NTFS") || fs.eq_ignore_ascii_case("ReFS"))
        && flags & FILE_SUPPORTS_OPEN_BY_FILE_ID != 0
}

/// ハンドルがネットワーク上のファイルか (`FileRemoteProtocolInfo` はリモートのファイルでだけ成功する)。ドライブの割当て
/// を通らないパス (シンボリックリンクの先が共有フォルダ) も、パスの文字列の判定 (`is_remote`) と違って見分けられる。
pub(crate) fn is_remote_handle(f: &File) -> bool {
    let mut info = FILE_REMOTE_PROTOCOL_INFO::default();
    // SAFETY: `f` は開いているファイルのハンドル。書込先は `FILE_REMOTE_PROTOCOL_INFO` で、渡す大きさはその大きさ。
    let ok = unsafe {
        GetFileInformationByHandleEx(
            f.as_raw_handle(),
            FileRemoteProtocolInfo,
            (&raw mut info).cast(),
            u32::try_from(std::mem::size_of::<FILE_REMOTE_PROTOCOL_INFO>()).unwrap_or(u32::MAX),
        )
    };
    ok != 0
}

/// 開いたハンドルが文字デバイス (`NUL`・コンソール。`FILE_TYPE_CHAR`) かパイプ (`FILE_TYPE_PIPE`) か。問い合わせの失敗
/// (`FILE_TYPE_UNKNOWN`) は `false` (呼び側が属性で決める)。05 §2.1 の `open_read()` の行。
pub(crate) fn is_char_or_pipe(f: &File) -> bool {
    // SAFETY: `f` が持つ開いたハンドルをこの呼出しの間だけ借りる (閉じない・複製しない)。種類を問い合わせるだけで、
    // ハンドルの状態を変えない。
    let t = unsafe { GetFileType(f.as_raw_handle()) };
    t == FILE_TYPE_CHAR || t == FILE_TYPE_PIPE
}

/// ネットワーク上か (UNC、またはドライブがリモート)。判定できなければ `false`。
pub(crate) fn is_remote(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let s = s.strip_prefix("\\\\?\\").unwrap_or(&s);
    if s.starts_with("UNC\\") || (s.starts_with("\\\\") && !s.starts_with("\\\\.\\")) {
        return true;
    }
    let mut it = s.chars();
    let (Some(d), Some(':')) = (it.next(), it.next()) else {
        return false;
    };
    if !d.is_ascii_alphabetic() {
        return false;
    }
    let root: Vec<u16> = format!("{}:\\", d.to_ascii_uppercase())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: `root` は NUL 終端の UTF-16。
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 空き容量を取れる() {
        let free = free_bytes(&std::env::temp_dir()).expect("空き容量");
        assert!(free > 0);
        assert!(free_bytes(Path::new(r"\\?\Z:\no\such\drive\x")).is_none());
    }
}
