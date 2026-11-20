# LADON 安装/卸载共用函数（PATH 处理等）。由 Install-LADON.ps1 与 Uninstall-LADON.ps1 点引用。
# ---------- 纯函数：PATH 条目处理（不触碰系统，便于测试） ----------
function Add-PathEntry([string]$Path, [string]$Entry) {
  $parts = @($Path -split ';' | Where-Object { $_ })
  if (@($parts | Where-Object { $_.TrimEnd('\') -ieq $Entry.TrimEnd('\') }).Count -gt 0) { return ($parts -join ';') }
  return (($parts + $Entry) -join ';')
}
function Remove-PathEntry([string]$Path, [string]$Entry) {
  return ((@($Path -split ';' | Where-Object { $_ -and ($_.TrimEnd('\') -ine $Entry.TrimEnd('\')) })) -join ';')
}

# ---------- 与系统交互的小函数 ----------
function Get-UserPathRaw {
  $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $false)
  if (-not $k) { return '' }
  try { return [string]$k.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } finally { $k.Close() }
}
function Set-UserPathRaw([string]$Value) {
  $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
  try { $k.SetValue('Path', $Value, [Microsoft.Win32.RegistryValueKind]::ExpandString) } finally { $k.Close() }
  if (-not ('LadonNative' -as [type])) {
    Add-Type -Namespace '' -Name 'LadonNative' -MemberDefinition '[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true, CharSet=System.Runtime.InteropServices.CharSet.Auto)] public static extern System.IntPtr SendMessageTimeout(System.IntPtr h, uint m, System.UIntPtr w, string l, uint f, uint t, out System.UIntPtr r);' | Out-Null
  }
  $r = [UIntPtr]::Zero
  [void][LadonNative]::SendMessageTimeout([IntPtr]0xFFFF, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 3000, [ref]$r)
}
