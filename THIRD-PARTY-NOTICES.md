# Third-Party Notices

DiffLens is MIT-licensed (see [LICENSE](./LICENSE)). This file attributes the
third-party components that are **bundled with the installers**, **linked into
the app**, or **downloaded at runtime** through DiffLens' download manager.
Versions and pins were audited on **2026-11-18** against `Cargo.lock`,
`package-lock.json`, `resources/manifests/llm-manifest.json`,
`scripts/fetch-models.sh` and `scripts/fetch-pdfium.sh`.

No AGPL/GPL components are used. Where a component is dual-licensed
(`MIT OR Apache-2.0`), DiffLens uses it under the MIT option unless noted.

Short-form license references used below:

- **MIT**: Permission is hereby granted, free of charge, to any person
  obtaining a copy of this software … to deal in the Software without
  restriction … (full text: <https://opensource.org/license/MIT>)
- **Apache-2.0**: Licensed under the Apache License, Version 2.0; you may not
  use this file except in compliance with the License. You may obtain a copy
  at <https://www.apache.org/licenses/LICENSE-2.0>. Apache-2.0 requires
  retaining copyright/NOTICE contents — see the per-component NOTICE pointers
  in the tables (Apache-2.0 full text is bundled with this file's source at
  <https://www.apache.org/licenses/LICENSE-2.0.txt>).
- **BSD-3-Clause**: Redistribution and use in source and binary forms, with or
  without modification, are permitted provided that the three clauses
  (copyright retention, binary reproduction notice, non-endorsement) are met
  (full text: <https://opensource.org/license/BSD-3-Clause>).
- **ISC**: functionally equivalent to MIT/BSD
  (full text: <https://opensource.org/license/ISC>).

---

## 1. Bundled with the installers (build-time fetched binaries)

| Component | Version / pin | License | Source |
|---|---|---|---|
| PDFium | chromium build **8165** (`chromium/8165` tag of bblanchon/pdfium-binaries, published 2026-11-16) | **BSD-3-Clause** — Copyright 2014 The PDFium Authors. Redistribution notices must be retained; see <https://chromium.googlesource.com/chromium/pdfium/+/main/LICENSE> | <https://pdfium.googlesource.com/pdfium> |
| bblanchon/pdfium-binaries (build pipeline for the PDFium shared library) | chromium/8165 | **MIT** (repository) — the distributed `libpdfium.so/.dll/.dylib` carries PDFium's BSD-3-Clause license above | <https://github.com/bblanchon/pdfium-binaries> |
| pdfium-render (Rust bindings; loaded dynamically, not compiled into PDFium) | 0.9.5 | MIT OR Apache-2.0 | <https://github.com/ajrcarey/pdfium-render> |
| Tesseract OCR (optional; **not bundled** — detected on PATH if the user installs it) | 5.x | Apache-2.0 (since v4) | <https://github.com/tesseract-ocr/tesseract> |

## 2. Rust crates linked into the app

| Crate | Version (Cargo.lock 2026-11-18) | License | Source |
|---|---|---|---|
| tauri | 2.12.6 | MIT OR Apache-2.0 | <https://github.com/tauri-apps/tauri> |
| tauri-build | 2.7.5 | MIT OR Apache-2.0 | <https://github.com/tauri-apps/tauri> |
| tauri-plugin-dialog | 2.8.4 | MIT OR Apache-2.0 | <https://github.com/tauri-apps/plugins-workspace> |
| tauri-plugin-shell | 2.6.0 | MIT OR Apache-2.0 | <https://github.com/tauri-apps/plugins-workspace> |
| zip | 9.7.0 | MIT | <https://github.com/zip-rs/zip2> |
| roxmltree | 0.21.2 | MIT OR Apache-2.0 | <https://github.com/RazrFalcon/roxmltree> |
| calamine | 0.38.0 | MIT | <https://github.com/tafia/calamine> |
| similar | 3.2.1 | **Apache-2.0** — copyright/NOTICE retained: "Copyright (c) 2015-2025 Armin Ronacher" (see NOTICE file in the crate) | <https://github.com/mitsuhiko/similar> |
| strsim | 0.11.1 | MIT | <https://github.com/rapidfuzz/strsim-rs> |
| rusqlite | 0.32.1 | MIT | <https://github.com/rusqlite/rusqlite> |
| libsqlite3-sys / SQLite | 0.30.1 / 3.x (bundled) | MIT (crate); **SQLite is in the public domain** — the authors dedicate their copyright to the public domain ("The author or authors of this code dedicate any and all copyright interest in this code to the public domain"); no attribution required, blessing at <https://www.sqlite.org/copyright.html> | <https://www.sqlite.org> |
| ureq | 2.12.1 | MIT OR Apache-2.0 | <https://github.com/algesten/ureq> |
| flate2 | 1.1.11 | MIT OR Apache-2.0 | <https://github.com/rust-lang/flate2-rs> |
| tar | 0.4.46 | MIT OR Apache-2.0 | <https://github.com/alexcrichton/tar-rs> |
| ort | 2.0.0-rc.15 | MIT OR Apache-2.0 | <https://github.com/pykeio/ort> |
| ONNX Runtime (prebuilt, statically linked by ort-sys at build time) | ~1.x per pyke.io binaries | **MIT** — Copyright (c) Microsoft Corporation | <https://github.com/microsoft/onnxruntime> |
| tokenizers | 0.23.2 | **Apache-2.0** — Copyright (c) 2019 The HuggingFace Inc. team (NOTICE retained in crate; used to load the pinned `tokenizer.json`) | <https://github.com/huggingface/tokenizers> |
| serde / serde_json | 1.0.229 / 1.0.154 | MIT OR Apache-2.0 | <https://github.com/serde-rs/serde> |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | <https://github.com/dtolnay/thiserror> |
| tracing / tracing-subscriber | 0.1.44 / 0.3.x | MIT OR Apache-2.0 | <https://github.com/tokio-rs/tracing> |

All other transitive crates are licensed MIT, Apache-2.0, or BSD-class
(permissively compatible); a machine-generated complete inventory can be
reproduced from `Cargo.lock`.

## 3. Components downloaded at runtime (never bundled in installers)

These are fetched on explicit user action through the in-app download manager,
which only accepts URLs from the pinned manifest
`resources/manifests/llm-manifest.json` and verifies SHA-256 checksums before
use. They are stored in the user's own data directory.

### 3.1 Embedding model (required, ~118 MB)

| Item | Detail |
|---|---|
| Model | **Xenova/multilingual-e5-small** — ONNX export (`onnx/model_quantized.onnx` + `tokenizer.json`) |
| Pinned commit | `761b726dd34fb83930e26aab4e9ac3899aa1fa78` (resolved 2026-11-18; never re-resolved implicitly) |
| SHA-256 | model `f80102d3f2a1229f387d3c81909990d8945513e347b0eab049f7de3c6f98c193`; tokenizer `0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39` |
| License | **MIT** — base model [intfloat/multilingual-e5-small](https://huggingface.co/intfloat/multilingual-e5-small) (verified via HF model card); the Xenova export inherits the upstream MIT license |
| URL | <https://huggingface.co/Xenova/multilingual-e5-small> |

### 3.2 LLM sidecar + explanation models (optional)

| Item | Version / pin | License | Source |
|---|---|---|---|
| llama.cpp `llama-server` | release **b12503** (commit `4577932fa`, pinned 2026-11-18) | **MIT** — Copyright (c) 2013-2026 The ggml authors (Georgi Gerganov and contributors) | <https://github.com/ggml-org/llama.cpp> |
| Qwen3-4B-Instruct-2507 GGUF (Q4_K_M, ~2.4 GB, primary) | `unsloth/Qwen3-4B-Instruct-2507-GGUF`, sha256 `3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597` | **Apache-2.0** — Copyright (c) 2025 Alibaba Cloud | <https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF> |
| Qwen2.5-3B-Instruct GGUF (Q4_K_M, ~2.0 GB, low-RAM fallback) | `Qwen/Qwen2.5-3B-Instruct-GGUF`, sha256 `626b4a6678b86442240e33df819e00132d3ba7dddfe1cdc4fbb18e0a9615c62d` | **Apache-2.0** — Copyright (c) 2024 Alibaba Cloud | <https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF> |
| Qwen3-0.6B GGUF (Q8_0, ~0.6 GB, smoke-test model) | `Qwen/Qwen3-0.6B-GGUF`, sha256 `9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031` | **Apache-2.0** — Copyright (c) 2025 Alibaba Cloud | <https://huggingface.co/Qwen/Qwen3-0.6B-GGUF> |

Qwen model outputs are generated by the models, not by DiffLens; DiffLens'
severity classification never depends on LLM output (rule-based, deterministic).

## 4. Frontend packages (bundled into the UI assets)

| Package | Version | License | Source |
|---|---|---|---|
| react / react-dom | 19.3.1 | MIT | <https://react.dev> |
| vite | 8.3.8 | MIT | <https://vite.dev> |
| typescript | 7.0.2 | Apache-2.0 | <https://www.typescriptlang.org> |
| tailwindcss + @tailwindcss/vite | 4.3.5 | MIT | <https://tailwindcss.com> |
| @tanstack/react-query | 5.108.0 | MIT | <https://tanstack.com/query> |
| @tanstack/react-router | 1.170.46 | MIT | <https://tanstack.com/router> |
| @tanstack/react-virtual | 3.14.26 | MIT | <https://tanstack.com/virtual> |
| i18next | 27.1.5 | MIT | <https://www.i18next.com> |
| react-i18next | 18.0.2 | MIT | <https://react.i18next.com> |
| lucide-react | 1.74.0 | ISC — Copyright (c) lucide authors | <https://lucide.dev> |
| @tauri-apps/api | 2.12.5 | MIT OR Apache-2.0 | <https://tauri.app> |
| @tauri-apps/plugin-dialog / plugin-shell | 2.8.4 / 2.6.0 | MIT OR Apache-2.0 | <https://github.com/tauri-apps/plugins-workspace> |
| class-variance-authority | 0.7.1 | Apache-2.0 | <https://github.com/huntabyte/cva> |
| clsx | 2.1.1 | MIT | <https://github.com/lukeed/clsx> |
| tailwind-merge | 3.7.0 | MIT | <https://github.com/dcastil/tailwind-merge> |

Dev-only tooling (not shipped in installers): vitest 5.1.1 (MIT), esbuild
0.28.2 (MIT), oxlint 1.94.0 (MIT), @types/* (MIT).

## 5. Apache-2.0 NOTICE appendix

The Apache-2.0-licensed components above (`similar`, `tokenizers`,
`class-variance-authority`, TypeScript, the Apache-option of dual-licensed
crates if exercised, Tesseract when the user installs it) carry copyright and
NOTICE contents as follows; the full Apache License 2.0 text applies from
<https://www.apache.org/licenses/LICENSE-2.0.txt>:

- `similar` — Copyright (c) 2015-2025 Armin Ronacher. NOTICE: none additional
  shipped by the crate beyond copyright in its LICENSE file.
- `tokenizers` — Copyright (c) 2019 The HuggingFace Inc. team. NOTICE: none
  additional shipped by the crate beyond copyright in its LICENSE file.
- `class-variance-authority` — Copyright (c) 2023 Huntabyte & community.
- `typescript` — Copyright (c) Microsoft Corporation.
- Tesseract OCR — Copyright (c) 2015 Google, Inc. (only if the user installs
  it separately; DiffLens never bundles it).
- Qwen3-4B-Instruct-2507 / Qwen2.5-3B-Instruct / Qwen3-0.6B — Copyright (c)
  Alibaba Cloud (models downloaded at runtime, §3.2).

## 6. Regenerating this file

Pins and checksums live in:

- `scripts/fetch-models.sh` (e5-small ONNX export commit + SHA-256)
- `scripts/fetch-pdfium.sh` (PDFium build tag + SHA-256)
- `resources/manifests/llm-manifest.json` (llama-server + GGUF pins, SHA-256,
  licenses)
- `Cargo.lock` / `package-lock.json` (crate and package versions)

Re-run the audit and update the tables above when bumping any pin.
