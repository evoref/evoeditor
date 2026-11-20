//! ファイルの操作の失敗 (`FsError` / `JobError`) から、文言キー (`MessageKey`、04 §15.2) と `{reason}` の短い分類名への写像
//! (docs/review/v0.3-findings.md の STR-B7)。どの失敗をどのキーで知らせるかはここで決め、呼ぶ側は `{path}`・`{backup}` などの
//! 引数を組み立てるだけにする。
//!
//! spec: 04 §15.2, 03 §9.2, 03 §9.3

use eve_view::MessageLevel;

use crate::io::fs::{FsError, FsErrorDetail, FsErrorKind};
use crate::jobs::JobError;
use crate::ui_model::keys;

/// 開く (再読込・開き直しを含む) ジョブの失敗の水準と `MessageKey`。
pub(crate) fn open_failure(err: &JobError) -> (MessageLevel, &'static str) {
    match err {
        JobError::Cancelled => (MessageLevel::Info, keys::INFO_OPEN_CANCELLED),
        JobError::Fs(e) => (MessageLevel::Error, open_error_key(e)),
        JobError::OutOfMemory { .. } => (MessageLevel::Error, keys::MSG_OPEN_OUT_OF_MEMORY),
        _ => (MessageLevel::Error, keys::MSG_OPEN_FAILED),
    }
}

/// 開く失敗の `MessageKey`。
pub(crate) fn open_error_key(e: &FsError) -> &'static str {
    match (&e.kind, &e.detail) {
        (FsErrorKind::PermissionDenied, _) => keys::MSG_OPEN_PERMISSION_DENIED,
        (FsErrorKind::Busy, _) => keys::MSG_OPEN_BUSY,
        (_, Some(FsErrorDetail::NotAFile)) => keys::MSG_OPEN_NOT_A_FILE,
        (_, Some(FsErrorDetail::ReservedName)) => keys::MSG_OPEN_RESERVED_NAME,
        _ => keys::MSG_OPEN_FAILED,
    }
}

/// 保存ジョブの失敗の水準と `MessageKey`。失敗の後の選択肢 (確認ダイアログ) は別に出す。
pub(crate) fn save_failure(err: &JobError) -> (MessageLevel, &'static str) {
    match err {
        JobError::Cancelled => (MessageLevel::Info, keys::INFO_SAVE_ABORTED),
        JobError::Fs(e) => (MessageLevel::Error, save_error_key(e)),
        // 確認 (ii) の不一致 (03 §8.6 E2): 何も書いていない。別名で保存・退避先に保存を選べる
        JobError::VerifyFailed => (MessageLevel::Error, keys::MSG_SAVE_VERIFY_FAILED),
        _ => (MessageLevel::Error, keys::MSG_SAVE_FAILED),
    }
}

/// 保存失敗の `MessageKey`。
pub(crate) fn save_error_key(e: &FsError) -> &'static str {
    match (&e.kind, &e.detail) {
        (FsErrorKind::PermissionDenied, _) => keys::MSG_SAVE_PERMISSION_DENIED,
        (FsErrorKind::DiskFull, _) => keys::MSG_SAVE_DISK_FULL,
        (FsErrorKind::ReadOnlyFs, _) => keys::MSG_SAVE_READ_ONLY_FS,
        (FsErrorKind::InvalidPath, _) => keys::MSG_SAVE_INVALID_PATH,
        (FsErrorKind::Busy, _) => keys::MSG_SAVE_BUSY,
        // 置換の途中で失敗し、元のファイルは退避名に残っている (NFR-301 AC-2)。そのパスを必ず示す
        (FsErrorKind::ReplacedMovedToBackup { .. }, _) => keys::MSG_SAVE_MOVED_TO_BACKUP,
        // 直接上書きの途中の失敗: 元のファイルは壊れている可能性がある。バックアップの場所を必ず示す (05 §4.4 手順 8)
        (FsErrorKind::InPlaceFailed { backup: Some(_) }, _) => keys::MSG_SAVE_IN_PLACE_FAILED,
        (FsErrorKind::InPlaceFailed { backup: None }, _) => {
            keys::MSG_SAVE_IN_PLACE_FAILED_NO_BACKUP
        }
        (FsErrorKind::NotFound, Some(FsErrorDetail::ParentMissing { .. })) => {
            keys::MSG_SAVE_PARENT_MISSING
        }
        // リンク先が無いシンボリックリンク: 書かずに、リンク先を示して知らせる (FR-106 AC-2)
        (FsErrorKind::NotFound, Some(FsErrorDetail::DanglingLink { .. })) => {
            keys::MSG_SAVE_DANGLING_LINK
        }
        _ => keys::MSG_SAVE_FAILED,
    }
}

/// 「中止して終了」の最中に失敗した保存の、標準エラー出力の行のキー (`stderr.quit_abort.*`)。
pub(crate) fn quit_abort_stderr_key(kind: &FsErrorKind) -> &'static str {
    match kind {
        FsErrorKind::ReplacedMovedToBackup { .. } => "stderr.quit_abort.moved_to_backup",
        FsErrorKind::InPlaceFailed { backup: Some(_) } => "stderr.quit_abort.in_place_failed",
        FsErrorKind::InPlaceFailed { backup: None } => {
            "stderr.quit_abort.in_place_failed_no_backup"
        }
        _ => "stderr.quit_abort.save_failed",
    }
}

/// `{reason}` に入れる短い分類名 (技術的な識別子。OS のエラーの分類文言 `ui.reason.*` は目録ができてから)。
pub(crate) fn reason_label(err: &JobError) -> String {
    match err {
        JobError::Fs(e) => fs_reason(e),
        JobError::Panic => "panic".to_owned(),
        JobError::OutOfMemory { .. } => "out of memory".to_owned(),
        JobError::Cancelled => "cancelled".to_owned(),
        JobError::VerifyFailed => "verify failed".to_owned(),
    }
}

/// `FsError` の `{reason}`: 種類の名前を小文字にしたもの (`Conflict { .. }` なら `conflict`)。
pub(crate) fn fs_reason(e: &FsError) -> String {
    format!("{:?}", e.kind)
        .split([' ', '{'])
        .next()
        .unwrap_or("unknown")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FsPath;

    fn fs(kind: FsErrorKind) -> FsError {
        FsError::new(kind)
    }

    // verifies: AC-FR-106-2
    #[test]
    fn ダングリングリンクは専用のメッセージになる() {
        let e = FsError::with_detail(
            FsErrorKind::NotFound,
            FsErrorDetail::DanglingLink {
                target: FsPath::from(r"C:\gone\a.txt"),
            },
        );
        assert_eq!(save_error_key(&e), "msg.save.dangling_link");
    }

    #[test]
    fn 開く失敗のキー() {
        let cases = [
            (
                fs(FsErrorKind::PermissionDenied),
                "msg.open.permission_denied",
            ),
            (fs(FsErrorKind::Busy), "msg.open.busy"),
            (
                FsError::with_detail(FsErrorKind::InvalidPath, FsErrorDetail::NotAFile),
                "msg.open.not_a_file",
            ),
            (
                FsError::with_detail(FsErrorKind::InvalidPath, FsErrorDetail::ReservedName),
                "msg.open.reserved_name",
            ),
            (fs(FsErrorKind::NotFound), "msg.open.failed"),
        ];
        for (e, want) in cases {
            assert_eq!(open_error_key(&e), want, "{e:?}");
        }
        assert_eq!(
            open_failure(&JobError::Cancelled),
            (MessageLevel::Info, "info.open.cancelled")
        );
        assert_eq!(
            open_failure(&JobError::OutOfMemory { requested: 1 }),
            (MessageLevel::Error, "msg.open.out_of_memory")
        );
        assert_eq!(
            open_failure(&JobError::Panic),
            (MessageLevel::Error, "msg.open.failed")
        );
    }

    #[test]
    fn 保存失敗のキー() {
        let backup = || FsPath::from("/w/.a.txt.eve-bak");
        let cases = [
            (fs(FsErrorKind::DiskFull), "msg.save.disk_full"),
            (
                fs(FsErrorKind::ReplacedMovedToBackup { backup: backup() }),
                "msg.save.moved_to_backup",
            ),
            (
                fs(FsErrorKind::InPlaceFailed {
                    backup: Some(backup()),
                }),
                "msg.save.in_place_failed",
            ),
            (
                fs(FsErrorKind::InPlaceFailed { backup: None }),
                "msg.save.in_place_failed_no_backup",
            ),
            (
                FsError::with_detail(
                    FsErrorKind::NotFound,
                    FsErrorDetail::ParentMissing {
                        dir: FsPath::from("/w"),
                    },
                ),
                "msg.save.parent_missing",
            ),
            (FsError::conflict(None), "msg.save.failed"),
        ];
        for (e, want) in cases {
            assert_eq!(save_error_key(&e), want, "{e:?}");
        }
        assert_eq!(
            save_failure(&JobError::Cancelled),
            (MessageLevel::Info, "info.save.aborted")
        );
        assert_eq!(
            save_failure(&JobError::VerifyFailed),
            (MessageLevel::Error, "msg.save.verify_failed")
        );
    }

    #[test]
    fn 理由の分類名() {
        assert_eq!(fs_reason(&FsError::conflict(None)), "conflict");
        assert_eq!(
            fs_reason(&fs(FsErrorKind::InPlaceFailed { backup: None })),
            "inplacefailed"
        );
        assert_eq!(reason_label(&JobError::Fs(fs(FsErrorKind::Busy))), "busy");
        assert_eq!(
            reason_label(&JobError::OutOfMemory { requested: 1 }),
            "out of memory"
        );
    }
}
