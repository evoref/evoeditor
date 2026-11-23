//! `FileSystem::read_checked` (docs/02 §4.5): ワークスペース設定と `.editorconfig` を、所有者の検査を通して読む
//! (07 §12.1 の OWN-1〜OWN-7、05 §5.7)。
//!
//! 順: `dir` を開いて固定する → `name` の有無を開かずに確かめる (無ければ `Missing`。`C:\`・`/tmp` のような他人が書ける
//! ディレクトリに無いだけで診断を出さない) → `dir` を判定する → `name` をリンクを辿らずに開く → ふつうのファイルかと、
//! 判定した物と同じ所かを確かめる → ファイルを判定する → 同じハンドルから `max_len + 1` バイトまで読む。
//! 判定は「判定できないときは信頼しない」(問い合わせの失敗・読めない ACE・知らない FS の写像は読まない側) に倒す。
//! OS の API は `sys::windows::owner` / `sys::unix::owner` の安全な関数で、ここは unsafe を持たない。
//!
//! spec: 02 §4.5, 07 §12.1 (OWN-1〜OWN-7), 05 §5.7

use std::ffi::OsStr;
use std::fs::File;
use std::io::Read;

use eve_core::{
    CheckedObject, CheckedRead, FsError, FsErrorKind, FsPath, OwnerCheck, OwnerVerdict,
};

/// 問い合わせが失敗したときの `OwnerVerdict::Unknown` の `fs` (02 §4.5)。
const QUERY_FAILED: &str = "query_failed";

fn unknown(fs: &str) -> OwnerVerdict {
    OwnerVerdict::Unknown { fs: fs.to_owned() }
}

fn rejected(what: CheckedObject, verdict: OwnerVerdict) -> CheckedRead {
    CheckedRead::Rejected { what, verdict }
}

/// `name` が 1 つの名前か (空・`.`・`..`・区切り・NUL を含まない。Windows は `:` (代替のデータストリーム) も含まない)。
fn valid_name(name: &OsStr) -> bool {
    let Some(s) = name.to_str() else {
        // UTF-8 でない名前は区切りの判定ができないので受け付けない (呼ぶのは決まった名前だけ)
        return false;
    };
    let bad = |c: char| c == '/' || c == '\0' || (cfg!(windows) && (c == '\\' || c == ':'));
    !s.is_empty() && s != "." && s != ".." && !s.contains(bad)
}

/// `dir` の最後の要素が `.eve` か (07 §12.1 の OWN-6: `.eve` はリンクを辿らずに開く。Windows は大文字小文字を区別しない)。
fn is_eve(dir: &FsPath) -> bool {
    dir.as_path().file_name().is_some_and(|n| {
        if cfg!(windows) {
            n.eq_ignore_ascii_case(".eve")
        } else {
            n == ".eve"
        }
    })
}

/// `f` から `max_len + 1` バイトまで読む。`max_len` を超えたら `TooLarge` (読んだ分は捨てる)。
fn read_limited(f: File, max_len: u64, path: &FsPath) -> Result<CheckedRead, FsError> {
    let mut bytes = Vec::new();
    f.take(max_len.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| crate::sys::map_io_error(e, path))?;
    if bytes.len() as u64 > max_len {
        return Ok(CheckedRead::TooLarge);
    }
    Ok(CheckedRead::Read { bytes })
}

// ---------- Windows ----------

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::fs::File;

    use eve_core::{CheckedObject, CheckedRead, FsError, FsPath, OwnerCheck, OwnerVerdict};

    use super::{QUERY_FAILED, is_eve, read_limited, rejected, unknown};
    use crate::sys::owner::{self, Ace, Security, Sid, Trusted};

    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    /// FS の旗 `FILE_PERSISTENT_ACLS` (winnt.h。windows-sys では `Win32_System_SystemServices` にあり、feature を足さない)。
    const FILE_PERSISTENT_ACLS: u32 = 0x8;
    /// 書込の権利 (07 §12.1 の OWN-2 の (2)): `FILE_ADD_FILE` / `FILE_WRITE_DATA` (0x2)・`FILE_ADD_SUBDIRECTORY` /
    /// `FILE_APPEND_DATA` (0x4)・`FILE_DELETE_CHILD` (0x40)・`DELETE`・`WRITE_DAC`・`WRITE_OWNER`・`GENERIC_ALL`・`GENERIC_WRITE`。
    pub(super) const WRITE_MASK: u32 = 0x500D_0046;

    /// 再解析のタグが名前の代理 (シンボリックリンク・ジャンクションなど。`IsReparseTagNameSurrogate`) か。
    fn name_surrogate(tag: u32) -> bool {
        tag & 0x2000_0000 != 0
    }

    /// 書込を許された「自分たち」以外の相手。
    #[derive(Debug, PartialEq, Eq)]
    pub(super) enum Writer {
        /// その SID。
        Sid(Sid),
        /// NULL の DACL (全員)。
        Everyone,
        /// 読めない型の ACE。
        UnknownAce(u8),
    }

    /// DACL から、書込の権利を「自分たち」以外に与える相手を集める (07 §12.1 の OWN-2 の (2))。継承専用の ACE は
    /// この物に効かないので見ない。拒否の ACE は数えない (書込を打ち消すかを見ないので、読まない側に倒れる)。
    pub(super) fn others_writers(dacl: Option<&[Ace]>, trusted: &[Sid]) -> Vec<Writer> {
        let Some(aces) = dacl else {
            return vec![Writer::Everyone];
        };
        let mut out = Vec::new();
        for ace in aces {
            match ace {
                Ace::Allowed {
                    inherit_only: false,
                    mask,
                    sid,
                } if mask & WRITE_MASK != 0 && !trusted.contains(sid) => {
                    let w = Writer::Sid(sid.clone());
                    if !out.contains(&w) {
                        out.push(w);
                    }
                }
                Ace::Allowed { .. } | Ace::Denied => {}
                Ace::Other { ace_type } => out.push(Writer::UnknownAce(*ace_type)),
            }
        }
        out
    }

    /// 所有者と DACL の判定 (OWN-2 の (1)(2))。通れば `None`。
    pub(super) fn judge_security(
        sec: &Security,
        trusted: &Trusted,
        name: impl Fn(&Sid) -> String,
    ) -> Option<OwnerVerdict> {
        if !trusted.owners.contains(&sec.owner) {
            return Some(OwnerVerdict::NotOwner {
                owner: name(&sec.owner),
            });
        }
        let writers = others_writers(sec.dacl.as_deref(), &trusted.writers);
        if writers.is_empty() {
            return None;
        }
        let who = writers
            .iter()
            .map(|w| match w {
                Writer::Sid(s) => name(s),
                Writer::Everyone => "Everyone".to_owned(),
                Writer::UnknownAce(t) => format!("ACE type {t:#04x}"),
            })
            .collect();
        Some(OwnerVerdict::OthersCanWrite { who })
    }

    /// 開いたハンドルの判定 (OWN-2 / OWN-3)。通れば `None`。
    fn judge(h: &File) -> Option<OwnerVerdict> {
        let Some((fs, flags)) = crate::sys::volume_info(h) else {
            return Some(unknown(QUERY_FAILED));
        };
        // FAT32・exFAT (SMB の向こうがそうでも分かる)
        if flags & FILE_PERSISTENT_ACLS == 0 {
            return Some(unknown(&fs));
        }
        let Some(trusted) = owner::trusted() else {
            return Some(unknown(QUERY_FAILED));
        };
        // 9P の `\\wsl.localhost\…` は ERROR_INVALID_FUNCTION、アクセス拒否も判定できない
        let Ok(sec) = owner::security(h) else {
            return Some(unknown(QUERY_FAILED));
        };
        judge_security(&sec, trusted, owner::account_name)
    }

    /// `p` の親 (最後の `\` より前)。
    pub(super) fn parent(p: &[u16]) -> &[u16] {
        let bs = u16::from(b'\\');
        p.iter()
            .rposition(|&c| c == bs)
            .map_or(&[][..], |i| &p[..i])
    }

    /// 開いたファイルの最終のパス `file` の親が、固定したディレクトリの最終のパス `dir` と同じか。ボリュームの直下の
    /// ディレクトリの最終のパスは `\` で終わる (`\Device\HarddiskVolume3\`) ので、末尾の `\` を除いて比べる。
    pub(super) fn same_parent(file: &[u16], dir: &[u16]) -> bool {
        let bs = u16::from(b'\\');
        let dir = dir.strip_suffix(&[bs]).unwrap_or(dir);
        !dir.is_empty() && parent(file) == dir
    }

    pub(super) fn read_checked(
        dir: &FsPath,
        name: &OsStr,
        max_len: u64,
        check: OwnerCheck,
    ) -> Result<CheckedRead, FsError> {
        let map = |e, p: &FsPath| crate::sys::map_io_error(e, p);
        let no_follow = is_eve(dir);
        // ディレクトリを固定する (FILE_SHARE_DELETE なし。読み終えるまで `dh` を持つ)
        let dh = match owner::open_dir(dir.as_path(), no_follow) {
            Ok(h) => h,
            // ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND
            Err(e) if matches!(e.raw_os_error(), Some(2 | 3)) => return Ok(CheckedRead::Missing),
            Err(e) => return Err(map(e, dir)),
        };
        let (dattrs, dtag) = owner::attr_tag(&dh).map_err(|e| map(e, dir))?;
        if no_follow && dattrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 && name_surrogate(dtag) {
            return Ok(rejected(CheckedObject::Dir, OwnerVerdict::NotRegular));
        }
        // ディレクトリでない (`.eve` がファイル) なら、その中のファイルは無い
        if dattrs & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Ok(CheckedRead::Missing);
        }
        let path = dir.join(name);
        // 有無を開かずに確かめる (属性の問い合わせ。最後の要素のリンクは辿らない)。問い合わせの失敗は「在る」として進め、
        // 開くときの結果に任せる
        let attrs = match owner::path_attributes(path.as_path()) {
            Ok(None) => return Ok(CheckedRead::Missing),
            Ok(Some(a)) => Some(a),
            Err(_) => None,
        };
        if check == OwnerCheck::Check
            && let Some(v) = judge(&dh)
        {
            return Ok(rejected(CheckedObject::Dir, v));
        }
        if attrs.is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY != 0) {
            return Ok(rejected(CheckedObject::File, OwnerVerdict::NotRegular));
        }
        // リンクを辿らずに開く (リンクの先の UNC へ認証が走らない)
        let fh = match owner::open_file(path.as_path(), true) {
            Ok(h) => h,
            Err(e) if matches!(e.raw_os_error(), Some(2 | 3)) => return Ok(CheckedRead::Missing),
            Err(e) => return Err(map(e, &path)),
        };
        let (fattrs, ftag) = owner::attr_tag(&fh).map_err(|e| map(e, &path))?;
        let reparse = fattrs & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        if fattrs & FILE_ATTRIBUTE_DIRECTORY != 0
            || (reparse && name_surrogate(ftag))
            || crate::sys::is_char_or_pipe(&fh)
        {
            return Ok(rejected(CheckedObject::File, OwnerVerdict::NotRegular));
        }
        // 開いたファイルの親が、固定したディレクトリと同じ所か (途中のジャンクションの行き先を差し替えられても、判定した物と
        // 違う所を読まない)
        match (owner::final_path(&dh), owner::final_path(&fh)) {
            (Ok(d), Ok(f)) if same_parent(&f, &d) => {}
            _ => return Ok(rejected(CheckedObject::File, unknown(QUERY_FAILED))),
        }
        if check == OwnerCheck::Check
            && let Some(v) = judge(&fh)
        {
            return Ok(rejected(CheckedObject::File, v));
        }
        // 名前の代理でない再解析点 (OneDrive のクラウドのファイル・重複除去・WOF の圧縮) は、再解析点そのものの中身ではなく
        // フィルタを通した中身を読むために開き直す。開き直した物が判定した物と同じファイルかを識別で確かめる
        // (ディレクトリは判定を通っているので、他人はその中の名前を差し替えられない)
        let reader = if reparse {
            let again = owner::open_file(path.as_path(), false).map_err(|e| map(e, &path))?;
            let same = match (crate::sys::file_info(&fh), crate::sys::file_info(&again)) {
                (Ok(a), Ok(b)) => a.volume == b.volume && a.index == b.index,
                _ => false,
            };
            if !same {
                return Ok(rejected(CheckedObject::File, unknown(QUERY_FAILED)));
            }
            again
        } else {
            fh
        };
        let out = read_limited(reader, max_len, &path);
        drop(dh);
        out
    }
}

// ---------- Unix ----------

#[cfg(unix)]
mod imp {
    use std::ffi::OsStr;
    use std::fs::File;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    use eve_core::{CheckedObject, CheckedRead, FsError, FsPath, OwnerCheck, OwnerVerdict};

    use super::{QUERY_FAILED, is_eve, read_limited, rejected, unknown};
    use crate::sys::owner;

    /// `st_mode` の種類の部分 (`S_IFMT`) と、ふつうのファイルの種類 (`S_IFREG`)。POSIX の値。
    const S_IFMT: u32 = 0o170_000;
    const S_IFREG: u32 = 0o100_000;
    /// 他人の書込 (`o+w`)。
    const S_IWOTH: u32 = 0o002;

    /// 所有者と mode の判定 (07 §12.1 の OWN-2 / OWN-4)。「自分」は実効 uid と uid 0 だけ (root で動かしたときは uid 0 だけ。
    /// `FileMeta.owner_is_me` の「root はすべて自分」は使わない。`SUDO_UID` は見ない)。`o+w` は sticky でも読まない。
    /// `g+w` は見ない。
    pub(super) fn judge_stat(uid: u32, mode: u32, euid: u32) -> Option<StatVerdict> {
        if uid != euid && uid != 0 {
            return Some(StatVerdict::NotOwner);
        }
        if mode & S_IWOTH != 0 {
            return Some(StatVerdict::OthersWrite);
        }
        None
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(super) enum StatVerdict {
        NotOwner,
        OthersWrite,
    }

    /// 判定できない FS か (OWN-3)。`fstatfs` の失敗も判定できない。
    #[cfg(target_os = "linux")]
    fn unknown_fs(f: &File) -> Option<OwnerVerdict> {
        match owner::fs_type(f) {
            Ok(t) => crate::sys::owner_unknown_fs(t).map(unknown),
            Err(_) => Some(unknown(QUERY_FAILED)),
        }
    }

    /// Linux 以外の Unix は、所有者を写す FS を見分けていない (macOS へ移すときに `f_fstypename` で入れる) ので判定できない。
    #[cfg(not(target_os = "linux"))]
    fn unknown_fs(_: &File) -> Option<OwnerVerdict> {
        Some(unknown("unsupported"))
    }

    /// 開いた物の判定 (OWN-2 / OWN-3)。通れば `None`。
    fn judge(f: &File) -> Option<OwnerVerdict> {
        // FS を先に見る (DrvFs は所有者も mode も写像で、先に mode を見ると理由を誤る。S-25)
        if let Some(v) = unknown_fs(f) {
            return Some(v);
        }
        let Ok(m) = f.metadata() else {
            return Some(unknown(QUERY_FAILED));
        };
        match judge_stat(m.uid(), m.mode(), crate::sys::euid()) {
            Some(StatVerdict::NotOwner) => {
                return Some(OwnerVerdict::NotOwner {
                    owner: owner::user_name(m.uid()).unwrap_or_else(|| m.uid().to_string()),
                });
            }
            Some(StatVerdict::OthersWrite) => {
                return Some(OwnerVerdict::OthersCanWrite {
                    who: vec!["others".to_owned()],
                });
            }
            None => {}
        }
        // 拡張 ACL (`setfacl -m u:<他人>:w` は `st_mode` に `o+w` が出ない)。中身を解析せず、あれば書けるとみなす
        match crate::sys::get_xattr(f, b"system.posix_acl_access") {
            Ok(_) => Some(OwnerVerdict::OthersCanWrite {
                who: vec!["acl".to_owned()],
            }),
            Err(e) if matches!(e.raw_os_error(), Some(libc::ENODATA | libc::ENOTSUP)) => None,
            Err(_) => Some(unknown(QUERY_FAILED)),
        }
    }

    pub(super) fn read_checked(
        dir: &FsPath,
        name: &OsStr,
        max_len: u64,
        check: OwnerCheck,
    ) -> Result<CheckedRead, FsError> {
        let map = |e, p: &FsPath| crate::sys::map_io_error(e, p);
        let no_follow = is_eve(dir);
        let flags = libc::O_DIRECTORY | if no_follow { libc::O_NOFOLLOW } else { 0 };
        let dh = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(flags)
            .open(dir.as_path())
        {
            Ok(h) => h,
            Err(e) => {
                return match e.raw_os_error() {
                    // `.eve` がシンボリックリンク (Linux は `O_DIRECTORY | O_NOFOLLOW` で `ENOTDIR`、ほかは `ELOOP` を返す)。
                    // 何であったかを知るためだけに `lstat` する (読まない側の理由を選ぶだけで、辿らない)
                    Some(libc::ENOTDIR | libc::ELOOP)
                        if no_follow
                            && std::fs::symlink_metadata(dir.as_path())
                                .is_ok_and(|m| m.file_type().is_symlink()) =>
                    {
                        Ok(rejected(CheckedObject::Dir, OwnerVerdict::NotRegular))
                    }
                    Some(libc::ENOENT | libc::ENOTDIR) => Ok(CheckedRead::Missing),
                    _ => Err(map(e, dir)),
                };
            }
        };
        let path = dir.join(name);
        let Some(mode) = owner::mode_at(&dh, name).map_err(|e| map(e, &path))? else {
            return Ok(CheckedRead::Missing);
        };
        if check == OwnerCheck::Check
            && let Some(v) = judge(&dh)
        {
            return Ok(rejected(CheckedObject::Dir, v));
        }
        // シンボリックリンク・ディレクトリ・FIFO・デバイスは開かない (デバイスを開くだけで起きることを避ける)
        if mode & S_IFMT != S_IFREG {
            return Ok(rejected(CheckedObject::File, OwnerVerdict::NotRegular));
        }
        let fh = match owner::open_at_nofollow(&dh, name) {
            Ok(h) => h,
            Err(e) => {
                return match e.raw_os_error() {
                    Some(libc::ENOENT) => Ok(CheckedRead::Missing),
                    Some(libc::ELOOP) => {
                        Ok(rejected(CheckedObject::File, OwnerVerdict::NotRegular))
                    }
                    _ => Err(map(e, &path)),
                };
            }
        };
        // 有無を確かめた後に差し替えられた物を読まない
        match fh.metadata() {
            Ok(m) if m.file_type().is_file() => {}
            Ok(_) => return Ok(rejected(CheckedObject::File, OwnerVerdict::NotRegular)),
            Err(e) => return Err(map(e, &path)),
        }
        if check == OwnerCheck::Check
            && let Some(v) = judge(&fh)
        {
            return Ok(rejected(CheckedObject::File, v));
        }
        let out = read_limited(fh, max_len, &path);
        drop(dh);
        out
    }
}

/// `FileSystem::read_checked` の実装 (`WinFs` と `UnixFs` が呼ぶ)。
pub(crate) fn read_checked(
    dir: &FsPath,
    name: &OsStr,
    max_len: u64,
    check: OwnerCheck,
) -> Result<CheckedRead, FsError> {
    if !valid_name(name) {
        return Err(FsError::new(FsErrorKind::InvalidPath));
    }
    imp::read_checked(dir, name, max_len, check)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 名前は_1_つの要素だけ() {
        for ok in [".editorconfig", "config.toml", "keymap.toml"] {
            assert!(valid_name(OsStr::new(ok)), "{ok}");
        }
        for bad in ["", ".", "..", "a/b", "a\0b"] {
            assert!(!valid_name(OsStr::new(bad)), "{bad:?}");
        }
        assert_eq!(valid_name(OsStr::new(r"a\b")), !cfg!(windows));
        assert_eq!(valid_name(OsStr::new("a:s")), !cfg!(windows));
    }

    #[test]
    fn eve_の判定は最後の要素() {
        assert!(is_eve(&FsPath::from("/w/.eve")));
        assert!(!is_eve(&FsPath::from("/w/.eve/x")));
        assert!(!is_eve(&FsPath::from("/w/eve")));
        assert_eq!(is_eve(&FsPath::from("/w/.EVE")), cfg!(windows));
    }

    #[cfg(windows)]
    mod windows {
        use super::super::imp::{Writer, judge_security, others_writers};
        use crate::sys::owner::{Ace, Security, Sid, Trusted};
        use eve_core::OwnerVerdict;

        fn sid(n: u8) -> Sid {
            // 版 1・副権限 1 つの形 (S-1-5-n)。比べるだけなので形が正しければよい
            Sid::from_bytes_for_test(&[1, 1, 0, 0, 0, 0, 0, 5, n, 0, 0, 0])
        }

        fn trusted() -> Trusted {
            Trusted {
                owners: vec![sid(1), sid(18)],
                writers: vec![sid(1), sid(18), sid(99)],
            }
        }

        fn allow(mask: u32, who: u8) -> Ace {
            Ace::Allowed {
                inherit_only: false,
                mask,
                sid: sid(who),
            }
        }

        #[test]
        fn 書込の権利を自分たち以外に与える_ace_を数える() {
            let t = trusted();
            // Modify (0x1301bf)・Write (0x100116)・AD だけ (0x4)・GENERIC_WRITE・WRITE_DAC は書込
            for mask in [0x0013_01BF, 0x0010_0116, 0x4, 0x4000_0000, 0x0004_0000] {
                assert_eq!(
                    others_writers(Some(&[allow(mask, 11)]), &t.writers),
                    vec![Writer::Sid(sid(11))],
                    "{mask:#x}"
                );
            }
            // 読み取りと実行 (0x1200a9) は書込でない
            assert_eq!(
                others_writers(Some(&[allow(0x0012_00A9, 11)]), &t.writers),
                Vec::<Writer>::new()
            );
            // 自分たち・CREATOR OWNER の類は数えない
            assert_eq!(
                others_writers(
                    Some(&[allow(0x1F_01FF, 18), allow(0x1F_01FF, 99)]),
                    &t.writers
                ),
                Vec::<Writer>::new()
            );
            // 継承専用の ACE はこの物に効かない
            let io = Ace::Allowed {
                inherit_only: true,
                mask: 0x0013_01BF,
                sid: sid(11),
            };
            assert_eq!(
                others_writers(Some(&[io]), &t.writers),
                Vec::<Writer>::new()
            );
            // 拒否の ACE は数えない (打ち消すかを見ないので、許可があれば書けるとみなす)
            assert_eq!(
                others_writers(Some(&[Ace::Denied, allow(0x2, 11)]), &t.writers),
                vec![Writer::Sid(sid(11))]
            );
            assert_eq!(
                others_writers(Some(&[Ace::Denied]), &t.writers),
                Vec::<Writer>::new()
            );
            // NULL の DACL・読めない型の ACE は書けるとみなす
            assert_eq!(others_writers(None, &t.writers), vec![Writer::Everyone]);
            assert_eq!(
                others_writers(Some(&[Ace::Other { ace_type: 5 }]), &t.writers),
                vec![Writer::UnknownAce(5)]
            );
            // 同じ相手は 1 回
            assert_eq!(
                others_writers(Some(&[allow(0x2, 11), allow(0x4, 11)]), &t.writers).len(),
                1
            );
        }

        #[test]
        fn 開いたファイルの親と固定したディレクトリを比べる() {
            use super::super::imp::same_parent;
            let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
            let v = r"\Device\HarddiskVolume3";
            assert!(same_parent(
                &w(&format!(r"{v}\w\.eve\config.toml")),
                &w(&format!(r"{v}\w\.eve"))
            ));
            // ボリュームの直下 (ディレクトリの最終のパスが `\` で終わる)
            assert!(same_parent(
                &w(&format!(r"{v}\.editorconfig")),
                &w(&format!(r"{v}\"))
            ));
            // 違う所・名前の前方だけの一致は違う
            assert!(!same_parent(
                &w(&format!(r"{v}\x\config.toml")),
                &w(&format!(r"{v}\w"))
            ));
            assert!(!same_parent(
                &w(&format!(r"{v}\wx\config.toml")),
                &w(&format!(r"{v}\w"))
            ));
            assert!(!same_parent(
                &w(&format!(r"{v}\w\sub\config.toml")),
                &w(&format!(r"{v}\w"))
            ));
            assert!(!same_parent(&w(r"\config.toml"), &w(r"\")));
        }

        #[test]
        fn 所有者を先に見る() {
            let t = trusted();
            let name = |s: &Sid| format!("{s:?}");
            let sec = |owner, dacl| Security { owner, dacl };
            assert_eq!(judge_security(&sec(sid(1), Some(vec![])), &t, name), None);
            assert_eq!(
                judge_security(&sec(sid(18), Some(vec![])), &t, name),
                None,
                "SYSTEM"
            );
            assert!(matches!(
                judge_security(&sec(sid(11), None), &t, name),
                Some(OwnerVerdict::NotOwner { .. })
            ));
            // CREATOR OWNER の類は所有者としては受け入れない
            assert!(matches!(
                judge_security(&sec(sid(99), Some(vec![])), &t, name),
                Some(OwnerVerdict::NotOwner { .. })
            ));
            assert_eq!(
                judge_security(&sec(sid(1), None), &t, name),
                Some(OwnerVerdict::OthersCanWrite {
                    who: vec!["Everyone".to_owned()]
                })
            );
            assert_eq!(
                judge_security(
                    &sec(sid(1), Some(vec![Ace::Other { ace_type: 0xB }])),
                    &t,
                    name
                ),
                Some(OwnerVerdict::OthersCanWrite {
                    who: vec!["ACE type 0x0b".to_owned()]
                })
            );
        }
    }

    #[cfg(unix)]
    mod unix {
        use super::super::imp::{StatVerdict, judge_stat};

        #[test]
        fn 所有者と_mode_の判定() {
            // 自分 (uid 1000) で動く
            assert_eq!(judge_stat(1000, 0o40755, 1000), None);
            assert_eq!(judge_stat(1000, 0o40775, 1000), None, "g+w は見ない");
            assert_eq!(
                judge_stat(0, 0o40755, 1000),
                None,
                "root の所有は受け入れる"
            );
            assert_eq!(judge_stat(1001, 0o40755, 1000), Some(StatVerdict::NotOwner));
            assert_eq!(
                judge_stat(1000, 0o40777, 1000),
                Some(StatVerdict::OthersWrite)
            );
            assert_eq!(
                judge_stat(1000, 0o41777, 1000),
                Some(StatVerdict::OthersWrite),
                "sticky でも"
            );
            assert_eq!(
                judge_stat(0, 0o41777, 1000),
                Some(StatVerdict::OthersWrite),
                "/tmp"
            );
            // root で動かしたとき (OWN-4): 「自分」は uid 0 だけ
            assert_eq!(judge_stat(0, 0o40755, 0), None);
            assert_eq!(judge_stat(1000, 0o40755, 0), Some(StatVerdict::NotOwner));
            // 所有者を先に見る
            assert_eq!(judge_stat(1001, 0o40777, 1000), Some(StatVerdict::NotOwner));
        }
    }
}
