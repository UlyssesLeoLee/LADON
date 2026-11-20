@echo off
rem Start the LADON Explorer install wizard (per-user, no administrator rights needed).
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Install-LADON.ps1" -PayloadDir "%~dp0."
