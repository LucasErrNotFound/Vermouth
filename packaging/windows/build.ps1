param(
    [string]$IsccPath = '',
    [string]$RcPath = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'Run Vermouth Windows packaging on Windows with the MSVC Rust toolchain.'
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$target = 'x86_64-pc-windows-msvc'
$requiredFiles = @(
    'Cargo.toml',
    'Cargo.lock',
    'build.rs',
    'assets\icons\vermouth.ico',
    'packaging\windows\resource.ps1',
    'packaging\windows\Vermouth.iss'
)

foreach ($relativePath in $requiredFiles) {
    if (-not (Test-Path -LiteralPath (Join-Path $repoRoot $relativePath) -PathType Leaf)) {
        throw "Missing required repository file: $relativePath"
    }
}

if ([string]::IsNullOrWhiteSpace($IsccPath)) {
    $compiler = Get-Command 'ISCC.exe' -CommandType Application -ErrorAction SilentlyContinue
    if ($null -ne $compiler) {
        $IsccPath = $compiler.Source
    }
}

if ([string]::IsNullOrWhiteSpace($IsccPath)) {
    $roots = @($env:ProgramFiles, ${env:ProgramFiles(x86)}, (Join-Path $env:LOCALAPPDATA 'Programs')) |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
        Select-Object -Unique

    foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root -PathType Container)) {
            continue
        }
        $installations = Get-ChildItem -LiteralPath $root -Directory -Filter 'Inno Setup*' -ErrorAction SilentlyContinue |
            Sort-Object Name -Descending
        foreach ($install in $installations) {
            $candidate = Join-Path $install.FullName 'ISCC.exe'
            if (Test-Path -LiteralPath $candidate -PathType Leaf) {
                $IsccPath = $candidate
                break
            }
        }
        if (-not [string]::IsNullOrWhiteSpace($IsccPath)) {
            break
        }
    }
}

if ([string]::IsNullOrWhiteSpace($IsccPath) -or
    -not (Test-Path -LiteralPath $IsccPath -PathType Leaf)) {
    throw 'Inno Setup 6.3+ ISCC.exe was not found. Install Inno Setup, or pass -IsccPath.'
}

Push-Location $repoRoot
try {
    $metadataJson = & cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw 'cargo metadata failed. Check Cargo.toml and Cargo.lock.'
    }

    $metadata = ($metadataJson -join "`n") | ConvertFrom-Json
    $package = @($metadata.packages | Where-Object { $_.name -eq 'vermouth' }) |
        Select-Object -First 1
    if ($null -eq $package) {
        throw 'Expected a Cargo package named "vermouth" (repository folder: vermouth).'
    }

    $version = [string]$package.version
    if ($version -notmatch '^\d+\.\d+\.\d+$') {
        throw 'Cargo package version must be three numeric components, e.g. 0.1.0.'
    }

    $authors = @($package.authors | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
    $publisher = if ($authors.Count -gt 0) {
        [string]$authors[0]
    } else {
        Write-Warning 'package.authors is empty; using "Vermouth" as placeholder publisher metadata.'
        'Vermouth'
    }
    if ([string]::IsNullOrWhiteSpace($publisher) -or $publisher -match '["\x00-\x1f\x7f]') {
        throw 'package.authors[0] must be a nonempty publisher without quotes/control characters.'
    }
	
	$description = 'Vermouth'

    $repoUrl = [string]$package.repository
    if ($repoUrl -match '["\x00-\x1f\x7f]') {
        throw 'package.repository must not contain quotes/control characters.'
    }

    $binTargets = @($package.targets | Where-Object { $_.kind -contains 'bin' })
    if ($binTargets.Count -ne 1) {
        throw 'Expected exactly one Cargo binary target for Vermouth.'
    }
    $binaryTarget = [string]$binTargets[0].name

    $resourceArgs = @{
        Version = $version
        Publisher = $publisher
        Description = $description
        TargetDirectory = [string]$metadata.target_directory
        RcPath = $RcPath
    }
    $resourcePath = & (Join-Path $PSScriptRoot 'resource.ps1') @resourceArgs

    $buildArgs = @(
        'rustc', '--locked', '--release', '--target', $target,
        '--bin', $binaryTarget, '--', '-C', "link-arg=$resourcePath"
    )
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) {
        throw 'Vermouth release build failed.'
    }

    $builtBinary = Join-Path $metadata.target_directory "$target\release\$binaryTarget.exe"
    if (-not (Test-Path -LiteralPath $builtBinary -PathType Leaf)) {
        throw "Release executable not found: $builtBinary"
    }

    $stageDir = Join-Path $metadata.target_directory "vermouth-packaging\windows-x64\$version"
    New-Item -ItemType Directory -Path $stageDir -Force | Out-Null
    $stagedBinary = Join-Path $stageDir 'Vermouth.exe'
    Copy-Item -LiteralPath $builtBinary -Destination $stagedBinary -Force

    $readmePath = Join-Path $repoRoot 'README.md'
    $licensePath = Join-Path $repoRoot 'LICENSE'
    $portableFiles = @(@{ Source = $stagedBinary; Name = 'Vermouth.exe' })

    $defines = @(
        "/DAppVersion=$version",
        "/DAppPublisher=$publisher",
        "/DAppBinary=$stagedBinary",
        "/DAppRoot=$repoRoot"
    )
    if (-not [string]::IsNullOrWhiteSpace($repoUrl)) {
        $defines += "/DAppRepositoryUrl=$($repoUrl.TrimEnd('/'))"
    }
    if (Test-Path -LiteralPath $readmePath -PathType Leaf) {
        $defines += "/DAppReadme=$readmePath"
        $portableFiles += @{ Source = $readmePath; Name = 'README.md' }
    } else {
        Write-Warning 'README.md not found; neither package will contain a README.'
    }
    if (Test-Path -LiteralPath $licensePath -PathType Leaf) {
        $defines += "/DAppLicense=$licensePath"
        $portableFiles += @{ Source = $licensePath; Name = 'LICENSE' }
    } else {
        Write-Warning 'LICENSE not found. Add the appropriate license before public distribution.'
    }

    $outputDir = Join-Path $repoRoot "dist\windows\$version"
    New-Item -ItemType Directory -Path $outputDir -Force | Out-Null

    $issPath = Join-Path $PSScriptRoot 'Vermouth.iss'
    & $IsccPath @defines "/O$outputDir" $issPath
    if ($LASTEXITCODE -ne 0) {
        throw 'Inno Setup failed to compile Vermouth.iss.'
    }

    $installerPath = Join-Path $outputDir "Vermouth-$version-windows-x64-Setup.exe"
    if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
        throw "Inno Setup did not produce expected installer: $installerPath"
    }

    $portablePath = Join-Path $outputDir "Vermouth-$version-windows-x64-Portable.zip"
    $tempZip = Join-Path $outputDir "$([Guid]::NewGuid().ToString('N')).zip"
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem

    try {
        $zip = [IO.Compression.ZipFile]::Open($tempZip, [IO.Compression.ZipArchiveMode]::Create)
        try {
            foreach ($file in $portableFiles) {
                [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                    $zip,
                    [string]$file.Source,
                    [string]$file.Name,
                    [IO.Compression.CompressionLevel]::Optimal
                ) | Out-Null
            }
        } finally {
            $zip.Dispose()
        }
        Move-Item -LiteralPath $tempZip -Destination $portablePath -Force
    } finally {
        if (Test-Path -LiteralPath $tempZip -PathType Leaf) {
            Remove-Item -LiteralPath $tempZip -Force
        }
    }

    $checksumPath = Join-Path $outputDir 'SHA256SUMS.txt'
    $lines = foreach ($file in @($installerPath, $portablePath)) {
        $hash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $([IO.Path]::GetFileName($file))"
    }
    [IO.File]::WriteAllText(
        $checksumPath,
        ($lines -join "`n") + "`n",
        [Text.UTF8Encoding]::new($false)
    )

    Write-Host "Release packaging completed for Vermouth $version"
    Write-Host "Installer : $installerPath"
    Write-Host "Portable  : $portablePath"
    Write-Host "Checksums : $checksumPath"
} finally {
    Pop-Location
}
