# AGENTS.md

Durable rules for agents working in this repository. Keep this file short. Every rule is tied to a repository file or a recorded project decision; remove a rule when its source no longer holds.

## Language and formatting

- Code and comments are in English. Use four spaces where indentation applies. Existing sources and `rustfmt.toml` set the convention; do not introduce tabs.
- Keep formatting under the `max_width` in `rustfmt.toml` and leave its settings intact.

## Canonical checks

Run these before handing work off. Only the tester agent executes them.

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

- `.github/workflows/ci.yml` defines the two Clippy commands, the test command, and the formatting check added at commit `58c0987`. The test job also runs the asset policy check through xtask.
- `rustfmt.toml` and the tester baseline define the formatting check. The portable Linux matrix is still not configured, and MSVC CI coverage is unverified.

## One tester

- Only the tester agent runs builds, tests, GPU work, and game runs. Every other agent makes static changes and sends test requests to the tester.
- This serializes access to the machine and the GPU. The hardware warning in `README.md` explains why uncontrolled runs can reset the driver.

## Assets

- No game files, keys, executables, saves, captures, or derived assets in the repository. Game paths, dumps, caches, captures, and runtime artifacts stay outside it and are configured in `victory.toml`.
- Follow `docs/asset-policy.md`. `cargo xtask policy` is the authoritative allowlist and runs from the pre-commit hook in `.githooks/pre-commit`, installed with `cargo xtask install-hooks`.
- The single documented exception is in `docs/asset-policy.md` and `packages/xtask/src/policy.rs`. Do not add new exceptions.

## License and attribution

- The project is GPL-3.0-or-later, as stated in `LICENSE` and `Cargo.toml`. Preserve license notices.
- Preserve attribution to the upstream forks named in `README.md`: Marvisak/VIctory and fxceless/victory-fork.

## Git

- Commit messages are in English and conventional, in the `type: summary` form used by the existing history in `git log`, such as `docs:`, `gpu:`, `refactor:`, and `shader:`.
- No agent pushes without explicit permission. Pushes are allowed only for the designated account after authentication.

## Verification

- At commit `d80b396`, `cargo fmt --all -- --check`, both Clippy variants with `-D warnings`, `cargo test --workspace --locked`, and `cargo xtask policy` all exit 0.
- Test totals at that commit are 543 passed, 0 failed, 10 ignored, 97 filtered.
- The evidence index lives in `ROADMAP.md` entries E10 and E11, with matching entries in `README.md`.
- The verified host was `x86_64-pc-windows-gnu` while `rust-toolchain.toml` pins MSVC, so MSVC checks and all runtime behavior remain unverified.

## Binary analysis

- Use the Reverse Engineer Anything (REA) toolkit for every investigation that depends on a shipped binary, package, or crash artifact rather than on source in this repository. Do not wait for supplier symbols or PDB files that may never be provided.
- Runtime crashes inside the title image are resolved through REA static analysis of the original executable. Keep the title and all runtime artifacts outside the repository, and cite the REA Evidence ID for every conclusion.
- State explicitly what was observed, what was inferred, and what remains unknown. Static analysis never proves execution.

## Web research

- Agents may use the web to research binary formats, file signatures, SDK APIs, GUIDs, error codes, and public documentation when local sources are insufficient. Web findings are supporting evidence only: cite the source URL and separate what the source states from what was observed locally.
- Prefer primary sources such as official documentation, published format specifications, and reputable open-source parsers. Treat blog posts, forums, and wiki mirrors as leads to verify, not as proof.
- Never fetch, download, or execute third-party binaries, game files, keys, or copyrighted assets from the web. Web research is read-only; the supplied game payload stays local and outside the repository.
- A web source does not replace the tester. Do not claim runtime behavior from documentation alone.

## Orchestration

- Maestro runs autonomously: do not stop for input, do not open tabs or viewers for the user, and finish with one report. Record every autonomous decision in `decisions.jsonl`.
- Use the full Maestro skill: planning, delegation, review, autonomous mode, teacher and student mode when the user names it, review panels for pull requests, team mode for long hard runs, and the central when asked. Prefer the skill's recommended option at every decision and record the reason.
- Maestro plans, routes, reviews, and reports. Workers write the code and run the commands. If a source or docs change is needed, send it back to a worker chat with a self-contained brief; do not author the change directly.
- Route by model tier and cost. Prefer free models only: Hy4 Flash Free (no rate limit) and HungerG Free (rate limited) first; Beam Free / Beam 1.0 is a weaker free fallback. Paid models are limited to DeepSeek v4.1 Flash and GLM 5.3 Flash, and only for extremely long debugging or code that cannot fail. If a preferred model is rate limited, cool it down and fall back; never retry unchanged.
- Spread work across available machines. This host and `h-acer-nitro-v15` may both run workers; do not concentrate all tasks on one. Remote Paseo agents need `daemon.remoteAccess` on both daemons; when unavailable, use the existing SSH alias and Paseo CLI on the Acer host.
- Kill old, idle OpenCode processes that are no longer in use, without killing the tester, active workers, or the current session.
- Keep the idle watchdog active so work resumes automatically if Maestro stalls.

## Evidence updates

- Update `ROADMAP.md` and `AGENTS.md` only from verified evidence: the exact command, exit code, date, hardware and toolchain where relevant, and the source file or tester log.
- Mark claims without evidence as unknown. Do not assert FPS, crash-free behavior, or full-game completion without tester output.
- Keep every `AGENTS.md` rule tied to a repository file or a recorded decision. Remove stale rules instead of leaving them to accumulate.

## Title input commands

- Use `VICTORY_KEYS` or the host key map for title input. Controller support is a future goal and is not implemented now.
- Known host key to Xbox button mapping from `FALLBACK_BUTTONS` in `packages/victory-hle/src/modules/gameinput.rs`:

| Host key | Xbox button |
| --- | --- |
| Esc, Tab, Enter, Backspace | Menu, View, A, B |
| X, Y | X, Y |
| Arrows | D-pad |
| Q, E | LB, RB |
| F, H | L3, R3 |
| WASD, IJKL | Left stick, right stick |
| Z, C | LT, RT |

- Live probe at the final `TERMINAR DE CONFIGURAR?` screen: B opens `CONFIGURACOES` (brightness), Right adjusts brightness, Down changes menu focus, and A activates the focused item. Move focus to `CONTINUAR` before pressing A; pressing A while `CONFIGURACOES` is focused opens the submenu or restarts the wizard.
- The full map with live-probe evidence lives outside the repo at `C:/Users/Gio/.gti/maestro/muskybox/runtime/input-commands.md` (logs `B0-map-commands.log` and `map-*.png`). Do not add those files to the repo.
