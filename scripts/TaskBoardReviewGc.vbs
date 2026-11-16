Option Explicit
Dim shell, fso, exe, command
Set shell = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")
exe = fso.BuildPath(fso.GetParentFolderName(WScript.ScriptFullName), "..\packages\task-cli\target\release\taskboard.exe")
If Not fso.FileExists(exe) Then WScript.Quit 2
command = Chr(34) & exe & Chr(34) & " --json task review gc --all-boards"
' The CLI is a separate executable from the Tauri GUI. WindowStyle 0 hides
' its console for a no-focus scheduled sweep; desktop GUI ignores CLI arguments.
WScript.Quit shell.Run(command, 0, True)
