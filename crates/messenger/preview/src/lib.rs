// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Previews of pages outside.
//!
//! Asking a site for its title tells the site who asked, so nothing here
//! runs on its own: the user presses a button, one page is read, and the
//! picture it names is brought by this code, never by the webview.
//!
//! - `target`: which addresses may be asked at all.
//! - `html`: what a page says about itself.
//! - `fetch`: the network, behind a trait.
//! - `service`: one preview from one address.
//!
//! Refusals are stable codes (`preview_not_https`, …) the UI translates.

pub mod fetch;
pub mod html;
pub mod service;
pub mod target;

pub use fetch::{Fetched, Fetcher, ReqwestFetcher};
pub use html::PageMeta;
pub use service::{Preview, PreviewService};
pub use target::{is_public, Target};

use messenger_core::MessengerError;

pub(crate) fn refuse(code: &str) -> MessengerError {
    MessengerError::Invalid(code.into())
}
