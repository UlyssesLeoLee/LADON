# LADON 安装向导（按当前用户安装，不需要管理员权限）
# 交互使用：双击 Setup.cmd。静默安装：加 -Silent。
param(
  [switch]$Silent,
  [string]$PayloadDir = $PSScriptRoot,
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\LADON'),
  [string]$SampleDir = (Join-Path $env:USERPROFILE 'LADON-Sandbox'),
  [string]$StartMenuRoot = (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'),
  [string]$RegistryName = 'LADON',
  [switch]$NoPath,
  [switch]$NoShortcuts,
  [switch]$NoSample,
  [switch]$NoRegister,
  [string]$ScreenshotDir
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
$Version = '0.0.3'
$AppName = 'LADON'

. (Join-Path $PSScriptRoot 'LADON-Common.ps1')

function New-Shortcut([string]$LinkPath, [string]$Target, [string]$Arguments, [string]$WorkDir, [string]$Icon, [string]$Description) {
  $sh = New-Object -ComObject WScript.Shell
  $lnk = $sh.CreateShortcut($LinkPath)
  $lnk.TargetPath = $Target; $lnk.Arguments = $Arguments; $lnk.WorkingDirectory = $WorkDir; $lnk.Description = $Description
  if ($Icon -and (Test-Path -LiteralPath $Icon)) { $lnk.IconLocation = $Icon }
  $lnk.Save()
}

# ---------- 安装主体：返回日志行，失败时抛异常 ----------
function Install-Ladon([hashtable]$Opt, [scriptblock]$Log) {
  $payload = $Opt.PayloadDir; $dir = $Opt.InstallDir
  foreach ($f in 'ladon.exe', 'Uninstall-LADON.ps1', 'LADON-Shell.ps1', 'LADON-Common.ps1') {
    if (-not (Test-Path -LiteralPath (Join-Path $payload $f))) { throw "安装包不完整：缺少 $f" }
  }
  $marker = Join-Path $dir 'install-manifest.json'
  if ((Test-Path -LiteralPath $dir) -and -not (Test-Path -LiteralPath $marker)) {
    if (@(Get-ChildItem -LiteralPath $dir -Force).Count -gt 0) { throw "目标目录非空且不是 LADON 安装目录：$dir`n请换一个空目录。" }
  }
  & $Log "安装目录：$dir"
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $manifest = [ordered]@{
    product = $AppName; version = $Version; installDir = $dir; files = @()
    pathAdded = $false; shortcutDir = $null; shortcuts = @(); sampleDir = $null; sampleCreated = $false
    registryName = $null
  }
  if (Test-Path -LiteralPath $marker) {
    try {
      $prev = Get-Content -LiteralPath $marker -Raw -Encoding UTF8 | ConvertFrom-Json
      $manifest.pathAdded = [bool]$prev.pathAdded
      $manifest.sampleCreated = [bool]$prev.sampleCreated
      if ($prev.sampleDir) { $manifest.sampleDir = [string]$prev.sampleDir }
      if ($prev.shortcutDir) { $manifest.shortcutDir = [string]$prev.shortcutDir }
      $manifest.shortcuts = @($prev.shortcuts | Where-Object { $_ })
      if ($prev.registryName) { $manifest.registryName = [string]$prev.registryName }
      & $Log '检测到已有安装，沿用其清单中的记录'
    } catch { & $Log '提示：旧清单无法读取，按全新安装处理' }
  }
  $files = @()
  $sampleNow = $false
  try {
  $copy = @(
    @('ladon.exe', 'ladon.exe'), @('LADON_Explorer.ico', 'LADON_Explorer.ico'), @('LICENSE', 'LICENSE'),
    @('default.yaml', 'config\default.yaml'), @('LADONignore.example', 'config\examples\.LADONignore'),
    @('README_TRIAL.txt', 'README_TRIAL.txt'), @('QUICKSTART.txt', 'QUICKSTART.txt'),
    @('LADON-Shell.ps1', 'LADON-Shell.ps1'), @('LADON-Common.ps1', 'LADON-Common.ps1'), @('Uninstall-LADON.ps1', 'Uninstall-LADON.ps1'))
  foreach ($c in $copy) {
    $src = Join-Path $payload $c[0]
    if (-not (Test-Path -LiteralPath $src)) { & $Log "  跳过（安装包中没有）：$($c[0])"; continue }
    $dst = Join-Path $dir $c[1]
    New-Item -ItemType Directory -Force -Path (Split-Path $dst) | Out-Null
    Copy-Item -LiteralPath $src -Destination $dst -Force
    $files += $c[1]
    & $Log "  复制 $($c[1])"
  }
  if ($Opt.AddPath) {
    $old = Get-UserPathRaw
    $new = Add-PathEntry $old $dir
    if ($new -ne $old) { Set-UserPathRaw $new; $manifest.pathAdded = $true; & $Log '已把安装目录加入当前用户的 PATH（新打开的终端生效）' }
    else { & $Log 'PATH 中已有安装目录，未修改' }
  }
  $ws = $null
  if ($Opt.CreateSample) {
    $ws = $Opt.SampleDir
    if (Test-Path -LiteralPath $ws) {
      & $Log "示例工作区目录已存在，保持原样：$ws"
    } else {
      New-Item -ItemType Directory -Force -Path $ws | Out-Null
      $manifest.sampleCreated = $true
      $sampleNow = $true
      $req = "# REQ-001 示例要件`r`n`r`n系统必须记录文件的每一次稳定保存。`r`n`r`n# REQ-002 敏感文件`r`n`r`n*.pem 与 *.key 文件不得外传。`r`n"
      [IO.File]::WriteAllText((Join-Path $ws 'requirements.md'), $req, (New-Object Text.UTF8Encoding($false)))
      [IO.File]::WriteAllText((Join-Path $ws 'secret.pem'), "TEST-ONLY-NOT-A-REAL-KEY`r`n", (New-Object Text.UTF8Encoding($false)))
      $qs = Join-Path $dir 'QUICKSTART.txt'
      if (Test-Path -LiteralPath $qs) { Copy-Item -LiteralPath $qs -Destination (Join-Path $ws 'QUICKSTART.txt') -Force }
      & $Log "已创建示例工作区：$ws"
    }
    $manifest.sampleDir = $ws
  }
  $shell = Join-Path $dir 'LADON-Shell.ps1'
  $ico = Join-Path $dir 'LADON_Explorer.ico'
  if ($Opt.CreateShortcuts) {
    $folder = Join-Path $Opt.StartMenuRoot $AppName
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    $manifest.shortcutDir = $folder
    $workDir = if ($ws -and (Test-Path -LiteralPath $ws)) { $ws } else { [Environment]::GetFolderPath('MyDocuments') }
    $l1 = Join-Path $folder 'LADON 命令行.lnk'
    New-Shortcut $l1 "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" "-NoExit -NoLogo -ExecutionPolicy Bypass -File `"$shell`"" $workDir $ico 'LADON 命令行（ladon 已加入本窗口 PATH）'
    $l2 = Join-Path $folder '卸载 LADON.lnk'
    New-Shortcut $l2 "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" "-NoProfile -ExecutionPolicy Bypass -File `"$(Join-Path $dir 'Uninstall-LADON.ps1')`"" $dir $ico '卸载 LADON'
    $manifest.shortcuts = @($l1, $l2)
    & $Log "已创建开始菜单项：$folder"
  }
  if ($Opt.Register) {
    $key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$($Opt.RegistryName)"
    New-Item -Path $key -Force | Out-Null
    $props = @{
      DisplayName = "$AppName（试用构建）"; DisplayVersion = $Version; Publisher = 'LADON'; InstallLocation = $dir
      UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$(Join-Path $dir 'Uninstall-LADON.ps1')`""
      DisplayIcon = $ico; NoModify = 1; NoRepair = 1
    }
    foreach ($p in $props.GetEnumerator()) { New-ItemProperty -Path $key -Name $p.Key -Value $p.Value -Force | Out-Null }
    $manifest.registryName = $Opt.RegistryName
    & $Log '已登记到「设置 → 应用 → 已安装的应用」（可从那里卸载）'
  }
  } finally {
    $manifest.files = @($files) + 'install-manifest.json'
    ($manifest | ConvertTo-Json -Depth 4) | Set-Content -LiteralPath $marker -Encoding UTF8
  }
  # 自检：运行刚装好的程序
  $exe = Join-Path $dir 'ladon.exe'
  $ver = & $exe --version 2>&1 | Out-String
  if ($LASTEXITCODE -ne 0) { throw "安装后自检失败：ladon --version 返回 $LASTEXITCODE`n$ver" }
  & $Log ("自检通过：" + $ver.Trim())
  if ($ws -and $sampleNow) {
    $o1 = & $exe --root $ws init 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0) { $o2 = & $exe --root $ws scan 2>&1 | Out-String; & $Log ('示例工作区已初始化并扫描：' + (($o2.Trim() -split "`n" | Select-Object -First 2) -join ' / ')) }
    else { & $Log ('提示：示例工作区初始化失败，可稍后手动运行 ladon init。' + $o1.Trim()) }
  }
  return $manifest
}

function New-Options {
  return @{
    PayloadDir = $PayloadDir; InstallDir = $InstallDir; SampleDir = $SampleDir; StartMenuRoot = $StartMenuRoot
    RegistryName = $RegistryName; AddPath = (-not $NoPath); CreateShortcuts = (-not $NoShortcuts)
    CreateSample = (-not $NoSample); Register = (-not $NoRegister)
  }
}

# ---------- 静默模式 ----------
if ($Silent) {
  $opt = New-Options
  $m = Install-Ladon $opt { param($t) Write-Host $t }
  Write-Output 'SILENT INSTALL OK'
  exit 0
}

# ---------- 图形向导 ----------
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()
$font = New-Object System.Drawing.Font('Microsoft YaHei UI', 9)

$form = New-Object System.Windows.Forms.Form
$form.Text = "$AppName $Version 安装向导"
$form.ClientSize = New-Object System.Drawing.Size(640, 440)
$form.StartPosition = 'CenterScreen'
$form.FormBorderStyle = 'FixedDialog'; $form.MaximizeBox = $false; $form.MinimizeBox = $true
$form.Font = $font
$icoPath = Join-Path $PayloadDir 'LADON_Explorer.ico'
if (Test-Path -LiteralPath $icoPath) { try { $form.Icon = New-Object System.Drawing.Icon($icoPath) } catch {} }

$header = New-Object System.Windows.Forms.Panel
$header.Dock = 'Top'; $header.Height = 64; $header.BackColor = [System.Drawing.Color]::FromArgb(36, 44, 60)
$title = New-Object System.Windows.Forms.Label
$title.Text = "$AppName"; $title.ForeColor = [System.Drawing.Color]::White
$title.Font = New-Object System.Drawing.Font('Microsoft YaHei UI', 15, [System.Drawing.FontStyle]::Bold)
$title.Location = New-Object System.Drawing.Point(20, 8); $title.AutoSize = $true
$sub = New-Object System.Windows.Forms.Label
$sub.ForeColor = [System.Drawing.Color]::FromArgb(190, 200, 215); $sub.Location = New-Object System.Drawing.Point(22, 40); $sub.AutoSize = $true
$header.Controls.AddRange(@($title, $sub))

$footer = New-Object System.Windows.Forms.Panel
$footer.Dock = 'Bottom'; $footer.Height = 56
$btnBack = New-Object System.Windows.Forms.Button; $btnBack.Text = '< 上一步'; $btnBack.Size = New-Object System.Drawing.Size(90, 30); $btnBack.Location = New-Object System.Drawing.Point(330, 13)
$btnNext = New-Object System.Windows.Forms.Button; $btnNext.Text = '下一步 >'; $btnNext.Size = New-Object System.Drawing.Size(90, 30); $btnNext.Location = New-Object System.Drawing.Point(430, 13)
$btnCancel = New-Object System.Windows.Forms.Button; $btnCancel.Text = '取消'; $btnCancel.Size = New-Object System.Drawing.Size(90, 30); $btnCancel.Location = New-Object System.Drawing.Point(535, 13)
$footer.Controls.AddRange(@($btnBack, $btnNext, $btnCancel))

$host1 = New-Object System.Windows.Forms.Panel; $host1.Dock = 'Fill'
$form.Controls.AddRange(@($host1, $footer, $header))

function New-Page { $p = New-Object System.Windows.Forms.Panel; $p.Dock = 'Fill'; $p.Visible = $false; $p.Padding = New-Object System.Windows.Forms.Padding(24, 16, 24, 8); $host1.Controls.Add($p); return $p }
function New-Label([string]$Text, [int]$X, [int]$Y, [int]$W, [int]$H) { $l = New-Object System.Windows.Forms.Label; $l.Text = $Text; $l.Location = New-Object System.Drawing.Point($X, $Y); $l.Size = New-Object System.Drawing.Size($W, $H); return $l }

# 页 0：欢迎
$p0 = New-Page
$p0.Controls.Add((New-Label ("欢迎使用 $AppName $Version（试用构建）`r`n`r`n" +
  "LADON 是事件驱动的文件历史与治理内核，本安装包提供命令行程序 ladon：记录文件每次稳定保存的版本与事件链、按 .LADONignore 阻止敏感内容外传、支持加密归档与恢复。`r`n`r`n" +
  "· 按当前用户安装，不需要管理员权限，可随时卸载。`r`n" +
  "· 安装完成后会运行一次自检，并可创建示例工作区供你试用。`r`n`r`n" +
  "试用提示：这是试用构建。根 crate 的自动化测试 167 项于 2026-11-24 通过（debug 构建），手工系统测试与用户验收尚未执行。请不要用它管理重要数据，先在示例工作区里试用。") 24 12 590 280))

# 页 1：选项
$p1 = New-Page
$p1.Controls.Add((New-Label '安装位置：' 24 14 200 22))
$tbDir = New-Object System.Windows.Forms.TextBox; $tbDir.Location = New-Object System.Drawing.Point(24, 40); $tbDir.Size = New-Object System.Drawing.Size(480, 26); $tbDir.Text = $InstallDir
$btnBrowse = New-Object System.Windows.Forms.Button; $btnBrowse.Text = '浏览…'; $btnBrowse.Location = New-Object System.Drawing.Point(514, 38); $btnBrowse.Size = New-Object System.Drawing.Size(86, 28)
$cbPath = New-Object System.Windows.Forms.CheckBox; $cbPath.Text = '把安装目录加入当前用户的 PATH（之后在任何终端直接输入 ladon）'; $cbPath.Checked = (-not $NoPath); $cbPath.Location = New-Object System.Drawing.Point(24, 88); $cbPath.Size = New-Object System.Drawing.Size(580, 26)
$cbMenu = New-Object System.Windows.Forms.CheckBox; $cbMenu.Text = '创建开始菜单项（「LADON 命令行」和「卸载」）'; $cbMenu.Checked = (-not $NoShortcuts); $cbMenu.Location = New-Object System.Drawing.Point(24, 120); $cbMenu.Size = New-Object System.Drawing.Size(580, 26)
$cbSample = New-Object System.Windows.Forms.CheckBox; $cbSample.Text = ('创建示例工作区：' + $SampleDir); $cbSample.Checked = (-not $NoSample); $cbSample.Location = New-Object System.Drawing.Point(24, 152); $cbSample.Size = New-Object System.Drawing.Size(580, 26)
$cbReg = New-Object System.Windows.Forms.CheckBox; $cbReg.Text = '登记到「设置 → 应用 → 已安装的应用」（便于卸载）'; $cbReg.Checked = (-not $NoRegister); $cbReg.Location = New-Object System.Drawing.Point(24, 184); $cbReg.Size = New-Object System.Drawing.Size(580, 26)
$note = New-Label '以上操作只写入当前用户的位置（用户目录、用户 PATH、开始菜单、HKCU 注册表），卸载程序会按安装清单把它们逐项还原；示例工作区里的数据不会被自动删除。' 24 226 580 52
$note.ForeColor = [System.Drawing.Color]::FromArgb(90, 96, 110)
$p1.Controls.AddRange(@($tbDir, $btnBrowse, $cbPath, $cbMenu, $cbSample, $cbReg, $note))
$btnBrowse.Add_Click({ $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = '选择安装目录（建议选一个空目录）'; if ($d.ShowDialog() -eq 'OK') { $tbDir.Text = (Join-Path $d.SelectedPath 'LADON') } })

# 页 2：安装
$p2 = New-Page
$p2.Controls.Add((New-Label '正在安装…' 24 12 400 22))
$bar = New-Object System.Windows.Forms.ProgressBar; $bar.Style = 'Marquee'; $bar.MarqueeAnimationSpeed = 30; $bar.Location = New-Object System.Drawing.Point(24, 40); $bar.Size = New-Object System.Drawing.Size(580, 18)
$logBox = New-Object System.Windows.Forms.TextBox; $logBox.Multiline = $true; $logBox.ReadOnly = $true; $logBox.ScrollBars = 'Vertical'; $logBox.Location = New-Object System.Drawing.Point(24, 68); $logBox.Size = New-Object System.Drawing.Size(580, 240)
$logBox.BackColor = [System.Drawing.Color]::White
$p2.Controls.AddRange(@($bar, $logBox))

# 页 3：完成
$p3 = New-Page
$lblDone = New-Label '' 24 14 580 150
$cbOpen = New-Object System.Windows.Forms.CheckBox; $cbOpen.Text = '关闭向导后打开「LADON 命令行」开始试用'; $cbOpen.Checked = $true; $cbOpen.Location = New-Object System.Drawing.Point(24, 176); $cbOpen.Size = New-Object System.Drawing.Size(580, 26)
$p3.Controls.AddRange(@($lblDone, $cbOpen))

$pages = @($p0, $p1, $p2, $p3)
$subs = @('欢迎', '选择安装选项', '安装', '完成')
$script:cur = 0
$script:result = $null
$script:failed = $false
function Show-Page([int]$i) {
  $script:cur = $i
  for ($k = 0; $k -lt $pages.Count; $k++) { $pages[$k].Visible = ($k -eq $i) }
  $sub.Text = ('步骤 {0}/4 · {1}' -f ($i + 1), $subs[$i])
  $btnBack.Enabled = ($i -eq 1)
  $btnNext.Enabled = $true
  switch ($i) {
    0 { $btnNext.Text = '下一步 >'; $btnCancel.Enabled = $true }
    1 { $btnNext.Text = '安装'; $btnCancel.Enabled = $true }
    2 { $btnNext.Enabled = $false; $btnBack.Enabled = $false; $btnCancel.Enabled = $false }
    3 { $btnNext.Text = '完成'; $btnBack.Enabled = $false; $btnCancel.Enabled = $false }
  }
}
function Start-Install {
  Show-Page 2
  $logBox.Clear()
  $opt = @{
    PayloadDir = $PayloadDir; InstallDir = $tbDir.Text.Trim(); SampleDir = $SampleDir; StartMenuRoot = $StartMenuRoot
    RegistryName = $RegistryName; AddPath = $cbPath.Checked; CreateShortcuts = $cbMenu.Checked
    CreateSample = $cbSample.Checked; Register = $cbReg.Checked
  }
  $logger = { param($t) $logBox.AppendText($t + "`r`n"); [System.Windows.Forms.Application]::DoEvents() }
  try {
    $script:result = Install-Ladon $opt $logger
    $bar.Style = 'Continuous'; $bar.Value = 100
    $m = $script:result
    $txt = "安装完成。`r`n`r`n程序位置：$($m.installDir)`r`n"
    if ($m.pathAdded) { $txt += "已加入用户 PATH：新打开的终端里直接输入 ladon 即可。`r`n" }
    if ($m.sampleDir) { $txt += "示例工作区：$($m.sampleDir)（内有 QUICKSTART.txt）`r`n" }
    $txt += "`r`n卸载：开始菜单「卸载 LADON」，或 设置 → 应用 → 已安装的应用。"
    $lblDone.Text = $txt
    $cbOpen.Visible = [bool]$m.shortcutDir -or [bool]$m.sampleDir -or $true
    $script:failed = $false
    Show-Page 3
  } catch {
    $script:failed = $true
    $bar.Style = 'Continuous'; $bar.Value = 0
    $logBox.AppendText("`r`n安装失败：" + $_.Exception.Message + "`r`n")
    $lblDone.Text = "安装没有完成。`r`n`r`n" + $_.Exception.Message + "`r`n`r`n已写入的内容可用安装目录里的卸载程序清理；请修正后重新运行安装向导。"
    $cbOpen.Visible = $false
    Show-Page 3
    $btnNext.Text = '关闭'
  }
}
$btnNext.Add_Click({
  switch ($script:cur) {
    0 { Show-Page 1 }
    1 {
      if (-not $tbDir.Text.Trim()) { [void][System.Windows.Forms.MessageBox]::Show('请填写安装目录。', $AppName); return }
      Start-Install
    }
    3 { $form.Close() }
  }
})
$btnBack.Add_Click({ if ($script:cur -eq 1) { Show-Page 0 } })
$btnCancel.Add_Click({ $form.Close() })
$form.Add_FormClosing({
  param($s, $e)
  if ($script:cur -eq 2) { $e.Cancel = $true }
})

if ($ScreenshotDir) {
  # 仅用于检查界面布局：逐页渲染成图片后退出，不安装任何东西
  New-Item -ItemType Directory -Force -Path $ScreenshotDir | Out-Null
  $form.Show()
  for ($i = 0; $i -lt 4; $i++) {
    Show-Page $i
    if ($i -eq 2) { $logBox.Text = "安装目录：示例`r`n  复制 ladon.exe`r`n自检通过：ladon 0.0.3" }
    if ($i -eq 3) { $lblDone.Text = "安装完成。`r`n`r`n程序位置：示例`r`n已加入用户 PATH。`r`n示例工作区：示例（内有 QUICKSTART.txt）`r`n`r`n卸载：开始菜单「卸载 LADON」。"; $cbOpen.Visible = $true }
    [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 300; [System.Windows.Forms.Application]::DoEvents()
    $bmp = New-Object System.Drawing.Bitmap($form.Width, $form.Height)
    $form.DrawToBitmap($bmp, (New-Object System.Drawing.Rectangle(0, 0, $form.Width, $form.Height)))
    $bmp.Save((Join-Path $ScreenshotDir ("page$i.png")), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
  }
  $form.Close()
  exit 0
}

Show-Page 0
[void]$form.ShowDialog()
if (-not $script:failed -and $script:result -and $cbOpen.Checked) {
  $shellScript = Join-Path $script:result.installDir 'LADON-Shell.ps1'
  Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList @('-NoExit', '-NoLogo', '-ExecutionPolicy', 'Bypass', '-File', "`"$shellScript`"")
}
