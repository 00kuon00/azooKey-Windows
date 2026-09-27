# インストーラを実行する前の準備をまとめて行い、インストーラを起動する
#
# 使い方:  scripts\install-azookey.cmd をダブルクリック
#          powershell -ExecutionPolicy Bypass -File scripts\install-azookey.ps1 [-Installer <azookey-setup.exe のパス>]
#   1. 常駐プロセス（launcher・azookey-server・ui）を止める（stop-azookey.ps1）
#   2. 前回までに名前を変えた *.old* のうち、もう使われていないものを消す
#   3. IME を使うアプリが読み込んでいて上書きできないファイル（azookey.dll・azookey32.dll・vcruntime140.dll など）の
#      名前を *.old-<日時> に変える。読み込み中のファイルでも名前は変えられる。読み込んでいるアプリは古い方を使い続け、
#      新しく開いたアプリ（または再起動のあと）から新しい方を読む
#   4. インストーラを起動する
#   管理者でなければ自分を管理者で起動し直す（UAC の確認が 1 回出る）。
#   ⚠ 3 のあとインストールしないと azookey.dll が無い状態になり、新しく開いたアプリで IME が読み込めない。
#     止めるだけのときは stop-azookey.cmd を使う
param(
  [string]$Installer = ''
)
$ErrorActionPreference = 'Stop'

if (-not $Installer) {
  $Installer = Join-Path (Split-Path $PSScriptRoot -Parent) 'build\azookey-setup.exe'
}
if (-not (Test-Path $Installer)) {
  Write-Host "インストーラが無い: $Installer"
  Start-Sleep -Seconds 5
  exit 1
}
$Installer = (Resolve-Path $Installer).Path

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
  [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
  Start-Process powershell -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Installer `"$Installer`""
  exit
}

# 1. 常駐プロセスを止める
& (Join-Path $PSScriptRoot 'stop-azookey.ps1')

$installDir = Join-Path $env:APPDATA 'Azookey'
if (Test-Path $installDir) {
  function Test-InUse([string]$Path) {
    try {
      $stream = [IO.File]::Open($Path, 'Open', 'ReadWrite', 'None')
      $stream.Close()
      return $false
    } catch {
      return $true
    }
  }

  # 2. もう使われていない *.old* を消す
  foreach ($file in Get-ChildItem $installDir -File | Where-Object { $_.Name -match '\.old(-\d+)?$' }) {
    if (-not (Test-InUse $file.FullName)) {
      Remove-Item $file.FullName -Force
      Write-Host "消した: $($file.Name)"
    }
  }

  # 3. 使用中で上書きできないファイルの名前を変える（アンインストーラは除く）
  $stamp = Get-Date -Format 'yyyyMMddHHmmss'
  foreach ($file in Get-ChildItem $installDir -File | Where-Object { $_.Extension -in '.dll', '.exe' -and $_.Name -ne 'unins000.exe' }) {
    if (Test-InUse $file.FullName) {
      $newName = "$($file.Name).old-$stamp"
      Rename-Item $file.FullName $newName
      Write-Host "使用中なので名前を変えた: $($file.Name) -> $newName"
    }
  }
}

# 4. インストーラを起動する
Write-Host "インストーラを起動する: $Installer"
Start-Process $Installer
Start-Sleep -Seconds 3
