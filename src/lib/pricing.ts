import type { ModelStat } from "./ipc";

/** 모델 계열별 USD/MTok 단가 (claude-api 레퍼런스, 2026-06 기준).
 *  cache write = 입력가 × 1.25 (5분 TTL), cache read = 입력가 × 0.1. */
interface Price {
  input: number;
  output: number;
}

const FAMILY_PRICE: Record<string, Price> = {
  opus: { input: 5, output: 25 },
  sonnet: { input: 3, output: 15 },
  haiku: { input: 1, output: 5 },
  fable: { input: 10, output: 50 },
  mythos: { input: 10, output: 50 },
};

/** 모델 id에서 계열 추출 → 단가. 미인식 모델은 null(비용 미산정). */
export function priceFor(model: string): Price | null {
  const m = model.toLowerCase();
  for (const key of Object.keys(FAMILY_PRICE)) {
    if (m.includes(key)) return FAMILY_PRICE[key];
  }
  return null;
}

/** 모델 한 종의 USD 비용. 단가 없으면 null. */
export function modelCost(s: ModelStat): number | null {
  // Claude's public token pricing is the only supported heuristic here. A cache-incomplete
  // sample must stay partial rather than lending a precise-looking dollar amount to mixed logs.
  if (s.provider !== "claude" || (s.cache_unknown_messages ?? 0) > 0) return null;
  const p = priceFor(s.model);
  if (!p) return null;
  return (
    (s.input_tokens * p.input +
      s.output_tokens * p.output +
      s.cache_creation_tokens * p.input * 1.25 +
      s.cache_read_tokens * p.input * 0.1) /
    1e6
  );
}

/** 전체 모델 합산 USD 비용 + 단가 미산정 모델 존재 여부. */
export function totalCost(models: ModelStat[]): { usd: number; partial: boolean } {
  let usd = 0;
  let partial = false;
  for (const m of models) {
    const c = modelCost(m);
    if (c == null) partial = true;
    else usd += c;
  }
  return { usd, partial };
}

/** USD 포맷 — 1달러 미만은 센트까지, 이상은 천단위 구분. */
export function fmtUsd(n: number): string {
  if (n < 1) return `$${n.toFixed(2)}`;
  if (n < 1000) return `$${n.toFixed(2)}`;
  return `$${n.toLocaleString(undefined, { maximumFractionDigits: 0 })}`;
}
