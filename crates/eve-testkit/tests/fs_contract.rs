//! `MemFs` に `FileSystem` の契約試験を掛ける (docs/02 §4.5、STR-D3)。同じ検査を eve-platform の
//! `tests/fs_contract.rs` が実 FS に掛ける。

use std::sync::Arc;

use eve_core::{FileMeta, FileSystem, FsError, FsPath, ReadChunks, SaveOptions, SaveReport};
use eve_testkit::fs_contract::{Caps, MemFixture, Subject, failures, suite};
use eve_testkit::{ManualClock, MemFs};

fn memfs_subject(fs: Arc<MemFs>, as_port: Arc<dyn FileSystem>) -> Subject {
    let root = "/contract";
    fs.add_dir(root);
    let fixture = MemFixture(fs.clone());
    Subject::new(as_port, FsPath::from(root))
        .fixture(fixture)
        .caps(Caps {
            // MemFs の `list_recovery_dirs` はテストが積んだものだけを返す (既知の差)
            recovery_listing_reflects_writes: false,
            read_dir_len: true,
            ..Caps::default()
        })
        .lister(move |dir| {
            fs.paths()
                .into_iter()
                .map(|p| FsPath::from(p.as_str()))
                .filter(|p| p.parent().as_ref() == Some(dir))
                .filter_map(|p| p.file_name())
                .collect()
        })
}

fn memfs() -> Arc<MemFs> {
    Arc::new(MemFs::new(Arc::new(ManualClock::new())))
}

#[test]
fn memfs_は_filesystem_の契約を満たす() {
    suite(|| {
        let fs = memfs();
        memfs_subject(fs.clone(), fs)
    });
}

/// 必須の 4 つだけを `MemFs` に任せ、残りは trait の既定の実装のままの Port (上書きを忘れた実装の再現)。
struct OnlyRequired(Arc<MemFs>);

impl FileSystem for OnlyRequired {
    fn metadata(&self, path: &FsPath) -> Result<FileMeta, FsError> {
        self.0.metadata(path)
    }
    fn canonicalize(&self, path: &FsPath) -> Result<FsPath, FsError> {
        self.0.canonicalize(path)
    }
    fn open_read(&self, path: &FsPath) -> Result<Box<dyn ReadChunks + Send>, FsError> {
        self.0.open_read(path)
    }
    fn write_atomic(
        &self,
        path: &FsPath,
        data: &mut dyn ReadChunks,
        opts: &SaveOptions,
    ) -> Result<SaveReport, FsError> {
        self.0.write_atomic(path, data, opts)
    }
}

#[test]
fn 契約試験は既定の実装のままの操作を見逃さない() {
    let failed = failures(|| {
        let fs = memfs();
        memfs_subject(fs.clone(), Arc::new(OnlyRequired(fs)))
    });
    // 書く操作の既定 (Unsupported / Other) はどれかの検査で落ちる。一覧の既定 (空) は仕様の初期状態と区別できない
    for want in [
        "stamp_で保存先が無ければ_conflict_none",
        "rename_no_replace_は置き換えない",
        "remove_file_は消し_無ければ_notfound",
        "直接上書きは同じファイルを書き換える",
        "状態ファイルを読み書きし退ける",
        "スワップを書いて消す",
        "退避先に保存はディレクトリを返す",
        // read_dir の既定 (Unsupported) は空のディレクトリと区別できる
        "read_dir_は空のディレクトリで空",
        "read_dir_は種類を返しリンクを辿らない",
        "read_dir_は_max_entries_で打ち切る",
        // read_checked の既定 (Unsupported) は「無い」とも「読んだ」とも区別できる
        "read_checked_は判定を通ったファイルを読む",
        "read_checked_は無いファイルを_missing_で返す",
    ] {
        assert!(failed.contains(&want), "{want} が落ちない: {failed:?}");
    }
}
