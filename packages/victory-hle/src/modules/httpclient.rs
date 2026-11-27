//! Offline answers for the documented HTTP surface of libHttpClient.
//!
//! The console has no network cable. `net` refuses Winsock and WinHTTP
//! traffic, so nothing a title asks libHttpClient to send can leave the
//! machine. Every import in `libhttpclient.gdk.dll` is answered here: handles
//! are static sentinels that are never dereferenced, strings are static and
//! empty, settings take their documented defaults, and the one async call we
//! start fails immediately through `xasync` so the title's completion routine
//! still runs.
//!
//! Only the imports reported by `census --missing` are implemented. Provider
//! callbacks, routed handlers, WebSocket calls and memory hooks are absent on
//! purpose: a title importing one still reports a missing import instead of
//! receiving an invented answer.
#![allow(non_snake_case)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{c_char, c_void};
use std::sync::Mutex;

use crate::modules::xasync::{AsyncBlock, E_INVALIDARG, finished};
use crate::modules::xgameruntime::{HRESULT, S_OK};
use crate::{HleModule, hle_export};

/// `HRESULT_FROM_WIN32(ERROR_WINHTTP_CANNOT_CONNECT)`, the error `net` uses
/// for an address off the machine.
const NO_NETWORK: HRESULT = 0x8007_2EFD_u32 as i32;

/// Documented defaults for request settings.
const DEFAULT_RETRY_DELAY: u32 = 2;
const DEFAULT_TIMEOUT: u32 = 30;
const DEFAULT_TIMEOUT_WINDOW: u32 = 20;
const NO_RETRY_CACHE_ID: u32 = 0;
const TRACE_LEVEL_OFF: u32 = 0;

/// Empty, NUL-terminated string; every string getter returns this static.
static EMPTY: [u8; 1] = [0];

/// Opaque call handle. Nothing reads it, so one static byte is enough.
static CALL: u8 = 0;

/// Identity of the perform call, to tell it apart from other async calls.
static PERFORM: u8 = 0;

/// Logs `export` the first time it is called, without arguments or payload.
fn once(export: &'static str) {
    static SEEN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.contains(&export) {
        tracing::info!(export, "libHttpClient stub called");
        seen.push(export);
    }
}

/// Writes `value` only when the title supplied an out pointer.
fn set<T: Copy>(out: *mut T, value: T) {
    if !out.is_null() {
        // SAFETY: the title passes a writable out pointer or null.
        unsafe { out.write(value) };
    }
}

/// The shared empty string as a UTF-8 pointer.
fn empty() -> *const c_char {
    EMPTY.as_ptr().cast()
}

/// The shared, never dereferenced call handle.
fn call_handle() -> *mut c_void {
    std::ptr::from_ref(&CALL).cast_mut().cast::<c_void>()
}

/// `HCCleanupAsync` ends the library; offline there is nothing to end.
pub extern "system" fn HCCleanupAsync(_block: *mut AsyncBlock) -> HRESULT {
    once("HCCleanupAsync");
    S_OK
}

/// `HCHttpCallCloseHandle` has nothing to free for a sentinel handle.
pub extern "system" fn HCHttpCallCloseHandle(_call: *mut c_void) {
    once("HCHttpCallCloseHandle");
}

/// `HCHttpCallCreate` hands out the shared, opaque call handle.
pub extern "system" fn HCHttpCallCreate(call: *mut *mut c_void) -> HRESULT {
    once("HCHttpCallCreate");
    set(call, call_handle());
    S_OK
}

/// `HCHttpCallDuplicateHandle` returns the same sentinel handle.
pub extern "system" fn HCHttpCallDuplicateHandle(_call: *mut c_void) -> *mut c_void {
    once("HCHttpCallDuplicateHandle");
    call_handle()
}

/// `HCHttpCallGetRequestUrl` returns the empty URL; no request was made.
pub extern "system" fn HCHttpCallGetRequestUrl(
    _call: *mut c_void,
    url: *mut *const c_char,
) -> HRESULT {
    once("HCHttpCallGetRequestUrl");
    set(url, empty());
    S_OK
}

/// `HCHttpCallPerformAsync` fails with no network and completes the block
/// through `xasync`, so a completion routine still sees the same error.
pub extern "system" fn HCHttpCallPerformAsync(
    _call: *mut c_void,
    block: *mut AsyncBlock,
) -> HRESULT {
    once("HCHttpCallPerformAsync");
    let _ = finished(block, (&raw const PERFORM).cast(), NO_NETWORK);
    NO_NETWORK
}

/// `HCHttpCallRequestGetHeaderAtIndex` has no request header to name.
pub extern "system" fn HCHttpCallRequestGetHeaderAtIndex(
    _call: *mut c_void,
    _index: u32,
    _name: *mut *const c_char,
    _value: *mut *const c_char,
) -> HRESULT {
    once("HCHttpCallRequestGetHeaderAtIndex");
    E_INVALIDARG
}

/// `HCHttpCallRequestGetNumHeaders` reports no request headers.
pub extern "system" fn HCHttpCallRequestGetNumHeaders(
    _call: *mut c_void,
    count: *mut u32,
) -> HRESULT {
    once("HCHttpCallRequestGetNumHeaders");
    set(count, 0);
    S_OK
}

/// `HCHttpCallRequestGetRequestBodyBytes` reports the documented empty body.
pub extern "system" fn HCHttpCallRequestGetRequestBodyBytes(
    _call: *mut c_void,
    bytes: *mut *const u8,
    size: *mut u32,
) -> HRESULT {
    once("HCHttpCallRequestGetRequestBodyBytes");
    set(bytes, std::ptr::null());
    set(size, 0);
    S_OK
}

/// `HCHttpCallRequestGetRetryAllowed` returns the documented default, true.
pub extern "system" fn HCHttpCallRequestGetRetryAllowed(
    _call: *mut c_void,
    allowed: *mut bool,
) -> HRESULT {
    once("HCHttpCallRequestGetRetryAllowed");
    set(allowed, true);
    S_OK
}

/// `HCHttpCallRequestGetRetryCacheId` returns no cache id.
pub extern "system" fn HCHttpCallRequestGetRetryCacheId(
    _call: *mut c_void,
    cache_id: *mut u32,
) -> HRESULT {
    once("HCHttpCallRequestGetRetryCacheId");
    set(cache_id, NO_RETRY_CACHE_ID);
    S_OK
}

/// `HCHttpCallRequestGetRetryDelay` returns the documented two seconds.
pub extern "system" fn HCHttpCallRequestGetRetryDelay(
    _call: *mut c_void,
    delay: *mut u32,
) -> HRESULT {
    once("HCHttpCallRequestGetRetryDelay");
    set(delay, DEFAULT_RETRY_DELAY);
    S_OK
}

/// `HCHttpCallRequestGetTimeout` returns the documented 30 seconds.
pub extern "system" fn HCHttpCallRequestGetTimeout(
    _call: *mut c_void,
    timeout: *mut u32,
) -> HRESULT {
    once("HCHttpCallRequestGetTimeout");
    set(timeout, DEFAULT_TIMEOUT);
    S_OK
}

/// `HCHttpCallRequestGetTimeoutWindow` returns the documented 20 seconds.
pub extern "system" fn HCHttpCallRequestGetTimeoutWindow(
    _call: *mut c_void,
    window: *mut u32,
) -> HRESULT {
    once("HCHttpCallRequestGetTimeoutWindow");
    set(window, DEFAULT_TIMEOUT_WINDOW);
    S_OK
}

/// `HCHttpCallRequestGetUrl` returns the empty method and URL.
pub extern "system" fn HCHttpCallRequestGetUrl(
    _call: *mut c_void,
    method: *mut *const c_char,
    url: *mut *const c_char,
) -> HRESULT {
    once("HCHttpCallRequestGetUrl");
    set(method, empty());
    set(url, empty());
    S_OK
}

/// `HCHttpCallRequestSetHeader` accepts the header and sends nothing.
pub extern "system" fn HCHttpCallRequestSetHeader(
    _call: *mut c_void,
    _name: *const c_char,
    _value: *const c_char,
) -> HRESULT {
    once("HCHttpCallRequestSetHeader");
    S_OK
}

/// `HCHttpCallRequestSetRequestBodyBytes` accepts the body and sends nothing.
pub extern "system" fn HCHttpCallRequestSetRequestBodyBytes(
    _call: *mut c_void,
    _bytes: *const u8,
    _size: u32,
) -> HRESULT {
    once("HCHttpCallRequestSetRequestBodyBytes");
    S_OK
}

/// `HCHttpCallRequestSetRequestBodyString` accepts the body and sends nothing.
pub extern "system" fn HCHttpCallRequestSetRequestBodyString(
    _call: *mut c_void,
    _body: *const c_char,
) -> HRESULT {
    once("HCHttpCallRequestSetRequestBodyString");
    S_OK
}

/// `HCHttpCallRequestSetRetryAllowed` accepts the setting without retrying.
pub extern "system" fn HCHttpCallRequestSetRetryAllowed(
    _call: *mut c_void,
    _allowed: bool,
) -> HRESULT {
    once("HCHttpCallRequestSetRetryAllowed");
    S_OK
}

/// `HCHttpCallRequestSetRetryCacheId` accepts the setting without a cache.
pub extern "system" fn HCHttpCallRequestSetRetryCacheId(
    _call: *mut c_void,
    _cache_id: u32,
) -> HRESULT {
    once("HCHttpCallRequestSetRetryCacheId");
    S_OK
}

/// `HCHttpCallRequestSetRetryDelay` accepts the setting without waiting.
pub extern "system" fn HCHttpCallRequestSetRetryDelay(_call: *mut c_void, _delay: u32) -> HRESULT {
    once("HCHttpCallRequestSetRetryDelay");
    S_OK
}

/// `HCHttpCallRequestSetTimeout` accepts the setting without waiting.
pub extern "system" fn HCHttpCallRequestSetTimeout(_call: *mut c_void, _timeout: u32) -> HRESULT {
    once("HCHttpCallRequestSetTimeout");
    S_OK
}

/// `HCHttpCallRequestSetTimeoutWindow` accepts the setting without waiting.
pub extern "system" fn HCHttpCallRequestSetTimeoutWindow(
    _call: *mut c_void,
    _window: u32,
) -> HRESULT {
    once("HCHttpCallRequestSetTimeoutWindow");
    S_OK
}

/// `HCHttpCallRequestSetUrl` accepts the method and URL and sends nothing.
pub extern "system" fn HCHttpCallRequestSetUrl(
    _call: *mut c_void,
    _method: *const c_char,
    _url: *const c_char,
) -> HRESULT {
    once("HCHttpCallRequestSetUrl");
    S_OK
}

/// `HCHttpCallResponseGetHeader` has no header to answer.
pub extern "system" fn HCHttpCallResponseGetHeader(
    _call: *mut c_void,
    _name: *const c_char,
    _value: *mut *const c_char,
) -> HRESULT {
    once("HCHttpCallResponseGetHeader");
    E_INVALIDARG
}

/// `HCHttpCallResponseGetHeaderAtIndex` has no header to answer either.
pub extern "system" fn HCHttpCallResponseGetHeaderAtIndex(
    _call: *mut c_void,
    _index: u32,
    _name: *mut *const c_char,
    _value: *mut *const c_char,
) -> HRESULT {
    once("HCHttpCallResponseGetHeaderAtIndex");
    E_INVALIDARG
}

/// `HCHttpCallResponseGetNetworkErrorCode` reports the perform failure.
pub extern "system" fn HCHttpCallResponseGetNetworkErrorCode(
    _call: *mut c_void,
    error: *mut HRESULT,
    platform_error: *mut u32,
) -> HRESULT {
    once("HCHttpCallResponseGetNetworkErrorCode");
    set(error, NO_NETWORK);
    set(platform_error, 0);
    S_OK
}

/// `HCHttpCallResponseGetNumHeaders` reports no response headers.
pub extern "system" fn HCHttpCallResponseGetNumHeaders(
    _call: *mut c_void,
    count: *mut u32,
) -> HRESULT {
    once("HCHttpCallResponseGetNumHeaders");
    set(count, 0);
    S_OK
}

/// `HCHttpCallResponseGetPlatformNetworkErrorMessage` returns a static empty
/// string, so no host-specific network detail leaks to the title.
pub extern "system" fn HCHttpCallResponseGetPlatformNetworkErrorMessage(
    _call: *mut c_void,
    message: *mut *const c_char,
) -> HRESULT {
    once("HCHttpCallResponseGetPlatformNetworkErrorMessage");
    set(message, empty());
    S_OK
}

/// `HCHttpCallResponseGetResponseBodyBytes` reports zero bytes used and
/// leaves the title's buffer untouched.
pub extern "system" fn HCHttpCallResponseGetResponseBodyBytes(
    _call: *mut c_void,
    _size: usize,
    _buffer: *mut u8,
    used: *mut usize,
) -> HRESULT {
    once("HCHttpCallResponseGetResponseBodyBytes");
    set(used, 0);
    S_OK
}

/// `HCHttpCallResponseGetResponseBodyBytesSize` reports an empty response.
pub extern "system" fn HCHttpCallResponseGetResponseBodyBytesSize(
    _call: *mut c_void,
    size: *mut usize,
) -> HRESULT {
    once("HCHttpCallResponseGetResponseBodyBytesSize");
    set(size, 0);
    S_OK
}

/// `HCHttpCallResponseGetResponseString` returns an empty response body.
pub extern "system" fn HCHttpCallResponseGetResponseString(
    _call: *mut c_void,
    response: *mut *const c_char,
) -> HRESULT {
    once("HCHttpCallResponseGetResponseString");
    set(response, empty());
    S_OK
}

/// `HCHttpCallResponseGetStatusCode` reports that no response arrived.
pub extern "system" fn HCHttpCallResponseGetStatusCode(
    _call: *mut c_void,
    status: *mut u32,
) -> HRESULT {
    once("HCHttpCallResponseGetStatusCode");
    set(status, 0);
    S_OK
}

/// `HCHttpCallSetTracing` accepts the setting but writes no trace.
pub extern "system" fn HCHttpCallSetTracing(_enabled: bool) -> HRESULT {
    once("HCHttpCallSetTracing");
    S_OK
}

/// `HCInitialize` starts the library; offline there is nothing to start.
pub extern "system" fn HCInitialize(_args: *const c_void) -> HRESULT {
    once("HCInitialize");
    S_OK
}

/// `HCSettingsGetTraceLevel` reports the documented off level.
pub extern "system" fn HCSettingsGetTraceLevel(level: *mut u32) -> HRESULT {
    once("HCSettingsGetTraceLevel");
    set(level, TRACE_LEVEL_OFF);
    S_OK
}

/// `HCTraceImplMessage` ignores a trace message. It is variadic in C, so the
/// Rust side uses a fixed three-argument signature; see OPEN.
pub extern "system" fn HCTraceImplMessage(_level: u32, _message: *const c_char, _argument: usize) {
    once("HCTraceImplMessage");
}

/// `HCTraceSetEtwEnabled` accepts the setting; no ETW session is opened.
pub extern "system" fn HCTraceSetEtwEnabled(_enabled: bool) -> HRESULT {
    once("HCTraceSetEtwEnabled");
    S_OK
}

pub(crate) fn module() -> HleModule {
    HleModule {
        name: "httpclient",
        dlls: &["libhttpclient.gdk.dll"],
        exports: vec![
            hle_export!(HCCleanupAsync),
            hle_export!(HCHttpCallCloseHandle),
            hle_export!(HCHttpCallCreate),
            hle_export!(HCHttpCallDuplicateHandle),
            hle_export!(HCHttpCallGetRequestUrl),
            hle_export!(HCHttpCallPerformAsync),
            hle_export!(HCHttpCallRequestGetHeaderAtIndex),
            hle_export!(HCHttpCallRequestGetNumHeaders),
            hle_export!(HCHttpCallRequestGetRequestBodyBytes),
            hle_export!(HCHttpCallRequestGetRetryAllowed),
            hle_export!(HCHttpCallRequestGetRetryCacheId),
            hle_export!(HCHttpCallRequestGetRetryDelay),
            hle_export!(HCHttpCallRequestGetTimeout),
            hle_export!(HCHttpCallRequestGetTimeoutWindow),
            hle_export!(HCHttpCallRequestGetUrl),
            hle_export!(HCHttpCallRequestSetHeader),
            hle_export!(HCHttpCallRequestSetRequestBodyBytes),
            hle_export!(HCHttpCallRequestSetRequestBodyString),
            hle_export!(HCHttpCallRequestSetRetryAllowed),
            hle_export!(HCHttpCallRequestSetRetryCacheId),
            hle_export!(HCHttpCallRequestSetRetryDelay),
            hle_export!(HCHttpCallRequestSetTimeout),
            hle_export!(HCHttpCallRequestSetTimeoutWindow),
            hle_export!(HCHttpCallRequestSetUrl),
            hle_export!(HCHttpCallResponseGetHeader),
            hle_export!(HCHttpCallResponseGetHeaderAtIndex),
            hle_export!(HCHttpCallResponseGetNetworkErrorCode),
            hle_export!(HCHttpCallResponseGetNumHeaders),
            hle_export!(HCHttpCallResponseGetPlatformNetworkErrorMessage),
            hle_export!(HCHttpCallResponseGetResponseBodyBytes),
            hle_export!(HCHttpCallResponseGetResponseBodyBytesSize),
            hle_export!(HCHttpCallResponseGetResponseString),
            hle_export!(HCHttpCallResponseGetStatusCode),
            hle_export!(HCHttpCallSetTracing),
            hle_export!(HCInitialize),
            hle_export!(HCSettingsGetTraceLevel),
            hle_export!(HCTraceImplMessage),
            hle_export!(HCTraceSetEtwEnabled),
        ],
    }
}
