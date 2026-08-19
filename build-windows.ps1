param([string]$Version)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$tauriConfig = Join-Path ([IO.Path]::GetTempPath()) "wotstat-repl-$PID.json"

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

    bun install --frozen-lockfile
    if ($LASTEXITCODE -ne 0) { throw 'Bun dependency install failed' }

    $mod = 'src-tauri/resources/wotstat.repl.mod'
    Remove-Item $mod -Force -ErrorAction SilentlyContinue

    foreach ($test in @('test_exec.py', 'selftest.py', 'itest.py', 'test_complete.py')) {
        & $python "mod/tests/$test"
        if ($LASTEXITCODE -ne 0) { throw "Agent test failed: $test" }
    }

    & $python mod/build.py --version $Version --out $mod
    if ($LASTEXITCODE -ne 0) { throw 'Game mod build failed' }

    cargo test --manifest-path src-tauri/Cargo.toml
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed' }

    bun run lint:fsd
    if ($LASTEXITCODE -ne 0) { throw 'FSD lint failed' }

    @{ version = $Version } | ConvertTo-Json -Compress |
        Set-Content $tauriConfig -Encoding ASCII
    bun run tauri build --config $tauriConfig
    if ($LASTEXITCODE -ne 0) { throw 'Tauri build failed' }
} finally {
    Remove-Item $tauriConfig -Force -ErrorAction SilentlyContinue
    Pop-Location
}
