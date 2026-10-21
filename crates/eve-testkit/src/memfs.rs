//! `MemFs`: メモリ上の `FileSystem` と障害注入 (docs/02 §4.5、docs/09 §3.4)。
//!
//! M1 の範囲は `metadata` / `canonicalize` / `open_read` / `write_atomic`。障害注入は
//! (1) 操作ごとの次の 1 回の失敗、(2) `Busy` を k 回返してから成功、(3) `write_atomic` の各ステップでの失敗、
//! (4) N バイト書込後の `DiskFull`、(5) 読込の途中の失敗、(6) 外部からの変更 (Stamp の不一致)、を持つ。
//! 失敗したステップより後は実行せず、保存先は元のまま (一時ファイルは作らない・残さない) にする。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use eve_core::{
    ChunkError, Clock, Expect, FileId, FileKind, FileMeta, FileStamp, FileSystem, FileTime,
    FsError, FsErrorDetail, FsErrorKind, FsKind, FsPath, FsyncLevel, HardlinkPolicy,
    InflightRecord, ReadChunks, RecoveryDirEntry, SaveMethod, SaveOptions, SaveReport,
};

/// 障害を注入できる操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FsOp {
    Metadata,
    Canonicalize,
    OpenRead,
    WriteAtomic,
}

/// `write_atomic` のステップ (失敗を注入でき、到達したステップは `MemFs::steps` に残る)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WriteStep {
    CreateTemp,
    WriteData,
    Fsync,
    Replace,
    DirFsync,
}

#[derive(Clone)]
struct Entry {
    data: Vec<u8>,
    mtime: FileTime,
    readonly: bool,
    id: u128,
    nlink: u32,
}

struct State {
    files: BTreeMap<FsPath, Entry>,
    dirs: BTreeSet<FsPath>,
    next_id: u128,
    seq: u32,
    chunk_size: usize,
    fail_next: Vec<(FsOp, FsErrorKind)>,
    busy: BTreeMap<FsOp, u32>,
    fail_step: Vec<(WriteStep, FsErrorKind)>,
    disk_full_after: Option<u64>,
    read_fail_after: Option<u64>,
    steps: Vec<WriteStep>,
    /// 保存中フラグ (テストが `add_inflight` で積む)。
    inflight: Vec<InflightRecord>,
    /// 退避ディレクトリ (テストが `add_recovery_dir` で積む)。
    recovery: Vec<RecoveryDirEntry>,
}

pub struct MemFs {
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
}

/// 既定のチャンク (02 §4.5: 1 MiB)。
const DEFAULT_CHUNK: usize = 1 << 20;

impl MemFs {
    pub fn new(clock: Arc<dyn Clock>) -> MemFs {
        MemFs {
            clock,
            state: Mutex::new(State {
                files: BTreeMap::new(),
                dirs: BTreeSet::new(),
                next_id: 1,
                seq: 0,
                chunk_size: DEFAULT_CHUNK,
                fail_next: Vec::new(),
                busy: BTreeMap::new(),
                fail_step: Vec::new(),
                disk_full_after: None,
                read_fail_after: None,
                steps: Vec::new(),
                inflight: Vec::new(),
                recovery: Vec::new(),
            }),
        }
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("MemFs の Mutex が壊れた")
    }

    fn now_time(&self, s: &mut State) -> FileTime {
        // 同じ ms の中の連続した書込も別の mtime になるよう、連番を ns に足す
        s.seq = s.seq.wrapping_add(1);
        let ms = self.clock.now().0;
        FileTime {
            secs: (ms / 1000) as i64,
            nanos: ((ms % 1000) as u32) * 1_000_000 + s.seq % 1_000_000,
        }
    }

    // ---------- 準備と観測 ----------

    /// ファイルを作る (祖先のフォルダも作る)。既にあれば内容を置き換える (`set_file` と同じ)。
    pub fn add_file(&self, path: &str, data: &[u8]) {
        self.set_file(&FsPath::from(path), data);
    }

    pub fn add_dir(&self, path: &str) {
        let mut s = self.st();
        Self::add_ancestors_and_self(&mut s, &FsPath::from(path));
    }

    fn add_ancestors_and_self(s: &mut State, path: &FsPath) {
        let mut cur = Some(path.clone());
        while let Some(p) = cur {
            cur = p.parent();
            s.dirs.insert(p);
        }
    }

    /// 外部からの変更: 内容と mtime を変える (file_id は保つ)。無ければ作る。
    pub fn set_file(&self, path: &FsPath, data: &[u8]) {
        let mut s = self.st();
        let mtime = self.now_time(&mut s);
        if let Some(parent) = path.parent() {
            Self::add_ancestors_and_self(&mut s, &parent);
        }
        let (id, readonly, nlink) = match s.files.get(path) {
            Some(e) => (e.id, e.readonly, e.nlink),
            None => {
                s.next_id += 1;
                (s.next_id, false, 1)
            }
        };
        s.files.insert(
            path.clone(),
            Entry {
                data: data.to_vec(),
                mtime,
                readonly,
                id,
                nlink,
            },
        );
    }

    pub fn remove(&self, path: &FsPath) {
        self.st().files.remove(path);
    }

    /// 保存中フラグを積む (中断の痕跡の再現。`InflightRecord.flag` は `/state/inflight/…` のような任意のパス)。
    pub fn add_inflight(&self, record: InflightRecord) {
        self.st().inflight.push(record);
    }

    /// 退避ディレクトリを積む (起動時の案内の再現)。`rescue` は「退避先に保存」の退避。
    pub fn add_recovery_dir(&self, path: &str, rescue: bool) {
        self.st().recovery.push(RecoveryDirEntry {
            path: FsPath::from(path),
            rescue,
        });
    }

    /// 残っている保存中フラグ。
    pub fn inflight_records(&self) -> Vec<InflightRecord> {
        self.st().inflight.clone()
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.st()
            .files
            .get(&FsPath::from(path))
            .map(|e| e.data.clone())
    }

    pub fn stamp(&self, path: &str) -> Option<FileStamp> {
        let s = self.st();
        s.files
            .get(&FsPath::from(path))
            .map(|e| FileStamp::from_meta(&meta_of(e), None))
    }

    pub fn exists(&self, path: &str) -> bool {
        let s = self.st();
        let p = FsPath::from(path);
        s.files.contains_key(&p) || s.dirs.contains(&p)
    }

    pub fn set_readonly(&self, path: &str, readonly: bool) {
        if let Some(e) = self.st().files.get_mut(&FsPath::from(path)) {
            e.readonly = readonly;
        }
    }

    pub fn set_nlink(&self, path: &str, nlink: u32) {
        if let Some(e) = self.st().files.get_mut(&FsPath::from(path)) {
            e.nlink = nlink;
        }
    }

    pub fn set_chunk_size(&self, bytes: usize) {
        self.st().chunk_size = bytes.max(1);
    }

    // ---------- 障害注入 ----------

    /// `op` の次の 1 回を `kind` で失敗させる (何度も呼べば順に消費する)。
    pub fn fail_next(&self, op: FsOp, kind: FsErrorKind) {
        self.st().fail_next.push((op, kind));
    }

    /// `op` を `k` 回 `Busy` で失敗させてから成功させる。
    pub fn busy_times(&self, op: FsOp, k: u32) {
        self.st().busy.insert(op, k);
    }

    /// `write_atomic` の `step` で 1 回失敗させる。保存先は元のまま。
    /// `DirFsync` だけは置換の後なので、保存先は新しい内容になる (`Err` は返る)。
    pub fn fail_step(&self, step: WriteStep, kind: FsErrorKind) {
        self.st().fail_step.push((step, kind));
    }

    /// 1 回の `write_atomic` で `n` バイトを超えて書くと `DiskFull` にする (`None` で解除)。
    pub fn disk_full_after(&self, n: Option<u64>) {
        self.st().disk_full_after = n;
    }

    /// `open_read` の読込が `n` バイトを読んだ後に失敗する (`None` で解除)。
    pub fn read_fail_after(&self, n: Option<u64>) {
        self.st().read_fail_after = n;
    }

    /// 直近の `write_atomic` が到達したステップ。
    pub fn steps(&self) -> Vec<WriteStep> {
        self.st().steps.clone()
    }
}

fn meta_of(e: &Entry) -> FileMeta {
    FileMeta {
        kind: FileKind::File,
        len: e.data.len() as u64,
        mtime: e.mtime,
        readonly: e.readonly,
        file_id: Some(FileId {
            volume: 1,
            index: e.id,
        }),
        nlink: e.nlink,
        link: None,
        fs_kind: FsKind::Local,
        owner_is_me: Some(true),
    }
}

fn dir_meta() -> FileMeta {
    FileMeta {
        kind: FileKind::Dir,
        len: 0,
        mtime: FileTime::default(),
        readonly: false,
        file_id: None,
        nlink: 1,
        link: None,
        fs_kind: FsKind::Local,
        owner_is_me: Some(true),
    }
}

fn dir_exists(s: &State, dir: &FsPath) -> bool {
    s.dirs.contains(dir) || dir.parent().is_none()
}

fn not_found(s: &State, path: &FsPath) -> FsError {
    match path.parent() {
        Some(dir) if !dir_exists(s, &dir) => {
            FsError::with_detail(FsErrorKind::NotFound, FsErrorDetail::ParentMissing { dir })
        }
        _ => FsError::new(FsErrorKind::NotFound),
    }
}

/// 注入された失敗を 1 つ消費する (`Busy` の k 回が先)。
fn injected(s: &mut State, op: FsOp) -> Result<(), FsError> {
    if let Some(k) = s.busy.get_mut(&op)
        && *k > 0
    {
        *k -= 1;
        return Err(FsError::new(FsErrorKind::Busy));
    }
    if let Some(i) = s.fail_next.iter().position(|(o, _)| *o == op) {
        return Err(FsError::new(s.fail_next.remove(i).1));
    }
    Ok(())
}

fn step(s: &mut State, st: WriteStep) -> Result<(), FsError> {
    s.steps.push(st);
    if let Some(i) = s.fail_step.iter().position(|(x, _)| *x == st) {
        return Err(FsError::new(s.fail_step.remove(i).1));
    }
    Ok(())
}

impl FileSystem for MemFs {
    fn list_recovery_dirs(&self) -> Result<Vec<RecoveryDirEntry>, FsError> {
        Ok(self.st().recovery.clone())
    }

    fn inflight_for(&self, path: &FsPath) -> Result<Vec<InflightRecord>, FsError> {
        let s = self.st();
        Ok(s.inflight
            .iter()
            .filter(|r| &r.target == path)
            .cloned()
            .collect())
    }

    fn remove_file(&self, path: &FsPath) -> Result<(), FsError> {
        let mut s = self.st();
        // 保存中フラグのファイル、または通常のファイル
        let before = s.inflight.len();
        s.inflight.retain(|r| &r.flag != path);
        if s.inflight.len() != before || s.files.remove(path).is_some() {
            Ok(())
        } else {
            Err(FsError::new(FsErrorKind::NotFound))
        }
    }

    fn metadata(&self, path: &FsPath) -> Result<FileMeta, FsError> {
        let mut s = self.st();
        injected(&mut s, FsOp::Metadata)?;
        if let Some(e) = s.files.get(path) {
            return Ok(meta_of(e));
        }
        if s.dirs.contains(path) {
            return Ok(dir_meta());
        }
        Err(not_found(&s, path))
    }

    fn canonicalize(&self, path: &FsPath) -> Result<FsPath, FsError> {
        let mut s = self.st();
        injected(&mut s, FsOp::Canonicalize)?;
        if s.files.contains_key(path) || s.dirs.contains(path) {
            Ok(path.clone())
        } else {
            Err(not_found(&s, path))
        }
    }

    fn open_read(&self, path: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
        let mut s = self.st();
        injected(&mut s, FsOp::OpenRead)?;
        match s.files.get(path) {
            Some(e) => Ok(Box::new(MemReader {
                data: e.data.clone(),
                pos: 0,
                chunk: s.chunk_size,
                fail_after: s.read_fail_after,
            })),
            None if s.dirs.contains(path) => Err(FsError::with_detail(
                FsErrorKind::InvalidPath,
                FsErrorDetail::NotAFile,
            )),
            None => Err(not_found(&s, path)),
        }
    }

    fn write_atomic(
        &self,
        path: &FsPath,
        data: &mut dyn ReadChunks,
        opts: &SaveOptions,
    ) -> Result<SaveReport, FsError> {
        let mut s = self.st();
        s.steps.clear();
        injected(&mut s, FsOp::WriteAtomic)?;

        if let Some(dir) = path.parent()
            && !dir_exists(&s, &dir)
        {
            return Err(FsError::with_detail(
                FsErrorKind::NotFound,
                FsErrorDetail::ParentMissing { dir },
            ));
        }
        if s.dirs.contains(path) {
            return Err(FsError::with_detail(
                FsErrorKind::InvalidPath,
                FsErrorDetail::NotAFile,
            ));
        }
        let existing = s.files.get(path).cloned();
        let current = existing
            .as_ref()
            .map(|e| FileStamp::from_meta(&meta_of(e), None));
        let conflict = |current| FsError::new(FsErrorKind::Conflict { current });
        match (opts.expected, &current) {
            (Expect::Absent, Some(_)) => return Err(conflict(current)),
            (Expect::Stamp(_), None) => return Err(conflict(None)),
            (Expect::Stamp(want), Some(have)) if !stamp_matches(&want, have) => {
                return Err(conflict(current));
            }
            _ => {}
        }
        if let Some(e) = &existing {
            if e.readonly {
                return Err(FsError::new(FsErrorKind::PermissionDenied));
            }
            if e.nlink > 1 && opts.hardlink == HardlinkPolicy::Ask {
                return Err(FsError::with_detail(
                    FsErrorKind::AtomicUnsupported,
                    FsErrorDetail::Hardlink { nlink: e.nlink },
                ));
            }
        }

        step(&mut s, WriteStep::CreateTemp)?;
        step(&mut s, WriteStep::WriteData)?;
        let mut buf: Vec<u8> = Vec::new();
        loop {
            match data.next_chunk() {
                Ok(Some(chunk)) => {
                    if s.disk_full_after
                        .is_some_and(|n| (buf.len() + chunk.len()) as u64 > n)
                    {
                        return Err(FsError::new(FsErrorKind::DiskFull));
                    }
                    buf.extend_from_slice(chunk);
                }
                Ok(None) => break,
                // 供給側の失敗: 一時ファイルは消え、元ファイルは無傷
                Err(_) => return Err(FsError::new(FsErrorKind::SourceFailed)),
            }
        }
        if opts.fsync != FsyncLevel::None {
            step(&mut s, WriteStep::Fsync)?;
        }
        step(&mut s, WriteStep::Replace)?;
        let mtime = self.now_time(&mut s);
        s.next_id += 1;
        let entry = Entry {
            data: buf,
            mtime,
            readonly: false,
            id: s.next_id,
            nlink: 1,
        };
        let new_stamp = FileStamp::from_meta(&meta_of(&entry), None);
        s.files.insert(path.clone(), entry);
        if opts.fsync == FsyncLevel::Full {
            step(&mut s, WriteStep::DirFsync)?;
        }
        let method = if existing.is_some() {
            SaveMethod::Atomic
        } else {
            SaveMethod::NewFile
        };
        Ok(SaveReport {
            method,
            new_stamp,
            warnings: Vec::new(),
        })
    }
}

/// Stamp の照合 (05 §5.5): mtime・len・file_id、ハッシュは双方にあるときだけ。
fn stamp_matches(want: &FileStamp, have: &FileStamp) -> bool {
    want.mtime == have.mtime
        && want.len == have.len
        && want.file_id == have.file_id
        && match (want.hash, have.hash) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
}

struct MemReader {
    data: Vec<u8>,
    pos: usize,
    chunk: usize,
    fail_after: Option<u64>,
}

impl ReadChunks for MemReader {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        let mut end = (self.pos + self.chunk).min(self.data.len());
        if let Some(n) = self.fail_after {
            let n = n as usize;
            if self.pos >= n {
                return Err(ChunkError::Io(FsError::new(FsErrorKind::Other)));
            }
            end = end.min(n);
        }
        let start = self.pos;
        self.pos = end;
        Ok(Some(&self.data[start..end]))
    }
}

/// 保存の供給側の失敗の種類 (`BytesSource::fail_after`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceFailure {
    Unmappable,
    Aborted,
}

/// バイト列を決まった大きさのチャンクで供給する `ReadChunks` (保存のテスト用の供給側)。
pub struct BytesSource {
    data: Vec<u8>,
    pos: usize,
    chunk: usize,
    fail: Option<(usize, SourceFailure)>,
}

impl BytesSource {
    pub fn new(data: &[u8], chunk: usize) -> BytesSource {
        BytesSource {
            data: data.to_vec(),
            pos: 0,
            chunk: chunk.max(1),
            fail: None,
        }
    }

    /// `n` バイトを供給した後に失敗する。
    pub fn fail_after(mut self, n: usize, how: SourceFailure) -> BytesSource {
        self.fail = Some((n, how));
        self
    }
}

impl ReadChunks for BytesSource {
    fn next_chunk(&mut self) -> Result<Option<&[u8]>, ChunkError> {
        let mut end = (self.pos + self.chunk).min(self.data.len());
        if let Some((n, how)) = self.fail {
            if self.pos >= n {
                return Err(match how {
                    SourceFailure::Unmappable => ChunkError::Unmappable,
                    SourceFailure::Aborted => ChunkError::Aborted,
                });
            }
            end = end.min(n);
        }
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        let start = self.pos;
        self.pos = end;
        Ok(Some(&self.data[start..end]))
    }
}

#[cfg(test)]
mod tests {
    use eve_core::{ContentHash, Millis};

    use super::*;
    use crate::ManualClock;

    fn fs() -> (MemFs, ManualClock) {
        let c = ManualClock::new();
        (MemFs::new(Arc::new(c.clone())), c)
    }

    fn save(fs: &MemFs, path: &str, bytes: &[u8], expected: Expect) -> Result<SaveReport, FsError> {
        let mut src = BytesSource::new(bytes, 4);
        fs.write_atomic(
            &FsPath::from(path),
            &mut src,
            &SaveOptions::user_file(expected),
        )
    }

    fn read_all(fs: &MemFs, path: &str) -> Vec<u8> {
        let mut r = fs.open_read(&FsPath::from(path)).unwrap();
        let mut out = Vec::new();
        while let Some(c) = r.next_chunk().unwrap() {
            out.extend_from_slice(c);
        }
        out
    }

    #[test]
    fn メタ情報と_notfound_の区別() {
        let (fs, _) = fs();
        fs.add_file("/w/a.txt", b"hello");
        let m = fs.metadata(&FsPath::from("/w/a.txt")).unwrap();
        assert_eq!((m.kind, m.len, m.nlink), (FileKind::File, 5, 1));
        assert_eq!(
            fs.metadata(&FsPath::from("/w")).unwrap().kind,
            FileKind::Dir
        );
        let e = fs.metadata(&FsPath::from("/w/none.txt")).unwrap_err();
        assert_eq!(
            (e.kind, e.detail),
            (FsErrorKind::NotFound, None),
            "親があれば detail なし"
        );
        let e = fs.metadata(&FsPath::from("/nodir/x.txt")).unwrap_err();
        assert!(matches!(
            e.detail,
            Some(FsErrorDetail::ParentMissing { .. })
        ));
    }

    #[test]
    fn チャンク読込と途中の失敗() {
        let (fs, _) = fs();
        fs.add_file("/w/a.txt", b"0123456789");
        fs.set_chunk_size(4);
        let mut r = fs.open_read(&FsPath::from("/w/a.txt")).unwrap();
        let mut sizes = Vec::new();
        while let Some(c) = r.next_chunk().unwrap() {
            sizes.push(c.len());
        }
        assert_eq!(sizes, [4, 4, 2]);

        fs.read_fail_after(Some(6));
        let mut r = fs.open_read(&FsPath::from("/w/a.txt")).unwrap();
        let mut got = 0;
        let err = loop {
            match r.next_chunk() {
                Ok(Some(c)) => got += c.len(),
                Ok(None) => panic!("失敗するはず"),
                Err(e) => break e,
            }
        };
        assert_eq!(got, 6, "6 バイトまでは読める");
        assert!(matches!(err, ChunkError::Io(_)));
        let e = fs.open_read(&FsPath::from("/w")).err().unwrap();
        assert_eq!(e.detail, Some(FsErrorDetail::NotAFile));
    }

    #[test]
    fn 新規作成と置換() {
        let (fs, clock) = fs();
        fs.add_dir("/w");
        let r = save(&fs, "/w/new.txt", b"abcdefghij", Expect::Absent).unwrap();
        assert_eq!(r.method, SaveMethod::NewFile);
        assert_eq!(fs.read("/w/new.txt").unwrap(), b"abcdefghij");
        assert_eq!(
            Some(r.new_stamp),
            fs.stamp("/w/new.txt"),
            "報告の Stamp は実際と一致"
        );
        // 既にある保存先への Absent は Conflict
        let e = save(&fs, "/w/new.txt", b"x", Expect::Absent).unwrap_err();
        assert!(matches!(e.kind, FsErrorKind::Conflict { current: Some(_) }));
        // Stamp が合えば置換できる
        clock.advance(Millis(10));
        let r2 = save(&fs, "/w/new.txt", b"12", Expect::Stamp(r.new_stamp)).unwrap();
        assert_eq!(r2.method, SaveMethod::Atomic);
        assert_eq!(fs.read("/w/new.txt").unwrap(), b"12");
        assert_ne!(r2.new_stamp, r.new_stamp);
        assert_eq!(read_all(&fs, "/w/new.txt"), b"12");
    }

    #[test]
    fn 外部変更は_conflict_で何も書かない() {
        let (fs, _) = fs();
        fs.add_file("/w/a.txt", b"v1");
        let stamp = fs.stamp("/w/a.txt").unwrap();
        fs.set_file(&FsPath::from("/w/a.txt"), b"v2"); // 同じ長さでも mtime が変わる
        let e = save(&fs, "/w/a.txt", b"mine", Expect::Stamp(stamp)).unwrap_err();
        assert!(
            matches!(e.kind, FsErrorKind::Conflict { current: Some(c) } if Some(c) == fs.stamp("/w/a.txt"))
        );
        assert_eq!(fs.read("/w/a.txt").unwrap(), b"v2");
        assert!(
            fs.steps().is_empty(),
            "照合で止まり、書込のステップに入らない"
        );
        // 削除された保存先は current = None
        fs.remove(&FsPath::from("/w/a.txt"));
        let e = save(&fs, "/w/a.txt", b"mine", Expect::Stamp(stamp)).unwrap_err();
        assert!(matches!(e.kind, FsErrorKind::Conflict { current: None }));
    }

    #[test]
    fn 各ステップの失敗は元ファイルを保つ() {
        for step in [
            WriteStep::CreateTemp,
            WriteStep::WriteData,
            WriteStep::Fsync,
            WriteStep::Replace,
        ] {
            let (fs, _) = fs();
            fs.add_file("/w/a.txt", b"orig");
            let stamp = fs.stamp("/w/a.txt").unwrap();
            fs.fail_step(step, FsErrorKind::PermissionDenied);
            let e = save(&fs, "/w/a.txt", b"new", Expect::Stamp(stamp)).unwrap_err();
            assert_eq!(e.kind, FsErrorKind::PermissionDenied, "{step:?}");
            assert_eq!(fs.read("/w/a.txt").unwrap(), b"orig", "{step:?}");
            assert_eq!(fs.stamp("/w/a.txt"), Some(stamp), "{step:?}");
            assert_eq!(fs.steps().last(), Some(&step));
            // 1 回限り: 次は成功する
            save(&fs, "/w/a.txt", b"new", Expect::Stamp(stamp)).unwrap();
            assert_eq!(fs.read("/w/a.txt").unwrap(), b"new");
        }
    }

    #[test]
    fn ディレクトリの_fsync_の失敗は置換の後() {
        let (fs, _) = fs();
        fs.add_file("/w/a.txt", b"orig");
        fs.fail_step(WriteStep::DirFsync, FsErrorKind::Other);
        assert!(save(&fs, "/w/a.txt", b"new", Expect::Any).is_err());
        assert_eq!(fs.read("/w/a.txt").unwrap(), b"new");
        assert_eq!(
            fs.steps(),
            [
                WriteStep::CreateTemp,
                WriteStep::WriteData,
                WriteStep::Fsync,
                WriteStep::Replace,
                WriteStep::DirFsync
            ]
        );
    }

    #[test]
    fn 容量不足と供給側の失敗() {
        let (fs, _) = fs();
        fs.add_file("/w/a.txt", b"orig");
        fs.disk_full_after(Some(5));
        let e = save(&fs, "/w/a.txt", b"0123456789", Expect::Any).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::DiskFull);
        assert_eq!(fs.read("/w/a.txt").unwrap(), b"orig");
        fs.disk_full_after(None);

        for how in [SourceFailure::Unmappable, SourceFailure::Aborted] {
            let mut src = BytesSource::new(b"0123456789", 4).fail_after(6, how);
            let e = fs
                .write_atomic(
                    &FsPath::from("/w/a.txt"),
                    &mut src,
                    &SaveOptions::user_file(Expect::Any),
                )
                .unwrap_err();
            assert_eq!(e.kind, FsErrorKind::SourceFailed, "{how:?}");
            assert_eq!(fs.read("/w/a.txt").unwrap(), b"orig");
        }
    }

    #[test]
    fn busy_を_k_回返してから成功する() {
        let (fs, _) = fs();
        fs.add_file("/w/a.txt", b"x");
        fs.busy_times(FsOp::Metadata, 2);
        let p = FsPath::from("/w/a.txt");
        assert_eq!(fs.metadata(&p).unwrap_err().kind, FsErrorKind::Busy);
        assert_eq!(fs.metadata(&p).unwrap_err().kind, FsErrorKind::Busy);
        assert!(fs.metadata(&p).is_ok());
        fs.fail_next(FsOp::Canonicalize, FsErrorKind::PermissionDenied);
        assert_eq!(
            fs.canonicalize(&p).unwrap_err().kind,
            FsErrorKind::PermissionDenied
        );
        assert_eq!(fs.canonicalize(&p).unwrap(), p);
    }

    #[test]
    fn 書けない保存先() {
        let (fs, _) = fs();
        fs.add_file("/w/ro.txt", b"x");
        fs.set_readonly("/w/ro.txt", true);
        assert_eq!(
            save(&fs, "/w/ro.txt", b"y", Expect::Any).unwrap_err().kind,
            FsErrorKind::PermissionDenied
        );
        fs.add_file("/w/hl.txt", b"x");
        fs.set_nlink("/w/hl.txt", 2);
        let e = save(&fs, "/w/hl.txt", b"y", Expect::Any).unwrap_err();
        assert_eq!(
            (e.kind, e.detail),
            (
                FsErrorKind::AtomicUnsupported,
                Some(FsErrorDetail::Hardlink { nlink: 2 })
            )
        );
        let e = save(&fs, "/nodir/z.txt", b"y", Expect::Absent).unwrap_err();
        assert!(matches!(
            e.detail,
            Some(FsErrorDetail::ParentMissing { .. })
        ));
        assert_eq!(
            save(&fs, "/w", b"y", Expect::Any).unwrap_err().detail,
            Some(FsErrorDetail::NotAFile)
        );
    }

    #[test]
    fn ハッシュは双方にあるときだけ比べる() {
        let a = FileStamp {
            mtime: FileTime::default(),
            len: 1,
            file_id: None,
            hash: Some(ContentHash([1; 8])),
        };
        let mut b = a;
        assert!(stamp_matches(&a, &b));
        b.hash = None;
        assert!(stamp_matches(&a, &b));
        b.hash = Some(ContentHash([2; 8]));
        assert!(!stamp_matches(&a, &b));
    }
}
