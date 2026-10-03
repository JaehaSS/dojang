/** 에이전트(하네스) 프리셋·라벨 단일 원천.
 *  백엔드 agent::PRESETS(src-tauri/src/agent/mod.rs)와 수동 동기 유지. */
export const AGENT_PRESETS: { key: string; label: string; badge: string }[] = [
  { key: "claude", label: "Claude Code", badge: "Claude" },
  { key: "codex", label: "Codex", badge: "Codex" },
  { key: "agy", label: "Antigravity · Gemini", badge: "Agy" },
];

const DEPRECATED_AGENT_ALIASES: Readonly<Record<string, string>> = {
  gemini: "agy",
  opencode: "claude",
};

/** 저장된 선택에서 지원 종료된 CLI를 현재 대체 CLI로 치환하고 중복을 제거한다. */
export const normalizeAgentSelection = (agents: readonly string[]): string[] => {
  const normalized = new Set<string>();
  for (const agent of agents) {
    const trimmed = agent.trim();
    if (trimmed) normalized.add(DEPRECATED_AGENT_ALIASES[trimmed] ?? trimmed);
  }
  return normalized.size ? [...normalized] : ["claude"];
};

/** 컴포저가 쓰는 선택 — 1.0은 에이전트 하나다. 앙상블 시절 저장된 다중 선택은 첫 항목만 남긴다. */
export const singleAgentSelection = (agents: readonly string[]): string[] =>
  normalizeAgentSelection(agents).slice(0, 1);

/** 전체 라벨 — 피커 칩처럼 공간 여유 있는 곳. */
export const labelFor = (agent: string) =>
  AGENT_PRESETS.find((p) => p.key === agent)?.label ?? `커스텀: ${agent}`;

/** 짧은 배지 라벨 — 사이드바 목록·워크스페이스 헤더·홈 목록.
 *  커스텀 에이전트는 원문 그대로, 구버전 행(agent NULL)은 null을 반환해 배지를 생략시킨다. */
export const badgeLabelFor = (agent?: string | null): string | null => {
  const a = agent?.trim();
  if (!a) return null;
  if (a === "gemini") return "Gemini";
  return AGENT_PRESETS.find((p) => p.key === a)?.badge ?? a;
};
