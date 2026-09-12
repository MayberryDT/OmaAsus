# OmaAsus reliability and product plan

Status: proposed implementation, 12 September 2026. Baseline reviewed: `63c4170`; 79 workspace tests passed. The branding work accompanying this document is implemented locally. The reliability work below is not yet implemented. This plan does not change hardware settings or authorize a release by itself.

## Outcome

A user selects a profile, sees what is being applied, and can distinguish confirmed hardware state from requested settings. A missing sensor cannot silently become zero, block unrelated telemetry, or prevent cooling recovery. The window, tray, and overlay present the same trustworthy state.

Preserve the Rust workspace, capability-based hardware model, Omarchy-inspired design, existing profiles, and daemon integrations. Prioritize correctness over adding more controls. AUR publication and an Omarchy companion plugin are outside this milestone.

## Delivery sequence

| Phase | Deliverable | Depends on | Completion gate |
|---|---|---|---|
| 0 | Failure harness and baseline evidence | None | Deterministic reproductions of critical failures |
| 1 | Helper recovery and bounded device work | 0 | Failed restores remain tracked; unrelated controls remain responsive |
| 2 | Independent telemetry and cooling safety | 0, 1 | A permanently blocked sensor cannot stop safety evaluation |
| 3 | Serialized profile coordination | 0, 1 | Last requested profile wins across every integration |
| 4 | Truthful telemetry and automation | 2, 3 | Missing values are explicit; AMD thermal rules work |
| 5 | Clear operational UI and component boundaries | 3, 4 | Window, tray, and panel agree about actual state |
| 6 | Packaging, documentation, release qualification | 1–5 | Reproducible artifacts and completed hardware matrix |

Use small reviewable changes in this order. Add each regression test before its corresponding fix. Keep branding separate from hardware behavior changes so it can be reviewed and reverted independently.

## Phase 0 — Establish a reproducible failure harness

Scope: `oma-hw`, `oma-helper`, GUI apply coordination and automation tests.

Introduce narrow interfaces for sensor reads, hardware writes, and daemon calls. Production implementations retain existing backends; fakes can succeed, fail, delay, block until released by the test, or record ordering. Use an injectable monotonic clock for deadlines, retries, and hold periods. Do not emulate entire drivers or require root for the default suite.

Record baseline startup time, frame age, visible/hidden CPU usage, and profile completion time on the desktop. Capture hardware capabilities and software versions without secrets. Distinguish measured performance from targets.

Required regression cases:

- A sensor stops returning after initially successful reads.
- Restoring automatic fan control fails before the client disconnects.
- Profile A waits while B is requested; A later completes a daemon operation.
- An obsolete profile reaches CoolerControl activation.
- A GPU temperature rule runs on AMD-only hardware.
- A sensor disappears while its gauge remains visible.

Acceptance: failures can be reproduced without real hardware writes; tests assert externally meaningful results rather than private implementation details.

## Phase 1 — Make privileged recovery dependable

Scope: helper request handling, claims, restore logic, client proxy, service lifecycle.

1. Move synchronous sysfs, HID, and NVML operations off the helper's async dispatcher. Use bounded queues and serialize operations per physical device. Calls for an unrelated device must remain serviceable.
2. Preserve canonical-path validation and polkit classification. Recheck lifecycle state before dispatching authorized work. Track work until the physical operation completes, not merely until a client timeout.
3. Treat claims as durable recovery intent within the helper process: record original state before a change, clear it only after confirmed hand-back. Keep partial NVIDIA fan failures tracked individually where feasible.
4. Define exclusive ownership of an output. A second client must not overwrite another client's recovery state or allow a late restore to undo a newer claim.
5. Retain failed restoration work and retry with bounded backoff. Expose recovery pending/failed in diagnostics. Preserve duty-before-mode restoration ordering.
6. Separate user-visible request deadlines from worker lifetime. An async timeout does not cancel a blocked kernel operation. Do not launch unbounded replacement workers for a stuck device.
7. Evaluate per-backend process isolation if a blocking worker cannot meet shutdown needs. Preserve systemd's shutdown budget and report what could not be restored. Do not promise recovery from uninterruptible kernel I/O or power loss.
8. Assess Lian Li manual control separately: current crash recovery does not cover it. Add an explicit recovery protocol or clearly constrain unattended operation until one exists.

Acceptance: failed automatic-mode writes retain claims; client disappearance initiates recovery; unrelated devices respond during a blocked operation; old-client restores cannot overwrite new ownership; repeated timeouts do not grow thread count indefinitely. No weakening of path or authorization checks.

## Phase 2 — Decouple cooling safety from sensor delivery

Scope: telemetry sampler, snapshot schema, fan scheduler, fan backend.

Replace the sequential whole-machine sampling dependency with bounded per-device workers and an independent aggregator. Every reading carries identity, observation time, last success, and availability. A stuck worker is marked unhealthy and is not replaced repeatedly while still running. Healthy devices continue publishing.

Run fan evaluation on its own monotonic schedule, independent of telemetry frames and animation. Resolve only sufficiently fresh measurements for control. Keep display history distinct from control inputs: a retained last value is not automatically a valid temperature for a curve.

Define behavior for each transition: valid to missing, missing at startup, missing after switching from fixed duty, recovery after prolonged loss, suspend/resume, and device removal. Preserve the existing 30-second missing-source policy initially, but explicitly evaluate whether each backend should use curve maximum, full duty, or firmware hand-back. Document that a custom curve's maximum is not necessarily 100%.

Keep one ordered command stream per fan target. Reject obsolete results and prevent delayed writes from restoring a previous profile's duty. Maintain pump floors through fixed, software-curve, and supported hardware-curve paths.

Acceptance: safety evaluation continues without new frames; healthy sensors retain their cadence; stale sources do not drive curves; each fallback happens within one safety tick of its deadline; recovery ramps correctly; suspend time is handled deliberately; queues remain bounded.

## Phase 3 — Make profile application ordered and observable

Scope: extract profile coordination from `app.rs`; extend `apply.rs` and integration adapters.

Introduce a coordinator with requested profile, in-flight generation, confirmed per-subsystem state, and last result. Keep the user's saved preference separate from the current hardware result.

Coalesce rapid requests to the latest intent. Allow an already-dispatched operation to settle before dispatching conflicting work. Generation checks must cover firmware, CPU, GPU, lighting, CoolerControl, deferred GPU work, and completion messages. Never activate CoolerControl after receiving a superseded report.

Apply in explicit dependency order: select power mode; wait for bounded settling/readback; apply firmware overrides; CPU/GPU settings; program cooling after firmware resets; lighting and external cooling mode as appropriate to ownership. Independent non-conflicting operations may run concurrently.

Read back supported controls and compare normalized values. Label write-only operations as acknowledged but unverified. Report unsupported, pending, failed, and successful steps separately. Provide retry for failed steps and a known-safe profile action. Avoid generic automatic rollback: reversing a graphics switch or firmware change may have additional effects.

Acceptance: after A→B→C, C is the final requested and verified state where supported; late results never claim an old profile is active; deferred GPU work uses the latest intent; partial failures remain visible; repeated applies are idempotent where backends allow it.

## Phase 4 — Unify telemetry meaning and automation

Scope: snapshots, smoothing, gauges, histories, automation decisions.

Use optional typed readings with explicit freshness throughout. Smooth valid numbers only. Show unavailable or stale readings with age; draw chart gaps instead of fabricated zeroes. Keep stable device ordering. Distinguish stopped fan, sleeping GPU, unplugged device, permission failure, and stalled driver.

Select GPU temperature through the shared hardware model and an explicit GPU identity. Support AMD and NVIDIA; decide how multi-GPU rules select their source rather than silently binding to index zero. Missing data must not be interpreted as a cool device.

Add automation tests for priority ties, duration, post-match hold, overnight windows, deleted profiles, missing/stale temperature, AMD-only machines, suspend/resume, and manual override behavior. The stored Idle trigger is currently unimplemented: reject it with a clear explanation or implement a real idle source before presenting it as functional.

Acceptance: no missing reading appears as 0°C; AMD thermal rules trigger; UI and automation agree on sensor identity; rule decisions explain which condition selected the profile; invalid rules do not fail silently.

## Phase 5 — Make the product state easy to understand

Preserve the current visual direction and original OA branding. No wholesale visual redesign is needed for this milestone.

- Dashboard: emphasize requested/confirmed profile, cooling health, and useful temperatures. Reduce repeated power metrics and decorative space where they displace status.
- Application result: persistent compact status with expandable per-subsystem details, timestamps, and retry. Keep toasts as secondary feedback.
- Cooling: show owner, requested duty, confirmed write state, measured RPM, source freshness, and recovery status with clear distinctions.
- Quick panel: profile choices, key temperatures, cooling warning, and open-window action. Match the main window's state exactly.
- Advanced controls: disclose prerequisites, units, supported limits, and confirmation only when the action warrants it.
- Accessibility: review keyboard focus, theme contrast, reduced-motion behavior, scaling, long hardware names, and unavailable states. Test narrow overlay and large desktop layouts.

Extract coordination, cooling lifecycle, and status presentation incrementally from the application module. Avoid a broad framework rewrite. Define a read-only status API for future integrations only after these state semantics stabilize.

Acceptance: screenshots and interaction checks across small/large layouts, dark/light themes, mixed scaling, no-device and failure states; no clipping of recovery actions; consistent state across all surfaces.

## Phase 6 — Qualify installation and release

Remove the README's broad wildcard overwrite advice. Keep one migration procedure for manual helper installs. Check ownership of every file the uninstaller removes, not just the helper binary; stop with an actionable message if ownership is mixed.

Test clean install, manual-to-package migration, upgrade with active fan ownership, removal, and reinstall in disposable environments. Verify icon assets, activation files, permissions, version consistency, and desktop integration in both portable and Arch artifacts. No AUR work is required.

Required automated checks: locked workspace tests, warnings-as-errors Clippy, fixture reconstruction, failure-injection tests, package staging/content checks, and release version checks. Add dependency/advisory review to the release process without confusing a clean scan with a complete security audit.

Live matrix: ASUS desktop + NVIDIA; ASUS laptop with asusd and hybrid GPU; AMD-only or a supported fixture plus explicitly marked hardware verification pending. Exercise repeated profile changes, tray close/reopen, helper restart, sensor disappearance, suspend/resume, graphics settling, and competing cooling ownership. Perform fault injection in fakes first; do not deliberately stop critical cooling on a live machine.

Publish only after release gates pass. Record exact tested machines, driver versions, known limitations, and migration instructions. Do not upgrade “detected” to “tested” in the support table. Select the next version according to actual compatibility changes; this document does not create a tag.

## Branding delivered with this plan

The new original OA monogram is installed in the source tree as SVG and six PNG sizes for both application and symbolic tray use. The main window and quick panel share the symbolic SVG directly, removing duplicated canvas logo geometry. Repository presentation artwork and usage guidance live in `docs/brand/`. Rebuilding/reinstalling is required to update an already running or packaged application. No reliability fixes above should be inferred from these branding changes.
