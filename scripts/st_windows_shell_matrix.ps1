param([Parameter(Mandatory=$true)][string]$Workspace,[string]$RelativeFile="requirements.md")
$ErrorActionPreference="Stop"
Write-Host "LADON ST: Native Windows Shell Menu (manual interaction needed)"
Write-Host "1. Open the same file in Explorer; note all registered menus (Git / 7-Zip)."
Write-Host "2. Run: cargo run -- --root `"$Workspace`" shell-menu `"$Workspace\$RelativeFile`""
Write-Host "3. Compare results; test submenus, owner-drawn items, and failure fallback."
Write-Host "4. Repeat with folder, multi-selection, empty folder area; mark unsupported features as Design, not Passed."
Write-Host "5. Check sensitive option: verify .LADONignore and pending cloud-cleanup assessment."
Write-Host "No automatic success is asserted. Fill tests in docs/04_tests/要求追踪_测试管理.xlsx."
