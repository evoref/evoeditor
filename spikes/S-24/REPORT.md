# S-24 REPORT (ファイル監視 notify)

状態: **計測済み** (2026-11-21)。合否の基準 (1)〜(5) は **すべて満たした** (下の「判定」。(3) の Linux は `statfs` による判定の実装が前提)。ただし、**notify の最新の安定版 8.2.2 は Windows で監視のスレッドが 100 ms ごとに起きる** (CPU 使用率は 0.1% を大きく下回るので基準 (4) は満たすが、FR-117 AC-10 の「一定の間隔で起きるスレッドを持たない」に反する)。9.0.0-rc.9 では起きない。どちらの版で入れるかは M33 の仕様の PR で決める (下の「M33 で決めること」の 1)。未確認: macOS (FSEvents)・実機の SMB の共有 (別の機械のサーバ)・OneDrive の同期フォルダ・Linux の CIFS / NFS のマウント (下の「人が確かめること」)

合否の基準の正本は 08 §3 の S-24 の行。関連: 05 §12 (監視の方式)、01 FR-117 AC-7〜10、NFR-204、NFR-207、02 §13.7、`docs/review/v0.5-plan.md` §1.2 / §2.4 / §2.6。

## 目的

FR-117 の監視 (v0.5、Should) を notify (debouncer の crate を使わない) で作れるかを、M33 の仕様の PR の前に確かめる。見るのは、保存の種類ごとに届くイベントの列と、100 ms の待ち合わせの後に知らせが 1 回にまとまるか、自分の保存を抑えられるか、監視しない FS を監視の前に見分けられるか、Windows のディレクトリのロック、アイドル時の起床、依存。

## 環境

| 項目 | 値 |
|---|---|
| 機械 | 開発機 (Windows 11、x86_64、論理 24 コア)。基準機ではない |
| Windows の FS | NTFS (システムドライブの `%TEMP%` の下)。SMB は同じ機械の管理共有 (`\\localhost\C$\…`、ループバック) |
| WSL2 | Ubuntu、カーネル 6.18 (`microsoft-standard-WSL2`)。ext4 (`~`)、tmpfs (`/dev/shm`)、DrvFs (`/mnt/c`、9p)。`fs.inotify.max_user_watches` 524288、`max_user_instances` 128 |
| ツールチェーン | Windows: rustc 1.100.0。WSL: rustc 1.99.1。どちらも release ビルド (既定の `[profile.release]`) |
| notify | **8.2.2** (2026-11-08、crates.io の最新の安定版) と、比べる相手の **9.0.0-rc.9** (2026-11-15)。どちらも `default-features = false` (macOS の FSEvents を切る。Windows / Linux の backend は変わらない) |
| リポジトリ | `spike/S-24-notify` (`develop` の先頭 `d18cb55`)。`eve-core`・`eve-platform` を path 依存で使い、evoeditor の原子的保存と直接上書きは **製品のコード** (`WinFs` / `UnixFs` の `write_atomic` / `write_in_place`) で行った |
| スパイクのコード | リポジトリの外の使い捨ての cargo のプロジェクト (Windows は `E:\tmp\s24-spike\`、WSL は `~/s24-spike`)。リポジトリには入れない (要点は末尾) |
| git | Windows 2.56.0、WSL 2.53.0 |

## 方法

監視の対象は 05 §12.2 の 1 のとおり、文書 `a.txt` の **親ディレクトリ (非再帰)**。試作は Host の形を真似た:

- notify の `recommended_watcher` のコールバックは、イベントと受け取った時刻を `mpsc` に送るだけ (notify 自身のスレッドで動く。05 §12.2 の 6)。
- **待ち合わせ** は別のスレッド (Host のメインの代わり) で、`a.txt` に触れるイベント (と、自分の一時ファイル・退避名 `.a.txt.eve-tmp-<pid>-*` / `.eve-bak-<pid>-*` のイベント) が来るたびに期限を「今 + 100 ms」に延ばし、期限が来たら 1 回の知らせにする。期限が無いときは `recv()` で寝る (一定の間隔では起きない。v0.5-plan §1.2 の「Host の待機の期限で自前に行う」)。読むだけの開閉 (`Access(Open)` など。inotify の `IN_OPEN`) は変更として数えない (下の所見 3)。
- **自分の保存の抑え方** は 2 通りを同時に記録した。(i) **stamp の照合**: 知らせを出す時点で `metadata(a.txt)` の `(mtime, len, FileId)` を取り、自分の最後の保存の `SaveReport.new_stamp` と一致すれば抑える (05 §12.2 の 3)。保存の途中 (`write_atomic` / `write_in_place` を呼んでから返るまで) は知らせを出さずに期限を延ばす。(ii) **名前**: まとめたイベントに自分の一時ファイル・退避名が含まれていれば抑える。
- 1 回の操作の後、イベントも知らせも 600 ms 来なくなってから次の回へ進む。1 回の操作で出た知らせの数を数える。文書は 4 KiB (大きいファイルの確認だけ 64 MiB)。

保存の種類 (08 §3 の S-24 の (a)):

| 名前 | 中身 |
|---|---|
| eve-atomic | evoeditor の原子的保存 (`write_atomic`、`Expect::Stamp`)。Windows は一時ファイル + `ReplaceFileW` (退避名を渡す)、Linux は一時ファイル + `rename` |
| eve-inplace | evoeditor の直接上書き (`write_in_place`。事前バックアップは監視していない状態ディレクトリへ) |
| ext-direct | ほかのエディタの直接書込 (`std::fs::write`: 切り詰めて書く) |
| ext-rename | ほかのエディタの一時ファイル + 改名 (`a.txt.ext-tmp` に書いて `rename` で置き換える。Windows は `MoveFileExW(REPLACE_EXISTING)`) |
| ext-vim | 退避の改名 + 新規作成 (`a.txt` → `a.txt~` に改名し、`a.txt` を作って書き、`a.txt~` を消す。Vim の `backupcopy=no` 相当) |
| git checkout | 同じディレクトリをリポジトリにして、`a.txt` の内容が違う 2 つのブランチを `git checkout` で行き来する |
| del-recreate | 削除して、すぐに作り直す |
| del-gap | 削除して、300 ms 後に作り直す (参考。待ち合わせより長く離れた 2 つの操作なので、知らせは 2 回が正しい) |
| own-then-ext | 自分の原子的保存の 20 ms 後に、ほかのプログラムが直接書く (抑えすぎて外部の変更を隠さないかの確認。正しい答えは「抑えた後に 1 回」) |
| ext-read | ほかのプログラムが読むだけ (知らせ 0 回が正しい) |

## 結果

### (a) と (1): 待ち合わせの後の知らせの回数 (各 20 回、4 KiB)

「知らせ」は待ち合わせの後の回数 (抑える前)。「抑えた後」は stamp の照合で抑えた後に残る回数。表の値は「知らせが期待どおりだった回数 / 抑えた後が期待どおりだった回数」(どちらも 20 回のうち)。

| 保存の種類 | 期待 (知らせ / 抑えた後) | NTFS (8.2.2) | NTFS (9.0-rc.9) | ext4 (8.2.2) | ext4 (9.0-rc.9) | tmpfs (8.2.2) |
|---|---|---|---|---|---|---|
| eve-atomic | 1 / **0** | 20 / 20 | 19 / 20 (下の (b) の 4 点目)、再計測 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 |
| eve-inplace | 1 / **0** | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 |
| ext-direct | 1 / 1 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 |
| ext-rename | 1 / 1 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 |
| ext-vim | 1 / 1 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 |
| git checkout | 1 / 1 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 |
| del-recreate | 1 / 1 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 |
| del-gap (参考) | 2 / 2 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 | 20 / 20 |
| own-then-ext | — / 1 | 抑えた後 20 | 抑えた後 20 | 抑えた後 20 | 抑えた後 20 | 抑えた後 20 |
| ext-read | 0 / 0 | 20 / 20 | — | 20 / 20 | 20 / 20 | 20 / 20 |

SMB のループバック (`\\localhost\C$\…`、8.2.2、各 20 回): eve-atomic・eve-inplace・ext-direct・ext-rename・ext-vim・del-recreate はすべて期待どおり (知らせ 1 回、自分の保存は抑えた後 0)。サーバ側 (ローカルのパス) から書いても、SMB で監視している側に届いた (各 5 回)。ただし 05 §12.2 の 4 と FR-117 AC-9 のとおり、SMB は監視しないで縮退する対象なので、これは参考の値である (届くかどうかはサーバの実装による)。

WSL2 の DrvFs (`/mnt/c`、8.2.2): `watch()` は成功するが、**イベントは 1 つも来ない**。Linux の側から書いても (eve-atomic・eve-inplace・ext-direct・ext-rename・del-recreate、各 5 回)、Windows の側から書いても (ext-direct・ext-rename・del-recreate、各 5 回) 0 回だった。監視を始めた時点では失敗が分からないので、FS の種類で先に見分ける必要がある (下の (c))。

### イベントの列 (20 回の中で最も多かった列。tmp は `.a.txt.eve-tmp-*`、bak は `.a.txt.eve-bak-*`)

| 保存の種類 | Windows (NTFS、ReadDirectoryChangesW) | Linux (ext4、inotify。読むだけの `Access(Open)` を除く) |
|---|---|---|
| eve-atomic | `Create tmp` → `Modify tmp` ×2 → `Name(From) a.txt` → `Name(To) bak` → `Name(From) tmp` → `Name(To) a.txt` → `Modify a.txt` → `Remove bak` | `Create(File) tmp` → `Modify(Data) tmp` → `Modify(Metadata) tmp` → `Close(Write) tmp` → `Name(From) tmp` → `Name(To) a.txt` → `Name(Both) tmp -> a.txt` |
| eve-inplace | `Modify a.txt` (1 つ) | `Modify(Data) a.txt` → `Close(Write) a.txt` |
| ext-direct | `Modify a.txt` ×2 | `Modify(Data)` ×1〜2 → `Close(Write)` |
| ext-rename | `Create tmp` → `Modify tmp` → `Remove a.txt` → `Name(From) tmp` → `Name(To) a.txt` | `Create(File) tmp` → … → `Name(From) tmp` → `Name(To) a.txt` → `Name(Both)` |
| ext-vim | `Name(From) a.txt` → `Name(To) a.txt~` → (`Modify a.txt~`) → `Create a.txt` → `Modify a.txt` → `Remove a.txt~` | `Name(From / To / Both) a.txt -> a.txt~` → `Create(File) a.txt` → `Modify(Data)` → `Close(Write)` → `Remove(File) a.txt~` |
| git checkout | `Modify .git` → `Remove a.txt` → `Create a.txt` → `Modify a.txt` → `Modify .git` ×5 | `Remove(File) a.txt` → `Create(File) a.txt` → `Modify(Data)` → `Close(Write)` |
| del-recreate | `Remove a.txt` → `Create a.txt` → `Modify a.txt` | `Remove(File)` → `Create(File)` → `Modify(Data)` → `Close(Write)` |

Windows の変更はすべて `Modify(Any)` で、Linux は `Modify(Data)` / `Modify(Metadata)` を区別する。notify 8.2.2 と 9.0.0-rc.9 で列は同じだった (違いは下の (d) の監視中のディレクトリの削除だけ)。

### 時間の幅 (8.2.2。20 回の 最小 / 中央値 / 最大、ms。「最初」は操作の開始から最初の関係するイベントまで、「幅」は関係するイベントの最初から最後まで)

| 保存の種類 | NTFS 最初 ・ 幅 | ext4 最初 ・ 幅 | SMB ループバック 最初 ・ 幅 |
|---|---|---|---|
| eve-atomic | 5.5 / 9.4 / 18.1 ・ 4.8 / 8.6 / 12.1 | 4.4 / 6.5 / 17.8 ・ 3.9 / 4.8 / 8.7 | 34.8 / 58.6 / 111.6 ・ **53.3 / 95.7 / 160.2** |
| eve-inplace | 20.2 / 28.7 / 60.4 ・ 0 | 12.2 / 15.4 / 23.4 ・ 3.5 / 4.1 / 5.9 | 65.8 / 96.8 / 151.2 ・ 0 |
| ext-direct | 0.5 / 0.7 / 1.0 ・ 0.3 / 0.5 / 1.0 | 0.2 / 0.3 / 0.5 ・ 0.0 / 0.0 / 0.2 | 2.6 / 3.7 / 4.7 ・ 0.6 / 1.1 / 18.4 |
| ext-rename | 2.0 / 3.9 / 7.5 ・ 0.0 / 0.0 / 0.1 | 0.3 / 0.6 / 1.1 ・ 0 | 5.1 / 14.7 / 78.8 ・ 0.3 / 0.5 / 2.5 |
| ext-vim | 1.3 / 2.0 / 2.9 ・ 0.6 / 1.1 / 1.8 | 0.2 / 0.2 / 0.4 ・ 0.0 / 0.1 / 1.0 | 5.0 / 8.8 / 37.8 ・ 2.6 / 12.8 / 29.9 |
| git checkout (プロセスの起動を含む) | 66.1 / 101.7 / 139.3 ・ 0.6 / 1.0 / 2.6 | 3.8 / 5.5 / 36.4 ・ 0.1 / 0.2 / 0.5 | — |
| del-recreate | 0.5 / 0.8 / 1.0 ・ 0.6 / 1.0 / 1.4 | 0.1 / 0.2 / 0.3 ・ 0.0 / 0.1 / 1.1 | 2.2 / 4.1 / 5.1 ・ 1.6 / 3.4 / 26.6 |

evoeditor の保存そのものの時間 (中央値 / 最大) は、NTFS の原子的保存 16.7 / 28.3 ms・直接上書き 32.5 / 65.9 ms、ext4 は 16.6 / 26.7 ms・19.9 / 28.5 ms、SMB のループバックの原子的保存 161.8 / 274.6 ms。

**大きいファイル (64 MiB)**: NTFS の ext-direct で、**13 回中 1 回、イベントの間が 100 ms を超えて知らせが 2 回に分かれた** (書込の途中の `Modify` が 40 ms・115 ms のように間を空けて届く。幅は 52〜124 ms)。ext4 では `Modify(Data)` が書込の間ずっと続くので分かれなかった (3 回)。evoeditor 自身の 64 MiB の保存は、保存の途中は知らせを出さない作りなので、NTFS・ext4 とも 1 回にまとまり、抑えた後 0 だった (各 3 回。SMB の原子的保存の幅 160 ms も同じ理由で 1 回にまとまった)。

### (b) と (2): 自分の保存を抑えられるか

- **stamp の照合で、evoeditor 自身の原子的保存と直接上書きを、NTFS・ext4・tmpfs・SMB のループバックで 20 回とも抑えられた** (抑えた後に残る知らせ 0)。原子的保存の後の T は一時ファイルの FileId を持ち (Windows の `ReplaceFileW`、Linux の `rename`)、直接上書きは FileId が変わらないが、どちらも `SaveReport.new_stamp` が事後の `metadata` の値なので一致する。
- own-then-ext (自分の保存の 20 ms 後にほかのプログラムが書く) では、stamp の照合は 20 回とも **外部の変更を 1 回知らせた** (隠さなかった)。一方、**名前で抑える方式は、同じ 20 回のうち NTFS で 16〜19 回、ext4 で 20 回、外部の変更を誤って抑えた** (待ち合わせの中に自分の一時ファイルのイベントが入るため)。名前は「自分の一時ファイル・退避名だけのイベントを捨てる」(文書の知らせにしない) ことにだけ使い、文書の変更を抑える判断には使えない。
- **保存の途中は知らせを出さない** ことが要る: 原子的保存の置換のイベントから保存のジョブが返るまで (退避名の削除・保存中フラグの削除・事後の `metadata`) が、SMB で 100 ms を超えた。保存の途中で期限が来たら延ばす作りで、すべて 1 回にまとまった。
- **照合の I/O は遅くなりうる**: NTFS の 9.0-rc.9 の 1 回目の計測で、原子的保存の 1 回の知らせが次の回に数えられた (19 / 20)。知らせを出す前の `metadata` が 600 ms を超えて止まったためと見ている (同じ時間帯の保存が 1 回 681 ms かかっていた。ウイルス対策の走査と見ている (推測))。知らせ自体は失われていない (抑えた後は 20 回とも 0)。再計測では 20 / 20。製品では、この照合を UI スレッドで行わない (FR-117 の「検知と再読込のための I/O は UI スレッドで行わない」、AC-8)。

### (c) と (3): 監視しない FS を監視の前に見分けられるか

| パス | 判定 | 結果 |
|---|---|---|
| `%TEMP%\…` (NTFS) | eve-platform の `FsKind` (`metadata`) | `Local` |
| `\\localhost\C$\…` (UNC、SMB) | 同上 | `Network` |
| `Z:\…` (`net use` で一時的に割り当てたネットワークドライブ。確認の後に解除) | 同上 (`GetDriveTypeW` = `DRIVE_REMOTE`) | `Network` |
| `\\wsl.localhost\Ubuntu\tmp` (Windows から見た WSL のファイル) | 同上 | `Network` |
| `~` (ext4)、`/` | eve-platform の `FsKind` / 試作の `statfs` | `Unknown` / `0xEF53` (ext2/3/4) → 監視する |
| `/dev/shm`、`/tmp` (tmpfs) | 同上 | `Unknown` / `0x01021994` (tmpfs) → 監視する |
| `/mnt/c/…`、`/mnt/e/…` (DrvFs) | 同上 | `Unknown` / `0x01021997` (v9fs) → **監視しない** |
| `/proc` | 同上 | `Unknown` / `0x9FA0` (知らない FS) → 監視しない |

- Windows は **今の eve-platform の判定 (05 §5.5 の表) がそのまま使える** (4 つとも正しく判定し、監視する側に倒れた回は 0)。
- Linux は **今の eve-platform では使えない**: `UnixFs::metadata` は `fs_kind` を常に `Unknown` にしている (`crates/eve-platform/src/unix_fs.rs`。05 §5.5 の「(v0.2、Unix) `statfs` の `f_type`」の行は未実装)。基準 (3) の「判定できない FS は監視しない側に倒す」をこのまま当てると、ext4 でも監視しないことになる。試作では `statfs(2)` の `f_type` を **許可リスト** (ext2/3/4・xfs・btrfs・tmpfs・f2fs だけを監視し、ほかはすべて監視しない) で判定し、上の 7 つを正しく分けた。許可リストにすると、知らない FS (今後の WSL の virtiofs、FUSE、overlayfs など) は監視しない側に倒れる。
- DrvFs は `watch()` が成功してイベントが来ない (上の (1))。**「監視を始めて失敗しなければ監視できる」とは言えない** ので、監視を始める前の判定が必須である。

### (d): Windows で監視中のディレクトリを外から改名・削除できるか

`P\D\a.txt` の `D` を監視した状態で、同じ操作を監視なしでも行って比べた (8.2.2 と 9.0-rc.9 で同じ結果。違いは最後の行だけ):

| 操作 | 監視あり | 監視なし |
|---|---|---|
| 監視中のディレクトリ `D` を改名 | **できる** (notify は `FILE_SHARE_DELETE` で開く)。改名の後も監視は続き、`D-renamed\b.txt` の作成が **古いパス `P\D\b.txt` で届く** | できる |
| 親 `P` を改名 (`std::fs::rename`、PowerShell の `Rename-Item` とも) | **できない** (`ERROR_ACCESS_DENIED`) | できる |
| 親 `P` を中身ごと削除 (`Remove-Item -Recurse`) | できる | できる |
| 空にした `D` を削除し、同じ名前で作り直す | できる (削除の後 `D` は見えなくなり、作り直せる) | できる |
| `D` を削除したときのイベント | 8.2.2: **何も来ない** (監視は黙って止まる)。9.0-rc.9: `Remove(Folder) P\D` | — |

05 §12.1 の「監視中のディレクトリは削除・リネームできなくなる」は、この環境では「**監視中のディレクトリの祖先を改名・移動できなくなる**」(監視中のディレクトリ自身の改名と削除はできる) だった。エクスプローラでプロジェクトのフォルダを改名すると、その下で文書を開いている間は失敗する見込み (エクスプローラでは試していない)。

### (e) と (4): アイドル (監視あり、別々のディレクトリの 10 個を監視、準備の 10 秒後から 60 秒)

| 環境 | notify | CPU 時間の増分 | 起床 |
|---|---|---|---|
| Windows | 8.2.2 | 0 ms / 60.5 s (0.1% の基準を満たす。分解能は約 15.6 ms) | 監視のスレッドが **564 回 (約 9.3 回/秒)** (`Thread` の性能カウンタの `Context Switches/sec` の生の値の差。ほかの 2 スレッドは 0) |
| Windows | 8.2.2 (監視 0 個) | 0 ms | 553 回 (監視が無くても watcher を作るだけで起きる) |
| Windows | 9.0.0-rc.9 | 0 ms / 60.4 s | **0 回** (3 スレッドとも) |
| Linux (ext4) | 8.2.2 | 0 tick (`/proc/<pid>/stat`) | **0 回** (`/proc/<pid>/task/*/status` の文脈切替) / **syscall 0 回** (`strace -f`) |
| Linux (ext4) | 9.0.0-rc.9 | 0 tick | 0 回 / syscall 0 回 |

原因: notify 8.2.2 の Windows の backend (`src/windows.rs`) は、監視のスレッドが `WaitForSingleObjectEx(wakeup_sem, 100, TRUE)` を繰り返す (100 ms ごとに操作の要求を見に起きる)。9.0.0-rc.9 では `INFINITE` で待つ。Linux の backend (inotify) は `mio::Poll::poll(&mut events, None)` で、版に依らず寝たままになる。WSL には `perf` が無く、`perf stat` は使っていない (`strace` と文脈切替の回数で代えた)。

### (f) と (5): 依存 (2026-11-20〜21 に確かめた)

| 項目 | 結果 |
|---|---|
| 版 | 8.2.2 (2026-11-08) が最新の安定版。9.0.0 は rc.1 (2026-01-25) から rc.9 (2026-11-15) まで rc が続いている |
| `cargo tree` (`default-features = false`、normal 依存) | Windows (`x86_64-pc-windows-msvc`): notify・notify-types・walkdir・same-file・winapi-util・log・bitflags・libc・windows-sys 0.61.2・windows-link。Linux (`x86_64` / `aarch64-unknown-linux-musl`): notify・notify-types・walkdir・same-file・log・bitflags・libc・**inotify**・**inotify-sys**・**mio**。9.0.0-rc.9 は両方に xxhash-rust (eve-platform が既に使う) が加わる |
| `links` を持つ crate | **0** (全プラットフォームの `cargo metadata`。inotify-sys は libc の関数の宣言だけで、C をビルドしない)。リポジトリの写しの `cargo xtask check-deps` も「C を含む依存 0 個」 |
| ライセンス | notify `CC0-1.0`、walkdir / same-file / winapi-util `Unlicense/MIT`、inotify / inotify-sys `ISC`、mio `MIT`、notify-types / log / bitflags / libc `MIT OR Apache-2.0`。リポジトリの写し (`git archive HEAD`) の `crates/eve-platform/Cargo.toml` に足して **`cargo deny check licenses bans` が通った** (8.2.2 と 9.0.0-rc.9 の両方) |
| 推移的依存の数 | **73 → 85 (+12)** (`cargo xtask check-deps` の数え方。全プラットフォームの和で、Lock に足されたのは notify・notify-types・walkdir・same-file・winapi-util・inotify・inotify-sys・mio・kqueue・kqueue-sys・wasi の 11 個。もう 1 個は、今は xtask からしか届かない log が製品から届くようになった分)。9.0.0-rc.9 でも 85。NFR-207 の目安 (90) の内だが、残りは 5 |
| macOS (参考) | `default-features = false` では FSEvents の backend が入らない。出荷する版で `macos_fsevent` (objc2-core-foundation・objc2-core-services。どちらも `links` なし、`Zlib OR Apache-2.0 OR MIT`) か `macos_kqueue` (kqueue・mio) を選ぶ |
| スレッド | notify は watcher ごとに自分のスレッドを 1 本作る (`notify-rs windows loop` / `notify-rs inotify loop`)。02 §9.1 のスレッドの一覧の「ファイル監視」の行がこれに当たる |

### (g): inotify の上限を超えたときの失敗の形 (WSL。ユーザー名前空間の中で `/proc/sys/user/max_inotify_*` を小さくして試した。`fs.inotify.*` は変えていない)

| 上限 | 失敗の形 (notify 8.2.2) |
|---|---|
| `max_inotify_watches` = 50 | 51 個目の `watch()` が `ErrorKind::MaxFilesWatch` (`ENOSPC`)、パス付き |
| `max_inotify_instances` = 8 | watcher を作る (`recommended_watcher`) ときに `ErrorKind::Io(TooManyOpenFiles)` (`EMFILE`)。ファイル記述子の枯渇と区別できない |

notify の watcher 1 個が inotify のインスタンス 1 個を使う。evoeditor がプロセスに 1 個だけ作れば、インスタンスの上限には事実上当たらない。

## 判定 (08 §3 の S-24 の合否の基準ごと)

| 基準 | 判定 | 根拠 |
|---|---|---|
| (1) 取りこぼしと重複 | **合格** | NTFS・ext4 で (a) の各保存 (evoeditor の原子的保存・直接上書き、ほかのエディタの一時ファイル + 改名・直接書込・退避の改名、`git checkout`、削除と作り直し) を 20 回ずつ行い、待ち合わせの後の知らせは、どの回もちょうど 1 回 (8.2.2)。9.0-rc.9 の 1 回の例外は照合の I/O の遅れによる数え方の境目で、知らせは失われていない (上の (b))。条件: 4 KiB の文書。**64 MiB の外部の直接書込は NTFS で 13 回中 1 回 2 回に分かれた** (所見 2) |
| (2) 自分の保存 | **合格** | evoeditor 自身の原子的保存と直接上書きを 20 回ずつ、NTFS・ext4 (と tmpfs・SMB のループバック) で行い、stamp の照合で抑えた後に残る知らせは 0。外部の変更を隠さないこと (own-then-ext) も 20 / 20。条件: 保存の途中は照合しない (完了を待つ)。名前だけで抑える方式は不可 |
| (3) 縮退 | **合格 (Windows・WSL2 の `/mnt/c`)。ただし Linux は `statfs` の判定の実装が前提。Linux の CIFS / NFS は未確認** | Windows: UNC (SMB・`\\wsl.localhost`) とネットワークドライブを今の `FsKind` で `Network` と判定 (誤って監視する側に倒れた回 0)。WSL2 の `/mnt/c`: `statfs` の `f_type` (v9fs) で判定でき、許可リスト方式なら知らない FS も監視しない側に倒れる。ただし今の `UnixFs` は `FsKind::Unknown` を返すだけで、`statfs` の判定は未実装 (05 §5.5 の v0.2 の行)。Linux に CIFS / NFS をマウントする権限が手元に無く、実際のマウントでは試していない |
| (4) アイドル | **合格 (基準の文言として)。ただし notify 8.2.2 は Windows で FR-117 AC-10 に反する** | Windows: 60 秒の CPU 時間の増分 0 ms (0.1% 未満)。Linux: 起床 0 回・syscall 0 回 (8.2.2・9.0-rc.9)。**8.2.2 の Windows の監視のスレッドは約 9.3 回/秒起きる** (基準 (4) は Windows では CPU 使用率だけを見るので合格だが、FR-117 AC-10 の「監視のために一定の間隔で起きるスレッドを持たない」と NFR-204 の「不要なタイマ起床をしない」に反する)。9.0.0-rc.9 は 0 回 |
| (5) 依存 | **合格** | `links` を持つ crate 0、ライセンスは許可リストの内 (`cargo deny` が通る)、推移的依存 73 → 85 (+12) で目安 90 の内 |
| (d)・(g) (合否に使わない) | 記録 | 上の (d)・(g)。05 §12.1 の Windows の行の書き直しと、01 §4.6 の既知の制約の材料 |

退路 (08 §3 の S-24 の行の最後の列) は使わない: (5) は満たした。(1)〜(4) も満たしたので、監視を v0.6 以降に送る必要はない。

## 所見

1. **notify の版**: 8.2.2 (最新の安定版) は Windows で 100 ms ごとに起きる。9.0.0-rc.9 は起きないが rc で、rc が 10 か月続いている。8.2.2 と 9.0.0-rc.9 は、試作で使った範囲 (`recommended_watcher`・`watch`・`unwatch`・`Event`) の API が同じで、同じコードがどちらでもビルドできた。
2. **100 ms の待ち合わせと大きいファイル**: Windows の ReadDirectoryChangesW は書込の途中のイベントを、間を空けて届けることがある (64 MiB で最大 124 ms の幅、13 回中 1 回 2 回に分かれた)。2 回目の知らせで同じ文書に同じ印を付け直すだけなら、利用者に見える害は「メッセージが 2 回出る」だけ。印とメッセージを、ディスクの stamp が前の知らせと同じなら出し直さない形にすれば、分かれても見た目は 1 回になる。
3. **読むだけでもイベントが来る (Linux)**: notify 8.2.2 / 9.0-rc.9 の inotify の backend は `IN_OPEN` を監視の対象に含め、`Access(Open)` を送る。読むだけのプロセス (grep・バックアップの道具・evoeditor 自身の照合) でも来るので、`Access` の種類 (書込の後の `Close(Write)` を除く) は変更として数えない。数えたまま (試作の `S24_KEEP_ACCESS=1`) だと、ext-read で 20 回とも知らせが出た。Windows の backend は読むだけでは何も送らない (NTFS で 20 回とも 0)。
4. **Windows のイベントは種類が粗い**: ReadDirectoryChangesW の変更はすべて `Modify(Any)` (データ・属性・ACL の区別が無い)。notify 8.2.2 は `FILE_NOTIFY_CHANGE_ATTRIBUTES` と `FILE_NOTIFY_CHANGE_SECURITY` も監視するので、属性・ACL だけの変更でも来る (試していない)。stamp の照合を「自分の最後の保存」とではなく「文書が知っているディスクの stamp (`disk_stamp`)」と比べれば、内容の変わらない変更はまとめて抑えられる。
5. **照合の置き場所**: 試作は知らせの直前に `metadata` を呼んだが、製品では UI スレッドで I/O をしない (FR-117)。保存と照合を同じ `io` のレーン (02 §9.1、直列) に積めば、「保存の途中は照合しない」が自然に満たされる (保存のジョブの結果で `disk_stamp` が更新されてから照合のジョブが走る)。照合は v0.2 の `file.check_external_changes` (`io/external.rs`) の仕組みを使える見込み (未検証)。
6. **範囲外 (別の Issue の候補)**: exFAT のドライブで試作を走らせたところ、**evoeditor の原子的保存が毎回 `SaveWarning::ChangedAfterSave` を返した** (2 回とも再現。置換の後の FileId が一時ファイルの FileId と一致しない。05 §4.1 手順 7)。この警告が出ると次の保存は照合で Conflict になる (05 §4.1 手順 7 の説明) ので、USB メモリなどの exFAT で保存のたびに外部変更の確認が出るおそれがある。また exFAT では evoeditor の直接上書きでイベントが来なかった (3 回とも 0。ほかのプログラムの直接書込は来た)。監視の基準の対象 (NTFS) ではないので深追いしていない。

## M33 (FR-117 の監視の仕様の PR) で決めること

1. **notify の版**: (a) 9.0.0 の安定版が出たら 9.0 で入れる (それまでは 8.2.x の API で書き、版だけを上げる)。(b) 9.0.0-rc を `=` で固定して入れる (rc に依存する方針の可否)。(c) 8.2.2 で入れ、Windows の 100 ms の起床を既知の制約として AC-10 を Windows では CPU 使用率だけにする。(d) 08 §3 の退路と同じく、Windows の `ReadDirectoryChangesW` を eve-platform で直接呼ぶ (unsafe と OS の分岐が増えるので、安全性・OS の違いの 2 視点のレビューに掛ける)。v0.5.0 の凍結の時点で 9.0 の安定版が無いときにどれを採るかを、先に決めておく。BM-204w に、Windows の監視のスレッドの起床の回数 (文脈切替) の記録を加えるか。
2. **Linux の `FsKind` の判定**: `UnixFs::metadata` に `statfs` の判定を入れる (05 §5.5 の v0.2 の行の実装)。監視の可否は **許可リスト** (ext2/3/4・xfs・btrfs・tmpfs・f2fs など) にするか、05 §5.5 の否定のリスト (NFS・CIFS/SMB・9p・FUSE) にするか。基準 (3) の「判定できない FS は監視しない側に倒す」に合うのは許可リスト。`FsKind` の 3 値 (`Local` / `Network` / `Unknown`) のままにするか、監視の可否を別の値 (`WatchCaps.native_events` の判定) に分けるか。
3. **待ち合わせの細目**: 100 ms で始め、期限は最後のイベントから延ばす (試作の形)。07 §6 の 300 ms (FR-706 の設定ファイルの自動再読込) と揃えるか。大きいファイルで分かれたときに、印とメッセージを出し直さない規則 (所見 2)。書き続けるログのファイルで期限が延び続けて知らせが出ないときの上限を設けるか。
4. **自分の保存の抑え方**: stamp の照合を正にし、比べる相手は文書の `disk_stamp` (所見 4)。保存の途中 (保存のジョブが `io` に積まれてから結果が返るまで) は照合しない (所見 5)。自分の一時ファイル・退避名のイベントは名前で捨てる (文書の知らせにしない) が、文書のイベントを名前で抑えない (own-then-ext の結果)。FR-706 では自分の保存を抑えない (v0.5-plan §2.4) ことの扱い。
5. **イベントの種類の扱い**: `Access` (Linux の `IN_OPEN` など) を捨てる。`Remove` で終わる回 (削除されたまま) と、`Create` で終わる回 (作り直された) を、待ち合わせの後の `metadata` の結果 (NotFound / stamp) で区別する。notify の `need_rescan()` (inotify の `IN_Q_OVERFLOW` など。試していない) は、開いているすべての文書の照合に変える。
6. **監視の単位**: 同じディレクトリの複数の文書は 1 つの監視を共有する (参照の数え上げ)。文書を閉じたら `unwatch` する。監視中のディレクトリが改名・削除されたときの扱い (8.2.2 の Windows は削除を知らせない。改名の後は古いパスで届く): 監視を張り直すか、照合が NotFound になった文書を「削除された」と表示するだけにするか。
7. **縮退の知らせ**: `MaxFilesWatch` (ENOSPC) と `Io(EMFILE)` を「監視を始められない」として 1 回だけ知らせる (05 §12.2 の 5)。文書ごとか、プロセスで 1 回か。
8. **仕様の文言の揃え**: 05 §12.1 の Windows の行の「監視中のディレクトリは削除・リネームできなくなる」を「監視中のディレクトリの祖先を改名・移動できなくなる (監視中のディレクトリ自身の改名・削除はできる)」に直し、01 §4.6 の既知の制約に足す。05 §12.2 の 2 の「`notify-debouncer-mini` 相当、または自前」を「自前 (Host の待機の期限)」に、02 §13.7 の「notify (+ debouncer)」を「notify (debouncer の crate は使わない)」に揃える (v0.5-plan §1.2 で決めたことの反映)。
9. **macOS の feature** (出荷する版): `macos_fsevent` と `macos_kqueue` のどちらにするか (FSEvents はディレクトリ単位の再帰で合体する、05 §12.1)。

## 人が確かめること (M33 の前。手元で試せなかったもの)

1. **実機の SMB の共有** (別の機械の Windows / NAS の Samba): UNC とネットワークドライブで `Network` と判定されること (判定はパスの文字列と `GetDriveTypeW` なので、サーバに依らない見込み)。参考に、イベントが届くか。
2. **OneDrive の同期フォルダ**: `Local` と判定されて監視されること。同期による更新 (別の PC で編集して同期) が 1 回の知らせになること。「ファイル オンデマンド」の状態の変化 (オンラインのみ ↔ ローカル) で、内容が変わらないのに知らせが出ないこと (所見 4 の `disk_stamp` との照合)。
3. **ディレクトリのシンボリックリンク・ジャンクションが UNC を指す場合** (Windows): 今の判定はパスの文字列とドライブ文字で見るので `Local` になり、監視する側に倒れる見込み。イベントが来るか、`FsKind` の判定で解決後のパスを見るべきか。
4. **エクスプローラでの改名**: 監視中のディレクトリの親をエクスプローラで改名したときの表示 (上の (d) は `std::fs::rename` と `Rename-Item` で確かめた)。
5. **Linux の CIFS / NFS のマウント** (root の権限が要る): `statfs` の `f_type` が許可リストの外になり、監視しないこと。
6. **macOS** (出荷する版): FSEvents の合体・遅延 (05 §12.1) の下で、(1)〜(4) を測り直す。
7. **基準機** での BM-204w (M33 の実装の後。09 §5.2)。

## 反対の論点

- **notify を使わず自前で書く案**: Windows の 100 ms の起床は notify 8.2.2 の作りの問題で、9.0 では直っている。ReadDirectoryChangesW と inotify を自前で呼ぶと unsafe と OS の分岐を自分で持つことになり (R5)、FSEvents も後で要る。9.0 の安定版を待つほうが安い、という見方が強い。一方で、9.0 の rc は 10 か月続いており、v0.5.0 までに安定版が出る保証は無い。
- **監視は要らない、という見方**: フォーカス復帰・切替・保存の確認 (v0.2) で、データの損失は既に防いでいる。監視が足すのは「表示していない文書の印」と「フォーカスの復帰を知らせない端末」での早い気付きだけで、そのために依存を 12 個 (目安までの余地 17 個のうち) 使う。FR-117 の監視は Should である。
- **待ち合わせの 100 ms は短すぎる、という見方**: 大きいファイルの書込 (Windows) と SMB の保存で、関係するイベントの幅が 100 ms を超えた。300 ms (07 §6) にすれば分かれにくいが、知らせが遅れる。知らせを出し直さない規則 (所見 2) で足りるなら 100 ms のままでよい。
- **stamp の照合の I/O**: 知らせのたびに `metadata` (と、mtime と長さが同じなら 16 MiB 以下の内容ハッシュ) を読む。書き続けるログのファイルを開いているとき、100 ms ごとに照合が走りうる。
- **Linux の許可リスト**: 許可リストに無いが inotify が効く FS (zfs・bcachefs・ecryptfs など) は監視されない (v0.2 の確認に縮退する)。誤って監視して取りこぼすよりは安全だが、利用者には「監視していない」理由が分かりにくい。縮退の知らせに FS の名前を出すか。

## 再現

スパイクのコードは `E:\tmp\s24-spike\` (Windows) と `~/s24-spike` (WSL) に置いて試し、試した後に消した。`Cargo.toml` は `notify` = `=8.2.2` (`default-features = false`) と、feature `n9` で `=9.0.0-rc.9`、`eve-core`・`eve-platform` を path 依存 (この作業ブランチ)、Unix は `libc`。

```text
s24 run <dir> <保存の種類,...> [--reps 20] [--size 4096] [--verbose 1] [--via <書く側の exe> <そのパス>]
s24 idle <base> <監視の数> <秒>        # アイドルの計測 (外から CPU 時間・文脈切替・strace で数える)
s24 lock <base>                        # 監視中のディレクトリの改名・削除 (Windows)
s24 probe <path>...                    # FsKind (eve-platform) と statfs の f_type
s24 limits <base> <ディレクトリの数> <watcher の数>   # inotify の上限 (unshare -Ur の中で /proc/sys/user/max_inotify_* を下げる)
```

Windows のアイドルは、`idle` を起動して 10 秒後から 60 秒間の `Process.TotalProcessorTime` の差と、`Thread` の性能カウンタ (`id process` / `id thread` / `context switches/sec` の `RawValue`) の差で数えた。Linux は同じ区間の `/proc/<pid>/task/*/status` の `voluntary_ctxt_switches` + `nonvoluntary_ctxt_switches` の差と、`strace -f -ttt` の下で同じ区間に呼ばれた syscall の数。依存の数は、`git archive HEAD` で作った写しの `crates/eve-platform/Cargo.toml` に `notify = { version = "8.2.2", default-features = false }` を足し、`cargo xtask check-deps` と `cargo deny check licenses bans` で見た。

### スパイクのコードの要点

待ち合わせ (Host の待機の期限。期限が無ければ寝たまま):

```rust
loop {
    let msg = match deadline {
        None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        Some(d) => rx.recv_timeout(d.saturating_duration_since(Instant::now())),
    };
    match msg {
        Ok(raw) => {
            let related = match &raw.ev {
                // 読むだけの開閉 (IN_OPEN など) は変更ではない
                Ok(ev) if matches!(ev.kind, EventKind::Access(a)
                    if a != AccessKind::Close(AccessMode::Write)) => false,
                Ok(ev) => ev.need_rescan()
                    || ev.paths.iter().any(|p| *p == target || is_own_temp(p)),
                Err(_) => true,
            };
            if related {
                batch.push(raw);
                deadline = Some(Instant::now() + DEBOUNCE); // 100 ms。最後のイベントから延ばす
            }
        }
        Err(RecvTimeoutError::Timeout) => {
            if saving() { deadline = Some(Instant::now() + DEBOUNCE); continue; } // 保存の途中は出さない
            deadline = None;
            let b = std::mem::take(&mut batch);
            if !touches(&b, &target) { continue; } // 自分の一時ファイル・退避名だけの動きは捨てる
            // 自分の保存か: 今の (mtime, len, FileId) が SaveReport.new_stamp と同じなら抑える
            let now = fsys.metadata(&target).ok().map(|m| key(&FileStamp::from_meta(&m, None)));
            let by_stamp = now.is_some() && now == own_new_stamp();
            notify_document(b, by_stamp);
        }
        Err(RecvTimeoutError::Disconnected) => break,
    }
}
```

監視してよい FS の判定 (Linux。許可リストの外は監視しない):

```rust
let mut s: libc::statfs = unsafe { std::mem::zeroed() };
if unsafe { libc::statfs(c_path.as_ptr(), &mut s) } != 0 { return false; } // 判定できない → 監視しない
matches!(s.f_type as i64,
    0xEF53            // ext2/3/4
    | 0x5846_5342     // xfs
    | 0x9123_683E     // btrfs
    | 0x0102_1994     // tmpfs
    | 0xF2F5_2010)    // f2fs
// 0x0102_1997 (v9fs: WSL2 の DrvFs)・0x6969 (nfs)・0xFF53_4D42 (cifs)・0xFE53_4D42 (smb2)・0x6573_5546 (fuse)・
// 0x6A65_6A63 (virtiofs)・0x794C_7630 (overlayfs) などはすべて監視しない
```
