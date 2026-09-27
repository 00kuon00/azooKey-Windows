@echo off
rem 常駐を止め、使用中のファイルの名前を変えてから、build の azookey-setup.exe を起動する（管理者の確認が出る）
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-azookey.ps1" %*
