# Changelog

[日本語](CHANGELOG.md) | English

This file records the changes of each version and is the source of the release notes (docs/12-licensing-distribution.md, docs/09-testing-quality.md §9).
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

Before v0.1.0, `0.0.x` was an internal counter of milestones; no tags or Releases were made for it (docs/11-dev-workflow.md §9.1).

The Japanese version ([CHANGELOG.md](CHANGELOG.md)) is authoritative. Both are updated in the same pull request.

## [Unreleased]

### Changed

- eve-view: changed the type of `DocumentInfo.read_only` from `Vec<String>` (variant names as strings) to `Vec<ReadOnlyReasonView>` (a new enum) (breaking; the `Query::Documents` answer). Aligned the documentation of `Capabilities.osc52_write` and `InputEvent::Resize` with the implementation (no type change).
- eve-view: added `key_notation(strokes, ascii)`, the function that formats keys for display (addition only; shared by the core and eve-tui, docs/04 §7.8).

### Fixed

- **Some non-setting names in the config file were silently ignored instead of warned about**: writing a name that is not a setting key (a command or a color role name), such as `[ui] palette_open = 1`, gave no warning. The list of reserved keys now matches the spec table, and such names are reported.
- **Moving a multi-line selection up or down broke the selection from the second move**: with lines of different lengths, the selected range shifted from the second move on. The same lines now stay selected however many times you move them.
- **Undoing a line-ending conversion also reverted a later encoding or BOM change**: undo now restores only the line ending (a later save uses the encoding you chose).
- **Reloading a file could move the cursor to a different line**: after the file was changed externally near the top, reload kept the same byte offset and the cursor jumped to another line. It now stays at the same line and column (in characters).
- **The line-ending and long-line counts could go wrong after replacing in many places at once**: after edits that change several places in one step, or that join or split CR and LF, the status bar's line-ending indicator and the long-line count could disagree with the text.
- **Keys were written differently depending on where they appeared**: the help key list, keys inside messages and the display after `Ctrl+K` showed `Ctrl+PageDown` and `Ctrl+Shift+Up`, while the command palette and the hint bar showed `Ctrl+PgDn` and `Del` for the same keys. All places now use `PgUp` / `PgDn`, arrow symbols such as `↑` (`Up` etc. when box drawing is limited to ASCII), and `Delete` / `Insert` / `PrintScreen`.
- **The "characters that cannot be saved" confirmation for Shift_JIS / EUC-JP could point to a later character first**: when a character that changes into another character on saving (such as `¥`) came before a character that cannot be represented, the confirmation showed (and moved to) the later one first. Characters are now reported in document order.
- **`--check-config` printed message keys for settings not supported in this version**: with a reserved section such as `[ai]`, or `preset` directly under `[keys]` in the keymap, `evoeditor --check-config` printed the message key as is (such as `msg.config.unsupported.text`). It now prints "{item} is not supported in this version and was ignored".
- **Reopening with another encoding could lose another document's unsaved changes**: after choosing "Reopen with Encoding" on a document with unsaved changes and choosing "Save" in the confirmation, switching to another document with unsaved changes before the save finished made evoeditor reopen the document you switched to, without asking, discarding its unsaved edits and undo history. Only the saved document is reopened now.
- **Saving a file with invalid bytes in another encoding produced a broken file**: when a file containing invalid bytes was saved in another encoding (such as UTF-16) with "Save with Encoding", evoeditor wrote the original invalid bytes as they were, producing a file in which the following characters were shifted (or merged with the neighboring bytes), and reported that the file was saved. Invalid bytes are now treated as characters that cannot be represented when saving in another encoding: nothing is written and a confirmation is shown. Invalid-byte markers pasted from another file are handled the same way.
- **The automatic backup of unsaved changes (swap) was removed too early, for example after a failed save**: the swap of a document whose save failed for lack of write permission, or of a document switched to read-only with unsaved changes, was removed before the changes were saved, so a forced termination or power loss afterwards lost the edits. The swap is now removed only when the unsaved changes are gone (saved or discarded) and at a normal exit. A swap under the old path also no longer remains after "Save As" changes the path (it could offer to recover stale content at the next start).
- **"Abort Save and Quit" could exit without telling where the original file was moved**: choosing "Abort Save and Quit" while a save was running treated a save that failed after replacing had started (the original file moved to a backup name, or a direct overwrite that failed midway) as cancelled, and exited without showing where the original file was. Such failures are now reported with the backup location at exit even after the cancellation is sent. The failure list of "Save All" also shows the backup location of the original file.
- **Warnings about a successful save were not shown**: evoeditor only showed "Saved" even when the write to the disk could not be confirmed on a network or removable drive (fsync failure), when another program may have changed the file right after the save, when some permissions or attributes could not be carried over, or when the temporary name of the moved original file could not be deleted. The most serious one is now shown as a warning after the save completes.
- **An unexpected error in input handling could leave the terminal unusable**: when the thread that reads input stopped abnormally, the editor did not notice, so the terminal stayed in its special mode without accepting input, and unsaved changes were not backed up. The editor now notices it, backs up unsaved changes, restores the terminal, and exits. It also guards against very large numbers in control sequences from the terminal (such as crafted input).
- **On Linux, saving a file of another group could make it readable by a different group**: when saving a file whose group you do not belong to, evoeditor applied the original permissions (and a shared ACL) to your own group. When the group cannot be kept, the file now gets owner-only permissions with a warning that the permissions for other users became narrower. Also fixed: when the cleanup after a failed save could not delete the temporary file, the mark used to find it at the next start was deleted too; and on some file systems (network, FUSE and others) where an atomic replace is not possible, evoeditor did not proceed to the confirmation for a direct overwrite.
- **Saving read-only documents, and handling around reopening**: fixed the following. Choosing "Save" in a confirmation (such as at exit) for a read-only document with unsaved changes (for example, one switched to read-only) wrote to the same file (it now asks for a name). A save could start while a document was being loaded (during a reopen or reload). The result of an operation started before reopening (such as Replace All) could be applied to the reopened document. After a successful direct overwrite, a failure of only the final status check was reported as "could not be saved" (the save now stays successful with a warning that it could not be confirmed).
- **Crash logs were created readable by other users and kept growing**: the crash log written to the state folder after an abnormal exit is now created readable only by you, like the other files there, and only the latest 20 are kept.
- **Opening or reloading a huge file with too little memory crashed the editor**: when the memory needed for loading could not be allocated, evoeditor terminated without backing up the unsaved changes of other documents. It now checks whether the memory can be allocated before reading and, if not, shows "Loading ... was stopped because memory ran out" and keeps running (a reload keeps the previous content).

## [0.2.0] - 2026-11-04

The first release of v0.2. The only product changes from [0.2.0-rc.1] are dependency updates (`signal-hook` 0.4, `unicode-width` 0.2.3, `encoding_rs` 0.8.44) and an adjustment that keeps the width of the half-width katakana voiced and semi-voiced marks (`ﾞ` / `ﾟ`) at 1 regardless of the `unicode-width` version; there is no user-visible difference. This release fills everyday gaps and **adds Linux (x86_64 / aarch64)**. It adds automatic saving of unsaved content and recovery, multiple documents, regular-expression search and replace, soft wrap, mouse, themes, an English user interface and more. Downloads are the Windows zip and the Linux tar.gz on GitHub Releases; installation, updating and known limitations are in the bundled `README.md` (English: `README.en.md`). When updating from v0.1.0, settings and rescued files keep working as they are.

### Added

- **Linux support**: a statically linked executable (musl, x86_64 and aarch64) that runs on any distribution, distributed as a `tar.gz`. Settings and state use the XDG locations (`~/.config/evoeditor/`, `~/.local/state/evoeditor/`). When the terminal is closed or on `kill` (SIGTERM / SIGHUP / SIGINT), evoeditor rescues unsaved content before quitting, and `Ctrl+K Z` returns to the shell temporarily (come back with `fg`). The clipboard uses `wl-copy` / `xclip` / `xsel`, `powershell.exe` on WSL, and OSC 52 when none of them is available or over SSH. The environments checked are Windows Terminal on WSL2 and tmux.
- **Automatic saving of unsaved content (swap) and recovery**: edited content is written to the state folder about 2 seconds after the last input. After a forced termination or power loss, opening the same file (or starting untitled) asks "Apply recovered content / Discard swap". Applying does not change the file until you save, and can be undone. Documents larger than 64 MiB, read-only documents, and files that may contain secrets (`.env`, `*.pem`, `id_rsa*`, and so on) are excluded. Settings are in `[file.swap]`. Temporary files and backups left by an interrupted save are also found on the next start, with a proposal to restore or clean them up.
- **Multiple documents**: open several files and switch between them (`Ctrl+PageDown` / `Ctrl+PageUp`, list with `Ctrl+E`, close with `Ctrl+W`, save all with `Ctrl+K A`). The command line accepts several paths, `+line` and `path:line:column`. On quit, unsaved documents are confirmed one by one. The undo history is kept under a total limit across all documents (default 256 MiB, `[editor.undo] total_max_bytes`) by discarding the oldest entries.
- **Search and replace**: regular expressions (`Alt+R`), whole words (`Alt+W`) and search within the selection (`Alt+L`), replacing one match at a time (`Enter` in the replace field), and regular-expression replacement (`$1` / `${name}`).
- **Display**: soft wrap (`Alt+Z` / `Ctrl+K O`), showing whitespace (`view.show_whitespace`), built-in themes that paint the background and theme files, switching themes, and an explicit color depth (`truecolor` / `256` / `16` / `none` for `[theme] color_mode`). Lines longer than 1 MiB can now be edited.
- **Editing and input**: duplicate, delete and move lines (`Ctrl+K D` / `Ctrl+K L` / `Alt+Up` / `Alt+Down`), mouse (click, drag to select, wheel), overriding key bindings (`keymap.toml`), the unit that Backspace deletes, a guide to the keys available after `Ctrl+K`, enhanced keyboard protocols (kitty and xterm modifyOtherKeys, on terminals that send keys with `Ctrl+Shift` distinctly), and the clipboard through OSC 52.
- **Help and messages**: a help screen (`Ctrl+K H`, keys and commands), the message history (`Ctrl+K M`), and fuzzy search in the command palette. The first start of a new version shows the highlights of that version in one line (no network access).
- **English user interface**: the user interface can be shown in English. The language is chosen in the order `--locale ja|en` > setting `[ui] locale` > environment variable `EVE_LANG` > the OS display language; if none applies, Japanese. The stderr lines of `--help`, usage errors, panics and exit show the English (ASCII) line first and the Japanese line indented by two spaces next. English versions of the user README and the changelog are bundled.
- **Settings and command line**: reloading settings (`config.reload`), `--set key=value`, `--check-config`, `--dump-config` and `--crash-report`. The thresholds for huge files and long lines (`file.huge_file_warn_bytes`, `file.long_line_threshold`) can be set.
- **Protecting files**: external changes are checked when you come back from another application (focus regained) and with `file.check_external_changes`. Toggling read-only (`file.toggle_read_only`) and "remove read-only and save", creating the target when saving through a symbolic link whose target is missing, keeping extended attributes, ACLs and alternate data streams on Windows, and rescuing on Windows log-off and shutdown.

### Known limitations

See "Known limitations" in the bundled `README.md`. Main ones: macOS is not supported (it cannot be checked on real hardware yet; a later version). On Linux, the clipboard and the Japanese input candidate position on desktop environments (GNOME / KDE) have not been checked. In Windows Terminal on WSL, `Ctrl+Shift+<letter>` is not told apart from `Ctrl+<letter>` (use `Ctrl+Y` to redo). The executable is not signed (SmartScreen, Smart App Control).

### Fixed

- **Secret-file marker of emergency rescues**: when a document whose file name may contain secrets (`.env`, `*.pem`, `id_rsa*`, `.ssh/**`, and so on, docs/07 §5.12.2) was rescued, `sensitive = true` in the manifest and the "please delete it" note on exit were missing. They are now shown. The rescue itself already happened in v0.1.0, and no data was lost.
- **Emergency rescue gave up the rest after one failed write**: with several documents to rescue, a failed write of one (permissions, locking) ended the rescue without the documents after it. Now only the failed document is marked `written = false`, and the rescue continues for the rest (only a timeout stops it). This prepares for multiple documents in v0.2; it could not happen in v0.1.0 with a single document.
- **Keep the content of a closed document that is being saved in the emergency rescue**: closing a document while its save was running removed its content from the rescue targets. It is now kept until the result of the save arrives, and written to the rescue location if the save fails.

## [0.2.0-rc.1] - 2026-11-03

The release candidate of v0.2.0. Its contents are included in [0.2.0].

## [0.1.0] - 2026-10-27

The first release of v0.1 (the minimal core). The product code is unchanged from [0.1.0-rc.1] (only the specification and records changed after rc.1). It is a text editor with a Japanese user interface that runs in Windows Terminal and the console host on Windows 11 (x86_64). The only distribution is the zip on GitHub Releases; installation, updating and known limitations are in the bundled `README.md`.

### Added

- **Reading and writing files**: detects and opens UTF-8 (with or without BOM), UTF-16 (LE/BE), Shift_JIS, EUC-JP and ISO-2022-JP, and saves **in the original encoding, BOM, line endings (LF/CRLF/CR, including mixed) and presence of a final newline**. Also supports reopening with a given encoding, saving with a different encoding and line ending, and reloading (undoable).
- **Safe saving**: atomic saving through a temporary file and replacement, so the original file is not damaged if it stops halfway. Permissions, attributes and alternate data streams are kept, and symbolic links and junctions update their targets. Failures such as hard links, read-only, no write permission, sharing violations and a full disk show the reason and what to do next. "Save to the rescue location" is available when saving is impossible.
- **Detecting external changes**: just before saving, the file's modification time, length and ID are checked, plus a content hash (XXH3-64) for files up to 16 MiB; if another program rewrote the file, you are asked "Overwrite / Save as / Cancel".
- **Characters that cannot be represented**: characters that the target encoding cannot represent are shown with their positions before writing, and you choose "Save as UTF-8 / Replace and save". Nothing is silently replaced. Files with invalid bytes show them as `<0xNN>` and are opened read-only.
- **Large files**: shows loading progress with cancel, and stays responsive to input while loading, saving and searching 100 MB files. Files larger than 512 MiB and files with a high ratio of invalid bytes trigger a warning before opening. Files with a line longer than 1 MiB are opened read-only and show the first 64 KiB.
- **Editing**: undo and redo, indentation (Tab / Shift+Tab), paste (control characters in the text are not executed as commands), the system clipboard (Windows).
- **Search and replace**: plain text search (case toggle, count, highlighting of matches, wrap-around), literal replace all (undone with one undo).
- **Command palette** (`F1` / `Ctrl+P`): the list of commands with their keys and argument choices.
- **Japanese user interface**: widths of full-width characters, half-width katakana and emoji, a setting for ambiguous width (1/2), the IME cursor following the caret, the hint bar, confirmation dialogs, and messages (what happened + what to do).
- **Configuration**: 7 items in `config.toml` (tab width, indentation, encoding and line ending of new files, ambiguous width, color depth). Mistakes are reported without stopping startup. An example is the bundled `config.example.toml`.
- **Preparing for abnormal termination**: on panic, closing the console, and Ctrl+Break, unsaved content is rescued together with its line-ending and encoding information (with a free space check), and the next start tells you where it is.
- **Command line**: `--version`, `--help`, `--readonly`, `--encoding`.
- **Distribution**: a Windows x86_64 zip with `SHA256SUMS`, `THIRD-PARTY-NOTICES.md`, `README.md` and `config.example.toml`. The signature verification steps are in `README.md`.
- Development foundations: the specification (`docs/`), CI (fmt, clippy, tests, E2E, dependency checks, `cargo deny`, DCO, gitleaks), `cargo xtask` (check, check-deps, check-dist, dist, gen-notices, bench, i18n and more), and the release procedure (`release.yml`).

### Known limitations

See "Known limitations (v0.1)" in the bundled `README.md`. The main ones: Windows 11 only (Windows 10, macOS and Linux are not supported), only one open document at a time, no regular expression search, the executable is not signed (SmartScreen, Smart App Control), and unsaved content cannot be rescued on forced termination or power loss.

### Fixed

- Bugs found and fixed in pre-release verification: the encoder panicked on ISO-2022-JP control characters; editing a read-only document was silently ignored; files with paths longer than 260 characters could not be saved; saving to a dangling link gave no reason; a rewrite with the same mtime and length was missed on save; opening the search bar stopped screen drawing (Ctrl+F); key input was slow during a full search of 100 MB; DLL search limited to System32 (DLLs in the executable's folder are not loaded; defense in depth); `ui.redraw` (Ctrl+K R) and some default keys (Shift+Enter and Ctrl+H in search, Ctrl+N / Ctrl+P in the palette, and so on) were not implemented.

## [0.1.0-rc.1] - 2026-10-24

The release candidate of v0.1.0. Its contents are included in [0.1.0].
