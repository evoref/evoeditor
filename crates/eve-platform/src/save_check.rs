//! 保存の事後の確かめ (docs/05 §4.1 手順 7): 置換・直接上書きの後の T が自分の書いたものかを決め、`new_stamp` を作る。
//! OS に依らない (Windows の `save`・Unix の `unix_fs`・両方の `inplace` が呼ぶ)。
//!
//! 置換: (a) 置換をまたいで識別が保たれる FS (05 §5.4) か更新時刻を控えられなかったとき、事後の `FileId` と長さが控えた
//! 値と等しい → 自分の書いたもの。(b) 保たれない FS (FAT 系・ネットワークなど。識別は使い回される) で更新時刻を控えられた
//! とき、または `FileId` を控えられなかったときは、長さと更新時刻が等しい → 自分の書いたもの (識別が同じでも更新時刻が
//! 違えば自分のものとしない)。(c) それ以外 → 確かめられない (自分の書いた値を返し、呼び側が `ChangedAfterSave` を積む)。
//! 直接上書き: 改名しないので識別はどの FS でも変わらない。(a) と、保たれない FS では (b) の両方を求める。
//! 識別をハンドルで取れずパスで取り直したとき (別のファイルを掴み得る) は、保たれない FS と同じに扱う (更新時刻も比べる)。
//! 置換の後に事後の metadata を取れないとき (`unverified`) も確かめられない扱いで、保存は成功のまま自分の書いた値を返す。

use eve_core::{FileId, FileMeta, FileStamp, FileTime, SaveWarning};

/// T の FS で、置換 (`ReplaceFileW`・`MoveFileExW`・`rename`・`link`) をまたいで一時ファイルの識別が保たれるか
/// (05 §5.4 の「置換をまたいで識別が保たれる FS」)。`FileMeta.file_id_unique` (別のファイルと重ならないか) とは別の性質。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdAcrossReplace {
    /// 保たれる (NTFS / `ReFS` のローカルのボリューム、Linux のローカルの FS (FAT 系を除く))。
    Kept,
    /// 変わり得る、または分からない (FAT 系・ネットワーク・FUSE・9p・判定の失敗)。識別は使い回され得るので、更新時刻も比べる。
    MayChange,
}

/// 自分の書いたファイルが T になった経路。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Via {
    /// 一時ファイルの置換 (新規ファイルの移動を含む)。保たれない FS では識別が変わる。
    Replace,
    /// 直接上書き。改名しないので、自分のファイルの識別はどの FS でも変わらない。
    InPlace,
}

/// 控えた識別の出どころ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdSource {
    /// 書いたハンドルで取った (自分のファイルの識別)。
    Handle,
    /// ハンドルで取れず、パスで取り直した (閉じた後に差し替えられていれば別のファイルの識別)。
    Reread,
}

/// 手順 3b で控えた、自分の書いたファイルの値 (05 §4.1 手順 7 の「控える値」)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Written {
    /// 識別 (取れなければ `None`)。
    pub file_id: Option<FileId>,
    /// `file_id` の出どころ。
    pub id_source: IdSource,
    /// 書いたバイト数。
    pub len: u64,
    /// 閉じた後にパスで取り直した更新時刻 (取れない・別のファイルだったときは `None`)。
    pub mtime: Option<FileTime>,
    /// T になった経路。
    pub via: Via,
}

impl Written {
    /// 置換の一時ファイルの、閉じる前にハンドルで控えた値 (識別と書いた長さ)。更新時刻は `with_reread` で足す。
    pub(crate) fn replaced(file_id: Option<FileId>, len: u64) -> Written {
        Written {
            file_id,
            id_source: IdSource::Handle,
            len,
            mtime: None,
            via: Via::Replace,
        }
    }

    /// 直接上書きの、開いたハンドルで控えた値。
    pub(crate) fn in_place(file_id: Option<FileId>, len: u64) -> Written {
        Written {
            via: Via::InPlace,
            ..Written::replaced(file_id, len)
        }
    }

    /// ハンドルで識別を取れなかったときに、パスで取った識別 `fallback` に退く (出どころは `Reread`)。
    #[must_use]
    pub(crate) fn or_reread_id(self, fallback: Option<FileId>) -> Written {
        match (self.file_id, fallback) {
            (None, Some(id)) => Written {
                file_id: Some(id),
                id_source: IdSource::Reread,
                ..self
            },
            _ => self,
        }
    }

    /// 閉じた後にパスで取り直したメタ情報から更新時刻を控える。取り直した長さ (と、控えた識別があれば識別) が控えた値と
    /// 同じときだけ (別のファイルの値を控えない)。識別を控えられなかったときは、取り直した識別にも退く (`or_reread_id`)。
    #[must_use]
    pub(crate) fn with_reread(self, reread: Option<&FileMeta>) -> Written {
        let Some(m) = reread
            .filter(|m| m.len == self.len && self.file_id.is_none_or(|id| m.file_id == Some(id)))
        else {
            return self;
        };
        Written {
            mtime: Some(m.mtime),
            ..self.or_reread_id(m.file_id)
        }
    }
}

/// 事後の確かめの結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub(crate) enum Verdict {
    /// 自分の書いたもの: 事後の値。
    Mine(FileStamp),
    /// 確かめられない: 自分の書いた値。`verify` の (c) では控えられなかった欄は事後の値で、ディスクの T の
    /// (mtime, len, `FileId`) のどれかと必ず違う (次の保存の照合は、中身が同じでなければ Conflict になる)。
    /// `unverified` (事後の metadata を取れない) ではディスクの T と比べていない (次の保存の照合が確かめる)。
    Unconfirmed(FileStamp),
}

impl Verdict {
    /// `new_stamp` にする値。確かめられなければ `ChangedAfterSave` を積む。
    pub(crate) fn into_stamp(self, warnings: &mut Vec<SaveWarning>) -> FileStamp {
        match self {
            Verdict::Mine(s) => s,
            Verdict::Unconfirmed(s) => {
                warnings.push(SaveWarning::ChangedAfterSave);
                s
            }
        }
    }
}

/// 事後の metadata(T) `after` が、控えた `written` の自分の書いたものか (05 §4.1 手順 7 の (a)〜(c))。
pub(crate) fn verify(written: &Written, after: &FileMeta, ids: IdAcrossReplace) -> Verdict {
    // パスで取り直した識別は別のファイルのものかもしれないので、保たれない FS と同じに扱う (更新時刻も比べる)
    let ids = match written.id_source {
        IdSource::Handle => ids,
        IdSource::Reread => IdAcrossReplace::MayChange,
    };
    let same_len = after.len == written.len;
    let same_mtime = |m: FileTime| after.mtime == m;
    let mine = match (written.file_id, written.mtime, written.via) {
        // 識別も更新時刻も控えられなかった: 長さだけ
        (None, None, _) => same_len,
        // (b) 識別を控えられなかった: 長さと更新時刻
        (None, Some(m), _) => same_len && same_mtime(m),
        // 直接上書き: 識別と長さを必ず。保たれない FS (識別が使い回される) では更新時刻も
        (Some(id), m, Via::InPlace) => {
            same_len
                && after.file_id == Some(id)
                && (ids == IdAcrossReplace::Kept || m.is_none_or(same_mtime))
        }
        // (b) 置換で保たれない FS: 識別は変わる・使い回されるので、長さと更新時刻
        (Some(_), Some(m), Via::Replace) if ids == IdAcrossReplace::MayChange => {
            same_len && same_mtime(m)
        }
        // (a) 置換で保たれる FS か、更新時刻を控えられなかった: 識別と長さ
        (Some(id), _, Via::Replace) => same_len && after.file_id == Some(id),
    };
    if mine {
        Verdict::Mine(FileStamp::from_meta(after, None))
    } else {
        Verdict::Unconfirmed(FileStamp {
            mtime: written.mtime.unwrap_or(after.mtime),
            len: written.len,
            file_id: written.file_id.or(after.file_id),
            hash: None,
        })
    }
}

/// 原子的保存の置換 (新規ファイルの移動を含む) の後の確かめ。`after` は事後の metadata(T) (取れなければ `None`)。
/// 取れたら `verify`、取れなければ `unverified` (どちらも保存は成功のまま。Windows の `save` と Unix の `unix_fs` が呼ぶ)。
///
/// spec: 05 §4.1
pub(crate) fn after_replace(
    written: &Written,
    after: Option<&FileMeta>,
    ids: IdAcrossReplace,
) -> Verdict {
    match after {
        Some(after) => verify(written, after, ids),
        None => unverified(written),
    }
}

/// 置換に成功した後に事後の metadata(T) を取れなかったときの値 (確かめられない)。自分の書いた値 (控えた `FileId`・len・
/// mtime) で、控えられなかった欄は `FileId` を `None`、mtime を [`FileTime::MIN`] (照合で必ず不一致になる目印) にする。
/// 呼び側は保存を成功のまま返し、後片付けも確かめられたときと同じに行う (直接上書きの同じ場面 (SAF-20) と揃える)。
///
/// spec: 05 §4.1
fn unverified(written: &Written) -> Verdict {
    Verdict::Unconfirmed(FileStamp {
        mtime: written.mtime.unwrap_or(FileTime::MIN),
        len: written.len,
        file_id: written.file_id,
        hash: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eve_core::{FileKind, FsKind};

    use IdAcrossReplace::{Kept, MayChange};

    const ID_TMP: FileId = FileId {
        volume: 7,
        index: 100,
    };
    const ID_OTHER: FileId = FileId {
        volume: 7,
        index: 200,
    };
    const T0: FileTime = FileTime {
        secs: 1_800_000_000,
        nanos: 0,
    };
    const T1: FileTime = FileTime {
        secs: 1_800_000_002,
        nanos: 0,
    };

    fn meta(file_id: Option<FileId>, len: u64, mtime: FileTime) -> FileMeta {
        FileMeta {
            kind: FileKind::File,
            len,
            mtime,
            readonly: false,
            file_id,
            nlink: 1,
            link: None,
            fs_kind: FsKind::Local,
            owner_is_me: None,
            offline: false,
            file_id_unique: false,
        }
    }

    /// 置換の、ハンドルで控えた値。
    fn written(file_id: Option<FileId>, len: u64, mtime: Option<FileTime>) -> Written {
        Written {
            mtime,
            ..Written::replaced(file_id, len)
        }
    }

    /// 確かめられないときの値は、ディスクの T (`after`) の (mtime, len, `FileId`) のどれかと違う (Port の照合 (`matches`) で一致しない)。
    fn assert_differs_from_disk(v: Verdict, after: &FileMeta) {
        let Verdict::Unconfirmed(s) = v else {
            panic!("確かめられないはず: {v:?}");
        };
        let disk = FileStamp::from_meta(after, None);
        assert!(
            s.mtime != disk.mtime || s.len != disk.len || s.file_id != disk.file_id,
            "{s:?} と {disk:?} が一致する"
        );
    }

    #[test]
    fn a_保たれる_fs_では識別と長さが同じなら事後の値() {
        // 更新時刻が違っても (a) で決まる
        let w = written(Some(ID_TMP), 5, Some(T0));
        let after = meta(Some(ID_TMP), 5, T1);
        assert_eq!(
            verify(&w, &after, Kept),
            Verdict::Mine(FileStamp::from_meta(&after, None))
        );
        // 保たれない FS でも、更新時刻を控えられなかったときは (a)
        let w = written(Some(ID_TMP), 5, None);
        assert_eq!(
            verify(&w, &after, MayChange),
            Verdict::Mine(FileStamp::from_meta(&after, None))
        );
    }

    #[test]
    fn 保たれない_fs_では識別と長さが同じでも更新時刻が違えば確かめられない() {
        // FAT 系の識別はディレクトリの項目の位置で使い回される: 置換の直後に他者が同じ長さの T を作り、空いた位置に入ると
        // 識別が同じになる。更新時刻を控えられたら、それも満たすときだけ自分のものとする (05 §4.1 手順 7 の (b))
        let w = written(Some(ID_TMP), 5, Some(T0));
        let after = meta(Some(ID_TMP), 5, T1);
        let v = verify(&w, &after, MayChange);
        assert_eq!(
            v,
            Verdict::Unconfirmed(FileStamp {
                mtime: T0,
                len: 5,
                file_id: Some(ID_TMP),
                hash: None
            })
        );
        assert_differs_from_disk(v, &after);
    }

    #[test]
    fn a_識別が同じでも長さが違えば確かめられない() {
        let w = written(Some(ID_TMP), 5, Some(T0));
        let after = meta(Some(ID_TMP), 6, T0);
        for ids in [Kept, MayChange] {
            let v = verify(&w, &after, ids);
            assert_eq!(
                v,
                Verdict::Unconfirmed(FileStamp {
                    mtime: T0,
                    len: 5,
                    file_id: Some(ID_TMP),
                    hash: None
                }),
                "{ids:?}"
            );
            assert_differs_from_disk(v, &after);
        }
    }

    #[test]
    fn b_保たれない_fs_では識別が違っても長さと更新時刻が同じなら事後の値() {
        let w = written(Some(ID_TMP), 5, Some(T0));
        let after = meta(Some(ID_OTHER), 5, T0);
        assert_eq!(
            verify(&w, &after, MayChange),
            Verdict::Mine(FileStamp::from_meta(&after, None))
        );
    }

    #[test]
    fn b_保たれない_fs_でも長さか更新時刻が違えば確かめられない() {
        let w = written(Some(ID_TMP), 5, Some(T0));
        for after in [meta(Some(ID_OTHER), 5, T1), meta(Some(ID_OTHER), 6, T0)] {
            let v = verify(&w, &after, MayChange);
            assert_eq!(
                v,
                Verdict::Unconfirmed(FileStamp {
                    mtime: T0,
                    len: 5,
                    file_id: Some(ID_TMP),
                    hash: None
                }),
                "{after:?}"
            );
            assert_differs_from_disk(v, &after);
        }
    }

    #[test]
    fn c_保たれる_fs_では識別が違えば長さと更新時刻が同じでも確かめられない() {
        let w = written(Some(ID_TMP), 5, Some(T0));
        let after = meta(Some(ID_OTHER), 5, T0);
        let v = verify(&w, &after, Kept);
        assert_eq!(
            v,
            Verdict::Unconfirmed(FileStamp {
                mtime: T0,
                len: 5,
                file_id: Some(ID_TMP),
                hash: None
            })
        );
        assert_differs_from_disk(v, &after);
    }

    #[test]
    fn 保たれない_fs_で更新時刻を控えられなければ確かめられない() {
        let w = written(Some(ID_TMP), 5, None);
        let after = meta(Some(ID_OTHER), 5, T0);
        let v = verify(&w, &after, MayChange);
        // 更新時刻は事後の値で埋めるが、識別が自分のものなのでディスクとは一致しない
        assert_eq!(
            v,
            Verdict::Unconfirmed(FileStamp {
                mtime: T0,
                len: 5,
                file_id: Some(ID_TMP),
                hash: None
            })
        );
        assert_differs_from_disk(v, &after);
    }

    #[test]
    fn 識別を控えられなければ保たれる_fs_でも長さと更新時刻で確かめる() {
        let w = written(None, 5, Some(T0));
        let after = meta(Some(ID_OTHER), 5, T0);
        assert_eq!(
            verify(&w, &after, Kept),
            Verdict::Mine(FileStamp::from_meta(&after, None))
        );
        let after = meta(Some(ID_OTHER), 5, T1);
        let v = verify(&w, &after, Kept);
        assert_eq!(
            v,
            Verdict::Unconfirmed(FileStamp {
                mtime: T0,
                len: 5,
                file_id: Some(ID_OTHER),
                hash: None
            })
        );
        assert_differs_from_disk(v, &after);
    }

    #[test]
    fn 何も控えられなければ長さだけで確かめる() {
        let w = written(None, 5, None);
        for ids in [Kept, MayChange] {
            let after = meta(Some(ID_OTHER), 5, T1);
            assert_eq!(
                verify(&w, &after, ids),
                Verdict::Mine(FileStamp::from_meta(&after, None)),
                "{ids:?}"
            );
            let after = meta(Some(ID_OTHER), 9, T1);
            let v = verify(&w, &after, ids);
            assert_eq!(
                v,
                Verdict::Unconfirmed(FileStamp {
                    mtime: T1,
                    len: 5,
                    file_id: Some(ID_OTHER),
                    hash: None
                }),
                "{ids:?}"
            );
            assert_differs_from_disk(v, &after);
        }
    }

    #[test]
    fn 取り直したファイルが控えた値と違えば更新時刻を控えない() {
        let w = Written::replaced(Some(ID_TMP), 5);
        assert_eq!(
            w.with_reread(Some(&meta(Some(ID_TMP), 5, T1))),
            written(Some(ID_TMP), 5, Some(T1))
        );
        assert_eq!(
            w.with_reread(Some(&meta(Some(ID_OTHER), 5, T1))),
            w,
            "別の識別"
        );
        assert_eq!(
            w.with_reread(Some(&meta(Some(ID_TMP), 6, T1))),
            w,
            "別の長さ"
        );
        assert_eq!(w.with_reread(None), w, "取り直せなかった");
    }

    #[test]
    fn 閉じる前に識別を控えられなければ取り直した識別に退く() {
        let w = Written::replaced(None, 5);
        assert_eq!(
            w.with_reread(Some(&meta(Some(ID_OTHER), 5, T1))),
            Written {
                id_source: IdSource::Reread,
                ..written(Some(ID_OTHER), 5, Some(T1))
            }
        );
        // 直接上書きの手順 1 の識別に退くときも出どころは `Reread`
        let w = Written::in_place(None, 5).or_reread_id(Some(ID_TMP));
        assert_eq!((w.file_id, w.id_source), (Some(ID_TMP), IdSource::Reread));
        // ハンドルで取れていれば退かない
        let w = Written::in_place(Some(ID_TMP), 5).or_reread_id(Some(ID_OTHER));
        assert_eq!((w.file_id, w.id_source), (Some(ID_TMP), IdSource::Handle));
        assert_eq!(
            w.with_reread(Some(&meta(Some(ID_OTHER), 6, T1))),
            w,
            "長さが違えば何も控えない"
        );
    }

    #[test]
    fn 取り直した識別は保たれる_fs_でも更新時刻を比べる() {
        // 閉じた後に一時ファイルが差し替えられていると、取り直した識別と更新時刻は相手のもの。保たれる FS でも識別だけで
        // 決めず、更新時刻も比べる (保たれない FS と同じ (b))
        let w = Written::replaced(None, 5).with_reread(Some(&meta(Some(ID_TMP), 5, T0)));
        let v = verify(&w, &meta(Some(ID_TMP), 5, T1), Kept);
        assert_differs_from_disk(v, &meta(Some(ID_TMP), 5, T1));
        assert_eq!(
            verify(&w, &meta(Some(ID_OTHER), 5, T0), Kept),
            Verdict::Mine(FileStamp::from_meta(&meta(Some(ID_OTHER), 5, T0), None)),
            "(b): 長さと更新時刻"
        );
    }

    #[test]
    fn 直接上書きは識別と長さを必ず比べ_保たれない_fs_では更新時刻も比べる() {
        let w = Written {
            mtime: Some(T0),
            ..Written::in_place(Some(ID_TMP), 5)
        };
        let mine = |after: &FileMeta| Verdict::Mine(FileStamp::from_meta(after, None));
        // 自分のファイル (識別は変わらない)
        let same = meta(Some(ID_TMP), 5, T0);
        assert_eq!(verify(&w, &same, Kept), mine(&same));
        assert_eq!(verify(&w, &same, MayChange), mine(&same));
        // 他者が同じ長さ・同じ更新時刻のファイルを被せた: 識別の違いで見つける (保たれない FS でも)
        let other = meta(Some(ID_OTHER), 5, T0);
        for ids in [Kept, MayChange] {
            assert_differs_from_disk(verify(&w, &other, ids), &other);
        }
        // 識別が同じで更新時刻が違う: 保たれる FS では自分のもの、保たれない FS (識別が使い回される) では確かめられない
        let touched = meta(Some(ID_TMP), 5, T1);
        assert_eq!(verify(&w, &touched, Kept), mine(&touched));
        assert_differs_from_disk(verify(&w, &touched, MayChange), &touched);
        // 更新時刻を控えられなかったときは識別と長さ
        let w = Written::in_place(Some(ID_TMP), 5);
        assert_eq!(verify(&w, &touched, MayChange), mine(&touched));
        assert_differs_from_disk(verify(&w, &other, MayChange), &other);
    }

    #[test]
    fn 事後の_metadata_を取れなければ控えた値で_控えられなかった欄は_none_と最小値() {
        // 控えた識別・長さ・更新時刻をそのまま使う (FS の性質に依らない)
        let w = written(Some(ID_TMP), 5, Some(T1));
        let want = Verdict::Unconfirmed(FileStamp {
            mtime: T1,
            len: 5,
            file_id: Some(ID_TMP),
            hash: None,
        });
        assert_eq!(after_replace(&w, None, Kept), want);
        assert_eq!(after_replace(&w, None, MayChange), want);
        // 識別も更新時刻も控えられなかった: 識別は None、更新時刻は最小値 (照合で必ず不一致になる目印)
        let w = written(None, 5, None);
        let v = after_replace(&w, None, Kept);
        assert_eq!(
            v,
            Verdict::Unconfirmed(FileStamp {
                mtime: FileTime::MIN,
                len: 5,
                file_id: None,
                hash: None
            })
        );
        // 最小値は実在のファイルの値と一致しない: 長さが同じでも照合 (mtime, len, FileId) で止まる
        assert_differs_from_disk(v, &meta(None, 5, T0));
        // 成功のまま返す値には ChangedAfterSave を積む
        let mut warnings = Vec::new();
        let _ = v.into_stamp(&mut warnings);
        assert_eq!(warnings, [SaveWarning::ChangedAfterSave]);
        // 取れたときは verify と同じ
        let after = meta(Some(ID_TMP), 5, T0);
        let w = written(Some(ID_TMP), 5, Some(T0));
        assert_eq!(
            after_replace(&w, Some(&after), Kept),
            verify(&w, &after, Kept)
        );
    }

    #[test]
    fn 確かめられないときは警告を積む() {
        let s = FileStamp::from_meta(&meta(Some(ID_TMP), 5, T0), None);
        let mut w = Vec::new();
        assert_eq!(Verdict::Mine(s).into_stamp(&mut w), s);
        assert_eq!(w, Vec::<SaveWarning>::new());
        assert_eq!(Verdict::Unconfirmed(s).into_stamp(&mut w), s);
        assert_eq!(w, [SaveWarning::ChangedAfterSave]);
    }
}

/// 置換・直接上書きの直後の本当の割り込み (T を他のファイルで置き換える) を、実際の FS で見つけること (05 §4.1 手順 7)。
/// 割り込みは呼び出し点 `after_replace` / `after_in_place_write` (`hooks::on_point`) で起こす。
#[cfg(all(test, any(windows, target_os = "linux")))]
mod on_disk {
    use std::cell::Cell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::time::{Duration, SystemTime};

    use eve_core::{
        CancelToken, ChunkError, Expect, FileStamp, FileSystem, FsErrorKind, FsPath, ReadChunks,
        SaveOptions, SaveWarning,
    };

    use super::IdAcrossReplace;
    use crate::hooks;

    #[cfg(windows)]
    type NativeFs = crate::fs::WinFs;
    #[cfg(unix)]
    type NativeFs = crate::unix_fs::UnixFs;

    /// 作業ディレクトリ (終わりに消す)。
    struct Dir(PathBuf);

    impl Dir {
        fn new(base: &Path, tag: &str) -> Dir {
            static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let d = base.join(format!("eve-clobber-{tag}-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&d);
            fs::create_dir_all(&d).expect("作業ディレクトリ");
            Dir(d)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Bytes<'a>(Option<&'a [u8]>);

    impl ReadChunks for Bytes<'_> {
        fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
            Ok(self.0.take())
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Method {
        /// 既存の T の置換 (`ReplaceFileW` / `rename`)。
        Replace,
        /// 新規ファイル (`MoveFileExW` / `renameat2`・`link` の退路)。
        NewFile,
        /// 直接上書き。
        InPlace,
    }

    /// 被せるファイルの更新時刻。
    #[derive(Clone, Copy, Debug)]
    enum Mtime {
        /// 過去 (2001 年)。更新時刻だけでも見つかる。
        Past,
        /// 被せる直前の T (自分の書いたもの) と同じ。識別か長さでしか見つからない。
        Same,
    }

    /// T を他のファイルで置き換える: 同じディレクトリに書き、更新時刻を `mtime` にして、改名で被せる。
    fn clobber(t: &Path, body: &[u8], mtime: Mtime) {
        let at = match mtime {
            Mtime::Past => SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000),
            Mtime::Same => fs::metadata(t)
                .and_then(|m| m.modified())
                .expect("T の更新時刻"),
        };
        let other = t.with_extension("other");
        fs::write(&other, body).expect("他のファイル");
        fs::OpenOptions::new()
            .write(true)
            .open(&other)
            .and_then(|f| f.set_modified(at))
            .expect("更新時刻");
        fs::rename(&other, t).expect("被せる");
    }

    const OURS: &[u8] = b"ours: 0123456789\n";

    /// `method` で保存する途中 (置換・書込の直後) に T を `other` で置き換える。`detected` なら `ChangedAfterSave` が出て、
    /// `new_stamp` が自分の書いた値になり、次の保存が Conflict になることを確かめる。`detected` でなければ (既知の制約。
    /// 05 §4.1 手順 7 の (b))、警告が出ず `new_stamp` が他者の値になることを確かめる。新規ファイルを保存できない FS
    /// (WSL の `DrvFs`: link の退路が EPERM) では確かめない。
    fn check(base: &Path, method: Method, other: &'static [u8], mtime: Mtime, detected: bool) {
        let what = format!("{method:?}・{} バイト・{mtime:?}", other.len());
        let d = Dir::new(base, "c");
        let fsys = NativeFs::new(d.0.join("state"));
        let t = d.0.join("a.txt");
        let p = FsPath::new(t.clone());
        let expected = match method {
            Method::NewFile => Expect::Absent,
            Method::Replace | Method::InPlace => {
                fs::write(&t, b"old").expect("元のファイル");
                Expect::Stamp(FileStamp::from_meta(
                    &fsys.metadata(&p).expect("metadata"),
                    None,
                ))
            }
        };
        let point = match method {
            Method::InPlace => "after_in_place_write",
            Method::Replace | Method::NewFile => "after_replace",
        };
        let ran = Rc::new(Cell::new(false));
        {
            let (t, ran) = (t.clone(), Rc::clone(&ran));
            hooks::on_point(point, move || {
                clobber(&t, other, mtime);
                ran.set(true);
            });
        }
        let opts = SaveOptions::user_file(expected);
        let r = match method {
            Method::InPlace => fsys.write_in_place(
                &p,
                &mut Bytes(Some(OURS)),
                OURS.len() as u64,
                &opts,
                &CancelToken::new(),
            ),
            Method::Replace | Method::NewFile => {
                fsys.write_atomic(&p, &mut Bytes(Some(OURS)), &opts)
            }
        };
        let r = match r {
            Ok(r) => r,
            Err(e) if cfg!(unix) && e.kind == FsErrorKind::PermissionDenied => {
                // 新規ファイルを保存できない FS: 積んだ割り込みを走らせずに捨てて終える
                assert!(matches!(method, Method::NewFile), "{what}: {e:?}");
                hooks::forget_point(point);
                return;
            }
            Err(e) => panic!("{what}: {e:?}"),
        };
        assert!(ran.get(), "{what}: 割り込みが起きていない");
        assert_eq!(fs::read(&t).expect("読む"), other, "{what}");
        let disk = FileStamp::from_meta(&fsys.metadata(&p).expect("metadata"), None);
        if !detected {
            assert!(
                !r.warnings.contains(&SaveWarning::ChangedAfterSave),
                "{what}: 見分けられないはず (既知の制約): {:?}",
                r.warnings
            );
            assert_eq!(r.new_stamp, disk, "{what}: 他者の値を自分のものとする");
            return;
        }
        assert!(
            r.warnings.contains(&SaveWarning::ChangedAfterSave),
            "{what}: {:?}",
            r.warnings
        );
        assert_ne!(r.new_stamp, disk, "{what}: new_stamp は自分の書いた値");
        assert_eq!(r.new_stamp.len, OURS.len() as u64, "{what}");
        // 次の保存は照合で Conflict になる (他者の内容を黙って上書きしない)
        let e = fsys
            .write_atomic(
                &p,
                &mut Bytes(Some(b"next")),
                &SaveOptions::user_file(Expect::Stamp(r.new_stamp)),
            )
            .expect_err("次の保存は Conflict");
        assert!(
            matches!(e.kind, FsErrorKind::Conflict { .. }),
            "{what}: {e:?}"
        );
        assert_eq!(
            fs::read(&t).expect("読む"),
            other,
            "{what}: 他者の内容が残る"
        );
    }

    /// `ids` は `base` の FS の性質 (置換をまたいで識別が保たれるか)。
    fn check_all(base: &Path, ids: IdAcrossReplace) {
        // 同じ長さ (中身だけ違う) と、違う長さ
        const SAME_LEN: &[u8] = b"them: abcdefghij\n";
        const OTHER_LEN: &[u8] = b"them\n";
        assert_eq!(SAME_LEN.len(), OURS.len());
        for method in [Method::Replace, Method::NewFile, Method::InPlace] {
            for other in [SAME_LEN, OTHER_LEN] {
                for mtime in [Mtime::Past, Mtime::Same] {
                    // 見つからないのは、保たれない FS の置換で、同じ長さ・同じ更新時刻のファイルを被せたときだけ
                    // (05 §4.1 手順 7 の (b) の弱さ。直接上書きと保たれる FS の置換は識別の違いで見つける)
                    let detected = !(matches!(mtime, Mtime::Same)
                        && other.len() == OURS.len()
                        && ids == IdAcrossReplace::MayChange
                        && !matches!(method, Method::InPlace));
                    check(base, method, other, mtime, detected);
                }
            }
        }
    }

    #[test]
    fn 置換と直接上書きの直後の割り込みを一時ディレクトリの_fs_で見つける() {
        // Windows の %TEMP% は NTFS、WSL / Linux の一時ディレクトリは ext4・tmpfs など (置換をまたいで識別が保たれる FS)
        check_all(&std::env::temp_dir(), IdAcrossReplace::Kept);
    }

    /// 状態ディレクトリ `state` に残る保存中フラグのファイルの数 (自分のプロセスの記録は `list_inflight` に出ないので数える)。
    fn flag_files(state: &Path) -> usize {
        let Ok(rd) = fs::read_dir(state.join("recovery").join("inflight")) else {
            return 0;
        };
        rd.flatten()
            .flat_map(|k| fs::read_dir(k.path()).into_iter().flatten().flatten())
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .count()
    }

    /// `dir` の直下に残る一時ファイル・退避名 (`.eve-` を含む名前)。
    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .expect("作業ディレクトリ")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".eve-"))
            .collect()
    }

    /// 置換 (新規ファイルの移動を含む) の直後に T を消して、事後の metadata(T) を失敗させる (05 §4.1 手順 7 の「事後の
    /// metadata(T) を取れないとき」、Issue #637)。保存は成功のまま `ChangedAfterSave` を積み、`new_stamp` は自分の書いた値
    /// (置換の直後の T の識別・長さ・更新時刻) で、後片付け (退避名・保存中フラグ) は確かめられたときと同じに行う。
    /// T を消しているので、次の保存は `new_stamp` の値に依らず照合で Conflict になる (T が在るときの照合は Unix の EACCES の
    /// 試験 (`unix_fs`) が確かめる)。新規ファイルを保存できない FS (WSL の `DrvFs`) では新規ファイルを確かめない。
    #[test]
    fn 置換の後に事後の_metadata_を取れなくても成功のまま警告し後片付けする() {
        for method in [Method::Replace, Method::NewFile] {
            let what = format!("{method:?}");
            let d = Dir::new(&std::env::temp_dir(), "m");
            let state = d.0.join("state");
            let fsys = NativeFs::new(state.clone());
            let t = d.0.join("a.txt");
            let p = FsPath::new(t.clone());
            let expected = match method {
                Method::NewFile => Expect::Absent,
                Method::Replace | Method::InPlace => {
                    fs::write(&t, b"old").expect("元のファイル");
                    Expect::Stamp(FileStamp::from_meta(
                        &fsys.metadata(&p).expect("metadata"),
                        None,
                    ))
                }
            };
            // 置換の直後の T (自分の書いたもの) の値と、その時点の保存中フラグの数を控えてから T を消す
            let seen = Rc::new(Cell::new(None));
            {
                let (p, state, seen) = (p.clone(), state.clone(), Rc::clone(&seen));
                hooks::on_point("after_replace", move || {
                    let m = NativeFs::default().metadata(&p).expect("置換の直後の T");
                    seen.set(Some((FileStamp::from_meta(&m, None), flag_files(&state))));
                    fs::remove_file(p.as_path()).expect("T を消す");
                });
            }
            let r = match fsys.write_atomic(
                &p,
                &mut Bytes(Some(OURS)),
                &SaveOptions::user_file(expected),
            ) {
                Ok(r) => r,
                Err(e) if cfg!(unix) && e.kind == FsErrorKind::PermissionDenied => {
                    // 新規ファイルを保存できない FS: 積んだ割り込みを走らせずに捨てて次へ
                    assert!(matches!(method, Method::NewFile), "{what}: {e:?}");
                    hooks::forget_point("after_replace");
                    continue;
                }
                Err(e) => panic!("{what}: 失敗にしない: {e:?}"),
            };
            let (mine, flags_during) = seen.get().expect("割り込みが起きていない");
            assert_eq!(flags_during, 1, "{what}: 保存中フラグを書いていた");
            assert_eq!(
                r.warnings,
                [SaveWarning::ChangedAfterSave],
                "{what}: 成功のまま警告する"
            );
            assert_eq!(r.new_stamp, mine, "{what}: new_stamp は自分の書いた値");
            assert_eq!(r.new_stamp.len, OURS.len() as u64, "{what}");
            assert!(r.new_stamp.file_id.is_some(), "{what}: 控えた識別");
            // 後片付け: 退避名・一時ファイルもフラグも残さない
            assert!(!t.exists(), "{what}");
            assert_eq!(leftovers(&d.0), Vec::<String>::new(), "{what}");
            assert_eq!(flag_files(&state), 0, "{what}: 保存中フラグを消す");
            // 次の保存は照合で Conflict (T が無いので、new_stamp の値に依らない)
            let e = fsys
                .write_atomic(
                    &p,
                    &mut Bytes(Some(b"next")),
                    &SaveOptions::user_file(Expect::Stamp(r.new_stamp)),
                )
                .expect_err("次の保存は Conflict");
            assert!(
                matches!(e.kind, FsErrorKind::Conflict { .. }),
                "{what}: {e:?}"
            );
        }
    }

    /// `EVE_TEST_FAT_DIR` (FAT・exFAT、後ろがそれらの SMB の共有・WSL の `DrvFs`) を置いたときだけ (CI には無い)。
    #[test]
    fn 置換と直接上書きの直後の割り込みを識別が保たれない_fs_でも見つける() {
        let Some(base) = std::env::var_os("EVE_TEST_FAT_DIR") else {
            return;
        };
        check_all(Path::new(&base), IdAcrossReplace::MayChange);
    }
}
