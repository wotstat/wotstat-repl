param([string]$Version)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$tauriConfig = Join-Path ([IO.Path]::GetTempPath()) "fuflo-wot-repl-$PID.json"

Push-Location $root
try {
    $packageVersion = (Get-Content package.json | ConvertFrom-Json).version
    if ($Version) {
        $Version = $Version -replace '^v', ''
    } else {
        $Version = $packageVersion
    }
    if ($Version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') {
        throw "Invalid version: $Version"
    }

    $python = 'C:\Python27\python.exe'
    if (-not (Test-Path $python)) {
        throw "Python 2.7 not found at $python"
    }

    if (-not (Test-Path 'node_modules/.bin/steiger.cmd') -or
        -not (Test-Path 'node_modules/.bin/tauri.cmd')) {
        npm ci
        if ($LASTEXITCODE -ne 0) { throw 'npm dependency install failed' }
    }

    $mod = 'src-tauri/resources/me.fuflo.wotrepl.mod'
    Remove-Item $mod -Force -ErrorAction SilentlyContinue

    foreach ($test in @('test_exec.py', 'selftest.py', 'itest.py', 'test_complete.py', 'test_dump.py')) {
        & $python "mod/tests/$test"
        if ($LASTEXITCODE -ne 0) { throw "Agent test failed: $test" }
    }

    & $python mod/build.py --version $Version --out $mod
    if ($LASTEXITCODE -ne 0) { throw 'Game mod build failed' }

    cargo test --manifest-path src-tauri/Cargo.toml
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed' }

    npm run lint:fsd
    if ($LASTEXITCODE -ne 0) { throw 'FSD lint failed' }

    @{ version = $Version } | ConvertTo-Json -Compress |
        Set-Content $tauriConfig -Encoding ASCII
    npm run tauri build -- --config $tauriConfig
    if ($LASTEXITCODE -ne 0) { throw 'Tauri build failed' }
} finally {
    Remove-Item $tauriConfig -Force -ErrorAction SilentlyContinue
    Pop-Location
}
