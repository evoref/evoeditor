# セキュリティ上の確認の記録 (v0.1.0-rc.1)

NFR-501 AC-1 の (R) と、NFR-502 AC-2 (リリース前の確認、08 §2.2 のリリースゲート (8)) の記録。確認日: 2026-10-24。基準機の固有情報は書かない。

## NFR-501 AC-1: ネットワーク通信を行わない

| 確認 | 方法 | 結果 |
|---|---|---|
| ネットワーク系の依存が無い | `Cargo.lock` に HTTP / TLS / 非同期 I/O / ソケットの crate (`reqwest` `hyper` `ureq` `curl` `openssl` `native-tls` `rustls` `socket2` `mio` `tokio` `tungstenite` `h2` など) が 1 つも無い。`cargo deny check` の禁止 crate と依存規則 R3 を CI が毎回確かめる | 無い (xtask の試験 `製品のコードと依存にネットワーク通信が無い`) |
| `windows-sys` のネットワークの feature を使わない | 各 crate の `Cargo.toml` に `Win32_Networking*` / `Win32_NetworkManagement*` が無い | 無い (同上) |
| ネットワークを使うコードが無い (レビュー) | 試験・ベンチ・補助を除く製品のソースに `std::net`・`TcpStream`・`TcpListener`・`UdpSocket`・`WSAStartup`・`WinHttp`・`InternetOpen`・`URLDownload` が無い (同上の試験が機械で確かめる) | 無い |
| 更新確認・テレメトリ | 実装していない (ADR-0040)。`--check-update` も未実装 | 無い |

システムコールの検査 (AC-2) は v0.2。

## NFR-502 AC-2 (a): 実行ファイルと同じフォルダ・カレントディレクトリの DLL を読み込まない

- 配布する `evoeditor.exe` の起動時インポート: `api-ms-win-core-synch-l1-2-0.dll`・`bcryptprimitives.dll`・`kernel32.dll`・`user32.dll`・`advapi32.dll`・`ntdll.dll` の 6 つ。CRT は静的リンク (VC ランタイム・UCRT は取り込まない、`check-dist` の (4))。
- `kernel32` `user32` `advapi32` `ntdll` は KnownDLLs、`api-ms-win-*` は API セットで、置いた DLL には差し替えられない。**`bcryptprimitives.dll` は KnownDLLs ではなく**、Windows の既定の検索順では実行ファイルのフォルダが先に探される。
- **対策**: リンカーオプション `/DEPENDENTLOADFLAG:0x800` (`LOAD_LIBRARY_SEARCH_SYSTEM32`) を `.cargo/config.toml` の `rustflags` に入れ、実行ファイルのヘッダの `DependentLoadFlags` に 0x800 を設定した。起動時のインポートは System32 だけから探す (実行ファイルのフォルダとカレントディレクトリは探さない)。
- **機械的な確認**: `cargo xtask check-dist` の (5) が、配布ビルドの exe のヘッダの `DependentLoadFlags` に 0x800 があることを確かめる (CI の `build-release-smoke` で PR ごと、`release.yml` の展開後の exe でも)。実行時の `LoadLibrary` による動的な読込は製品のコードに無い。
- 結果: 合格。

## NFR-502 AC-2 (b): UNC パスを開いたときの認証情報

- UNC パス (`\server\share\…`) のファイルを開くと、Windows が NTLM の認証情報をそのサーバーへ送ることがある。これは OS の動作で、evoeditor からは制御できない。
- **判断**: 既知の制約に書く。`docs/01-requirements.md` §4.6 に「UNC パス」の行を足した (注意喚起のみ。信頼できないサーバーの UNC パスは開かない)。リリースノートと README の既知の制約に同じ文言を載せる (リリースの手順で転記する)。
- 結果: 確かめて記録した (合格)。
