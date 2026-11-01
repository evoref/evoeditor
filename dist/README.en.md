# evoeditor

[日本語](README.md) | English

A text editor written in Rust. It aims to let you edit and save Japanese text safely in Windows terminals (Windows Terminal / the console host) and Linux terminals, keeping the original encoding and line endings.

Supported operating systems are **Windows 11 (x86_64)** and **Linux (x86_64 / aarch64, from v0.2)**.

- License: Apache License 2.0 ([LICENSE](LICENSE), [NOTICE](NOTICE))
- Notices for other software used: [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)
- Bug reports and feedback: GitHub Issues. Do not report vulnerabilities in public issues; use the contact in the repository's `SECURITY.md`.

## What it does

- Opens UTF-8 (with or without BOM), UTF-16, Shift_JIS, EUC-JP and ISO-2022-JP files and saves them **in the original encoding and line endings**. If some characters cannot be represented, it tells you before saving.
- Saving is atomic (if it stops halfway, the original file is not damaged). Before saving, it checks whether another program changed the file.
- Search and replace all. For huge files, it keeps accepting input while loading and searching.
- Undo and redo. On abnormal termination, it rescues unsaved content.

Main keys (the command palette is `F1` or `Ctrl+P`; it shows the key assigned to each command):

| Key | Action |
|---|---|
| `Ctrl+S` | Save (asks for a name for an untitled document) |
| `Ctrl+O` | Open a file |
| `Ctrl+F` | Search (`Ctrl+R` to replace) |
| `Ctrl+Z` | Undo |
| `Ctrl+G` | Go to line |
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
evoeditor memo.txt        # open a file (only one)
evoeditor --readonly a.txt
evoeditor --encoding shift_jis old.txt   # open with a given encoding
evoeditor --help
```

### Configuration

A configuration file is not required. To change settings, copy the bundled [config.example.toml](config.example.toml) to `%APPDATA%\evoeditor\config.toml` on Windows, or to `~/.config/evoeditor/config.toml` on Linux (under `XDG_CONFIG_HOME` if you set it), and edit it (create the folder if it does not exist). Mistakes do not stop startup; only the affected item falls back to its default, and you are told about it.

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

## Known limitations (v0.1)

- **The target is Windows Terminal and the console host on Windows 11 (x86_64).** Windows 10, macOS and Linux are not supported. The user interface is Japanese only.
- **The executable is not signed.** SmartScreen may show a warning (usually not, if you verified the files and unblocked them in step 3). Even if a warning appears, after confirming the verification you can choose "More info" → "Run anyway". On Windows 11 with **Smart App Control turned on**, unsigned executables may be refused; this version does not provide a workaround. With "Controlled folder access" enabled, you cannot save into protected folders (such as Documents). If antivirus software flags it by mistake, verify the signature and hash again before restoring it from quarantine, and let us know in Issues.
- **Only one document can be open at a time.** Opening another file replaces the current document (with a confirmation if it is unsaved). At most one path can be given on the command line.
- **Unsaved content cannot be rescued on forced termination or power loss.** It is rescued on normal exit and on panic. It may also not be rescued on Windows logoff or shutdown; save before logging off. Files on disk are not damaged, thanks to atomic saving.
- **Files with invalid bytes are opened read-only.** Reopening with a given encoding may recover them. Read-only files and documents opened with `--readonly` cannot be edited (editing is refused with the reason). You can edit after saving under another name.
- **Files with a line longer than 1 MiB are opened read-only.** Such a line is shown up to its first 64 KiB.
- **When saving as ISO-2022-JP, characters that cannot be represented** (half-width katakana, compatibility characters not in JIS, and so on) are listed before saving. Replace them, or change the encoding. When a document containing supplementary kanji and the like is saved in its original encoding, some byte representations may change; in that case, too, you are asked before saving.
- **Search is plain text only** (with a case sensitivity toggle). There are no regular expressions. Replacement is "replace all" only (undone with one undo).
- **External changes are not reported until you save.** On save, you are asked "Overwrite / Save as / Cancel". Files larger than 16 MiB are checked by modification time and length, so a rewrite that keeps the same length and modification time may be missed.
- Whitespace visualization covers only full-width spaces and tabs. There is no help screen (see the key list in the command palette). The clipboard is the Windows clipboard; OSC 52 is not supported. There are 7 configuration keys and 5 command-line options.
- **Opening a file on a UNC path (`\\server\share\…`) may make Windows send your credentials to that server.** This is OS behavior that evoeditor cannot control. Do not open paths on untrusted servers.
- When the terminal has 256 or 16 colors, colors are approximated automatically. Distinctions are kept with symbols and attributes.
- The display during Japanese input (IME) conversion and key combinations may differ depending on the terminal and IME. Please tell us in Issues if you notice anything.

---

The Japanese version ([README.md](README.md)) is authoritative. If the two differ, the Japanese version applies.

## Known limitations (Linux)

- **The environments checked are Windows Terminal on WSL2 (Ubuntu) and tmux inside it.** The clipboard and the position of the Japanese input (mozc and others) candidate window on desktop environments such as GNOME or KDE (Wayland / X11) have not been checked yet. Please report what you notice in Issues. The Linux console (`TERM=linux`) is not supported. macOS is not supported.
- **Clipboard**: evoeditor uses `wl-copy` / `wl-paste` (Wayland), `xclip` or `xsel` (X11), and `powershell.exe` on WSL. When connected over SSH (the commands would write on the remote side) or when none of them is available, it writes to the terminal clipboard with OSC 52 if the terminal supports it (VTE-based terminals such as GNOME Terminal do not). Otherwise, the clipboard works only inside evoeditor.
- **`Ctrl+Z` is undo.** To go back to the shell temporarily, press `Ctrl+K Z`, and return with `fg`.
- **When the terminal is closed or on `kill` (SIGTERM / SIGHUP / SIGINT), evoeditor rescues unsaved content before quitting.** It cannot rescue on `kill -9` (SIGKILL) or power loss. Files on disk are not damaged, thanks to atomic saving.
- **Inside tmux**, we recommend `set -s escape-time 10`, `set -g set-clipboard on`, `set -s extended-keys on` and `set -g focus-events on` in `~/.tmux.conf` (for `Esc` responsiveness, the clipboard, and telling keys with `Ctrl+Shift` apart). Inside tmux, when the outer terminal is unknown, evoeditor draws with 256 colors (override with `color_mode = "truecolor"` under `[theme]`).
- **On WSL, edit files on the Linux side (under `~`).** Files on Windows drives (under `/mnt/c`) can be opened too, but are slower.
