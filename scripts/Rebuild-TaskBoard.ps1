param([switch]$NoStart)
# Canonical production build/restart. Other release build routes update the
# same binary automatically because root TaskBoard.exe is a symbolic link.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$launcher = Join-Path $repo 'TaskBoard.exe'
$release = Join-Path $repo 'src-tauri\target\release\taskboard.exe'
$tauri = Join-Path $repo 'node_modules\.bin\tauri.cmd'
if (-not (Test-Path -LiteralPath $tauri)) {
    throw "Install dependencies first with npm install: $tauri"
}

$wasRunning = $false
foreach ($proc in @(Get-Process -Name taskboard -ErrorAction SilentlyContinue)) {
    try {
        if ($proc.Path -and ($proc.Path -ieq $launcher -or $proc.Path -ieq $release)) {
            $wasRunning = $true
            Stop-Process -Id $proc.Id -Force -ErrorAction Stop
            Write-Output "Stopped TaskBoard PID $($proc.Id)"
        }
    } catch {
        throw "Unable to stop TaskBoard PID $($proc.Id): $($_.Exception.Message)"
    }
}

Push-Location $repo
try {
    # Tauri builds the embedded production frontend AND the native executable.
    # A plain 'npm run build' creates only the frontend, not a new desktop EXE.
    & $tauri build --no-bundle
    if ($LASTEXITCODE -ne 0) { throw "Tauri production build failed (exit $LASTEXITCODE)" }
    # OMCPT uses debug CLI and T3 open-task uses release CLI. Both must read
    # the same task-core schema as the desktop app (including new attention flags).
    $cliManifest = Join-Path $repo 'packages\task-cli\Cargo.toml'
    & cargo build --quiet --manifest-path $cliManifest
    if ($LASTEXITCODE -ne 0) { throw "TaskBoard debug CLI schema sync failed (exit $LASTEXITCODE)" }
    & cargo build --quiet --release --manifest-path $cliManifest
    if ($LASTEXITCODE -ne 0) { throw "TaskBoard release CLI schema sync failed (exit $LASTEXITCODE)" }
    & (Join-Path $PSScriptRoot 'Ensure-TaskBoardExecutable.ps1')
    if ($LASTEXITCODE -ne 0) { throw "Failed to synchronize TaskBoard launcher" }
    if (-not $NoStart) {
        Start-Process -FilePath $launcher -WorkingDirectory $repo
        Write-Output "Started canonical TaskBoard: $launcher"
    }
} catch {
    # A failed build must not be described as deployed or overwrite a good EXE.
    # If the release artifact is intact, restore the prior app for the user.
    if ($wasRunning -and (Test-Path $launcher -PathType Leaf) -and -not $NoStart) {
        try { Start-Process -FilePath $launcher -WorkingDirectory $repo } catch {}
    }
    throw
} finally {
    Pop-Location
}
