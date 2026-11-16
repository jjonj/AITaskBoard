param([switch]$Uninstall)
$ErrorActionPreference = 'Stop'
$name = 'TaskBoard Review GC'
if ($Uninstall) {
    Unregister-ScheduledTask -TaskName $name -Confirm:$false -ErrorAction SilentlyContinue
    Write-Output 'Removed TaskBoard Review GC scheduled task.'
    exit 0
}
$runner = Join-Path $PSScriptRoot 'TaskBoardReviewGc.vbs'
if (!(Test-Path -LiteralPath $runner)) { throw "Missing GC runner: $runner" }
$action = New-ScheduledTaskAction -Execute (Join-Path $env:SystemRoot 'System32\wscript.exe') -Argument ('//B //Nologo "' + $runner + '"')
$triggers = @(
    (New-ScheduledTaskTrigger -Daily -At '03:30'),
    (New-ScheduledTaskTrigger -AtLogOn -User ([System.Security.Principal.WindowsIdentity]::GetCurrent().Name))
)
$settings = New-ScheduledTaskSettingsSet -Hidden -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Hours 1)
$principal = New-ScheduledTaskPrincipal -UserId ([System.Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited
Register-ScheduledTask -TaskName $name -Action $action -Trigger $triggers -Settings $settings -Principal $principal -Description 'Hidden, lock-protected review-media Recycle Bin maintenance. Runs at logon and daily.' -Force | Out-Null
$task = Get-ScheduledTask -TaskName $name
Write-Output ('Installed ' + $name + ' [hidden=' + $task.Settings.Hidden + ', triggers=' + $task.Triggers.Count + ']')
