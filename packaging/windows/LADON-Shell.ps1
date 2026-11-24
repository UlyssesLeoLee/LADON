# 打开 ladon 试用命令行：本窗口的 PATH 里加入安装目录，并切换到示例工作区。
$dir = $PSScriptRoot
$env:Path = "$dir;$env:Path"
$ws = $null
try {
  $m = Get-Content -LiteralPath (Join-Path $dir 'install-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
  if ($m.sampleDir -and (Test-Path -LiteralPath $m.sampleDir)) { $ws = $m.sampleDir }
} catch {}
if (-not $ws) { $ws = [Environment]::GetFolderPath('MyDocuments') }
Set-Location -LiteralPath $ws
$Host.UI.RawUI.WindowTitle = 'LADON 命令行'
Write-Host ''
Write-Host 'LADON 试用命令行（试用构建，请勿用于重要数据）' -ForegroundColor Cyan
Write-Host ('当前目录：' + $ws)
Write-Host ''
$qs = Join-Path $dir 'QUICKSTART.txt'
if (Test-Path -LiteralPath $qs) { Get-Content -LiteralPath $qs -Encoding UTF8 | ForEach-Object { Write-Host $_ } }
Write-Host ''
Write-Host '输入 ladon --help 查看全部命令。' -ForegroundColor Yellow
Write-Host ''
