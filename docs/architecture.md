# Architecture

VItoriaBr is an Xbox Series X|S emulation runtime and shader translator, not a
native game port. The title's x86-64 code runs unchanged inside our process;
the runtime supplies the console platform and translates console resources and
shaders (D3D12.X / GFX10 to host D3D12 / DXIL). The user supplies every file,
and all derived data lives outside the repository. See
[docs/asset-policy.md](asset-policy.md).

```
 user files (outside repo)              VItoriaBr
 ──────────────────────────              ──────────────────────────────────────
 game assets ──────────────────────────► victory-pipeline + victory-game
                                         (victory-formats, victory-shader)
                                                    │
                                                    ▼
                                         cache_dir: converted data
                                                    │
 game executable ─► victory-loader ────► title code (native x86-64)
                    prepare, map, bind      │ imports
                                            ▼
                                         victory-hle ── host Windows APIs
                                            │ D3D12.X
                                            ▼
                                         victory-gpu ─► host D3D12
                                            │ shaders at pipeline creation
                                            ▼
                                         victory-shader ◄─► cache_dir
```

Shaders are translated when the title creates a pipeline, from the bytes it
passes, wherever it loaded them from (executable, archive, or memory).
Converting files ahead of time is only a warm-up.

## Workspace layout

After the `packages/` migration the workspace members are:

| Path | Role |
|------|------|
| `packages/victory-core` | config, asset-policy guard, logging |
| `packages/victory-pe` | read-only PE32+ and import-library analysis |
| `packages/victory-formats` | console texture layouts (sizes, tiling, detiling), audio codecs; BCPack planned |
| `packages/victory-shader` | console shader containers to host DXIL |
| `packages/victory-gpu` | D3D12.X objects, command lists, GPU memory to host D3D12 |
| `packages/victory-hle` | console API implementations, host pass-through policy |
| `packages/victory-loader` | prepare, map, bind and start the title image |
| `packages/victory-pipeline` | converter trait, content-addressed cache |
| `packages/victory-game` | title-specific formats, build identities, hooks |
| `packages/victory` | command-line front end |
| `packages/xtask` | asset-policy check, git hooks |
| `tests/fixtures/hle-probe` | stand-in title (our own code) for loader tests, excluded from the workspace |

Everything except `packages/victory-game` is platform-level and can be tested
with any Xbox Series title.

## Loader and boot path

`packages/victory-loader` prepares, maps and binds the title image, then starts
it as native x86-64 code. An executable reaches its static TLS through slot 0;
mapped as a DLL it gets another slot, so the loader redirects those loads
(`tls.rs`). `victory run --execute` installs a crash reporter that names faults
as `module+rva` and unwinds the faulting thread's stack (`crash.rs`).
`victory census` classifies imports without running anything; it does not map
title DLLs, while `victory run` without `--execute` does.

## Import resolution and HLE

`packages/victory-hle` owns a registry of console API implementations
(`registry.rs`, `modules/`). For every imported `dll!function`, in order:

1. our implementation in `victory-hle::modules` always wins;
2. the host's own function, when `HostPolicy` allows that DLL;
3. an export of a title DLL shipped beside the executable, mapped like the
   executable (only DLLs the first two steps do not serve);
4. a generated stub that names the import when called (`--unimplemented trap`
   breaks into the debugger and aborts; `zero` logs once and returns 0).

`LoadLibrary*` of a console-only DLL we implement (for example
`xgameruntime.dll`) returns a pseudo handle, and `GetProcAddress` on it finds
our functions. `LoadLibrary*` of a title DLL returns its mapped image, mapping
it first if nothing imports it. A title gets only the host DLLs `HostPolicy`
passes through, and `GetProcAddress` returns our override where import binding
would. `QueryApiImpl` hands out runtime objects by provider and interface id
(`modules/xgameruntime.rs` lists the areas); online areas answer as an offline
console. Game saves are files under `VICTORY_DATA_DIR`.

Title storage is a drive on the console, and titles build paths on that.
`modules/files.rs` binds every path-taking title import (kernel32 and the C/C++
runtime) and rewrites virtual-drive paths to the host folder behind them;
persistent local storage is one such drive over a folder under
`VICTORY_DATA_DIR`. Other paths reach the host unchanged.

## D3D12.X host

`packages/victory-gpu` implements D3D12.X on host D3D12. D3D12.X records
commands from code inlined into the title, which writes packets into the
command list while its put pointer is below a limit and calls the list method
otherwise. Our lists keep the limits at zero, so every command arrives as a
call and is run on host D3D12. Title GPU addresses are CPU addresses because
console memory is unified (`lib.rs`).

`d3d12_xs.dll` (`modules/d3d12x.rs` in `victory-hle`) handles virtual memory
setup, device creation and root signature serialization; the device and
everything it creates live in `victory-gpu`. The host device is created with
`D3D12CreateDevice`, or with WARP when `VICTORY_D3D12_WARP=1`. The runtime reads
`WaveLaneCountMax` and reports it to the translator, so a console wave64
compute program whose cross-lane code stays in 32-lane halves can run as two
host waves of 32. `host.rs`, `shadow.rs`, `watch.rs` and `inplace.rs` keep title
memory, host copies and write-watches coherent; `hand_back` in `host.rs` moves
what shaders wrote back into title memory. One submission at a time runs from
preparation to copy-back under a submission lock.

The module map includes `device`, `dxgi`, `resource`, `fence`, `queue`, `list`,
`descriptor`, `root`, `query`, `signature`, `pipeline`, `frame` and `object`,
plus diagnostics modules behind the `diagnostics` feature.

## Shader translator

`packages/victory-shader` translates console shader containers:

- `ir::build` decodes a native program into SSA, one instruction per machine
  instruction (`ir.rs`).
- `structure` removes EXEC: lane masks become per-lane booleans, wave-wide
  mask tests become per-lane branches, and vector instructions keep a guard
  only where a lane can reach them with its EXEC bit clear (`structure.rs`).
- `resource` traces each descriptor and scalar memory address to its root
  parameter, and follows a descriptor read from the first lane of a waterfall
  loop to the per-lane load or choice that gives each lane its own value, then
  accesses it with a non-uniform heap index (`resource.rs`).
- `emit` lowers the result to host DXIL, including the wave64 to 32-lane wave
  forms, and applies exact clean-up passes (`emit/`).

`victory translate` runs every container or SGD5 file it finds through these
stages and lists what they cannot handle yet. See
[docs/performance.md](performance.md) for measured coverage and bottlenecks.

## Caches

All converted data lives in the user's `cache_dir` (default
`%LOCALAPPDATA%\victory\cache`), never inside the repository.
`packages/victory-pipeline` provides a content-addressed store whose key is a
BLAKE3 hash of a converter id, version and input; writes go to a temporary file
and are renamed into place so readers never see a partial result.

`victory-gpu` keeps translated shaders and compiled host pipelines in that
store (`shader_cache.rs`, `pipeline/`). The shader cache key adds a build
fingerprint to the container bytes: the BLAKE3 of the running executable plus
the loop limit, view heap limit, group-sync mode, selected wave and F16
switches, and clean-up pass bits. An entry that cannot be read back is
translated again. The host wave width is not part of that key yet; see open
work in [docs/performance.md](performance.md).

## Test harness

Testing needs no game assets:

- unit tests live beside the code in each `packages/` crate;
- `packages/victory-loader` integration tests (`load_probe.rs`,
  `title_dlls.rs`) prepare, load, bind and call into the stand-in title at
  `tests/fixtures/hle-probe`;
- `victory translate` with `--validate --pipelines` exercises the translator
  and, on a machine with the Windows SDK, validates emitted DXIL and creates
  host pipelines;
- `victory census` classifies imports without running the title;
- `cargo xtask policy`, wired as a pre-commit hook, enforces the asset policy.

These checks cover the platform layer. Runtime FPS, GPU timing and stability
require a tester on real hardware and are not established by this
documentation task.
