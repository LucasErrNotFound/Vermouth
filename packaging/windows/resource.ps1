param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$Publisher,

    [Parameter(Mandatory = $true)]
    [string]$Description,

    [Parameter(Mandatory = $true)]
    [string]$TargetDirectory,

    [string]$RcPath = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function ConvertTo-ResourceString([string]$Value) {
    if ($Value -match '[\x00-\x1f\x7f]') {
        throw 'Windows resource strings must not contain control characters.'
    }
    return $Value.Replace('\', '\\').Replace('"', '\"')
}

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw 'Cargo package.version must contain three numeric parts (for example, 0.1.0).'
}

$parts = @($Version.Split('.') | ForEach-Object { [UInt16]::Parse($_) }) + @(0)
$fixedVersion = $parts -join ','
$stringVersion = $parts -join '.'

if ([string]::IsNullOrWhiteSpace($RcPath)) {
    $compiler = Get-Command 'rc.exe' -CommandType Application -ErrorAction SilentlyContinue
    if ($null -ne $compiler) {
        $RcPath = $compiler.Source
    }
}

if ([string]::IsNullOrWhiteSpace($RcPath)) {
    $sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    if (Test-Path -LiteralPath $sdkRoot -PathType Container) {
        $sdkVersions = Get-ChildItem -LiteralPath $sdkRoot -Directory |
            Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
            Sort-Object { [Version]$_.Name } -Descending

        foreach ($sdkVersion in $sdkVersions) {
            $candidate = Join-Path $sdkVersion.FullName 'x64\rc.exe'
            if (Test-Path -LiteralPath $candidate -PathType Leaf) {
                $RcPath = $candidate
                break
            }
        }
    }
}

if ([string]::IsNullOrWhiteSpace($RcPath) -or
    -not (Test-Path -LiteralPath $RcPath -PathType Leaf)) {
    throw 'Windows SDK rc.exe not found. Pass -RcPath with the absolute path to x64\rc.exe.'
}

$company = ConvertTo-ResourceString $Publisher
$descriptionText = ConvertTo-ResourceString $Description

$source = @"
#pragma code_page(65001)
LANGUAGE 9, 1
1 VERSIONINFO
FILEVERSION $fixedVersion
PRODUCTVERSION $fixedVersion
FILEFLAGSMASK 0x3f
FILEFLAGS 0
FILEOS 0x40004
FILETYPE 1
FILESUBTYPE 0
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "$company\0"
            VALUE "FileDescription", "$descriptionText\0"
            VALUE "FileVersion", "$stringVersion\0"
            VALUE "InternalName", "Vermouth\0"
            VALUE "OriginalFilename", "Vermouth.exe\0"
            VALUE "ProductName", "Vermouth\0"
            VALUE "ProductVersion", "$stringVersion\0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"@

$hasher = [System.Security.Cryptography.SHA256]::Create()
try {
    $hashBytes = $hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($source))
    $digest = [BitConverter]::ToString($hashBytes).Replace('-', '').ToLowerInvariant()
} finally {
    $hasher.Dispose()
}

$resourceDir = Join-Path $TargetDirectory 'vermouth-resources\windows-x64'
New-Item -ItemType Directory -Path $resourceDir -Force | Out-Null

$sourcePath = Join-Path $resourceDir "Vermouth-$digest.rc"
$resourcePath = Join-Path $resourceDir "Vermouth-$digest.res"
[IO.File]::WriteAllText($sourcePath, $source, [Text.UTF8Encoding]::new($false))

& $RcPath /nologo "/fo$resourcePath" $sourcePath | Out-Host
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $resourcePath -PathType Leaf)) {
    throw 'Could not compile Windows VERSIONINFO resource.'
}

Write-Output $resourcePath
