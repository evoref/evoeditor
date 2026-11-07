//! 置換途中の痕跡の起動時の走査 (docs/05 §4.6「起動時の提案の規則」、03 §10.3 手順 1b)。
//!
//! `list_inflight` の記録を T のパスごとにまとめ、記録された一時ファイル (`.eve-tmp-*`)・退避名 (`.eve-bak-*`) の実在と T の有無から、
//! 提案 (`Leftover`) を作る。利用者のディレクトリは列挙せず、記録された名前の `metadata` だけを見る。ジョブの中で呼ぶ。
//! 痕跡が何も残っていない記録のフラグと、どのフラグにも記録されていない書込途中の事前バックアップ (`*.bak.tmp`) はここで片付ける。

use std::collections::BTreeSet;

use crate::io::fs::{FileSystem, FileTime, FsError, FsErrorKind, InflightRecord, SaveMethod};
use crate::io::fspath::FsPath;

/// 提案の種類 (05 §4.6)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftoverKind {
    /// T が無く、退避名 (bak) と一時ファイル (tmp) の両方がある。
    BakAndTmp,
    /// T が無く、退避名だけがある。
    Bak,
    /// T があり、退避名がある (置換の成功後の削除漏れ)。
    StaleBak,
    /// 一時ファイルがある (T の有無に依らない)。
    Tmp,
}

/// 退避名と T の内容を比べるときの大きさの上限 (05 §4.6 の提案の規則 2)。これより大きければ比べない。
const COMPARE_LIMIT: u64 = 64 * 1024 * 1024;

/// 提案への答えのうち、ディスクを変える操作 (`JobKind::LeftoverAct`)。「別バッファで開く」は文書を開くだけなので含めない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftoverOp {
    /// 退避名を T の名前に戻す (T が在れば失敗して何もしない)。
    Restore,
    /// 退避名を消す。
    DeleteBak,
    /// 一時ファイルを消す。
    DeleteTmp,
}

/// 起動時に提案する 1 件。T のパス 1 つ分。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leftover {
    /// 痕跡の種類。
    pub kind: LeftoverKind,
    /// 保存先 T。
    pub target: FsPath,
    /// 実在する退避名。`Bak` / `StaleBak` / `BakAndTmp` で `Some`。
    pub bak: Option<FsPath>,
    /// 実在する一時ファイル。`Tmp` / `BakAndTmp` で `Some`。
    pub tmp: Option<FsPath>,
    /// この T の保存中フラグ (片付けたら消す)。
    pub flags: Vec<FsPath>,
    /// 記録した保存の開始時刻 (新しい順に出すため)。
    pub recorded_at: Option<FileTime>,
}

fn exists(fs: &dyn FileSystem, p: &FsPath) -> bool {
    fs.metadata(p).is_ok()
}

/// 同じ T か (Windows は大文字小文字を区別しない、05 §3.1)。
fn group_key(p: &FsPath) -> String {
    let s = p.display();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

fn newer(a: FileTime, b: FileTime) -> bool {
    (a.secs, a.nanos) > (b.secs, b.nanos)
}

/// `list_inflight` と `list_orphan_backups` を読み、提案を新しい順に返す。片付けも行う (ジョブ内)。
pub(crate) fn scan(fs: &dyn FileSystem) -> Vec<Leftover> {
    let records = fs.list_inflight().unwrap_or_default();
    cleanup_partial_backups(fs, &records);

    // T ごとにまとめる。解析できないフラグ・直接上書きの記録は提案の対象にしない (開くときの警告に任せ、フラグは残す)
    let mut groups: Vec<(String, Vec<&InflightRecord>)> = Vec::new();
    for r in records
        .iter()
        .filter(|r| !r.malformed && r.method != SaveMethod::InPlace)
    {
        let key = group_key(&r.target);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(r),
            None => groups.push((key, vec![r])),
        }
    }

    let mut out = Vec::new();
    for (_, recs) in groups {
        let target = recs[0].target.clone();
        let Some(mut t) = Traces::collect(fs, &recs) else {
            continue;
        };
        // T の有無。調べられない (ネットワークの不通など) ときは、状態が分からないので提案しない
        let t_exists = match fs.metadata(&target) {
            Ok(_) => true,
            Err(e) if e.kind == FsErrorKind::NotFound => false,
            Err(_) => continue,
        };
        // T のハードリンクとして残った新規ファイルの一時ファイル (退路で link の後に消せなかった) は、確認なしで消す
        // (提案の規則 2。docs/review/v0.3-findings.md の SAF-40)。消した後は記録ごとに痕跡を見直す (別の記録の tmp を隠さない)
        if t_exists {
            let mut removed = false;
            for r in recs.iter().filter(|r| r.method == SaveMethod::NewFile) {
                if let Some(tmp) = r.tmp.as_ref().filter(|p| exists(fs, p))
                    && hard_link_of(fs, tmp, &target)
                    && fs.remove_file(tmp).is_ok()
                {
                    removed = true;
                }
            }
            if removed {
                match Traces::collect(fs, &recs) {
                    Some(n) => t = n,
                    None => continue,
                }
            }
        }
        // T と内容が同一の退避名は、置換の成功後の削除漏れで、確認なしで消す (提案の規則 2)
        if t_exists
            && let Some(b) = t.bak.clone()
            && identical(fs, &b, &target)
            && fs.remove_file(&b).is_ok()
        {
            match Traces::collect(fs, &recs) {
                Some(n) => t = n,
                None => continue,
            }
        }
        let Traces {
            bak,
            tmp,
            flags,
            recorded_at,
        } = t;
        let mk = |kind, bak, tmp| Leftover {
            kind,
            target: target.clone(),
            bak,
            tmp,
            flags: flags.clone(),
            recorded_at,
        };
        match (t_exists, bak, tmp) {
            (false, Some(b), Some(t)) => out.push(mk(LeftoverKind::BakAndTmp, Some(b), Some(t))),
            (false, Some(b), None) => out.push(mk(LeftoverKind::Bak, Some(b), None)),
            (true, Some(b), t) => {
                out.push(mk(LeftoverKind::StaleBak, Some(b), None));
                if let Some(t) = t {
                    out.push(mk(LeftoverKind::Tmp, None, Some(t)));
                }
            }
            (_, None, Some(t)) => out.push(mk(LeftoverKind::Tmp, None, Some(t))),
            (_, None, None) => {}
        }
    }
    // 新しい順 (記録時刻の無いものは最後)
    out.sort_by_key(|l| std::cmp::Reverse(l.recorded_at.map(|t| (t.secs, t.nanos))));
    out
}

/// 1 つの T の記録から集めた、実在する痕跡と、それを記録したフラグ。
struct Traces {
    bak: Option<FsPath>,
    tmp: Option<FsPath>,
    flags: Vec<FsPath>,
    recorded_at: Option<FileTime>,
}

impl Traces {
    /// 記録ごとに一時ファイル・退避名の実在を見る。痕跡が何も残っていない記録はフラグだけを消す (`clear_others` と同じ考え方)。
    /// どの記録にも痕跡が無ければ `None`。
    fn collect(fs: &dyn FileSystem, recs: &[&InflightRecord]) -> Option<Traces> {
        let mut t = Traces {
            bak: None,
            tmp: None,
            flags: Vec::new(),
            recorded_at: None,
        };
        for r in recs {
            let b = r.backup.as_ref().filter(|b| exists(fs, b));
            let tm = r.tmp.as_ref().filter(|p| exists(fs, p));
            if b.is_none() && tm.is_none() {
                let _ = fs.remove_file(&r.flag);
                continue;
            }
            t.bak = t.bak.or_else(|| b.cloned());
            t.tmp = t.tmp.or_else(|| tm.cloned());
            t.flags.push(r.flag.clone());
            if let Some(at) = r.started_at
                && t.recorded_at.is_none_or(|c| newer(at, c))
            {
                t.recorded_at = Some(at);
            }
        }
        (t.bak.is_some() || t.tmp.is_some()).then_some(t)
    }
}

/// `tmp` が T のハードリンクか。FileId は FS によって 0 や同じ値を返し得る (Windows のネットワークのリダイレクタ・WebDAV・
/// `ReFS` など) ので、FileId の一致に加えて両方の `nlink >= 2` と、index が 0 でないことを求める (外れたら提案に回す)。
fn hard_link_of(fs: &dyn FileSystem, tmp: &FsPath, target: &FsPath) -> bool {
    match (fs.metadata(tmp), fs.metadata(target)) {
        (Ok(a), Ok(b)) => {
            a.nlink >= 2
                && b.nlink >= 2
                && a.file_id.is_some_and(|id| id.index != 0)
                && a.file_id == b.file_id
        }
        _ => false,
    }
}

/// 2 つのファイルの内容が同じか。どちらかが上限より大きい・読めないときは `false`。
fn identical(fs: &dyn FileSystem, a: &FsPath, b: &FsPath) -> bool {
    let (Ok(ma), Ok(mb)) = (fs.metadata(a), fs.metadata(b)) else {
        return false;
    };
    if ma.len != mb.len || ma.len > COMPARE_LIMIT {
        return false;
    }
    let read_all = |p: &FsPath| -> Option<Vec<u8>> {
        let mut r = fs.open_read(p).ok()?;
        let mut out = Vec::new();
        while let Some(chunk) = r.next_chunk().ok()? {
            out.extend_from_slice(chunk);
        }
        Some(out)
    };
    match (read_all(a), read_all(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// 提案への答えを実行する (ジョブ内)。終わったら、記録された痕跡が何も残っていなければ T のフラグを消す。
pub(crate) fn act(fs: &dyn FileSystem, l: &Leftover, op: LeftoverOp) -> Result<(), FsError> {
    let missing = || FsError::new(FsErrorKind::NotFound);
    match op {
        LeftoverOp::Restore => {
            let bak = l.bak.as_ref().ok_or_else(missing)?;
            fs.rename_no_replace(bak, &l.target)?;
        }
        LeftoverOp::DeleteBak => fs.remove_file(l.bak.as_ref().ok_or_else(missing)?)?,
        LeftoverOp::DeleteTmp => fs.remove_file(l.tmp.as_ref().ok_or_else(missing)?)?,
    }
    let left = [l.bak.as_ref(), l.tmp.as_ref()]
        .into_iter()
        .flatten()
        .any(|p| exists(fs, p));
    if !left {
        for f in &l.flags {
            let _ = fs.remove_file(f);
        }
    }
    Ok(())
}

/// どのフラグにも記録されていない書込途中の事前バックアップ (`*.bak.tmp`、書いたプロセスは動いていない) を消す。
/// 完全な写し (`*.bak`) は消さない (写しの元の T が名前から分からないため、残す)。
fn cleanup_partial_backups(fs: &dyn FileSystem, records: &[InflightRecord]) {
    let Ok(orphans) = fs.list_orphan_backups() else {
        return;
    };
    let referenced: BTreeSet<String> = records
        .iter()
        .filter_map(|r| r.backup.as_ref())
        .map(FsPath::display)
        .collect();
    for o in orphans.iter().filter(|o| o.partial) {
        let name = o.path.display();
        let final_name = name.strip_suffix(".tmp").unwrap_or(&name);
        if !referenced.contains(final_name) {
            let _ = fs.remove_file(&o.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::fs::{FileId, FileKind, FileMeta, FsKind, ReadChunks};
    use std::sync::Mutex;

    /// 名前ごとに `FileId` を持つだけの `FileSystem` (ハードリンク = 同じ `FileId` の 2 つの名前)。MemFs はハードリンクを作れない。
    struct Links {
        /// (名前, `FileId` の index, nlink)
        files: Mutex<Vec<(FsPath, u128, u32)>>,
        records: Vec<InflightRecord>,
    }

    impl Links {
        fn has(&self, p: &str) -> bool {
            let p = FsPath::from(p);
            self.files.lock().unwrap().iter().any(|(n, _, _)| *n == p)
        }
    }

    impl FileSystem for Links {
        fn metadata(&self, path: &FsPath) -> Result<FileMeta, FsError> {
            let files = self.files.lock().unwrap();
            let (_, id, nlink) = files
                .iter()
                .find(|(n, _, _)| n == path)
                .ok_or_else(|| FsError::new(FsErrorKind::NotFound))?;
            Ok(FileMeta {
                kind: FileKind::File,
                len: 1,
                mtime: FileTime::default(),
                readonly: false,
                file_id: Some(FileId {
                    volume: 1,
                    index: *id,
                }),
                nlink: *nlink,
                link: None,
                fs_kind: FsKind::Local,
                owner_is_me: Some(true),
            })
        }
        fn canonicalize(&self, path: &FsPath) -> Result<FsPath, FsError> {
            Ok(path.clone())
        }
        fn open_read(&self, _path: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
            Err(FsError::new(FsErrorKind::Other))
        }
        fn list_inflight(&self) -> Result<Vec<InflightRecord>, FsError> {
            Ok(self.records.clone())
        }
        fn remove_file(&self, path: &FsPath) -> Result<(), FsError> {
            self.files.lock().unwrap().retain(|(n, _, _)| n != path);
            Ok(())
        }
        fn write_atomic(
            &self,
            _path: &FsPath,
            _data: &mut dyn ReadChunks,
            _opts: &crate::io::fs::SaveOptions,
        ) -> Result<crate::io::fs::SaveReport, FsError> {
            Err(FsError::new(FsErrorKind::Unsupported))
        }
    }

    fn rec(method: SaveMethod, seq: u32) -> InflightRecord {
        InflightRecord {
            target: FsPath::from("/w/a.txt"),
            pid: 100,
            started_at: None,
            method,
            tmp: Some(FsPath::from(format!("/w/.a.txt.eve-tmp-{seq}").as_str())),
            backup: None,
            backup_recorded: false,
            backup_complete: false,
            flag: FsPath::from(format!("/s/{seq}.toml").as_str()),
            malformed: false,
        }
    }

    /// T (index 1) と一時ファイル 1 つ (index `tmp_id`、nlink `nlink`。T も同じ nlink)、記録 1 件 (`method`)。
    fn env(method: SaveMethod, tmp_id: u128, nlink: u32) -> Links {
        Links {
            files: Mutex::new(vec![
                (FsPath::from("/w/a.txt"), 1, nlink),
                (FsPath::from("/w/.a.txt.eve-tmp-1"), tmp_id, nlink),
                (FsPath::from("/s/1.toml"), 9, 1),
            ]),
            records: vec![rec(method, 1)],
        }
    }

    // SAF-40: T のハードリンクとして残った新規ファイルの一時ファイルは、提案せずに消してフラグも消す
    // (05 §4.6 の表の「T のハードリンクとして残った新規ファイルの tmp」の行)
    #[test]
    fn t_のハードリンクとして残った一時ファイルは確認なしで消す() {
        let fs = env(SaveMethod::NewFile, 1, 2);
        let actual = scan(&fs);
        assert!(actual.is_empty(), "{actual:?}");
        assert!(!fs.has("/w/.a.txt.eve-tmp-1"), "一時ファイルを消す");
        assert!(fs.has("/w/a.txt"), "T は残す");
        assert!(!fs.has("/s/1.toml"), "フラグを消す");
    }

    /// 消さずに「別名で開いて確認」の提案にする (一時ファイルもフラグも残る)。
    fn assert_offered(fs: &Links) {
        let l = scan(fs);
        assert_eq!(l.len(), 1, "{l:?}");
        assert_eq!(l[0].kind, LeftoverKind::Tmp);
        assert!(fs.has("/w/.a.txt.eve-tmp-1") && fs.has("/s/1.toml"));
    }

    #[test]
    fn 別の実体の一時ファイルは提案にする() {
        assert_offered(&env(SaveMethod::NewFile, 2, 2));
    }

    // FileId が一致しても nlink が 1 なら、FileId を信用できない FS (Windows のリダイレクタ・ReFS など) とみなして消さない
    #[test]
    fn file_id_が一致しても_nlink_が_1_なら消さずに提案() {
        assert_offered(&env(SaveMethod::NewFile, 1, 1));
    }

    // 既存の T の置換 (Atomic) の記録の一時ファイルは、置換の前の完全な新しい内容かもしれないので消さない
    #[test]
    fn 新規ファイル以外の記録の一時ファイルは消さない() {
        assert_offered(&env(SaveMethod::Atomic, 1, 2));
    }

    // FileId の index が 0 (FS が FileId を持たない) なら消さない
    #[test]
    fn file_id_の_index_が_0_なら消さない() {
        let fs = Links {
            files: Mutex::new(vec![
                (FsPath::from("/w/a.txt"), 0, 2),
                (FsPath::from("/w/.a.txt.eve-tmp-1"), 0, 2),
                (FsPath::from("/s/1.toml"), 9, 1),
            ]),
            records: vec![rec(SaveMethod::NewFile, 1)],
        };
        assert_offered(&fs);
    }

    // 同じ T の記録が 2 件 (SAF-40 の残りと、その後の保存の中断の一時ファイル) なら、消すのは 1 件目の一時ファイルとフラグだけで、
    // 2 件目の一時ファイルは提案を続ける (記録の無い隠しファイルにしない)
    #[test]
    fn 消した後も別の記録の一時ファイルは提案を続ける() {
        let fs = Links {
            files: Mutex::new(vec![
                (FsPath::from("/w/a.txt"), 1, 2),
                (FsPath::from("/w/.a.txt.eve-tmp-1"), 1, 2),
                (FsPath::from("/w/.a.txt.eve-tmp-2"), 3, 1),
                (FsPath::from("/s/1.toml"), 9, 1),
                (FsPath::from("/s/2.toml"), 9, 1),
            ]),
            records: vec![rec(SaveMethod::NewFile, 1), rec(SaveMethod::Atomic, 2)],
        };
        let l = scan(&fs);
        assert_eq!(l.len(), 1, "{l:?}");
        assert_eq!(l[0].kind, LeftoverKind::Tmp);
        assert_eq!(l[0].tmp, Some(FsPath::from("/w/.a.txt.eve-tmp-2")));
        assert_eq!(l[0].flags, vec![FsPath::from("/s/2.toml")]);
        assert!(!fs.has("/w/.a.txt.eve-tmp-1") && !fs.has("/s/1.toml"));
        assert!(fs.has("/w/.a.txt.eve-tmp-2") && fs.has("/s/2.toml"));
    }
}
