# Video evidence for TaskBoard Review Inbox (MISC-0157)

Cross-project contract, 2026-11-15. The existing task ID owns the review, evidence and verdict. Do not make a duplicate task.

## Choosing an evidence type

- **Video:** motion, animations, real-time gameplay and interactions, timing and transitions. Submit a real 5–20 second clip with an extracted poster and source run/seed. It proves only visible motion, not a gameplay/test result.
- **Image:** static pose, appearance, HUD, before-and-after or detail comparisons.
- **Interactive HTML/URL:** direct manipulation is necessary. Preserve generic website review for **every** board, not a whole Wartribes Dashboard embed.
- **Nonvisual question:** product direction, choices or requirements; use the TaskBoard decision workflow.
- Mix image and video in a native review packet. Wartribes alone may additionally use its existing specialized task-scoped Dashboard gallery.

The CLI command task review request accepts multiple --artifact "ABSOLUTE_PATH=Label" arguments. Core source: packages/task-core/src/review.rs. Each file <=25 MiB is copied to Reviews/TASK/rN and synced by GDrive; larger videos use **local-only PC cache** with required --poster and are not durable across remote devices. The supported native kinds include mp4/webm, png/jpeg/webp/gif, HTML, and URLs (URL requires poster).

## Source-backed capture comparison

| Existing path | Source | Practical role and limitations |
| --- | --- | --- |
| TaskBoard review | packages/task-core/src/review.rs; src/ReviewView.tsx; src/VisualReviewInspector.tsx | Existing posters, mixed gallery, timestamped video pins and verdicts. Replaces full-video IPC base64 with on-demand HTTP Range playback; images/HTML retained. |
| Generic agent recorder | scripts/review_video.py | FFmpeg gdigrab start/stop/one-shot/trim/encode/poster. Window-title capture may freeze D3D rendering when unfocused; compositor/desktop region shows motion but can be occluded and cannot capture minimized/headless windows. |
| Wartribes tour | D:/Wartribes/Tools/run_scripted_tour.py; Tools/tourlib/cli.py; Tools/record_tour_video.py | Existing scripted virtual-input Unreal gameplay report, per-step screenshots and postconditions. Wrapper locates tour-owned game HWND and records compositor client rectangle. --background-virtual uses -RenderOffScreen and has no visible HWND to record. |
| Unreal native render | D:/Wartribes/Source/WarTribes/UI/WTTourRunner.* | Per-step screenshots are real. Movie Render Queue output for interactive tours is **not proven** and is not claimed supported. |
| Wartribes Dashboard | D:/Wartribes/Tools/development_dashboard.py; Tools/publish_development_evidence.py; Mockup/DevelopmentDashboard/index.html; Tools/gateway_web.py | Existing task-scoped image and video publication, now real poster/duration and HTTP byte-range playback for browser Dashboard and native specialized TaskBoard gallery (no whole-Dashboard iframe). |
| OTF browser | D:/SSDProjects/OrderOfTheFallen/Mockup/tools/capture_visual.py; tools/record_review_video.py; src/App.tsx dev bridge | Real Playwright Chromium GPU WebGL video staged by existing seeded scenario, normalized mobile MP4 and poster. Historical dev laboratory, **not** official account/server/PvP. Audio absent. |
| Remote | D:/SSDProjects/Tools/Omnish/main/docs/GATEWAY_REMOTE.md; scripts/gateway-remote-proxy.cjs | Authenticated full-admin HTTPS Gateway tool proxy forwards Range/If-Range, Content-Range/Accept-Ranges and pipes 206 binary without whole-file buffering. Physical cellular Android remains unverified. |

## Shared FFmpeg commands

Workdir D:/SSDProjects/Tools/TaskBoard. All are **programmatic CLI invocations** (agents do not click Record):

    py -B scripts/review_video.py start --state D:/Wartribes/Saved/ReviewEvidence/clip.state.json --title "WarTribes (64-bit DebugGame PCD3D_SM6) " --raw D:/Wartribes/Saved/ReviewEvidence/clip.mkv --fps 20 --max-seconds 45
    py -B scripts/review_video.py stop --state D:/Wartribes/Saved/ReviewEvidence/clip.state.json
    py -B scripts/review_video.py trim D:/Wartribes/Saved/ReviewEvidence/clip.mkv D:/Wartribes/Saved/ReviewEvidence/segment.mp4 --start 2 --seconds 8
    py -B scripts/review_video.py encode D:/Wartribes/Saved/ReviewEvidence/clip.mkv D:/Wartribes/Saved/ReviewEvidence/clip.mp4 --poster D:/Wartribes/Saved/ReviewEvidence/clip.webp --fps 20 --width 1280 --crf 27
    py -B scripts/review_video.py record --title "WarTribes (64-bit DebugGame PCD3D_SM6) " --seconds 10 --raw D:/Wartribes/Saved/ReviewEvidence/one-shot.mkv

Start runs a hidden FFmpeg process and writes state/log, stop sends q to finalize it, record blocks. Intermediate Matroska is more crash-tolerant than incomplete MP4, but power loss is not guaranteed recoverable. Default normalized format: H.264 AVC/yuv420p, MP4 +faststart, 20fps <=1280 px, CRF 27, AAC if input already has audio. Recommend <=25 MiB synced file, 5–20 seconds, real poster near 0.5 s and caption with run ID and limitations. GDI and Playwright do not capture audio by default; system loopback needs separate consent and verification. GPU/compositor capture may change frame pacing and seek is approximate to keyframes. Inspect final frames for change, not just exit code.


## Wartribes capture/publish example

From D:/Wartribes (normal rendered tour, not --background-virtual):

    py -B Tools/record_tour_video.py --scenario combat_arena --report-dir CombatArenaTour --start-step approach --grab desktop --seconds 12 --output Saved/ReviewEvidence/WT-CombatArena-Desktop.mp4 --task WT-0017 --title "Combat arena motion" --caption "Actual rendered tour; see run report for pass/fail"

The script discovers Unreal HWND via tour process ancestry and waits for named heartbeat, records *desktop compositor rectangle*, normalizes through TaskBoard's generic recorder, emits MP4/poster/JSON/tour log, then optionally publishes video with WT task/title/caption/sourceReport/capturedAt/duration. Existing Wartribes Dashboard and native TaskBoard **specialized task-scoped** gallery show videos mixed with screenshots and enlarged playback; they do NOT embed the entire Dashboard.

For an existing clip, publish directly:

    py -B Tools/publish_development_evidence.py Saved/ReviewEvidence/WT-CombatArena-Desktop.mp4 --task WT-0017 --title "Combat motion" --caption "Actual tour, stage 7 PressSwallowed, NOT a full pass" --kind tour --source-report Saved/CombatArenaTour/combat_arena_20261115T024520Z-4a2071.json

For a new native review on an existing WT task (do NOT duplicate an already open review):

    taskboard.exe task review request WT-XXXX --mode approve --judge "Assess captured movement; run was partial" --artifact "D:/Wartribes/Saved/ReviewEvidence/WT-CombatArena-Desktop.mp4=Actual movement" --artifact "D:/Wartribes/Saved/CombatArenaTour/20261115T024520Z-4a2071/step_03_approach.png=Reference frame"

Alternatively use existing WT task-scoped Dashboard URL in --artifact with a representative --poster. Do not assume this works for OTF.

## Order of the Fallen: actual browser recording

From D:/SSDProjects/OrderOfTheFallen/Mockup:

    py -B tools/record_review_video.py --scenario duel --seed 18421 --duration 3 --width 960 --height 540 --output Saved/ReviewEvidence/OTF-Review-Duel.mp4
    taskboard.exe task review request OTF-XXXX --mode approve --judge "Evaluate actual dev-lab duel motion, NOT official-server PvP" --artifact "D:/SSDProjects/OrderOfTheFallen/Mockup/Saved/ReviewEvidence/OTF-Review-Duel.mp4=Three-second seeded duel"

This uses the existing dev bridge and real GPU Chromium context, trims loading, records actual changing frames plus a poster and before/after simulation snapshots. For real server-account UI use generic visible browser capture; the seeded lab tool is not official Play Mode evidence. No Wartribes service or Dashboard dependency. Generic HTML/URL and nonvisual question review also remain available.

## Desktop and Android playback architecture

Desktop Tauri command ensure_review_media_server starts scripts/review_media_server.py on **127.0.0.1:8785**. It resolves only a valid board/task/round/artifact identifier from the canonical review manifest, rejects paths escaping the approved review data/cache roots, bounds file size and supports GET/HEAD + Range with 206 Partial Content, Content-Range/Accept-Ranges, and 416 invalid ranges. React native HTML5 video uses the byte stream, not base64 whole-file IPC. Full-window playback and Escape/back use the existing fullscreen inspector. The specialized WT Dashboard gallery obtains range-streamed published media by content hash.

The same loopback service is registered as the authenticated OmniHub Gateway web tool taskboard-review-media. Native desktop Phone link targets:

    https://hub.rancorous.online/gateway/tools/taskboard-review-media/review/<BOARD>/<TASK-ID>

For example /review/Misc/MISC-0157 . Android HTML5 video gets the actual MP4 via relative /media URLs through the Gateway's signed HTTPS proxy (no raw local Windows file:// URLs). This is a playback-only fallback/page; approvals and time-based pin annotations already exist in Android's TaskBoardReviewScreen.kt and TaskBoardReviewStore.kt via its own synced review manifest. The native app currently uses Android VideoView with synced local files; it does not automatically stream files that have not synced, and official mobile Review Inbox action parity is incomplete. Do not claim mobile UI and desktop TaskBoard are fully equivalent.

Host PC, review vault mount, OmniHub, and signed-in session must be active. Do not bind port 8785 publicly or bypass the login. Mobile H.264 MP4 is safer than WebM/VP9; network latency and keyframes affect seek. Physical phone verification is still unperformed.

## Observed results, 2026-11-15

- OTF developer duel recorded **real 97,081-byte H.264 MP4** (3.16s, 79 frames at 25 fps, 960x540), poster (20,544 bytes) and source JSON with seed 18421; no browser errors. 1s-spaced frames differed by mean 2.77 and 0.93. Not official PvP.
- WT main_menu tour passed **6/6**, four virtual clicks. Direct gdigrab title=window capture from actual HWND had near-static frames. That is NOT motion evidence.
- WT combat_arena compositor recording **12s/262,195 bytes**, real changing frames (mean 1s differences 16.55, 1.56, 2.62, 2.39, 5.44). Its tour **FAILED** 7/8 with PressSwallowed at step seven; report D:/Wartribes/Saved/CombatArenaTour/combat_arena_20261115T024520Z-4a2071.json. Video published under WT-0017 with truthful failure caption, 12s duration and genuine poster. Clip shows motion but NOT a passing tour.
- TaskBoard npm run build and cargo check src-tauri passed after code edits; Python py_compile passed. Actual phone on cellular remains UNVERIFIED.
- Source-backed additional audit E:/_AgentScratch/Tasks/Misc/MISC-0157/Deliverables/research-handoff-20261115.md independently verified old 2017 FFmpeg does **not** support gdigrab hwnd=, and another OTF parry example captured a playable curated MP4. It also identified legacy_rest.rs as an additional existing authenticated 206 streaming option; current implementation instead uses registered Gateway tool proxy for narrow path-scoped access.

## Retention decision pending user confirmation

Existing TaskBoard GC (packages/task-core/src/review.rs, scripts/Install-TaskBoardReviewGc.ps1) already runs on login and daily (also desktop startup/6-hour sweep), prunes losers/superseded after **48h** while preserving selected media, archives receipt up to **90 days**. User proposes **3 days after answered**. Asked for approval to integrate video retention into the existing GC and preserve manifest/poster/decision record before changing cleanup behavior. Do not delete media early or alter other GC policies without confirmation.

### Audio opt-in and verified mobile playback

The generic CLI defaults to silent. When sound is relevant and explicitly appropriate, choose a **named DirectShow audio capture device** in `--audio-device`, for example an intentionally routed `CABLE Output (VB-Audio Virtual Cable)`; this records the named device simultaneously with desktop/window video and encodes AAC 96kbps. List available names with FFmpeg `-list_devices true -f dshow -i dummy`, and check that the device carries *game audio rather than microphone or private conversations*. This host exposes a VB-Audio virtual cable; **no audio recording was performed** as part of MISC-0157 and routing it to game output is unverified. Consent and isolation remain the agent's responsibility.

**Remote validation executed:** an isolated scratch-board manifest with two actual captured MP4s (OTF + WT) and one WT PNG was served by the same `scripts/review_media_server.py` implementation. Range GET `bytes=0-1023` returned **206 and 1024 bytes**, suffix `bytes=-128` returned **206 and 128 bytes**, overrun returned **416**, HEAD returned correct 97,081-byte length, unknown artifact/traversal returned **404**. Mobile Chromium emulating Pixel 7/5 loaded both posters and both video players, decoded OTF video duration **3.16s**, played to **0.777s** and sought to **2.1s** without video error. Test script: `E:/_AgentScratch/Tasks/Misc/MISC-0157/Validation/review_mobile_smoke.py`. This is comparable remote-mobile browser playback on local HTTP, not a physical Android device or authenticated cellular round trip.

Live `https://hub.rancorous.online/gateway/tools/taskboard-review-media/health` returns **302 to /__rancorous_login** without session, proving the public route has authentication. The signed-in end-to-end browser and physical phone remain untested; local port 3333 does not expose the same remote tool route (404), so do not claim local Gateway proxy smoke succeeded. The actual phone native ReviewScreen already implements human pick/approve/reject/rank, notes, image attachments and **timecoded video pins** via synced review manifests. This pass adds full-window native playback and a PC-streaming link, but action parity with the newer desktop Review Inbox still has gaps (see OmniSync.Android/docs/TASKBOARD_ANDROID_PARITY_AUDIT_20261106.md). Android Kotlin compilation passed; no ADB device attached for runtime verification.
