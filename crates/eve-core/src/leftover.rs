//! 置換途中の痕跡の起動時の走査 (docs/05 §4.6「起動時の提案の規則」、03 §10.3 手順 1b)。
//!
//! `list_inflight` の記録を T のパスごとにまとめ、記録された一時ファイル (`.eve-tmp-*`)・退避名 (`.eve-bak-*`) の実在と T の有無から、
//! 提案 (`Leftover`) を作る。利用者のディレクトリは列挙せず、記録された名前の `metadata` だけを見る。ジョブの中で呼ぶ。
//! 痕跡が何も残っていない記録のフラグと、どのフラグにも記録されていない書込途中の事前バックアップ (`*.bak.tmp`) はここで片付ける。

use std::collections::BTreeSet;

use crate::fs::{FileSystem, FileTime, FsErrorKind, InflightRecord, SaveMethod};
use crate::fspath::FsPath;

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

/// 起動時に提案する 1 件。T のパス 1 つ分。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leftover {
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
        let mut bak: Option<FsPath> = None;
        let mut tmp: Option<FsPath> = None;
        let mut flags = Vec::new();
        let mut recorded_at: Option<FileTime> = None;
        for r in &recs {
            let b = r.backup.as_ref().filter(|b| exists(fs, b));
            let t = r.tmp.as_ref().filter(|t| exists(fs, t));
            if b.is_none() && t.is_none() {
                // 痕跡が何も残っていない記録: フラグだけを消す
                let _ = fs.remove_file(&r.flag);
                continue;
            }
            bak = bak.or_else(|| b.cloned());
            tmp = tmp.or_else(|| t.cloned());
            flags.push(r.flag.clone());
            if let Some(at) = r.started_at
                && recorded_at.is_none_or(|c| newer(at, c))
            {
                recorded_at = Some(at);
            }
        }
        if bak.is_none() && tmp.is_none() {
            continue;
        }
        // T の有無。調べられない (ネットワークの不通など) ときは、状態が分からないので提案しない
        let t_exists = match fs.metadata(&target) {
            Ok(_) => true,
            Err(e) if e.kind == FsErrorKind::NotFound => false,
            Err(_) => continue,
        };
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
