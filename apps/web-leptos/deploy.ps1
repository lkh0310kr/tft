# Build and deploy the static web shell. The local config is copied into dist
# by Trunk; the API key is removed from the public bundle before upload.
param([switch]$WithApiKey)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

trunk build --release
Copy-Item vercel.json dist\vercel.json -Force
if (-not $WithApiKey) {
    $config = Get-Content dist\config.js -Raw
    $config = $config -replace 'apiKey: "[^"]*"', 'apiKey: ""'
    [IO.File]::WriteAllText((Resolve-Path dist\config.js), $config, (New-Object Text.UTF8Encoding($false)))
}
vercel deploy dist --prod --yes --name tft-web
