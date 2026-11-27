# 用 WiX Toolset 5 生成 LADON 的 MSI 安装包。
# 前置：dotnet tool install --global wix --version 5.*
# 用法：build-msi.ps1 -SourceDir <已组装的安装目录> [-Version 0.0.4] [-OutDir E:\LADON-dist]
# SourceDir 应为 Build-Package.ps1 生成的 LADON-<版本>-Setup 目录（含 ladon.exe 与 ladon-gui.exe）。
param(
  [Parameter(Mandatory = $true)][string]$SourceDir,
  [string]$Version = '0.0.4',
  [string]$OutDir = 'E:\LADON-dist'
)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
foreach ($f in 'ladon.exe', 'ladon-gui.exe', 'LICENSE', 'default.yaml', 'LADONignore.example', 'LADON_Explorer.ico', 'README_TRIAL.txt', 'QUICKSTART.txt') {
  if (-not (Test-Path -LiteralPath (Join-Path $SourceDir $f))) { throw "缺少文件：$f（$SourceDir）" }
}
$wix = Get-Command wix -ErrorAction SilentlyContinue
if (-not $wix) { throw '未找到 wix 命令。请先运行：dotnet tool install --global wix --version 5.*' }
if (-not (Test-Path -LiteralPath $OutDir)) { New-Item -ItemType Directory -Path $OutDir | Out-Null }
$out = Join-Path $OutDir "LADON-$Version-x64.msi"
& wix build -arch x64 -culture zh-CN -ext WixToolset.UI.wixext -d "SourceDir=$SourceDir" -d "Version=$Version" -d "LicenseRtf=$(Join-Path $here 'License.rtf')" -o $out (Join-Path $here 'LADON.wxs')
if ($LASTEXITCODE -ne 0) { throw "wix build 失败，退出码 $LASTEXITCODE" }
$h = (Get-FileHash -LiteralPath $out -Algorithm SHA256).Hash.ToLower()
Write-Output ("MSI：{0}（{1:N1} MB），SHA256：{2}" -f $out, ((Get-Item -LiteralPath $out).Length / 1MB), $h)

