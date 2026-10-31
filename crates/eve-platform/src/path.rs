//! パスの正規化 (docs/05 §3.1〜§3.3。Windows の規則 `normalize_windows` と Unix の規則 `normalize_unix`)。I/O をしない計算で、
//! どの OS でも動く (テストのため)。
//!
//! 手順 (05 §3.2): (1) チルダ展開 (`~`、`~\`、`~/` だけ) → (3) 相対を絶対に (ドライブ相対・ルート相対を含む) →
//! (5) Windows 固有: `/` → `\`、連続区切りの圧縮、末尾区切りの除去、**予約名・禁止文字・デバイス名前空間・
//! 代替データストリームの拒否**。`..` は畳まない (シンボリックリンクを跨ぐと意味が変わる。`canonicalize` で解決する)。

use eve_core::{FsError, FsErrorDetail, FsErrorKind, FsPath};

/// 正規化の入力となる、起動時に解決済みの環境。
#[derive(Clone, Debug)]
pub struct PathCtx {
    /// 起動時の作業ディレクトリ (絶対パス。以後 `chdir` しない)。
    pub cwd: String,
    /// ホームディレクトリ (Windows は `%USERPROFILE%`、Unix は `$HOME`、05 §3.6)。不明なら `None`。
    pub home: Option<String>,
}

fn invalid() -> FsError {
    FsError::new(FsErrorKind::InvalidPath)
}

fn not_a_file() -> FsError {
    FsError::with_detail(FsErrorKind::InvalidPath, FsErrorDetail::NotAFile)
}

fn reserved() -> FsError {
    FsError::with_detail(FsErrorKind::InvalidPath, FsErrorDetail::ReservedName)
}

/// 予約名か (拡張子付きでも、大文字小文字を区別せず)。末尾のドット・空白は呼び側が先に弾く。
pub fn is_reserved_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or("");
    let up = stem.trim_end_matches(' ').to_uppercase();
    match up.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => true,
        _ => {
            let b = up.as_bytes();
            let dev = up.starts_with("COM") || up.starts_with("LPT");
            if !dev {
                return false;
            }
            let rest = &up[3..];
            // COM0-9 / LPT0-9、上付き数字の COM¹ COM² COM³ LPT¹ LPT² LPT³
            let _ = b;
            matches!(
                rest,
                "0" | "1"
                    | "2"
                    | "3"
                    | "4"
                    | "5"
                    | "6"
                    | "7"
                    | "8"
                    | "9"
                    | "\u{B9}"
                    | "\u{B2}"
                    | "\u{B3}"
            )
        }
    }
}

/// 要素の検査 (ドライブ・UNC の接頭辞を除いた各要素)。
fn check_component(c: &str) -> Result<(), FsError> {
    if c.contains(':') {
        // 代替データストリーム (a.txt:s、a.txt::$DATA)
        return Err(not_a_file());
    }
    if c.chars()
        .any(|ch| matches!(ch, '<' | '>' | '"' | '|' | '?' | '*') || (ch as u32) < 0x20)
    {
        return Err(invalid());
    }
    if c != "." && c != ".." && (c.ends_with('.') || c.ends_with(' ')) {
        return Err(invalid());
    }
    if is_reserved_name(c) {
        return Err(reserved());
    }
    Ok(())
}

/// ドライブ文字 (`C:` の `C`) を返す。
fn drive_of(s: &str) -> Option<char> {
    let mut it = s.chars();
    let (a, b) = (it.next()?, it.next()?);
    (a.is_ascii_alphabetic() && b == ':').then_some(a.to_ascii_uppercase())
}

/// 文字列の先頭の接頭辞 (`C:\`・`\\server\share\`) と残りに分ける。接頭辞は正規化済みの形で返す。
enum Root {
    Drive(char),
    Unc { server: String, share: String },
}

fn render_root(r: &Root) -> String {
    match r {
        Root::Drive(d) => format!("{d}:\\"),
        Root::Unc { server, share } => format!("\\\\{server}\\{share}\\"),
    }
}

/// 絶対パス (区切りは `\` に統一済み) を接頭辞と残りに分ける。
fn split_abs(s: &str) -> Result<(Root, &str), FsError> {
    if let Some(rest) = s.strip_prefix("\\\\?\\") {
        // 許すのは `\\?\X:\` と `\\?\UNC\`
        if let Some(d) = drive_of(rest)
            && rest[2..].starts_with('\\')
        {
            return Ok((Root::Drive(d), &rest[3..]));
        }
        if let Some(unc) = rest.strip_prefix("UNC\\") {
            return split_unc(unc);
        }
        return Err(not_a_file());
    }
    if s.starts_with("\\\\.\\") {
        return Err(not_a_file());
    }
    if let Some(unc) = s.strip_prefix("\\\\") {
        return split_unc(unc);
    }
    if let Some(d) = drive_of(s)
        && s[2..].starts_with('\\')
    {
        return Ok((Root::Drive(d), &s[3..]));
    }
    Err(invalid())
}

fn split_unc(unc: &str) -> Result<(Root, &str), FsError> {
    let mut parts = unc.splitn(3, '\\');
    let server = parts.next().unwrap_or("");
    let share = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("");
    if server.is_empty() || share.is_empty() {
        return Err(invalid());
    }
    check_component(server).map_err(|_| invalid())?;
    check_component(share)?;
    Ok((
        Root::Unc {
            server: server.to_owned(),
            share: share.to_owned(),
        },
        rest,
    ))
}

/// Unix のパスの正規化 (05 §3.2 の手順 1・3・4)。チルダ展開 (`~` と `~/` だけ) → 相対を作業ディレクトリ基準の絶対に →
/// 連続の `/` と `.` の要素を除き、末尾の `/` を除く (ルートは `/`)。`..` は畳まない。存在の確認はしない。
pub fn normalize_unix(raw: &str, ctx: &PathCtx) -> Result<FsPath, FsError> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(invalid());
    }
    let s = if raw == "~" || raw.starts_with("~/") {
        let home = ctx.home.as_deref().ok_or_else(invalid)?;
        format!("{}{}", home.trim_end_matches('/'), &raw[1..])
    } else {
        raw.to_owned()
    };
    let absolute = if s.starts_with('/') {
        s
    } else {
        format!("{}/{s}", ctx.cwd.trim_end_matches('/'))
    };
    if !absolute.starts_with('/') {
        return Err(invalid());
    }
    let comps: Vec<&str> = absolute
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    Ok(FsPath::new(format!("/{}", comps.join("/"))))
}

/// Windows のパスの正規化 (手順 1・3・5)。存在の確認はしない。
pub fn normalize_windows(raw: &str, ctx: &PathCtx) -> Result<FsPath, FsError> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(invalid());
    }
    // 1 チルダ展開
    let mut s = if raw == "~" || raw.starts_with("~\\") || raw.starts_with("~/") {
        let home = ctx.home.as_deref().ok_or_else(invalid)?;
        format!("{}{}", home.trim_end_matches(['\\', '/']), &raw[1..])
    } else {
        raw.to_owned()
    };
    // 5 区切りを `\` に統一
    s = s.replace('/', "\\");

    // 3 相対 → 絶対
    let absolute = if s.starts_with("\\\\") {
        s
    } else if let Some(d) = drive_of(&s) {
        if s[2..].starts_with('\\') {
            s
        } else if drive_of(&ctx.cwd) == Some(d) {
            // ドライブ相対 (C:foo): 同じドライブの作業ディレクトリ基準
            format!("{}\\{}", ctx.cwd.trim_end_matches('\\'), &s[2..])
        } else {
            // 他のドライブのカレントは分からないので、そのドライブのルートを基準にする
            format!("{d}:\\{}", &s[2..])
        }
    } else if s.starts_with('\\') {
        // ルート相対 (\foo): 作業ディレクトリのドライブ
        let d = drive_of(&ctx.cwd).ok_or_else(invalid)?;
        format!("{d}:{s}")
    } else {
        format!("{}\\{}", ctx.cwd.trim_end_matches('\\'), s)
    };

    let (root, rest) = split_abs(&absolute)?;
    let mut out = render_root(&root);
    let comps: Vec<&str> = rest
        .split('\\')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    for c in &comps {
        check_component(c)?;
    }
    out.push_str(&comps.join("\\"));
    // 末尾の区切りはルートを除いて除去済み (join で付かない)。ルートだけなら `C:\` のまま
    Ok(FsPath::new(out))
}

/// 表示用のパス (verbatim 接頭辞を外せる場合だけ外す。05 §3.3)。260 文字以上・予約名・`..` を含むときは外さない。
pub fn simplify_verbatim(path: &str) -> String {
    let Some(rest) = path.strip_prefix("\\\\?\\") else {
        return path.to_owned();
    };
    let (simple, tail) = if let Some(unc) = rest.strip_prefix("UNC\\") {
        (format!("\\\\{unc}"), unc)
    } else if drive_of(rest).is_some() && rest[2..].starts_with('\\') {
        (rest.to_owned(), &rest[3..])
    } else {
        return path.to_owned();
    };
    let bad = simple.chars().count() >= 260
        || tail.split('\\').any(|c| {
            c == ".." || c == "." || is_reserved_name(c) || c.ends_with('.') || c.ends_with(' ')
        });
    if bad { path.to_owned() } else { simple }
}

#[cfg(test)]
mod unix_tests {
    use super::*;

    fn n(raw: &str) -> Result<FsPath, FsError> {
        normalize_unix(
            raw,
            &PathCtx {
                cwd: "/home/me/w".into(),
                home: Some("/home/me/".into()),
            },
        )
    }

    #[test]
    fn unix_の正規化() {
        assert_eq!(n("a.txt").unwrap(), FsPath::new("/home/me/w/a.txt"));
        assert_eq!(n("./x//y/").unwrap(), FsPath::new("/home/me/w/x/y"));
        assert_eq!(n("~").unwrap(), FsPath::new("/home/me"));
        assert_eq!(n("~/note.md").unwrap(), FsPath::new("/home/me/note.md"));
        assert_eq!(n("/").unwrap(), FsPath::new("/"));
        assert_eq!(
            n("/a/../b").unwrap(),
            FsPath::new("/a/../b"),
            "`..` は畳まない"
        );
        assert_eq!(
            n("~user/x").unwrap(),
            FsPath::new("/home/me/w/~user/x"),
            "`~user` は展開しない"
        );
        assert_eq!(
            n("C:\\x").unwrap(),
            FsPath::new("/home/me/w/C:\\x"),
            "`\\` は区切りではない"
        );
        assert!(n("").is_err() && n("a\0b").is_err());
        let no_home = normalize_unix(
            "~/x",
            &PathCtx {
                cwd: "/".into(),
                home: None,
            },
        );
        assert!(no_home.is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> PathCtx {
        PathCtx {
            cwd: "C:\\work\\proj".into(),
            home: Some("C:\\Users\\me".into()),
        }
    }

    fn n(raw: &str) -> Result<String, FsError> {
        normalize_windows(raw, &ctx()).map(|p| p.as_path().to_string_lossy().into_owned())
    }

    #[test]
    fn 相対は作業ディレクトリ基準で絶対になる() {
        assert_eq!(n("a.txt").unwrap(), "C:\\work\\proj\\a.txt");
        assert_eq!(n(".\\a\\b.txt").unwrap(), "C:\\work\\proj\\a\\b.txt");
        assert_eq!(n("a/b/c.txt").unwrap(), "C:\\work\\proj\\a\\b\\c.txt");
    }

    #[test]
    fn 絶対パスの区切りと連続区切りと末尾() {
        assert_eq!(n("C:/x//y/").unwrap(), "C:\\x\\y");
        assert_eq!(
            n("c:\\x\\y").unwrap(),
            "C:\\x\\y",
            "ドライブ文字は大文字にする"
        );
        assert_eq!(n("C:\\").unwrap(), "C:\\");
        assert_eq!(n("D:\\.\\a").unwrap(), "D:\\a", "`.` は除く");
    }

    #[test]
    fn ドット_ドットは畳まない() {
        assert_eq!(n("..\\a").unwrap(), "C:\\work\\proj\\..\\a");
        assert_eq!(n("C:\\x\\..\\y").unwrap(), "C:\\x\\..\\y");
    }

    #[test]
    fn ドライブ相対とルート相対() {
        assert_eq!(
            n("C:foo").unwrap(),
            "C:\\work\\proj\\foo",
            "同じドライブは作業ディレクトリ基準"
        );
        assert_eq!(n("D:foo").unwrap(), "D:\\foo", "他のドライブはルート基準");
        assert_eq!(
            n("\\foo\\bar").unwrap(),
            "C:\\foo\\bar",
            "作業ディレクトリのドライブ"
        );
    }

    #[test]
    fn チルダ展開() {
        assert_eq!(n("~").unwrap(), "C:\\Users\\me");
        assert_eq!(n("~\\a.txt").unwrap(), "C:\\Users\\me\\a.txt");
        assert_eq!(n("~/a.txt").unwrap(), "C:\\Users\\me\\a.txt");
        // `~user` 形式は展開しない (相対パスの要素)
        assert_eq!(n("~user\\a").unwrap(), "C:\\work\\proj\\~user\\a");
        let no_home = PathCtx {
            home: None,
            ..ctx()
        };
        assert_eq!(
            normalize_windows("~", &no_home).unwrap_err().kind,
            FsErrorKind::InvalidPath
        );
    }

    #[test]
    fn unc_と_verbatim() {
        assert_eq!(
            n("\\\\server\\share\\a\\b.txt").unwrap(),
            "\\\\server\\share\\a\\b.txt"
        );
        assert_eq!(n("//server/share/a").unwrap(), "\\\\server\\share\\a");
        assert_eq!(
            n("\\\\?\\C:\\a\\b").unwrap(),
            "C:\\a\\b",
            "verbatim は接頭辞を外した形に統一する"
        );
        assert_eq!(n("\\\\?\\UNC\\srv\\sh\\a").unwrap(), "\\\\srv\\sh\\a");
        assert!(n("\\\\server").is_err(), "共有名の無い UNC");
    }

    #[test]
    fn デバイス名前空間とストリームは_io_の前に拒否する() {
        for raw in [
            "\\\\.\\pipe\\x",
            "\\\\.\\PhysicalDrive0",
            "\\\\?\\GLOBALROOT\\Device\\x",
            "\\\\?\\Volume{1234}\\a",
            "a.txt:stream",
            "a.txt::$DATA",
            "C:\\x\\a.txt:s",
        ] {
            let e = n(raw).unwrap_err();
            assert_eq!(e.kind, FsErrorKind::InvalidPath, "{raw}");
            assert_eq!(e.detail, Some(FsErrorDetail::NotAFile), "{raw}");
        }
    }

    #[test]
    fn 予約名と禁止文字() {
        for raw in [
            "con",
            "CON.txt",
            "x\\nul",
            "C:\\a\\PRN.log",
            "com1",
            "LPT9.md",
            "aux ",
            "COM\u{B9}",
            "conin$",
            "CONOUT$.x",
        ] {
            let r = n(raw);
            assert!(r.is_err(), "{raw}: {r:?}");
        }
        assert_eq!(
            n("con").unwrap_err().detail,
            Some(FsErrorDetail::ReservedName)
        );
        // 予約名に似ているだけのものは通る
        for ok in ["console.txt", "com10", "comx", "lpt", "nul2", "a.con"] {
            assert!(n(ok).is_ok(), "{ok}");
        }
        for raw in [
            "a<b",
            "a>b",
            "a\"b",
            "a|b",
            "a?b",
            "a*b",
            "a\u{1}b",
            "dir\\name.",
            "dir\\name ",
        ] {
            assert_eq!(
                n(raw).unwrap_err().kind,
                FsErrorKind::InvalidPath,
                "{raw:?}"
            );
        }
        assert!(n("").is_err());
    }

    #[test]
    fn 日本語のパスは通る() {
        assert_eq!(
            n("日本語\\ファイル.txt").unwrap(),
            "C:\\work\\proj\\日本語\\ファイル.txt"
        );
    }

    #[test]
    fn verbatim_の接頭辞を外す() {
        assert_eq!(simplify_verbatim("\\\\?\\C:\\a\\b.txt"), "C:\\a\\b.txt");
        assert_eq!(
            simplify_verbatim("\\\\?\\UNC\\srv\\sh\\a"),
            "\\\\srv\\sh\\a"
        );
        assert_eq!(
            simplify_verbatim("C:\\a"),
            "C:\\a",
            "接頭辞が無ければそのまま"
        );
        // 260 文字以上・予約名・`..` を含むときは外さない
        let long = format!("\\\\?\\C:\\{}", "x".repeat(300));
        assert_eq!(simplify_verbatim(&long), long);
        assert_eq!(simplify_verbatim("\\\\?\\C:\\a\\con"), "\\\\?\\C:\\a\\con");
        assert_eq!(
            simplify_verbatim("\\\\?\\C:\\a\\..\\b"),
            "\\\\?\\C:\\a\\..\\b"
        );
        // 未知の形はそのまま
        assert_eq!(
            simplify_verbatim("\\\\?\\Volume{1}\\a"),
            "\\\\?\\Volume{1}\\a"
        );
    }
}
