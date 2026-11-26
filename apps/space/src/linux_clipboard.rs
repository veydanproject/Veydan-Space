// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A paste with nobody owning the clipboard closes the window on Linux
//! (ROO-1835): WebKitGTK 2.54.x asks for the targets of the clipboard with
//! `gtk_clipboard_request_targets`, and GTK answers a clipboard without an
//! owner with no atoms and a count of -1. `Clipboard::formats` of WebKit
//! (Source/WebKit/UIProcess/gtk/ClipboardGtk3.cpp) makes a span of that
//! count and reads atom 0 of NULL: SIGSEGV in the UI process, on ctrl+v, on
//! «Paste» of the menu, on `navigator.clipboard`.
//!
//! WebKit main mends it with `std::max(0, atomsCount)` in the same function
//! (https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/gtk/ClipboardGtk3.cpp);
//! no released 2.54.x has it. The binary defines the function of GTK
//! itself, exported from it (build.rs), so the dynamic linker binds WebKit's
//! call here; the callback it is given sees `(NULL, 0)` instead of
//! `(NULL, -1)` and every other answer unchanged. Remove this file and its
//! line in build.rs once the WebKitGTK of the distributions Space supports
//! carries the fix.

use std::ffi::{c_int, c_void};
use std::sync::OnceLock;

/// `GtkClipboardTargetsReceivedFunc`: the clipboard, its atoms, their count,
/// the caller's data.
type TargetsReceived = unsafe extern "C" fn(*mut c_void, *mut *mut c_void, c_int, *mut c_void);
/// `gtk_clipboard_request_targets`.
type RequestTargets = unsafe extern "C" fn(*mut c_void, Option<TargetsReceived>, *mut c_void);

/// The function of GTK, the next definition after this binary's; `None`
/// when there is none.
fn real() -> Option<RequestTargets> {
    static REAL: OnceLock<Option<RequestTargets>> = OnceLock::new();
    *REAL.get_or_init(|| {
        // SAFETY: a NUL-terminated name; a non-null result is GTK's function
        // of the signature above.
        let found = unsafe { libc::dlsym(libc::RTLD_NEXT, c"gtk_clipboard_request_targets".as_ptr()) };
        (!found.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, RequestTargets>(found) })
    })
}

/// The caller's callback and data, carried to [`received`].
struct Request {
    callback: TargetsReceived,
    data: *mut c_void,
}

/// What the callback gets in place of GTK's answer: `(NULL, 0)` for no
/// atoms or a negative count, the answer itself otherwise.
fn mended(atoms: *mut *mut c_void, count: c_int) -> (*mut *mut c_void, c_int) {
    if atoms.is_null() || count < 0 {
        (std::ptr::null_mut(), 0)
    } else {
        (atoms, count)
    }
}

unsafe extern "C" fn received(clipboard: *mut c_void, atoms: *mut *mut c_void, count: c_int, request: *mut c_void) {
    // SAFETY: `request` is the box `gtk_clipboard_request_targets` below
    // leaked; GTK calls back once per request.
    let request = unsafe { Box::from_raw(request.cast::<Request>()) };
    let (atoms, count) = mended(atoms, count);
    unsafe { (request.callback)(clipboard, atoms, count, request.data) }
}

/// GTK's `gtk_clipboard_request_targets`, its callback wrapped by
/// [`received`].
///
/// # Safety
///
/// The contract of GTK's function: a `GtkClipboard`, a callback of its type,
/// the data of that callback.
#[no_mangle]
pub unsafe extern "C" fn gtk_clipboard_request_targets(
    clipboard: *mut c_void,
    callback: Option<TargetsReceived>,
    data: *mut c_void,
) {
    let Some(callback) = callback else {
        if let Some(real) = real() {
            unsafe { real(clipboard, None, data) }
        }
        return;
    };
    match real() {
        Some(real) => {
            let request = Box::into_raw(Box::new(Request { callback, data }));
            unsafe { real(clipboard, Some(received), request.cast()) }
        }
        // No GTK to ask: the caller hears that there are no targets.
        None => unsafe { callback(clipboard, std::ptr::null_mut(), 0, data) },
    }
}

#[cfg(test)]
mod tests {
    use super::mended;
    use std::ffi::c_void;

    #[test]
    fn no_owner_becomes_no_atoms() {
        assert_eq!(mended(std::ptr::null_mut(), -1), (std::ptr::null_mut(), 0));
        assert_eq!(mended(std::ptr::null_mut(), 3), (std::ptr::null_mut(), 0));
    }

    #[test]
    fn an_answer_with_atoms_passes_unchanged() {
        let mut atoms = [std::ptr::null_mut::<c_void>(); 2];
        let ptr = atoms.as_mut_ptr();
        assert_eq!(mended(ptr, 2), (ptr, 2));
        assert_eq!(mended(ptr, 0), (ptr, 0));
        assert_eq!(mended(ptr, -1), (std::ptr::null_mut(), 0));
    }
}
