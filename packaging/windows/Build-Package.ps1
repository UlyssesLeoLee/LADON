# 组装 LADON 安装包：整理文件夹 + 校验和 + zip。安装向导入口是文件夹里的 Setup.cmd。
# 用法：Build-Package.ps1 -Binary <ladon.exe 路径> [-GuiBinary <ladon-gui.exe 路径>] [-OutDir E:\LADON-dist] [-Version 0.0.4]
param(
  [Parameter(Mandatory = $true)][string]$Binary,
  [string]$GuiBinary = '',
  [string]$OutDir = 'E:\LADON-dist',
  [string]$Version = '0.0.4'
)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$repo = Resolve-Path (Join-Path $here '..\..')
if (-not (Test-Path -LiteralPath $Binary)) { throw "找不到二进制：$Binary" }
$name = "LADON-$Version-Setup"
$stage = Join-Path $OutDir $name
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

$map = @(
  @($Binary, 'ladon.exe'),
  @((Join-Path $repo 'LICENSE'), 'LICENSE'),
  @((Join-Path $repo 'config\default.yaml'), 'default.yaml'),
  @((Join-Path $repo 'config\examples\.LADONignore'), 'LADONignore.example'),
  @((Join-Path $repo 'logo\platform\windows\LADON_Explorer.ico'), 'LADON_Explorer.ico')
)
if ($GuiBinary) {
  if (-not (Test-Path -LiteralPath $GuiBinary)) { throw "找不到图形程序：$GuiBinary" }
  $map += , @($GuiBinary, 'ladon-gui.exe')
}
foreach ($f in 'Setup.cmd', 'Install-LADON.ps1', 'Uninstall-LADON.ps1', 'LADON-Shell.ps1', 'LADON-Common.ps1', 'QUICKSTART.txt', 'README_TRIAL.txt') {
  $map += , @((Join-Path $here $f), $f)
}
foreach ($m in $map) {
  if (-not (Test-Path -LiteralPath $m[0])) { throw "缺少文件：$($m[0])" }
  Copy-Item -LiteralPath $m[0] -Destination (Join-Path $stage $m[1]) -Force
}
$sums = Get-ChildItem -LiteralPath $stage -File | Sort-Object Name | ForEach-Object {
  '{0}  {1}' -f (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name
}
$sums | Set-Content -LiteralPath (Join-Path $stage 'SHA256SUMS.txt') -Encoding ASCII
Write-Output "安装包目录：$stage（$((Get-ChildItem -LiteralPath $stage -File | Measure-Object).Count) 个文件）"

$zip = Join-Path $OutDir "$name.zip"
if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -CompressionLevel Optimal
$h = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLower()
Write-Output ("压缩包：{0}（{1:N1} MB），SHA256：{2}" -f $zip, ((Get-Item -LiteralPath $zip).Length / 1MB), $h)
Write-Output "启动安装向导：双击 $stage\Setup.cmd（zip 需先完整解压再运行）"
