# S-25 REPORT (ワークスペース設定の所有者の検査)

状態: **計測済み** (2026-11-21)。合否の基準の 4 項目のうち、(1) 安全側・(2) 使える・(3) 判定できない FS の一覧は **測った範囲で合格**、(4) は依存と時間が合格で、「UI スレッドの外で判定する」は試作では確かめられない (M33 の実装で確かめる)。未確認: 昇格した端末で作ったフォルダを昇格した端末で判定すること・別の人のユーザーのフォルダ・別の機械の SMB の共有・FAT32 と取り外せる (removable) ドライブ・NTFS の 2 台目のドライブの既定の ACL・root で動かしたとき・NFS / CIFS・macOS (下の「M33 の前に人が確かめる項目」)

合否の基準の正本は 08 §3 の S-25 の行。検査の中身は 07 §12.1 の「所有者の検査」(OQ-35 の (2))。

## 目的

07 §12.1 の所有者の検査 (置き場所のディレクトリの所有者が今のユーザーでない、または他のユーザーが書き込めるとき、ワークスペース設定と `.editorconfig` を読まない) を、各環境で **読む / 読まない / 判定できない** の 3 つに分けられるかを確かめ、M33 の FR-705 の仕様の PR で決める点を洗い出す。

## 環境

| 項目 | 値 |
|---|---|
| 機械 | 開発機 (Windows 11 Pro、x86_64) |
| Windows のアカウント | Entra ID (組織) のアカウント。Administrators のメンバーで、**昇格していない** 端末 (分割トークン。Administrators は deny-only、`TokenElevationType` = 3 (Limited)) |
| ドライブ | `C:` NTFS (システム)。`E:` **exFAT** の外付けのドライブ (`GetDriveTypeW` は `DRIVE_FIXED`)。リポジトリの作業ツリーと `E:\tmp` は `E:` にある。文字の割り当てられた NTFS の 2 台目のドライブ・FAT32 のドライブは無い |
| WSL | WSL2 の Ubuntu (カーネル 6.18)。`/mnt/c`・`/mnt/e` は DrvFs (`9p`、`aname=drvfs;uid=1000;gid=1000`、`metadata` なし)。`/tmp` は tmpfs |
| ツールチェーン | Windows: rustc 1.100.0。WSL: rustc 1.99.1。どちらも release ビルド |
| git (比べる相手) | git for Windows 2.56.0 (`GIT_CONFIG_GLOBAL=NUL`、`GIT_CONFIG_NOSYSTEM=1` で利用者の `safe.directory` を外して走らせた) |
| スパイクのコード | リポジトリの外の使い捨ての cargo のプロジェクト (`E:\tmp\s25-spike\`、出力先 `E:\tmp\s25-target`。WSL では `~/s25-spike`)。作業の後に消した |
| 依存 | Windows は `windows-sys` 0.61 (eve-platform と同じ版。feature は eve-platform が既に有効にしているものの部分集合: `Win32_Foundation`・`Win32_Security`・`Win32_Security_Authorization`・`Win32_Storage_FileSystem`・`Win32_System_Threading`)。Unix は std (`MetadataExt`) と `libc` 0.2 (`geteuid`・`statfs`。eve-platform は Unix で既に `libc` を使っている。`rustix` は eve-platform の直接の依存ではなく推移的な依存なので使わなかった) |

NTFS の試験用のフォルダは、`E:` が exFAT なので `E:\tmp` には作れない。そこで、利用者のプロファイルの下の一時ディレクトリ (`%TEMP%` の下のセッション用のフォルダ) に `s25-ntfs\` を作り、その下の自分で作ったフォルダの ACL だけを `icacls` で変えた (作業の後に消した)。システムの設定と、自分で作っていないフォルダの ACL は変えていない。以下、利用者の名前は `<me>`、組織の名前は `<組織>` と書く。

## 方法

### 判定の試作

1 つのパス (ディレクトリかファイル) を、次の順に判定する。

**Windows** (2 通り。(P) パスで問い合わせる、(H) 開いたハンドルで問い合わせる):

1. FS の印: (P) `GetVolumePathNameW` → `GetVolumeInformationW`、(H) `CreateFileW(READ_CONTROL, FILE_FLAG_BACKUP_SEMANTICS)` → `GetVolumeInformationByHandleW`。`FILE_PERSISTENT_ACLS` が無ければ **判定できない**。
2. 所有者と DACL: (P) `GetNamedSecurityInfoW`、(H) `GetSecurityInfo` (`OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION`)。失敗したら **判定できない**。
3. 所有者が自分のユーザーの SID か、トークンの既定の所有者 (`TokenOwner`。既存の `owned_by_me` と同じ) でなければ **読まない**。
4. DACL を 1 つずつ見て、`INHERIT_ONLY_ACE` でない許可の ACE のうち、書込の権利 (下の `WRITE_MASK`) を **自分たち以外** に与えるものがあれば **読まない**。DACL が NULL (全員にフルアクセス) も **読まない**。拒否の ACE は数えない (書込を打ち消すかを見ないので、読まない側に倒れる)。許可・拒否以外の型の ACE (オブジェクト ACE など) は読めないので、書込ありとみなす。
5. 残りは **読む**。

```rust
/// FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY | FILE_DELETE_CHILD | DELETE | WRITE_DAC | WRITE_OWNER
/// | GENERIC_ALL | GENERIC_WRITE (ファイルでは FILE_WRITE_DATA | FILE_APPEND_DATA | …)
const WRITE_MASK: u32 = 0x2 | 0x4 | 0x40 | 0x1_0000 | 0x4_0000 | 0x8_0000 | 0x1000_0000 | 0x4000_0000;

/// 「自分たち」: 自分・トークンの既定の所有者・SYSTEM (S-1-5-18)・Administrators (S-1-5-32-544)・
/// TrustedInstaller・CREATOR OWNER (S-1-3-0)・OWNER RIGHTS (S-1-3-4)。これ以外の SID への書込の許可は「他人が書ける」。
fn is_trusted_writer(name: &str, c: &Ctx, sid: PSID) -> bool { /* … */ }
```

`ACCESS_ALLOWED_ACE_TYPE` などの定数は `Win32_System_SystemServices` にあるので、feature を足さずに値を写した。SID の比較は `EqualSid` で、決まった SID は起動時に 1 回だけ `ConvertStringSidToSidW` で作る (自分の SID・トークンの情報も 1 回だけ取る)。

**Unix** (2 通り。(P) `stat`、(H) `open` してから `fstat`):

1. FS の種類: `statfs` の `f_type`。所有者と mode が写像・既定値になる FS (`9p` (WSL2 の DrvFs)・`fuse` (virtiofs を含む)・`nfs`・`cifs`・`smb2`・`vfat`・`exfat`) は **判定できない**。
2. 所有者の uid が実効 uid でなければ **読まない**。
3. `o+w` (sticky でも) なら **読まない**。`g+w` は記録だけして判定に使わない (07 §12.1 の文言どおり)。
4. 残りは **読む**。

### 時間

1 つのパスにつき、最初の 1 回と、続けて 200 回 (一部 50 回) の中央値・p99・最大を測った。自分の SID などの 1 回だけ取る値は、最初の 1 回に入る。

## 結果: Windows

(P) はパスで問い合わせる形、(H) はハンドルで問い合わせる形。所有者の「自分」はユーザーの SID。「他人の書込」は書込の権利を持つ自分たち以外の SID と、その権利のマスク。

| ID | 場所 | 作り方 | FS | 所有者 | 他人の書込 | 判定 | (P) 中央値 / p99 (ms) | (H) 中央値 / p99 (ms) |
|---|---|---|---|---|---|---|---|---|
| W1 | `%TEMP%\…\s25-ntfs\own` | 通常の端末で作った | NTFS | 自分 | なし | **読む** | 2.06 / 2.62 | 0.12 / 0.35 |
| W1b | `C:\Users\<me>` (プロファイルのフォルダそのもの) | 既存 | NTFS | **SYSTEM** | なし | 読まない | 0.50 / 0.94 | 0.05 / 0.10 |
| W1c | `%TEMP%` (`C:\Users\<me>\AppData\Local\Temp`) | 既存 | NTFS | 自分 | なし | **読む** | 1.08 / 1.95 | — |
| W2 | `…\everyone-m` | W1 と同じ形に `Everyone:(OI)(CI)M` を足した | NTFS | 自分 | Everyone:0x1301bf | 読まない | 1.90 / 2.94 | — |
| W3 | `…\users-w` | `Users:(OI)(CI)W` を足した | NTFS | 自分 | Users:0x100116 | 読まない | 1.92 / 3.31 | — |
| W3b | `…\authusers-m` | `Authenticated Users:(OI)(CI)M` を足した | NTFS | 自分 | AuthenticatedUsers:0x1301bf | 読まない | 1.59 / 2.35 | — |
| W3c | `…\everyone-rx` | `Everyone:(OI)(CI)RX` を足した (読むだけ) | NTFS | 自分 | なし | **読む** | 1.92 / 2.85 | — |
| W3d | `…\own-filew` と、その中の `.editorconfig` | ファイルだけに `Everyone:M` を足した | NTFS | 自分 | ディレクトリ: なし / **ファイル: Everyone:0x1301bf** | ディレクトリは読む / ファイルは読まない | 2.01 / 3.72 (ファイル 1.53 / 2.73) | — |
| W3e | `…\deny-dc` | `Everyone:(CI)(DENY)(DC)` を足した (OneDrive と同じ形) | NTFS | 自分 | なし (拒否の ACE は数えない) | **読む** | 1.87 / 2.29 | — |
| W4 | `C:\` の直下の、インストーラが管理者の権限で作った 3 つのフォルダ (昇格した端末で作ったフォルダの代わり) | 既存 | NTFS | **Administrators** | AuthenticatedUsers:0x1301bf (ドライブの直下から継承) | 読まない | 0.22〜0.29 / 0.49〜0.62 | — |
| W5 | `C:\Users\Public` | 既存 | NTFS | SYSTEM | INTERACTIVE・SERVICE・BATCH:0x1200af | 読まない | 0.55 / 0.99 | — |
| W5b | `C:\Users\Default` | 既存 | NTFS | SYSTEM | なし | 読まない | 0.75 / 1.37 | — |
| W5c | `C:\Users\` の下の別のアカウント (システムのアカウント) のプロファイル | 既存 | NTFS | (読めない) | (読めない) | **判定できない** (`GetNamedSecurityInfoW` が 5 = アクセス拒否) | 1.01 / 1.40 | — |
| W5d | `C:\Windows` | 既存 | NTFS | TrustedInstaller | なし | 読まない | 0.30 / 0.71 | — |
| W5e | `C:\ProgramData` | 既存 | NTFS | SYSTEM | Users:0x116 | 読まない | 0.30 / 0.60 | — |
| W6 | `C:\` (ドライブの直下) | 既存 | NTFS | TrustedInstaller | AuthenticatedUsers:0x4 (`AD`) | 読まない | 0.14 / 0.45 | 0.07 / 0.32 |
| W6b | `E:\` (ドライブの直下) | 既存 | exFAT | (Everyone と返る) | (NULL DACL) | **判定できない** (`FILE_PERSISTENT_ACLS` なし) | 1.95 / 410 | — |
| W7 | `C:\` の直下の、通常の端末で作った 2 つのフォルダ | 既存 | NTFS | 自分 | **AuthenticatedUsers:0x1301bf** (ドライブの直下の `(OI)(CI)(IO)(M)` を継承) | 読まない | 0.42〜0.43 / 0.84〜1.16 | — |
| W8 | OneDrive の同期フォルダ (`C:\Users\<me>\OneDrive - <組織>`) と、その中のフォルダ | 既存 (読むだけ。中に作っていない) | NTFS | 自分 | なし (`Everyone:(CI)(DENY)(DC)` の拒否の ACE だけ) | **読む** | 0.76〜1.14 / 1.03〜1.96 | — |
| W9 | `E:\tmp\s25-exfat\own` | 通常の端末で作った | exFAT | (Everyone と返る) | (NULL DACL) | **判定できない** | 8.7 / 2,157 | 0.43 / 317 |
| W9b | リポジトリの作業ツリー (`E:` の上) | 既存 | exFAT | (Everyone) | (NULL DACL) | **判定できない** | 9.2 / 1,641 | — |
| W10 | `\\localhost\C$\…\s25-ntfs\own` (SMB。ループバックの管理共有) | W1 と同じもの | NTFS (`DRIVE_REMOTE`) | 自分 | なし | **読む** | 20.2 / 31.9 | 1.28 / 2.90 |
| W10b | `\\localhost\C$\…\s25-ntfs\everyone-m` | W2 と同じもの | NTFS (remote) | 自分 | Everyone:0x1301bf | 読まない | 19.0 / 28.7 | — |
| W10c | `\\localhost\C$\Users\Public` | 既存 | NTFS (remote) | SYSTEM | INTERACTIVE ほか | 読まない | 11.2 / 19.6 | — |
| W10d | `\\localhost\E$\tmp\s25-exfat\own` (SMB の向こうが exFAT) | W9 と同じもの | exFAT (remote) | (Everyone) | (NULL DACL) | **判定できない** (SMB 越しでも `FILE_PERSISTENT_ACLS` が無いと分かる) | 1,139 / 2,424 | 3.6 / 270 |
| W10e | `\\wsl.localhost\Ubuntu\tmp`・`…\home\<me>\…` (WSL の 9P の共有) | 既存・WSL で作った | 9P (remote) | (返さない) | (返さない) | **判定できない** (`GetNamedSecurityInfoW` / `GetSecurityInfo` が 1 = `ERROR_INVALID_FUNCTION`) | 11.0 / 15.6 | 1.77 / 2.71 |

所見:

- **(P) の時間は `GetVolumePathNameW` が大半**。パスの成分ごとに問い合わせるので、深いパスほど遅い (W1 の 2.06 ms のうちボリュームの問い合わせが 1.14 ms、`GetNamedSecurityInfoW` だけは 0.17 ms)。(H) は開いたハンドルで問うので、NTFS で 0.05〜0.12 ms (p99 0.35 ms 以下) になる。
- **exFAT の外付けのドライブの p99・最大の 0.3〜2.5 秒は、判定の費用ではなくドライブの止まり**。同じパスに `GetFileAttributes` だけを 300 回かけても、中央値 0.65 ms・p99 295 ms・最大 314 ms だった (NTFS の `C:\Users\Public` では中央値 0.07 ms・p99 0.3 ms)。判定を UI スレッドで行えない理由の実例になる。
- **SMB は、ループバック (同じ機械・同じアカウント) では NTFS と同じ判定になった**。所有者は自分の SID のまま見え、DACL も同じ。別の機械・別のアカウントの共有は試せていない (未確認)。
- git (2.56.0) は exFAT のリポジトリ (W9) を `detected dubious ownership … is on a file system that does not record ownership` で拒み、NTFS の `everyone-m` (W2) は受け付けた (git は Windows では所有者だけを見て、DACL を見ない)。

### 書込の権利のマスクの実例

| マスク | 中身 | 出た場所 |
|---|---|---|
| 0x1301bf | Modify (`M`) | `icacls … M`、ドライブの直下からの継承 (W4・W7) |
| 0x100116 | Write (`W`) | W3 |
| 0x1200af | 読み取りと実行 + `FILE_ADD_FILE` / `FILE_ADD_SUBDIRECTORY` 相当 | `C:\Users\Public` |
| 0x116 / 0x4 | ファイルの作成・フォルダの作成 (`WD,AD,WEA,WA` / `AD`) | `C:\ProgramData` / `C:\` |

## 結果: Unix (WSL2 の Ubuntu。実効 uid 1000)

| ID | 場所 | FS | 所有者 / mode | 判定 | (P) 中央値 / p99 (µs) | (H) 中央値 / p99 (µs) |
|---|---|---|---|---|---|---|
| U1 | `~/s25-u/own` | ext4 | 自分 / 0755 | **読む** | 4.9 / 11.7 | 10.5 / 13.2 |
| U1b | `~` (ホーム) | ext4 | 自分 / 0750 | **読む** | 3.7 / 5.0 | 9.7 / 33.1 |
| U2 | `~/s25-u/gw` | ext4 | 自分 / 0775 (`g+w`) | **読む** (`g+w` は判定に使わない) | 4.2 / 4.9 | 10.5 / 12.2 |
| U3 | `~/s25-u/ow` | ext4 | 自分 / 0777 | 読まない (`o+w`) | 4.2 / 4.9 | 10.6 / 44.8 |
| U3b | `~/s25-u/ows` | ext4 | 自分 / 1777 | 読まない (sticky でも `o+w`) | 4.2 / 11.0 | 10.6 / 28.9 |
| U4 | `/tmp` | tmpfs | root / 1777 | 読まない | 2.9 / 3.0 | 7.9 / 9.6 |
| U4b | `/tmp/s25-mine` | tmpfs | 自分 / 0700 | **読む** | 3.1 / 3.2 | — |
| U4c | `/dev/shm` | tmpfs | root / 1777 | 読まない | 3.1 / 3.9 | 8.3 / 17.6 |
| U4d | `/dev/shm/s25-mine` | tmpfs | 自分 / 0755 | **読む** | 3.3 / 3.5 | — |
| U4e | `/run/user/1000` | tmpfs | 自分 / 0700 | **読む** | 3.3 / 10.1 | 8.9 / 31.1 |
| U5 | `/root` | ext4 | root / 0700 | 読まない ((H) は `open` が EACCES で判定できない) | 3.5 / 3.7 | 3.1 / 4.3 |
| U5b | `/etc`・`/` | ext4 | root / 0755 | 読まない | 3.2〜3.6 / 3.6〜3.9 | 9.1〜9.6 / 24.5〜28.7 |
| U5c | `/var/lib/chrony` (システムのユーザーの所有) | ext4 | uid 988 / 0750 | 読まない ((H) は EACCES) | 4.0 / 5.5 | 3.7 / 11.5 |
| U6 | `/mnt/c` | 9p (DrvFs) | **自分 / 0777** (写像) | **判定できない** | 928 / 2,111 | 2,049 / 3,087 |
| U6b | `/mnt/c/Users/<me>` (Windows では SYSTEM の所有) | 9p (DrvFs) | **自分 / 0777** (写像) | **判定できない** | 4,272 / 7,118 | 6,019 / 8,511 |
| U6c | `/mnt/c/Windows` (Windows では TrustedInstaller の所有) | 9p (DrvFs) | **自分 / 0555** (写像) | **判定できない** | 3,251 / 4,226 | 3,973 / 4,925 |
| U6d | W1 を `/mnt/c/…` から | 9p (DrvFs) | 自分 / 0777 | **判定できない** | 24,164 / 32,251 | 22,243 / 31,086 |
| U6e | W2 (`Everyone:M`) を `/mnt/c/…` から | 9p (DrvFs) | 自分 / 0777 (W1 と区別できない) | **判定できない** | 19,632 / 29,356 | 22,460 / 37,841 |
| U6f | W9 (exFAT) を `/mnt/e/…` から | 9p (DrvFs) | 自分 / 0777 | **判定できない** | 22,789 / 41,723 | 6,196 / 18,113 |

所見:

- **DrvFs は所有者も mode も写像で、判定の根拠にならない**: Windows で SYSTEM・TrustedInstaller・自分のどれが所有していても uid 1000 に見え、mode は 0777 (読み取り専用の属性があると 0555) になる。`Everyone:M` の W2 と自分だけの W1 が区別できない。FS の種類を先に見ないと、DrvFs は全部「`o+w` で読まない」になり、理由の文言が誤る (試作の 1 回目はこの順で、全部「`o+w`」と出た)。`statfs` の `f_type` は `0x01021997` (`V9FS_MAGIC`)。
- DrvFs の 1 回の判定は 0.9〜24 ms (パスが深いほど遅い。9P の往復)。ネットワークの FS と同じ扱い (10 ms の目安の外) にするのが妥当。
- git (WSL の側) は `/mnt/e` (exFAT を DrvFs で見たもの) のリポジトリを受け付けた (所有者が写像で自分に見えるため)。Windows の git (W9) と結論が食い違う。
- root で動かしたとき: `sudo` が対話の認証を要したので **試していない** (未確認)。

## 判定 (S-25 の合否の基準ごと)

| 基準 | 判定 | 根拠 |
|---|---|---|
| (1) 安全側: 他のユーザーの所有・他のユーザーが書き込めるディレクトリを「読む」と判定する環境が 0 | **合格 (測った範囲)** | 他人の所有 (W1b・W4・W5・W5b・W5d・W5e・W6・U4・U5・U5b・U5c) はすべて「読まない」か「判定できない」。他人が書けるもの (W2・W3・W3b・W7・W10b・U3・U3b) はすべて「読まない」。所有者や ACL を返さない・写像する FS (W6b・W9・W10d・W10e・U6〜U6f) はすべて「判定できない」。**ただし** (a) ディレクトリだけを見るとファイルに `Everyone:M` を付けた W3d を見逃す (ファイルも見る必要がある)、(b) パスで判定してから読むと、その間に差し替えられうる (下の M33 の (5))、(c) 別の機械の SMB・NFS / CIFS・root は未確認 |
| (2) 使える: 自分で作ったディレクトリは「読む」 | **合格** | Windows の通常の端末でプロファイルの下に作った W1・W1c・W3c・W3e・OneDrive (W8)・ループバックの SMB (W10)、Unix のホームの下 (U1・U1b・U2) と自分の tmpfs のディレクトリ (U4b・U4d・U4e) は「読む」。**自分のものなのに「読まない」/「判定できない」になる環境** は下の一覧 |
| (3) 判定できない FS が一覧になり、M33 で扱いを決められる | **合格** | 下の「判定できない」の行。判定の方法 (Windows は `FILE_PERSISTENT_ACLS` と問い合わせの失敗、Unix は `statfs` の `f_type`) も確かめた |
| (4) 依存と時間: 新しい依存なし・UI スレッドの外・1 つのディレクトリが 10 ms 以内 (共有を除く) | **依存と時間は合格。UI スレッドの外は未確認 (M33)** | 依存: `windows-sys` は既存の feature の部分集合、Unix は std と既存の `libc`。時間: ローカルの FS では (P) で中央値 2.1 ms 以下・p99 3.7 ms 以下、(H) で中央値 0.05〜0.43 ms。Unix のローカルは 3〜11 µs。外付けの exFAT の止まり (0.3〜2.5 秒) はドライブの止まりで、`GetFileAttributes` でも起きる。共有: ループバックの SMB は (P) 11〜20 ms・(H) 1.3 ms、WSL の 9P の共有は (P) 11 ms・(H) 1.8 ms、DrvFs は 0.9〜24 ms。UI スレッドの外 (読込のジョブ) で行うことは、試作ではなく M33 の実装で確かめる |

退路 (S-25 の行の最後の列) は使わずに済む: Windows でも判定は成り立つ。

## 「読む / 読まない / 判定できない」の分け方の案

### 案 (M33 の仕様の PR の材料)

| 分け方 | Windows | Unix |
|---|---|---|
| **判定できない** (読まずに診断) | ボリュームに `FILE_PERSISTENT_ACLS` が無い (FAT32・exFAT。SMB の向こうがそうでも分かる)。所有者・DACL の問い合わせが失敗する (9P の `\\wsl.localhost\…`、アクセス拒否)。 | `statfs` の `f_type` が、所有者と mode を写像・既定値にする FS (WSL の DrvFs (`9p`)・`fuse`・`cifs` / `smb2`・`vfat` / `exfat`。`nfs` は M33 で決める)。`stat` / `open` の失敗。 |
| **読まない** (所有者) | 所有者の SID が自分 (か `TokenOwner`) でない。 | 所有者の uid が実効 uid でない。 |
| **読まない** (他人が書ける) | 継承専用でない許可の ACE が、自分たち (自分・`TokenOwner`・SYSTEM・Administrators・TrustedInstaller・CREATOR OWNER・OWNER RIGHTS) 以外の SID に書込の権利 (`WRITE_MASK`) を与える。NULL DACL。読めない型の ACE。 | `o+w` (sticky でも)。 |
| **読む** | 上のどれでもない。 | 上のどれでもない。 |

判定する物: 設定ファイルを置いたディレクトリ (ワークスペースのルート・`.editorconfig` のあるディレクトリ・`.eve`) と、**設定ファイルそのもの** (W3d)。祖先のディレクトリは見ない (`C:\Users\<me>` は SYSTEM、`/home` は root の所有なので、祖先まで見ると何も読めない)。祖先のディレクトリを他人が書けても、置き場所のディレクトリを差し替えると所有者が変わるので、読む時点の判定で見つかる (下の M33 の (5) のとおり、判定と読込を同じハンドルで行うのが前提)。

### 誤って「読まない」側に倒れる場合 (使える側の損。安全は損なわない)

| 場合 | 例 (実測) | 理由 | 診断の文言の案 (理由と対処) |
|---|---|---|---|
| プロファイルのフォルダそのもの | W1b `C:\Users\<me>` | Windows の既定で所有者が SYSTEM。`~/.editorconfig` (利用者の既定として置く人が多い) と、ホームをワークスペースにしたときが読まれない | 「このフォルダの所有者は SYSTEM です (Windows のプロファイルのフォルダの既定)」。下の M33 の (3) (特権の所有者) で救える |
| ドライブの直下に作ったフォルダ (`C:\src`・`C:\work` など) | W7 | ドライブの直下の `Authenticated Users:(OI)(CI)(IO)(M)` を継承し、**同じ機械の他のユーザーが実際に書ける**。誤りではなく本当の危険だが、開発者が多く使う置き場所 | 「Authenticated Users が書き込めるため無視しました。プロファイルの下へ移すか、継承を切って (`icacls <dir> /inheritance:d` の後に Authenticated Users を外す) ください」 |
| NTFS の 2 台目のドライブ (`D:\dev` など) | (未確認。文字の割り当てられた NTFS の 2 台目のドライブが無い) | ドライブの直下の既定の ACL が `C:\` と同じなら W7 と同じになる見込み | 同上 |
| 昇格した端末で作ったフォルダを、昇格していない端末で開く | W4 (インストーラが作ったフォルダで代用) | 所有者が Administrators。既存の `owned_by_me` は `TokenOwner` (昇格していないと自分) と比べるので一致しない | 「所有者が Administrators です (管理者として作ったフォルダ)。`takeown` で自分の所有にしてください」 |
| exFAT / FAT32 のドライブ | W9・W9b (このリポジトリの作業ツリーそのもの) | 所有者を記録しない FS | 「所有者を記録しない FS (exFAT) のため、ワークスペース設定を読みません」。git も同じ理由で拒む |
| WSL から `/mnt/c` | U6〜U6f | DrvFs の写像 | 「Windows のドライブ (DrvFs) は所有者を判定できないため読みません。WSL のホームの下で開くか、Windows 版で開いてください」 |
| WSL の中を Windows から `\\wsl.localhost\…` | W10e | 9P の共有が ACL を返さない | 「この共有は所有者を返さないため読みません」 |
| 自分のディレクトリが `o+w` | U3・U3b | 本当に他人が書ける | 「他のユーザーが書き込めるため (`o+w`) 読みません。`chmod o-w` で外してください」 |

### 誤って「読む」側に倒れる場合 (安全側の損。仕様で手当てが要る)

| 場合 | 起きる条件 | 手当ての案 |
|---|---|---|
| ディレクトリは自分だけだが、設定ファイルに他人の書込がある | W3d (実測)。Unix でもファイルだけ `o+w` はありうる | 設定ファイルそのものも同じ判定に掛ける |
| 判定と読込の間の差し替え (TOCTOU) | パスで判定 (P) してから、同じパスを開き直して読む | 判定と読込を同じハンドルで行う。Unix はディレクトリを `open` → `fstat` → `openat(…, O_NOFOLLOW)` → `fstat`。Windows はファイルを開いて `GetSecurityInfo` で判定し、同じハンドルから読む (親のディレクトリは `FILE_SHARE_DELETE` なしで開いて名前の変更を止める案。**未検証**) |
| 「自分たち」の SID に入れたグループに他人がいる | Administrators は同じ機械の他の管理者を含む。昇格した端末では `TokenOwner` が Administrators になり、既存の `owned_by_me` の規則で「自分」になる | 管理者は他人のファイルの所有者を奪えるので、守る対象の外とみなす (反対の論点に書いた) |
| 別の機械の SMB で、サーバの SID と手元の SID の対応が違う | 未確認 | M33 の前に人が確かめる |
| root で動かしたとき、既存の `owner_is_me` (root は全部「自分」、05 §4.4) を流用する | `crates/eve-platform/src/unix_fs.rs` の `owner_is_me: Some(meta.uid() == my_euid() \|\| my_euid() == 0)` | 所有者の検査では流用しない。root のときは uid 0 の所有だけを「自分」とする (git は `sudo` のとき `SUDO_UID` と比べる。採るかは M33 で決める) |

読まない側に倒れるだけで手当ての要らないもの: 書込を許すグループが実は自分しか含まない (ドメインのグループなど)、拒否の ACE で書込を打ち消している (数えないので「書ける」とみなす)。

## M33 の FR-705 の仕様の PR で決めるべき点

1. **Windows の「他人が書ける」の範囲**: 07 §12.1 は「`Everyone` / `Users` に書込許可」と書くが、実測では `Authenticated Users` (W7・W4・W3b)・`INTERACTIVE` / `SERVICE` / `BATCH` (W5) も書込を持つ。**推奨**: 「自分たち (自分・トークンの既定の所有者・SYSTEM・Administrators・TrustedInstaller・CREATOR OWNER・OWNER RIGHTS) 以外の SID への書込の許可」とし、書込の権利 (上の `WRITE_MASK`) と、継承専用の ACE を除くこと、拒否の ACE を数えないことを書く。
2. **ドライブの直下に作ったフォルダの扱い** (W7): 本当に他人が書けるので「読まない」が正しいが、使う人は多い。**推奨**: 読まないままにし、診断で理由と対処 (継承を切る・プロファイルの下へ移す) を出す。
3. **特権の所有者 (SYSTEM・TrustedInstaller・Administrators、Unix の root) を受け入れるか**: 受け入れれば、プロファイルのフォルダ (W1b)・昇格した端末で作ったフォルダ (W4 の形)・`/etc` が「他人の書込が無ければ読む」になる。特権のアカウントは今のユーザーのファイルも書き換えられるので、守れる範囲は変わらない。**推奨**: 受け入れる (OQ-35 の (2) の「所有者が自分でない」の例外。下の OQ-35 の追記の案)。ただし git の `safe.directory` と挙動が変わる (反対の論点)。
4. **判定できない FS の扱い**: v0.5-plan §2.6 の推奨どおり **読まずに診断**。ただし **この開発機のリポジトリは exFAT にある** (W9b) ので、v0.5 では開発者自身のワークスペース設定も `.editorconfig` も読まれない。`workspace.trust` を後の版に送るなら、v0.5 の間の逃げ道を決める: (a) 逃げ道なし (既知の制約、01 §4.6)、(b) **ユーザー層だけに書ける** パスの一覧 (git の `safe.directory` と同じ形。ワークスペース層には書けない) を v0.5 で足す、(c) `workspace.trust` を v0.5 に入れる。**推奨**: (b) (信頼の UI と記録が要らず、ユーザー層は他人に書かれない)。
5. **判定と読込を同じハンドルで行う**: パスで判定してから開き直すと差し替えられうる。ハンドルで問う形 (H) は時間も短い (NTFS で 0.05〜0.12 ms。(P) の約 1/10)。**推奨**: ハンドルで判定し、同じハンドルから読む。Windows の親のディレクトリの固定のしかたは M33 で試す。
6. **判定する物**: 置き場所のディレクトリ (`<root>`・`.eve`・`.editorconfig` のあるディレクトリ) と設定ファイルそのもの。祖先は見ない。
7. **Unix の `g+w`**: 07 §12.1 は `o+w` だけ。共有のグループのディレクトリ (`/srv/proj` を `g+w` で共有する形) では、グループの他の人が置ける。Ubuntu の既定 (ユーザーごとのグループ) では `g+w` は自分だけ。**推奨**: 今の文言 (`o+w` だけ) のまま。理由を仕様に書く。
8. **root のとき**: 既存の `owner_is_me` (root は全部「自分」) を流用しない。uid 0 の所有だけを「自分」とするか、`SUDO_UID` も見るか。
9. **`nfs` を判定できる FS にするか**: NFS の uid は (idmap があれば) 意味を持つが、`root_squash` や `all_squash` で写像される。**推奨**: v0.5 は判定できない側 (安全側)。
10. **時間とキャッシュ**: ボリューム (Windows) / `st_dev` (Unix) ごとに FS の種類をキャッシュする。外付けのドライブの止まり (0.3〜2.5 秒) があるので、判定は必ずジョブで行い、待つ間は設定の層なしで表示する (02 §4.4 規則 2)。
11. **Windows の `owner_is_me`**: `crates/eve-platform/src/fs.rs` の Windows の `FileMeta` は `owner_is_me: None` のまま。所有者の検査は `FileMeta.owner_is_me` (保存の所有者の保持、05 §4.4。root は「自分」) とは意味が違うので、別の関数・別の値にする。

## OQ-35 (docs/10) への追記の案 (状態は変えない)

> **S-25 の実測 (2026-11-21)**: 判定の方式 (所有者の SID / uid と、他人の書込) は Windows・Linux とも成り立つ (`spikes/S-25/REPORT.md`)。決定の (2) の細目として M33 で決める: (a) Windows の「world-writable」は `Everyone` / `Users` に限らず、自分・SYSTEM・Administrators・TrustedInstaller 以外の SID への書込の許可とする (`Authenticated Users` が既定でドライブの直下から継承されるため)、(b) 所有者が SYSTEM・TrustedInstaller・Administrators (Unix は root) で他人の書込が無いディレクトリを読むか (プロファイルのフォルダ `C:\Users\<名前>` は SYSTEM の所有)、(c) 所有者を記録しない FS (FAT32・exFAT・WSL の DrvFs・9P の共有) は読まずに診断し、v0.5 の逃げ道をユーザー層のパスの一覧にするか、(d) 設定ファイルそのものも判定し、判定と読込を同じハンドルで行う。

## M33 の前に人が確かめる項目 (未確認)

| 項目 | 試せなかった理由 | 見込み |
|---|---|---|
| 昇格した端末で作ったフォルダを、昇格した端末と昇格していない端末で判定する | 昇格の確認 (UAC) が対話を要する | 昇格した端末では `TokenOwner` が Administrators なので「読む」、昇格していない端末では W4 と同じ「読まない (所有者 = Administrators)」 |
| 別の人のユーザーのフォルダ (2 つ目の人のアカウント) | 人のアカウントが 1 つ | プロファイルの ACL は本人・SYSTEM・Administrators だけなので、W5c と同じ「判定できない (アクセス拒否)」か、読めれば「読まない」 |
| 別の機械の SMB の共有 (別のアカウント・ドメイン) | 共有が無い | 所有者の SID がサーバの側の SID になり「読まない」になる見込み。時間も記録する |
| FAT32 と、取り外せる (`DRIVE_REMOVABLE`) USB のドライブ | FAT32 の領域にドライブの文字が無い。外付けのドライブは `DRIVE_FIXED` を返した | exFAT と同じく `FILE_PERSISTENT_ACLS` が無く「判定できない」 |
| NTFS の 2 台目のドライブ (`D:\…`) の既定の ACL | 文字の割り当てられた NTFS の 2 台目のドライブが無い | `C:\` と同じなら、直下に作ったフォルダは W7 と同じ「読まない」 |
| ReFS / Dev Drive | 無い | ACL を持つので NTFS と同じ見込み |
| root で動かしたとき (Linux) | `sudo` が対話の認証を要する | 判定の規則 (M33 の (8)) を決めてから試す |
| NFS / CIFS (Linux) | 無い | `f_type` で見分けられる |
| WSL の DrvFs を virtiofs で使う構成 | 既定の 9P の構成だけ | `f_type` が `fuse` になる見込み (判定できない側) |
| macOS | 出荷する版で (08 §3 の S-25 の行) | — |

## 反対の論点

- **特権の所有者を受け入れる案 (M33 の (3)) は、git の `safe.directory` と挙動が変わる**。git は Windows では所有者だけを、Unix では uid だけを見て、SYSTEM の所有のディレクトリも拒む。「git で動くのに evoeditor で読まれない」「その逆」が出ると説明しにくい。一方で、本案はすでに DACL と `o+w` を見ていて git より厳しいので、git と完全には揃わない。
- **Administrators を「自分たち」に入れると、共有の PC の他の管理者が置いた設定を読む**。管理者は所有者を奪える (`takeown`) ので、守れる範囲は変わらないと見たが、「他のユーザーが置いた設定を読まない」という OQ-35 の言葉どおりではない。
- **ドライブの直下のフォルダ (W7) を読まないのは、使い勝手の損が大きい**。Windows の開発者は `C:\src` や `D:\dev` をよく使い、git もそこを受け付ける。逆に、そこは本当に同じ機械の他のユーザーが書けるので、読むのは OQ-35 の決定に反する。診断の文言で対処を示すことで埋める案にした。
- **exFAT の作業ツリーで何も読まれないのは、この開発機そのものに当たる**。v0.5 で逃げ道を足さないと、FR-705 の受入をこの開発機で確かめられない (exFAT の上では `.editorconfig` が効かない)。逃げ道 (ユーザー層のパスの一覧) は「信頼の仕組みは後の版」という v0.5-plan §2.5 の範囲を広げる。
- **ファイルそのものも判定すると、判定の回数が増える**。`.editorconfig` の祖先探索では、見つかった `.editorconfig` ごとにディレクトリとファイルの 2 回になる。(H) なら 1 回 0.1 ms 程度なので、ローカルでは問題にならない見込み (ネットワークの共有では 1〜20 ms × 回数)。
- 時間は 1 台の開発機で測った値で、基準機ではない。exFAT の外付けのドライブの止まりは、同じ時間帯に別の作業がそのドライブを使っていた影響を含みうる。
