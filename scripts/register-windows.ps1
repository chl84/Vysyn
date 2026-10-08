param([string]$Executable = (Join-Path $PSScriptRoot 'vysyn.exe'))
$ErrorActionPreference = 'Stop'
$Executable = (Resolve-Path $Executable).Path
$root = 'HKCU:\Software\Classes\Vysyn.Image'
New-Item -Path "$root\shell\open\command" -Force | Out-Null
Set-Item -Path "$root\shell\open\command" -Value ('"' + $Executable + '" "%1"')
foreach ($extension in 'jpg','jpeg','png','webp','gif','bmp','tif','tiff','heic','heif','avif','svg','ico') {
    $key = "HKCU:\Software\Classes\.$extension\OpenWithProgids"
    New-Item -Path $key -Force | Out-Null
    New-ItemProperty -Path $key -Name 'Vysyn.Image' -PropertyType String -Value '' -Force | Out-Null
}
Write-Output 'Vysyn is available in Open with. Select it in Windows Default Apps to make it the default.'
