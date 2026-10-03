import { fmtPct } from "./format";

/** Only the same observed cohort may contribute numerator and denominator. */
export function cacheReadRate(stats: {
  cache_observed_messages?: number;
  cache_observed_input_tokens?: number;
  cache_observed_read_tokens?: number;
}): string {
  const input = stats.cache_observed_input_tokens;
  const read = stats.cache_observed_read_tokens;
  if (!(stats.cache_observed_messages && stats.cache_observed_messages > 0)
    || input == null || read == null || !Number.isFinite(input) || !Number.isFinite(read)
    || input < 0 || read < 0 || read > input) return "미관측";
  return input === 0 ? "입력 없음" : fmtPct(read, input);
}
