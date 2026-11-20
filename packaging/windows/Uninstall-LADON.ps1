# LADON Explorer 卸载程序：只还原安装清单（install-manifest.json）里记录的内容。
param(
  [switch]$Silent,
  [switch]$RemoveSample,
  [string]$InstallDir = $PSScriptRoot
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'LADON-Common.ps1')
$manifestPath = Join-Path $InstallDir 'install-manifest.json'
if (-not (Test-Path -LiteralPath $manifestPath)) { Write-Output "找不到安装清单：$manifestPath"; exit 1 }
$m = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json

if (-not $Silent) {
  Add-Type -AssemblyName System.Windows.Forms
  $ans = [System.Windows.Forms.MessageBox]::Show("确定卸载 LADON Explorer 吗？`r`n`r`n安装目录：$($m.installDir)", 'LADON Explorer 卸载', 'YesNo', 'Question')
  if ($ans -ne 'Yes') { exit 0 }
  if ($m.sampleCreated -and $m.sampleDir -and (Test-Path -LiteralPath $m.sampleDir)) {
    $a2 = [System.Windows.Forms.MessageBox]::Show("是否同时删除示例工作区？`r`n$($m.sampleDir)`r`n`r`n选「否」将保留其中的文件和历史。", 'LADON Explorer 卸载', 'YesNo', 'Question', 'Button2')
    if ($a2 -eq 'Yes') { $RemoveSample = $true }
  }
}
function Say([string]$t) { Write-Output $t }

if ($m.registryName) {
  $key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$($m.registryName)"
  if (Test-Path -LiteralPath $key) { Remove-Item -LiteralPath $key -Recurse -Force; Say '已移除「已安装的应用」登记' }
}
if ($m.pathAdded) {
  $old = Get-UserPathRaw
  $new = Remove-PathEntry $old $m.installDir
  if ($new -ne $old) { Set-UserPathRaw $new; Say '已从用户 PATH 移除安装目录' }
}
foreach ($l in @($m.shortcuts)) { if ($l -and (Test-Path -LiteralPath $l)) { Remove-Item -LiteralPath $l -Force } }
if ($m.shortcutDir -and (Test-Path -LiteralPath $m.shortcutDir)) {
  if (@(Get-ChildItem -LiteralPath $m.shortcutDir -Force).Count -eq 0) { Remove-Item -LiteralPath $m.shortcutDir -Force; Say '已移除开始菜单项' }
}
if ($RemoveSample -and $m.sampleCreated -and $m.sampleDir -and (Test-Path -LiteralPath $m.sampleDir)) {
  Remove-Item -LiteralPath $m.sampleDir -Recurse -Force; Say "已删除示例工作区：$($m.sampleDir)"
}
foreach ($f in @($m.files)) {
  $p = Join-Path $InstallDir $f
  if ((Test-Path -LiteralPath $p) -and ((Split-Path $p -Leaf) -ne 'Uninstall-LADON.ps1') -and ((Split-Path $p -Leaf) -ne 'LADON-Common.ps1')) { Remove-Item -LiteralPath $p -Force }
}
foreach ($d in 'config\examples', 'config') {
  $p = Join-Path $InstallDir $d
  if ((Test-Path -LiteralPath $p) -and @(Get-ChildItem -LiteralPath $p -Force).Count -eq 0) { Remove-Item -LiteralPath $p -Force }
}
Say '文件已清理；稍后移除自身脚本，并在安装目录已空时移除该目录…'
# 卸载脚本自己还在这个目录里：交给一个延迟执行的隐藏 cmd 在本进程退出后删除这两个脚本，再用不带 /s 的 rmdir 移除目录；目录里清单之外的文件不会被删除，此时 rmdir 会失败并保留目录。
$cmdLine = "/c ping -n 3 127.0.0.1 >nul & del /q `"$InstallDir\Uninstall-LADON.ps1`" `"$InstallDir\LADON-Common.ps1`" & rmdir `"$InstallDir`""
Start-Process -FilePath "$env:SystemRoot\System32\cmd.exe" -ArgumentList $cmdLine -WindowStyle Hidden
if (-not $Silent) { [void][System.Windows.Forms.MessageBox]::Show('LADON Explorer 已卸载。', 'LADON Explorer 卸载') }
Say 'UNINSTALL OK'
