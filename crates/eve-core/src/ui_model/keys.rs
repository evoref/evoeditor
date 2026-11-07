//! eve-core が使う文言キー (MessageKey、04 §15.2 の目録。docs/review/v0.3-findings.md の STR-B4 (3))。
//! 製品のコードはキーを文字列で書かず、ここの定数を使う。`msg.*` / `info.*` はメッセージの基の名前
//! (`.text` / `.hint` を付けずに `push_message` などに渡す名前)。どの定数も同梱の `ja` と `en` にあることを
//! 試験で確かめる (キーが無いと文言の代わりにキー名が出る)。

pub(crate) const HINT_UI_CANCEL: &str = "hint.ui.cancel";
pub(crate) const INFO_APP_RELEASE_SUMMARY: &str = "info.app.release_summary";
pub(crate) const INFO_APP_UPDATED: &str = "info.app.updated";
pub(crate) const INFO_CONFIG_RELOAD_RESTART: &str = "info.config.reload_restart";
pub(crate) const INFO_CONFIG_RELOAD_UNCHANGED: &str = "info.config.reload_unchanged";
pub(crate) const INFO_CONFIG_RELOADED: &str = "info.config.reloaded";
pub(crate) const INFO_CONFIG_UNSUPPORTED: &str = "info.config.unsupported";
pub(crate) const INFO_EOL_CONVERTED: &str = "info.eol.converted";
pub(crate) const INFO_EOL_UNCHANGED: &str = "info.eol.unchanged";
pub(crate) const INFO_EXTERNAL_UNCHANGED: &str = "info.external.unchanged";
pub(crate) const INFO_GOTO_LINE_CLAMPED: &str = "info.goto_line.clamped";
pub(crate) const INFO_HINT_BAR_HIDDEN: &str = "info.hint_bar.hidden";
pub(crate) const INFO_HINT_BAR_SHOWN: &str = "info.hint_bar.shown";
pub(crate) const INFO_HISTORY_COPIED: &str = "info.history.copied";
pub(crate) const INFO_LEFTOVER_DELETED: &str = "info.leftover.deleted";
pub(crate) const INFO_LEFTOVER_RESTORED: &str = "info.leftover.restored";
pub(crate) const INFO_MOUSE_DISABLED: &str = "info.mouse.disabled";
pub(crate) const INFO_MOUSE_ENABLED: &str = "info.mouse.enabled";
pub(crate) const INFO_OPEN_ALREADY_OPEN: &str = "info.open.already_open";
pub(crate) const INFO_OPEN_CANCELLED: &str = "info.open.cancelled";
pub(crate) const INFO_OPEN_GUESS_UNSUPPORTED: &str = "info.open.guess_unsupported";
pub(crate) const INFO_OPEN_LONG_LINE: &str = "info.open.long_line";
pub(crate) const INFO_OPEN_LOSSY_ROUNDTRIP: &str = "info.open.lossy_roundtrip";
pub(crate) const INFO_OPEN_NEW_FILE: &str = "info.open.new_file";
pub(crate) const INFO_OPEN_REDETECTED: &str = "info.open.redetected";
pub(crate) const INFO_READ_ONLY_RELEASED: &str = "info.read_only.released";
pub(crate) const INFO_READ_ONLY_SET: &str = "info.read_only.set";
pub(crate) const INFO_RELOAD_BUSY: &str = "info.reload.busy";
pub(crate) const INFO_RELOAD_DONE: &str = "info.reload.done";
pub(crate) const INFO_RELOAD_DONE_READ_ONLY: &str = "info.reload.done_read_only";
pub(crate) const INFO_REOPEN_HISTORY_CLEARED: &str = "info.reopen.history_cleared";
pub(crate) const INFO_SAVE_ABORTED: &str = "info.save.aborted";
pub(crate) const INFO_SAVE_DONE: &str = "info.save.done";
pub(crate) const INFO_SAVE_IN_PLACE: &str = "info.save.in_place";
pub(crate) const INFO_SAVE_IN_PROGRESS: &str = "info.save.in_progress";
pub(crate) const INFO_SAVE_TO_RECOVERY: &str = "info.save.to_recovery";
pub(crate) const INFO_SAVE_UNCHANGED: &str = "info.save.unchanged";
pub(crate) const INFO_SAVE_ALL_DONE: &str = "info.save_all.done";
pub(crate) const INFO_SAVE_ALL_DONE_SKIPPED: &str = "info.save_all.done_skipped";
pub(crate) const INFO_SAVE_ALL_NONE: &str = "info.save_all.none";
pub(crate) const INFO_SEARCH_NO_SELECTION: &str = "info.search.no_selection";
pub(crate) const INFO_SEARCH_REPLACE_EMPTY_QUERY: &str = "info.search.replace_empty_query";
pub(crate) const INFO_SEARCH_REPLACE_NO_MATCH: &str = "info.search.replace_no_match";
pub(crate) const INFO_SEARCH_REPLACED: &str = "info.search.replaced";
pub(crate) const INFO_SEARCH_WRAPPED_BACKWARD: &str = "info.search.wrapped_backward";
pub(crate) const INFO_SEARCH_WRAPPED_FORWARD: &str = "info.search.wrapped_forward";
pub(crate) const INFO_SWAP_DISCARDED: &str = "info.swap.discarded";
pub(crate) const INFO_SWAP_RECOVERED: &str = "info.swap.recovered";
pub(crate) const INFO_UNDO_HISTORY_TRIMMED: &str = "info.undo.history_trimmed";
pub(crate) const MSG_CLIPBOARD_FALLBACK: &str = "msg.clipboard.fallback";
pub(crate) const MSG_CONFIG_PROBLEM: &str = "msg.config.problem";
pub(crate) const MSG_CONFIG_PROBLEMS: &str = "msg.config.problems";
pub(crate) const MSG_CONFIG_RELOAD_KEPT: &str = "msg.config.reload_kept";
pub(crate) const MSG_EDIT_READ_ONLY: &str = "msg.edit.read_only";
pub(crate) const MSG_EOL_CONVERT_STALE: &str = "msg.eol.convert_stale";
pub(crate) const MSG_EXTERNAL_CHECK_FAILED: &str = "msg.external.check_failed";
pub(crate) const MSG_LEFTOVER_FAILED: &str = "msg.leftover.failed";
pub(crate) const MSG_LEFTOVER_TMP_OPENED: &str = "msg.leftover.tmp_opened";
pub(crate) const MSG_OPEN_BUSY: &str = "msg.open.busy";
pub(crate) const MSG_OPEN_CHANGED_DURING_LOAD: &str = "msg.open.changed_during_load";
pub(crate) const MSG_OPEN_FAILED: &str = "msg.open.failed";
pub(crate) const MSG_OPEN_INTERRUPTED_NEW: &str = "msg.open.interrupted_new";
pub(crate) const MSG_OPEN_INTERRUPTED_SAVE: &str = "msg.open.interrupted_save";
pub(crate) const MSG_OPEN_INTERRUPTED_SAVE_INCOMPLETE: &str =
    "msg.open.interrupted_save_incomplete";
pub(crate) const MSG_OPEN_INTERRUPTED_SAVE_NO_BACKUP: &str = "msg.open.interrupted_save_no_backup";
pub(crate) const MSG_OPEN_NOT_A_FILE: &str = "msg.open.not_a_file";
pub(crate) const MSG_OPEN_OUT_OF_MEMORY: &str = "msg.open.out_of_memory";
pub(crate) const MSG_OPEN_PARENT_MISSING: &str = "msg.open.parent_missing";
pub(crate) const MSG_OPEN_PERMISSION_DENIED: &str = "msg.open.permission_denied";
pub(crate) const MSG_OPEN_RESERVED_NAME: &str = "msg.open.reserved_name";
pub(crate) const MSG_RECOVERY_FOUND: &str = "msg.recovery.found";
pub(crate) const MSG_RECOVERY_FOUND_RESCUE: &str = "msg.recovery.found_rescue";
pub(crate) const MSG_RELOAD_MISSING: &str = "msg.reload.missing";
pub(crate) const MSG_SAVE_ATTRIBUTES_NOT_COPIED: &str = "msg.save.attributes_not_copied";
pub(crate) const MSG_SAVE_BACKUP_NOT_REMOVED: &str = "msg.save.backup_not_removed";
pub(crate) const MSG_SAVE_BUSY: &str = "msg.save.busy";
pub(crate) const MSG_SAVE_CHANGED_AFTER_SAVE: &str = "msg.save.changed_after_save";
pub(crate) const MSG_SAVE_CHECK_FAILED: &str = "msg.save.check_failed";
pub(crate) const MSG_SAVE_DANGLING_LINK: &str = "msg.save.dangling_link";
pub(crate) const MSG_SAVE_DISK_FULL: &str = "msg.save.disk_full";
pub(crate) const MSG_SAVE_ENCODING_UNSUPPORTED_ISO2022: &str =
    "msg.save.encoding_unsupported_iso2022";
pub(crate) const MSG_SAVE_FAILED: &str = "msg.save.failed";
pub(crate) const MSG_SAVE_FSYNC_FAILED: &str = "msg.save.fsync_failed";
pub(crate) const MSG_SAVE_IN_PLACE_FAILED: &str = "msg.save.in_place_failed";
pub(crate) const MSG_SAVE_IN_PLACE_FAILED_NO_BACKUP: &str = "msg.save.in_place_failed_no_backup";
pub(crate) const MSG_SAVE_INFLIGHT_FLAG_NOT_WRITTEN: &str = "msg.save.inflight_flag_not_written";
pub(crate) const MSG_SAVE_INVALID_PATH: &str = "msg.save.invalid_path";
pub(crate) const MSG_SAVE_MOVED_TO_BACKUP: &str = "msg.save.moved_to_backup";
pub(crate) const MSG_SAVE_PARENT_MISSING: &str = "msg.save.parent_missing";
pub(crate) const MSG_SAVE_PERMISSION_DENIED: &str = "msg.save.permission_denied";
pub(crate) const MSG_SAVE_PERMISSION_NARROWED: &str = "msg.save.permission_narrowed";
pub(crate) const MSG_SAVE_READ_ONLY_FS: &str = "msg.save.read_only_fs";
pub(crate) const MSG_SAVE_RECOVERY_FAILED: &str = "msg.save.recovery_failed";
pub(crate) const MSG_SAVE_TMP_NOT_REMOVED: &str = "msg.save.tmp_not_removed";
pub(crate) const MSG_SAVE_AS_SAME_FILE_READ_ONLY: &str = "msg.save_as.same_file_read_only";
pub(crate) const MSG_SEARCH_INVALID_REGEX_TEXT: &str = "msg.search.invalid_regex.text";
pub(crate) const MSG_SEARCH_NEWLINE_UNSUPPORTED_TEXT: &str = "msg.search.newline_unsupported.text";
pub(crate) const MSG_SEARCH_REPLACE_STALE: &str = "msg.search.replace_stale";
pub(crate) const MSG_SWAP_CHANGED_SINCE_OFFER: &str = "msg.swap.changed_since_offer";
pub(crate) const MSG_SWAP_FOUND: &str = "msg.swap.found";
pub(crate) const MSG_SWAP_OTHER_INSTANCE: &str = "msg.swap.other_instance";
pub(crate) const MSG_SWAP_READ_FAILED: &str = "msg.swap.read_failed";
pub(crate) const MSG_SWAP_REMOVE_FAILED: &str = "msg.swap.remove_failed";
pub(crate) const MSG_SWAP_TOO_LARGE: &str = "msg.swap.too_large";
pub(crate) const MSG_SWAP_UNREADABLE: &str = "msg.swap.unreadable";
pub(crate) const STATUS_DEGRADED_LONG_LINE_LONG: &str = "status.degraded.long_line.long";
pub(crate) const STATUS_DEGRADED_LONG_LINE_SHORT: &str = "status.degraded.long_line.short";
pub(crate) const STATUS_ENCODING_BOM: &str = "status.encoding.bom";
pub(crate) const STATUS_EOL_MIXED: &str = "status.eol_mixed";
pub(crate) const STATUS_EXTERNAL_CHANGE_LONG: &str = "status.external_change.long";
pub(crate) const STATUS_EXTERNAL_CHANGE_SHORT: &str = "status.external_change.short";
pub(crate) const STATUS_LOADING_LONG: &str = "status.loading.long";
pub(crate) const STATUS_LOADING_SHORT: &str = "status.loading.short";
pub(crate) const STATUS_POSITION_LONG: &str = "status.position.long";
pub(crate) const STATUS_POSITION_SHORT: &str = "status.position.short";
pub(crate) const STATUS_READ_ONLY_LONG: &str = "status.read_only.long";
pub(crate) const STATUS_READ_ONLY_SHORT: &str = "status.read_only.short";
pub(crate) const STATUS_SAVING_LONG: &str = "status.saving.long";
pub(crate) const STATUS_SAVING_SHORT: &str = "status.saving.short";
pub(crate) const STATUS_SELECTION: &str = "status.selection";
pub(crate) const STATUS_UNTITLED: &str = "status.untitled";
pub(crate) const UI_BUFFER_LIST_NO_MATCH: &str = "ui.buffer_list.no_match";
pub(crate) const UI_BUFFER_LIST_PLACEHOLDER: &str = "ui.buffer_list.placeholder";
pub(crate) const UI_BUFFER_LIST_TITLE: &str = "ui.buffer_list.title";
pub(crate) const UI_CONFIRM_CANCEL: &str = "ui.confirm.cancel";
pub(crate) const UI_CONFIRM_NAV_HINT: &str = "ui.confirm.nav_hint";
pub(crate) const UI_CONFIRM_QUIT_SAVING_OTHERS: &str = "ui.confirm.quit_saving.others";
pub(crate) const UI_CONFIRM_SAVE_UNMAPPABLE_REPLACE_ITEM: &str =
    "ui.confirm.save.unmappable_replace.item";
pub(crate) const UI_CONFIRM_SAVE_UNMAPPABLE_REPLACE_MORE: &str =
    "ui.confirm.save.unmappable_replace.more";
pub(crate) const UI_CONFIRM_SAVE_ALL_FAILED_NEEDS_CONFIRM: &str =
    "ui.confirm.save_all_failed.needs_confirm";
pub(crate) const UI_CONFIRM_SWAP_RECOVERY_CHANGED: &str = "ui.confirm.swap_recovery.changed";
pub(crate) const UI_CONFIRM_SWAP_RECOVERY_SENSITIVE: &str = "ui.confirm.swap_recovery.sensitive";
pub(crate) const UI_HELP_FOOTER: &str = "ui.help.footer";
pub(crate) const UI_HELP_INTRO_PALETTE: &str = "ui.help.intro.palette";
pub(crate) const UI_HELP_INTRO_QUIT: &str = "ui.help.intro.quit";
pub(crate) const UI_HELP_INTRO_SAVE: &str = "ui.help.intro.save";
pub(crate) const UI_HELP_KEYS_NOTE: &str = "ui.help.keys.note";
pub(crate) const UI_HELP_KEYS_UNREACHABLE: &str = "ui.help.keys.unreachable";
pub(crate) const UI_HELP_SECTION_COMMANDS: &str = "ui.help.section.commands";
pub(crate) const UI_HELP_SECTION_INTRO: &str = "ui.help.section.intro";
pub(crate) const UI_HELP_SECTION_KEYS: &str = "ui.help.section.keys";
pub(crate) const UI_HELP_SECTION_TERMINAL: &str = "ui.help.section.terminal";
pub(crate) const UI_HELP_TERMINAL_ALT: &str = "ui.help.terminal.alt";
pub(crate) const UI_HELP_TERMINAL_CHORD: &str = "ui.help.terminal.chord";
pub(crate) const UI_HELP_TERMINAL_IME: &str = "ui.help.terminal.ime";
pub(crate) const UI_HELP_TERMINAL_README: &str = "ui.help.terminal.readme";
pub(crate) const UI_HELP_TITLE: &str = "ui.help.title";
pub(crate) const UI_HISTORY_EMPTY: &str = "ui.history.empty";
pub(crate) const UI_HISTORY_FOOTER: &str = "ui.history.footer";
pub(crate) const UI_HISTORY_LEVEL_ERROR: &str = "ui.history.level.error";
pub(crate) const UI_HISTORY_LEVEL_INFO: &str = "ui.history.level.info";
pub(crate) const UI_HISTORY_LEVEL_WARN: &str = "ui.history.level.warn";
pub(crate) const UI_HISTORY_TITLE: &str = "ui.history.title";
pub(crate) const UI_PALETTE_ARG_ENCODING: &str = "ui.palette.arg.encoding";
pub(crate) const UI_PALETTE_ARG_EOL: &str = "ui.palette.arg.eol";
pub(crate) const UI_PALETTE_ARG_TITLE: &str = "ui.palette.arg_title";
pub(crate) const UI_PALETTE_DISABLED: &str = "ui.palette.disabled";
pub(crate) const UI_PALETTE_FOOTER: &str = "ui.palette.footer";
pub(crate) const UI_PALETTE_NO_MATCH: &str = "ui.palette.no_match";
pub(crate) const UI_PALETTE_PLACEHOLDER: &str = "ui.palette.placeholder";
pub(crate) const UI_PALETTE_TITLE: &str = "ui.palette.title";
pub(crate) const UI_SEARCH_COUNT_APPROX: &str = "ui.search.count.approx";
pub(crate) const UI_SEARCH_COUNT_AT_LEAST: &str = "ui.search.count.at_least";
pub(crate) const UI_SEARCH_COUNT_EXACT: &str = "ui.search.count.exact";
pub(crate) const UI_SEARCH_COUNT_NO_MATCH: &str = "ui.search.count.no_match";
pub(crate) const UI_SEARCH_COUNT_RUNNING: &str = "ui.search.count.running";
pub(crate) const UI_SEARCH_LABEL: &str = "ui.search.label";
pub(crate) const UI_SEARCH_REPLACE_ALL_BUTTON: &str = "ui.search.replace_all_button";
pub(crate) const UI_SEARCH_REPLACE_LABEL: &str = "ui.search.replace_label";
pub(crate) const UI_TOO_SMALL: &str = "ui.too_small";

/// すべての定数 (キーの順。試験に使う)。
#[cfg(test)]
const ALL: [&str; 170] = [
    HINT_UI_CANCEL,
    INFO_APP_RELEASE_SUMMARY,
    INFO_APP_UPDATED,
    INFO_CONFIG_RELOAD_RESTART,
    INFO_CONFIG_RELOAD_UNCHANGED,
    INFO_CONFIG_RELOADED,
    INFO_CONFIG_UNSUPPORTED,
    INFO_EOL_CONVERTED,
    INFO_EOL_UNCHANGED,
    INFO_EXTERNAL_UNCHANGED,
    INFO_GOTO_LINE_CLAMPED,
    INFO_HINT_BAR_HIDDEN,
    INFO_HINT_BAR_SHOWN,
    INFO_HISTORY_COPIED,
    INFO_LEFTOVER_DELETED,
    INFO_LEFTOVER_RESTORED,
    INFO_MOUSE_DISABLED,
    INFO_MOUSE_ENABLED,
    INFO_OPEN_ALREADY_OPEN,
    INFO_OPEN_CANCELLED,
    INFO_OPEN_GUESS_UNSUPPORTED,
    INFO_OPEN_LONG_LINE,
    INFO_OPEN_LOSSY_ROUNDTRIP,
    INFO_OPEN_NEW_FILE,
    INFO_OPEN_REDETECTED,
    INFO_READ_ONLY_RELEASED,
    INFO_READ_ONLY_SET,
    INFO_RELOAD_BUSY,
    INFO_RELOAD_DONE,
    INFO_RELOAD_DONE_READ_ONLY,
    INFO_REOPEN_HISTORY_CLEARED,
    INFO_SAVE_ABORTED,
    INFO_SAVE_DONE,
    INFO_SAVE_IN_PLACE,
    INFO_SAVE_IN_PROGRESS,
    INFO_SAVE_TO_RECOVERY,
    INFO_SAVE_UNCHANGED,
    INFO_SAVE_ALL_DONE,
    INFO_SAVE_ALL_DONE_SKIPPED,
    INFO_SAVE_ALL_NONE,
    INFO_SEARCH_NO_SELECTION,
    INFO_SEARCH_REPLACE_EMPTY_QUERY,
    INFO_SEARCH_REPLACE_NO_MATCH,
    INFO_SEARCH_REPLACED,
    INFO_SEARCH_WRAPPED_BACKWARD,
    INFO_SEARCH_WRAPPED_FORWARD,
    INFO_SWAP_DISCARDED,
    INFO_SWAP_RECOVERED,
    INFO_UNDO_HISTORY_TRIMMED,
    MSG_CLIPBOARD_FALLBACK,
    MSG_CONFIG_PROBLEM,
    MSG_CONFIG_PROBLEMS,
    MSG_CONFIG_RELOAD_KEPT,
    MSG_EDIT_READ_ONLY,
    MSG_EOL_CONVERT_STALE,
    MSG_EXTERNAL_CHECK_FAILED,
    MSG_LEFTOVER_FAILED,
    MSG_LEFTOVER_TMP_OPENED,
    MSG_OPEN_BUSY,
    MSG_OPEN_CHANGED_DURING_LOAD,
    MSG_OPEN_FAILED,
    MSG_OPEN_INTERRUPTED_NEW,
    MSG_OPEN_INTERRUPTED_SAVE,
    MSG_OPEN_INTERRUPTED_SAVE_INCOMPLETE,
    MSG_OPEN_INTERRUPTED_SAVE_NO_BACKUP,
    MSG_OPEN_NOT_A_FILE,
    MSG_OPEN_OUT_OF_MEMORY,
    MSG_OPEN_PARENT_MISSING,
    MSG_OPEN_PERMISSION_DENIED,
    MSG_OPEN_RESERVED_NAME,
    MSG_RECOVERY_FOUND,
    MSG_RECOVERY_FOUND_RESCUE,
    MSG_RELOAD_MISSING,
    MSG_SAVE_ATTRIBUTES_NOT_COPIED,
    MSG_SAVE_BACKUP_NOT_REMOVED,
    MSG_SAVE_BUSY,
    MSG_SAVE_CHANGED_AFTER_SAVE,
    MSG_SAVE_CHECK_FAILED,
    MSG_SAVE_DANGLING_LINK,
    MSG_SAVE_DISK_FULL,
    MSG_SAVE_ENCODING_UNSUPPORTED_ISO2022,
    MSG_SAVE_FAILED,
    MSG_SAVE_FSYNC_FAILED,
    MSG_SAVE_IN_PLACE_FAILED,
    MSG_SAVE_IN_PLACE_FAILED_NO_BACKUP,
    MSG_SAVE_INFLIGHT_FLAG_NOT_WRITTEN,
    MSG_SAVE_INVALID_PATH,
    MSG_SAVE_MOVED_TO_BACKUP,
    MSG_SAVE_PARENT_MISSING,
    MSG_SAVE_PERMISSION_DENIED,
    MSG_SAVE_PERMISSION_NARROWED,
    MSG_SAVE_READ_ONLY_FS,
    MSG_SAVE_RECOVERY_FAILED,
    MSG_SAVE_TMP_NOT_REMOVED,
    MSG_SAVE_AS_SAME_FILE_READ_ONLY,
    MSG_SEARCH_INVALID_REGEX_TEXT,
    MSG_SEARCH_NEWLINE_UNSUPPORTED_TEXT,
    MSG_SEARCH_REPLACE_STALE,
    MSG_SWAP_CHANGED_SINCE_OFFER,
    MSG_SWAP_FOUND,
    MSG_SWAP_OTHER_INSTANCE,
    MSG_SWAP_READ_FAILED,
    MSG_SWAP_REMOVE_FAILED,
    MSG_SWAP_TOO_LARGE,
    MSG_SWAP_UNREADABLE,
    STATUS_DEGRADED_LONG_LINE_LONG,
    STATUS_DEGRADED_LONG_LINE_SHORT,
    STATUS_ENCODING_BOM,
    STATUS_EOL_MIXED,
    STATUS_EXTERNAL_CHANGE_LONG,
    STATUS_EXTERNAL_CHANGE_SHORT,
    STATUS_LOADING_LONG,
    STATUS_LOADING_SHORT,
    STATUS_POSITION_LONG,
    STATUS_POSITION_SHORT,
    STATUS_READ_ONLY_LONG,
    STATUS_READ_ONLY_SHORT,
    STATUS_SAVING_LONG,
    STATUS_SAVING_SHORT,
    STATUS_SELECTION,
    STATUS_UNTITLED,
    UI_BUFFER_LIST_NO_MATCH,
    UI_BUFFER_LIST_PLACEHOLDER,
    UI_BUFFER_LIST_TITLE,
    UI_CONFIRM_CANCEL,
    UI_CONFIRM_NAV_HINT,
    UI_CONFIRM_QUIT_SAVING_OTHERS,
    UI_CONFIRM_SAVE_UNMAPPABLE_REPLACE_ITEM,
    UI_CONFIRM_SAVE_UNMAPPABLE_REPLACE_MORE,
    UI_CONFIRM_SAVE_ALL_FAILED_NEEDS_CONFIRM,
    UI_CONFIRM_SWAP_RECOVERY_CHANGED,
    UI_CONFIRM_SWAP_RECOVERY_SENSITIVE,
    UI_HELP_FOOTER,
    UI_HELP_INTRO_PALETTE,
    UI_HELP_INTRO_QUIT,
    UI_HELP_INTRO_SAVE,
    UI_HELP_KEYS_NOTE,
    UI_HELP_KEYS_UNREACHABLE,
    UI_HELP_SECTION_COMMANDS,
    UI_HELP_SECTION_INTRO,
    UI_HELP_SECTION_KEYS,
    UI_HELP_SECTION_TERMINAL,
    UI_HELP_TERMINAL_ALT,
    UI_HELP_TERMINAL_CHORD,
    UI_HELP_TERMINAL_IME,
    UI_HELP_TERMINAL_README,
    UI_HELP_TITLE,
    UI_HISTORY_EMPTY,
    UI_HISTORY_FOOTER,
    UI_HISTORY_LEVEL_ERROR,
    UI_HISTORY_LEVEL_INFO,
    UI_HISTORY_LEVEL_WARN,
    UI_HISTORY_TITLE,
    UI_PALETTE_ARG_ENCODING,
    UI_PALETTE_ARG_EOL,
    UI_PALETTE_ARG_TITLE,
    UI_PALETTE_DISABLED,
    UI_PALETTE_FOOTER,
    UI_PALETTE_NO_MATCH,
    UI_PALETTE_PLACEHOLDER,
    UI_PALETTE_TITLE,
    UI_SEARCH_COUNT_APPROX,
    UI_SEARCH_COUNT_AT_LEAST,
    UI_SEARCH_COUNT_EXACT,
    UI_SEARCH_COUNT_NO_MATCH,
    UI_SEARCH_COUNT_RUNNING,
    UI_SEARCH_LABEL,
    UI_SEARCH_REPLACE_ALL_BUTTON,
    UI_SEARCH_REPLACE_LABEL,
    UI_TOO_SMALL,
];

#[cfg(test)]
mod tests {
    use eve_config::Catalog;

    #[test]
    fn どの文言キーも同梱の言語ファイルにある() {
        for (lang, cat) in [("ja", Catalog::builtin_ja()), ("en", Catalog::builtin_en())] {
            let missing: Vec<&str> = super::ALL
                .iter()
                .copied()
                .filter(|k| !cat.contains(k) && !cat.contains(&format!("{k}.text")))
                .collect();
            assert!(missing.is_empty(), "{lang} に無いキー: {missing:?}");
        }
    }
}
