param([switch]$Uninstall, [switch]$NoStart)
$ErrorActionPreference = 'Stop'
$name = 'TaskBoard Review Media'
if ($Uninstall) {
    Unregister-ScheduledTask -TaskName $name -Confirm:$false -ErrorAction SilentlyContinue
    Write-Output "Removed $name logon service."
    exit 0
}
$script = Join-Path $PSScriptRoot 'review_media_server.py'
if (!(Test-Path -LiteralPath $script)) { throw "Missing review media server: $script" }
$pythonw = 'C:\Program Files\Python313\pythonw.exe'
if (!(Test-Path -LiteralPath $pythonw)) { throw "Python 3.13 pythonw.exe not installed: $pythonw" }
$action = New-ScheduledTaskAction -Execute $pythonw -Argument ('-B "' + $script + '" --host 127.0.0.1 --port 8785') -WorkingDirectory (Split-Path $script)
$trigger = New-ScheduledTaskTrigger -AtLogOn -User ([Security.Principal.WindowsIdentity]::GetCurrent().Name)
$settings = New-ScheduledTaskSettingsSet -Hidden -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Seconds 0) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
$principal = New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited
Register-ScheduledTask -TaskName $name -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Description 'Loopback-only TaskBoard Review Inbox media streaming for authenticated OmniHub phone reviews; no public listener.' -Force | Out-Null
if (!$NoStart) { Start-ScheduledTask -TaskName $name }
$task = Get-ScheduledTask -TaskName $name
Write-Output ("Configured $name; hidden=$($task.Settings.Hidden); state=$($task.State); trigger=logon; bind=127.0.0.1:8785")
