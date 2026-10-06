// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The product of this build (internal/platform-spec.md 13.5): its id as in
// products.json, the name the user reads (productName of its Tauri config)
// the line under that name in About, in each locale, and the product's own
// repository, where its releases are (products.json).
// The Vite plugin veydanModules defines them for every build and test run;
// there is no fallback: a build without them fails, it does not become
// Space.

export const product = {
  id: __VEYDAN_PRODUCT__,
  name: __VEYDAN_PRODUCT_NAME__,
  tagline: __VEYDAN_PRODUCT_TAGLINE__,
  repo: __VEYDAN_PRODUCT_REPO__,
} as const;
