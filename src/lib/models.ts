/** 에이전트 CLI(--model/-m)에 전달할 알려진 모델 후보.
 *  best-effort 시드 — 자유입력이 항상 폴백이므로 목록이 낡아도 사용자를 막지 않는다.
 *  id = CLI에 그대로 넘길 문자열, label = 드롭다운 표시명(선택). */
export interface ModelOption {
  id: string;
  label?: string;
  reasoningEfforts?: readonly ReasoningEffort[];
}

export const CODEX_REASONING_EFFORTS = [
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
] as const;

export type ReasoningEffort = (typeof CODEX_REASONING_EFFORTS)[number];

const STANDARD_REASONING_EFFORTS: readonly ReasoningEffort[] = [
  "low",
  "medium",
  "high",
  "xhigh",
];
const MAX_REASONING_EFFORTS: readonly ReasoningEffort[] = [
  ...STANDARD_REASONING_EFFORTS,
  "max",
];
const CLAUDE_REASONING_EFFORTS: readonly ReasoningEffort[] = [
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
];
const ANTIGRAVITY_REASONING_EFFORTS: readonly ReasoningEffort[] = ["low", "medium", "high"];

// `agy models`(CLI 1.3.1) 목록과 맞춘다. 목록에 없는 id는 agy가 즉시 오류로 끝낸다.
const ANTIGRAVITY_MODELS: ModelOption[] = [
  { id: "gemini-3.8-flash-high", label: "Gemini 3.8 Flash · High" },
  { id: "gemini-3.8-flash-medium", label: "Gemini 3.8 Flash · Medium" },
  { id: "gemini-3.8-flash-low", label: "Gemini 3.8 Flash · Low" },
  { id: "gemini-3.7-flash-high", label: "Gemini 3.7 Flash · High" },
  { id: "gemini-3.7-flash-medium", label: "Gemini 3.7 Flash · Medium" },
  { id: "gemini-3.7-flash-low", label: "Gemini 3.7 Flash · Low" },
  { id: "gemini-3.6-flash-high", label: "Gemini 3.6 Flash · High" },
  { id: "gemini-3.6-flash-medium", label: "Gemini 3.6 Flash · Medium" },
  { id: "gemini-3.6-flash-low", label: "Gemini 3.6 Flash · Low" },
  { id: "gemini-3.1-pro-high", label: "Gemini 3.1 Pro · High" },
  { id: "gemini-3.1-pro-low", label: "Gemini 3.1 Pro · Low" },
  { id: "claude-opus-5-5-high", label: "Claude Opus 5.5 · High" },
  { id: "claude-sonnet-5-5-high", label: "Claude Sonnet 5.5 · High" },
  { id: "gpt-oss-120b-medium", label: "GPT-OSS 120B · Medium" },
];

/** 벤더(AgentPicker의 프리셋 key)별 후보 목록. 키는 agent::PRESETS와 동기. */
export const AGENT_MODEL_CATALOG: Record<string, ModelOption[]> = {
  // Claude Code: 별칭(opus/sonnet/haiku) + 정식 ID 모두 --model 허용.
  // reasoningEfforts: claude CLI --effort는 모델과 무관하게 전역 검증(low/medium/high/xhigh/max).
  claude: [
    { id: "opus", label: "Opus (별칭)", reasoningEfforts: CLAUDE_REASONING_EFFORTS },
    { id: "sonnet", label: "Sonnet (별칭)", reasoningEfforts: CLAUDE_REASONING_EFFORTS },
    { id: "haiku", label: "Haiku (별칭)", reasoningEfforts: CLAUDE_REASONING_EFFORTS },
    { id: "claude-opus-5", label: "Opus 5", reasoningEfforts: CLAUDE_REASONING_EFFORTS },
    // `[1m]`은 컨텍스트 변형 접미사 — 별개 모델이 아니라 같은 모델의 1M 컨텍스트 판이다.
    {
      id: "claude-opus-5[1m]",
      label: "Opus 5 · 1M 컨텍스트",
      reasoningEfforts: CLAUDE_REASONING_EFFORTS,
    },
    { id: "claude-opus-4-8", label: "Opus 4.8", reasoningEfforts: CLAUDE_REASONING_EFFORTS },
    { id: "claude-sonnet-5", label: "Sonnet 5", reasoningEfforts: CLAUDE_REASONING_EFFORTS },
    {
      id: "claude-haiku-4-5-20251001",
      label: "Haiku 4.5",
      reasoningEfforts: CLAUDE_REASONING_EFFORTS,
    },
    { id: "claude-fable-5", label: "Fable 5", reasoningEfforts: CLAUDE_REASONING_EFFORTS },
  ],
  // OpenAI Codex CLI (-m): 설치된 Codex의 현재 작업 모델 목록.
  codex: [
    { id: "gpt-6-astra", label: "GPT-6 Astra", reasoningEfforts: STANDARD_REASONING_EFFORTS },
    {
      id: "gpt-5.6-sol",
      label: "GPT-5.6 Sol",
      reasoningEfforts: CODEX_REASONING_EFFORTS,
    },
    {
      id: "gpt-5.6-terra",
      label: "GPT-5.6 Terra",
      reasoningEfforts: CODEX_REASONING_EFFORTS,
    },
    {
      id: "gpt-5.6-luna",
      label: "GPT-5.6 Luna",
      reasoningEfforts: MAX_REASONING_EFFORTS,
    },
    { id: "gpt-5.5", label: "GPT-5.5", reasoningEfforts: STANDARD_REASONING_EFFORTS },
    { id: "gpt-5.4", label: "GPT-5.4", reasoningEfforts: STANDARD_REASONING_EFFORTS },
    {
      id: "gpt-5.4-mini",
      label: "GPT-5.4 Mini",
      reasoningEfforts: STANDARD_REASONING_EFFORTS,
    },
    {
      id: "gpt-5.3-codex-spark",
      label: "GPT-5.3 Codex Spark",
      reasoningEfforts: STANDARD_REASONING_EFFORTS,
    },
  ],
  // Gemini CLI (-m).
  gemini: [
    { id: "gemini-3-pro", label: "Gemini 3 Pro" },
    { id: "gemini-2.5-pro", label: "Gemini 2.5 Pro" },
    { id: "gemini-2.5-flash", label: "Gemini 2.5 Flash" },
  ],
  // Antigravity(Gemini 백엔드) (-m).
  agy: ANTIGRAVITY_MODELS,
};

/** 벤더 key → 후보 목록. 미등록 벤더/빈 문자열은 빈 배열(자유입력만). */
export function modelsForAgent(agent: string): ModelOption[] {
  return AGENT_MODEL_CATALOG[agent] ?? [];
}

/** 실제로 관측된 실행 모델 한 건 (`db::ObservedModel` 미러). */
export interface ObservedModel {
  agent: string;
  model: string;
  last_used_at: number;
}

/**
 * 하드코딩 카탈로그 + 실제 관측 목록. 카탈로그에 없는 모델을 뒤에 덧붙인다.
 *
 * claude·codex CLI 모두 "사용 가능한 모델 목록"을 노출하지 않아(`--model`/`-m`은 입력 전용)
 * 신모델은 카탈로그를 손으로 고치기 전까지 드롭다운에 뜨지 않는다. 관측을 합치면 **자유입력으로
 * 한 번 쓴 모델이 다음부터 목록에 남는다** — 완전 자동은 아니고 "한 번 쓰면 기억한다"에 가깝다.
 *
 * 중복은 카탈로그 쪽을 남긴다(사람이 붙인 라벨이 원시 id보다 읽기 좋다).
 */
export function modelsForAgentWithObserved(
  agent: string,
  observed: readonly ObservedModel[],
): ModelOption[] {
  const catalog = modelsForAgent(agent);
  const known = new Set(catalog.map((option) => option.id));
  const extra = observed
    .filter((entry) => entry.agent === agent && !known.has(entry.model))
    .sort((left, right) => right.last_used_at - left.last_used_at)
    // 같은 모델이 여러 번 관측돼도 한 번만 — DB가 GROUP BY로 이미 줄이지만 방어적으로.
    .filter((entry, index, all) => all.findIndex((x) => x.model === entry.model) === index)
    .map((entry) => ({
      id: entry.model,
      label: `${entry.model} (사용 기록)`,
      reasoningEfforts: reasoningEffortsForModel(agent, entry.model),
    }));
  return [...catalog, ...extra];
}

export function reasoningEffortsForModel(
  agent: string,
  model: string,
): readonly ReasoningEffort[] {
  const normalizedAgent = agent.trim();
  if (["agy", "gemini", "antigravity"].includes(normalizedAgent)) {
    return ANTIGRAVITY_REASONING_EFFORTS;
  }
  if (normalizedAgent !== "codex" && normalizedAgent !== "claude") return [];
  const option = modelsForAgent(normalizedAgent).find((candidate) => candidate.id === model.trim());
  if (option) return option.reasoningEfforts ?? STANDARD_REASONING_EFFORTS;
  // claude CLI는 미등록/커스텀 모델에도 --effort를 모델 무관하게 전역 검증하므로 동일 목록을 노출.
  if (normalizedAgent === "claude") return CLAUDE_REASONING_EFFORTS;
  return STANDARD_REASONING_EFFORTS;
}

export function normalizeReasoningEffort(
  agent: string,
  model: string,
  effort: string,
): string {
  if (!effort.trim()) return "";
  const supported = reasoningEffortsForModel(agent, model);
  return supported.includes(effort.trim() as ReasoningEffort) ? effort.trim() : "";
}
