//! `FileSystem::read_checked` (docs/02 §4.5): ワークスペース設定と `.editorconfig` を、所有者の検査を通して読む
//! (07 §12.1 の OWN-1〜OWN-7、05 §5.7)。
//!
//! 順: `dir` を開いて固定する (`DirLinks::NoFollowFromEve` なら `.eve` から先の要素はリンクを辿らずに 1 つずつ開く) →
//! `name` の有無を開かずに確かめる (無ければ `Missing`。`C:\`・`/tmp` のような他人が書けるディレクトリに無いだけで診断を
//! 出さない) → 開いた `dir` の最終のパスが `workspace.safe_directories` の下かを決める (OWN-5) → 下でなければ `dir` を判定する
//! → `name` をリンクを辿らずに開く → ふつうのファイルかと、判定した物と同じ所かを確かめる → 下でなければファイルを判定する →
//! 同じハンドルから `max_len + 1` バイトまで読む。
//! 判定は「判定できないときは信頼しない」(問い合わせの失敗・読めない ACE・知らない再解析点・所有者を写す FS は読まない側) に倒す。
//! OS の API は `sys::windows::owner` / `sys::unix::owner` の安全な関数で、ここは unsafe を持たない。
//!
//! spec: 02 §4.5, 07 §12.1 (OWN-1〜OWN-7), 05 §5.7

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use eve_core::{
    CheckedObject, CheckedRead, DirLinks, FsError, FsErrorKind, FsPath, OwnerVerdict,
    WORKSPACE_CONFIG_DIR,
};

/// 問い合わせが失敗したとき・開いた物が判定した場所と違うときの `OwnerVerdict::Unknown` の `fs` (02 §4.5)。
const QUERY_FAILED: &str = "query_failed";

/// 判定と開く間の呼び出し点 (`hooks::point`。単体試験が差し替えを起こす)。
const BEFORE_OPEN: &str = "read_checked_before_open";

fn unknown(fs: &str) -> OwnerVerdict {
    OwnerVerdict::Unknown { fs: fs.to_owned() }
}

fn rejected(what: CheckedObject, verdict: OwnerVerdict) -> CheckedRead {
    CheckedRead::Rejected { what, verdict }
}

/// Windows で、Win32 のパスの正規化が落とす・別の意味に読む形の要素か (末尾の `.`・空白、代替のデータストリームの `:`)。
/// `.eve.` が `.eve` を開き、`.eve` の見分けを外せる (PO の決定 2026-11-23 の A)。
fn bad_win_component(s: &str) -> bool {
    s.ends_with('.') || s.ends_with(' ') || s.contains(':')
}

/// `name` が 1 つの名前か (空・`.`・`..`・区切り・NUL を含まない。Windows は `\`・`:`・末尾の `.` と空白も受け付けない)。
fn valid_name(name: &OsStr) -> bool {
    let Some(s) = name.to_str() else {
        // UTF-8 でない名前は区切りの判定ができないので受け付けない (呼ぶのは決まった名前だけ)
        return false;
    };
    let bad = |c: char| c == '/' || c == '\0' || (cfg!(windows) && c == '\\');
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains(bad)
        && !(cfg!(windows) && bad_win_component(s))
}

/// 要素が `.eve` か (全 OS で大文字小文字を区別しない)。
fn is_eve(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|s| s.eq_ignore_ascii_case(WORKSPACE_CONFIG_DIR))
}

/// `dir` を、リンクを辿って開く前半と、辿らずに 1 つずつ開く後半 (`.eve` から) に分ける。`..` を含む・Windows で
/// 末尾が `.`・空白か `:` を含む要素がある・`NoFollowFromEve` で `.eve` が無い、なら `None` (`InvalidPath`)。
fn split_dir(dir: &Path, links: DirLinks) -> Option<(PathBuf, Vec<OsString>)> {
    let comps: Vec<Component<'_>> = dir.components().collect();
    for c in &comps {
        match c {
            Component::ParentDir => return None,
            Component::Normal(n) if cfg!(windows) && bad_win_component(&n.to_string_lossy()) => {
                return None;
            }
            _ => {}
        }
    }
    match links {
        DirLinks::Follow => Some((dir.to_path_buf(), Vec::new())),
        DirLinks::NoFollowFromEve => {
            let i = comps
                .iter()
                .rposition(|c| matches!(c, Component::Normal(n) if is_eve(n)))?;
            let base: PathBuf = comps[..i].iter().collect();
            let chain = comps[i..]
                .iter()
                .map(|c| c.as_os_str().to_os_string())
                .collect();
            Some((base, chain))
        }
    }
}

/// 比べるための要素の列 (Windows は `\\?\` と `\\?\UNC\` を外し、`/` を `\` にしてから分ける。UTF-16 として正しくない
/// パスは比べない)。大文字小文字は畳まない (比べるときに ASCII だけを畳む、`same_component`)。
fn comparable(p: &Path) -> Option<Vec<String>> {
    let s = p.to_str()?;
    let normalized = if cfg!(windows) {
        let s = s.replace('/', "\\");
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else {
            s.strip_prefix(r"\\?\").unwrap_or(&s).to_owned()
        }
    } else {
        s.to_owned()
    };
    Path::new(&normalized)
        .components()
        .map(|c| c.as_os_str().to_str().map(str::to_owned))
        .collect()
}

/// 要素が同じか。Windows は ASCII の大文字小文字だけを区別しない (NTFS は Unicode の大文字小文字も畳むが、Rust の
/// `to_lowercase` とは表が違い、KELVIN SIGN (U+212A) と `k` のように NTFS が別の名前とみなすものを同じにしてしまう。
/// ASCII の外の違いは「違う」とみなし、検査を省かない側に倒す)。
fn same_component(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// UNC の形のパスか (Windows。`\\server\share\…`・`\\?\UNC\…`。`\\?\C:\…`・`\\.\…` は違う)。
fn is_unc(p: &Path) -> bool {
    let s = p.to_string_lossy();
    cfg!(windows)
        && (s.starts_with(r"\\?\UNC\")
            || (s.starts_with(r"\\") && !s.starts_with(r"\\?\") && !s.starts_with(r"\\.\")))
}

/// 安全な場所の解いた形を覚える数の上限 (プロセスで)。設定に書く数より十分に大きい。
const RESOLVED_LIMIT: usize = 256;

/// 安全な場所を `canonicalize` した形 (リンク・ドライブの割当て・短い名前を解いた形。できなければ書いた形)。書いた形ごとに
/// プロセスで覚える (失敗も)。応答しない共有を安全な場所に書いても、待つのは最初の 1 回だけにする。
fn resolved(s: &Path) -> PathBuf {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(r) = cache
        .lock()
        .expect("安全な場所の表の Mutex が壊れた")
        .get(s)
    {
        return r.clone();
    }
    // 解く間は表を握らない (ほかのジョブを止めない)
    let r = std::fs::canonicalize(s).unwrap_or_else(|_| s.to_path_buf());
    let mut c = cache.lock().expect("安全な場所の表の Mutex が壊れた");
    if c.len() < RESOLVED_LIMIT {
        c.insert(s.to_path_buf(), r.clone());
    }
    r
}

/// 開いたディレクトリの最終のパス `final_path` が、安全な場所 (`workspace.safe_directories`) のどれかか、その下か (07 §12.1 の
/// OWN-5)。安全な場所は解いた形 (`resolved`) と要素ごとに比べる (Windows は ASCII の大文字小文字だけを区別しない)。
/// 相対のパスは比べない。最終のパスが UNC でなければ、UNC の形で書いた安全な場所は解かずに飛ばす (その共有に問い合わせない)。
fn under_safe(final_path: &Path, safe_dirs: &[FsPath]) -> bool {
    let Some(fin) = comparable(final_path) else {
        return false;
    };
    let fin_unc = is_unc(final_path);
    safe_dirs.iter().any(|s| {
        let s = s.as_path();
        if !s.is_absolute() || (!fin_unc && is_unc(s)) {
            return false;
        }
        comparable(&resolved(s)).is_some_and(|c| {
            c.len() <= fin.len() && c.iter().zip(&fin).all(|(a, b)| same_component(a, b))
        })
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
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::path::PathBuf;

    use eve_core::{CheckedObject, CheckedRead, FsError, FsPath, OwnerVerdict};

    use super::{BEFORE_OPEN, QUERY_FAILED, read_limited, rejected, under_safe, unknown};
    use crate::sys::owner::{self, Ace, Security, Sid, Trusted};

    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    /// FS の旗 `FILE_PERSISTENT_ACLS` (winnt.h。windows-sys では `Win32_System_SystemServices` にあり、feature を足さない)。
    const FILE_PERSISTENT_ACLS: u32 = 0x8;
    /// 書込の権利 (07 §12.1 の OWN-2 の (2)): `FILE_ADD_FILE` / `FILE_WRITE_DATA` (0x2)・`FILE_ADD_SUBDIRECTORY` /
    /// `FILE_APPEND_DATA` (0x4)・`FILE_WRITE_EA` (0x10)・`FILE_DELETE_CHILD` (0x40)・`FILE_WRITE_ATTRIBUTES` (0x100)・
    /// `DELETE`・`WRITE_DAC`・`WRITE_OWNER`・`GENERIC_ALL`・`GENERIC_WRITE`。
    pub(super) const WRITE_MASK: u32 = 0x500D_0156;

    /// 再解析のタグが名前の代理 (シンボリックリンク・ジャンクションなど。`IsReparseTagNameSurrogate`) か。
    fn name_surrogate(tag: u32) -> bool {
        tag & 0x2000_0000 != 0
    }

    /// 名前の代理でない再解析点のうち、判定して中身を読むもの (PO の決定 2026-11-23 の B。許可リスト): WOF
    /// (`IO_REPARSE_TAG_WOF`)・重複除去 (`IO_REPARSE_TAG_DEDUP`)・Cloud Files の一族 (`IO_REPARSE_TAG_CLOUD_*`。
    /// `tag & 0xFFFF0FFF == 0x9000001A`) と古い `IO_REPARSE_TAG_ONEDRIVE`。ほか (`ProjFS`・`STORAGE_SYNC`・HSM・DFS・WCI・
    /// `AF_UNIX`・`LX_*`・App Execution Alias・知らないタグ) は読まない。
    pub(super) fn readable_reparse(tag: u32) -> bool {
        const WOF: u32 = 0x8000_0017;
        const DEDUP: u32 = 0x8000_0013;
        const ONEDRIVE: u32 = 0x8000_0021;
        const CLOUD: u32 = 0x9000_001A;
        const CLOUD_MASK: u32 = 0xFFFF_0FFF;
        !name_surrogate(tag)
            && (tag == WOF || tag == DEDUP || tag == ONEDRIVE || tag & CLOUD_MASK == CLOUD)
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

    /// ボリュームで判定できるか (OWN-3。PO の決定 2026-11-23 の C)。遠くのボリューム・NTFS / `ReFS` でない FS・
    /// `FILE_PERSISTENT_ACLS` の無い FS は判定できない。判定できれば `None`。
    pub(super) fn volume_verdict(fs: &str, flags: u32, remote: bool) -> Option<OwnerVerdict> {
        if remote {
            return Some(unknown("remote"));
        }
        if !(fs.eq_ignore_ascii_case("NTFS") || fs.eq_ignore_ascii_case("ReFS"))
            || flags & FILE_PERSISTENT_ACLS == 0
        {
            return Some(unknown(fs));
        }
        None
    }

    /// NT の形の最終のパスが、ネットワークの経路 (Multiple UNC Provider、`\Device\Mup\`) の下か (ASCII の大文字小文字を
    /// 区別しない)。
    pub(super) fn is_mup(nt_path: &[u16]) -> bool {
        const MUP: &str = r"\Device\Mup\";
        let s = String::from_utf16_lossy(nt_path);
        s.get(..MUP.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(MUP))
    }

    /// 開いたハンドルの判定 (OWN-2 / OWN-3)。通れば `None`。
    fn judge(h: &File) -> Option<OwnerVerdict> {
        let Some((fs, flags)) = crate::sys::volume_info(h) else {
            return Some(unknown(QUERY_FAILED));
        };
        // 遠くのボリューム: ハンドルが `FileRemoteProtocolInfo` に答えるか、最終のパス (NT の形) が `\Device\Mup\` の下
        // (どちらかが答えなくても、もう片方で見つける)
        let remote =
            crate::sys::is_remote_handle(h) || owner::final_path(h).is_ok_and(|p| is_mup(&p));
        if let Some(v) = volume_verdict(&fs, flags, remote) {
            return Some(v);
        }
        let Some(trusted) = owner::trusted() else {
            return Some(unknown(QUERY_FAILED));
        };
        // アクセス拒否なども判定できない
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

    /// 開いた `child` の最終のパスの親が、開いた `parent` の最終のパスと同じか。
    fn opened_under(child: &File, parent: &File) -> bool {
        matches!((owner::final_path(parent), owner::final_path(child)), (Ok(p), Ok(c)) if same_parent(&c, &p))
    }

    /// 開いたディレクトリ (固定したもの) と、そのパス。`Err(結果)` は開けずに決まった結果 (`Missing`・`Rejected`)。
    type Opened = Result<(File, PathBuf, Vec<File>), CheckedRead>;

    /// `base` をリンクを辿って開き、`chain` の要素を 1 つずつリンクを辿らずに開く (07 §12.1 の OWN-6)。開いた祖先も
    /// 読み終えるまで持つ (固定)。
    fn open_dir_chain(base: PathBuf, chain: &[OsString], dir: &FsPath) -> Result<Opened, FsError> {
        let map = |e| crate::sys::map_io_error(e, dir);
        let mut dh = match owner::open_dir(&base, false) {
            Ok(h) => h,
            // ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND
            Err(e) if matches!(e.raw_os_error(), Some(2 | 3)) => {
                return Ok(Err(CheckedRead::Missing));
            }
            Err(e) => return Err(map(e)),
        };
        let mut path = base;
        let mut pins = Vec::new();
        for comp in chain {
            let (attrs, _) = owner::attr_tag(&dh).map_err(map)?;
            if attrs & FILE_ATTRIBUTE_DIRECTORY == 0 {
                return Ok(Err(CheckedRead::Missing));
            }
            let child_path = path.join(comp);
            let child = match owner::open_dir(&child_path, true) {
                Ok(h) => h,
                Err(e) if matches!(e.raw_os_error(), Some(2 | 3)) => {
                    return Ok(Err(CheckedRead::Missing));
                }
                Err(e) => return Err(map(e)),
            };
            let (cattrs, ctag) = owner::attr_tag(&child).map_err(map)?;
            // リンク・許可リストに無い再解析点のディレクトリは辿らない
            if cattrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 && !readable_reparse(ctag) {
                return Ok(Err(rejected(CheckedObject::Dir, OwnerVerdict::NotRegular)));
            }
            if cattrs & FILE_ATTRIBUTE_DIRECTORY == 0 {
                return Ok(Err(CheckedRead::Missing));
            }
            // 開いた子が、開いた親の中にある (途中の行き先を差し替えられていない)
            if !opened_under(&child, &dh) {
                return Ok(Err(rejected(CheckedObject::Dir, unknown(QUERY_FAILED))));
            }
            pins.push(std::mem::replace(&mut dh, child));
            path = child_path;
        }
        let (attrs, _) = owner::attr_tag(&dh).map_err(map)?;
        // ディレクトリでない (`.eve` がファイル) なら、その中のファイルは無い
        if attrs & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Ok(Err(CheckedRead::Missing));
        }
        Ok(Ok((dh, path, pins)))
    }

    pub(super) fn read_checked(
        dir: &FsPath,
        name: &OsStr,
        max_len: u64,
        safe_dirs: &[FsPath],
        (base, chain): (PathBuf, Vec<OsString>),
    ) -> Result<CheckedRead, FsError> {
        let map = |e, p: &FsPath| crate::sys::map_io_error(e, p);
        // ディレクトリを固定する (FILE_SHARE_DELETE なし。読み終えるまで `dh` と `_pins` を持つ)
        let (dh, dpath, _pins) = match open_dir_chain(base, &chain, dir)? {
            Ok(o) => o,
            Err(r) => return Ok(r),
        };
        let path = FsPath::new(dpath.join(name));
        // 有無を開かずに確かめる (属性の問い合わせ。最後の要素のリンクは辿らない)。問い合わせの失敗は「在る」として進め、
        // 開くときの結果に任せる
        let attrs = match owner::path_attributes(path.as_path()) {
            Ok(None) => return Ok(CheckedRead::Missing),
            Ok(Some(a)) => Some(a),
            Err(_) => None,
        };
        // 安全な場所の下か (開いたディレクトリの最終のパスで比べる。取れなければ下でないとする)
        let safe = owner::final_path_dos(&dh).is_ok_and(|p| under_safe(&p, safe_dirs));
        if !safe && let Some(v) = judge(&dh) {
            return Ok(rejected(CheckedObject::Dir, v));
        }
        if attrs.is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY != 0) {
            return Ok(rejected(CheckedObject::File, OwnerVerdict::NotRegular));
        }
        crate::hooks::point(BEFORE_OPEN);
        // リンクを辿らずに開く (リンクの先の UNC へ認証が走らない)
        let fh = match owner::open_file(path.as_path(), true) {
            Ok(h) => h,
            Err(e) if matches!(e.raw_os_error(), Some(2 | 3)) => return Ok(CheckedRead::Missing),
            Err(e) => return Err(map(e, &path)),
        };
        let (fattrs, ftag) = owner::attr_tag(&fh).map_err(|e| map(e, &path))?;
        let reparse = fattrs & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        if fattrs & FILE_ATTRIBUTE_DIRECTORY != 0
            || (reparse && !readable_reparse(ftag))
            || crate::sys::is_char_or_pipe(&fh)
        {
            return Ok(rejected(CheckedObject::File, OwnerVerdict::NotRegular));
        }
        // 開いたファイルの親が、固定したディレクトリと同じ所か (途中のジャンクションの行き先を差し替えられても、判定した物と
        // 違う所を読まない)
        if !opened_under(&fh, &dh) {
            return Ok(rejected(CheckedObject::File, unknown(QUERY_FAILED)));
        }
        if !safe && let Some(v) = judge(&fh) {
            return Ok(rejected(CheckedObject::File, v));
        }
        // 許可リストの再解析点 (Cloud Files・重複除去・WOF) は、再解析点そのものの中身ではなくフィルタを通した中身を読むために、
        // 識別で開き直す (名前を解かないので、名前を差し替えられてもリンクの先を開かない)。識別の無いものは読まない
        let reader = if reparse {
            let Some(id) = owner::file_id_128(&fh) else {
                return Ok(rejected(CheckedObject::File, unknown(QUERY_FAILED)));
            };
            let again = owner::open_by_id(&dh, id).map_err(|e| map(e, &path))?;
            let same = match (crate::sys::file_info(&fh), crate::sys::file_info(&again)) {
                (Ok(a), Ok(b)) => a.index != 0 && a.volume == b.volume && a.index == b.index,
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
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};

    use eve_core::{CheckedObject, CheckedRead, FsError, FsErrorKind, FsPath, OwnerVerdict};

    use super::{BEFORE_OPEN, QUERY_FAILED, read_limited, rejected, under_safe, unknown};
    use crate::sys::owner;

    /// `st_mode` の種類の部分 (`S_IFMT`) と、ふつうのファイル・シンボリックリンクの種類。POSIX の値。
    const S_IFMT: u32 = 0o170_000;
    const S_IFREG: u32 = 0o100_000;
    const S_IFLNK: u32 = 0o120_000;
    /// 他人の書込 (`o+w`)。
    const S_IWOTH: u32 = 0o002;
    /// Linux の overflowuid (`/proc/sys/kernel/overflowuid` の既定)。user namespace・NFS で写せない uid がこの値に見える。
    const OVERFLOW_UID: u32 = 65_534;

    /// 所有者と mode の判定 (07 §12.1 の OWN-2 / OWN-4)。「自分」は実効 uid と uid 0 だけ (root で動かしたときは uid 0 だけ。
    /// `FileMeta.owner_is_me` の「root はすべて自分」は使わない。`SUDO_UID` は見ない)。overflowuid は実効 uid に依らず
    /// 自分でない (写せなかった誰か)。`o+w` は sticky でも読まない。`g+w` は見ない。
    pub(super) fn judge_stat(uid: u32, mode: u32, euid: u32) -> Option<StatVerdict> {
        if uid == OVERFLOW_UID || (uid != euid && uid != 0) {
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

    /// `O_NOFOLLOW` がリンクで断ったときの errno か (Linux・macOS は `ELOOP`、FreeBSD・DragonFly は `EMLINK`、NetBSD は
    /// `EFTYPE`)。
    fn refused_link(code: i32) -> bool {
        #[cfg(any(target_os = "freebsd", target_os = "dragonfly"))]
        const OTHER: &[i32] = &[libc::EMLINK];
        #[cfg(target_os = "netbsd")]
        const OTHER: &[i32] = &[libc::EFTYPE];
        #[cfg(not(any(target_os = "freebsd", target_os = "dragonfly", target_os = "netbsd")))]
        const OTHER: &[i32] = &[];
        code == libc::ELOOP || OTHER.contains(&code)
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

    /// 拡張 ACL (`setfacl -m u:<他人>:w` は `st_mode` に `o+w` が出ない)。中身を解析せず、あれば書けるとみなす。
    /// Linux は `system.posix_acl_access` の拡張属性。`ENODATA` (無い)・`ENOTSUP` (FS が拡張属性を持たない) は ACL なし。
    #[cfg(target_os = "linux")]
    fn acl_verdict(f: &File) -> Option<OwnerVerdict> {
        match crate::sys::get_xattr(f, b"system.posix_acl_access") {
            Ok(_) => Some(OwnerVerdict::OthersCanWrite {
                who: vec!["acl".to_owned()],
            }),
            Err(e) if matches!(e.raw_os_error(), Some(libc::ENODATA | libc::ENOTSUP)) => None,
            Err(_) => Some(unknown(QUERY_FAILED)),
        }
    }

    /// Linux 以外は拡張 ACL を確かめていないので判定できない (`ENOTSUP` を「ACL なし」と読まない)。
    #[cfg(not(target_os = "linux"))]
    fn acl_verdict(_: &File) -> Option<OwnerVerdict> {
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
        acl_verdict(f)
    }

    /// `base` をリンクを辿って開き、`chain` の要素を 1 つずつリンクを辿らずに開く (07 §12.1 の OWN-6)。
    /// `Err(結果)` は開けずに決まった結果 (`Missing`・`Rejected`)。
    fn open_dir_chain(
        base: &Path,
        chain: &[OsString],
        dir: &FsPath,
    ) -> Result<Result<File, CheckedRead>, FsError> {
        let map = |e| crate::sys::map_io_error(e, dir);
        let mut dh = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(base)
        {
            Ok(h) => h,
            Err(e) if matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) => {
                return Ok(Err(CheckedRead::Missing));
            }
            Err(e) => return Err(map(e)),
        };
        for comp in chain {
            dh = match owner::open_dir_at_nofollow(&dh, comp) {
                Ok(c) => c,
                Err(e) => {
                    let code = e.raw_os_error().unwrap_or(0);
                    if code == libc::ENOENT {
                        return Ok(Err(CheckedRead::Missing));
                    }
                    // リンク (Linux は `O_DIRECTORY | O_NOFOLLOW` で `ENOTDIR`) か、ディレクトリでないか。何であったかを
                    // 知るためだけに、開いた親から `fstatat` する (辿らない)
                    if code == libc::ENOTDIR || refused_link(code) {
                        let mode = owner::mode_at(&dh, comp).map_err(map)?;
                        return Ok(Err(match mode {
                            Some(m) if m & S_IFMT == S_IFLNK => {
                                rejected(CheckedObject::Dir, OwnerVerdict::NotRegular)
                            }
                            _ => CheckedRead::Missing,
                        }));
                    }
                    return Err(map(e));
                }
            };
        }
        Ok(Ok(dh))
    }

    pub(super) fn read_checked(
        dir: &FsPath,
        name: &OsStr,
        max_len: u64,
        safe_dirs: &[FsPath],
        (base, chain): (PathBuf, Vec<OsString>),
    ) -> Result<CheckedRead, FsError> {
        let map = |e, p: &FsPath| crate::sys::map_io_error(e, p);
        let dh = match open_dir_chain(&base, &chain, dir)? {
            Ok(h) => h,
            Err(r) => return Ok(r),
        };
        let path = dir.join(name);
        let Some(mode) = owner::mode_at(&dh, name).map_err(|e| map(e, &path))? else {
            return Ok(CheckedRead::Missing);
        };
        // 安全な場所の下か (開いたディレクトリの最終のパスで比べる。取れなければ下でないとする)
        let safe = owner::final_path(&dh).is_some_and(|p| under_safe(&p, safe_dirs));
        if !safe && let Some(v) = judge(&dh) {
            return Ok(rejected(CheckedObject::Dir, v));
        }
        // シンボリックリンク・ディレクトリ・FIFO・デバイスは開かない (デバイスを開くだけで起きることを避ける)
        if mode & S_IFMT != S_IFREG {
            return Ok(rejected(CheckedObject::File, OwnerVerdict::NotRegular));
        }
        crate::hooks::point(BEFORE_OPEN);
        let fh = match owner::open_at_nofollow(&dh, name) {
            Ok(h) => h,
            Err(e) => {
                return match e.raw_os_error() {
                    Some(libc::ENOENT) => Ok(CheckedRead::Missing),
                    Some(c) if refused_link(c) => {
                        Ok(rejected(CheckedObject::File, OwnerVerdict::NotRegular))
                    }
                    // リース (`fcntl(F_SETLEASE)`) を持つ物がいる: 一時の失敗
                    Some(c) if c == libc::EAGAIN || c == libc::EWOULDBLOCK => {
                        Err(FsError::new(FsErrorKind::Busy))
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
        // ふつうのファイルと確かめたので、非ブロックを外して読む
        crate::sys::clear_nonblocking(&fh).map_err(|e| map(e, &path))?;
        if !safe && let Some(v) = judge(&fh) {
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
    safe_dirs: &[FsPath],
    links: DirLinks,
) -> Result<CheckedRead, FsError> {
    if !valid_name(name) {
        return Err(FsError::new(FsErrorKind::InvalidPath));
    }
    let Some(split) = split_dir(dir.as_path(), links) else {
        return Err(FsError::new(FsErrorKind::InvalidPath));
    };
    imp::read_checked(dir, name, max_len, safe_dirs, split)
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
        for win_bad in [r"a\b", "a:s", "config.toml.", "config.toml "] {
            assert_eq!(
                valid_name(OsStr::new(win_bad)),
                !cfg!(windows),
                "{win_bad:?}"
            );
        }
    }

    fn split(p: &str, links: DirLinks) -> Option<(PathBuf, Vec<String>)> {
        split_dir(Path::new(p), links).map(|(b, c)| {
            (
                b,
                c.into_iter()
                    .map(|s| s.to_string_lossy().into_owned())
                    .collect(),
            )
        })
    }

    #[test]
    fn eve_から先をリンクを辿らない側に分ける() {
        let r = if cfg!(windows) { r"C:\w" } else { "/w" };
        let sep = std::path::MAIN_SEPARATOR;
        let p = |tail: &str| format!("{r}{sep}{}", tail.replace('/', &sep.to_string()));
        assert_eq!(
            split(&p(".eve"), DirLinks::NoFollowFromEve),
            Some((PathBuf::from(r), vec![".eve".to_owned()]))
        );
        assert_eq!(
            split(&p(".eve/themes"), DirLinks::NoFollowFromEve),
            Some((
                PathBuf::from(r),
                vec![".eve".to_owned(), "themes".to_owned()]
            ))
        );
        // 大文字小文字を区別しない (全 OS)。最後の `.eve` から
        assert_eq!(
            split(&p("a/.EVE"), DirLinks::NoFollowFromEve).map(|(_, c)| c),
            Some(vec![".EVE".to_owned()])
        );
        assert_eq!(
            split(&p(".eve/x/.eve/y"), DirLinks::NoFollowFromEve).map(|(_, c)| c),
            Some(vec![".eve".to_owned(), "y".to_owned()])
        );
        // `.eve` が無い・`..` がある
        assert_eq!(split(&p("eve"), DirLinks::NoFollowFromEve), None);
        assert_eq!(split(&p("a/../.eve"), DirLinks::NoFollowFromEve), None);
        assert_eq!(split(&p("a/../b"), DirLinks::Follow), None);
        // 辿る側は分けない
        assert_eq!(
            split(&p("a"), DirLinks::Follow),
            Some((PathBuf::from(p("a")), Vec::new()))
        );
        // Windows は末尾の `.`・空白と `:` の要素を受け付けない (正規化で `.eve` の見分けを外せる)
        for bad in ["a./.eve", "a /.eve", "a:b/.eve", ".eve/x.", ".eve/x "] {
            for links in [DirLinks::Follow, DirLinks::NoFollowFromEve] {
                assert_eq!(
                    split(&p(bad), links).is_none(),
                    cfg!(windows),
                    "{bad:?} {links:?}"
                );
            }
        }
    }

    #[test]
    fn 安全な場所の下かを要素ごとに比べる() {
        let base = std::env::temp_dir().join(format!("eve-safe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("s").join("sub")).unwrap();
        std::fs::create_dir_all(base.join("sx")).unwrap();
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap();
        let safe = [FsPath::new(base.join("s"))];
        assert!(under_safe(&canon(&base.join("s")), &safe), "同じ");
        assert!(under_safe(&canon(&base.join("s").join("sub")), &safe), "下");
        assert!(
            !under_safe(&canon(&base.join("sx")), &safe),
            "名前の前方だけの一致"
        );
        assert!(!under_safe(&canon(&base), &safe), "上");
        assert!(
            !under_safe(&canon(&base.join("s")), &[FsPath::new("s")]),
            "相対のパス"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 大文字小文字の比べ方 (`canonicalize` が名前を直さない、存在しない場所で確かめる)。Windows は ASCII の違いだけを同じと
    /// みなし、KELVIN SIGN (U+212A。Rust の `to_lowercase` では `k` になるが、NTFS は別の名前とみなす) は違うとする。
    /// Unix は大文字小文字を区別する。
    #[test]
    fn 安全な場所は_ascii_の大文字小文字だけを畳んで比べる() {
        let none = std::env::temp_dir().join(format!("eve-none-{}", std::process::id()));
        assert!(!none.exists());
        let safe = [FsPath::new(none.join("work"))];
        assert!(under_safe(&none.join("work").join("sub"), &safe));
        let upper = none.join("WORK").join("sub");
        assert_eq!(under_safe(&upper, &safe), cfg!(windows), "ASCII の違い");
        if cfg!(windows) {
            let slash = FsPath::new(none.join("Work").to_string_lossy().replace('\\', "/"));
            assert!(under_safe(&none.join("work"), &[slash]), "`/` の違い");
            let verbatim = PathBuf::from(format!(r"\\?\{}", none.join("work").display()));
            assert!(under_safe(&verbatim, &safe), "`\\\\?\\` の形");
        }
        let kelvin = none.join("wor\u{212A}").join("sub");
        assert!(!under_safe(&kelvin, &safe), "U+212A は k でない");
        let kelvin_safe = [FsPath::new(none.join("wor\u{212A}"))];
        assert!(!under_safe(&none.join("work"), &kelvin_safe));
    }

    /// 最終のパスが UNC でなければ、UNC の形の安全な場所は解かずに飛ばす (届かない共有で止まらない)。
    #[cfg(windows)]
    #[test]
    fn 届かない_unc_の安全な場所はローカルの判定で問い合わせない() {
        let start = std::time::Instant::now();
        // 文書用の IP アドレス (RFC 5737)。解こうとすれば接続を待つ
        let unc = [
            FsPath::new(r"\\192.0.2.1\share\work"),
            FsPath::new(r"\\?\UNC\192.0.2.1\share\work"),
        ];
        assert!(!under_safe(&std::env::temp_dir(), &unc));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        assert!(is_unc(Path::new(r"\\server\share")));
        assert!(is_unc(Path::new(r"\\?\UNC\server\share")));
        assert!(!is_unc(Path::new(r"\\?\C:\x")));
        assert!(!is_unc(Path::new(r"\\.\pipe\x")));
        assert!(!is_unc(Path::new(r"C:\x")));
    }

    #[cfg(windows)]
    mod windows {
        use super::super::imp::{
            Writer, is_mup, judge_security, others_writers, readable_reparse, volume_verdict,
        };
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
            // Modify (0x1301bf)・Write (0x100116)・AD だけ (0x4)・GENERIC_WRITE・WRITE_DAC・WA だけ (0x100)・WEA だけ (0x10) は書込
            for mask in [
                0x0013_01BF,
                0x0010_0116,
                0x4,
                0x4000_0000,
                0x0004_0000,
                0x100,
                0x10,
            ] {
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

        #[test]
        fn 読む再解析点のタグは許可リストだけ() {
            // WOF・重複除去・Cloud Files の一族 (CLOUD・CLOUD_1〜F)・古い ONEDRIVE
            for t in [
                0x8000_0017,
                0x8000_0013,
                0x9000_001A,
                0x9000_101A,
                0x9000_F01A,
                0x8000_0021,
            ] {
                assert!(readable_reparse(t), "{t:#x}");
            }
            // ProjFS・STORAGE_SYNC・HSM・HSM2・DFS・DFSR・WCI・AF_UNIX・LX_SYMLINK・LX_FIFO・LX_CHR・LX_BLK・
            // App Execution Alias・シンボリックリンク・ジャンクション・知らないタグ (Microsoft のでないものを含む)
            for t in [
                0x9000_001C,
                0x8000_001E,
                0xC000_0004,
                0x8000_0006,
                0x8000_000A,
                0x8000_0012,
                0x8000_0018,
                0x8000_0023,
                0xA000_001D,
                0x8000_0024,
                0x8000_0025,
                0x8000_0026,
                0x8000_001B,
                0xA000_000C,
                0xA000_0003,
                0x8000_00FF,
                0x0000_1234,
                0x9000_001B,
            ] {
                assert!(!readable_reparse(t), "{t:#x}");
            }
        }

        #[test]
        fn 遠くのボリュームと_ntfs_refs_でない_fs_は判定できない() {
            let unk = |fs: &str| Some(OwnerVerdict::Unknown { fs: fs.to_owned() });
            assert_eq!(volume_verdict("NTFS", 0x8, false), None);
            assert_eq!(volume_verdict("ReFS", 0x8, false), None);
            assert_eq!(volume_verdict("NTFS", 0x8, true), unk("remote"));
            assert_eq!(volume_verdict("exFAT", 0, false), unk("exFAT"));
            assert_eq!(volume_verdict("FAT32", 0, false), unk("FAT32"));
            // ACL を持つと名乗っても NTFS・ReFS でない FS (ユーザーモードの FS など) は判定しない
            assert_eq!(volume_verdict("WinFsp", 0x8, false), unk("WinFsp"));
            assert_eq!(volume_verdict("NTFS", 0, false), unk("NTFS"));
            let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
            assert!(is_mup(&w(r"\Device\Mup\localhost\C$\x")));
            assert!(is_mup(&w(r"\DEVICE\MUP\server\share")));
            assert!(!is_mup(&w(r"\Device\HarddiskVolume3\x")));
            assert!(!is_mup(&w(r"\Device\Mupx\a")));
            assert!(!is_mup(&w(r"\Device")));
        }
    }

    /// 一時ディレクトリ (終わりに消す)。
    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let p = std::env::temp_dir().join(format!("eve-rc-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn rc(dir: &Path, name: &str, links: DirLinks) -> CheckedRead {
        read_checked(&FsPath::new(dir), OsStr::new(name), 1024, &[], links)
            .unwrap_or_else(|e| panic!("read_checked: {e:?}"))
    }

    #[cfg(windows)]
    mod windows_fs {
        use super::{BEFORE_OPEN, CheckedObject, CheckedRead, DirLinks, OwnerVerdict, Tmp, rc};
        use crate::sys::owner::set_reparse_for_test;

        fn mklink_j(link: &std::path::Path, target: &std::path::Path) {
            let ok = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .expect("cmd")
                .status
                .success();
            assert!(ok, "mklink /J");
        }

        /// `FSCTL_SET_REPARSE_POINT` で付けた、許可リストに無いタグ (Microsoft のでないもの・名前の代理の印があるもの) の
        /// ファイルと `.eve` は読まない (PO の決定 2026-11-23 の B)。
        #[test]
        fn 許可リストに無い再解析点は読まない() {
            let t = Tmp::new("tag");
            let not_regular = |what| CheckedRead::Rejected {
                what,
                verdict: OwnerVerdict::NotRegular,
            };
            for (name, tag) in [("a.toml", 0x0000_1234), ("b.toml", 0x2000_1234)] {
                std::fs::write(t.0.join(name), b"x").unwrap();
                set_reparse_for_test(&t.0.join(name), tag).expect("再解析点を付ける");
                assert_eq!(
                    rc(&t.0, name, DirLinks::Follow),
                    not_regular(CheckedObject::File),
                    "{tag:#x}"
                );
            }
            let eve = t.0.join("w").join(".eve");
            std::fs::create_dir_all(&eve).unwrap();
            set_reparse_for_test(&eve, 0x0000_1234).expect("ディレクトリに再解析点を付ける");
            assert_eq!(
                rc(&eve, "config.toml", DirLinks::NoFollowFromEve),
                not_regular(CheckedObject::Dir)
            );
        }

        /// WOF で圧縮したファイル (`compact /exe`。許可リストのタグ `IO_REPARSE_TAG_WOF`) は、識別で開き直してフィルタを
        /// 通した中身を読む (再解析点そのものの中身でない)。`compact` を使えない・再解析点が見えない環境では確かめない。
        #[test]
        #[allow(clippy::print_stderr, reason = "試験の飛ばした理由を出す")]
        fn wof_で圧縮したファイルは識別で開き直して読む() {
            let t = Tmp::new("wof");
            let f = t.0.join("c.toml");
            let body = "key = 'value'\n".repeat(1000);
            std::fs::write(&f, &body).unwrap();
            let ok = std::process::Command::new("compact")
                .args(["/c", "/exe:xpress4k"])
                .arg(&f)
                .output()
                .is_ok_and(|o| o.status.success());
            let tag = crate::sys::owner::open_file(&f, true)
                .and_then(|h| crate::sys::owner::attr_tag(&h))
                .map(|(a, tag)| (a & 0x400 != 0).then_some(tag));
            // WOF のフィルタは、再解析点を隠して見せることがある (Windows 11 で実測: `FILE_FLAG_OPEN_REPARSE_POINT` で
            // 開いても属性に出ない)。そのときは開き直さずに同じハンドルから読み、中身はフィルタが解いたものになる
            if !matches!(&tag, Ok(Some(0x8000_0017))) {
                eprintln!(
                    "WOF の再解析点が見えない (compact: {ok}, {tag:?}): 開き直さない経路で確かめる"
                );
            }
            let r = super::read_checked(
                &super::FsPath::new(&t.0),
                std::ffi::OsStr::new("c.toml"),
                1 << 20,
                &[],
                DirLinks::Follow,
            )
            .expect("read_checked");
            assert!(
                r == CheckedRead::Read {
                    bytes: body.into_bytes()
                },
                "圧縮を解いた中身でない"
            );
        }

        /// ディレクトリを判定した後、ファイルを開く前に、祖先のジャンクションの行き先を別の所へ替えても、判定した所の外の
        /// ファイルを読まない (開いたファイルの親と固定したディレクトリの最終のパスが違うので `Unknown`。OWN-6)。
        #[test]
        fn 判定の後に祖先のジャンクションを替えても外を読まない() {
            let t = Tmp::new("retarget");
            for (d, body) in [("real", "real"), ("other", "other")] {
                std::fs::create_dir_all(t.0.join(d).join("sub")).unwrap();
                std::fs::write(t.0.join(d).join("sub").join("config.toml"), body).unwrap();
            }
            let j = t.0.join("j");
            mklink_j(&j, &t.0.join("real"));
            assert_eq!(
                rc(&j.join("sub"), "config.toml", DirLinks::Follow),
                CheckedRead::Read {
                    bytes: b"real".to_vec()
                }
            );
            let (j2, other) = (j.clone(), t.0.join("other"));
            crate::hooks::on_point(BEFORE_OPEN, move || {
                // ジャンクションそのものは固定していない (固定したのは行き先の real\sub とその祖先)
                std::fs::remove_dir(&j2).expect("ジャンクションを消す");
                mklink_j(&j2, &other);
            });
            assert_eq!(
                rc(&j.join("sub"), "config.toml", DirLinks::Follow),
                CheckedRead::Rejected {
                    what: CheckedObject::File,
                    verdict: OwnerVerdict::Unknown {
                        fs: "query_failed".to_owned()
                    }
                }
            );
        }

        /// 判定の後、開く前に、固定したディレクトリそのものは改名できない (`FILE_SHARE_DELETE` なしで開いている)。
        #[test]
        fn 判定の後に置き場所を改名できない() {
            let t = Tmp::new("pinned");
            let d = t.0.join("ws");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("config.toml"), b"x").unwrap();
            let (from, to) = (d.clone(), t.0.join("ws2"));
            let renamed = std::rc::Rc::new(std::cell::Cell::new(true));
            let r2 = renamed.clone();
            crate::hooks::on_point(BEFORE_OPEN, move || {
                r2.set(std::fs::rename(&from, &to).is_ok());
            });
            assert_eq!(
                rc(&d, "config.toml", DirLinks::Follow),
                CheckedRead::Read {
                    bytes: b"x".to_vec()
                }
            );
            assert!(!renamed.get(), "固定したディレクトリを改名できた");
        }
    }

    // Linux 以外の Unix は所有者を判定しない (どれも Unknown { unsupported }) ので、読めることを求める試験は Linux だけ
    #[cfg(target_os = "linux")]
    mod unix_fs {
        use super::{BEFORE_OPEN, CheckedObject, CheckedRead, DirLinks, OwnerVerdict, Tmp, rc};

        /// ディレクトリを判定した後、ファイルを開く前に、読むファイルをリンクに替えても辿らない (`openat(O_NOFOLLOW)`)。
        #[test]
        fn 判定の後にファイルをリンクに替えても辿らない() {
            let t = Tmp::new("swap");
            let d = t.0.join("ws");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(t.0.join("secret"), b"secret").unwrap();
            std::fs::write(d.join("config.toml"), b"x").unwrap();
            let (f, secret) = (d.join("config.toml"), t.0.join("secret"));
            crate::hooks::on_point(BEFORE_OPEN, move || {
                std::fs::remove_file(&f).unwrap();
                std::os::unix::fs::symlink(&secret, &f).unwrap();
            });
            assert_eq!(
                rc(&d, "config.toml", DirLinks::Follow),
                CheckedRead::Rejected {
                    what: CheckedObject::File,
                    verdict: OwnerVerdict::NotRegular
                }
            );
        }

        /// 判定の後、開く前に、置き場所のディレクトリを改名して別のディレクトリを置いても、判定したディレクトリ
        /// (開いた記述子) の中を読む。
        #[test]
        fn 判定の後に置き場所を差し替えても判定した所を読む() {
            let t = Tmp::new("dirswap");
            let d = t.0.join("ws");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("config.toml"), b"x").unwrap();
            let (from, old) = (d.clone(), t.0.join("ws-old"));
            crate::hooks::on_point(BEFORE_OPEN, move || {
                std::fs::rename(&from, &old).unwrap();
                std::fs::create_dir(&from).unwrap();
                std::fs::write(from.join("config.toml"), b"evil").unwrap();
            });
            assert_eq!(
                rc(&d, "config.toml", DirLinks::Follow),
                CheckedRead::Read {
                    bytes: b"x".to_vec()
                }
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
            // overflowuid は実効 uid に依らず自分でない
            assert_eq!(
                judge_stat(65_534, 0o40755, 65_534),
                Some(StatVerdict::NotOwner)
            );
            assert_eq!(judge_stat(65_534, 0o40755, 0), Some(StatVerdict::NotOwner));
        }
    }
}
