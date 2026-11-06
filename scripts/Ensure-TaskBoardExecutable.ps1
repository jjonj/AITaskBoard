# Keeps the user-facing executable tied to Cargo/Tauri's single release output.
# The link follows a new build automatically, including direct cargo/tauri builds.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$target = Join-Path $repo 'src-tauri\target\release\taskboard.exe'
$launch = Join-Path $repo 'TaskBoard.exe'
$relativeTarget = 'src-tauri\target\release\taskboard.exe'  # displayed only

if (-not (Test-Path -LiteralPath $target -PathType Leaf)) {
    throw "Release executable missing: $target. Build via 'npm run rebart'."
}

$existing = Get-Item -LiteralPath $launch -Force -ErrorAction SilentlyContinue
if ($existing -and $existing.LinkType -eq 'SymbolicLink') {
    $linkTarget = [string]@($existing.Target)[0]
    $resolved = if ([System.IO.Path]::IsPathRooted($linkTarget)) {
        [System.IO.Path]::GetFullPath($linkTarget)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $repo $linkTarget))
    }
    if ([string]::Equals($resolved, [System.IO.Path]::GetFullPath($target), [StringComparison]::OrdinalIgnoreCase)) {
        Write-Output "Canonical launcher link OK: $launch -> $relativeTarget"
        exit 0
    }
}

# A previous deployment may have copied the EXE; replace that obsolete copy.
# Windows refuses to remove a running executable, so rebuild/restart first.
if ($existing) {
    try { Remove-Item -LiteralPath $launch -Force -ErrorAction Stop }
    catch { throw "Cannot replace the old root TaskBoard.exe (possibly running). Close TaskBoard or run 'npm run rebart'. $($_.Exception.Message)" }
}
try {
    New-Item -ItemType SymbolicLink -Path $launch -Target $target -ErrorAction Stop | Out-Null
} catch {
    throw "Cannot create canonical TaskBoard.exe symlink; enable Windows Developer Mode or run elevated. $($_.Exception.Message)"
}
Write-Output "Canonical launcher linked: $launch -> $relativeTarget"
