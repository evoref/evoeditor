# Changelog

[日本語](CHANGELOG.md) | English

This file records the changes of each version and is the source of the release notes (docs/12-licensing-distribution.md, docs/09-testing-quality.md §9).
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

Before v0.1.0, `0.0.x` was an internal counter of milestones; no tags or Releases were made for it (docs/11-dev-workflow.md §9.1).

The Japanese version ([CHANGELOG.md](CHANGELOG.md)) is authoritative. Both are updated in the same pull request.

## [Unreleased]

### Added

- **Automatic saving of unsaved content (swap) and recovery** (v0.2, in development): edited content is written to the state folder (`%LOCALAPPDATA%\evoeditor\swap\`) about 2 seconds after the last input. After a forced termination or power loss, opening the same file (or starting untitled) asks "Apply recovered content / Discard swap". Applying does not change the file until you save, and can be undone. Documents larger than 64 MiB, read-only documents, and files that may contain secrets (`.env`, `*.pem`, `id_rsa*`, and so on) are excluded. Settings are `enabled` / `interval_ms` / `min_interval_ms` / `max_bytes` / `exclude_secret_files` in `[file.swap]`.
- **English user interface** (v0.2, in development): the user interface can be shown in English. The language is chosen in the order `--locale ja|en` > setting `[ui] locale` > environment variable `EVE_LANG` > the OS display language; if none applies, Japanese. Regions can be included, as in `ja-JP` or `en_US`. A language that is not bundled is logged as a warning and the next candidate is tried. The stderr lines of `--help`, usage errors, panics and exit always show the English (ASCII) line first and the Japanese line indented by two spaces next, regardless of the chosen language.
- **Process-wide limit of the undo history** (v0.2, in development): when the total history of all documents exceeds the limit (default 256 MiB, setting `[editor.undo] total_max_bytes`), the oldest entries of the document with the largest history are discarded first. The last operation is kept, and discarding does not change the unsaved indicator.

### Fixed

- **Secret-file marker of emergency rescues**: when a document whose file name may contain secrets (`.env`, `*.pem`, `id_rsa*`, `.ssh/**`, and so on, docs/07 §5.12.2) was rescued, `sensitive = true` in the manifest and the "please delete it" note on exit were missing. They are now shown. The rescue itself already happened in v0.1.0, and no data was lost.
- **Emergency rescue gave up the rest after one failed write**: with several documents to rescue, a failed write of one (permissions, locking) ended the rescue without the documents after it. Now only the failed document is marked `written = false`, and the rescue continues for the rest (only a timeout stops it). This prepares for multiple documents in v0.2; it could not happen in v0.1.0 with a single document.
- **Keep the content of a closed document that is being saved in the emergency rescue**: closing a document while its save was running removed its content from the rescue targets. It is now kept until the result of the save arrives, and written to the rescue location if the save fails.

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
