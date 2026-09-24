# 開発用ビルドの準備をする（本家 CI .github/workflows/actions.yml の準備工程の手元版）
#
# 使い方:  powershell -ExecutionPolicy Bypass -File scripts\setup-dev.ps1 [-CopyFrom <別のチェックアウト>]
#   -CopyFrom を付けると、llama_cpu / llama_cuda / llama_vulkan / zenz.gguf をそこから複製し、ダウンロードを省く
#
# 前提: Rust・Swift 6.1.2・protoc・node.js・Inno Setup・VS 2022 Build Tools（C++）が入っていること
# 準備のあとは README の「ビルド」の手順（vcvars64.bat と SDKROOT を通して cargo make build --release）でビルドする
param(
  [string]$CopyFrom = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')

$repo = Split-Path $PSScriptRoot -Parent
Set-Location $repo

$swiftVersion = '6.1.2'
$sdkRoot = "$env:LOCALAPPDATA\Programs\Swift\Platforms\$swiftVersion\Windows.platform\Developer\SDKs\Windows.sdk"
if (-not (Test-Path $sdkRoot)) {
  throw "Swift $swiftVersion の SDK が見つかりません: $sdkRoot"
}

# SwiftPM は変換エンジンの辞書サブモジュールを .build\checkouts\...\.git\modules\... へ clone する。
# Git for Windows の https 取得は GIT_DIR が 220 文字を超えると「'$GIT_DIR' too big」で止まるため、
# リポジトリのパスはおおむね 64 文字以内にしておく
if ($repo.Length -gt 64) {
  Write-Warning "リポジトリのパスが長すぎます（$($repo.Length) 文字）。swift build で辞書サブモジュールの取得に失敗します。64 文字以内の場所へ置いてください: $repo"
}

Write-Output '=== git: 長いパスを許可し、サブモジュール（辞書）を取得'
# SwiftPM が .build/checkouts に展開する変換エンジンはパスが長く、既定では checkout に失敗する
git config --global core.longpaths true
git submodule update --init --recursive
if ($LASTEXITCODE -ne 0) { throw 'git submodule update に失敗しました' }

Write-Output '=== rust: i686 ターゲット（32bit アプリ向けの DLL）'
rustup target add i686-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'rustup target add に失敗しました' }

Write-Output '=== swift: ucrt.modulemap の差し替え（CI と同じもの。元は .orig に退避）'
$share = "$sdkRoot\usr\share"
if (-not (Test-Path "$share\ucrt.modulemap.orig")) { Copy-Item "$share\ucrt.modulemap" "$share\ucrt.modulemap.orig" }
Invoke-WebRequest -Uri 'https://gist.githubusercontent.com/fkunn1326/ef8be2217082302b291f2b8d4178194a/raw/c424968c250afcd5afa1131aea1329dc0744a7f9/ucrt.modulemap' -OutFile "$share\ucrt.modulemap"

Write-Output '=== llama.cpp (b4846) の CPU / CUDA / Vulkan 版と zenz.gguf'
$llama = [ordered]@{
  'llama_cpu'    = 'https://github.com/fkunn1326/llama.cpp/releases/download/b4846/llama-b4846-bin-win-avx-x64.zip'
  'llama_cuda'   = 'https://github.com/fkunn1326/llama.cpp/releases/download/b4846/llama-b4846-bin-win-cuda-cu12.4-x64.zip'
  'llama_vulkan' = 'https://github.com/fkunn1326/llama.cpp/releases/download/b4846/llama-b4846-bin-win-vulkan-x64.zip'
}
foreach ($dir in $llama.Keys) {
  if (Test-Path "$dir\llama.dll") { continue }
  if ($CopyFrom -and (Test-Path "$CopyFrom\$dir\llama.dll")) {
    Copy-Item -Recurse -Force "$CopyFrom\$dir" $dir
    continue
  }
  $zip = Join-Path $env:TEMP "$dir.zip"
  Invoke-WebRequest -Uri $llama[$dir] -OutFile $zip
  Expand-Archive -Path $zip -DestinationPath $dir -Force
  Remove-Item $zip
}
Copy-Item 'llama_cpu\llama.lib' -Destination 'server-swift\' -Force
if (-not (Test-Path 'zenz.gguf')) {
  if ($CopyFrom -and (Test-Path "$CopyFrom\zenz.gguf")) {
    Copy-Item "$CopyFrom\zenz.gguf" 'zenz.gguf'
  } else {
    Invoke-WebRequest -Uri 'https://huggingface.co/Miwa-Keita/zenz-v3-small-gguf/resolve/main/ggml-model-Q5_K_M.gguf' -OutFile 'zenz.gguf'
  }
}

Write-Output '=== npm: 設定画面（frontend）の依存'
# npm ci は package-lock.json を書き換えない
Push-Location frontend
npm ci --no-audit --no-fund
$npmExit = $LASTEXITCODE
Pop-Location
if ($npmExit -ne 0) { throw 'npm ci に失敗しました' }

Write-Output '=== cargo-make'
if (-not (Get-Command cargo-make -ErrorAction SilentlyContinue)) {
  cargo install --locked cargo-make
  if ($LASTEXITCODE -ne 0) { throw 'cargo-make のインストールに失敗しました' }
}

Write-Output ''
Write-Output '準備ができました。ビルドは VS 2022 Build Tools の開発者環境で:'
Write-Output '  call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"'
Write-Output "  set SDKROOT=$sdkRoot\"
Write-Output '  cargo make build --release'
