import type { ReactNode } from "react";
import { runStateInfo, ticketStateInfo, vendorLabel, type Tone } from "../../../lib/pipeline";

export const TONE_CLASS: Record<Tone, string> = {
  neutral: "text-text-muted",
  running: "text-status-running",
  attention: "text-status-awaiting",
  success: "text-status-done",
  failed: "text-status-failed",
};

export const BTN_PRIMARY = "h-8 px-3 rounded-md bg-primary text-bg text-sm font-medium hover:opacity-90 disabled:opacity-40";
export const BTN_OUTLINE = "h-8 px-3 rounded-md border border-border text-sm text-text-secondary hover:text-text disabled:opacity-40";
export const BTN_DANGER = "h-8 px-3 rounded-md border border-status-failed text-sm text-status-failed hover:opacity-80 disabled:opacity-40";
export const INPUT = "rounded-md border border-border bg-surface px-2 py-1 text-sm text-text focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary";

/** 색만으로 상태를 말하지 않는다 — 라벨 텍스트를 항상 함께 둔다(DESIGN.md Do #2). */
export function StateBadge({ kind, state }: { kind: "run" | "ticket"; state: string }) {
  const info = kind === "run" ? runStateInfo(state) : ticketStateInfo(state);
  return (
    <span className={`shrink-0 rounded border border-border px-1.5 py-0.5 text-xs ${TONE_CLASS[info.tone]}`} data-state={state}>
      {info.label}
    </span>
  );
}

export function VendorTag({ vendor }: { vendor: string | null | undefined }) {
  return <span className="text-sm">{vendorLabel(vendor)}</span>;
}

export function Section({ title, children, aside }: { title: string; children: ReactNode; aside?: ReactNode }) {
  return (
    <section className="mb-6" aria-label={title}>
      <div className="mb-2 flex items-center gap-2">
        <h2 className="flex-1 text-sm font-medium uppercase tracking-wide text-text-muted">{title}</h2>
        {aside}
      </div>
      {children}
    </section>
  );
}

export function ErrorLine({ message }: { message: string | null }) {
  return message ? (
    <p role="alert" className="mb-2 whitespace-pre-wrap break-words text-sm text-status-failed">
      {message}
    </p>
  ) : null;
}
