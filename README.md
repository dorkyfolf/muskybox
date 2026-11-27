# MuskyBox (VItoriaBr)

MuskyBox is an Xbox Series X|S emulation runtime and shader translator for Windows. The title's original x86-64 code runs unchanged inside our process, while this project supplies the console platform underneath it: loader, high-level emulation (HLE), a D3D12.X host, and a translator from console shaders and resources (D3D12.X / GFX10) to host D3D12 / DXIL. This is not a native game port and does not rewrite the game. See [docs/architecture.md](docs/architecture.md).

Derived from the open source [Marvisak/VIctory](https://github.com/Marvisak/VIctory) with GPL-3.0 world rendering, lighting, relay/VRAM, and DXIL optimizer work integrated from [fxceless/victory-fork](https://github.com/fxceless/victory-fork). Licensed [GPL-3.0-or-later](LICENSE).

## Status

Claims are split by evidence source. A result is called verified here only when the single tester of this run reproduced it and recorded a log. See [ROADMAP.md](ROADMAP.md) for the live milestone state.

### Upstream-reported measurements, not reproduced in this run

The inherited README, dated 2026-11-24, reports the following on a user-supplied Xbox Series build `1.0.2.1`, Windows 11 Pro build 26300, Ryzen 7 5700X, 40 GB RAM, RTX 5060 Ti 16 GB, driver 621.24:

- Boot screens, intro video, account and profile screens, and persistent saves: reported working at 30 FPS.
- Loading into the 3D world: reported working, about 50 s.
- 3D world, opening cutscene and prologue: reported rendering live at about 3 FPS on the first run with `VICTORY_RELAY=2`, with shadowed areas still dark because ray-traced bounce light is incomplete.
- Shader coverage from `victory translate --validate --pipelines`: 26,599 containers scanned, 25,701 of 25,758 DXIL emitted (99.8%), 25,630 validated with `dxil.dll`, 25,304 host PSOs accepted (98.4%). The 397 refused Compute PSOs are reported as wave64-only shaders on a wave32 GPU.

These are inherited reports, not evidence of current behavior on this branch or hardware. [docs/fork-ray-tracing.md](docs/fork-ray-tracing.md), dated 2026-11-25, documents ray tracing as translated and unit-tested but not running end to end in the game: 49 of 52 builder programs and 59 of 64 trace programs translate, and a GPU test covers 311 ray/node cases. Its GPU cost is unmeasured.

### Integrated branch

The single tester recorded the final canonical check set on commit `9636569`, after benign HLE stubs were added for the `libhttpclient.gdk.dll` imports and the `oleaut32.dll` ordinals (see [Evidence and links](#evidence-and-links)). Hardware: Windows 10 Home build 19045.6466, Ryzen 9 5900XT, RTX 4060 Ti 8 GB (driver 595.97), 63.93 GiB RAM, Rust and Cargo 1.98.1, LLVM 22.1.8. The active host was `x86_64-pc-windows-gnu` although `rust-toolchain.toml` declares `x86_64-pc-windows-msvc`.

| Check | Recorded result |
|---|---|
| `cargo fmt --all -- --check` | exit 0, empty log |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0, clean |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0, clean |
| `cargo test --workspace --locked` | exit 0 |
| `cargo xtask policy` | exit 0, `asset policy: 183 files ok` |
| Stability rerun of `cargo test --workspace --locked` | exit 0, 543 passed, 0 failed, 10 ignored, 97 filtered over 26 summaries |

The stability totals sum 26 test-binary summaries. The 10 ignored tests are documented in the logs. No game was launched. A census ran later against title files outside the repository. It scanned 18 images, 0 unreadable, and resolved all 38 `libhttpclient.gdk.dll` functions and both `oleaut32.dll` ordinals through HLE. Of 995 functions in total, 170 are HLE, 791 are host, and 34 are still missing, down from 74 before the stubs. The remaining gaps include AMD FidelityFX, Bink, the BVH builder, and other title DLLs. Save, audio, and async paths have not been exercised in a real run. The verified root-cause analysis (E15 and E16 in [ROADMAP.md](ROADMAP.md)) resolves the crash to a NULL registry root whose required serialized registry blob is absent from the supplied payload; no fix is claimed.

E17 to E20 in [ROADMAP.md](ROADMAP.md) record a later revalidation at HEAD `af81ab8` and the runtime observations behind it. The int3 watch diagnostic in `victory-loader` is opt-in and inert by default (`VICTORY_WATCH_RVA` and `VICTORY_WATCH_COUNT`), and all five canonical checks pass with it integrated (546 passed, 0 failed, 10 ignored, 97 filtered over 26 summaries, policy 184 files). The watch captured interner names and title filesystem URIs, while a synthesized compatibility blob was never opened. The runtime remains unverified, with the `0xC0000005` at `victory-image+0xb5a5f8` persisting; no FPS data or stable gameplay is claimed.

E21 to E23 in [ROADMAP.md](ROADMAP.md) record the opt-in presets compatibility shim in `victory-loader` and the runtime frontier it reached. The shim redirects the title's internal `platform:/data/settings/presets.zmt` open to a `memory:` URI carrying a synthesized 160-byte blob, which eliminated the original `0xC0000005` at `victory-image+0xb5a5f8`. It exposed a pool-size assert that a runtime correction stopped, but with both mechanisms active the run ends in `STATUS_SINGLE_STEP` (`0x80000004`) exceptions at `+0x28cd2a0` without reaching `XUserAddAsync` or frame presented. No FPS telemetry was observed, no title patch was written to disk, and stable gameplay and FPS remain unverified.

E24 to E27 in [ROADMAP.md](ROADMAP.md) update the root cause and the runtime state. Read-only findings from `Belobug/VIctory-Belobug-AMD` (issues #168, #331, #44, #317, #300, same executable SHA `b99f876b`) identified the in-box Windows 10 D3D12 runtime as refusing the title's root signatures with flags `0xc00` or `0xc01` at `hr=0x80070057`, immediately before the `+0xb5a5f8` null dereference in our own root-signature log; no code was copied from that repository. An Agility SDK 1.620.2 fix is applied: `victory.exe` exports `D3D12SDKVersion=620` as DATA and `D3D12SDKPath` as a pointer to `.\D3D12\`, with the official DLLs staged outside the repository, and changing the path export from a byte array to a pointer removed `STATUS_DLL_INIT_FAILED`. A 300 s WARP run at HEAD `1668234` presented 4096 frames with `draws=26624` and `dispatches=2048` at a steady 30.0 fps and 33.3 ms per frame, with no title exception, no root-signature refusals, and no reproduction of the original crash. Real gameplay is still not verified: `drawn=0` and a few host pipelines are refused with `0x80070057`, the opt-in `VICTORY_PLAIN_ROOT=1` diverts the title to a different int3 at `+0x1e6c29f` and is not recommended, and real-adapter runs hit a different title assert at `+0x2474680`; WARP remains the validated path.

E28 to E31 in [ROADMAP.md](ROADMAP.md) record the first visible gameplay on real hardware. At HEAD `9bbf99a`, with the opt-in `VICTORY_AGILITY_CONFIG=1` on an NVIDIA RTX 4060 Ti under Windows 10 build 19045, the tester reached a steady 30.0 fps and 33.3 ms per frame with zero host pipeline refusals and `13 draws/frame, dropped 0` (E28). The log `920-draws-screens.log` shows `frame presented ... frame=2048 draws=13312 drawn=13312 dispatches=1024`, so every title draw became a real host draw (E29). Three whole-screen captures at about 20, 60 and 120 s show the game window titled `Americas | VIctory 30 fps 33.3 ms/frame 13 draws/frame` with the `SELETOR DE CONTA / PLAYER A` account selector rendered (E30). The fix is the opt-in `VICTORY_AGILITY_CONFIG=1` device factory selected before `D3D12CreateDevice`, written independently from a read-only finding with no code copied; the earlier in-box exports alone left `drawn=0` on NVIDIA (E31). Reproducing this on NVIDIA requires `VICTORY_AGILITY_CONFIG=1` and Windows 10.

E32 to E34 in [ROADMAP.md](ROADMAP.md) record the first run that rendered a real map on hardware. At HEAD `813e7d8`, with `VICTORY_AGILITY_CONFIG=1` and `VICTORY_KEYS` driving the title's own stand-in gamepad, a real NVIDIA RTX 4060 Ti under Windows 10 build 19045 ran at 20-30 fps with `drawn=draws` on every presented frame and zero pipeline refusals in `960-step-map.log` (E32). Ten screenshots `step-01.png` through `step-10.png` were captured two seconds after each Enter press; `step-05.png` shows the full GTA VI key art with characters, wings, fire and a textured backdrop, and `step-10.png` shows the `TERMINAR DE CONFIGURAR?` setup screen with a flamingo. Earlier `draws-01/02/03.png` and `vkeys-01.png` show the account selector and `EXPERIENCIA DE NAVEGAÇÃO`. These are real game textures and UI, not a blank window, and all captures live outside the repository in `C:/Users/Gio/AppData/Local/Temp/opencode/` (E33). `VICTORY_KEYS=<seconds>:<key>,...` is the built-in diagnostics input script in `packages/victory-hle/src/modules/gameinput.rs`; for example `VICTORY_KEYS=4:Enter,10:Enter` holds each key for 0.2 s with or without window focus and maps Enter to the stand-in pad A button, which is how the title was driven past its first screens, unlike PowerShell `SendKeys` that needs foreground focus (E34). Prologue completion and full-game completion are still not claimed.

#### Baseline, predating the migration

The earlier baseline in `07-result.log` predates the `packages/` migration commit `f688b57` and is kept only as historical context, not as the current state of the integrated branch:

| Check | Recorded result |
|---|---|
| `cargo fmt --check` | exit 1, 6.104 s, workspace formatting diffs |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0, 47.424 s, clean |
| `cargo test --workspace --locked` | exit 101, 55.499 s, `victory-gpu` 113 passed, 2 failed, 1 ignored, later crates not run |

The baseline failures were `depth_bounds_pipeline_loads_and_filters_draws` (HRESULT `0x80070057`) and `dynamic_allocation_finds_contiguous_range_crossing_search_cursor` (`Some(20)` instead of `Some(14)`). No TDR or device-removal event occurred and the repository diff was empty. A later reproducibility investigation on the GNU host traced the GPU concurrency crash and GNU stack-walk flake to test interference; commit `3a84658` addressed them with `RUST_TEST_THREADS=1` and a `cfg_attr` ignore gate, while `8f2231b` only fixed an invalid checkpoint-order test.

### Unknown, not verified

- 30 FPS 1% low in any scenario.
- Crash-free gameplay, prologue completion, and map or mission transitions.
- Full-game completion.
- The save, audio, and async paths in a real run.
- End-to-end ray-traced lighting and its GPU cost.
- The canonical checks on the MSVC host pinned in `rust-toolchain.toml`. The recorded run used GNU.

The remaining content blocker is that missing serialized registry blob: E15 and E16 in [ROADMAP.md](ROADMAP.md) show it is absent from the supplied payload, so the runtime stays unverified pending user-supplied registry content or an explicit compatibility-blob decision.

#### Runtime attempt

On integration HEAD `7476655`, the single tester recorded a bounded runtime attempt (outside the repository log `C:/Users/Gio/.gti/maestro/muskybox/runtime/game/05-runtime-summary.log`, also documented in [issue #1](https://github.com/dorkyfolf/muskybox/issues/1) on `dorkyfolf/muskybox`):

- Config resolved and the prepare-only run exited 0 in about 1.9 s. The image mapped, the console reported Xbox Series X, and binding produced 88 HLE, 427 host, 0 title, and 72 missing imports. The missing groups were `libHttpClient.GDK.dll` (38), `Bink2Scarlett.dll` (23), `XboxBvhBuilder.dll` (7), and `amd_fidelityfx_loader_xbox.dll` (4).
- The diagnostics build exited 0.
- A bounded WARP execute with DRED and the debug layer exited `0x80000003` after about 2.1 s with a first-chance and unhandled breakpoint inside the title image at `victory-image-b99f876b74d4a77f.dll+0x2237178`. There was no DRED output, device removal, or TDR. The title attempted a minidump upload to `submit.backtrace.io`, which the HLE network policy blocked.
- The supplied game folder contained only `game_scarlett_master.exe`: no `MicrosoftGame.config`, companion DLLs, or media files.
- No hardware run was attempted. FPS, 1% low, frame times, prologue, and map or mission transitions remain unmeasured.

## Architecture

A title executable is mapped and bound by `victory-loader`. Imports resolve in this order: our `victory-hle` implementation, an allowed host function, a title DLL shipped beside the executable, or a named stub. Console D3D12.X objects are translated by `victory-gpu` into host D3D12 work, and shaders are translated when the title creates a pipeline by `victory-shader`, from whatever bytes the title supplies. Converted data goes to a content-addressed cache outside the repository. Details and diagrams are in [docs/architecture.md](docs/architecture.md).

## Repository layout

All workspace members live under `packages/`:

| Package | Role |
|---|---|
| `victory-core` | configuration, asset-policy guard, logging |
| `victory-pe` | read-only PE32+ and import-library analysis |
| `victory-loader` | prepare, map, bind, and start the title image |
| `victory-hle` | console API implementations and host pass-through policy |
| `victory-gpu` | D3D12.X objects, command lists, GPU memory to host D3D12 |
| `victory-shader` | Xbox shader containers to host shaders and DXIL |
| `victory-formats` | console texture layouts and audio codecs, planned BCPack support |
| `victory-pipeline` | converter trait and content-addressed cache |
| `victory-game` | title-specific formats, build identities, hooks |
| `victory` | command-line front end |
| `xtask` | asset-policy check and git hooks |

`tests/fixtures` holds standalone fixtures built on demand by integration tests. It is excluded from the workspace. Documentation is in `docs/`. The single CI workflow is `.github/workflows/ci.yml`.

## Prerequisites

- Windows 10 or 11 x64, CPU with AVX2.
- Direct3D 12 GPU with Shader Model 6.6 and mesh shader support. Use a diagnostics build with WARP for low-risk inspection.
- Rust and Cargo as pinned in `rust-toolchain.toml` (channel 1.98.1, `x86_64-pc-windows-msvc`, rustfmt and clippy), Visual Studio 2022 MSVC build tools, and a recent Windows SDK.
- `dxil.dll` from the Windows SDK for `victory translate --validate`.
- Free commit memory. If `crashes.log` reports error 1455, close other programs or enlarge the page file.

## Quick start

```powershell
copy victory.example.toml victory.toml
# Edit victory.toml so game_dir, cache_dir and data_dir point outside this repository.
cargo run -p victory -- config
cargo fmt --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
cargo build --release -p victory
.\target\release\victory.exe run <game_dir>\<title_exe> --execute
```

The fmt, Clippy, and test commands above reproduce the integrated results recorded in [Status](#status). The release build and `victory run` are not verified here. Without `--execute`, `victory run` only prepares, maps, and binds the title. Other useful commands are `victory census`, `victory inspect`, `victory shader-info`, and `victory translate`.

## Diagnostics

```powershell
cargo build --profile diag -p victory --features diagnostics
```

Selected environment variables, all read from the source in this repository:

| Variable | Effect |
|---|---|
| `VICTORY_CONFIG` | use a different config file instead of `./victory.toml` |
| `VICTORY_GAME_DIR`, `VICTORY_CACHE_DIR`, `VICTORY_DATA_DIR` | override paths from the config file |
| `VICTORY_LOG` | filter logging, for example `victory_loader=trace,info` |
| `VICTORY_D3D12_WARP=1` | force the software adapter in diagnostics builds, avoiding TDR |
| `VICTORY_D3D12_DRED=1` | record device-removal breadcrumbs in diagnostics builds |
| `VICTORY_D3D12_DEBUG=1`, `VICTORY_D3D12_GBV=1` | enable the D3D12 debug layer and GPU-based validation |
| `VICTORY_CAPTURE`, `VICTORY_CAPTURE_FRAMES`, `VICTORY_DUMP_TARGETS` | save frames and render targets |
| `VICTORY_SHADER_LOOP_LIMIT` | cap iterations in translated shader loops |
| `VICTORY_SKIP_PIPELINES` | skip draws or dispatches by shader hash |
| `VICTORY_VIEW_LIMIT`, `VICTORY_HOST_WAVE` | simulate a capped view heap or wave size during translation |
| `VICTORY_NO_AUDIO=1` | temporary mute switch: decoded PCM is written as silence while HLE calls still succeed (OFF by default) |

## Controls

When no XInput controller is connected, the keyboard drives a stand-in pad:

| Keys | Control |
|---|---|
| `W A S D` / `I J K L` | left / right analog stick |
| Arrow keys | D-pad |
| `Enter`, `Backspace`, `X`, `Y` | A, B, X, Y |
| `Q` / `E`, `Z` / `C` | LB/RB, LT/RT |
| `F`, `H` | L3, R3 |
| `Esc`, `Tab` | Menu, View |

## Asset policy

Nothing from the game is stored in this repository, including assets, executables, derived data, dumps, or screenshots. Paths and derived data live outside the repository and are configured in `victory.toml`. The `.victory-root` marker makes tools refuse game-derived output inside the tree, and `cargo xtask policy` is an allowlist run by the pre-commit hook (`cargo xtask install-hooks`). The only exception is `packages/victory-formats/src/bcpack-tables.bin`, which is documented in [docs/asset-policy.md](docs/asset-policy.md). Tests that need real files read paths from environment variables and skip when unset.

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). The BCPack table exception is not covered by this license.

## Evidence and links

- [ROADMAP.md](ROADMAP.md): measurable milestones and their evidence.
- [AGENTS.md](AGENTS.md): repository working rules.
- [docs/architecture.md](docs/architecture.md): runtime and translator design.
- [docs/asset-policy.md](docs/asset-policy.md): what may and may not enter the repository.
- [docs/fork-ray-tracing.md](docs/fork-ray-tracing.md): ray-tracing status and known gaps.
- Tester baseline log: `C:/Users/Gio/.gti/maestro/muskybox/runtime/w1-tester-20261125/07-result.log` (outside the repository).
- Integrated canonical check set: HEAD `8f2231b`, log directory `C:/Users/Gio/.gti/maestro/muskybox/runtime/integration-wave2-3/` (outside the repository). Commands and exit codes: `cargo fmt --all -- --check` (0), `cargo clippy --workspace --all-targets --locked -- -D warnings` (0), `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` (0), `cargo test --workspace --locked` (0), `cargo xtask policy` (0), and a stability rerun of `cargo test --workspace --locked` (0). `06-result.log` holds the summary and `12-final-stability.log` holds the stability totals.
- Canonical revalidation after the benign HLE stubs: HEAD `9636569`, same log directory (outside the repository). `64-result.log` records all five canonical checks exit 0 and 543 passed, 0 failed, 10 ignored, 97 filtered over 26 summaries; `63-policy.log` records `asset policy: 183 files ok`. `58-census-after-stubs.log` records 18 images scanned, 0 unreadable, 38 of 38 `libhttpclient.gdk.dll` and 2 of 2 `oleaut32.dll` functions resolved by HLE, and 34 missing imports out of 995.
