//! ディレクトリの所有者の検査と空き容量の問い合わせ (docs/05 §5.6 / §7.5 の緊急退避の代替の退避先)。

#![allow(unsafe_code)]

use std::io;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, OWNER_SECURITY_INFORMATION, PSID, TOKEN_OWNER, TOKEN_QUERY, TokenOwner,
};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::save::{current_user_sid, wide};

fn sid_string(sid: PSID) -> io::Result<String> {
    let mut s: *mut u16 = ptr::null_mut();
    // SAFETY: `sid` は呼び側が有効な SID を渡す。`s` は書込先の有効なポインタ。
    if unsafe { ConvertSidToStringSidW(sid, &mut s) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `s` は NUL 終端の UTF-16 (`LocalAlloc` で確保)。読んでから `LocalFree` する。
    Ok(unsafe {
        let mut n = 0;
        while *s.add(n) != 0 {
            n += 1;
        }
        let t = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
        LocalFree(s.cast());
        t
    })
}

/// トークンの既定の所有者の SID (管理者権限で動くプロセスが作ったものは Administrators になる)。
pub(crate) fn token_owner_sid() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: 自プロセスのトークンを問い合わせ用に開く。
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        let mut len = 0u32;
        // SAFETY: 長さの問い合わせ (バッファなし)。失敗 (ERROR_INSUFFICIENT_BUFFER) が正常。
        unsafe { GetTokenInformation(token, TokenOwner, ptr::null_mut(), 0, &mut len) };
        let mut buf = vec![0u8; len as usize];
        // SAFETY: `buf` は `len` バイトの書込先。
        if unsafe { GetTokenInformation(token, TokenOwner, buf.as_mut_ptr().cast(), len, &mut len) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `TOKEN_OWNER` が先頭に書かれる。アラインに依らないよう `read_unaligned` で読む。
        let owner: TOKEN_OWNER = unsafe { ptr::read_unaligned(buf.as_ptr().cast()) };
        sid_string(owner.Owner)
    })();
    // SAFETY: 上で開いたトークンのハンドル。
    unsafe { CloseHandle(token) };
    result
}

/// `path` の所有者の SID。
pub(crate) fn owner_sid_of(path: &Path) -> io::Result<String> {
    let w = wide(path);
    let mut owner: PSID = ptr::null_mut();
    let mut descriptor: *mut core::ffi::c_void = ptr::null_mut();
    // SAFETY: NUL 終端のパス、有効な書込先。`descriptor` は成功時に `LocalAlloc` で確保され、`owner` はその中を指す。
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc as i32));
    }
    let r = sid_string(owner);
    // SAFETY: 上で確保された記述子。`owner` はこの後使わない。
    unsafe { LocalFree(descriptor) };
    r
}

/// `path` の所有者が、現在のユーザーまたはトークンの既定の所有者か (05 §7.5。他のユーザーが先に作った場所に書かない)。
pub(crate) fn owned_by_me(path: &Path) -> io::Result<bool> {
    let owner = owner_sid_of(path)?;
    Ok(owner == current_user_sid()? || token_owner_sid().is_ok_and(|t| t == owner))
}

/// `path` のあるボリュームの、呼び出し元が使える空き容量 (バイト)。調べられなければ `None`。
pub(crate) fn free_bytes(path: &Path) -> Option<u64> {
    let w = wide(path);
    let mut avail = 0u64;
    // SAFETY: NUL 終端のパスと、書込先の有効なポインタ (他の出力は不要なので null)。
    let ok =
        unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, ptr::null_mut(), ptr::null_mut()) };
    (ok != 0).then_some(avail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 自分で作ったディレクトリは自分の所有() {
        let d = std::env::temp_dir().join(format!("eve-owner-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(owned_by_me(&d).unwrap());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn 他者が所有するディレクトリは自分の所有ではない() {
        // C:\Windows は TrustedInstaller の所有 (管理者権限で動いていても Administrators ではない)
        let win: std::path::PathBuf =
            std::env::var_os("SystemRoot").map_or_else(|| r"C:\Windows".into(), Into::into);
        assert!(!owned_by_me(&win).unwrap());
    }

    #[test]
    fn 空き容量を取れる() {
        let free = free_bytes(&std::env::temp_dir()).expect("空き容量");
        assert!(free > 0);
        assert!(free_bytes(Path::new(r"\\?\Z:\no\such\drive\x")).is_none());
    }
}
