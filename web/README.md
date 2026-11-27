# TaskBoard Web

Remote browser companion to the native TaskBoard, sharing its real project catalog, Markdown boards and revision-aware phone API. No separate task database.

## Running

From the TaskBoard repo: `npm install` (if dependencies are missing), then `npx vite build web --base ./` and `node web/server.mjs`. The server listens on `127.0.0.1:8795` only; `/api/health` identifies the service. OmniHub Gateway registers it as the private `taskboard` tool, with start-with-Omni enabled. After server changes, restart that tool through the Gateway.

Remote entry: https://taskboard.rancorous.online redirects to https://hub.rancorous.online/taskboard and uses the existing OmniHub login and private tool proxy. The hostname alias is configured in the existing Cloudflare tunnel ingress and `rancorous-deployment-prep/hub-gate.js` outside this repository. Do not expose the loopback origin directly or put the OmniHub API credential in browser code.

## Features and limits

Project switching, Kanban/list layout, filtering, search, task details/editing/creation/movement/archive, queue/backlog controls, unified Inbox, and image/video review decisions. Refreshes from native data. Revision conflicts are rejected by the shared backend. Interactive HTML review artifacts and pin markup remain desktop-only. If an operation's outcome is uncertain, refresh and inspect the task before submitting again.

Verified against the live catalog and boards, including desktop and narrow mobile browser layouts, stale-revision conflict, review media delivery and public HTTPS/login routing. No authenticated WAN browser session or successful production task mutation was exercised during initial deployment.
