# azooKey の常駐プロセス（launcher・azookey-server・ui）を止める。-Start で起動し直す
#
# 使い方:  scripts\stop-azookey.cmd をダブルクリック（止めるだけ）
#          powershell -ExecutionPolicy Bypass -File scripts\stop-azookey.ps1 [-Start]
#   3 つとも起動時のタスク「Azookey Startup」から管理者権限で動くので、止めるにも管理者権限が要る。
#   管理者でなければ自分を管理者で起動し直す（UAC の確認が 1 回出る）。
#   止めている間は日本語の変換と候補ウィンドウが使えない。-Start を付けるとタスクで起動し直す。
#   インストール済みのファイル（%APPDATA%\Azookey）を差し替えるときは、先にこれで止める。
param(
  [switch]$Start
)
$ErrorActionPreference = 'Stop'

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
  [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
  $arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
  if ($Start) { $arguments += ' -Start' }
  Start-Process powershell -Verb RunAs -ArgumentList $arguments
  exit
}

$installDir = Join-Path $env:APPDATA 'Azookey'
# launcher を先に止める（残っていると子の終了を待ち続けるだけで、起動し直しはしない）
foreach ($name in 'launcher.exe', 'azookey-server.exe', 'ui.exe') {
  $processes = Get-CimInstance Win32_Process -Filter "Name = '$name'" |
    Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($installDir, [StringComparison]::OrdinalIgnoreCase) }
  if (-not $processes) {
    Write-Host "$name : 動いていない"
    continue
  }
  foreach ($process in $processes) {
    Stop-Process -Id $process.ProcessId -Force
    Write-Host "$name : 止めた (PID $($process.ProcessId))"
  }
}

if ($Start) {
  schtasks /Run /TN 'Azookey Startup' | Out-Null
  Write-Host 'タスク「Azookey Startup」で起動し直した'
}

Start-Sleep -Seconds 3
