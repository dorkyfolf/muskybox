//! Opt-in, read-only CPU context probe at one title code address.
//!
//! `VICTORY_WATCH_RVA` (hex with `0x`, or decimal) names an RVA in the loaded
//! title image; `VICTORY_WATCH_COUNT` (default 1) caps how many times the
//! context is logged. With neither set, nothing is armed and this module is
//! inert. A software `int3` is written over the first byte at that address
//! before title code runs. Our vectored handler reads the bounded strings at
//! both RCX and RDX, logs the hits, puts the original byte back, single-steps
//! the original instruction and re-arms while hits remain. The byte and the
//! page protection are restored after the last hit.
//!
//! `VICTORY_WATCH_QWORDS=1`, active only with `VICTORY_WATCH_RVA`, adds one log
//! line with RSP and the four qwords at `[rsp+0x70]`, `[rsp+0x78]`,
//! `[rsp+0x80]` and `[rsp+0x88]`. Guarded reads yield 0 when a word is not
//! readable. Absent or any other value leaves the existing log unchanged.
//!
//! `VICTORY_DUMP_MEMORY_URI=1`, active only with `VICTORY_WATCH_RVA`, adds an
//! opt-in dump of a real title `memory:$` blob at a watch hit. The URI at RCX
//! or RDX is parsed for its address and size; when the size is between 1 byte
//! and 1 MiB and the range is committed and readable, the bytes are written to
//! `C:\Users\Gio\AppData\Local\Temp\opencode\memuri-<n>.bin`. At most eight dumps
//! are written per process. Absent or any other value leaves the watch
//! unchanged.
//!
//! `VICTORY_NAME_SWAP=from:to`, active only with `VICTORY_WATCH_RVA`, swaps the
//! name at RCX on a watch hit. Both names must be non-empty printable ASCII
//! with no spaces and one `:` separator. When the current name equals `from`
//! exactly, RCX is pointed at a process-lifetime copy of `to` and one event is
//! logged. No other register changes; an absent or malformed value is inert.
//!
//! `VICTORY_PRESETS_BLOB` names a host blob file and arms a separate, opt-in
//! compatibility shim at the title's internal `presets.zmt` open site
//! (RVA `0x1972d95`). The blob is copied at offset 0 into the first bytes of a
//! process-lifetime RW pool sized by the allocation constant, while a
//! `memory:$%016X,%u,0:%08X` URI declaring a separate, smaller size is built
//! in a second allocation. On each hit the shim reads the ANSI path at RDX
//! and, when it names `presets.zmt`, swaps RDX for the URI pointer.
//! It shares the handler, the byte restore and the single-step logic above,
//! and is inert without the variable.
//!
//! `VICTORY_SKIP_ASSERT=1` is a narrow runtime diagnostic: when the byte saved
//! by `VICTORY_WATCH_RVA` is itself `0xcc`, the breakpoint is a genuine title
//! `int3` rather than our patch. The hit is consumed but the byte, the trap
//! flag and re-arming are left untouched, and RIP moves past the instruction.
//! Any other value leaves behavior unchanged.
//!
//! `VICTORY_SKIP_PRESETS_ASSERT=1`, active only with `VICTORY_PRESETS_BLOB`,
//! patches the title's pool-size assert branch (RVA `0x247af2b`) from JBE to
//! JMP in memory after the presets int3 is armed. The byte is not restored and
//! no handler is added for it. Absent or any other value leaves behavior
//! unchanged.
//!
//! `VICTORY_FIX_PRESETS_ASSERT=1`, active only with `VICTORY_WATCH_RVA` when the
//! watched address is the title's pool assert (RVA `0x247af2d`), corrects the
//! declared pool size in place at the int3. The title reports `size - ptr +
//! base` in RAX; when that exceeds `0x100000`, the excess is subtracted from
//! the qword at `[rsp+0x78]` and RAX is set to `0x100000`. The hit continues at
//! `va + 1` without a single-step or re-arm. Absent or any other value leaves
//! the watch unchanged.
//!
//! `VICTORY_NO_STEP=1` narrows the control watch only: after its hit is logged
//! and any assert correction applied, RIP moves to `va + 1` past the genuine
//! title `int3`, and no trap flag or step owner is set, so no re-arm follows.
//! The presets shim is unaffected: it restores the byte, redirects, single-steps
//! and re-arms as if the variable were absent. Any other value leaves the watch
//! unchanged.

use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use windows_sys::Win32::Foundation::{EXCEPTION_BREAKPOINT, STATUS_SINGLE_STEP};
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, CONTEXT, EXCEPTION_POINTERS, FlushInstructionCache,
};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE, PAGE_GUARD,
    PAGE_NOACCESS, PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualAlloc, VirtualProtect,
    VirtualQuery,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId};

const CONTINUE_SEARCH: i32 = 0;
const CONTINUE_EXECUTION: i32 = -1;
/// `EFLAGS.TF`, bit 8: request one single-step after the breakpoint hit.
const TRAP_FLAG: u32 = 1 << 8;
/// Bytes read from `[rcx]` and `[rdx]`: the most a probe may copy.
const MAX_STRING_BYTES: usize = 256;
/// Stack offsets from RSP logged under `VICTORY_WATCH_QWORDS=1`.
const QWORD_OFFSETS: [usize; 4] = [0x70, 0x78, 0x80, 0x88];
/// x86-64 pages are 4 KiB; the patch never spans a page boundary.
const PAGE_SIZE: usize = 0x1000;
/// Internal open of `platform:/data/settings/presets.zmt` in the title image.
const PRESETS_RVA: usize = 0x1972d95;
/// Allocation size of the presets memory pool, 64 MiB. The blob is copied at
/// offset 0 into the first `bytes.len()` bytes; committed pages past it stay
/// zero. This is the size passed to `VirtualAlloc` and is not the size the URI
/// declares. The title's assert at RVA `0x247af2d` tests the URI's declared
/// size, so the allocation and the declaration are deliberately separate.
const PRESETS_POOL_SIZE: usize = 0x4000000;
/// Declared size embedded in the presets memory URI, 18743296 bytes. Runtime
/// evidence showed the working assert correction rewrote only the declared
/// size at `[rsp+0x78]`, leaving the allocation untouched; using this value
/// for the allocation as well failed. Used only when formatting the URI and
/// checking its round-trip.
const PRESETS_URI_SIZE: usize = 0x11E0000;
/// RVA of the pool-size assert branch. The title has `76 01` (JBE rel8 +1)
/// here and an INT3 at `+1`; the assert fails after the redirected memory URI.
const PRESETS_ASSERT_RVA: usize = 0x247af2b;
/// Expected byte at [`PRESETS_ASSERT_RVA`]: JBE rel8 +1.
const PRESETS_ASSERT_JBE: u8 = 0x76;
/// Replacement byte at [`PRESETS_ASSERT_RVA`]: JMP rel8 +1.
const PRESETS_ASSERT_JMP: u8 = 0xeb;
/// RVA of the title `int3` inside the pool-size assert. The JBE at
/// [`PRESETS_ASSERT_RVA`] skips this byte when the declared size passes.
const PRESETS_ASSERT_INT3_RVA: usize = 0x247af2d;
/// Target RAX value at the pool assert. RAX holds `size - ptr + base`; the
/// excess above this target is removed from the declared size at `[rsp+0x78]`.
const PRESETS_ASSERT_SIZE_TARGET: u64 = 0x100000;
/// Generous cap on intercepted opens before the shim disarms for good.
const PRESETS_HIT_BUDGET: usize = 64;
/// Largest `memory:$` payload this opt-in dump will copy, one MiB.
const MEMURI_MAX_SIZE: usize = 1024 * 1024;
/// Maximum number of `memory:$` dumps written per process.
const MEMURI_MAX_DUMPS: usize = 8;
/// Directory receiving `memuri-<n>.bin` files; kept outside the repository.
const MEMURI_DUMP_DIR: &str = r"C:\Users\Gio\AppData\Local\Temp\opencode";

/// Identifies which patch requested the single-step pending on a thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PatchKind {
    /// The control watch armed by `VICTORY_WATCH_RVA`.
    Watch,
    /// The presets shim armed by `VICTORY_PRESETS_BLOB`.
    Presets,
}

thread_local! {
    /// The single-step owner on this thread. A patch sets this when it writes
    /// `EFLAGS.TF`; the next `STATUS_SINGLE_STEP` takes and clears it.
    static STEP_OWNER: Cell<Option<PatchKind>> = const { Cell::new(None) };
}

/// Records `kind` as the owner of the single-step just requested on this
/// thread. Called alongside every write of `EFLAGS.TF`, so a step is always
/// attributed to the patch that armed it.
fn set_step_owner(kind: PatchKind) {
    STEP_OWNER.with(|owner| owner.set(Some(kind)));
}

/// Takes and clears this thread's pending single-step owner. Returns `None`
/// when no patch on this thread requested a step.
fn take_step_owner() -> Option<PatchKind> {
    STEP_OWNER.with(Cell::take)
}

/// The one software breakpoint. Every field is guarded by [`WATCH`].
struct Watch {
    /// The watched VA; zero while unwatched.
    va: usize,
    /// The byte at `va` before the `int3` was written.
    original: u8,
    /// The page protection before it was made writable.
    old_protect: PAGE_PROTECTION_FLAGS,
    /// Logs still allowed.
    remaining: usize,
    /// Whether `0xcc` is currently written at `va`.
    armed: bool,
    /// Whether our single-step is pending after a hit.
    stepping: bool,
    /// Skip a saved `0xcc` as a title `int3`, from `VICTORY_SKIP_ASSERT`.
    skip_assert: bool,
    /// Correct the pool assert size at this watch, from
    /// `VICTORY_FIX_PRESETS_ASSERT` and the watched RVA.
    fix_assert: bool,
}

static WATCH: Mutex<Watch> = Mutex::new(Watch {
    va: 0,
    original: 0,
    old_protect: 0,
    remaining: 0,
    armed: false,
    stepping: false,
    skip_assert: false,
    fix_assert: false,
});

/// Locks the breakpoint state, recovering from a poisoned lock.
fn watch() -> MutexGuard<'static, Watch> {
    WATCH.lock().unwrap_or_else(|e| e.into_inner())
}

/// Number of `memory:$` dumps already written, shared by all threads.
static MEMURI_DUMPS: AtomicUsize = AtomicUsize::new(0);

/// Reserves the next sequential dump index, or `None` once the cap is reached.
fn reserve_dump_slot() -> Option<usize> {
    MEMURI_DUMPS
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
            next_dump_index(used).map(|index| index + 1)
        })
        .ok()
}

/// The presets shim's own breakpoint and blob. Kept separate from [`Watch`]
/// so the control watch stays byte-for-byte unchanged when it is the only
/// variable set. Every field is guarded by [`PRESETS`].
struct Presets {
    /// The shim's VA; zero while unarmed.
    va: usize,
    /// The byte at `va` before the `int3` was written.
    original: u8,
    /// The page protection before it was made writable.
    old_protect: PAGE_PROTECTION_FLAGS,
    /// Intercepted opens still allowed.
    remaining: usize,
    /// Whether `0xcc` is currently written at `va`.
    armed: bool,
    /// Whether our single-step is pending after a hit.
    stepping: bool,
    /// Process-lifetime allocation holding the blob bytes.
    blob: usize,
    /// Length of the real blob at the start of the pool, not the URI size.
    blob_len: usize,
    /// Process-lifetime allocation holding the NUL-terminated URI.
    uri: usize,
    /// Same URI as a Rust string, for logging without a second pointer read.
    uri_text: String,
}

static PRESETS: Mutex<Presets> = Mutex::new(Presets {
    va: 0,
    original: 0,
    old_protect: 0,
    remaining: 0,
    armed: false,
    stepping: false,
    blob: 0,
    blob_len: 0,
    uri: 0,
    uri_text: String::new(),
});

/// Locks the presets state, recovering from a poisoned lock.
fn presets() -> MutexGuard<'static, Presets> {
    PRESETS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Process-lifetime name substitution installed by `VICTORY_NAME_SWAP`. It is
/// set once in `configure` and read without locking from the exception handler.
struct NameSwap {
    /// Pointer to the leaked NUL-terminated `from` buffer.
    from: usize,
    /// Length of `from` in bytes, excluding its NUL.
    from_len: usize,
    /// Pointer to the leaked NUL-terminated `to` buffer.
    to: usize,
    /// Length of `to` in bytes, excluding its NUL.
    to_len: usize,
}

/// The installed name swap, or `None` when no usable value was supplied.
static NAME_SWAP: OnceLock<NameSwap> = OnceLock::new();

/// Leaks a NUL-terminated ASCII copy of `text` and returns its pointer. The
/// allocation is never freed because the title may read it for the whole
/// process lifetime.
fn leak_nul_terminated(text: &str) -> usize {
    let mut bytes = Vec::with_capacity(text.len() + 1);
    bytes.extend_from_slice(text.as_bytes());
    bytes.push(0);
    let boxed = bytes.into_boxed_slice();
    let ptr = Box::into_raw(boxed) as *mut u8;
    ptr as usize
}

/// Reads back one leaked name buffer using its known byte length, which
/// excludes the trailing NUL written by [`leak_nul_terminated`].
fn swap_text(ptr: usize, len: usize) -> String {
    // SAFETY: `install_name_swap` leaked a buffer of `len + 1` bytes at `ptr`,
    // with `len` printable ASCII bytes followed by a NUL.
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    String::from_utf8_lossy(bytes).into_owned()
}

/// Stores the parsed name swap in process-lifetime storage. Called once from
/// `configure` after the watch is armed, so the buffers outlive the title.
fn install_name_swap(parsed: Option<(String, String)>) {
    let Some((from_text, to_text)) = parsed else {
        return;
    };
    let from = leak_nul_terminated(&from_text);
    let to = leak_nul_terminated(&to_text);
    let _ = NAME_SWAP.set(NameSwap {
        from,
        from_len: from_text.len(),
        to,
        to_len: to_text.len(),
    });
}

/// The VA the presets shim patched, or `None` when it was never armed. Used
/// by the crash handler to leave the shim's own single-step to this VEH.
pub(crate) fn shim_watch_va() -> Option<usize> {
    let state = presets();
    (state.va != 0).then_some(state.va)
}

/// Builds the filesystem URI the title reads blob bytes from. Pure, for testing.
/// The trailing eight hex digits mirror the declared size.
fn format_presets_uri(address: usize, size: usize) -> String {
    format!("memory:${address:016X},{size},0:{size:08X}")
}

/// Reads back a URI built by [`format_presets_uri`]. Pure, for testing.
/// Returns `(address, size, suffix)` so the new suffix can be round-tripped.
fn parse_presets_uri(text: &str) -> Option<(usize, usize, usize)> {
    let rest = text.strip_prefix("memory:$")?;
    let (hex, rest) = rest.split_once(',')?;
    let (size, rest) = rest.split_once(',')?;
    let (kind, suffix) = rest.split_once(':')?;
    if hex.len() != 16 || kind != "0" || suffix.len() != 8 {
        return None;
    }
    let address = usize::from_str_radix(hex, 16).ok()?;
    let suffix = usize::from_str_radix(suffix, 16).ok()?;
    Some((address, size.parse::<usize>().ok()?, suffix))
}

/// True when `text` contains `presets.zmt` in any ASCII letter case.
fn contains_presets_zmt(text: &str) -> bool {
    const NEEDLE: &[u8] = b"presets.zmt";
    text.as_bytes()
        .windows(NEEDLE.len())
        .any(|window| window.eq_ignore_ascii_case(NEEDLE))
}

/// True only when `VICTORY_SKIP_ASSERT` is exactly `"1"`. Pure, for testing.
fn skip_assert_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// True only when `VICTORY_SKIP_PRESETS_ASSERT` is exactly `"1"`. Pure, for testing.
fn skip_presets_assert_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// True only when `VICTORY_WATCH_QWORDS` is exactly `"1"`. Pure, for testing.
fn qwords_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// True only when `VICTORY_FIX_PRESETS_ASSERT` is exactly `"1"`. Pure, for testing.
fn fix_presets_assert_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// True only when `VICTORY_NO_STEP` is exactly `"1"`. Pure, for testing.
fn no_step_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// True only when `VICTORY_DUMP_MEMORY_URI` is exactly `"1"`. Pure, for testing.
fn memory_uri_dump_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Parses a title `memory:$%016X,<decimal>,...` URI into `(address, size)`.
/// The suffix after the second comma is ignored because the title emits a
/// trailing `0:` with no marker bytes. Pure, for testing.
fn parse_memory_uri(text: &str) -> Option<(usize, usize)> {
    let rest = text.strip_prefix("memory:$")?;
    let (hex, rest) = rest.split_once(',')?;
    if hex.len() != 16 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let address = usize::from_str_radix(hex, 16).ok()?;
    let size_text = rest.split_once(',').map_or(rest, |(size, _)| size);
    if size_text.is_empty() || !size_text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((address, size_text.parse::<usize>().ok()?))
}

/// True when a `memory:$` payload size is within the dump window.
/// Pure, for testing.
fn memory_uri_size_allowed(size: usize) -> bool {
    (1..=MEMURI_MAX_SIZE).contains(&size)
}

/// The dump index to use when `used` dumps have already been written.
/// Pure, for testing.
fn next_dump_index(used: usize) -> Option<usize> {
    (used < MEMURI_MAX_DUMPS).then_some(used)
}

/// Parses `VICTORY_NAME_SWAP=from:to` into its two names. Pure, for testing.
/// Both sides must be non-empty printable ASCII with no spaces, and exactly
/// one `:` must separate them. Any other value yields `None`.
fn parse_name_swap(value: Option<&str>) -> Option<(String, String)> {
    let value = value?;
    let (from, to) = value.split_once(':')?;
    let valid = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_graphic());
    if !valid(from) || !valid(to) || to.contains(':') {
        return None;
    }
    Some((from.to_owned(), to.to_owned()))
}

/// Returns the corrected declared pool size, or `None` when no correction
/// applies. At the assert, `current_rax` holds `size - ptr + base`, so the
/// amount above `target` is the excess already consumed from the pool. That
/// excess is removed from the declared size at `[rsp+0x78]`. Returns `None`
/// when RAX is not above the target or the subtraction would underflow.
/// Pure, for testing.
fn corrected_assert_size(current_size: u64, current_rax: u64, target: u64) -> Option<u64> {
    if current_rax <= target {
        return None;
    }
    let excess = current_rax - target;
    current_size.checked_sub(excess)
}

/// Parses the two environment values and resolves the watched VA against
/// `base`. Returns `(VA, log count)`, or `None` when the RVA is absent or
/// malformed, the count is zero, or there is no image base. Pure, for testing.
fn parse_watch(
    rva: Option<&str>,
    count: Option<&str>,
    base: Option<usize>,
) -> Option<(usize, usize)> {
    let text = rva?.trim();
    let (digits, radix) = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(rest) => (rest, 16),
        None => (text, 10),
    };
    let offset = usize::from_str_radix(digits, radix).ok()?;
    let left = match count {
        Some(c) => c.trim().parse::<usize>().ok()?,
        None => 1,
    };
    if left == 0 {
        return None;
    }
    let va = base?.checked_add(offset)?;
    Some((va, left))
}

/// The committed, readable region containing `at`, as `(base, size)`.
fn region(at: usize) -> Option<(usize, usize)> {
    // SAFETY: a zeroed information block is valid for the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size; `at` is only queried.
    let found = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) } != 0;
    if !found || info.State != MEM_COMMIT || info.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
        return None;
    }
    let base = info.BaseAddress as usize;
    let size = info.RegionSize;
    (at >= base && at - base < size).then_some((base, size))
}

/// True when `[at, at + len)` lies wholly inside committed, readable regions.
/// Uses the same guarded `VirtualQuery` check as [`region`], applied to every
/// region the range crosses.
fn readable_range(at: usize, len: usize) -> bool {
    if at == 0 || len == 0 {
        return false;
    }
    let Some(end) = at.checked_add(len) else {
        return false;
    };
    let mut cursor = at;
    while cursor < end {
        let Some((base, size)) = region(cursor) else {
            return false;
        };
        let region_end = base.saturating_add(size);
        if region_end <= cursor {
            return false;
        }
        cursor = region_end.min(end);
    }
    true
}

/// The byte at `at`, if its page is committed and readable.
fn read_byte(at: usize) -> Option<u8> {
    region(at)?;
    // SAFETY: `region` confirmed `at` is committed and readable.
    Some(unsafe { (at as *const u8).read() })
}

/// Makes the page containing `at` writable, returning its old protection.
///
/// # Safety
/// `at`'s page must be committed; the caller restores the returned protection.
unsafe fn make_writable(at: usize) -> Option<PAGE_PROTECTION_FLAGS> {
    region(at)?;
    let page = at & !(PAGE_SIZE - 1);
    let mut old = 0;
    // SAFETY: a committed page and a valid out pointer.
    let ok = unsafe {
        VirtualProtect(
            page as *const _,
            PAGE_SIZE,
            PAGE_EXECUTE_READWRITE,
            &mut old,
        )
    } != 0;
    ok.then_some(old)
}

/// Puts the page containing `at` back to `old`.
///
/// # Safety
/// `at`'s page must be the one [`make_writable`] changed.
unsafe fn restore_page(at: usize, old: PAGE_PROTECTION_FLAGS) {
    let page = at & !(PAGE_SIZE - 1);
    let mut ignored = 0;
    // SAFETY: the page was made writable by `make_writable`.
    unsafe { VirtualProtect(page as *const _, PAGE_SIZE, old, &mut ignored) };
}

/// Writes a byte at `at` and flushes the instruction cache there.
///
/// # Safety
/// `at`'s page must be writable.
unsafe fn write_byte(at: usize, byte: u8) {
    // SAFETY: the caller made `at`'s page writable.
    unsafe { (at as *mut u8).write(byte) };
    // SAFETY: a pseudo-handle for this process and the one changed byte.
    unsafe { FlushInstructionCache(GetCurrentProcess(), at as *const _, 1) };
}

/// Registers the one vectored handler shared by the control watch and the
/// presets shim. Both arming paths call it, so at most one handler exists.
fn ensure_handler() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: registers a handler that touches only exceptions at the
        // addresses armed by `configure` or `configure_presets`.
        // Registered at index 0 so ours runs before the loader's crash handler.
        let _ = unsafe { AddVectoredExceptionHandler(0, Some(handler)) };
    });
}

/// Enables the watch from the environment, using the loaded title image's
/// `base`. Called once from [`crate::load`] before title code runs. A set but
/// unusable `VICTORY_WATCH_RVA` is reported and skipped.
pub(crate) fn configure(base: Option<usize>) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Some(rva) = std::env::var_os("VICTORY_WATCH_RVA") else {
            return;
        };
        let rva = rva.to_string_lossy();
        let count = std::env::var("VICTORY_WATCH_COUNT").ok();
        let swap_value = std::env::var("VICTORY_NAME_SWAP").ok();
        let name_swap = parse_name_swap(swap_value.as_deref());
        let skip_value = std::env::var("VICTORY_SKIP_ASSERT").ok();
        let skip_assert = skip_assert_enabled(skip_value.as_deref());
        let fix_value = std::env::var("VICTORY_FIX_PRESETS_ASSERT").ok();
        let Some((va, left)) = parse_watch(Some(&rva), count.as_deref(), base) else {
            tracing::warn!(
                rva = %rva,
                "VICTORY_WATCH_RVA: no image base or bad RVA/COUNT; watch disabled"
            );
            return;
        };
        let fix_assert = fix_presets_assert_enabled(fix_value.as_deref())
            && base.and_then(|b| b.checked_add(PRESETS_ASSERT_INT3_RVA)) == Some(va);
        let Some(original) = read_byte(va) else {
            tracing::warn!(
                at = format_args!("{va:#x}"),
                "VICTORY_WATCH_RVA: code byte unavailable; watch disabled"
            );
            return;
        };
        // SAFETY: `read_byte` confirmed the page is committed.
        let Some(old_protect) = (unsafe { make_writable(va) }) else {
            tracing::warn!(
                at = format_args!("{va:#x}"),
                "VICTORY_WATCH_RVA: page not writable; watch disabled"
            );
            return;
        };
        // SAFETY: the page was just made writable.
        unsafe { write_byte(va, 0xcc) };
        {
            let mut state = watch();
            state.va = va;
            state.original = original;
            state.old_protect = old_protect;
            state.remaining = left;
            state.armed = true;
            state.stepping = false;
            state.skip_assert = skip_assert;
            state.fix_assert = fix_assert;
        }
        install_name_swap(name_swap);
        ensure_handler();
        tracing::info!(
            at = format_args!("{va:#x}"),
            count = left,
            "watching title address (CPU context logged on hit)"
        );
    });
}

/// Patches the title's pool-size assert into an unconditional jump. Uses the
/// process-lifetime shim: the byte is not restored and no handler is added.
///
/// # Safety
/// `assert_va` must name committed, writable title code.
unsafe fn disable_presets_pool_assert(assert_va: usize) {
    // SAFETY: `make_writable` confirms the page is committed.
    let Some(_old_protect) = (unsafe { make_writable(assert_va) }) else {
        tracing::warn!(
            at = format_args!("{assert_va:#x}"),
            "VICTORY_SKIP_PRESETS_ASSERT: page not writable; assert left unchanged"
        );
        return;
    };
    // SAFETY: the page was just made writable.
    let original = unsafe { (assert_va as *const u8).read() };
    if original != PRESETS_ASSERT_JBE {
        tracing::warn!(
            at = format_args!("{assert_va:#x}"),
            original = format_args!("{original:#04x}"),
            "VICTORY_SKIP_PRESETS_ASSERT: unexpected byte; assert left unchanged"
        );
        return;
    }
    // SAFETY: the page was just made writable.
    unsafe { write_byte(assert_va, PRESETS_ASSERT_JMP) };
    tracing::info!(
        at = format_args!("{assert_va:#x}"),
        original = format_args!("{original:#04x}"),
        "presets pool assert disabled"
    );
}

/// Arms the presets compatibility shim from `VICTORY_PRESETS_BLOB`. Called at
/// the same point as [`configure`]: once, before title code runs. Without the
/// variable, or on any read or allocation failure, it warns and stays inert.
/// Both allocations are kept alive for the process lifetime.
pub(crate) fn configure_presets(base: Option<usize>) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Some(path) = std::env::var_os("VICTORY_PRESETS_BLOB") else {
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) if !bytes.is_empty() => bytes,
            Ok(_) => {
                tracing::warn!(
                    path = %path.to_string_lossy(),
                    "VICTORY_PRESETS_BLOB: empty blob; shim disabled"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    path = %path.to_string_lossy(),
                    %error,
                    "VICTORY_PRESETS_BLOB: cannot read blob; shim disabled"
                );
                return;
            }
        };
        if bytes.len() > PRESETS_POOL_SIZE {
            tracing::warn!(
                path = %path.to_string_lossy(),
                len = bytes.len(),
                pool = PRESETS_POOL_SIZE,
                "VICTORY_PRESETS_BLOB: blob larger than pool; shim disabled"
            );
            return;
        }
        let Some(base) = base else {
            tracing::warn!("VICTORY_PRESETS_BLOB: no image base; shim disabled");
            return;
        };
        let Some(va) = base.checked_add(PRESETS_RVA) else {
            tracing::warn!("VICTORY_PRESETS_BLOB: RVA overflow; shim disabled");
            return;
        };
        // SAFETY: a null address lets the system pick the region; neither
        // allocation is freed, so both stay alive for the process lifetime.
        // VirtualAlloc zeroes committed pages, so bytes past the blob stay
        // zero. The pool size is already page-aligned.
        let blob = unsafe {
            VirtualAlloc(
                std::ptr::null_mut(),
                PRESETS_POOL_SIZE,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if blob.is_null() {
            tracing::warn!(
                size = PRESETS_POOL_SIZE,
                "VICTORY_PRESETS_BLOB: pool allocation failed; shim disabled"
            );
            return;
        }
        // SAFETY: `blob` is a fresh writable region of `PRESETS_POOL_SIZE` bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), blob.cast::<u8>(), bytes.len()) };

        let uri_text = format_presets_uri(blob as usize, PRESETS_URI_SIZE);
        if parse_presets_uri(&uri_text) != Some((blob as usize, PRESETS_URI_SIZE, PRESETS_URI_SIZE))
        {
            tracing::warn!("VICTORY_PRESETS_BLOB: URI round-trip failed; shim disabled");
            return;
        }
        // SAFETY: a null address and enough bytes for the text plus its NUL.
        let uri = unsafe {
            VirtualAlloc(
                std::ptr::null_mut(),
                uri_text.len() + 1,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if uri.is_null() {
            tracing::warn!("VICTORY_PRESETS_BLOB: URI allocation failed; shim disabled");
            return;
        }
        // SAFETY: `uri` is a fresh writable page sized for `uri_text` and its NUL.
        unsafe {
            std::ptr::copy_nonoverlapping(uri_text.as_ptr(), uri.cast::<u8>(), uri_text.len());
            uri.cast::<u8>().add(uri_text.len()).write(0);
        }

        let Some(original) = read_byte(va) else {
            tracing::warn!(
                at = format_args!("{va:#x}"),
                "VICTORY_PRESETS_BLOB: code byte unavailable; shim disabled"
            );
            return;
        };
        // SAFETY: `read_byte` confirmed the page is committed.
        let Some(old_protect) = (unsafe { make_writable(va) }) else {
            tracing::warn!(
                at = format_args!("{va:#x}"),
                "VICTORY_PRESETS_BLOB: page not writable; shim disabled"
            );
            return;
        };
        // SAFETY: the page was just made writable.
        unsafe { write_byte(va, 0xcc) };
        {
            let mut state = presets();
            state.va = va;
            state.original = original;
            state.old_protect = old_protect;
            state.remaining = PRESETS_HIT_BUDGET;
            state.armed = true;
            state.stepping = false;
            state.blob = blob as usize;
            state.blob_len = bytes.len();
            state.uri = uri as usize;
            state.uri_text = uri_text.clone();
        }
        let skip_presets_assert = std::env::var("VICTORY_SKIP_PRESETS_ASSERT").ok();
        if skip_presets_assert_enabled(skip_presets_assert.as_deref()) {
            if let Some(assert_va) = base.checked_add(PRESETS_ASSERT_RVA) {
                // SAFETY: the assert RVA names committed title code.
                unsafe { disable_presets_pool_assert(assert_va) };
            } else {
                tracing::warn!("VICTORY_SKIP_PRESETS_ASSERT: RVA overflow; assert left unchanged");
            }
        }
        ensure_handler();
        tracing::info!(
            path = %path.to_string_lossy(),
            at = format_args!("{va:#x}"),
            blob = format_args!("{:#x}", blob as usize),
            blob_len = bytes.len(),
            pool_size = PRESETS_POOL_SIZE,
            uri_size = PRESETS_URI_SIZE,
            uri = %uri_text,
            "presets compatibility shim armed"
        );
    });
}

/// Applies the opt-in pool-assert correction after the watch has logged the
/// hit. Returns `true` when the correction was applied and the hit must be
/// consumed without a single-step or re-arm.
///
/// The declared size is the qword at `[rsp+0x78]` and RAX holds `size - ptr +
/// base`. When RAX exceeds [`PRESETS_ASSERT_SIZE_TARGET`], the excess is
/// written back to the declared size, RAX is set to the target and execution
/// resumes past the `int3`. A failed read, write or arithmetic leaves
/// everything unchanged.
fn apply_presets_assert_fix(va: usize, enabled: bool, context: &mut CONTEXT) -> bool {
    if !enabled {
        return false;
    }
    let Some(slot) = (context.Rsp as usize).checked_add(0x78) else {
        return false;
    };
    let current_size = read_u64(slot);
    let Some(new_size) =
        corrected_assert_size(current_size, context.Rax, PRESETS_ASSERT_SIZE_TARGET)
    else {
        return false;
    };
    if !write_u64(slot, new_size) {
        return false;
    }
    let old_rax = context.Rax;
    context.Rax = PRESETS_ASSERT_SIZE_TARGET;
    tracing::info!(
        at = format_args!("{va:#x}"),
        old_size = format_args!("{current_size:#x}"),
        new_size = format_args!("{new_size:#x}"),
        old_rax = format_args!("{old_rax:#x}"),
        new_rax = format_args!("{:#x}", PRESETS_ASSERT_SIZE_TARGET),
        "presets assert size corrected"
    );
    context.Rip = (va as u64).wrapping_add(1);
    {
        let mut state = watch();
        state.stepping = false;
        state.remaining = state.remaining.saturating_sub(1);
        if state.remaining == 0 {
            // SAFETY: the same page `configure` changed.
            unsafe { restore_page(va, state.old_protect) };
        }
    }
    true
}

/// Applies the opt-in name swap at a watch hit. The current name at RCX is
/// read with [`read_string`] and compared case-sensitively with `from`. On a
/// match, RCX is pointed at the process-lifetime `to` buffer and exactly one
/// event is logged. No other register is written.
fn apply_name_swap(context: &mut CONTEXT) {
    let Some(swap) = NAME_SWAP.get() else {
        return;
    };
    let current = read_string(context.Rcx as usize);
    let from = swap_text(swap.from, swap.from_len);
    if current != from {
        return;
    }
    let to = swap_text(swap.to, swap.to_len);
    tracing::info!(from = %from, to = %to, "interner name swapped");
    context.Rcx = swap.to as u64;
}

/// Dumps the real bytes behind a `memory:$` URI seen at RCX or RDX. The first
/// string that parses as such a URI wins; writes stop at the eight-dump cap.
/// Called only when `VICTORY_DUMP_MEMORY_URI=1` alongside an armed watch.
fn maybe_dump_memory_uri(context: &CONTEXT) {
    let rcx = read_string(context.Rcx as usize);
    let rdx = read_string(context.Rdx as usize);
    let Some((uri, address, size)) = [rcx.as_str(), rdx.as_str()]
        .into_iter()
        .find_map(|text| parse_memory_uri(text).map(|(address, size)| (text, address, size)))
    else {
        return;
    };
    if !memory_uri_size_allowed(size) || !readable_range(address, size) {
        return;
    }
    let Some(index) = reserve_dump_slot() else {
        return;
    };
    let file = format!("{MEMURI_DUMP_DIR}\\memuri-{index}.bin");
    // SAFETY: `readable_range` confirmed `size` readable bytes at `address`.
    let bytes = unsafe { std::slice::from_raw_parts(address as *const u8, size) };
    if let Err(error) = std::fs::write(&file, bytes) {
        tracing::warn!(file = %file, %error, "memory uri dump failed");
        return;
    }
    tracing::info!(path = %uri, size, file = %file, "dumped memory uri");
}

/// Consumes one `int3` hit: logs, unpatches, and requests one single-step.
///
/// With `VICTORY_SKIP_ASSERT=1`, a saved `0xcc` marks a genuine title `int3`
/// rather than our patch: the hit is consumed, but the byte, the trap flag and
/// re-arming are left untouched and RIP moves past the instruction.
///
/// With `VICTORY_NO_STEP=1`, the hit is still logged and the assert correction
/// still runs, but RIP moves to `va + 1` past the genuine title `int3` and no
/// trap flag or step owner is set, so no re-arm follows.
fn on_breakpoint(context: &mut CONTEXT) -> i32 {
    let no_step = no_step_enabled(std::env::var("VICTORY_NO_STEP").ok().as_deref());
    let (va, original, skip, fix_assert) = {
        let mut state = watch();
        if state.va == 0 || !state.armed || state.remaining == 0 {
            return CONTINUE_SEARCH;
        }
        let watched = state.va as u64;
        if context.Rip != watched && context.Rip != watched.wrapping_add(1) {
            return CONTINUE_SEARCH;
        }
        let skip = state.skip_assert && state.original == 0xcc;
        state.armed = false;
        state.stepping = !skip && !no_step;
        (state.va, state.original, skip, state.fix_assert)
    };
    if skip {
        let mut state = watch();
        state.remaining = state.remaining.saturating_sub(1);
        if state.remaining == 0 {
            // SAFETY: the same page `configure` changed.
            unsafe { restore_page(va, state.old_protect) };
        }
        tracing::info!(
            at = format_args!("{va:#x}"),
            rip = format_args!("{:#x}", context.Rip),
            rcx = format_args!("{:#x}", context.Rcx),
            rdx = format_args!("{:#x}", context.Rdx),
            r8 = format_args!("{:#x}", context.R8),
            r9 = format_args!("{:#x}", context.R9),
            "skipping title int3"
        );
        context.Rip = (va as u64).wrapping_add(1);
        return CONTINUE_EXECUTION;
    }
    // SAFETY: `configure` made the page writable and no last hit restored it.
    unsafe { write_byte(va, original) };
    log(va, context);
    let dump_value = std::env::var("VICTORY_DUMP_MEMORY_URI").ok();
    if memory_uri_dump_enabled(dump_value.as_deref()) {
        maybe_dump_memory_uri(context);
    }
    apply_name_swap(context);
    if apply_presets_assert_fix(va, fix_assert, context) {
        return CONTINUE_EXECUTION;
    }
    let mut state = watch();
    state.remaining = state.remaining.saturating_sub(1);
    if state.remaining == 0 {
        // SAFETY: the same page `configure` changed.
        unsafe { restore_page(va, state.old_protect) };
    }
    context.Rip = if no_step {
        (va as u64).wrapping_add(1)
    } else {
        va as u64
    };
    if !no_step {
        context.EFlags |= TRAP_FLAG;
        set_step_owner(PatchKind::Watch);
    }
    CONTINUE_EXECUTION
}

/// Consumes the single-step requested after a hit. Any RIP is accepted: the
/// reported step may land past the rewound instruction rather than at `va`.
/// Clears the pending step and trap flag, then re-arms while hits remain; the
/// last hit leaves the watch unarmed.
fn on_single_step(context: &mut CONTEXT) -> i32 {
    let mut state = watch();
    if state.va == 0 || state.armed || !state.stepping {
        return CONTINUE_SEARCH;
    }
    state.stepping = false;
    context.EFlags &= !TRAP_FLAG;
    if state.remaining > 0 {
        // SAFETY: the page stays writable while hits remain.
        unsafe { write_byte(state.va, 0xcc) };
        state.armed = true;
    }
    CONTINUE_EXECUTION
}

/// Consumes one shim `int3` hit: swaps RDX for the blob URI when the title is
/// opening `presets.zmt`, restores the byte, and requests one single-step.
/// This path is unaffected by `VICTORY_NO_STEP`.
fn on_presets_breakpoint(context: &mut CONTEXT) -> i32 {
    let (va, original, uri, uri_text) = {
        let mut state = presets();
        if state.va == 0 || state.remaining == 0 {
            return CONTINUE_SEARCH;
        }
        let watched = state.va as u64;
        if context.Rip != watched && context.Rip != watched.wrapping_add(1) {
            return CONTINUE_SEARCH;
        }
        state.armed = false;
        state.stepping = true;
        (state.va, state.original, state.uri, state.uri_text.clone())
    };
    // SAFETY: `configure_presets` made the page writable and the last hit has not restored it.
    unsafe { write_byte(va, original) };

    let path = read_string(context.Rdx as usize);
    if contains_presets_zmt(&path) {
        tracing::info!(
            path = %path,
            uri = %uri_text,
            "redirecting presets open to emulator blob"
        );
        // SAFETY: the system supplied the faulting thread's writable context.
        context.Rdx = uri as u64;
    }

    let mut state = presets();
    state.remaining = state.remaining.saturating_sub(1);
    if state.remaining == 0 {
        // SAFETY: the same page `configure_presets` changed.
        unsafe { restore_page(va, state.old_protect) };
    }
    context.Rip = va as u64;
    context.EFlags |= TRAP_FLAG;
    set_step_owner(PatchKind::Presets);
    CONTINUE_EXECUTION
}

/// Consumes the single-step after a shim hit. Any RIP is accepted: the
/// reported step may land past the rewound instruction rather than at `va`.
fn on_presets_single_step(context: &mut CONTEXT) -> i32 {
    let mut state = presets();
    if state.va == 0 || !state.stepping {
        return CONTINUE_SEARCH;
    }
    state.stepping = false;
    context.EFlags &= !TRAP_FLAG;
    if state.remaining > 0 {
        // SAFETY: the page stays writable while hits remain.
        unsafe { write_byte(state.va, 0xcc) };
        state.armed = true;
    }
    CONTINUE_EXECUTION
}

/// Handles only our `int3` and the single-step after it, leaving every other
/// exception to the existing handlers.
///
/// # Safety
/// `info` comes from the system, for an exception being dispatched.
unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    // SAFETY: the system passes the exception being dispatched.
    unsafe {
        let code = (*(*info).ExceptionRecord).ExceptionCode;
        // SAFETY: the system passes the faulting thread's context.
        let context = &mut *(*info).ContextRecord;

        if matches!(code, EXCEPTION_BREAKPOINT | STATUS_SINGLE_STEP) {
            let configured = watch().va != 0;
            if configured {
                tracing::info!(
                    code = format_args!("{code:#x}"),
                    rip = format_args!("{:#x}", context.Rip),
                    "watch exception dispatched"
                );
            }
        }

        match code {
            EXCEPTION_BREAKPOINT => {
                let handled = on_breakpoint(context);
                if handled == CONTINUE_SEARCH {
                    on_presets_breakpoint(context)
                } else {
                    handled
                }
            }
            STATUS_SINGLE_STEP => match take_step_owner() {
                Some(PatchKind::Presets) => on_presets_single_step(context),
                Some(PatchKind::Watch) => on_single_step(context),
                None => {
                    // With the presets shim armed, unowned single-steps are
                    // swallowed here rather than left to become unhandled
                    // 0x80000004 faults at unrelated addresses. The trap flag
                    // is deliberately left untouched.
                    if presets().va != 0 {
                        CONTINUE_EXECUTION
                    } else {
                        CONTINUE_SEARCH
                    }
                }
            },
            _ => CONTINUE_SEARCH,
        }
    }
}

/// Logs the context at the hit, with bounded reads of the strings at RCX and
/// RDX.
fn log(at: usize, context: &CONTEXT) {
    // SAFETY: a plain query of the calling thread, the one that reached `at`.
    let thread = unsafe { GetCurrentThreadId() };
    let rcx_string = read_string(context.Rcx as usize);
    let rdx_string = read_string(context.Rdx as usize);
    tracing::info!(
        thread,
        at = format_args!("{at:#x}"),
        rip = format_args!("{:#x}", context.Rip),
        rax = format_args!("{:#x}", context.Rax),
        rcx = format_args!("{:#x}", context.Rcx),
        rdx = format_args!("{:#x}", context.Rdx),
        r8 = format_args!("{:#x}", context.R8),
        r9 = format_args!("{:#x}", context.R9),
        rcx_string = %rcx_string,
        rdx_string = %rdx_string,
        "watch hit"
    );
    if qwords_enabled(std::env::var("VICTORY_WATCH_QWORDS").ok().as_deref()) {
        let rsp = context.Rsp as usize;
        let words: [u64; QWORD_OFFSETS.len()] =
            std::array::from_fn(|i| read_u64(rsp.wrapping_add(QWORD_OFFSETS[i])));
        tracing::info!(
            thread,
            at = format_args!("{at:#x}"),
            rsp = format_args!("{:#x}", context.Rsp),
            rsp_70 = format_args!("{:#x}", words[0]),
            rsp_78 = format_args!("{:#x}", words[1]),
            rsp_80 = format_args!("{:#x}", words[2]),
            rsp_88 = format_args!("{:#x}", words[3]),
            "watch hit qwords"
        );
    }
}

/// Best-effort read of one `u64` at `at`. Uses the same `VirtualQuery` guard as
/// [`read_string`] and returns 0 when all eight bytes are not readable.
fn read_u64(at: usize) -> u64 {
    if at == 0 {
        return 0;
    }
    // SAFETY: a zeroed information block is valid for the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size; `at` is only queried.
    let found = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) } != 0;
    if !found || info.State != MEM_COMMIT || info.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
        return 0;
    }
    let region_end = (info.BaseAddress as usize).saturating_add(info.RegionSize);
    if region_end.saturating_sub(at) < size_of::<u64>() {
        return 0;
    }
    // SAFETY: `VirtualQuery` confirmed eight readable bytes from `at`. The
    // address may be unaligned on x86-64, so `read_unaligned` is used.
    unsafe { std::ptr::read_unaligned(at as *const u64) }
}

/// Best-effort write of one `u64` at `at`, guarded like [`read_u64`]. The page
/// is made writable for the write and its protection is restored afterwards.
/// Returns `false` when the qword is not committed, is guarded, or the page
/// cannot be made writable.
fn write_u64(at: usize, value: u64) -> bool {
    if at == 0 {
        return false;
    }
    // SAFETY: a zeroed information block is valid for the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size; `at` is only queried.
    let found = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) } != 0;
    if !found || info.State != MEM_COMMIT || info.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
        return false;
    }
    let region_end = (info.BaseAddress as usize).saturating_add(info.RegionSize);
    if region_end.saturating_sub(at) < size_of::<u64>() {
        return false;
    }
    let page = at & !(PAGE_SIZE - 1);
    let mut old = 0;
    // SAFETY: a committed page and a valid out pointer.
    let ok = unsafe { VirtualProtect(page as *const _, PAGE_SIZE, PAGE_READWRITE, &mut old) } != 0;
    if !ok {
        return false;
    }
    // SAFETY: `VirtualProtect` made the page writable and the query confirmed
    // eight bytes in the region. The address may be unaligned on x86-64.
    unsafe { std::ptr::write_unaligned(at as *mut u64, value) };
    // SAFETY: the page was made writable by the `VirtualProtect` above.
    unsafe { restore_page(page, old) };
    true
}

/// Best-effort read of up to [`MAX_STRING_BYTES`] from `at`, stopping at NUL.
/// Never reads past the committed region or guarded memory: `VirtualQuery`
/// bounds the slice to what is actually readable.
fn read_string(at: usize) -> String {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    };
    if at == 0 {
        return String::new();
    }
    // SAFETY: a zeroed information block is valid for the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size; `at` is only queried.
    let found = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) } != 0;
    if !found || info.State != MEM_COMMIT || info.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
        return String::new();
    }
    let region_end = (info.BaseAddress as usize).saturating_add(info.RegionSize);
    let len = region_end.saturating_sub(at).min(MAX_STRING_BYTES);
    if len == 0 {
        return String::new();
    }
    // SAFETY: `VirtualQuery` confirmed `len` bytes from `at` are committed
    // and readable; nothing writes them, and the slice is decoded at once.
    let bytes = unsafe { std::slice::from_raw_parts(at as *const u8, len) };
    decode_string(bytes)
}

/// Decodes a bounded byte slice as a NUL-terminated UTF-8 or UTF-16LE string.
/// A zero first or second byte names UTF-16LE, otherwise UTF-8 is used.
fn decode_string(bytes: &[u8]) -> String {
    if bytes.first() == Some(&0) || bytes.get(1) == Some(&0) {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|&unit| unit != 0)
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..len]).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_watch_resolves_hex_and_decimal_rvas() {
        assert_eq!(
            parse_watch(Some("0x1234"), None, Some(0x1_0000)),
            Some((0x1_1234, 1))
        );
        assert_eq!(
            parse_watch(Some("4660"), Some("3"), Some(0x1_0000)),
            Some((0x1_1234, 3))
        );
        assert_eq!(
            parse_watch(Some(" 0X10 "), Some(" 2 "), Some(0x2000)),
            Some((0x2010, 2))
        );
    }

    #[test]
    fn parse_watch_rejects_unusable_values() {
        assert_eq!(parse_watch(None, None, Some(0)), None);
        assert_eq!(parse_watch(Some(""), None, Some(0)), None);
        assert_eq!(parse_watch(Some("xyz"), None, Some(0)), None);
        assert_eq!(parse_watch(Some("0x10"), Some("0"), Some(0)), None);
        assert_eq!(parse_watch(Some("0x10"), Some("x"), Some(0)), None);
        assert_eq!(parse_watch(Some("0x10"), None, None), None);
        assert_eq!(parse_watch(Some("0x10"), None, Some(usize::MAX)), None);
    }

    #[test]
    fn decode_string_reads_utf8_and_utf16() {
        let mut utf8 = [0u8; MAX_STRING_BYTES];
        utf8[..5].copy_from_slice(b"hello");
        assert_eq!(decode_string(&utf8), "hello");
        let mut utf16 = [0u8; MAX_STRING_BYTES];
        for (i, unit) in "hi".encode_utf16().enumerate() {
            utf16[i * 2..i * 2 + 2].copy_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_string(&utf16), "hi");
        assert_eq!(decode_string(&[0x41, 0, 0, 0]), "A");
    }

    #[test]
    fn presets_uri_formats_and_parses() {
        let address = 0x0123_4567_89AB_CDEF;
        let uri = format_presets_uri(address, PRESETS_URI_SIZE);
        assert_eq!(
            uri,
            format!("memory:${address:016X},{PRESETS_URI_SIZE},0:{PRESETS_URI_SIZE:08X}")
        );
        assert_eq!(
            parse_presets_uri(&uri),
            Some((address, PRESETS_URI_SIZE, PRESETS_URI_SIZE))
        );
    }

    #[test]
    fn presets_allocation_and_uri_sizes_are_distinct() {
        assert_eq!(PRESETS_POOL_SIZE, 0x4000000);
        assert_eq!(PRESETS_URI_SIZE, 0x11E0000);
        const {
            assert!(PRESETS_URI_SIZE < PRESETS_POOL_SIZE);
        };
    }

    #[test]
    fn presets_uri_round_trips_the_suffix() {
        let uri = format_presets_uri(1, 160);
        assert_eq!(uri, "memory:$0000000000000001,160,0:000000A0");
        assert_eq!(parse_presets_uri(&uri), Some((1, 160, 160)));
    }

    #[test]
    fn presets_uri_rejects_malformed_text() {
        assert_eq!(parse_presets_uri(""), None);
        assert_eq!(parse_presets_uri("memory:$1,160,0:000000A0"), None);
        assert_eq!(
            parse_presets_uri("memory:$0000000000000001,160,1:000000A0"),
            None
        );
        assert_eq!(
            parse_presets_uri("memory:$0000000000000001,x,0:000000A0"),
            None
        );
        assert_eq!(
            parse_presets_uri("memory:$0000000000000001,160,0:000000A"),
            None
        );
        assert_eq!(
            parse_presets_uri("memory:$0000000000000001,160,0:000000AG"),
            None
        );
        assert_eq!(
            parse_presets_uri("platform:/data/settings/presets.zmt"),
            None
        );
    }

    #[test]
    fn presets_path_match_is_ascii_case_insensitive_anywhere() {
        assert!(contains_presets_zmt("platform:/data/settings/presets.zmt"));
        assert!(contains_presets_zmt("PLATFORM:/DATA/SETTINGS/PRESETS.ZMT"));
        assert!(contains_presets_zmt(
            "platform:/data/lighting/photomodelightpresets.zmt"
        ));
        assert!(contains_presets_zmt(
            "PLATFORM:/DATA/LIGHTING/PHOTOMODELIGHTPRESETS.ZMT"
        ));
        assert!(contains_presets_zmt(
            "G:platform:/data/settings/presets.zmt"
        ));
        assert!(contains_presets_zmt(
            "d:PLATFORM:/data/settings/PRESETS.ZMT"
        ));
        assert!(contains_presets_zmt(
            "prefix:platform:/data/settings/presets.zmt"
        ));
        assert!(contains_presets_zmt(
            "platform:/data/settings/presets.zmt.bak"
        ));
        assert!(!contains_presets_zmt("platform:/data/settings/other.zmt"));
        assert!(!contains_presets_zmt(""));
        assert!(!contains_presets_zmt("presets.zm"));
    }

    #[test]
    fn skip_assert_requires_exactly_one() {
        assert!(skip_assert_enabled(Some("1")));
        assert!(!skip_assert_enabled(Some(" 1")));
        assert!(!skip_assert_enabled(Some("01")));
        assert!(!skip_assert_enabled(Some("true")));
        assert!(!skip_assert_enabled(Some("0")));
        assert!(!skip_assert_enabled(None));
    }

    #[test]
    fn skip_presets_assert_requires_exactly_one() {
        assert!(skip_presets_assert_enabled(Some("1")));
        assert!(!skip_presets_assert_enabled(Some(" 1")));
        assert!(!skip_presets_assert_enabled(Some("01")));
        assert!(!skip_presets_assert_enabled(Some("true")));
        assert!(!skip_presets_assert_enabled(Some("0")));
        assert!(!skip_presets_assert_enabled(None));
    }

    #[test]
    fn qwords_requires_exactly_one() {
        assert!(qwords_enabled(Some("1")));
        assert!(!qwords_enabled(Some(" 1")));
        assert!(!qwords_enabled(Some("01")));
        assert!(!qwords_enabled(Some("true")));
        assert!(!qwords_enabled(Some("0")));
        assert!(!qwords_enabled(None));
    }

    #[test]
    fn memory_uri_dump_requires_exactly_one() {
        assert!(memory_uri_dump_enabled(Some("1")));
        assert!(!memory_uri_dump_enabled(Some(" 1")));
        assert!(!memory_uri_dump_enabled(Some("01")));
        assert!(!memory_uri_dump_enabled(Some("true")));
        assert!(!memory_uri_dump_enabled(Some("0")));
        assert!(!memory_uri_dump_enabled(None));
    }

    #[test]
    fn parse_memory_uri_reads_the_runtime_format() {
        assert_eq!(
            parse_memory_uri("memory:$00007FFBEF9CE110,14532,0:"),
            Some((0x00007FFB_EF9CE110, 14532))
        );
        assert_eq!(
            parse_memory_uri("memory:$0000000000000001,160"),
            Some((1, 160))
        );
        assert_eq!(
            parse_memory_uri("memory:$DEADBEEFCAFEBABE,1048576,0:00000000"),
            Some((0xDEAD_BEEF_CAFE_BABE, MEMURI_MAX_SIZE))
        );
    }

    #[test]
    fn parse_memory_uri_rejects_malformed_text() {
        assert_eq!(parse_memory_uri(""), None);
        assert_eq!(
            parse_memory_uri("platform:/data/settings/presets.zmt"),
            None
        );
        assert_eq!(parse_memory_uri("memory:$1,160,0:"), None);
        assert_eq!(parse_memory_uri("memory:$00007FFBEF9CE11,160,0:"), None);
        assert_eq!(parse_memory_uri("memory:$00007FFBEF9CE1100,160,0:"), None);
        assert_eq!(parse_memory_uri("memory:$00007FFBEF9CE11G,160,0:"), None);
        assert_eq!(parse_memory_uri("memory:$00007FFBEF9CE110,x,0:"), None);
        assert_eq!(parse_memory_uri("memory:$00007FFBEF9CE110,,0:"), None);
        assert_eq!(parse_memory_uri("memory:$00007FFBEF9CE110,14532x,0:"), None);
        assert_eq!(
            parse_memory_uri("memory:$FFFFFFFFFFFFFFFF,18446744073709551616,0:"),
            None
        );
    }

    #[test]
    fn memory_uri_size_allowed_bounds_the_dump() {
        assert!(!memory_uri_size_allowed(0));
        assert!(memory_uri_size_allowed(1));
        assert!(memory_uri_size_allowed(MEMURI_MAX_SIZE));
        assert!(!memory_uri_size_allowed(MEMURI_MAX_SIZE + 1));
        assert!(!memory_uri_size_allowed(usize::MAX));
    }

    #[test]
    fn next_dump_index_stops_at_the_cap() {
        for index in 0..MEMURI_MAX_DUMPS {
            assert_eq!(next_dump_index(index), Some(index));
        }
        assert_eq!(next_dump_index(MEMURI_MAX_DUMPS), None);
        assert_eq!(next_dump_index(usize::MAX), None);
    }

    #[test]
    fn fix_presets_assert_requires_exactly_one() {
        assert!(fix_presets_assert_enabled(Some("1")));
        assert!(!fix_presets_assert_enabled(Some(" 1")));
        assert!(!fix_presets_assert_enabled(Some("01")));
        assert!(!fix_presets_assert_enabled(Some("true")));
        assert!(!fix_presets_assert_enabled(Some("0")));
        assert!(!fix_presets_assert_enabled(None));
    }

    #[test]
    fn no_step_requires_exactly_one() {
        assert!(no_step_enabled(Some("1")));
        assert!(!no_step_enabled(Some(" 1")));
        assert!(!no_step_enabled(Some("01")));
        assert!(!no_step_enabled(Some("true")));
        assert!(!no_step_enabled(Some("0")));
        assert!(!no_step_enabled(None));
    }

    #[test]
    fn parse_name_swap_reads_one_ascii_separator() {
        assert_eq!(
            parse_name_swap(Some("missing.obj:registry.obj")),
            Some(("missing.obj".to_owned(), "registry.obj".to_owned()))
        );
        assert_eq!(
            parse_name_swap(Some("a:b")),
            Some(("a".to_owned(), "b".to_owned()))
        );
    }

    #[test]
    fn parse_name_swap_rejects_unusable_values() {
        assert_eq!(parse_name_swap(None), None);
        assert_eq!(parse_name_swap(Some("")), None);
        assert_eq!(parse_name_swap(Some("from")), None);
        assert_eq!(parse_name_swap(Some(":to")), None);
        assert_eq!(parse_name_swap(Some("from:")), None);
        assert_eq!(parse_name_swap(Some("from:to:extra")), None);
        assert_eq!(parse_name_swap(Some("from name:to")), None);
        assert_eq!(parse_name_swap(Some("from:to name")), None);
        assert_eq!(parse_name_swap(Some("from\0:to")), None);
        assert_eq!(parse_name_swap(Some("from:to\u{e9}")), None);
    }

    #[test]
    fn leaked_name_buffers_round_trip_without_their_nul() {
        let from = leak_nul_terminated("missing.obj");
        assert_eq!(swap_text(from, 11), "missing.obj");
        let to = leak_nul_terminated("registry.obj");
        assert_eq!(swap_text(to, 12), "registry.obj");
    }

    #[test]
    fn corrected_assert_size_removes_only_the_excess() {
        let target = PRESETS_ASSERT_SIZE_TARGET;
        let current_size = 0x12_0000;
        assert_eq!(corrected_assert_size(current_size, target, target), None);
        assert_eq!(
            corrected_assert_size(current_size, target - 1, target),
            None
        );
        assert_eq!(
            corrected_assert_size(current_size, target + 0x1_0000, target),
            Some(0x11_0000)
        );
    }

    #[test]
    fn corrected_assert_size_never_underflows() {
        assert_eq!(
            corrected_assert_size(1, u64::MAX, PRESETS_ASSERT_SIZE_TARGET),
            None
        );
        assert_eq!(corrected_assert_size(0, u64::MAX, 0), None);
    }

    #[test]
    fn step_owner_is_taken_once_and_routes_each_patch() {
        assert_eq!(take_step_owner(), None);
        set_step_owner(PatchKind::Presets);
        assert_eq!(take_step_owner(), Some(PatchKind::Presets));
        assert_eq!(take_step_owner(), None);
        set_step_owner(PatchKind::Watch);
        assert_eq!(take_step_owner(), Some(PatchKind::Watch));
        assert_eq!(take_step_owner(), None);
    }
}
