// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The document picker of iOS for `super`, through the same Objective-C
//! bindings tao and wry use. Nothing here but std and objc2: the file is
//! checked on its own for `aarch64-apple-ios` (no Apple SDK on the build
//! machines of Linux).

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::Path;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use objc2_foundation::{NSArray, NSString, NSURL};
use objc2_ui_kit::{
    UIAdaptivePresentationControllerDelegate, UIDocumentPickerDelegate, UIDocumentPickerViewController,
    UIPresentationController, UIViewController,
};

/// The answer of an export: saved, given up, or why the picker was not shown.
pub(super) type Done = Box<dyn FnOnce(Result<bool, &'static str>)>;

pub(super) struct Ivars {
    done: Cell<Option<Done>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class
    // implements no `dealloc`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VeydanShellExportDelegate"]
    #[ivars = Ivars]
    pub(super) struct ExportDelegate;

    unsafe impl NSObjectProtocol for ExportDelegate {}

    // SAFETY: the signatures are those of the protocol.
    unsafe impl UIDocumentPickerDelegate for ExportDelegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn picked(&self, _picker: &UIDocumentPickerViewController, _urls: &NSArray<NSURL>) {
            self.answer(Ok(true));
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn cancelled(&self, _picker: &UIDocumentPickerViewController) {
            self.answer(Ok(false));
        }
    }

    // SAFETY: as above.
    unsafe impl UIAdaptivePresentationControllerDelegate for ExportDelegate {
        /// Swiped away: given up.
        #[unsafe(method(presentationControllerDidDismiss:))]
        fn dismissed(&self, _controller: &UIPresentationController) {
            self.answer(Ok(false));
        }
    }
);

thread_local! {
    /// The delegate of the picker on the screen: UIKit holds a delegate
    /// weakly, so it is kept here until it answers. One at a time.
    static SHOWN: RefCell<Option<Retained<ExportDelegate>>> = const { RefCell::new(None) };
}

impl ExportDelegate {
    fn new(main: MainThreadMarker, done: Done) -> Retained<Self> {
        let this = Self::alloc(main).set_ivars(Ivars { done: Cell::new(Some(done)) });
        // SAFETY: NSObject's `init` on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }

    /// Answers once; the picker may report both a pick or a cancel and the
    /// dismissal that follows it.
    fn answer(&self, answer: Result<bool, &'static str>) {
        let Some(done) = self.ivars().done.take() else {
            return;
        };
        // Released by the pool of the run loop, not inside this call.
        if let Some(kept) = SHOWN.with(|shown| shown.borrow_mut().take()) {
            let _ = Retained::autorelease_ptr(kept);
        }
        done(answer);
    }
}

/// The document picker for exporting `file` as a copy, over whatever the
/// window shows on top of `controller` (its root view controller). `done`
/// is called once: `Ok(true)` when the system copied the file to the folder
/// the user picked, `Ok(false)` when the user gave up, `Err` when nothing
/// was shown (`busy`, `no_screen`). Main thread only.
pub(super) fn export(controller: *mut c_void, file: &Path, done: Done) {
    let Some(main) = MainThreadMarker::new() else {
        return done(Err("no_screen"));
    };
    // SAFETY: the closure of `with_webview` runs on the main thread, where
    // view_controller() is the root view controller of the window, alive
    // while it runs; a null pointer is none.
    let Some(root) = (unsafe { controller.cast::<UIViewController>().as_ref() }) else {
        return done(Err("no_screen"));
    };
    if SHOWN.with(|shown| shown.borrow().is_some()) {
        return done(Err("busy"));
    }
    let mut top = root.retain();
    while let Some(next) = top.presentedViewController() {
        top = next;
    }
    let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
    let picker = UIDocumentPickerViewController::initForExportingURLs_asCopy(
        UIDocumentPickerViewController::alloc(main),
        &NSArray::from_retained_slice(&[url]),
        true,
    );
    let delegate = ExportDelegate::new(main, done);
    picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    picker.setShouldShowFileExtensions(true);
    if let Some(presentation) = picker.presentationController() {
        // SAFETY: a weak property; `SHOWN` keeps the delegate until it answers.
        unsafe { presentation.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    }
    SHOWN.with(|shown| *shown.borrow_mut() = Some(delegate));
    top.presentViewController_animated_completion(&picker, true, None);
}
