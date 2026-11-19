// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! UIKit for `super`, through the same Objective-C bindings tao and wry use
//! on iOS. Nothing here but std and objc2: the file is checked on its own
//! for `aarch64-apple-ios` (no Apple SDK on the build machines of Linux).

use std::ffi::c_void;

use objc2::runtime::AnyObject;
use objc2::{msg_send, sel, MainThreadMarker};
use objc2_foundation::{NSArray, NSString};
use objc2_ui_kit::{UIColor, UIPasteboard, UIUserInterfaceStyle, UIView, UIViewController};

/// What a paste may be, by the type of the pasteboard and the extension of
/// the file it becomes, in the order taken: an animated GIF before the PNG
/// its copy carries too, a JPEG before the HEIC of a photo (the receiving
/// phone may show no HEIC). No video: the pasteboard hands the whole thing
/// over in memory.
const KINDS: &[(&str, &str)] = &[
    ("com.compuserve.gif", "gif"),
    ("public.jpeg", "jpg"),
    ("public.png", "png"),
    ("org.webmproject.webp", "webp"),
    ("public.heic", "heic"),
    ("public.heif", "heif"),
    ("com.adobe.pdf", "pdf"),
];

/// The status bar and the colour behind the page from the page's theme.
/// The style set on the window reaches the view controller of tao, whose
/// `preferredStatusBarStyle` is the default one: dark text on `Light`, light
/// text on `Dark`. The page does not follow the system's appearance (its
/// theme is its own), so the override changes nothing it shows; the sheets
/// of the system it opens (the photo picker, the share sheet) take the
/// app's theme with it. Main thread only.
pub(super) fn bars(webview: *mut c_void, controller: *mut c_void, light: bool, background: Option<[u8; 3]>) {
    let Some(_main) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: the closure of `with_webview` runs on the main thread, where
    // inner() is the WKWebView of the window (a UIView) and
    // view_controller() the UIViewController holding it, both alive while
    // it runs; a null pointer is no view.
    let (Some(view), Some(controller)) = (unsafe { webview.cast::<UIView>().as_ref() }, unsafe {
        controller.cast::<UIViewController>().as_ref()
    }) else {
        return;
    };
    let style = if light { UIUserInterfaceStyle::Light } else { UIUserInterfaceStyle::Dark };
    let color = background.map(|[r, g, b]| {
        UIColor::colorWithRed_green_blue_alpha(f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0, 1.0)
    });
    // The controller first: the web view has no window yet when the page
    // tells its theme before it is shown.
    controller.setOverrideUserInterfaceStyle(style);
    if let (Some(root), Some(color)) = (controller.viewIfLoaded(), color.as_deref()) {
        root.setBackgroundColor(Some(color));
    }
    if let Some(window) = view.window() {
        window.setOverrideUserInterfaceStyle(style);
        if let Some(color) = color.as_deref() {
            window.setBackgroundColor(Some(color));
        }
    }
    controller.setNeedsStatusBarAppearanceUpdate();
}

/// The edges whose system gestures wait for a second swipe. The view
/// controller of tao keeps them (`setPreferredScreenEdgesDeferringSystemGestures:`,
/// vendor/tao `view.rs`) and asks UIKit to read them again. Main thread only.
pub(super) fn defer_edges(controller: *mut c_void, edges: u8) {
    let Some(_main) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: as in `bars`: the view controller of the window, alive on the
    // main thread while the closure runs.
    let Some(controller) = (unsafe { controller.cast::<AnyObject>().as_ref() }) else {
        return;
    };
    let setter = sel!(setPreferredScreenEdgesDeferringSystemGestures:);
    // SAFETY: `respondsToSelector:` is NSObject's; the setter is tao's own
    // property, of a UIRectEdge (an NSUInteger), declared where the class is.
    unsafe {
        let declared: bool = msg_send![controller, respondsToSelector: setter];
        if declared {
            let _: () = msg_send![controller, setPreferredScreenEdgesDeferringSystemGestures: usize::from(edges)];
        }
    }
}

/// Whether the pasteboard holds a picture or a PDF: only its types are
/// asked, which iOS does not announce.
pub(super) fn clipboard_has() -> bool {
    let board = UIPasteboard::generalPasteboard();
    let types: Vec<_> = KINDS.iter().map(|(uti, _)| NSString::from_str(uti)).collect();
    // SAFETY: a read of the general pasteboard, off the main thread as
    // UIPasteboard allows.
    let images = unsafe { board.hasImages() };
    images || board.containsPasteboardTypes(&NSArray::from_retained_slice(&types))
}

/// The bytes of the first picture or PDF on the pasteboard, with the
/// extension of their type. Reading them is what iOS asks the user about
/// («Allow Paste»): the call waits for the answer, so never on the main
/// thread. A picture held only as an image (no data of a type above) is
/// taken as a PNG.
pub(super) fn clipboard_take() -> Option<(Vec<u8>, &'static str)> {
    let board = UIPasteboard::generalPasteboard();
    for (uti, ext) in KINDS {
        let uti = NSString::from_str(uti);
        if !board.containsPasteboardTypes(&NSArray::from_retained_slice(std::slice::from_ref(&uti))) {
            continue;
        }
        if let Some(data) = board.dataForPasteboardType(&uti) {
            return Some((data.to_vec(), ext));
        }
    }
    // SAFETY: as in `clipboard_has`.
    if !unsafe { board.hasImages() } {
        return None;
    }
    // SAFETY: as in `clipboard_has`; the image is the pasteboard's copy.
    let image = unsafe { board.image() }?;
    Some((image.png_representation()?.to_vec(), "png"))
}
