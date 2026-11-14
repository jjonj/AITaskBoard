# TaskBoard project instructions

## Build and restart authorization

- **Standing permission:** After making changes to the TaskBoard app, agents are always authorized to rebuild and restart the running TaskBoard desktop application without asking the user for additional approval. This explicitly permits stopping the running TaskBoard process when needed for deployment.
- Use the canonical production rebuild-and-restart script from the repository root: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/Rebuild-TaskBoard.ps1`. Do not directly kill/relaunch TaskBoard for a routine rebuild: the script first shows the user a 30-second countdown and **Postpone** button. If postponed, it exits with code 3 without stopping/building; retry at a later suitable time. A frontend-only `npm run build` does not update the desktop executable.
- Do not defer an otherwise finished TaskBoard change merely because deploying it requires a rebuild/restart. Report whether the updated desktop app was actually restarted or only changed in source.
- This authorization applies to **TaskBoard**, not T3 Code or other running applications. Preserve concurrent agents' source changes and avoid overwriting their work.