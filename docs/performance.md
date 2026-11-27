# Performance

This document separates facts read from the source from measurements published
elsewhere. Nothing here was measured locally as part of this documentation
task: no build, GPU run or game run was performed.

## Verified from source

### Diagnostics instrumentation

`packages/victory-gpu/src/copystat.rs` defines the counters printed with the
frame-rate line when the `diagnostics` feature is on. Among them are
`CopyBytes`, `RefreshBytes`, `SkippedBytes`, `BackCompared`, `BackScanUs`,
`BackSyncUs`, `PipelineWaitUs`, `RecordNs`, `PaceUs`, `DynamicRefused`,
`UntilRecordUs`, `UntilSubmitUs` and `TailUs`. `VICTORY_COPY_MODE=plain` copies
without the mapped-page guard and `VICTORY_COPY_MODE=none` skips copies, both
for timing experiments only. A separate timestamp heap and read-back buffer in
`host.rs` provides GPU timing.

### Copy-back path

The title's heaps are mirrored by shadow copies. After a submission,
`hand_back` in `host.rs` moves what shaders wrote into title memory, and
`read_backs` moves texels copied from textures into host buffers. The source
records two measured bottlenecks:

- The world declares dozens of overlapping views of a 174 MB heap. Comparing
  every view whole cost gigabytes of checks per frame (0.3 to 0.5 s) for only a
  few MB actually written. `hand_back` now sorts and merges overlapping or
  touching ranges so each byte is compared once; the `wide_back()` path scans
  64 KiB pieces on the card in parallel.
- Submitting is serialized: one submission at a time runs from preparation to
  copy-back under a lock. A source comment for run 92 records a settle taking
  250 ms inside 135 ms of preparing. The rehash passes before each submission
  also cost time: run 102 sent up 514 ranges in one frame.

`copystat.rs` also records an address-space walk of committed and mapped
memory, with measured costs such as 0.6 s for the walk and 89 ms for a query
over a 6.2 GB page pool.

### Host wave width and TDR guards

At device creation `host.rs` queries `WaveLaneCountMax` and passes it to
`victory_shader::emit::set_host_max_wave`. On a 32-lane host, a console wave64
compute program whose cross-lane code stays inside 32-lane halves is lowered to
two host waves. Anti-TDR guards in `host.rs` and `indirect.rs` submit one
command list per fence wait, cap `DispatchMesh` arguments and group counts, and
cap translated loops at 4096 iterations by default
(`VICTORY_SHADER_LOOP_LIMIT`).

### Shader cache key

`shader_cache.rs` builds the cache fingerprint from the BLAKE3 of the executable
plus the loop limit, view heap limit, group-sync mode, the
`VICTORY_WAVE_WIDE`, `VICTORY_WAVE_DEEP_SKIPS`, `VICTORY_WAVE_OWN_BITS` and
`VICTORY_F16_NEAREST` switches, and the clean-up pass bits. The host wave width
is not included, although it changes translation. This is open work.

## Upstream-reported measurements (not locally verified)

The figures below are published in this repository's `README.md`. They are
upstream-reported, not locally verified by this documentation task, and no
claim is made here about current behavior or full-game completion.

Test setup as reported: Windows 11 Pro build 26300, Ryzen 7 5700X, 40 GB RAM,
NVIDIA RTX 5060 Ti 16 GB (SM 6.6, mesh shaders, driver 621.24), Xbox Series
build 1.0.2.1.

| Stage | Upstream-reported status |
|-------|--------------------------|
| Boot and first-boot screens | works, 30 fps |
| Intro video with audio | works |
| Start screen, account selector, saves | works |
| Loading into the world | works, about 50 s |
| 3D world, opening cutscene | rendering on the RTX 5060 Ti, about 3 fps on the first run with `VICTORY_RELAY=2`, zero TDR |

Shader and PSO coverage as reported via `victory translate --validate
--pipelines` with `dxil.dll` from Windows SDK 10.0.26100.0 and driver 621.24:

| Stage | DXIL emitted | Validated | Host PSOs |
|-------|--------------|-----------|-----------|
| Vertex and pixel | 14,211 / 14,259 | 14,211 | 14,211 |
| Mesh | 8,762 / 8,765 | 8,691 | 8,762 |
| Compute | 2,728 / 2,734 | 2,728 | 2,331 |
| Total | 25,701 / 25,758 (99.8%) | 25,630 | 25,304 (98.4%) |

The README attributes the 397 refused compute PSOs to wave64-only shaders on a
wave32 NVIDIA card. A separate source comment records that with the host's
background shader work running, one shot took 1,000 to 1,200 ms per frame and
830 to 950 ms after it stopped (run 54), but does not say which is faster
overall. These are reported observations, not benchmarks reproduced here.

## Metrics to measure

A tester should report these over named scenario windows:

- **average FPS**: frames presented divided by wall-clock seconds in the
  window;
- **1% low**: the frame rate derived from the mean frame time of the slowest 1%
  of frames in the window, reported in FPS;
- **frame time**: wall-clock milliseconds between consecutive presents,
  reported as mean and percentiles;
- **copy bytes**: bytes copied between title and host memory per frame or per
  submission (the `CopyBytes`, `RefreshBytes` and `BackCompared` counters);
- **GPU time**: host card execution time for submitted command lists from
  timestamp queries, kept separate from frame time;
- **crashes**: faults caught by the loader crash reporter, with `module+rva`
  and a stack unwind, counted per scenario;
- **TDRs**: Windows Timeout Detection and Recovery events (`DEVICE_HUNG` or
  `DEVICE_REMOVED`) counted per scenario;
- **scenario transitions**: named title phases such as boot, menus, loading
  and 3D world, with the frame or time marker where each begins and ends, so
  every metric can be segmented by phase.

## Test strategy

Keep the platform layer testable without game data: unit tests in each crate,
loader integration tests against `tests/fixtures/hle-probe`, `victory census`
for import classification, and `victory translate --validate --pipelines` for
translator and pipeline coverage. Timing and stability metrics need a tester on
real hardware, ideally with a `diagnostics` build; `VICTORY_D3D12_WARP=1` runs
on the software adapter for safe inspection without TDR risk. No such run was
performed for this document.

## Open work

- Add host wave width to the shader cache key so a cache built on one GPU
  cannot serve a translation made for another wave width.
- Reduce the copy-back and submission cost documented above; the measured
  figures quoted are source comments and upstream reports, not local results.
- Numeric results are still pending the performance scout; until then the
  table above is the only published data and remains unverified here.
