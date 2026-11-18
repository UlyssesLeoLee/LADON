# LADON Explorer ST smoke; creates and modifies only a temporary directory.
$ErrorActionPreference = 'Stop'
$root = Join-Path $env:TEMP ("ladon-st-" + [guid]::NewGuid().ToString('N'))
New-Item -Path $root -ItemType Directory | Out-Null
try {
  cargo run --bin ladon -- --root $root init
  Set-Content -Encoding UTF8 -Path (Join-Path $root 'requirements.md') -Value "# REQ-001`nversion one"
  cargo run --bin ladon -- --root $root scan
  Set-Content -Encoding UTF8 -Path (Join-Path $root 'requirements.md') -Value "# REQ-001`nversion two"
  cargo run --bin ladon -- --root $root scan
  cargo run --bin ladon -- --root $root history requirements.md
  cargo run --bin ladon -- --root $root verify requirements.md
  Set-Content -Encoding UTF8 -Path (Join-Path $root 'secret.pem') -Value 'SECRET'
  cargo run --bin ladon -- --root $root scan
  cargo run --bin ladon -- --root $root ignore-check secret.pem
  Write-Host 'ST smoke commands finished; inspect outputs for expected events.'
} finally {
  # Only remove the temp directory created by this script.
  Remove-Item -LiteralPath $root -Recurse -Force
}
