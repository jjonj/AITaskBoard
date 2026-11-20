# Video evidence for TaskBoard Review Inbox (MISC-0157)

Cross-project contract, 2026-11-15. The existing task ID owns the review, evidence and verdict. Do not make a duplicate task.

## Choosing an evidence type

- **Video:** motion, animations, real-time gameplay and interactions, timing and transitions. Submit a real 5–20 second clip with an extracted poster and source run/seed. It proves only visible motion, not a gameplay/test result.
- **Image:** static pose, appearance, HUD, before-and-after or detail comparisons.
- **Interactive HTML/URL:** direct manipulation is necessary. Preserve generic website review for **every** board, not a whole Wartribes Dashboard embed.
- **Nonvisual question:** product direction, choices or requirements; use the TaskBoard decision workflow.
- Mix image and video in a native review packet. Wartribes alone may additionally use its existing specialized task-scoped Dashboard gallery.

The CLI command task review request accepts multiple --artifact "ABSOLUTE_PATH=Label" arguments. **When an owning task has an acceptance criterion starting `VIDEO REQUIRED:`, the core review validator rejects a new approval packet with no actual local MP4/WebM artifact.** This deterministic marker prevents agents from replacing specifically requested motion video with stills or a generic Dashboard URL. Agents must attach the clip directly (even if it also appears in the Dashboard gallery); only the human may waive/remove the task requirement. Core source: packages/task-core/src/review.rs. Each file <=25 MiB is copied to Reviews/TASK/rN and synced by GDrive; larger videos use **local-only PC cache** with required --poster and are not durable across remote devices. The supported native kinds include mp4/webm, png/jpeg/webp/gif, HTML, and URLs (URL requires poster).

## Source-backed capture comparison

| Existing path | Source | Practical role and limitations |
| --- | --- | --- |
| TaskBoard review | packages/task-core/src/review.rs; src/ReviewView.tsx; src/VisualReviewInspector.tsx | Existing posters, mixed gallery, timestamped video pins and verdicts. Replaces full-video IPC base64 with on-demand HTTP Range playback; images/HTML retained. |
| Generic agent recorder | scripts/review_video.py | Legacy FFmpeg gdigrab start/stop/trim/encode/poster. Window-title capture may freeze D3D rendering when unfocused; desktop region can record unrelated occluding windows. Retain for debugging/encoding, NOT verified gameplay evidence. |
| **Canonical high-quality WGC HWND recorder** | **scripts/review_window_capture.py**, D:/Wartribes/Tools/anim_pie_video_capture.py and D:/Wartribes/Tools/record_tour_video.py | All **new Wartribes WAR, Development Dashboard, and TaskBoard Inbox motion videos** share a single capture backend: native client pixels (up to 1920px), 30fps, direct one-pass H.264 CRF12 MP4 without lossy intermediates. Windows.Graphics.Capture targets the exact HWND/PID and validates real GPU updates, nonblank frames, motion (when required) and optional reference scene. PIE wrapper also temporarily disables background throttling and enforces 100% render resolution. Requires `windows-capture`, cv2/numpy/FFmpeg. Cannot record minimized/closed/headless windows. |
| Wartribes tour | D:/Wartribes/Tools/run_scripted_tour.py; Tools/tourlib/cli.py; Tools/record_tour_video.py | Existing coordinated native tour reports, screenshots and postconditions. Default WGC wrapper binds to the exact tour-owned game HWND, records and validates motion, then produces poster/MP4/source report; `--background-virtual` has no visible HWND. |
| Unreal native render | D:/Wartribes/Source/WarTribes/UI/WTTourRunner.* | Per-step screenshots are real. Movie Render Queue output for interactive tours is **not proven** and is not claimed supported. |
| Wartribes Dashboard | D:/Wartribes/Tools/development_dashboard.py; Tools/publish_development_evidence.py; Mockup/DevelopmentDashboard/index.html; Tools/gateway_web.py | Existing task-scoped image and video publication, now real poster/duration and HTTP byte-range playback for browser Dashboard and native specialized TaskBoard gallery (no whole-Dashboard iframe). |
| OTF browser | D:/SSDProjects/OrderOfTheFallen/Mockup/tools/capture_visual.py; tools/record_review_video.py; src/App.tsx dev bridge | Real Playwright Chromium GPU WebGL video staged by existing seeded scenario, normalized mobile MP4 and poster. Historical dev laboratory, **not** official account/server/PvP. Audio absent. |
| Remote | D:/SSDProjects/Tools/Omnish/main/docs/GATEWAY_REMOTE.md; scripts/gateway-remote-proxy.cjs | Authenticated full-admin HTTPS Gateway tool proxy forwards Range/If-Range, Content-Range/Accept-Ranges and pipes 206 binary without whole-file buffering. Physical cellular Android remains unverified. |

## Verified window recording (preferred for native games / Windows applications)

The generic command is agent-callable and does **not** focus the game, move the mouse or capture the desktop:

    py -B scripts/review_window_capture.py --hwnd <TARGET_HWND> --expected-pid <TARGET_PID> --seconds 12 --output E:/_AgentScratch/Tasks/Wartribes/WT-XXXX/Evidence/game.mp4 --reference D:/Wartribes/Saved/.../known-game-frame.png

Use `--reference` for motion reviews to reject a different window/scene, and read `.validation.json`. The recorder rejects a wrong PID, minimized/closed target, blank/static frames or a clip whose scene disagrees with the reference. The final MP4 is generated **directly** (H.264 CRF12, 30fps, native client resolution), so do not pass it through the legacy `review_video.py encode`/`trim` pipeline before submitting. For WAR animations, use the PIE wrapper to temporarily override low-quality Unreal rendering and background throttling. Native standalone tours use their own high-quality render CLI override. A successful FFmpeg process/MP4 alone is **not acceptance**. Inspect at least three frames throughout the clip and ensure the subject and requested behavior are visible. **Do not publish or submit a failed recording as visual proof.** For synthesized or low-motion UI changes, adapt the semantic check and record real reference evidence; this command is intentionally conservative for motion.

**Ascent / OBS evaluation (2026-11-16):** Installed Ascent `C:/Users/crovea/AppData/Local/Programs/AscentApp/Ascent.exe` bundles OBS but its recorder log uses `duplicator-monitor-capture` / DXGI **monitor** capture, with an observed invalid-monitor fallback to the main screen. That is vulnerable to precisely the wrong-window failure seen in WT-0659. OBS Studio is installed under `E:/Program Files/obs-studio`, including its Windows capture plugin; however, integrating its interactive scene/profile state is unnecessary when a direct WGC HWND API is available. The new WGC path uses the same Windows graphics-capture family without manipulating OBS or Ascent settings. It was actually tested against an occluded moving window and a native Wartribes boar tour.

## Shared FFmpeg commands

Workdir D:/SSDProjects/Tools/TaskBoard. All are **programmatic CLI invocations** (agents do not click Record):

    py -B scripts/review_video.py start --state D:/Wartribes/Saved/ReviewEvidence/clip.state.json --title "WarTribes (64-bit DebugGame PCD3D_SM6) " --raw D:/Wartribes/Saved/ReviewEvidence/clip.mkv --fps 20 --max-seconds 45
    py -B scripts/review_video.py stop --state D:/Wartribes/Saved/ReviewEvidence/clip.state.json
    py -B scripts/review_video.py trim D:/Wartribes/Saved/ReviewEvidence/clip.mkv D:/Wartribes/Saved/ReviewEvidence/segment.mp4 --start 2 --seconds 8
    py -B scripts/review_video.py encode D:/Wartribes/Saved/ReviewEvidence/clip.mkv D:/Wartribes/Saved/ReviewEvidence/clip.mp4 --poster D:/Wartribes/Saved/ReviewEvidence/clip.webp --fps 20 --width 1280 --crf 27
    py -B scripts/review_video.py record --title "WarTribes (64-bit DebugGame PCD3D_SM6) " --seconds 10 --raw D:/Wartribes/Saved/ReviewEvidence/one-shot.mkv

Start runs a hidden FFmpeg process and writes state/log, stop sends q to finalize it, record blocks. Intermediate Matroska is more crash-tolerant than incomplete MP4, but power loss is not guaranteed recoverable. Updated fallback encoder defaults: H.264 AVC/yuv420p, MP4 +faststart, 30fps <=1920 px, CRF 12, AAC if input already has audio. **Do not use this legacy double-encode workflow for new Wartribes game evidence; use canonical direct WGC MP4.** Recommend <=25 MiB synced file, 5–20 seconds, real poster near 0.5 s and caption with run ID and limitations. GDI and Playwright do not capture audio by default; system loopback needs separate consent and verification. GPU/compositor capture may change frame pacing and seek is approximate to keyframes. Inspect final frames for change, not just exit code.


## Wartribes capture/publish example

From D:/Wartribes (normal rendered tour, not --background-virtual):

    py -B Tools/record_tour_video.py --scenario wt0659_boar_animation --report-dir WT0659BoarAnimationTour --start-step walk_frame_01 --grab wgc --seconds 12 --reference Saved/WT0659BoarAnimationTour/20261116T092139Z-894e23/step_05_walk_frame_01.png --output Saved/ReviewEvidence/WT0659-WGC-Capture-Retest.mp4 --task WT-0659 --title "Native boar live gait video" --caption "Native tour; inspect matching validation JSON and run report"

The script discovers **exact Unreal HWND and PID** via tour process ancestry and waits for a named heartbeat, records only that window with WGC, requires a positive video/scene validation, writes the original H.264 CRF12 MP4 directly (no second encode), emits poster/JSON/tour log, then optionally publishes with task/title/caption/sourceReport/capturedAt/duration. Desktop/GDI backends are now **diagnostic-only** and cannot auto-publish evidence. Existing Wartribes Dashboard and native TaskBoard **specialized task-scoped** gallery show videos mixed with screenshots and enlarged playback; they do NOT embed the entire Dashboard.

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

## Review video retention (approved 2026-11-16)

The user approved **automatic cleanup of temporary review video copies 3 full days after a human answers the review**. The existing TaskBoard GC in `packages/task-core/src/review.rs` handles this policy; no separate video scheduler is required. Its scheduled daily/logon and TaskBoard startup sweeps recycle eligible copied MP4/WebM media after 72 hours measured from the round's `resolved_at` time, including rejected and unselected candidates. Videos reused in an unanswered round or selected as an approved/picked/ranked deliverable remain protected, even if the task is archived. Original source footage and external Wartribes Dashboard published media are never deleted by this review GC. Poster files, captions, metadata, verdicts and the original manifest are retained so past Inbox items remain understandable; the remote media server continues to serve retained posters after video expiration. Nonvideo losers retain the previous 48-hour GC behavior, and archived review receipts retain the existing 90-day policy. Recycle Bin semantics allow manual recovery when supported. Do not treat a pruned temporary review video as a failed recording; it is intentionally unavailable after expiration.

### Audio opt-in and verified mobile playback

The generic legacy FFmpeg CLI defaults to silent. When sound is relevant and explicitly appropriate, choose a **named DirectShow audio capture device** in `--audio-device`, for example an intentionally routed `CABLE Output (VB-Audio Virtual Cable)`; this records the named device simultaneously with the legacy desktop/window video and encodes AAC 96kbps. **The verified WGC capture path is silent-only; it rejects `--audio-device` rather than quietly dropping audio.** List available names with FFmpeg `-list_devices true -f dshow -i dummy`, and check that the device carries *game audio rather than microphone or private conversations*. This host exposes a VB-Audio virtual cable; **no audio recording was performed** as part of MISC-0157 and routing it to game output is unverified. Consent and isolation remain the agent's responsibility.

**Remote validation executed:** an isolated scratch-board manifest with two actual captured MP4s (OTF + WT) and one WT PNG was served by the same `scripts/review_media_server.py` implementation. Range GET `bytes=0-1023` returned **206 and 1024 bytes**, suffix `bytes=-128` returned **206 and 128 bytes**, overrun returned **416**, HEAD returned correct 97,081-byte length, unknown artifact/traversal returned **404**. Mobile Chromium emulating Pixel 7/5 loaded both posters and both video players, decoded OTF video duration **3.16s**, played to **0.777s** and sought to **2.1s** without video error. Test script: `E:/_AgentScratch/Tasks/Misc/MISC-0157/Validation/review_mobile_smoke.py`. This is comparable remote-mobile browser playback on local HTTP, not a physical Android device or authenticated cellular round trip.

Live `https://hub.rancorous.online/gateway/tools/taskboard-review-media/health` returns **302 to /__rancorous_login** without session, proving the public route has authentication. The signed-in end-to-end browser and physical phone remain untested; local port 3333 does not expose the same remote tool route (404), so do not claim local Gateway proxy smoke succeeded. The actual phone native ReviewScreen already implements human pick/approve/reject/rank, notes, image attachments and **timecoded video pins** via synced review manifests. This pass adds full-window native playback and a PC-streaming link, but action parity with the newer desktop Review Inbox still has gaps (see OmniSync.Android/docs/TASKBOARD_ANDROID_PARITY_AUDIT_20261106.md). Android Kotlin compilation passed; no ADB device attached for runtime verification.


## Persistent PC media service (remote Android reliability)

The loopback media server is registered with OmniHub Gateway as the web tool taskboard-review-media and also runs at user login through Windows Scheduled Task **TaskBoard Review Media**. Its installer is scripts/Install-TaskBoardReviewMedia.ps1 (idempotent; -Uninstall removes the task; -NoStart registers without starting). The service launches Python 3.13 pythonw.exe without a console, binds **only 127.0.0.1:8785**, is hidden and restarts after failures. It does not open a public port: the existing signed-in OmniHub full-admin gate proxies the authenticated route. TaskBoard desktop also checks/starts the same service on demand if not running. Verified 2026-11-15: task state Running and /health returned HTTP 200 under pythonw; initial pythonw version exposed a no-stderr HTTP handler failure, fixed by suppressing default HTTP stderr logging. Host PC must remain on.

## Live native packet + Pixel mobile browser proof

A **real original-task** MISC-0157 Review Inbox request is open (round 1) with three synced artifacts: WT-CombatArena-Desktop.mp4 (12-second actual changing gameplay, failed tour at step 7 clearly disclosed), OTF-Review-Duel.mp4 (3-second seeded dev-lab motion) and WT approach screenshot, with real WebP posters. Do not create a duplicate MISC-0157 review. The actual loopback service served the live manifest with HTTP 200 and the first video with HTTP **206** for bytes 1024–4095 (3072 bytes). In Playwright **Pixel 5 mobile Chromium emulation**, that live review page rendered two native videos and one image; first video duration 12 s, playback advanced >0.55 s, and seeking to 1.6 s succeeded without decode error. A separate isolated validation covered suffix range requests, invalid-range HTTP 416, HEAD, missing media 404, traversal 404, mixed media, and OTF video seek. Scripts/records: E:/_AgentScratch/Tasks/Misc/MISC-0157/Validation/live_review_mobile.py and review_mobile_smoke.py. The public authenticated URL redirects an anonymous client to login (HTTP 302); **actual signed-in cellular Android playback has not been verified**. The signed-in Gateway forwards Range and binary 206 by source; do not claim the physical phone was tested.

## Opt-in audio

Agent capture defaults **silent**. For a verified audio device (e.g. DirectShow VB-Audio loopback cable where game audio was deliberately routed), the generic legacy scripts/review_video.py start/record commands accept --audio-device "Exact DirectShow capture device name". **Wartribes verified `--grab wgc` does not support audio recording and rejects this flag**, preventing a misleading silent video. Use the FFmpeg dshow device listing first, verify the chosen source is **system/game audio and not a microphone**, and check the final encoded MP4 actually contains permitted AAC sound. OTF Playwright native context video remains silent (no browser audio track); choose a separate explicit desktop recording workflow when sound itself is the question.


### Authenticated-route transport equivalence check (2026-11-15)

Ran the **unchanged production** OmniHub remote proxy module scripts/gateway-remote-proxy.cjs in an isolated loopback harness against the **live** registered TaskBoard media server and authoritative Rust Gateway tool catalog. The isolated harness wraps proxy.handleHttp directly and does *not* bypass or modify the deployed public login gate; public anonymous requests still returned HTTP 302 login. Through the real proxy code, GET with Range bytes=4096-8191 returned HTTP **206** Content-Range bytes 4096-8191/262195 with exactly 4096 bytes. Pixel 5 Chromium mobile emulation opened the actual MISC-0157 2-video/1-image review page **through this proxy**, decoded the 12-second Wartribes clip, played to >0.62 s and sought to 2.5 s without page errors. Reproduction harness: E:/_AgentScratch/Tasks/Misc/MISC-0157/Validation/proxy_lab.cjs and gateway_proxy_pixel_smoke.py. This validates proxy+mobile-browser transport using the production implementation, **not** the physical Android device, the real Cloudflare connection with signed cookies, or the APK release.


### Wartribes Dashboard runtime range check

Ran D:/Wartribes/Tools/gateway_web.py on an isolated temporary loopback port 18768 (did not restart the user's active WT web tool), tested published WT-0017 video hash 00a66f73e33d938a8cd4a64b177f5c2c3f500c366ab91c6698f6d84f5ba37f5a.mp4: GET Range bytes=0-4095 returned HTTP **206**, Content-Range bytes 0-4095/262195 and exactly 4096 bytes; Dashboard HTML returned HTTP 200. Temporary server was shut down after validation. This confirms the source-backed Dashboard media transport, not subjective user approval of appearance or physical Android playback.


### Recorder crash recovery

Generic scripts/review_video.py start/stop now checks the worker PID before waiting or rejecting a new session. If a prior worker died, it refuses to overwrite an **actively recording orphaned FFmpeg process**, but once both processes have exited it atomically preserves the stale .state.json as .recovered-TIMESTAMP.json and any nonempty raw Matroska as .recovered-TIMESTAMP.mkv before starting a new recording; no manual state deletion is required. Stop reports a crashed worker promptly rather than waiting for a result that will never arrive. Validation: E:/_AgentScratch/Tasks/Misc/MISC-0157/Validation/stale_capture_recovery.py used simulated crashed PIDs and interrupted raw bytes, verified both backups; real crash injection was **not** performed. PID validity check uses native Windows process handles without an optional Python dependency.


Additional recording-session regression check: a previous `done`/`failed` state is archived before starting the next worker, so a reused --state path cannot falsely report the old run as the new capture. Focused simulated completed-state restart in stale_capture_recovery.py passed; actual process restart without FFmpeg window playback was not claimed.

## WGC regression verification (2026-11-16, WT-0659)

- **Positive occlusion case:** Created a synthetic, moving 760×450 Tk window on the second monitor and deliberately covered it with an unrelated static red window. `review_window_capture.py --hwnd <target> --expected-pid <owner> --seconds 5` produced an actual 960×540 20fps / 100-frame MKV with true moving yellow object, motion score 0.0917. Frame at 2.5s contained <0.6% red pixels (the occluding window did not contaminate the capture).
- **Negative scene-match case:** Recorded the synthetic target while supplying an actual WT-0659 screenshot via `--reference`. The recorder returned **exit 1**, `.validation.json` had `verified: false`, `reference_score: 0.0151` (<0.22), and no publication occurred. A mismatched PID also raises a hard error before recording.
- **Real production-boundary case:** Ran `py -B Tools/record_tour_video.py --scenario wt0659_boar_animation --report-dir WT0659BoarAnimationTour --start-step walk_frame_01 --seconds 12 --grab wgc --reference Saved/WT0659BoarAnimationTour/20261116T092139Z-894e23/step_05_walk_frame_01.png --output Saved/ReviewEvidence/WT0659-WGC-Capture-Retest.mp4`. Native DebugGame tour run **20261116T215243Z-e03d31 passed 24/24**. Capture matched the exact Unreal PID/HWND, recorded **240 frames at 20 fps**, 12.0s, nonblank scene (luma 86.22, std 78.29), changing frames (motion score **0.3656**, 21/21 sampled intervals changed), reference similarity **0.9591**. Encoded clip 409,320 bytes, plus a real WebP poster. Examined original poster and six spaced frames: live boar clearly visible; no unrelated windows in clip. Video published to WT-0659 Dashboard, id `wt-0659-a0d00130b11d0865e7207c19`, source run linked. This proves the recording pipeline and shows moving boar in a real native game, **not** a hit/death scene or precise hoof IK.
- **Scope:** No OBS/Ascent settings changed. Their installed binaries/logs were examined: Ascent logs use OBS DXGI **monitor** capture and a monitor fallback, making that workflow unsafe for independently launched unfocused gameplay. OBS Studio itself is installed; WGC directly is simpler, per-HWND and robust against covering windows. Source tests: `E:/_AgentScratch/CaptureTools/smoke_window.py`, `synthetic_occluded_wgc.mkv`, `synthetic_wrong_scene.validation.json` and the final Wartribes source clip/report. **Do not mark motion review complete without playable media or an explicit human waiver.**
