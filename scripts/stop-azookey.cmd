@echo off
rem azooKey の常駐プロセスを止める（管理者の確認が出る）。-Start を付けると起動し直す
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0stop-azookey.ps1" %*
