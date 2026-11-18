//! TypeScript mirrors of the Rust serde types the backend sends over IPC.
//!
//! Source of truth: `core/src/model.rs`, `core/src/pipeline.rs` (Stage),
//! `src-tauri/src/state.rs` (JobSnapshot/JobState) and
//! `src-tauri/src/commands.rs` (event payloads, LLM/OCR status).
//! Serde emits lowercase unit variants (`rename_all = "lowercase"`) and
//! `snake_case` struct fields; keep both in sync or the UI silently breaks.

// ---------------------------------------------------------------------------
// Diff model (core/src/model.rs)
// ---------------------------------------------------------------------------

export type DocFormat = "pdf" | "docx" | "txt" | "xlsx" | "pptx";

export type SpanOp = "equal" | "insert" | "delete";

export type ChangeKind = "added" | "removed" | "modified" | "moved" | "equal";

export type Severity = "info" | "low" | "medium" | "high" | "critical";

/** Inline word-level diff span between two aligned blocks. */
export interface DiffSpan {
  op: SpanOp;
  text: string;
}

/** One fired heuristic rule, with the evidence trail for audit. */
export interface HeuristicFlag {
  /** Stable rule id, e.g. `MONEY_CHANGED`. */
  rule_id: string;
  /** i18n label key, e.g. `flag.moneyChanged`. */
  label_key: string;
  old_value: string | null;
  new_value: string | null;
  /** Normalized numeric delta when computable (e.g. -0.5 = −50 %). */
  delta: number | null;
  detail: string | null;
}

export interface Change {
  /** Stable change id (`c0`, `c1`, ...). */
  id: string;
  kind: ChangeKind;
  old_block_ids: string[];
  new_block_ids: string[];
  old_page: number | null;
  new_page: number | null;
  /** Inline word-level diff (present for Modified/Moved pairs). */
  spans: DiffSpan[];
  /** Cosine similarity of the aligned pair (0..1), when computed. */
  similarity: number | null;
  /** Position delta for moved blocks (dx, dy). */
  move_delta: [number, number] | null;
  severity: Severity | null;
  flags: HeuristicFlag[];
}

export interface DiffStats {
  added: number;
  removed: number;
  modified: number;
  moved: number;
  equal: number;
}

/** Ordered rule firings for one enriched change — the audit trail. */
export interface AuditEntry {
  change_id: string;
  severity: Severity;
  rules: HeuristicFlag[];
}

export interface DiffResult {
  old_path: string;
  new_path: string;
  format: DocFormat;
  changes: Change[];
  stats: DiffStats;
  /** Whether real (ort) embeddings were used for alignment. */
  embeddings_used: boolean;
  audit: AuditEntry[];
}

// ---------------------------------------------------------------------------
// Jobs (src-tauri/src/state.rs + pipeline Stage)
// ---------------------------------------------------------------------------

export type JobState = "running" | "done" | "failed" | "cancelled";

/** Stages emitted on `diff://progress` plus terminal job states. */
export type DiffStage =
  | "queued"
  | "extract"
  | "ocr"
  | "align"
  | "diff"
  | "enrich"
  | "done"
  | "failed"
  | "cancelled";

export interface JobSnapshot {
  job_id: string;
  state: JobState;
  stage: string;
  percent: number;
  error: string | null;
  result: DiffResult | null;
}

// ---------------------------------------------------------------------------
// Event payloads (src-tauri/src/commands.rs)
// ---------------------------------------------------------------------------

/** `diff://progress` */
export interface ProgressPayload {
  job_id: string;
  stage: string;
  percent: number;
}

/** `llm://download` — server install + model download progress. */
export interface LlmDownloadPayload {
  job_id: string;
  /** `server` or `model`. */
  kind: string;
  /** Model id (empty for server installs). */
  model_id: string;
  /** `running` | `done` | `failed` | `cancelled`. */
  state: string;
  bytes_done: number;
  bytes_total: number | null;
  percent: number;
  error: string | null;
}

/** `llm://token` — one streaming token delta of an explanation. */
export interface LlmTokenPayload {
  job_id: string;
  delta: string;
}

/** `llm://done` — terminal event for an explanation job. */
export interface LlmDonePayload {
  job_id: string;
  ok: boolean;
  cancelled: boolean;
  summary: string | null;
  impact: string | null;
  error: string | null;
  elapsed_ms: number;
}

// ---------------------------------------------------------------------------
// Status snapshots (llm_status / ocr_status commands)
// ---------------------------------------------------------------------------

export interface LlmModelStatus {
  id: string;
  /** `primary` | `fallback` | `test`. */
  role: string;
  present: boolean;
  /** Full size on disk once downloaded. */
  bytes: number;
  downloaded_bytes: number;
  license: string;
  verified: boolean;
}

export interface LlmStatus {
  server_tag: string;
  /** Manifest-pinned server installed under app data. */
  server_installed: boolean;
  /** A llama-server found on PATH (fallback). */
  server_on_path: boolean;
  /** Server currently running (healthy loopback sidecar). */
  server_running: boolean;
  models: LlmModelStatus[];
}

export interface OcrStatus {
  /** True when tesseract + the Vietnamese traineddata are both usable. */
  available: boolean;
  binary: string | null;
  langs: string[];
  has_vie: boolean;
}
