import { Icon } from "./icons";

/** 음성 파이프라인의 현재 국면 — Rust `voice://state` 의 phase 와 같은 어휘를 쓴다. */
export type VoicePhase = "idle" | "recording" | "transcribing";
export type VoiceHudMode = "command" | "dictation";

export interface VoiceHudState {
  phase: VoicePhase;
  mode: VoiceHudMode | null;
  /** 결과·에러 플래시 문구. 없으면 표시할 것이 없다는 뜻. */
  message: string | null;
  isError: boolean;
}

const MODE_LABEL: Record<VoiceHudMode, string> = {
  command: "커맨드",
  dictation: "받아쓰기",
};

/**
 * 하단 중앙 고정 오버레이. 녹음 중에는 어느 모드인지 항상 보인다 —
 * 커맨드로 말했는지 딕테이션으로 말했는지 모른 채 발화하면 결과를 예측할 수 없다.
 */
export function VoiceHUD({ state }: { state: VoiceHudState }) {
  const { phase, mode, message, isError } = state;
  if (phase === "idle" && !message) return null;

  const label =
    phase === "recording"
      ? `듣는 중 · ${mode ? MODE_LABEL[mode] : ""}`
      : phase === "transcribing"
        ? "전사 중…"
        : message;

  return (
    <div className="pointer-events-none fixed bottom-6 left-1/2 z-50 -translate-x-1/2">
      <div
        className={`flex items-center gap-2 rounded-lg border bg-surface px-3 py-2 text-sm shadow-e2 ${
          isError && phase === "idle"
            ? "border-dangerborder text-status-failed"
            : "border-border text-text-secondary"
        }`}
        role="status"
        aria-live="polite"
      >
        <span className={phase === "recording" ? "text-status-failed animate-pulse" : ""}>
          <Icon name={phase === "recording" ? "mic" : isError && phase === "idle" ? "x" : "sparkle"} />
        </span>
        <span className="max-w-[28rem] truncate">{label}</span>
      </div>
    </div>
  );
}
