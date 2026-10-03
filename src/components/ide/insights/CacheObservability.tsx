import type { Insights } from "../../../lib/ipc";
import { cacheReadRate } from "./cache";
import { fmtInt, fmtTokens } from "./format";

type Data = Pick<Insights, "cache_observed_messages" | "cache_unknown_messages"
  | "cache_observed_input_tokens" | "cache_observed_read_tokens" | "cache_days" | "prompt_injection">;

export function CacheObservability({ data }: { data: Data }) {
  const prompt = data.prompt_injection;
  const days = data.cache_days ?? [];
  return <>
    <div className="bg-surface border border-border rounded-lg p-4 mb-2.5">
      <div className="text-sm font-medium mb-2">캐시 읽기 비율 {cacheReadRate(data)}</div>
      <div className="text-xs text-text-muted">
        로컬 Claude·Codex CLI 기록 · 관측 {fmtInt(data.cache_observed_messages ?? 0)} · 미관측 {fmtInt(data.cache_unknown_messages ?? 0)}
      </div>
      <p className="text-xs text-text-muted mt-2">관측된 입력 토큰 중 재사용된 비율입니다. Dojang 밖에서 실행한 세션도 포함하며, 누락 구간의 사용량은 미집계일 수 있습니다.</p>
      <details className="mt-3">
        <summary className="text-xs text-text-secondary cursor-pointer">일별 캐시 추이 (최근 30활동일)</summary>
        {days.length === 0 ? <p className="text-sm text-text-muted mt-2">미관측</p> :
          <div className="overflow-x-auto mt-2"><table className="w-full text-xs">
            <thead className="text-text-muted"><tr>
              <th className="text-left py-2">날짜</th><th className="text-right">캐시 읽기 / 입력</th><th className="text-right">읽기 비율</th><th className="text-right">관측 / 미관측</th>
            </tr></thead>
            <tbody>{days.slice(-30).map(day => <tr key={day.date} className="border-t border-border">
              <td className="py-2">{day.date}</td>
              <td className="text-right">{day.observed_messages > 0 ? `${fmtTokens(day.cache_read_tokens)} / ${fmtTokens(day.input_tokens)}` : "미관측"}</td>
              <td className="text-right">{cacheReadRate({ cache_observed_messages: day.observed_messages, cache_observed_input_tokens: day.input_tokens, cache_observed_read_tokens: day.cache_read_tokens })}</td>
              <td className="text-right">{fmtInt(day.observed_messages)} / {fmtInt(day.unknown_messages)}</td>
            </tr>)}</tbody>
          </table></div>}
      </details>
    </div>
    <div className="bg-surface border border-border rounded-lg p-4 mb-2.5">
      <div className="text-sm font-medium mb-2">Dojang 주 대화 접수 입력</div>
      {prompt == null ? <p className="text-sm text-text-muted">조회 불가</p> : <>
        {prompt.observed_turns === 0 ? <p className="text-sm text-text-muted">미수집</p> :
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 text-sm">
            <span>원문 {fmtInt(prompt.user_bytes)} B</span><span>접수 입력 {fmtInt(prompt.sent_user_bytes)} B</span>
            <span>고정 지침 {fmtInt(prompt.instruction_bytes)} B</span><span>지침 변경 {fmtInt(prompt.instruction_changes)}회</span>
            <span>스킬 +{fmtInt(prompt.skill_added_bytes)} B</span><span>목표 +{fmtInt(prompt.goal_added_bytes)} B</span>
            <span>인계 +{fmtInt(prompt.capsule_added_bytes)} B</span><span>위키 근거 +{fmtInt(prompt.vault_added_bytes)} B</span>
          </div>}
        <p className="text-xs text-text-muted mt-2">접수 {fmtInt(prompt.accepted_turns)} · 관측 {fmtInt(prompt.observed_turns)} · 미수집 {fmtInt(prompt.unknown_turns)}</p>
      </>}
      <p className="text-xs text-text-muted mt-2">텍스트 바이트 측정이며 토큰·청구량이 아닙니다. 접수 입력과 고정 지침은 일부 겹칠 수 있습니다. 이미지, CLI 자체 메모리·도구 정의, 토론·runner 입력은 제외합니다. 지침 변경 횟수만으로 캐시 실패 원인을 판단할 수 없습니다.</p>
    </div>
  </>;
}
