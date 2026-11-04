# Game Server Builder (GSBuilder) [日本語]

**GSBuilder** は、対話式のコマンドライン（CLI）で Minecraft や Steam のゲームサーバーを簡単に構築できる Rust 製のツールです。SteamCMD のような操作感で、依存関係の自動導入や複数形式のサーバー構築に対応しています。表示言語は `config.toml` で設定します。

---

## 🚀 インストール方法・起動方法

GitHub Releases から Linux x86_64 向けアーカイブをダウンロードし、Rust/Cargo なしでインストールできます。最新版の `GSBuilder-linux-x86_64.tar.gz` をダウンロードしてから、次を実行してください。

```bash
mkdir GSBuilder
tar -xzf GSBuilder-linux-x86_64.tar.gz -C GSBuilder
cd GSBuilder
./install.sh
```

インストーラーが `gsbuilder` コマンドを設定します。現在のターミナルで使うには、表示されたコマンド（通常は次のとおり）を一度実行してください。新しいターミナルでは不要です。

```bash
. ~/.bashrc
```

インストール後は、ターミナルのどこからでも次で起動できます。

```bash
gsbuilder
```

`./install.sh` は `~/.local/bin/gsbuilder` から展開したフォルダー内の起動スクリプトへリンクし、シェル設定に `~/.local/bin` を追加します。ソースからインストールする場合は、Git と Rust/Cargo を用意してリポジトリをcloneし、同じく `./install.sh` を実行してください。

バイナリ配布アーカイブは Linux x86_64 向けです。`src/` は含まれず、コンパイル済み実行ファイルにプログラムが含まれるため、利用時にソースやRust/Cargoは不要です。アーカイブには `GSBuilder`、`start.sh`、`install.sh`、`config.toml`、`lang/`、`data/`、README を含めます。

---

## 🛠️ コマンドリファレンス

起動後に `help` を入力すると簡易コマンド一覧が表示されます。
各コマンドに `--verbose` または `-v` フラグを付けると、説明付きの詳細ヘルプが表示されます（フラグはコマンド名の前後どちらにも配置可能です）。
パスやゲーム名、サーバー名に空白が含まれる場合は、必ず `""` (ダブルクォーテーション) で囲んでください。

入力中に上／下矢印キーを押すと、この起動中に入力したコマンド履歴を呼び出せます。履歴は終了後には保存されません。

### 🧱 基本・Minecraft サーバー管理
```text
# ヘルプの表示
GSBuilder> help
GSBuilder> help steam install -v
GSBuilder> help --verbose mc create

# API・バージョンの確認
GSBuilder> mc api list
GSBuilder> mc versions mojang
GSBuilder> mc loaders fabric 1.21.1

# サーバーの新規作成 (各種コアに対応)
GSBuilder> mc create vanilla 1.21.1 "My Server"
GSBuilder> mc create fabric 1.21.1 "Modded Server"
GSBuilder> mc create forge 1.21.1 "Forge Server"
GSBuilder> mc create neoforge 1.21.1 "NeoForge Server"
GSBuilder> mc create paper 1.21.1 "Paper Server"

# カスタムAPI・ミラーサイトの追加
GSBuilder> mc api add vanilla mirror "My API" https://example.com
GSBuilder> mc create vanilla 1.21.1 "Mirror Server" mirror

# FabricカスタムAPIの追加
GSBuilder> mc api add fabric fabric-mirror "Fabric Mirror" "https://example.com{minecraft_version}" "https://example.com" "https://example.com{version}.jar"
GSBuilder> mc create fabric 1.21.1 "Custom Fabric" fabric-mirror

# ForgeカスタムAPIの追加
GSBuilder> mc api add forge forge-mirror "Forge Mirror" https://example.com "https://example.com{minecraft_version}-{forge_version}.jar"

# Fabric Modの追加
GSBuilder> mc mod add "Modded Server" "./my-mods/some-mod.jar"

# 終了
GSBuilder> exit
```

### 🔄 アップデート
`config.toml` の `[updates].github_repository` に `OWNER/REPOSITORY` を設定すると、起動時に GitHub Releases を確認します。新しいバージョンがあれば確認を表示します。Gitチェックアウトの `start.sh` 経由で起動している場合、承諾するとローカル変更がないことを確認してリリースタグまでソースをFast-forwardし、`cargo build --release --locked` で再ビルドして実行ファイルを更新します。未コミット変更がある場合は上書きを避けて更新を中止します。Gitチェックアウト外のバイナリでは従来どおりReleaseアーカイブから実行ファイルだけを更新します。設定・APIデータ・サーバーファイルは変更しません。自動アップデートは Linux x86_64 のみ対応し、設定が空欄なら確認もしません。

Gitチェックアウト外のバイナリ配布には、`GSBuilder-linux-x86_64.tar.gz` というアセット名で実行ファイル `GSBuilder` をアーカイブのルートに含めてください。Gitチェックアウトでのソース更新には、リリースタグがGitHub上に存在し、そのタグで `cargo build --release --locked` が成功することが必要です。どちらの方式でも実行ファイルの配置先に書き込み権限が必要です。

### 💨 Steam サーバー管理
```text
# サーバーのインストール (App IDをフォルダー名として使う短い形式)
GSBuilder> steam install 1007 "My Server"

# サーバーのインストール (ゲーム名(フォルダー名)を指定してインストールする場合)
GSBuilder> steam install CoreKeeper "My Server" 1007

# Modの追加 (対象ゲーム、サーバー名、コピー元パス、[配置先相対パス])
GSBuilder> steam mod add CoreKeeper "My Server" "./my-mods" mods

# 導入済みModの一覧表示
GSBuilder> steam mod list CoreKeeper "My Server"

# Left 4 Dead 2 のWorkshop Add-onを追加 (複数のID/URLを混在可能)
GSBuilder> steam workshop item L4D2 "My Server" 1234567890
GSBuilder> steam workshop item L4D2 "My Server" "1234567890,https://steamcommunity.com/sharedfiles/filedetails/?id=2345678901"

# Left 4 Dead 2 のWorkshopコレクションをサーバー起動時に読み込む
GSBuilder> steam workshop collection L4D2 "My Server" "1234567890,https://steamcommunity.com/sharedfiles/filedetails/?id=2345678901"

# Remove an installed item or unregister a collection
GSBuilder> steam workshop remove item L4D2 "My Server" 1234567890
GSBuilder> steam workshop remove collection L4D2 "My Server" 1234567890
```

SteamCMD が `*_legacy.bin` として保存するファイルも、VPK シグネチャを確認して `.vpk` として導入します。

Workshop コマンドは現在 Left 4 Dead 2 専用で、数字のIDまたはSteam Communityの `sharedfiles/filedetails/?id=...` URLをカンマ区切りで複数指定できます。`item` はアイテムとコレクションを混在して入力でき、コレクションは入れ子を含めて展開後、SteamCMD 1回の起動でまとめて取得し、`left4dead2/addons/workshop/` にアイテムID名のVPKとして配置します。同じIDで再実行すると該当VPKを更新します。既存ファイルの上書きと更新時に不要になる旧ファイルの削除はアイテムごとに確認します。`collection` はコレクションと入れ子コレクションを展開し、サーバーごとの `addons.lst` に登録された全コレクションを同期します。更新情報を比較して、新規・更新済みのアイテムだけを取得します。`steam workshop remove item` は指定アイテムのローカルVPKと同期情報を削除します。削除したアイテムが登録済みコレクションに含まれる場合、次回のコレクション同期で再導入されます。`steam workshop remove collection` はコレクション登録を解除し、他の登録コレクションで使われていない管理対象VPKも削除します。削除前には確認を表示します。Steam APIでコレクションを取得できない場合は公開Workshopページも確認します。サーバー起動用の `start-workshop.sh` は最後に指定したコレクションIDを `+host_workshop_collection` に設定します。Steamアカウントが必要なアイテムでは `[steam].login_mode = "account"` を設定してください。

---

## 📝 サーバー構築の仕様

### 📂 保存先
`config.toml` の `[paths]` セクションに従います。デフォルト設定は以下の通りです。
- **Minecraft:** `servers/Minecraft/<サーバー名>`
- **Steam:** `servers/<ゲーム名>/<サーバー名>`
- ⚠️ **上書き防止:** 既存データの上書きを避けるため、構築先フォルダーが空でない場合は構築を拒否します。
- 💡 **例外:** Fabric の構築が途中で失敗し、構築先に `fabric-installer.jar` だけが残っている場合に限り、同じ `mc create fabric ...` コマンドを再実行して処理を再開できます。それ以外の既存ファイルがある場合は引き続き拒否されます。

### 🟩 Minecraft の挙動
- Vanilla、Fabric、Forge、NeoForge、Paper と Minecraft のバージョンを選べます。
- 対応 API からサーバー JAR またはインストーラーを取得し、Forge／NeoForge は生成されたインストーラーから起動スクリプトを呼び出す起動ファイルを自動生成します。
- Minecraft の API 定義は `data/minecraft/apis.json` に保存されます。既定では `mojang`、`fabric`、`forge`、`neoforge`、`paper` の公式メタデータ URL を登録します。
- サーバーの起動ファイル (`start.bat` / `start.sh`) と `eula.txt` を自動生成します。起動前に必ず EULA を確認し、同意する場合に限り `eula.txt` の `eula=false` を `eula=true` に変更してください。

### 💨 Steam の挙動
- Steam サーバーは SteamCMD の dedicated-server App ID を入力して構築します。
- `[steam].login_mode` が `account` の場合、インストール時に Steam ユーザー名を求められます。パスワードや Steam Guard コードは SteamCMD の対話プロンプトに入力し、GSBuilder の設定やコマンド引数には保存されません。
- Fabric Mod は `mc mod add` で `mods/` へコピーできます。Steam Mod は `steam mod add` でローカルファイルをコピーできます。Left 4 Dead 2 は `steam workshop item` でWorkshop Add-onを取得し、`steam workshop collection` でコレクションの同期と更新を行います。ほかのSteamゲームのWorkshop連携には対応していません。

---

## ⚙️ 設定ファイル (`config.toml`)

各設定パスは、絶対パスでない限り `config.toml` が置かれたディレクトリからの相対パスとして処理されます。
`GSBUILDER_CONFIG` 環境変数を使用することで、任意の `config.toml` のパスを指定できます。

```toml
[language]
code = "ja"                         # "ja" または "en"
japanese_file = "lang/ja.json"
english_file = "lang/en.json"

[paths]
minecraft_servers = "servers/Minecraft"
steam_servers = "servers/{game}"    # {game} はゲーム名に自動置換

[minecraft]
java_executable = "java"            # PATH上の名前、または実行ファイルのパス
minimum_memory = "1G"
maximum_memory = "2G"
api_registry = "data/minecraft/apis.json"

[steam]
steamcmd_executable = "steamcmd"    # PATH上の名前、または実行ファイルのパス
validate_install = true
run_as_user = ""          # 空なら GSBuilder と同じ OS ユーザー。指定時は Unix系では sudo、Windowsでは runas で起動
login_mode = "anonymous"  # "anonymous" または "account"

[network]
timeout_seconds = 120
user_agent = "GSBuilder/1.3.0"

[updates]
github_repository = "radoemon-com/GSBuilder"
```

---

## 📋 動作環境 & 依存関係 (Requirements)

- **OS:** Linux および Rust ツールチェーン（GSBuilder のビルド・実行用）
- **Java:** Minecraft サーバーおよび Fabric, Forge, NeoForge インストール用に、`java` として利用可能であること。
- **SteamCMD:** `PATH` 上に存在するか、`config.toml` の `steamcmd_executable` で設定されていること。
- **ネットワーク環境:** サーバーファイルのダウンロードのためのインターネットアクセス。

### 🔄 起動時の自動検出と導入
起動時に Java または SteamCMD が見つからない場合は、インストールするか確認するプロンプトが表示されます。
- **依存関係の自動導入対象 OS:** Linux (apt / dnf / pacman / zypper)
- **Java:** OS のパッケージマネージャー経由で導入します。
- **SteamCMD:** 必要な 32-bit ランタイムを同じパッケージマネージャーから導入した後、Valve の Linux アーカイブから選択したユーザーの `~/.local/share/gsbuilder/steamcmd` に配置します。選んだユーザーと実行ファイルのパスは `config.toml` に自動で保存されます。別ユーザーへの導入（`run_as_user` 指定時など）には `sudo` 権限が必要です。なお、既存の SteamCMD インストール先は上書きしません。

---

## 📂 ソースコード構成 (Source Layout)

- `src/app/` : アプリケーションの起動、依存関係のセットアップ、CLI フロー制御
- `src/core/` : 設定管理、多言語化（i18n）、エラーハンドリング、HTTP クライアント、パス・ファイル操作
- `src/minecraft/` : Minecraft 関連機能、統合 Vanilla/Fabric インストーラー、API レジストリ (JSON)
- `src/steam/` : Steam サーバー機能、SteamCMD 制御、Mod ファイル管理
- `data/minecraft/apis.json` : Minecraft バージョンマニフェストの永続化用 API レジストリ
- `config.toml` : 実行時設定ファイル

---

## 🌐 言語設定 (Languages)

言語は `config.toml` の `[language].code` で切り替えます。翻訳ファイルのパスも同じセクションで個別に指定可能です。
- 翻訳キーが不足している場合はエラーになります。
- 言語ファイル、API レジストリ、サーバー保存先の相対パスは、すべて `config.toml` が置かれている場所を基準に解決されます。

# Game Server Builder (GSBuilder) [English]

**GSBuilder** is a Rust-based interactive command-line tool for easily setting up Minecraft and Steam game servers. It offers a SteamCMD-like interface, automatic dependency setup, and support for multiple server types. Set the display language in `config.toml`.

---

## 🚀 Installation & Launch

Binary releases are not available yet. Install from source using Git and Rust/Cargo. Run these commands in a terminal:

```bash
git clone https://github.com/radoemon-com/GSBuilder.git
cd GSBuilder
./install.sh
```

The installer sets up the `gsbuilder` command. To use it in the current terminal, run the command printed by the installer (usually this):

```bash
. ~/.bashrc
```

After installation, launch GSBuilder from any directory:

```bash
gsbuilder
```

Download the latest `GSBuilder-linux-x86_64.tar.gz` from GitHub Releases and extract it. Rust/Cargo are not required for binary installation:

```bash
mkdir GSBuilder
tar -xzf GSBuilder-linux-x86_64.tar.gz -C GSBuilder
cd GSBuilder
./install.sh
```

The installer links `~/.local/bin/gsbuilder` to the launcher in the extracted folder and adds `~/.local/bin` to your shell configuration. To build from source instead, clone the repository and run `./install.sh` with Git and Rust/Cargo installed.

The binary release archive supports Linux x86_64 and does not include `src/`. The compiled `GSBuilder` executable contains the program, so users do not need the source code or Rust/Cargo to run it. The archive includes `GSBuilder`, `start.sh`, `install.sh`, `config.toml`, `lang/`, `data/`, and this README.

---

## 🛠️ Command Reference

Type `help` after launching to display a concise command list.
Append `--verbose` or `-v` to a help topic to see detailed descriptions (the flag can appear before or after the topic).
If paths, game names, or server names contain spaces, enclose them in double quotes (`"..."`).

Press the Up/Down arrow keys while entering commands to recall commands entered during the current session. Command history is not saved after exiting.

### 🧱 General & Minecraft Server Management
```text
# Display help
GSBuilder> help
GSBuilder> help steam install -v
GSBuilder> help --verbose mc create

# Check APIs and versions
GSBuilder> mc api list
GSBuilder> mc versions mojang
GSBuilder> mc loaders fabric 1.21.1

# Create a server (supports multiple server types)
GSBuilder> mc create vanilla 1.21.1 "My Server"
GSBuilder> mc create fabric 1.21.1 "Modded Server"
GSBuilder> mc create forge 1.21.1 "Forge Server"
GSBuilder> mc create neoforge 1.21.1 "NeoForge Server"
GSBuilder> mc create paper 1.21.1 "Paper Server"

# Add a custom API or mirror
GSBuilder> mc api add vanilla mirror "My API" https://example.com
GSBuilder> mc create vanilla 1.21.1 "Mirror Server" mirror

# Add a custom Fabric API
GSBuilder> mc api add fabric fabric-mirror "Fabric Mirror" "https://example.com{minecraft_version}" "https://example.com" "https://example.com{version}.jar"
GSBuilder> mc create fabric 1.21.1 "Custom Fabric" fabric-mirror

# Add a custom Forge API
GSBuilder> mc api add forge forge-mirror "Forge Mirror" https://example.com "https://example.com{minecraft_version}-{forge_version}.jar"

# Add a Fabric Mod
GSBuilder> mc mod add "Modded Server" "./my-mods/some-mod.jar"

# Exit
GSBuilder> exit
```

### 🔄 Updates
Set `[updates].github_repository` in `config.toml` to `OWNER/REPOSITORY` to check GitHub Releases at startup. If a newer version is available, GSBuilder asks before updating. When launched through `start.sh` from a Git checkout, it verifies that the worktree has no local changes, fast-forwards the source to the release tag, rebuilds with `cargo build --release --locked`, and installs the rebuilt executable. If there are uncommitted changes, the update is stopped rather than overwriting them. Binaries outside a Git checkout continue to update only the executable from the release archive. Configuration, API data, and server files are not changed. Automatic updates are supported on Linux x86_64 only; leaving this setting empty disables update checks.

For binary installations outside a Git checkout, publish a release asset named `GSBuilder-linux-x86_64.tar.gz` containing an executable named `GSBuilder` at the archive root. For source installations, each release tag must build successfully with `cargo build --release --locked`. The directory containing the executable must be writable.

### 💨 Steam Server Management
```text
# Install a server (short form uses the App ID as the folder name)
GSBuilder> steam install 1007 "My Server"

# Install a server with a specified game/folder name
GSBuilder> steam install CoreKeeper "My Server" 1007

# Add a Mod (game, server, source path, [relative destination])
GSBuilder> steam mod add CoreKeeper "My Server" "./my-mods" mods

# List installed Mods
GSBuilder> steam mod list CoreKeeper "My Server"

# Install Left 4 Dead 2 Workshop add-ons (comma-separated IDs/URLs)
GSBuilder> steam workshop item L4D2 "My Server" 1234567890
GSBuilder> steam workshop item L4D2 "My Server" "1234567890,https://steamcommunity.com/sharedfiles/filedetails/?id=2345678901"

# Load a Left 4 Dead 2 Workshop collection when the server starts
GSBuilder> steam workshop collection L4D2 "My Server" "1234567890,https://steamcommunity.com/sharedfiles/filedetails/?id=2345678901"

# Remove an installed item or unregister a collection
GSBuilder> steam workshop remove item L4D2 "My Server" 1234567890
GSBuilder> steam workshop remove collection L4D2 "My Server" 1234567890
```

Files SteamCMD stores as `*_legacy.bin` are also installed as `.vpk` after checking their VPK signature.

Workshop commands currently support Left 4 Dead 2 only and accept comma-separated numeric IDs or Steam Community URLs. `item` accepts items and collections together, expands nested collections, downloads all resulting items in one SteamCMD run, and installs ID-named VPK files in `left4dead2/addons/workshop/`. It asks before overwriting existing files and before deleting stale files after an update. `collection` expands nested collections and synchronizes all collections registered in the server's `addons.lst`. `steam workshop remove item` deletes local files and sync metadata for an item; if a registered collection still contains it, a later sync reinstalls it. `steam workshop remove collection` unregisters a collection and deletes managed VPKs not used by another registered collection. Both deletion commands ask for confirmation. The generated `start-workshop.sh` passes the most recently specified collection ID through `+host_workshop_collection` to `srcds_run`. Set `[steam].login_mode = "account"` for items requiring a Steam account.

---

## 📝 Server Setup Details

### 📂 Save Locations
Locations are configured in the `[paths]` section of `config.toml`. The default locations are:
- **Minecraft:** `servers/Minecraft/<server_name>`
- **Steam:** `servers/<game_name>/<server_name>`
- ⚠️ **Preventing overwrites:** To protect existing data, setup is refused if the target folder is not empty.
- 💡 **Exception:** If Fabric setup fails and leaves only `fabric-installer.jar` in the target folder, you can rerun the same `mc create fabric ...` command to resume. Setup remains blocked if any other files are present.

### 🟩 Minecraft Behavior
- Choose a Minecraft version and one of Vanilla, Fabric, Forge, NeoForge, or Paper.
- The server JAR or installer is downloaded from the selected API. For Forge/NeoForge, GSBuilder creates launch files that call the startup script produced by the installer.
- GSBuilder creates server launch files (`start.bat` / `start.sh`) and `eula.txt`. Read the EULA before launching the server. Only if you agree to it, change `eula=false` to `eula=true` in `eula.txt`.
- Minecraft API definitions are stored in `data/minecraft/apis.json`. The default registry includes official metadata URLs for `mojang`, `fabric`, `forge`, `neoforge`, and `paper`.

### 💨 Steam Behavior
- Enter the Steam dedicated-server App ID to install a Steam server using SteamCMD.
- If `[steam].login_mode` is `account`, GSBuilder asks for your Steam username during installation. Enter your password and Steam Guard code in SteamCMD's interactive prompt; they are not saved in GSBuilder settings or command arguments.
- Use `mc mod add` to copy Fabric Mods into `mods/`. `steam mod add` copies local Steam mod files. For Left 4 Dead 2, `steam workshop item` downloads Workshop add-ons and `steam workshop collection` synchronizes registered collections and their updates. Other Steam games' Workshop integrations are not supported.

---

## ⚙️ Configuration (`config.toml`)

Unless a path is absolute, it is resolved relative to the directory containing `config.toml`.
Set the `GSBUILDER_CONFIG` environment variable to use a different config file.

```toml
[language]
code = "ja"                         # "ja" or "en"
japanese_file = "lang/ja.json"
english_file = "lang/en.json"

[paths]
minecraft_servers = "servers/Minecraft"
steam_servers = "servers/{game}"    # {game} is replaced with the game name

[minecraft]
java_executable = "java"            # Name on PATH or executable path
minimum_memory = "1G"
maximum_memory = "2G"
api_registry = "data/minecraft/apis.json"

[steam]
steamcmd_executable = "steamcmd"    # Name on PATH or executable path
validate_install = true
run_as_user = ""                    # Empty uses the current OS user; otherwise Unix uses sudo and Windows uses runas
login_mode = "anonymous"            # "anonymous" or "account"

[network]
timeout_seconds = 120
user_agent = "GSBuilder/1.3.0"

[updates]
github_repository = "radoemon-com/GSBuilder"
```

---

## 📋 Requirements & Dependencies

- **OS:** Linux and the Rust toolchain (to build and run GSBuilder).
- **Java:** Must be available as `java` to build or launch Minecraft servers and to install Fabric, Forge, or NeoForge.
- **SteamCMD:** Must be on `PATH` or configured via `steamcmd_executable` in `config.toml`.
- **Network:** Internet access is required to download server files.

### 🔄 Startup Detection & Installation
At startup, GSBuilder asks whether to install Java or SteamCMD if either is missing.
- **Supported package managers for automatic setup:** Linux (apt / dnf / pacman / zypper).
- **Java:** Installed through the OS package manager.
- **SteamCMD:** After installing the required 32-bit runtime packages, GSBuilder downloads Valve's Linux archive and installs it under `~/.local/share/gsbuilder/steamcmd` for the selected user. The selected user and executable path are saved in `config.toml`. Installing for a different user (when `run_as_user` is set) requires `sudo`. An existing SteamCMD installation is not overwritten.

---

## 📂 Source Layout

- `src/app/`: Application startup, dependency setup, and CLI flow.
- `src/core/`: Configuration, localization (i18n), error handling, HTTP client, and path/file operations.
- `src/minecraft/`: Minecraft features, unified Vanilla/Fabric installer, and JSON API registry.
- `src/steam/`: Steam server features, SteamCMD control, and Mod file management.
- `data/minecraft/apis.json`: Persistent Minecraft version-manifest API registry.
- `config.toml`: Runtime configuration file.

---

## 🌐 Languages

Set the language in `[language].code` in `config.toml`. You can also configure separate paths for the translation files in the same section.
- The application reports an error if required translation keys are missing.
- Relative paths for language files, the API registry, and server destinations are resolved relative to the directory containing `config.toml`.
