// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** Injected by vite.config.js `define` from TAURI_ENV_PLATFORM. */
declare const __TAURI_PLATFORM__: string;
/**
 * Defined by the Vite plugin veydanModules (vite-veydan-modules.js) from
 * VEYDAN_PRODUCT: the id of the product, its productName and its tagline in
 * each locale (products.json).
 */
declare const __VEYDAN_PRODUCT__: string;
declare const __VEYDAN_PRODUCT_NAME__: string;
declare const __VEYDAN_PRODUCT_TAGLINE__: { en: string; ru: string } & Record<string, string>;
/** https://github.com/<owner>/<repo> of the product (products.json `repo`). */
declare const __VEYDAN_PRODUCT_REPO__: string;
