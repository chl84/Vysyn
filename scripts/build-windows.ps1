param([switch]$SkipChecks)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$projectRoot = (Get-Location).Path
$vcpkgRoot = Join-Path $projectRoot '.native/vcpkg'
function Check-Exit([string]$Task) {
    if ($LASTEXITCODE -ne 0) { throw "$Task failed with exit code $LASTEXITCODE" }
}
if (!(Test-Path $vcpkgRoot)) {
    git clone https://github.com/microsoft/vcpkg.git $vcpkgRoot
    Check-Exit 'Clone vcpkg'
}
git -C $vcpkgRoot checkout ed934a65b65acefbb7a03181891edb00c86f8119
Check-Exit 'Pin vcpkg'
& (Join-Path $vcpkgRoot 'bootstrap-vcpkg.bat') -disableMetrics
Check-Exit 'Bootstrap vcpkg'
& (Join-Path $vcpkgRoot 'vcpkg.exe') install --triplet x64-windows "--x-install-root=$vcpkgRoot/installed"
Check-Exit 'Install native codecs'
$env:VCPKG_ROOT = $vcpkgRoot
$env:VCPKGRS_TRIPLET = 'x64-windows'
$env:VCPKGRS_DYNAMIC = '1'
$codecBin = Join-Path $vcpkgRoot 'installed/x64-windows/bin'
$env:PATH = "$codecBin;$env:PATH"
if (!$SkipChecks) {
    cargo fmt --all --check
    Check-Exit 'Formatting'
    cargo clippy --locked --all-targets -- -D warnings
    Check-Exit 'Clippy'
    cargo test --locked --all-targets
    Check-Exit 'Tests'
}
cargo build --locked --release --bins
Check-Exit 'Release build'
$bundle = Join-Path $projectRoot 'artifacts/vysyn-windows-x64'
New-Item -ItemType Directory -Force $bundle | Out-Null
Copy-Item target/release/vysyn.exe $bundle
Copy-Item target/release/vysyn-bench.exe $bundle
Copy-Item "$codecBin/*.dll" $bundle
Copy-Item README.md $bundle
Copy-Item -Recurse docs $bundle -Force
$licenses = Join-Path $bundle 'third-party-licenses'
New-Item -ItemType Directory -Force $licenses | Out-Null
Copy-Item vendor/winit/LICENSE (Join-Path $licenses 'winit-LICENSE')
Copy-Item vendor/winit/VYSYN_PATCH.md (Join-Path $licenses 'winit-VYSYN_PATCH.md')
Get-ChildItem (Join-Path $vcpkgRoot 'installed/x64-windows/share') -Directory | ForEach-Object {
    $copyright = Join-Path $_.FullName 'copyright'
    if (Test-Path $copyright) { Copy-Item $copyright (Join-Path $licenses ($_.Name + '.txt')) }
}
Copy-Item scripts/register-windows.ps1 $bundle
Compress-Archive -Path "$bundle/*" -DestinationPath artifacts/vysyn-windows-x64.zip -Force
Write-Output "Built $bundle. Run vysyn.exe beside its bundled DLLs."
