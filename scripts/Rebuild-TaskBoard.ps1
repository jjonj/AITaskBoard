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

# Only one agent may rebuild/restart the app at a time.
$mutex = [System.Threading.Mutex]::new($false, 'Local\TaskBoard-Rebuild')
if (-not $mutex.WaitOne(0)) {
    $mutex.Dispose()
    throw 'Another TaskBoard rebuild is already running.'
}
try {
$running = @(Get-Process -Name taskboard -ErrorAction SilentlyContinue | Where-Object {
    $_.Path -and ($_.Path -ieq $launcher -or $_.Path -ieq $release)
})
$wasRunning = $running.Count -gt 0

if ($wasRunning) {
    # The running app polls this request, offers Postpone, then saves editors.
    # Canceling or a missing save acknowledgement prevents the process stop.
    $noticeDir = Join-Path $env:LOCALAPPDATA 'TaskBoard'
    $requestFile = Join-Path $noticeDir 'restart-warning.json'
    $postponeFile = Join-Path $noticeDir 'restart-postponed.json'
    $savedFile = Join-Path $noticeDir 'restart-saved.json'
    New-Item -ItemType Directory -Path $noticeDir -Force | Out-Null
    $requestId = [Guid]::NewGuid().ToString()
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds(30).ToUnixTimeMilliseconds()
    $requestJson = @{ id = $requestId; deadlineUnixMs = $deadline } | ConvertTo-Json -Compress
    $tmpFile = Join-Path $noticeDir "restart-warning.$requestId.tmp"
    $postponed = $false
    try {
        Remove-Item -LiteralPath $postponeFile, $savedFile -Force -ErrorAction SilentlyContinue
        Get-ChildItem -LiteralPath $noticeDir -Filter 'restart-ready-*.json' -File | Remove-Item -Force
        [IO.File]::WriteAllText($tmpFile, $requestJson, [Text.UTF8Encoding]::new($false))
        Move-Item -LiteralPath $tmpFile -Destination $requestFile -Force
        Write-Output 'TaskBoard restart warning active (30s countdown; Postpone cancels this attempt).'
        while ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() -lt $deadline) {
            if (Test-Path -LiteralPath $postponeFile) {
                try {
                    $response = Get-Content -LiteralPath $postponeFile -Raw | ConvertFrom-Json
                    if ($response.id -eq $requestId) { $postponed = $true; break }
                } catch { }
            }
            Start-Sleep -Milliseconds 250
        }
        # Finalize only after every open webview has saved its active editor.
        # The request remains visible while the user can still choose Postpone.
        if (-not $postponed) {
            $finalRequest = @{ id = $requestId; deadlineUnixMs = $deadline; finalize = $true } | ConvertTo-Json -Compress
            [IO.File]::WriteAllText($tmpFile, $finalRequest, [Text.UTF8Encoding]::new($false))
            Move-Item -LiteralPath $tmpFile -Destination $requestFile -Force
            $saveDeadline = [DateTimeOffset]::UtcNow.AddSeconds(20).ToUnixTimeMilliseconds()
            $saved = $false
            while ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() -lt $saveDeadline) {
                if (Test-Path -LiteralPath $postponeFile) {
                    try {
                        $response = Get-Content -LiteralPath $postponeFile -Raw | ConvertFrom-Json
                        if ($response.id -eq $requestId) { $postponed = $true; break }
                    } catch { }
                }
                if (Test-Path -LiteralPath $savedFile) {
                    try {
                        $response = Get-Content -LiteralPath $savedFile -Raw | ConvertFrom-Json
                        if ($response.id -eq $requestId) { $saved = $true; break }
                    } catch { }
                }
                Start-Sleep -Milliseconds 250
            }
            if (-not $saved -and -not $postponed) {
                $postponed = $true
                Write-Warning 'TaskBoard did not confirm every editor was saved. Restart canceled to protect unsaved work.'
            }
        }
        # Give a last-moment Postpone click priority over an editor-save receipt.
        if (Test-Path -LiteralPath $postponeFile) {
            try {
                $response = Get-Content -LiteralPath $postponeFile -Raw | ConvertFrom-Json
                if ($response.id -eq $requestId) { $postponed = $true }
            } catch { }
        }
    } finally {
        Remove-Item -LiteralPath $tmpFile -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $requestFile) {
            try {
                $current = Get-Content -LiteralPath $requestFile -Raw | ConvertFrom-Json
                if ($current.id -eq $requestId) {
                    Remove-Item -LiteralPath $requestFile -Force -ErrorAction SilentlyContinue
                }
            } catch { }
        }
        Remove-Item -LiteralPath $postponeFile, $savedFile -Force -ErrorAction SilentlyContinue
        Get-ChildItem -LiteralPath $noticeDir -Filter 'restart-ready-*.json' -File | Remove-Item -Force
    }
    if ($postponed) {
        Write-Warning 'TaskBoard restart postponed by user. No build or restart was performed; rerun the script later.'
        exit 3
    }
}

# No warning is needed if TaskBoard was already closed.
foreach ($proc in @(Get-Process -Name taskboard -ErrorAction SilentlyContinue)) {
    try {
        if ($proc.Path -and ($proc.Path -ieq $launcher -or $proc.Path -ieq $release)) {
            Stop-Process -Id $proc.Id -Force -ErrorAction Stop
            # The executable stays locked until Windows finishes tearing down
            # the process. Rust's linker must not replace it before that point.
            if (-not $proc.WaitForExit(10000)) {
                throw "Timed out waiting for TaskBoard PID $($proc.Id) to exit"
            }
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
} finally {
    $mutex.ReleaseMutex()
    $mutex.Dispose()
}
