//! SID と所有者のみの DACL (docs/05 §5.6 / §7.5)。

use std::io;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, OWNER_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES, TOKEN_OWNER,
    TOKEN_QUERY, TOKEN_USER, TokenOwner, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::wide::{wide, wide_plain};

// ---------- 所有者のみの DACL (05 §5.6) ----------

/// 「所有者のみ」の `SECURITY_ATTRIBUTES` (現在のユーザーと SYSTEM だけにフルアクセス、継承を遮断)。
pub(crate) struct OwnerOnly {
    descriptor: *mut core::ffi::c_void,
    attrs: SECURITY_ATTRIBUTES,
}

impl OwnerOnly {
    /// ファイル用 (継承は遮断)。
    pub(crate) fn new() -> io::Result<OwnerOnly> {
        OwnerOnly::with_aces("")
    }

    /// ディレクトリ用 (中のファイルとディレクトリへ継承させる)。
    pub(crate) fn new_dir() -> io::Result<OwnerOnly> {
        OwnerOnly::with_aces("OICI")
    }

    fn with_aces(inherit: &str) -> io::Result<OwnerOnly> {
        let sid = current_user_sid()?;
        let sddl: Vec<u16> = format!("O:{sid}D:P(A;{inherit};FA;;;{sid})(A;{inherit};FA;;;SY)")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor: *mut core::ffi::c_void = ptr::null_mut();
        // SAFETY: `sddl` は NUL 終端の有効な文字列、`descriptor` は書込先の有効なポインタ。
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(OwnerOnly {
            descriptor,
            attrs: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    pub(crate) fn as_ptr(&self) -> *const SECURITY_ATTRIBUTES {
        &raw const self.attrs
    }
}

impl Drop for OwnerOnly {
    fn drop(&mut self) {
        // SAFETY: `descriptor` は `ConvertStringSecurityDescriptorToSecurityDescriptorW` が `LocalAlloc` で確保したもの。
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}

/// 1 段だけ作る (所有者のみの保護付き DACL。中のファイルとディレクトリへ継承させる)。在れば何もしない。
/// パスは拡張長形式にしない (`wide_plain`)。
pub(crate) fn create_dir_private(dir: &Path) -> io::Result<()> {
    let sa = OwnerOnly::new_dir()?;
    let w = wide_plain(dir);
    // SAFETY: NUL 終端のパス、有効な `SECURITY_ATTRIBUTES`。
    let ok = unsafe { CreateDirectoryW(w.as_ptr(), sa.as_ptr()) };
    if ok == 0 {
        let e = io::Error::last_os_error();
        // 並行して作られた
        if dir.is_dir() {
            return Ok(());
        }
        return Err(e);
    }
    Ok(())
}

// ---------- SID ----------

/// SID を文字列 (`S-1-5-21-…`) にする。
fn sid_string(sid: PSID) -> io::Result<String> {
    let mut s: *mut u16 = ptr::null_mut();
    // SAFETY: `sid` は呼び側が有効な SID を渡す。`s` は書込先の有効なポインタ。
    if unsafe { ConvertSidToStringSidW(sid, &raw mut s) } == 0 {
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

/// 現在のユーザーの SID の文字列 (`S-1-5-21-…`)。
pub(crate) fn current_user_sid() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: 自プロセスのトークンを問い合わせ用に開く。
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        let mut len = 0u32;
        // SAFETY: 長さの問い合わせ (バッファなし)。失敗 (ERROR_INSUFFICIENT_BUFFER) が正常。
        unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &raw mut len) };
        let mut buf = vec![0u8; len as usize];
        // SAFETY: `buf` は `len` バイトの書込先。
        if unsafe {
            GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &raw mut len)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `GetTokenInformation(TokenUser)` は `TOKEN_USER` を書く (先頭に置かれ、アラインは確保時の `Vec<u8>` に依らず
        // `read_unaligned` で読む)。
        let user: TOKEN_USER = unsafe { ptr::read_unaligned(buf.as_ptr().cast()) };
        // `user.User.Sid` は `buf` の中の有効な SID (`buf` はこの後まで生きている)
        sid_string(user.User.Sid)
    })();
    // SAFETY: 上で開いたトークンのハンドル。
    unsafe { CloseHandle(token) };
    result
}

/// トークンの既定の所有者の SID (管理者権限で動くプロセスが作ったものは Administrators になる)。
fn token_owner_sid() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: 自プロセスのトークンを問い合わせ用に開く。
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        let mut len = 0u32;
        // SAFETY: 長さの問い合わせ (バッファなし)。失敗 (ERROR_INSUFFICIENT_BUFFER) が正常。
        unsafe { GetTokenInformation(token, TokenOwner, ptr::null_mut(), 0, &raw mut len) };
        let mut buf = vec![0u8; len as usize];
        // SAFETY: `buf` は `len` バイトの書込先。
        if unsafe {
            GetTokenInformation(
                token,
                TokenOwner,
                buf.as_mut_ptr().cast(),
                len,
                &raw mut len,
            )
        } == 0
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
fn owner_sid_of(path: &Path) -> io::Result<String> {
    let w = wide(path);
    let mut owner: PSID = ptr::null_mut();
    let mut descriptor: *mut core::ffi::c_void = ptr::null_mut();
    // SAFETY: NUL 終端のパス、有効な書込先。`descriptor` は成功時に `LocalAlloc` で確保され、`owner` はその中を指す。
    let rc = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &raw mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut descriptor,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 所有者のみの_dacl_を作れる() {
        assert!(OwnerOnly::new().is_ok());
        let sid = current_user_sid().unwrap();
        // ローカル (S-1-5-21-…)・Microsoft アカウント / Entra ID (S-1-12-1-…) のどれでも SID の形
        assert!(sid.starts_with("S-1-"), "{sid}");
    }

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
}
