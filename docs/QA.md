# QA matrix: v1.0

One row per edge case from the spec
([Error handling and edge cases](specs/2026-10-09-purl-design.md#error-handling-and-edge-cases-qa-matrix)).
Each row is run by hand on a macOS **release build** (Apple M1 Pro). Windows
rows are covered by the CI smoke only and say "needs real hardware".

**Status: not run yet.** The release build has to include Parakeet (#12), the
platform layer (#5) and Smart cleanup (`feat/smart-cleanup`, no PR yet). The
"Automated cover" column lists the unit tests that already guard each case
and the branch they are on today. A unit test does not count as a pass for
the row: every row still gets a manual run on the release build.

Result values: `pass`, `fail (#issue)`, `known issue (#issue)`,
`needs real hardware`, `not run`, and `partial` (the automated cover runs
green, the manual release-build run is still owed).

**Automated run on main @ `35b751d` (2026-10-10):** `cargo test`: 222 passed,
0 failed (1 ignored: needs the real model). Playwright UI smoke against the
Vite dev server with a mocked backend: 44 of 44 passed. Rows below cite the
tests that bear on them.

| # | Case | Expected | How tested (manual, release build) | Automated cover | Result | Evidence |
|---|---|---|---|---|---|---|
| 1 | No mic / mic permission denied | Onboarding and overlay say so with a link to OS settings; hotkey does nothing harmful | Reset with `tccutil reset Microphone app.purl.desktop`, deny the prompt, press the hotkey | `onboarding.spec.ts` "denied microphone says so and links to Settings" (main) | partial | UI smoke green @ 35b751d |
| 2 | Mic unplugged mid-recording | Recording stops, captured audio is still transcribed, toast explains | Record with a USB/Bluetooth mic, disconnect it partway | none | not run | |
| 3 | Silence / <0.3 s audio | Nothing pasted, overlay shows "didn't catch that" | Tap the hotkey briefly; hold it 3 s in a quiet room | `audio::rms_of_silence_is_zero_and_scales_with_amplitude` (main) | not run | |
| 4 | Very long recording (10 min) | Segmented decode, no OOM, progress visible | Play a 10-minute LibriSpeech concatenation into the mic; watch memory in Activity Monitor | `stt::long_audio_splits_under_limit` (#12) | not run | |
| 5 | Model missing / corrupt / checksum fail | Re-download offered; dictation disabled with reason | Delete `encoder-model.int8.onnx`; then truncate it and remove its `.ok` marker | `download_http::checksum_mismatch_deletes_file` (main), `stt::missing_model_reports_missing` (#12); `screens.spec.ts` "Home banners: … speech model missing …" (main) | partial | download + UI tests green @ 35b751d |
| 6 | Offline during download | Paused with Retry; resume picks up the `.part` | Turn Wi-Fi off mid-download, relaunch, Retry | `download_http::connection_refused_is_offline`, `resume_after_restart_uses_part_file` (main); `onboarding.spec.ts` "download failure shows the reason, then Retry succeeds" (main) | partial | download + UI tests green @ 35b751d |
| 7 | Disk full | Refused before download starts | Fill a small disk image used as app-data dir, start the download | `download_http::refuses_when_disk_too_small`; `onboarding.spec.ts` "disk full disables Retry until the window regains focus" (main) | partial | download + UI tests green @ 35b751d |
| 8 | Sidecar crash / port in use | Restart once with a new port; then Failed with "Copy diagnostics" | `kill -9` llama-server mid-session; occupy its port | `llm::crashed_server_is_restarted_on_next_use`, `failed_start_reports_the_reason` (smart-cleanup) | not run | |
| 9 | LLM slow / hung | Deadline hit → Fast cleanup result pasted | `kill -STOP` llama-server, dictate in Smart mode | none yet | not run | |
| 10 | LLM returns junk / answers the prompt | Guard rejects → Fast cleanup | Dictate "what time is it" in Smart mode | `cleanup::guard_rejects_answer_like_output` and other `guard_rejects_*` tests (smart-cleanup) | not run | |
| 11 | Focused field is a password field (macOS) | No paste-read, no correction learning | Dictate into a Safari password field, then edit it | `corrections::start_refuses_a_secure_field`, `abandons_when_the_field_becomes_secure`, `a_secure_field_is_never_watched` (main) | partial | unit tests green @ 35b751d |
| 12 | No focused text field | Text left on clipboard, toast says so | Dictate with Finder's desktop focused | `inject::failed_paste_leaves_the_text_on_the_clipboard` (#5) | not run | |
| 13 | Hotkey conflict | Recorder shows the conflict, keeps the old binding | Bind a shortcut another app owns (e.g. Cmd+Space) | none | not run | |
| 14 | Two quick hotkey presses / press during processing | Debounced; one session at a time | Double-press; press again while the overlay is processing | `second_session_rejected_while_processing` (#5) | not run | |
| 15 | Multi-monitor, mixed DPI | Overlay on the cursor's monitor, correct scale | Retina laptop + external non-Retina display; dictate on each | none | not run | |
| 16 | App quit while sidecars run | Sidecars terminated; no orphans on next launch | Quit from the tray mid-Smart-cleanup; `pgrep llama-server` | none yet | not run | |
| 17 | Non-ASCII / accented speech | Round-trips through clipboard intact | Dictate "café, naïve, São Paulo" into TextEdit and Gmail | `snippets::non_ascii_lowercase_length_change_is_safe` (main) | not run | |
| 18 | Elevated target window (Windows UIPI) | Paste fails gracefully to clipboard + toast | CI smoke only | none | needs real hardware | |
