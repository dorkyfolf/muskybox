//! Ordinal imports of OLE Automation answered by the host.
//!
//! `oleaut32.dll` is not on the `HostPolicy` allow list, so a title that
//! imports it by ordinal would otherwise fail to bind. These two stubs
//! resolve the corresponding named host export once and forward the call.
//! No BSTR is synthesized here: when the host lacks the export, allocation
//! returns null and freeing is a no-op.
#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use crate::{HleExport, HleModule};

/// `SysAllocStringLen` as exported by the host.
type Alloc = extern "system" fn(*const u16, u32) -> *mut u16;
/// `SysFreeString` as exported by the host.
type Free = extern "system" fn(*mut u16);

static ALLOC: OnceLock<Option<usize>> = OnceLock::new();
static FREE: OnceLock<Option<usize>> = OnceLock::new();

/// Logs `export` the first time it is called, with no arguments or payload.
fn once(export: &'static str) {
    static SEEN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.contains(&export) {
        tracing::info!(export, "oleaut32 ordinal forwarded");
        seen.push(export);
    }
}

/// The host's `SysAllocStringLen`, resolved from System32 on first use.
fn alloc_address() -> Option<usize> {
    *ALLOC.get_or_init(|| crate::host::function("oleaut32.dll", "SysAllocStringLen"))
}

/// The host's `SysFreeString`, resolved from System32 on first use.
fn free_address() -> Option<usize> {
    *FREE.get_or_init(|| crate::host::function("oleaut32.dll", "SysFreeString"))
}

/// Ordinal 4 forwards to the host's `SysAllocStringLen`; it never builds a
/// BSTR of its own, and returns null if the host does not export the name.
pub extern "system" fn sys_alloc_string_len(text: *const u16, len: u32) -> *mut u16 {
    once("#4");
    let Some(address) = alloc_address() else {
        return std::ptr::null_mut();
    };
    // SAFETY: the host exports this function with this signature.
    let alloc: Alloc = unsafe { std::mem::transmute::<usize, Alloc>(address) };
    alloc(text, len)
}

/// Ordinal 6 forwards to the host's `SysFreeString`; an unknown host export
/// leaves the string alone instead of freeing memory we never allocated.
pub extern "system" fn sys_free_string(string: *mut u16) {
    once("#6");
    let Some(address) = free_address() else {
        return;
    };
    // SAFETY: the host exports this function with this signature.
    let free: Free = unsafe { std::mem::transmute::<usize, Free>(address) };
    free(string)
}

pub(crate) fn module() -> HleModule {
    HleModule {
        name: "oleaut",
        dlls: &["oleaut32.dll"],
        exports: vec![
            HleExport {
                name: "#4",
                address: sys_alloc_string_len as *const c_void as usize,
            },
            HleExport {
                name: "#6",
                address: sys_free_string as *const c_void as usize,
            },
        ],
    }
}
