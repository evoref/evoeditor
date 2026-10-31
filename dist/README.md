# evoeditor

日本語 | [English](README.en.md)

Rust 製のテキストエディタです。Windows の端末 (Windows Terminal / コンソールホスト) で、日本語のテキストを元の文字コードと改行コードのまま安全に編集・保存できることを目指しています。

この版は **v0.1 (最小核)** です。Windows 11 (x86_64) と日本語の画面だけに対応します。

- ライセンス: Apache License 2.0 ([LICENSE](LICENSE)、[NOTICE](NOTICE))
- 利用している他のソフトウェアの告知: [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)
- 不具合の報告・意見: GitHub の Issues。脆弱性は公開の Issue に書かず、リポジトリの `SECURITY.md` の窓口へ。

## できること

- UTF-8 (BOM の有無)・UTF-16・Shift_JIS・EUC-JP・ISO-2022-JP のファイルを開き、**元の文字コードと改行コードのまま** 保存します。表現できない文字があるときは、保存の前に知らせます。
- 保存は原子的です (途中で止まっても元のファイルは壊れません)。保存の前に、他のプログラムによる変更を確かめます。
- 検索と、すべて置換。巨大なファイルは、読み込みや検索の間も入力を受け付けます。
- 元に戻す・やり直し。異常終了のときは、未保存の内容を退避します。

主なキー (コマンドパレットは `F1` または `Ctrl+P`。各コマンドに割り当てられたキーが出ます):

| キー | 動作 |
|---|---|
| `Ctrl+S` | 保存 (名前の無い文書は名前を尋ねます) |
| `Ctrl+O` | ファイルを開く |
| `Ctrl+F` | 検索 (`Ctrl+R` で置換) |
| `Ctrl+Z` | 元に戻す |
| `Ctrl+G` | 行へ移動 |
| `Ctrl+Q` | 終了 |

## インストール

インストーラはありません。レジストリやシステムの設定も書きません。管理者権限は要りません。

1. **取得**: GitHub Releases の該当の版のページから、次の 3 つを同じフォルダにダウンロードします。
   - `evoeditor-<版>-x86_64-pc-windows-msvc.zip`
   - `SHA256SUMS`
   - `SHA256SUMS.minisig`
2. **検証** (一致しなければ、以降に進みません):
   1. [minisign](https://jedisct1.github.io/minisign/) で `SHA256SUMS` の署名を検証します。公開鍵は下の「署名の検証」にあります。

      ```powershell
      minisign -Vm SHA256SUMS -P RWRtkrzH70nKORpBFPEtLy6elo0xP5XsEli9SU/OaxnBmNM8o14GKMhv
      ```

   2. zip のハッシュを求め、`SHA256SUMS` の該当の行と一致することを確かめます。

      ```powershell
      Get-FileHash -Algorithm SHA256 .\evoeditor-<版>-x86_64-pc-windows-msvc.zip
      ```

3. **ブロックの解除**: ブラウザでダウンロードしたファイルには「インターネットから来た」印が付きます。**手順 2 の検証が済んだ後で、展開する前に** 解除します。検証していないファイルの印は外さないでください。

   ```powershell
   Unblock-File .\evoeditor-<版>-x86_64-pc-windows-msvc.zip
   ```

4. **展開**: 利用者ごとのフォルダに展開します。ダウンロードのフォルダに置いたまま使わないでください (更新のときに置き換える場所が分からなくなります)。

   ```powershell
   Expand-Archive .\evoeditor-<版>-x86_64-pc-windows-msvc.zip -DestinationPath "$env:LOCALAPPDATA\Programs\evoeditor"
   ```

5. **PATH への追加**: 利用者の環境変数 `Path` に `%LOCALAPPDATA%\Programs\evoeditor` を加えます (「環境変数を編集」の画面から、またはPowerShell で次のとおり)。`setx` は使わないでください (長い `Path` を切り詰めます)。追加したら、**端末を開き直します**。

   ```powershell
   $p = [Environment]::GetEnvironmentVariable('Path', 'User')
   [Environment]::SetEnvironmentVariable('Path', "$p;$env:LOCALAPPDATA\Programs\evoeditor", 'User')
   ```

6. **確認**: 新しい端末で `evoeditor --version` を実行し、版が表示されることを確かめます。

Rust の開発環境がある方は、`cargo install --locked --git <リポジトリ> --tag v<版> evoeditor` でもインストールできます。その場合は、C ランタイムを静的にリンクするため、環境変数 `RUSTFLAGS` に `-C target-feature=+crt-static` を設定してから実行してください。

### 起動

```powershell
evoeditor                 # 名前の無い新しい文書
evoeditor memo.txt        # ファイルを開く (1 つだけ)
evoeditor --readonly a.txt
evoeditor --encoding shift_jis old.txt   # 文字コードを指定して開く
evoeditor --help
```

### 設定

設定ファイルは必須ではありません。変えたいときは、同梱の [config.example.toml](config.example.toml) を `%APPDATA%\evoeditor\config.toml` に写して編集します (フォルダが無ければ作ってください)。書き間違いがあっても起動は止まらず、その項目だけ既定値に戻して知らせます。

## 更新

エディタの中に更新の機能はありません。通信もしません。新しい版は、GitHub の Watch (Releases と Security alerts) で知ってください。

1. 新しい版のアーカイブと `SHA256SUMS`・`SHA256SUMS.minisig` をダウンロードし、「インストール」の手順 2 のとおり **署名とハッシュを検証** します。
2. evoeditor を **すべて終了** します (Windows は実行中の exe を置き換えられません)。
3. 展開したフォルダのファイルで、`%LOCALAPPDATA%\Programs\evoeditor` の中身を置き換えます。
4. `evoeditor --version` で版を確かめます。

使っていた版のアーカイブと `SHA256SUMS`・`SHA256SUMS.minisig` は、戻すときのために残しておいてください。

`cargo install` でインストールした場合は、`cargo install --locked --git <リポジトリ> --tag v<新しい版> evoeditor` をもう一度実行します。

### 設定や退避は、更新しても戻しても消えません

- 設定ファイルは、更新してもそのまま読めます。v0.x の間は、版によってキーの名前が変わることがあります。変更点は各版のリリースノートと `CHANGELOG.md` にあります。古い版に戻したとき、その版が知らないキーは警告して無視します。設定ファイルを書き換えることはありません。
- 緊急退避の内容は、どの版でも移動も削除もしません。古い版で読めない退避は、新しい版で開くか、退避の中の `.txt` を直接開いてください。

### 直前の版に戻す

自動では戻せません。手順は更新と同じです。残しておいた古い版のアーカイブを、署名とハッシュを検証してから、evoeditor を終了した状態で置き換え、`evoeditor --version` で確かめてください。

## アンインストール

展開したフォルダ (`%LOCALAPPDATA%\Programs\evoeditor`) を削除し、環境変数 `Path` からそのフォルダを外します。`cargo install` なら `cargo uninstall evoeditor` です。設定と状態のフォルダ (`%APPDATA%\evoeditor`、`%LOCALAPPDATA%\evoeditor`、緊急退避の `%TEMP%\eve-recovery-*`) は、必要なら手で削除してください。

## 署名の検証

リリースの `SHA256SUMS` には minisign の署名 (`SHA256SUMS.minisig`) が付きます。公開鍵 (リポジトリの `minisign.pub` と同じものです。両方を見比べてください):

```text
untrusted comment: minisign public key 39CA49EFC7BC926D
RWRtkrzH70nKORpBFPEtLy6elo0xP5XsEli9SU/OaxnBmNM8o14GKMhv
```

## 既知の制約 (v0.1)

- **対象は Windows 11 (x86_64) の Windows Terminal とコンソールホストです。** Windows 10、macOS、Linux は対象外です。画面は日本語のみです。
- **実行ファイルに署名がありません。** SmartScreen の警告が出ることがあります (検証を済ませたうえで、手順 3 の解除をしていれば、通常は出ません)。警告が出ても、検証が済んでいることを確かめてから「詳細情報」→「実行」を選べます。Windows 11 の **Smart App Control がオン** の環境では、署名の無い実行ファイルの起動が拒否されることがあります。この版は、その回避の手順を用意していません。「コントロールされたフォルダー アクセス」が有効だと、保護されたフォルダ (ドキュメントなど) に保存できません。ウイルス対策ソフトが誤って検出したときは、隔離から戻す前に署名とハッシュを検証し直し、Issues で知らせてください。
- **同時に開ける文書は 1 つです。** 別のファイルを開くと、今の文書を置き換えます (未保存なら確認が出ます)。コマンドラインのパスは 1 つまでです。
- **強制終了・電源断のときは、未保存の内容を退避できません。** 正常な終了とパニックのときは退避します。Windows のログオフ・シャットダウンでも、退避されないことがあります。ログオフの前に保存してください。ディスク上のファイルは、原子的な保存により壊れません。
- **不正なバイトを含むファイルは、読み取り専用で開きます。** 文字コードを指定して開き直すと回復できることがあります。読み取り専用のファイルや `--readonly` で開いた文書は編集できません (理由を示して拒否します)。別名で保存すれば編集できます。
- **1 MiB を超える長い行を含むファイルは、読み取り専用で開きます。** その行は先頭の 64 KiB まで表示します。
- **ISO-2022-JP で保存するとき、表現できない文字** (半角カナ、JIS にない互換文字など) は、保存の前に一覧を示します。置き換えて保存するか、文字コードを変えてください。補助漢字などを含む文書を元の文字コードで保存すると、一部のバイト表現が変わることがあり、その場合も保存の前に確認が出ます。
- **検索は文字列だけです** (大文字小文字の区別の切り替えあり)。正規表現はありません。置換は「すべて置換」だけです (1 回の「元に戻す」で戻せます)。
- **外部の変更は、保存するまで知らされません。** 保存のときに「上書き / 別名で保存 / 中止」を尋ねます。16 MiB を超えるファイルは、更新日時と長さで確かめるため、同じ長さで更新日時を保った書き換えを見落とすことがあります。
- 空白の可視化は全角スペースとタブだけです。ヘルプ画面はありません (キーの一覧はコマンドパレットで見ます)。クリップボードは Windows のものを使い、OSC 52 には対応しません。設定のキーは 7 個、コマンドラインのオプションは 5 個です。
- **UNC パス (`\\server\share\…`) のファイルを開くと、Windows が認証情報をそのサーバーへ送ることがあります。** これは OS の動作で、evoeditor からは制御できません。信頼できないサーバーのパスは開かないでください。
- 端末が 256 色・16 色のときは、色を自動で近似します。区別は記号と属性で保ちます。
- 日本語の入力 (IME) の変換中の表示や、キーの組み合わせは、端末と IME の組み合わせで差が出ることがあります。気づいた点は Issues で知らせてください。
