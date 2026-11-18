//! Typed Tauri command/event layer — the only module allowed to touch IPC.
//!
//! Command names/signatures mirror `src-tauri/src/commands.rs` exactly.
//! When the app runs in a plain browser (`npm run dev` without Tauri),
//! every wrapper rejects with `TauriRequiredError` so screens can show a
//! "requires Tauri runtime" banner instead of crashing.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type Event, type UnlistenFn } from "@tauri-apps/api/event";
import { open as dialogOpen, save as dialogSave } from "@tauri-apps/plugin-dialog";
import type {
  JobSnapshot,
  LlmDonePayload,
  LlmDownloadPayload,
  LlmStatus,
  OcrStatus,
  ProgressPayload,
  LlmTokenPayload,
  ReportFormat,
  SessionSummary,
  StoredSession,
} from "./types";

/** True when running inside the Tauri webview (vs plain browser dev). */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Rejected by every command wrapper outside the Tauri runtime. */
export class TauriRequiredError extends Error {
  constructor() {
    super("This action requires the Tauri desktop runtime.");
    this.name = "TauriRequiredError";
  }
}

/** Serializable args map (Tauri v2 IPC). */
type InvokeArgs = Record<string, unknown>;

async function invoke<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  if (!isTauri()) throw new TauriRequiredError();
  return tauriInvoke<T>(cmd, args);
}

/** Typed `listen` that returns the unlisten cleanup. */
async function listen<P>(
  event: string,
  handler: (payload: P, event: Event<unknown>) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) throw new TauriRequiredError();
  return tauriListen<P>(event, (e) => handler(e.payload, e));
}

// ---------------------------------------------------------------------------
// Progress / LLM event names (keep in sync with commands.rs constants)
// ---------------------------------------------------------------------------

export const EVENTS = {
  progress: "diff://progress",
  llmDownload: "llm://download",
  llmToken: "llm://token",
  llmDone: "llm://done",
} as const;

// ---------------------------------------------------------------------------
// Typed event subscription helpers
// ---------------------------------------------------------------------------

export function onDiffProgress(
  handler: (payload: ProgressPayload) => void,
): Promise<UnlistenFn> {
  return listen<ProgressPayload>(EVENTS.progress, handler);
}

export function onLlmDownload(
  handler: (payload: LlmDownloadPayload) => void,
): Promise<UnlistenFn> {
  return listen<LlmDownloadPayload>(EVENTS.llmDownload, handler);
}

export function onLlmToken(
  handler: (payload: LlmTokenPayload) => void,
): Promise<UnlistenFn> {
  return listen<LlmTokenPayload>(EVENTS.llmToken, handler);
}

export function onLlmDone(
  handler: (payload: LlmDonePayload) => void,
): Promise<UnlistenFn> {
  return listen<LlmDonePayload>(EVENTS.llmDone, handler);
}

// ---------------------------------------------------------------------------
// Diff commands (phase 03/06)
// ---------------------------------------------------------------------------

/**
 * Start a diff job. `ocrEnabled` maps to the backend `ocr_enabled` param:
 * pass the stored settings preference; `undefined`/`null` keeps the
 * backend default (auto) behavior.
 */
export function runDiff(
  oldPath: string,
  newPath: string,
  ocrEnabled?: boolean | null,
): Promise<JobSnapshot> {
  return invoke<JobSnapshot>("run_diff", {
    oldPath,
    newPath,
    ocrEnabled: ocrEnabled ?? null,
  });
}

export function getDiff(jobId: string): Promise<JobSnapshot | null> {
  return invoke<JobSnapshot | null>("get_diff", { jobId });
}

export function cancelDiff(jobId: string): Promise<boolean> {
  return invoke<boolean>("cancel_diff", { jobId });
}

export function ocrStatus(): Promise<OcrStatus> {
  return invoke<OcrStatus>("ocr_status");
}

// ---------------------------------------------------------------------------
// LLM commands (phase 05)
// ---------------------------------------------------------------------------

export function llmStatus(): Promise<LlmStatus> {
  return invoke<LlmStatus>("llm_status");
}

/** Install the manifest-pinned llama-server bundle. Returns the job id. */
export function llmInstallServer(): Promise<string> {
  return invoke<string>("llm_install_server");
}

/** Download a manifest-pinned model. Returns the job id (`model-<id>`). */
export function llmDownloadModel(modelId: string): Promise<string> {
  return invoke<string>("llm_download_model", { modelId });
}

/**
 * Explain ONE change of a finished diff job. Returns the job id
 * (`explain-<n>`); tokens stream on `llm://token`, the parsed result
 * lands on `llm://done`.
 */
export function explainChange(
  diffJobId: string,
  changeId: string,
  lang: string,
): Promise<string> {
  return invoke<string>("explain_change", { diffJobId, changeId, lang });
}

/** Cancel a download (`model-*`/`server-*`) or explain (`explain-*`) job. */
export function llmCancel(jobId: string): Promise<boolean> {
  return invoke<boolean>("llm_cancel", { jobId });
}

// ---------------------------------------------------------------------------
// Dialog plugin (phase 07 wizard file pickers)
// ---------------------------------------------------------------------------

/** Per-format file filters mirroring `DocFormat::from_extension`. */
const FILE_FILTERS = [
  { name: "Supported documents", extensions: ["pdf", "docx", "txt", "md", "xlsx", "xlsm", "pptx"] },
  { name: "PDF", extensions: ["pdf"] },
  { name: "Word", extensions: ["docx"] },
  { name: "Text", extensions: ["txt", "md"] },
  { name: "Excel", extensions: ["xlsx", "xlsm"] },
  { name: "PowerPoint", extensions: ["pptx"] },
];

/** Open the native file picker; resolves the picked path or null. */
export async function pickDocument(): Promise<string | null> {
  if (!isTauri()) throw new TauriRequiredError();
  const picked = await dialogOpen({
    multiple: false,
    directory: false,
    filters: FILE_FILTERS,
  });
  return typeof picked === "string" ? picked : null;
}

// ---------------------------------------------------------------------------
// Session history + report export (phase-08)
// ---------------------------------------------------------------------------

/** Recent comparisons, newest first (`session` rows, no result blobs). */
export function listSessions(limit = 20, offset = 0): Promise<SessionSummary[]> {
  return invoke<SessionSummary[]>("list_sessions", { limit, offset });
}

/** Delete one stored session; `true` when a row was removed. */
export function deleteSession(sessionId: string): Promise<boolean> {
  return invoke<boolean>("delete_session", { sessionId });
}

/** Full stored session incl. the deserialized result (`null` if corrupt). */
export function getSession(sessionId: string): Promise<StoredSession | null> {
  return invoke<StoredSession | null>("get_session", { sessionId });
}

/**
 * Render a report for `id` (a live `job-*` or persisted `session-*` id)
 * and write it to `targetPath` (from the save dialog). Returns bytes
 * written. `lang` picks the report labels (en/vi).
 */
export function exportReport(
  id: string,
  format: ReportFormat,
  targetPath: string,
  lang?: string,
): Promise<number> {
  return invoke<number>("export_report", {
    id,
    format,
    targetPath,
    lang: lang ?? null,
  });
}

/** Save-dialog filters per report format. */
const REPORT_FILTERS: Record<ReportFormat, { name: string; extensions: string[] }> = {
  html: { name: "HTML report", extensions: ["html"] },
  md: { name: "Markdown report", extensions: ["md"] },
};

/** Default export file name from the compared basenames. */
export function reportFileName(id: string, format: ReportFormat): string {
  const ext = format === "html" ? "html" : "md";
  return `difflens-${id.replace(/[^a-zA-Z0-9_-]+/g, "_")}.${ext}`;
}

/**
 * Native save dialog for a report export; resolves the chosen path or
 * null (cancelled). Rust writes the file (plugin-fs is not installed).
 */
export async function pickReportSavePath(
  id: string,
  format: ReportFormat,
): Promise<string | null> {
  if (!isTauri()) throw new TauriRequiredError();
  const picked = await dialogSave({
    title: "Export report",
    defaultPath: reportFileName(id, format),
    filters: [REPORT_FILTERS[format]],
  });
  return typeof picked === "string" ? picked : null;
}
