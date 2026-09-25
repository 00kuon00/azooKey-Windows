# azooKey for Windows

[AzooKeyKanaKanjiConverter](https://github.com/azooKey/AzooKeyKanaKanjiConverter)を利用したWindows版IMEです。

> [!WARNING]
> 現在開発中であるため、安定性や機能に関しては保証できません。使用する際は自己責任でお願いします。

# インストール方法
[Release](https://github.com/fkunn1326/azooKey-Windows/releases)から`azookey-setup.exe`をダウンロードし、インストーラーを実行してください。

# 機能

- [x] ライブ変換
- [x] Zenzaiを使用したニューラルかな漢字変換

- [ ] 学習機能
- [ ] 辞書登録機能
- [ ] テーマ変更機能
- [ ] 辞書のインポート/エクスポート機能
- [ ] いい感じ変換
- [ ] 個人最適化システム
- [ ] 予測変換

# キー操作

| キー | 動作 |
|---|---|
| `Ctrl+Space` | 日本語 / 英字を切り替える（半角/全角キーと同じ） |
| `Shift+←` / `Shift+→` | 変換中に、いま選んでいる文節の読みを 1 文字縮める / 伸ばし、変換し直す |

`Ctrl+Space` は IME が受けるので、半角/全角キーの無い英語配列でも切り替えられます。AutoHotkey で `Ctrl+Space` を半角/全角キー（`vkF3`）に置き換えていた場合（`ime-toggle.ahk` の `^Space::Send "{vkF3}"` など）は、そのスクリプトを止めても切り替えられます。
なお、`Ctrl+Space` を自分のショートカットに使うアプリでは、そのショートカットより IME の切り替えが優先されます。

# 設定

## Zenzai

### 変換プロファイル
設定で変換プロファイルを指定すると、プロファイルに応じた変換候補が表示されます。

### バックエンド
以下の3種類のバックエンドをサポートしています。

- **CPU**: 動作が非常に遅いため、非推奨です。
- **CUDA**: NvidiaのGPU専用。[CUDA Toolkit 12系](https://developer.nvidia.com/cuda-downloads)をインストールする必要があります。
- **Vulkan**: GPUのドライバーに標準で含まれているため、追加のインストールは不要です。

# コミュニティ

## 開発を支援する
- [GitHub Sponsors (Miwa)](https://github.com/sponsors/ensan-hcl): 変換エンジンの開発者
- [Patreon (fkunn1326)](https://www.patreon.com/c/fkunn1326): Windowsに移植した人

## 開発に参加する

### 開発環境のセットアップ

- [Rust](https://www.rust-lang.org/tools/install)
- [Swift for Windows](https://www.swift.org/install/windows/) (Swift 6.1.2)
- [protoc](https://protobuf.dev/installation/) 
- [node.js](https://nodejs.org/en/download/)
- [inno setup](https://jrsoftware.org/isinfo.php)
- [Visual Studio 2022 Build Tools](https://visualstudio.microsoft.com/ja/downloads/) (「C++ によるデスクトップ開発」)

### ビルド

#### リポジトリのクローン
```
git clone https://github.com/fkunn1326/azookey-Windows --recursive
```
`--recursive`オプションを付けて、サブモジュールも一緒にクローンしてください。

#### 準備
```
powershell -ExecutionPolicy Bypass -File scripts\setup-dev.ps1
```
CI（`.github/workflows/actions.yml`）の準備工程と同じことを手元で行います。サブモジュールの取得、i686 ターゲットの追加、Swift SDK の `ucrt.modulemap` の差し替え（元のファイルは `ucrt.modulemap.orig` に退避）、llama.cpp と `zenz.gguf` のダウンロード、`npm ci`、cargo-make のインストールです。

#### ビルド
VS 2022 Build Tools の `vcvars64.bat` を通し、`SDKROOT` を設定したコマンドプロンプトで実行します。
```
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
set SDKROOT=%LOCALAPPDATA%\Programs\Swift\Platforms\6.1.2\Windows.platform\Developer\SDKs\Windows.sdk\
cargo make build [--debug/--release]
```
`--debug`オプションを付けるとデバッグビルド、`--release`オプションを付けるとリリースビルドになります。必ずどちらかを指定してください。

`build`フォルダーが作成され、ビルドされた実行ファイルが格納されます。

リポジトリのパスは 64 文字以内にしてください。長いと、SwiftPM が変換エンジンの辞書サブモジュールを取得するところで `'$GIT_DIR' too big` になって止まります。

#### Swift のテスト
上と同じコマンドプロンプトで、`llama.dll` が見つかるよう `llama_cpu` を `PATH` に足して実行します。
```
set PATH=%CD%\llama_cpu;%PATH%
cd server-swift
swift test -c release -Xcc -D_CRT_USE_C_COMPLEX_H -Xcxx -D_CRT_USE_C_COMPLEX_H
```
`-D_CRT_USE_C_COMPLEX_H` は、Zenzai の C++ 相互運用で Windows SDK の `complex.h` が `<ccomplex>` を読みに行って失敗するのを止めるためのものです（`cargo make build` でも同じものを渡しています）。

`launcher.exe`を管理者権限で実行すると、azookeyの変換エンジンが起動します。

また、IMEを登録する際は以下のように`regsvr32.exe`を使用して登録する必要があります。
```c
regsvr32.exe "path/to/build/azookey_windows.dll" /s
regsvr32.exe "path/to/build/x86/azookey_windows.dll" /s
```
逆に登録を解除する場合は`/u`オプションを付けて実行してください。

#### 開発時のヒント
- 開発は仮想マシンまたは専用のPCで行うことを推奨します。IMEがクラッシュするとWindowsがフリーズする可能性があります。
- IMEを解除する際、IMEを使用中のアプリケーション（メモ帳など）を終了しないと、解除できないことがあります。
- 常駐プロセス（`launcher.exe`・`azookey-server.exe`・`ui.exe`）は管理者権限で動くので、止めるには `scripts\stop-azookey.cmd` をダブルクリックします（UAC の確認が 1 回出ます）。`scripts\stop-azookey.cmd -Start` で、タスク「Azookey Startup」から起動し直します。インストール済みのファイルを差し替えたり、インストーラを実行し直したりする前に止めてください。

# 関連

- [azooKey/azooKey](https://github.com/azooKey/azooKey): iOS / iPadOS向けの日本語キーボードアプリ
- [7ka-Hiira/fcitx5-hazkey](https://github.com/7ka-Hiira/fcitx5-hazkey): fcitx5向けのLinux版azooKey
- [azooKey/AzookeyKanakanjiConverter](https://github.com/azooKey/AzooKeyKanaKanjiConverter): azooKeyの変換エンジン

# 参考
本プロジェクトの開発にあたり、以下のリソースを参考にしました。ありがとうございます！
- [OMAMA-Taioan/khiin-rs](https://github.com/OMAMA-Taioan/khiin-rs/tree/master/windows)
- [google/mozc](https://github.com/google/mozc/tree/master/src/win32/tip)
- [microsoft/Windows-classic-samples](https://github.com/microsoft/Windows-classic-samples/tree/main/Samples/Win7Samples/winui/input/tsf/textservice)
- [dec32/ajemi](https://github.com/dec32/ajemi)
- https://zenn.dev/mkpoli/scraps/6dc57fcd0335cf
