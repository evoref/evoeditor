//! 中断された保存の痕跡の写像 (docs/02 §4.5「中断の痕跡の写像」、05 §4.6)。開くジョブが `inflight_for` で得た記録を、
//! 保存先 T の有無とバックアップの実在から `InterruptedSave` にする。「報告しない」記録のうち、一時ファイル・退避名がどれも
//! 実在しないものは、フラグだけを黙って消す (一時ファイル・バックアップは消さない)。

use crate::fs::{FileSystem, InflightRecord, InterruptedSave, SaveMethod};
use crate::fspath::FsPath;

/// `<最終名>.tmp` (事前バックアップの書込中の名前)。
fn partial_name(backup: &FsPath) -> FsPath {
    let mut s = backup.as_path().as_os_str().to_owned();
    s.push(".tmp");
    FsPath::new(std::path::PathBuf::from(s))
}

fn exists(fs: &dyn FileSystem, p: &FsPath) -> bool {
    fs.metadata(p).is_ok()
}

/// 1 つの記録の判定の結果。`rank` は表の行の順 (小さいものが優先)。
struct Verdict {
    rank: u8,
    report: Option<InterruptedSave>,
}

fn judge(fs: &dyn FileSystem, r: &InflightRecord, t_exists: bool) -> Verdict {
    let tmp = r.tmp.clone().filter(|t| exists(fs, t));
    let none = |rank| Verdict { rank, report: None };
    let report = |rank, method, backup, tmp, complete| Verdict {
        rank,
        report: Some(InterruptedSave {
            method,
            backup,
            tmp,
            complete,
        }),
    };
    // 「上書きの可能性あり」: 解析できないフラグ、不正な組合せ、最終名が無い InPlace / true / true
    let suspicious = |tmp: Option<FsPath>| report(5, SaveMethod::InPlace, None, tmp, false);
    if r.malformed {
        return suspicious(tmp);
    }
    match r.method {
        SaveMethod::InPlace => match (r.backup_recorded, r.backup_complete) {
            (true, true) => match r.backup.as_ref().filter(|b| exists(fs, b)) {
                Some(b) => report(1, SaveMethod::InPlace, Some(b.clone()), tmp, true),
                None => suspicious(tmp),
            },
            (true, false) => {
                let partial = r
                    .backup
                    .as_ref()
                    .map(partial_name)
                    .filter(|p| exists(fs, p));
                match partial {
                    Some(p) => report(2, SaveMethod::InPlace, Some(p), tmp, false),
                    // バックアップの途中で中断して一時名も無い: 上書きは始まっておらず、示す痕跡も無い
                    None => none(3),
                }
            }
            (false, false) => report(4, SaveMethod::InPlace, None, None, false),
            (false, true) => suspicious(tmp),
        },
        SaveMethod::Atomic | SaveMethod::NewFile if r.backup_recorded || r.backup_complete => {
            suspicious(tmp)
        }
        SaveMethod::Atomic if !t_exists => match r.backup.as_ref().filter(|b| exists(fs, b)) {
            Some(b) => report(6, SaveMethod::Atomic, Some(b.clone()), tmp, true),
            None => none(7),
        },
        SaveMethod::NewFile if !t_exists => report(9, SaveMethod::NewFile, None, tmp, false),
        // T が在る Atomic / NewFile: T は置換・作成の前の版か後の版のどちらかで完全
        _ => none(8),
    }
}

/// 記録 × T の有無 → `InterruptedSave`。報告しない記録のフラグを掃除する。報告する記録は消さない。
pub(crate) fn map_interrupted(
    fs: &dyn FileSystem,
    records: &[InflightRecord],
    t_exists: bool,
) -> Option<InterruptedSave> {
    let mut best: Option<Verdict> = None;
    for r in records {
        let v = judge(fs, r, t_exists);
        if v.report.is_none() {
            // 一時ファイル・退避名のどれかが実在すれば、フラグを残す (v0.2 の起動時の走査の手掛かり)
            let traces = [r.tmp.as_ref(), r.backup.as_ref()]
                .into_iter()
                .flatten()
                .any(|p| exists(fs, p));
            if !traces {
                let _ = fs.remove_file(&r.flag);
            }
        } else if best.as_ref().is_none_or(|b| v.rank < b.rank) {
            best = Some(v);
        }
    }
    best.and_then(|v| v.report)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    use super::*;
    use crate::fs::{
        FileKind, FileMeta, FileTime, FsError, FsErrorKind, FsKind, ReadChunks, SaveOptions,
        SaveReport,
    };

    /// 実在するパスだけを持つ最小の `FileSystem`。
    struct Fake {
        files: BTreeSet<FsPath>,
        removed: Mutex<Vec<FsPath>>,
    }

    impl FileSystem for Fake {
        fn metadata(&self, p: &FsPath) -> Result<FileMeta, FsError> {
            if self.files.contains(p) {
                Ok(FileMeta {
                    kind: FileKind::File,
                    len: 0,
                    mtime: FileTime::default(),
                    readonly: false,
                    file_id: None,
                    nlink: 1,
                    link: None,
                    fs_kind: FsKind::Local,
                    owner_is_me: None,
                })
            } else {
                Err(FsError::new(FsErrorKind::NotFound))
            }
        }

        fn canonicalize(&self, p: &FsPath) -> Result<FsPath, FsError> {
            Ok(p.clone())
        }

        fn open_read(&self, _: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
            Err(FsError::new(FsErrorKind::Other))
        }

        fn write_atomic(
            &self,
            _: &FsPath,
            _: &mut dyn ReadChunks,
            _: &SaveOptions,
        ) -> Result<SaveReport, FsError> {
            Err(FsError::new(FsErrorKind::Other))
        }

        fn remove_file(&self, p: &FsPath) -> Result<(), FsError> {
            self.removed.lock().unwrap().push(p.clone());
            Ok(())
        }
    }

    fn fake(files: &[&str]) -> Fake {
        Fake {
            files: files.iter().map(|f| FsPath::from(*f)).collect(),
            removed: Mutex::new(Vec::new()),
        }
    }

    fn removed(f: &Fake) -> usize {
        f.removed.lock().unwrap().len()
    }

    fn rec(method: SaveMethod, recorded: bool, complete: bool) -> InflightRecord {
        InflightRecord {
            target: FsPath::from("/w/a.txt"),
            pid: 1,
            started_at: None,
            method,
            tmp: Some(FsPath::from("/w/.a.txt.eve-tmp-1-aaaaaaaa")),
            backup: Some(FsPath::from("/b/x.bak")),
            backup_recorded: recorded,
            backup_complete: complete,
            flag: FsPath::from("/s/1-1.toml"),
            malformed: false,
        }
    }

    #[test]
    fn 直接上書きの完全なバックアップ() {
        let f = fake(&["/b/x.bak"]);
        let r = map_interrupted(&f, &[rec(SaveMethod::InPlace, true, true)], true).unwrap();
        assert_eq!(r.method, SaveMethod::InPlace);
        assert!(r.complete);
        assert_eq!(r.backup, Some(FsPath::from("/b/x.bak")));
        assert!(r.tmp.is_none(), "実在しない一時ファイルは入れない");
    }

    #[test]
    fn バックアップの途中は一時名を案内する() {
        let f = fake(&["/b/x.bak.tmp"]);
        let r = map_interrupted(&f, &[rec(SaveMethod::InPlace, true, false)], true).unwrap();
        assert!(!r.complete);
        assert_eq!(r.backup, Some(FsPath::from("/b/x.bak.tmp")));
        // 一時名も無ければ報告せず、フラグを掃除する
        let f = fake(&[]);
        assert!(map_interrupted(&f, &[rec(SaveMethod::InPlace, true, false)], true).is_none());
        assert_eq!(removed(&f), 1);
    }

    #[test]
    fn バックアップなしの承認の後の中断() {
        let f = fake(&[]);
        let r = map_interrupted(&f, &[rec(SaveMethod::InPlace, false, false)], true).unwrap();
        assert_eq!((r.backup, r.complete), (None, false));
    }

    #[test]
    fn 上書きの可能性ありは安全側で報告しフラグを残す() {
        let f = fake(&[]); // 最終名が無い
        let r = map_interrupted(&f, &[rec(SaveMethod::InPlace, true, true)], true).unwrap();
        assert_eq!(
            (r.method, r.backup, r.complete),
            (SaveMethod::InPlace, None, false)
        );
        assert_eq!(removed(&f), 0);
        // 不正な組合せ・解析できないフラグ
        let bad = rec(SaveMethod::Atomic, true, false);
        assert!(map_interrupted(&f, &[bad], false).is_some());
        let mut m = rec(SaveMethod::Atomic, false, false);
        m.malformed = true;
        assert!(map_interrupted(&f, &[m], true).is_some());
    }

    #[test]
    fn 原子的保存の置換の途中は退避名を案内する() {
        let f = fake(&["/b/x.bak"]);
        let r = map_interrupted(&f, &[rec(SaveMethod::Atomic, false, false)], false).unwrap();
        assert_eq!((r.method, r.complete), (SaveMethod::Atomic, true));
        assert_eq!(r.backup, Some(FsPath::from("/b/x.bak")));
        // 退避名が無ければ報告しない (T は外部で消えた)
        let f = fake(&[]);
        assert!(map_interrupted(&f, &[rec(SaveMethod::Atomic, false, false)], false).is_none());
    }

    #[test]
    fn t_が在る原子的保存と新規は報告しない_痕跡が無ければフラグを消す() {
        let f = fake(&[]);
        assert!(map_interrupted(&f, &[rec(SaveMethod::Atomic, false, false)], true).is_none());
        assert!(map_interrupted(&f, &[rec(SaveMethod::NewFile, false, false)], true).is_none());
        assert_eq!(removed(&f), 2);
        // 一時ファイルが実在するなら、報告せずフラグを残す
        let f = fake(&["/w/.a.txt.eve-tmp-1-aaaaaaaa"]);
        assert!(map_interrupted(&f, &[rec(SaveMethod::Atomic, false, false)], true).is_none());
        assert_eq!(removed(&f), 0);
    }

    #[test]
    fn 新規ファイルの中断() {
        let f = fake(&["/w/.a.txt.eve-tmp-1-aaaaaaaa"]);
        let r = map_interrupted(&f, &[rec(SaveMethod::NewFile, false, false)], false).unwrap();
        assert_eq!((r.method, r.complete), (SaveMethod::NewFile, false));
        assert!(r.tmp.is_some());
    }

    #[test]
    fn 複数の記録は表の上の行を採る() {
        let f = fake(&["/b/x.bak"]);
        let recs = [
            rec(SaveMethod::NewFile, false, false),
            rec(SaveMethod::InPlace, true, true),
        ];
        let r = map_interrupted(&f, &recs, false).unwrap();
        assert_eq!(r.method, SaveMethod::InPlace);
    }
}
