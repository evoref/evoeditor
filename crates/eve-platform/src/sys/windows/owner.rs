//! 所有者の検査 (docs/07 §12.1 の OWN-1〜OWN-7、05 §5.7) の Windows の API: ディレクトリの固定、属性と再解析のタグ、
//! 所有者と DACL、ハンドルの最終のパス、「自分たち」の SID、SID の名前。判定の規則そのものは `read_checked` (unsafe なし)。
//!
//! spec: 05 §5.7, 07 §12.1 (OWN-2, OWN-3, OWN-6)

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::ptr;
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSidToSidW, GetSecurityInfo, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, GetAce, GetLengthSid, GetTokenInformation,
    INHERIT_ONLY_ACE, IsValidSid, LookupAccountSidW, OWNER_SECURITY_INFORMATION, PSID,
    SID_NAME_USE, TOKEN_OWNER, TOKEN_QUERY, TOKEN_USER, TokenOwner, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, ExtendedFileIdType, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_128,
    FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0, FILE_ID_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FileAttributeTagInfo, FileIdInfo, GetFileAttributesW,
    GetFileInformationByHandle, GetFileInformationByHandleEx, GetFinalPathNameByHandleW,
    INVALID_FILE_ATTRIBUTES, OPEN_EXISTING, OpenFileById, READ_CONTROL, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT, VOLUME_NAME_DOS, VOLUME_NAME_NT,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::file::last_error;
use super::security::sid_string;
use super::wide::wide;

/// 二進の SID (`EqualSid` はバイトの並びの比較と同じ)。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Sid(Vec<u8>);

/// SID の頭 (版・副権限の数・識別子の権威) の大きさ。副権限は 1 つ 4 バイト。
const SID_HEADER_LEN: usize = 8;

impl Sid {
    /// `psid` の指す SID を写す。読む前に、頭と副権限の数から求めた長さが `max_len` に収まることを確かめる。
    ///
    /// # Safety
    ///
    /// `psid` は null か、次を満たす領域の先頭を指すこと (この関数の間は解放されない): 先頭から `max_len` バイトか、
    /// 頭の副権限の数から求めた長さ (8 + 4 × 数) の小さいほうまでが読める。`max_len` が 8 以上なら先頭 8 バイトは読める。
    unsafe fn copy_from(psid: PSID, max_len: usize) -> Option<Sid> {
        if psid.is_null() || max_len < SID_HEADER_LEN {
            return None;
        }
        // SAFETY: `psid` は null でなく、呼び側の約束で `max_len` (≥ 8) バイト読める。副権限の数は 2 バイト目 (添字 1)。
        let count = usize::from(unsafe { psid.cast::<u8>().add(1).read() });
        let need = SID_HEADER_LEN + 4 * count;
        if need > max_len {
            return None;
        }
        // SAFETY: `psid` から `need` (≤ `max_len`) バイトは読める。`IsValidSid` が読むのは頭と副権限の範囲だけ。
        if unsafe { IsValidSid(psid) } == 0 {
            return None;
        }
        // SAFETY: 形の正しい SID の長さを問い合わせるだけ (頭の副権限の数から求める値で、`need` と同じ)。
        let len = unsafe { GetLengthSid(psid) } as usize;
        if len != need {
            return None;
        }
        // SAFETY: `psid` から `len` バイト (≤ `max_len`) は呼び側の約束で読める。写した後は元を参照しない。
        let bytes = unsafe { std::slice::from_raw_parts(psid.cast::<u8>(), len) }.to_vec();
        Some(Sid(bytes))
    }

    /// 決まった SID の文字列 (`S-1-5-18` など) から作る。
    fn parse(s: &str) -> Option<Sid> {
        let w: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
        let mut psid: PSID = ptr::null_mut();
        // SAFETY: `w` は NUL 終端の UTF-16、`psid` は書込先の有効なポインタ。成功すると `LocalAlloc` の領域を指す。
        if unsafe { ConvertStringSidToSidW(w.as_ptr(), &raw mut psid) } == 0 {
            return None;
        }
        // SAFETY: `psid` は `ConvertStringSidToSidW` が作った形の正しい SID で、副権限は最大 15 個 (8 + 4 * 15 = 68 バイト)。
        // `copy_from` は頭の副権限の数から求めた長さまでしか読まない。
        let sid = unsafe { Sid::copy_from(psid, SID_HEADER_LEN + 4 * 15) };
        // SAFETY: 上で `LocalAlloc` された領域。この後は使わない。
        unsafe { LocalFree(psid) };
        sid
    }

    /// 判定の規則の試験用 (比べるだけなので、OS に問い合わせずにバイトの並びから作る)。
    #[cfg(test)]
    pub(crate) fn from_bytes_for_test(bytes: &[u8]) -> Sid {
        Sid(bytes.to_vec())
    }

    fn as_psid(&self) -> PSID {
        // 読むだけの API (`LookupAccountSidW`・`ConvertSidToStringSidW`) に渡す
        self.0.as_ptr().cast_mut().cast()
    }
}

/// 「自分たち」(07 §12.1 の OWN-2 の (1)): 今のユーザー・トークンの既定の所有者・SYSTEM・Administrators・TrustedInstaller と、
/// 書込を数えない CREATOR OWNER・OWNER RIGHTS。プロセスで 1 回だけ作る。
#[derive(Debug)]
pub(crate) struct Trusted {
    /// 所有者として受け入れる SID。
    pub owners: Vec<Sid>,
    /// 書込の許可を数えない SID (`owners` に加えて CREATOR OWNER・OWNER RIGHTS)。
    pub writers: Vec<Sid>,
}

/// `NT SERVICE\TrustedInstaller` の SID。
const TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

/// 「自分たち」の SID。トークンを読めない・SID を作れないときは `None` (判定できないとする)。
pub(crate) fn trusted() -> Option<&'static Trusted> {
    static TRUSTED: OnceLock<Option<Trusted>> = OnceLock::new();
    TRUSTED
        .get_or_init(|| {
            let mut owners = vec![token_sid(TokenUser)?, token_sid(TokenOwner)?];
            for s in ["S-1-5-18", "S-1-5-32-544", TRUSTED_INSTALLER] {
                owners.push(Sid::parse(s)?);
            }
            let mut writers = owners.clone();
            for s in ["S-1-3-0", "S-1-3-4"] {
                writers.push(Sid::parse(s)?);
            }
            Some(Trusted { owners, writers })
        })
        .as_ref()
}

/// 自プロセスのトークンの `TokenUser` か `TokenOwner` の SID。
fn token_sid(class: i32) -> Option<Sid> {
    let mut token = ptr::null_mut();
    // SAFETY: 自プロセスのトークンを問い合わせ用に開く。`token` は書込先の有効なポインタ。
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
        return None;
    }
    let mut len = 0u32;
    // SAFETY: 長さの問い合わせ (バッファなし)。失敗 (ERROR_INSUFFICIENT_BUFFER) が正常。
    unsafe { GetTokenInformation(token, class, ptr::null_mut(), 0, &raw mut len) };
    // `TOKEN_USER` / `TOKEN_OWNER` はポインタを持つので、作業領域を 8 バイトに整列させる
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    let cap = u32::try_from(buf.len() * 8).unwrap_or(0);
    // SAFETY: `buf` は `cap` (≥ `len`) バイトの、8 バイトに整列した書込先。
    let ok =
        unsafe { GetTokenInformation(token, class, buf.as_mut_ptr().cast(), cap, &raw mut len) }
            != 0;
    // SAFETY: 上で開いたトークンのハンドル。この後は使わない。
    unsafe { windows_sys::Win32::Foundation::CloseHandle(token) };
    if !ok {
        return None;
    }
    let base = buf.as_ptr().cast::<u8>();
    // `TOKEN_USER` と `TOKEN_OWNER` はどちらも先頭に SID のポインタを持ち、SID は `buf` の中を指す
    let psid = if class == TokenUser {
        // SAFETY: `GetTokenInformation(TokenUser)` が整列した `buf` の先頭に `TOKEN_USER` を書いた。
        unsafe { base.cast::<TOKEN_USER>().read() }.User.Sid
    } else {
        // SAFETY: `GetTokenInformation(TokenOwner)` が整列した `buf` の先頭に `TOKEN_OWNER` を書いた。
        unsafe { base.cast::<TOKEN_OWNER>().read() }.Owner
    };
    // `psid` は `buf` (この後まで生きている) の中の SID を指す。`buf` の終わりを越えて読まないよう、残りの長さを求める
    let rest = (psid as usize)
        .checked_sub(base as usize)
        .and_then(|off| (len as usize).min(buf.len() * 8).checked_sub(off))?;
    // SAFETY: 上で `psid` が `buf` の中を指し、そこから `rest` バイトが読めることを確かめた。
    unsafe { Sid::copy_from(psid, rest) }
}

/// SID の名前を引く回数の上限 (プロセスで)。孤立した SID (消えたドメインのアカウント) は `LookupAccountSidW` が DC を待つので、
/// 引く回数を抑える。上限を超えたら引かずに `S-1-…` にする。
const LOOKUP_LIMIT: usize = 64;

/// SID の名前 (`NT AUTHORITY\Authenticated Users` など)。名前に直せなければ `S-1-…`、それもできなければ `?`。
/// 結果は SID ごとにプロセスで覚える (引けなかったものも)。
pub(crate) fn account_name(sid: &Sid) -> String {
    static CACHE: OnceLock<Mutex<HashMap<Sid, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let c = cache.lock().expect("SID の名前の表の Mutex が壊れた");
        if let Some(n) = c.get(sid) {
            return n.clone();
        }
        if c.len() >= LOOKUP_LIMIT {
            return sid_string(sid.as_psid()).unwrap_or_else(|_| "?".to_owned());
        }
    }
    // 引く間は表を握らない (DC を待つ間に、ほかのジョブの判定を止めない)
    let name = lookup_name(sid)
        .unwrap_or_else(|| sid_string(sid.as_psid()).unwrap_or_else(|_| "?".to_owned()));
    cache
        .lock()
        .expect("SID の名前の表の Mutex が壊れた")
        .insert(sid.clone(), name.clone());
    name
}

fn lookup_name(sid: &Sid) -> Option<String> {
    let mut name = [0u16; 256];
    let mut domain = [0u16; 256];
    let (mut n, mut d) = (256u32, 256u32);
    let mut use_ = SID_NAME_USE::default();
    // SAFETY: `sid` は形の正しい SID (`Sid` は確かめて写したもの)。名前とドメインの書込先は長さ 256 の配列で、長さを渡す。
    let ok = unsafe {
        LookupAccountSidW(
            ptr::null(),
            sid.as_psid(),
            name.as_mut_ptr(),
            &raw mut n,
            domain.as_mut_ptr(),
            &raw mut d,
            &raw mut use_,
        )
    } != 0;
    if !ok {
        return None;
    }
    let name = String::from_utf16_lossy(&name[..(n as usize).min(name.len())]);
    let domain = String::from_utf16_lossy(&domain[..(d as usize).min(domain.len())]);
    Some(if domain.is_empty() {
        name
    } else {
        format!("{domain}\\{name}")
    })
}

/// DACL の ACE 1 つ (判定に要る分だけ)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ace {
    /// 許可 (`ACCESS_ALLOWED_ACE_TYPE`)。
    Allowed {
        /// `INHERIT_ONLY_ACE` (この物には効かない)。
        inherit_only: bool,
        /// 許可する権利。
        mask: u32,
        /// 許可する相手。
        sid: Sid,
    },
    /// 拒否 (`ACCESS_DENIED_ACE_TYPE`・`ACCESS_DENIED_OBJECT_ACE_TYPE`・`ACCESS_DENIED_CALLBACK*`)。数えない。
    Denied,
    /// そのほかの型 (オブジェクト・コールバックの許可など)、または読めない ACE。書けるとみなす。
    Other {
        /// ACE の型 (`ACE_HEADER.AceType`。読めなければ 0xFF)。
        ace_type: u8,
    },
}

/// 所有者と DACL。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Security {
    /// 所有者の SID。
    pub owner: Sid,
    /// DACL (`None` は NULL の DACL = 全員にフルアクセス)。
    pub dacl: Option<Vec<Ace>>,
}

/// ACE の型 (winnt.h。windows-sys では `Win32_System_SystemServices` の feature にあり、そのために feature を足さない)。
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0x0;
const ACCESS_DENIED_ACE_TYPE: u8 = 0x1;
const ACCESS_DENIED_OBJECT_ACE_TYPE: u8 = 0x6;
const ACCESS_DENIED_CALLBACK_ACE_TYPE: u8 = 0xA;
const ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE: u8 = 0xC;

/// ハンドルの所有者と DACL (`GetSecurityInfo`)。
pub(crate) fn security(f: &File) -> io::Result<Security> {
    let mut owner: PSID = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor: *mut core::ffi::c_void = ptr::null_mut();
    // SAFETY: `f` は `READ_CONTROL` を付けて開いたハンドル。書込先はどれも有効なポインタ。成功すると `descriptor` は
    // `LocalAlloc` の領域で、`owner` と `dacl` はその中を指す (DACL が NULL なら `dacl` は null)。
    let rc = unsafe {
        GetSecurityInfo(
            f.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &raw mut owner,
            ptr::null_mut(),
            &raw mut dacl,
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if rc != 0 {
        // Win32 のエラー番号 (DWORD) を、io::Error の i32 にビットのまま入れる
        return Err(io::Error::from_raw_os_error(rc.cast_signed()));
    }
    // SAFETY: `owner` は `descriptor` の中の SID (null なら `None`)。`descriptor` はこの後で解放する。
    let owner = unsafe { Sid::copy_from(owner, usize::MAX) };
    let dacl = if dacl.is_null() {
        None
    } else {
        // SAFETY: `dacl` は `descriptor` の中の形の正しい ACL。
        Some(unsafe { read_aces(dacl) })
    };
    // SAFETY: 上で `GetSecurityInfo` が `LocalAlloc` で確保した記述子。`owner`・`dacl` は写し終えた。
    unsafe { LocalFree(descriptor) };
    match owner {
        Some(owner) => Ok(Security { owner, dacl }),
        None => Err(io::Error::other("所有者の SID を読めない")),
    }
}

/// ACL の ACE を写す。
///
/// # Safety
///
/// `acl` は形の正しい ACL を指し、この関数の間は解放されないこと。
unsafe fn read_aces(acl: *mut ACL) -> Vec<Ace> {
    // SAFETY: 呼び側の約束で `acl` は ACL の先頭を指す。アラインに依らず読む。
    let header: ACL = unsafe { ptr::read_unaligned(acl) };
    let mut out = Vec::with_capacity(usize::from(header.AceCount));
    for i in 0..u32::from(header.AceCount) {
        let mut pace: *mut core::ffi::c_void = ptr::null_mut();
        // SAFETY: `acl` は形の正しい ACL、`i` は `AceCount` 未満。成功すると `pace` は ACL の中の ACE を指す。
        if unsafe { GetAce(acl, i, &raw mut pace) } == 0 || pace.is_null() {
            out.push(Ace::Other { ace_type: 0xFF });
            continue;
        }
        // SAFETY: `pace` は ACL の中の ACE の先頭 (少なくとも `ACE_HEADER` の大きさがある)。
        let h: ACE_HEADER = unsafe { ptr::read_unaligned(pace.cast::<ACE_HEADER>()) };
        let ace = match h.AceType {
            ACCESS_ALLOWED_ACE_TYPE => {
                // 形: ACE_HEADER (4 バイト) + Mask (u32) + SID。SID は ACE の大きさの中に収まっていること
                let size = usize::from(h.AceSize);
                if size < 8 {
                    Ace::Other { ace_type: 0xFF }
                } else {
                    let base = pace.cast::<u8>();
                    // SAFETY: ACE は `AceSize` (≥ 8) バイトあり、Mask は先頭から 4 バイト目にある。
                    let mask = unsafe { ptr::read_unaligned(base.add(4).cast::<u32>()) };
                    // SAFETY: SID は先頭から 8 バイト目から ACE の終わり (`size - 8` バイト) までにある。
                    match unsafe { Sid::copy_from(base.add(8).cast(), size - 8) } {
                        Some(sid) => Ace::Allowed {
                            inherit_only: u32::from(h.AceFlags) & INHERIT_ONLY_ACE != 0,
                            mask,
                            sid,
                        },
                        None => Ace::Other {
                            ace_type: h.AceType,
                        },
                    }
                }
            }
            ACCESS_DENIED_ACE_TYPE
            | ACCESS_DENIED_OBJECT_ACE_TYPE
            | ACCESS_DENIED_CALLBACK_ACE_TYPE
            | ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE => Ace::Denied,
            t => Ace::Other { ace_type: t },
        };
        out.push(ace);
    }
    out
}

/// ディレクトリを開いて固定する (07 §12.1 の OWN-6)。`READ_CONTROL`・`FILE_LIST_DIRECTORY`、共有は `FILE_SHARE_READ |
/// FILE_SHARE_WRITE` で `FILE_SHARE_DELETE` を付けない (開いている間、そのディレクトリと祖先を改名・削除できない)。
/// `no_follow` なら `FILE_FLAG_OPEN_REPARSE_POINT` を付ける (`.eve` がリンクならリンクそのものを開く)。
/// ファイルのパスでも開ける (`FILE_FLAG_BACKUP_SEMANTICS`)。ディレクトリかは呼び側が `attr_tag` で確かめる。
pub(crate) fn open_dir(path: &Path, no_follow: bool) -> io::Result<File> {
    let flags = FILE_FLAG_BACKUP_SEMANTICS
        | if no_follow {
            FILE_FLAG_OPEN_REPARSE_POINT
        } else {
            0
        };
    open(
        path,
        READ_CONTROL | FILE_LIST_DIRECTORY,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        flags,
    )
}

/// 読むファイルを開く (`GENERIC_READ | READ_CONTROL`、共有は 3 つとも)。`no_follow` なら `FILE_FLAG_OPEN_REPARSE_POINT` を
/// 付ける。どちらも `SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION` を付ける (名前付きパイプへ誘われても成りすまされない)。
pub(crate) fn open_file(path: &Path, no_follow: bool) -> io::Result<File> {
    let flags = SECURITY_SQOS_PRESENT
        | SECURITY_IDENTIFICATION
        | if no_follow {
            FILE_FLAG_OPEN_REPARSE_POINT
        } else {
            0
        };
    open(
        path,
        GENERIC_READ | READ_CONTROL,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        flags,
    )
}

fn open(path: &Path, access: u32, share: u32, flags: u32) -> io::Result<File> {
    let w = wide(path);
    // SAFETY: `w` は NUL 終端の UTF-16。セキュリティ属性とテンプレートは null。
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            access,
            share,
            ptr::null(),
            OPEN_EXISTING,
            flags,
            ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `h` は有効なハンドルで、`File` が所有する (閉じるのは `File` の Drop)。
    Ok(unsafe { File::from_raw_handle(h) })
}

/// パスの属性 (`GetFileAttributesW`。最後の要素のリンクは辿らない)。無ければ `Ok(None)`。
pub(crate) fn path_attributes(path: &Path) -> io::Result<Option<u32>> {
    let w = wide(path);
    // SAFETY: NUL 終端のパス。
    let a = unsafe { GetFileAttributesW(w.as_ptr()) };
    if a != INVALID_FILE_ATTRIBUTES {
        return Ok(Some(a));
    }
    match last_error() {
        // ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND
        2 | 3 => Ok(None),
        code => Err(io::Error::from_raw_os_error(code.cast_signed())),
    }
}

/// 再解析のタグが分からないときに返すタグ (名前の代理の印だけを立てる。呼び側はリンクとして扱い、読まない側に倒れる)。
const UNKNOWN_SURROGATE_TAG: u32 = 0x2000_0000;

/// ハンドルの属性と再解析のタグ (`FileAttributeTagInfo`)。`FileAttributeTagInfo` に答えない FS (exFAT。
/// `ERROR_INVALID_PARAMETER`) では `GetFileInformationByHandle` の属性を使い、再解析点の属性があればタグを
/// `UNKNOWN_SURROGATE_TAG` にする (名前の代理かを確かめられないので辿らない側)。
pub(crate) fn attr_tag(f: &File) -> io::Result<(u32, u32)> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: `f` は開いているハンドル。書込先は `FILE_ATTRIBUTE_TAG_INFO` で、渡す大きさはその大きさ。
    let ok = unsafe {
        GetFileInformationByHandleEx(
            f.as_raw_handle(),
            FileAttributeTagInfo,
            (&raw mut info).cast(),
            u32::try_from(std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>()).unwrap_or(u32::MAX),
        )
    };
    if ok != 0 {
        return Ok((info.FileAttributes, info.ReparseTag));
    }
    // SAFETY: 全ビット 0 の `BY_HANDLE_FILE_INFORMATION` は有効な値 (すべて整数)。
    let mut basic: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `f` は開いているハンドル、`basic` は書込先の有効な構造体。
    if unsafe { GetFileInformationByHandle(f.as_raw_handle(), &raw mut basic) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let attrs = basic.dwFileAttributes;
    let tag = if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        UNKNOWN_SURROGATE_TAG
    } else {
        0
    };
    Ok((attrs, tag))
}

/// ハンドルの最終のパス (`GetFinalPathNameByHandleW`、`VOLUME_NAME_NT` の形。ドライブ文字の無いボリュームでも取れる)。
/// 開いた物どうしの親子を比べるのに使う。
pub(crate) fn final_path(f: &File) -> io::Result<Vec<u16>> {
    final_path_as(f, VOLUME_NAME_NT)
}

/// ハンドルの最終のパス (`VOLUME_NAME_DOS` の形。`\\?\C:\…`・`\\?\UNC\server\share\…`)。
/// `workspace.safe_directories` (`std::fs::canonicalize` の形) と比べるのに使う (07 §12.1 の OWN-5)。
pub(crate) fn final_path_dos(f: &File) -> io::Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    final_path_as(f, VOLUME_NAME_DOS)
        .map(|w| std::path::PathBuf::from(std::ffi::OsString::from_wide(&w)))
}

fn final_path_as(f: &File, form: u32) -> io::Result<Vec<u16>> {
    let mut buf = vec![0u16; 512];
    loop {
        let cap = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        // SAFETY: `f` は開いているハンドル。書込先は `buf` で、長さ `cap` 文字。
        let n = unsafe { GetFinalPathNameByHandleW(f.as_raw_handle(), buf.as_mut_ptr(), cap, form) }
            as usize;
        if n == 0 {
            return Err(io::Error::last_os_error());
        }
        if n < buf.len() {
            buf.truncate(n);
            return Ok(buf);
        }
        // 足りないときは NUL を含む長さが返る
        if n > 0x8000 {
            return Err(io::Error::other("最終のパスが長すぎる"));
        }
        buf.resize(n + 1, 0);
    }
}

/// ハンドルの 128 bit のファイルの識別 (`FileIdInfo`)。0 は「識別が無い」として `None`。
pub(crate) fn file_id_128(f: &File) -> Option<[u8; 16]> {
    let mut info = FILE_ID_INFO::default();
    // SAFETY: `f` は開いているハンドル。書込先は `FILE_ID_INFO` で、渡す大きさはその大きさ。
    let ok = unsafe {
        GetFileInformationByHandleEx(
            f.as_raw_handle(),
            FileIdInfo,
            (&raw mut info).cast(),
            u32::try_from(std::mem::size_of::<FILE_ID_INFO>()).unwrap_or(u32::MAX),
        )
    };
    let id = info.FileId.Identifier;
    (ok != 0 && id != [0; 16]).then_some(id)
}

/// 同じボリュームのファイルを識別で開き直す (`OpenFileById`、`ExtendedFileIdType`)。名前を解かないので、名前を差し替えても
/// 別の物 (リンクの先) を開かない。`FILE_FLAG_OPEN_REPARSE_POINT` を付けないので、再解析点のフィルタ (Cloud Files・
/// 重複除去・WOF) を通した中身を読める。共有は 3 つとも、`SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION` を付ける。
/// `hint` は同じボリュームの開いたハンドル (固定したディレクトリ)。
pub(crate) fn open_by_id(hint: &File, id: [u8; 16]) -> io::Result<File> {
    let desc = FILE_ID_DESCRIPTOR {
        dwSize: u32::try_from(std::mem::size_of::<FILE_ID_DESCRIPTOR>()).unwrap_or(u32::MAX),
        Type: ExtendedFileIdType,
        Anonymous: FILE_ID_DESCRIPTOR_0 {
            ExtendedFileId: FILE_ID_128 { Identifier: id },
        },
    };
    // SAFETY: `hint` は開いているハンドル、`desc` は大きさを書いた有効な記述子。セキュリティ属性は null。
    let h = unsafe {
        OpenFileById(
            hint.as_raw_handle(),
            &raw const desc,
            GENERIC_READ | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `h` は有効なハンドルで、`File` が所有する。
    Ok(unsafe { File::from_raw_handle(h) })
}

/// 試験用: ファイルに再解析点を付ける (`FSCTL_SET_REPARSE_POINT`。Microsoft のでないタグは GUID 付きの形)。
/// 一般のユーザーでも、自分の書けるファイルに付けられる。
#[cfg(test)]
pub(crate) fn set_reparse_for_test(path: &Path, tag: u32) -> io::Result<()> {
    /// winioctl.h の `FSCTL_SET_REPARSE_POINT` (windows-sys では `Win32_System_Ioctl` にあり、feature を足さない)。
    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
    let f = open(
        path,
        windows_sys::Win32::Foundation::GENERIC_WRITE,
        0,
        FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
    )?;
    // REPARSE_GUID_DATA_BUFFER: タグ (u32)・データの長さ (u16)・予約 (u16)・GUID (16 バイト)・データ (ここでは 4 バイト)
    let data = [1u8, 2, 3, 4];
    let mut buf = Vec::new();
    buf.extend_from_slice(&tag.to_le_bytes());
    buf.extend_from_slice(&u16::try_from(data.len()).unwrap_or(0).to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&[0x45; 16]);
    buf.extend_from_slice(&data);
    let mut ret = 0u32;
    // SAFETY: `f` は書込のために開いたハンドル。入力は `buf` (長さを渡す)、出力は無し。同期の呼出し。
    let ok = unsafe {
        windows_sys::Win32::System::IO::DeviceIoControl(
            f.as_raw_handle(),
            FSCTL_SET_REPARSE_POINT,
            buf.as_ptr().cast(),
            u32::try_from(buf.len()).unwrap_or(0),
            ptr::null_mut(),
            0,
            &raw mut ret,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `OpenFileById` の経路 (許可リストの再解析点のファイルを開き直す) を、ふつうのファイルで確かめる: 固定したディレクトリを
    /// 手掛かりに識別で開き直したハンドルから同じ中身が読め、識別を壊すと開けない。
    #[test]
    fn 識別で開き直して同じ中身を読み_壊した識別は開けない() {
        use std::io::Read;
        let base = std::env::temp_dir().join(format!("eve-byid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let f = base.join("config.toml");
        std::fs::write(&f, b"a = 1\n").unwrap();
        let dh = open_dir(&base, false).unwrap();
        let fh = open_file(&f, true).unwrap();
        let id = file_id_128(&fh).expect("NTFS の識別");
        let mut again = open_by_id(&dh, id).expect("識別で開ける");
        let mut body = Vec::new();
        again.read_to_end(&mut body).unwrap();
        assert_eq!(body, b"a = 1\n");
        assert_eq!(file_id_128(&again), Some(id), "同じ識別");
        let mut broken = id;
        broken[15] ^= 0x5A;
        broken[0] ^= 0xA5;
        assert!(open_by_id(&dh, broken).is_err(), "壊した識別");
        drop((dh, fh, again));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn 形の壊れた_sid_は写さない() {
        // 副権限の数 (2 バイト目) が、読める長さに収まらない
        let mut b = [1u8, 15, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
        // SAFETY: `b` は 12 バイト読める。`max_len` に 12 を渡すので、頭から求めた長さ (68) は読まずに断る。
        let broken = unsafe { Sid::copy_from(b.as_mut_ptr().cast(), b.len()) };
        assert_eq!(broken, None);
        // 頭も読めない長さ
        // SAFETY: `max_len` が 8 未満なので何も読まない。
        assert_eq!(unsafe { Sid::copy_from(b.as_mut_ptr().cast(), 4) }, None);
        b[1] = 1;
        // SAFETY: `b` は 12 バイトで、頭から求めた長さ (12) と同じ。
        let s = unsafe { Sid::copy_from(b.as_mut_ptr().cast(), b.len()) }.expect("S-1-5-18");
        assert_eq!(s, Sid::parse("S-1-5-18").unwrap());
    }

    #[test]
    fn 自分たちの_sid_を作れる() {
        let t = trusted().expect("トークンを読める");
        assert_eq!(t.owners.len(), 5);
        assert_eq!(t.writers.len(), 7);
        // 今のユーザーの SID は既存の文字列の関数と一致する
        assert_eq!(
            sid_string(t.owners[0].as_psid()).unwrap(),
            super::super::current_user_sid().unwrap()
        );
        assert_ne!(account_name(&Sid::parse("S-1-5-18").unwrap()), "");
    }

    #[test]
    fn 名前に直せない_sid_は文字列のまま() {
        let s = Sid::parse("S-1-5-21-1-2-3-4242").unwrap();
        assert_eq!(account_name(&s), "S-1-5-21-1-2-3-4242");
    }

    /// 開いている間は、ディレクトリとその祖先を改名できない (07 §12.1 の OWN-6 の固定。S-24 の (d))。閉じれば改名できる。
    #[test]
    fn 開いたディレクトリと祖先は改名できない() {
        let base = std::env::temp_dir().join(format!("eve-pin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("a").join("b");
        std::fs::create_dir_all(&dir).unwrap();
        let h = open_dir(&dir, false).expect("開ける");
        assert!(
            std::fs::rename(&dir, base.join("a").join("b2")).is_err(),
            "固定したディレクトリ"
        );
        assert!(
            std::fs::rename(base.join("a"), base.join("a2")).is_err(),
            "祖先"
        );
        // 中のファイルは作れる (共有の書込を許す)
        std::fs::write(dir.join("x"), b"1").unwrap();
        drop(h);
        std::fs::rename(base.join("a"), base.join("a2")).expect("閉じた後は改名できる");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn 自分で作ったディレクトリの所有者と_dacl() {
        let base = std::env::temp_dir().join(format!("eve-sec-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let h = open_dir(&base, false).unwrap();
        let s = security(&h).unwrap();
        let t = trusted().unwrap();
        assert!(t.owners.contains(&s.owner), "{}", account_name(&s.owner));
        assert!(s.dacl.is_some());
        let (attrs, _) = attr_tag(&h).unwrap();
        assert_ne!(attrs & 0x10, 0, "ディレクトリ");
        assert_ne!(final_path(&h).unwrap(), Vec::<u16>::new());
        drop(h);
        let _ = std::fs::remove_dir_all(&base);
    }
}
