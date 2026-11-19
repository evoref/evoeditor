# evoeditor

[日本語](README.md) | English

A text editor written in Rust. It aims to let you edit and save Japanese text safely in Windows terminals (Windows Terminal / the console host) and Linux terminals, keeping the original encoding and line endings.

Supported operating systems are **Windows 11 (x86_64)** and **Linux (x86_64 / aarch64, from v0.2)**.

- License: Apache License 2.0 ([LICENSE](LICENSE), [NOTICE](NOTICE))
- Notices for other software used: [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)
- Bug reports and feedback: GitHub Issues. Do not report vulnerabilities in public issues; use the contact in the repository's `SECURITY.md`.

## What it does

- Opens UTF-8 (with or without BOM), UTF-16, Shift_JIS, EUC-JP and ISO-2022-JP files and saves them **in the original encoding and line endings**. The parts you did not edit are written with the bytes they had when the file was opened (characters that have more than one byte representation, such as the NEC special characters and IBM extensions of Shift_JIS, EUC-JP supplementary kanji, and ISO-2022-JP escape sequences and lines with half-width katakana, do not change either). If some characters cannot be represented, it tells you before saving.
- Files with invalid bytes (shown like `<0x9A>`) open read-only and can be edited after a confirmation. When saved in the same encoding, the invalid bytes stay as the original byte values.
- Saving is atomic (if it stops halfway, the original file is not damaged). Before saving, it checks whether another program changed the file. It also checks when you come back from another app (when the terminal reports that it regained focus), and reloads the file if you have not edited it. A document for which you chose "Ignore" on such a change shows `[Changed on disk]` in the status bar.
- It writes what you are editing to a swap file about 2 seconds after your last input. If you open the same file after a forced termination or a power loss, it offers to recover the content. On abnormal termination, when the terminal is closed, and on logoff, it rescues unsaved content.
- You can open several documents and switch between them.
- Search (regular expressions, whole words, within the selection) and replace (one at a time or all). Regular expressions can match across lines with `\n` (see "Writing search patterns" in the help). CP932 compatible characters such as `〜` and `～` are searched as the same character (turn this off with `cp932_compat_fold` under `[search]`).
- For huge files, when loading takes a while, the beginning of the file is shown first, and input is accepted while loading and searching. Lines longer than 1 MiB can be edited too, with the same character units as normal lines.
- It can read standard input (`cmd | evoeditor -`).
- Undo and redo, duplicating and moving lines, soft wrap, mouse, themes (including high-contrast ones), whitespace display, and changing key bindings (`keymap.toml`).
- The user interface is in Japanese or English (`--locale ja|en`, the setting `[ui] locale`). With language files, you can change the text or add a language.
- The help screen (`Ctrl+K H`) lists the keys and commands.

Main keys (the command palette is `F1` or `Ctrl+P`; it shows the key assigned to each command):

| Key | Action |
|---|---|
| `Ctrl+S` | Save (asks for a name for an untitled document) |
| `Ctrl+O` | Open a file |
| `Ctrl+F` | Search (`Ctrl+R` to replace) |
| `Ctrl+Z` | Undo |
| `Ctrl+G` | Go to line |
| `Ctrl+E` | List of open documents (`Ctrl+PageDown` / `Ctrl+PageUp` for next / previous, `Ctrl+W` to close) |
| `Ctrl+K H` | Help (list of keys and commands) |
| `Ctrl+Q` | Quit |

## Installation

There is no installer. It writes nothing to the registry or system settings. Administrator rights are not required.

### Windows

1. **Download**: from the page of the version on GitHub Releases, download these three files into the same folder:
   - `evoeditor-<version>-x86_64-pc-windows-msvc.zip`
   - `SHA256SUMS`
   - `SHA256SUMS.minisig`
2. **Verify** (do not continue if they do not match):
   1. Verify the signature of `SHA256SUMS` with [minisign](https://jedisct1.github.io/minisign/). The public key is under "Verifying the signature" below.

      ```powershell
      minisign -Vm SHA256SUMS -P RWRtkrzH70nKORpBFPEtLy6elo0xP5XsEli9SU/OaxnBmNM8o14GKMhv
      ```

   2. Compute the hash of the zip and check that it matches the corresponding line of `SHA256SUMS`.

      ```powershell
      Get-FileHash -Algorithm SHA256 .\evoeditor-<version>-x86_64-pc-windows-msvc.zip
      ```

3. **Unblock**: files downloaded with a browser are marked as "came from the internet". Remove the mark **after the verification in step 2 and before extracting**. Do not remove the mark from files you have not verified.

   ```powershell
   Unblock-File .\evoeditor-<version>-x86_64-pc-windows-msvc.zip
   ```

4. **Extract**: extract into a per-user folder. Do not use it from the download folder (you would lose track of what to replace when updating).

   ```powershell
   Expand-Archive .\evoeditor-<version>-x86_64-pc-windows-msvc.zip -DestinationPath "$env:LOCALAPPDATA\Programs\evoeditor"
   ```

5. **Add to PATH**: add `%LOCALAPPDATA%\Programs\evoeditor` to your user environment variable `Path` (from the "Edit environment variables" screen, or in PowerShell as below). Do not use `setx` (it truncates long `Path` values). After adding it, **reopen the terminal**.

   ```powershell
   $p = [Environment]::GetEnvironmentVariable('Path', 'User')
   [Environment]::SetEnvironmentVariable('Path', "$p;$env:LOCALAPPDATA\Programs\evoeditor", 'User')
   ```

6. **Check**: in a new terminal, run `evoeditor --version` and check that the version is shown.

If you have a Rust development environment, you can also install with `cargo install --locked --git <repository> --tag v<version> evoeditor`. In that case, set the environment variable `RUSTFLAGS` to `-C target-feature=+crt-static` before running it, to link the C runtime statically.

### Linux

The Linux build is a statically linked executable (musl) that runs on any distribution. The same steps work in WSL.

1. **Download**: from the page of the version on GitHub Releases, download these three files into the same directory. Check `x86_64` or `aarch64` with `uname -m`. The `.debug` file is for investigating bugs and is not needed.
   - `evoeditor-<version>-x86_64-unknown-linux-musl.tar.gz` (or `aarch64-unknown-linux-musl`)
   - `SHA256SUMS`
   - `SHA256SUMS.minisig`
2. **Verify** (do not continue if they do not match): verify the signature with minisign (on Debian / Ubuntu, `sudo apt install minisign`) and check the hash of the archive. The archive matches when its line shows `OK`.

   ```sh
   minisign -Vm SHA256SUMS -P RWRtkrzH70nKORpBFPEtLy6elo0xP5XsEli9SU/OaxnBmNM8o14GKMhv
   sha256sum -c SHA256SUMS --ignore-missing
   ```

3. **Extract**: extract into a per-user directory. The archive has no top-level directory, so always give the destination.

   ```sh
   mkdir -p ~/.local/opt/evoeditor
   tar -xzf evoeditor-<version>-x86_64-unknown-linux-musl.tar.gz -C ~/.local/opt/evoeditor
   ```

4. **PATH**: put a link in `~/.local/bin`. On Debian / Ubuntu, `~/.local/bin` is added to `PATH` when you **log in again** after creating it. If `command -v evoeditor` does not find it, add `export PATH="$HOME/.local/bin:$PATH"` to `~/.profile` (`~/.zprofile` for zsh) and log in again.

   ```sh
   mkdir -p ~/.local/bin
   ln -sf ~/.local/opt/evoeditor/evoeditor ~/.local/bin/evoeditor
   ```

5. **Check**: in a new terminal, run `evoeditor --version` and check that the version is shown.

If you have a Rust development environment, you can also install with `cargo install --locked --git <repository> --tag v<version> evoeditor` (this builds an executable dynamically linked to glibc).

### Starting

```sh
evoeditor                 # a new untitled document
evoeditor memo.txt        # open a file
evoeditor a.txt b.txt     # open several files
evoeditor +12 memo.txt    # open at line 12 (memo.txt:12:5 for line 12, column 5)
evoeditor --readonly a.txt
evoeditor --encoding shift_jis old.txt   # open with a given encoding
git log | evoeditor -     # read standard input (saving it is Save As)
evoeditor --help
```

### Configuration

A configuration file is not required. To change settings, copy the bundled [config.example.toml](config.example.toml) to `%APPDATA%\evoeditor\config.toml` on Windows, or to `~/.config/evoeditor/config.toml` on Linux (under `XDG_CONFIG_HOME` if you set it), and edit it (create the folder if it does not exist). Mistakes do not stop startup; only the affected item falls back to its default, and you are told about it.

### High-contrast themes

Two built-in themes increase the difference in brightness between text and background: `high-contrast-dark` (white text on a black background) and `high-contrast-light` (black text on a white background). They are not selected from the high-contrast setting of the OS; select one yourself.

- **Try one**: `evoeditor --set theme.name=high-contrast-dark` uses it for that run only, without changing the configuration file.
- **Select one**: write, for example, `name = "high-contrast-dark"` under `[theme]` in the configuration file. While the editor is running, "Reload Settings" in the command palette switches to it (restarting also works).
- **Brightness**: choose the theme whose brightness matches your terminal's color scheme. With the opposite one (for example `high-contrast-light` in a terminal with a dark color scheme), the cursor drawn by the terminal can be close to the background color and hard to see.
- **Color depth**: use them with truecolor (24-bit color) or 256 colors. With 16 colors, the colors depend on the terminal's color scheme, so the contrast is only a guide. The depth is detected from the terminal; if it is wrong, set it with `color_mode` under `[theme]` (`"truecolor"` / `"256"` / `"16"` / `"none"`). Setting a depth the terminal cannot draw garbles the colors. A change to `color_mode` takes effect after a restart.
- **Keep `background` at `"auto"`**: `background` under `[theme]` also switches the brightness of the high-contrast themes. The opposite of the name (`"light"` for `high-contrast-dark`) gives the opposite theme, so keep it at `"auto"` (the default) when you select a high-contrast theme.
- **If you mistype the name**: when the theme is not found, the default theme is used, and close names are suggested (for example "There is no theme named high-contrast"). Check with `evoeditor --check-config` or "Show Settings Diagnostics" in the command palette.
- **Theme files with the same name**: your own theme files go in `themes/` in the same folder as the configuration file (`%APPDATA%\evoeditor\themes\` on Windows, `~/.config/evoeditor/themes/` on Linux). A file with the same name as a built-in theme (such as `themes/high-contrast-dark.toml`) is not read, and you are not told. A theme file whose `inherits` names one of them also inherits from the built-in theme. To keep using your own file, rename it and change the `theme.name` / `inherits` that refer to it.

### Display language and language files

Japanese (`ja`) and English (`en`) are built in. The language is chosen in this order: `--locale` > `locale` under `[ui]` in the configuration file > the environment variable `EVE_LANG` > the language of the OS > `ja` (a value with a region, such as `fr-FR`, is looked up by its language part, `fr`). The language is chosen once, at startup.

With a language file, you can change some of the built-in text, or add a language that is not built in. Text you do not write in a language file comes from the built-in text of its **fallback language**: Japanese for `ja.toml`, English for `en.toml`, and English for a language that is not built in (such as `fr.toml`).

- **Location and name**: put it in `locales/` in the same folder as the configuration file (`%APPDATA%\evoeditor\locales\` on Windows, `~/.config/evoeditor/locales/` on Linux), named `<language>.toml`. The name is 2 or 3 lowercase letters for the language only (`fr.toml`, `ko.toml`). Names such as `FR.toml`, `zh-TW.toml`, and `pt_BR.toml` are not read, and you are told about each one. Choosing the language does not read the contents; only the file of the chosen language is read. A file directly in the configuration folder (not in `locales/`), or in `themes/`, is not read. Notepad and similar editors on Windows may save the file as `fr.toml.txt` (show file name extensions in File Explorer to check).
- **Format**: the same TOML format as the built-in language files. Write each text as a string (not as an inline table `{ … }` or an array `[ … ]`). Keys can have up to 8 levels, and a file can have up to 20,000 key levels in total (the full built-in translation has about 3,200). `language` under `[meta]` is optional (if you write it, use the language of the file name). Save the file as UTF-8 (a leading BOM is skipped). The size limit is 1 MiB.
- **Changing some of the text**: write only the keys you change (you do not need to copy the whole file). For example, with the following two lines in `locales/en.toml`, only the name of untitled documents changes, and the text you do not write stays the built-in English. Look up the key names in `locales/ja.toml` / `locales/en.toml` at the tag of your version. The direct location has the form `https://github.com/evoref/evoeditor/blob/v0.4.0/locales/en.toml` (replace `v0.4.0` with the version shown by `evoeditor --version`). Searching `en.toml` for text you see on screen finds its key.

  ```toml
  [status]
  untitled = "New-{n}"
  ```

- **Adding a language**: copy `locales/en.toml` at the same tag to, for example, `locales/fr.toml` and translate it. Text you do not write in the file of a language that is not built in is shown in English. Select it with `--locale fr` or `locale = "fr"` under `[ui]`; if the language of the OS or `EVE_LANG` is `fr`, putting the file there is enough (to go back to English, set `locale = "en"`).
- **Checking**: `evoeditor --check-config --locale fr` shows the language that is selected and the location of the language file in use (the first line; "(not used)" is added when the whole file is not used), and every text that is not used (up to 10,000 per file). For an added language, it also shows the number and the list of keys without a translation. Run it also after you update (it shows keys removed in the new version ("There is no text key named …"), keys added (keys without a translation), and text that fails the checks). At startup, the first problem or the number of problems is shown on the message line, and "Show Settings Diagnostics" in the command palette lists them. Changes to a language file do not appear while the editor is running ("Reload Settings" tells you that they take effect after a restart).
- **Text that is not used**: text that fails one of the following checks is not used; the built-in text (English for an added language) is shown instead, and you are told the file name and the line. Startup does not stop. (1) It is a string. (2) It is at most 1,024 characters. (3) It has a visible character. (4) It has no control characters such as newlines, tabs, and escapes, no formatting characters such as bidirectional controls and direction marks, no private-use characters, and no noncharacters. (5) No more than 30 combining marks in a row. (6) `{` `}` and `〔` `〕` are balanced. (7) The placeholders (the `{…}` parts, such as `{path}`) have the same names as in the built-in text, no more times than it does, and `〔 〕` is used if and only if the built-in text uses it. (8) Bracketed letters such as `[S]` in the choices of a confirmation (the keys to press) are the same letters in the same order as in the built-in text, at the start of a choice, and not fullwidth (`［Ｓ］`). (9) Hint bar words (`hint.*`) should fit in 8 columns (longer ones are used and not reported at startup, but they may be dropped from the hint bar on narrow screens; "Show Settings Diagnostics" and `--check-config` list them as information).
- **When the whole file is not used**: if the file cannot be read, is not a regular file, is not UTF-8, has a syntax error, is over 1 MiB, has an inline table or an array, or has a key with more than 8 levels, the whole file is not used and the built-in text is shown (all in English for an added language).
- **Text that cannot be changed**: the messages printed on exit and on a crash and the `--help` text (`stderr.*`, `panic.*`, `cli.*`) cannot be changed in a language file (if you write them, you are told, and they are not used). Keys that are not in the built-in file (misspelled keys, keys for a newer version) are not used either. The command palette still finds commands by their English titles and command IDs even if you change the titles.
- **Characters that cannot be used**: variation selectors and tag characters cannot be used, so emoji sequences with U+FE0F (such as `⚠️`), ideographic variation sequences (IVS), and regional flags cannot be written. Zero-width non-joiner (ZWNJ) can be used, but it is shown as the `<ZWNJ>` symbol on screen (the width of a hint counts the symbol).
- **Right-to-left languages**: Arabic, Hebrew, and similar languages are not practical in this version, because direction marks cannot be used and terminals do not reorder the text.
- **Check where a language file comes from**: put a language file only after checking where it comes from. The checks look only at the format and the characters, not at the meaning of the translation (for example, whether "Discard" is translated as "Save").
- **When narrowing down a problem**: `--no-config` (`-N`) does not read language files either. If you had selected an added language, the display language changes, so also give a language, as in `evoeditor -N --locale en`.

## Updating

The editor has no built-in update function and does not communicate over the network. Learn about new versions with GitHub Watch (Releases and Security alerts).

1. Download the archive of the new version with `SHA256SUMS` and `SHA256SUMS.minisig`, and **verify the signature and the hash** as in step 2 of "Installation".
2. **Quit all** evoeditor processes (Windows cannot replace a running exe; on Linux, quit first as well so that the version in use is replaced cleanly).
3. Replace the contents of `%LOCALAPPDATA%\Programs\evoeditor` with the files from the extracted folder. On Linux, extract the new archive again into the same destination (`tar -xzf <new archive> -C ~/.local/opt/evoeditor`); the link keeps working.
4. Check the version with `evoeditor --version`.

Keep the archive, `SHA256SUMS` and `SHA256SUMS.minisig` of the version you were using, in case you need to go back.

If you installed with `cargo install`, run `cargo install --locked --git <repository> --tag v<new version> evoeditor` again.

### Settings and rescued content survive updates and downgrades

- The configuration file can be read as is after an update. During v0.x, key names may change between versions. Changes are listed in each version's release notes and in `CHANGELOG.md`. When you go back to an older version, keys that version does not know are ignored with a warning. The configuration file is never rewritten.
- Language files (`locales/`) and theme files (`themes/`) are also read as they are. After updating, run `evoeditor --check-config` to find keys and text that the new version no longer uses. From v0.4, `themes/high-contrast-dark.toml` and `themes/high-contrast-light.toml` are not read, because they have the same names as built-in themes (see "High-contrast themes" above).
- Emergency rescue content is never moved or deleted by any version. If an older version cannot read a rescue, open it with the newer version, or open the `.txt` inside the rescue directly.

### Going back to the previous version

There is no automatic way. The steps are the same as updating: verify the signature and hash of the older archive you kept, replace the files while evoeditor is not running, and check with `evoeditor --version`.

## Uninstalling

Delete the extracted folder (`%LOCALAPPDATA%\Programs\evoeditor`) and remove that folder from the environment variable `Path`. With `cargo install`, run `cargo uninstall evoeditor`. Delete the configuration and state folders (`%APPDATA%\evoeditor`, `%LOCALAPPDATA%\evoeditor`, and the emergency rescues `%TEMP%\eve-recovery-*`) by hand if you no longer need them.

On Linux, delete the destination (`~/.local/opt/evoeditor`) and the link (`~/.local/bin/evoeditor`). Delete the configuration and state directories (`~/.config/evoeditor`, `~/.local/state/evoeditor`, and the emergency rescues `$TMPDIR/eve-recovery-<uid>`) by hand if you no longer need them.

The `recovery/` and `swap/` folders in the state directory may still hold unsaved edits. Check their contents before deleting them.

## Verifying the signature

The `SHA256SUMS` of a release comes with a minisign signature (`SHA256SUMS.minisig`). The public key (the same as `minisign.pub` in the repository; compare the two):

```text
untrusted comment: minisign public key 39CA49EFC7BC926D
RWRtkrzH70nKORpBFPEtLy6elo0xP5XsEli9SU/OaxnBmNM8o14GKMhv
```

## Known limitations

- **The targets are Windows Terminal and the console host on Windows 11 (x86_64), and terminals on Linux (x86_64 / aarch64).** Windows 10 is not officially supported. macOS is not supported (it cannot be checked on real hardware yet; planned after 1.0).
- **The executable is not signed.** SmartScreen may show a warning (usually not, if you verified the files and unblocked them in step 3). Even if a warning appears, after confirming the verification you can choose "More info" → "Run anyway". On Windows 11 with **Smart App Control turned on**, unsigned executables may be refused; this version does not provide a workaround. With "Controlled folder access" enabled, you cannot save into protected folders (such as Documents). If antivirus software flags it by mistake, verify the signature and hash again before restoring it from quarantine, and let us know in Issues.
- **On forced termination or power loss, edits made after the last swap write are lost.** The swap file is written about 2 seconds after your last input. Documents larger than 64 MiB, read-only documents and files that may contain secrets (`.env`, `*.pem`, `id_rsa*` and so on) are not written to swap. The rescue on Windows logoff or shutdown runs within the time the OS waits, so it may not finish; save before logging off. Files on disk are not damaged, thanks to atomic saving.
- **What is still loading from standard input (`cmd | evoeditor -`) is not rescued.** If the terminal or SSH connection is lost while a pipe that does not close (such as `tail -f`) is being read, what is still loading is lost. To keep it, press `Esc` and choose "Open what was read" (content opened that way and content that has finished loading are rescued). Content read with `--readonly` counts as unchanged, so it is not rescued. Other notes (Windows PowerShell 5.1 pipes replace non-ASCII text such as Japanese with `?`, a writing program that reads the terminal competes for keys, and so on) are in the help (`Ctrl+K H`) under "Terminal settings".
- **Files with invalid bytes are opened read-only.** You can edit them after the confirmation of "Toggle read-only" (`file.toggle_read_only`). ISO-2022-JP files with invalid bytes, files opened as binary or huge files, and files with both invalid bytes and private-use characters (U+10FF00 to U+10FFFF) cannot be edited. Invalid bytes can be saved only in the original encoding. If the text looks garbled, reopening with a given encoding may recover it.
- **Read-only files and documents opened with `--readonly` cannot be edited** (editing is refused with the reason). You can edit after saving under another name, or after turning read-only off with `file.toggle_read_only`.
- **Files whose original bytes could not be kept may change some byte representations when saved.** The parts you did not edit are written with their original bytes only for files whose bytes were remembered when they were opened. Files for which this was not possible (a very long line stopped the check, there were too many such places, or the file was recovered from an emergency rescue or a swap file) and ISO-2022-JP lines that end in the Roman or another non-ASCII state are not remembered, so you are asked before saving them in the original encoding (the characters do not change).
- **When saving as ISO-2022-JP, characters you typed that cannot be represented** (half-width katakana, compatibility characters not in JIS, and so on) are listed before saving. Replace them, or change the encoding.
- **Documents with lines longer than 1 MiB are not wrapped** (`[Long lines: no wrap]` in the status bar). Such lines are shown and their columns counted with the same character units as normal lines, but a run of combining characters longer than 4 KiB is split partway, and only then do the columns differ from a normal line.
- **Regular expressions that can span any number of lines** (such as `(?s)a.*b` or `\n+`) are searched only in documents up to 16 MiB (limit the repetition, for example `(?s).{0,1000}`, to search larger ones). A query that is too heavy to process is stopped partway. Write a run of line breaks as `\n+` or `(?:\r\n)+` (`\r\n+` does not match in a CRLF document).
- **When a replacement puts a lone CR next to a lone LF, they become one line break (CRLF), and the number of lines changes** (for example, deleting `X` in `a` CR `X` LF `b`). This is more likely when replacing across lines in a document with mixed line endings.
- **The folding of CP932 compatible characters (on by default) also applies to replacing.** For example, replacing `—` with `-` also turns `―` into `-`. To replace only one of them, turn the folding off with `cp932_compat_fold = false` under `[search]`.
- **External changes are checked only for the document on screen** (when you come back from another app, or with `file.check_external_changes`). Other documents are checked after you switch to them, and are protected by the check on save ("Overwrite / Save as / Cancel"). If the terminal does not report regaining focus, you are not told until you save. Files larger than 16 MiB are checked by modification time and length, so a rewrite that keeps the same length and modification time may be missed.
- **Opening a file on a UNC path (`\\server\share\…`) may make Windows send your credentials to that server.** This is OS behavior that evoeditor cannot control. Do not open paths on untrusted servers.
- When the terminal has 256 or 16 colors, colors are approximated automatically. Distinctions are kept with symbols and attributes.
- The display during Japanese input (IME) conversion and key combinations may differ depending on the terminal and IME. Some keys such as `Ctrl+Shift+S` do not arrive on terminals that do not send enhanced keys (they are marked with `*` in the key list of the help screen; there are alternatives such as `Ctrl+K S`). Please tell us in Issues if you notice anything.
- **Not in this version**: encodings other than those listed above, backups when saving, measuring in the terminal the width of characters whose width is ambiguous between full-width and half-width (such as `○` and `※`; if the screen is misaligned, set `ambiguous_width` under `[view]` to `1` or `2`), and showing half-width spaces, trailing spaces and line breaks (only full-width spaces and tabs are shown).
- Notes on language files and high-contrast themes (right-to-left languages are not practical, the terminal's cursor can be hard to see with the opposite brightness to your terminal's color scheme, and so on) are in their sections above.

## Known limitations (Linux)

- **The environments checked are Windows Terminal on WSL2 (Ubuntu) and tmux inside it.** The clipboard and the position of the Japanese input (mozc and others) candidate window on desktop environments such as GNOME or KDE (Wayland / X11) have not been checked yet. Please report what you notice in Issues. The Linux console (`TERM=linux`) is not supported. macOS is not supported.
- **Clipboard**: evoeditor uses `wl-copy` / `wl-paste` (Wayland), `xclip` or `xsel` (X11), and `powershell.exe` on WSL. When connected over SSH (the commands would write on the remote side) or when none of them is available, it writes to the terminal clipboard with OSC 52 if the terminal supports it (VTE-based terminals such as GNOME Terminal do not). Otherwise, the clipboard works only inside evoeditor.
- **`Ctrl+Z` is undo.** To go back to the shell temporarily, press `Ctrl+K Z`, and return with `fg` (when started from a pipeline or from `git commit`, the whole job stops).
- **In Windows Terminal on WSL, `Ctrl+Shift+<letter>` such as `Ctrl+Shift+Z` is not told apart from `Ctrl+<letter>`** (Windows Terminal sends the same key; the same applies inside tmux). Use `Ctrl+Y` to redo.
- **When the terminal is closed or on `kill` (SIGTERM / SIGHUP / SIGINT), evoeditor rescues unsaved content before quitting.** It cannot rescue on `kill -9` (SIGKILL) or power loss, and edits made after the last swap write are lost. Files on disk are not damaged, thanks to atomic saving.
- **Inside tmux**, we recommend `set -s escape-time 10`, `set -g set-clipboard on`, `set -s extended-keys on` and `set -g focus-events on` in `~/.tmux.conf` (for `Esc` responsiveness, the clipboard, and telling keys with `Ctrl+Shift` apart). Inside tmux, when the outer terminal is unknown, evoeditor draws with 256 colors (override with `color_mode = "truecolor"` under `[theme]`).
- **On WSL, edit files on the Linux side (under `~`).** Files on Windows drives (under `/mnt/c`) can be opened too, but are slower.

---

The Japanese version ([README.md](README.md)) is authoritative. If the two differ, the Japanese version applies.
