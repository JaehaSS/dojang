import { useMemo, type ReactElement } from "react";
import { buildContextReport, TOOL_GROUP_LABELS, type ContextReportEvent } from "../../lib/context-breakdown";

interface Props {
  events: readonly ContextReportEvent[];
}

const number = (value: number): string => value.toLocaleString("ko-KR");
const stateLabel = (state: string | null): string => ({
  running: "진행 중", done: "완료", failed: "실패", stopped: "중단됨", unknown: "상태 미상",
}[state ?? "unknown"] ?? "상태 미상");

/** 현재 점유량과 기록의 크기는 서로 다른 단위·시점이다. 크기로 토큰 비율을 만들지 않는다. */
export function ContextDetails({ events }: Props): ReactElement {
  const report = useMemo(() => buildContextReport(events), [events]);
  const groups = Object.entries(TOOL_GROUP_LABELS).flatMap(([key, label]) => {
    const tools = report.tools.filter((tool) => tool.group === key);
    return tools.length === 0 ? [] : [{ key, label, tools }];
  });
  const injectionRows = report.injection == null ? [] : [
    ["사용자 입력", report.injection.userBytes],
    ["앱 지침", report.injection.instructionBytes],
    ["스킬 추가 내용", report.injection.skillBytes],
    ["작업 목표", report.injection.goalBytes],
    ["작업 요약", report.injection.capsuleBytes],
    ["지식창고 자료", report.injection.vaultBytes],
  ] as const;

  return (
    <section aria-label="컨텍스트 상세" className="text-sm text-text-secondary">
      <div className="space-y-5 px-4 py-4">
        <section aria-label="마지막 요청의 입력 구성">
          <h3 className="text-sm font-medium text-text">마지막 요청의 입력 구성</h3>
          <p className="mt-1 text-xs leading-relaxed">앱이 측정한 입력의 크기입니다. 지침 파일과 도구 정의를 포함한 전체 토큰 구성은 제공되지 않습니다.</p>
          {report.injection ? <>
            <dl className="mt-2 divide-y divide-border border-y border-border text-xs">
              {injectionRows.map(([label, bytes]) => <div key={label} className="flex items-baseline justify-between gap-3 py-1.5">
                <dt>{label}</dt><dd className="shrink-0 tabular-nums text-text">{number(bytes)} B</dd>
              </div>)}
            </dl>
            <p className="mt-1.5 text-xs">전송한 사용자 메시지 {number(report.injection.sentUserBytes)} B · 앱 지침은 실행 방식에 따라 별도 전달됩니다.</p>
          </> : <p className="mt-2 rounded border border-border px-3 py-2 text-xs">이 요청에는 입력 크기 기록이 없습니다.</p>}
        </section>

        <section aria-label="대화와 도구 기록">
          <h3 className="text-sm font-medium text-text">대화와 도구 기록</h3>
          <p className="mt-1 text-xs leading-relaxed">마지막 컨텍스트 비우기 이후 확보한 메인 세션 기록입니다. 압축 후 남아 있는 구성이나 현재 토큰 점유율을 뜻하지 않습니다.</p>
          <dl className="mt-2 border-y border-border py-2 text-xs">
            <div className="flex justify-between gap-3"><dt>대화 텍스트</dt><dd className="tabular-nums text-text">{number(report.messages.count)}건 · {number(report.messages.chars)}자</dd></div>
          </dl>
          {groups.map(({ key, label, tools }) => {
            const calls = tools.reduce((sum, tool) => sum + tool.calls, 0);
            const chars = tools.reduce((sum, tool) => sum + tool.chars, 0);
            const unknown = tools.reduce((sum, tool) => sum + tool.unknownSizes, 0);
            const orphanResults = tools.filter((tool) => tool.calls === 0).reduce((sum, tool) => sum + tool.resultCount, 0);
            return <details key={key} className="border-b border-border text-xs">
              <summary className="cursor-pointer py-2 focus-visible:outline-primary">
                <span className="ml-1 font-medium text-text">{label}</span>
                <span className="float-right ml-2 tabular-nums">{calls > 0 ? `${number(calls)}회` : `결과 ${number(orphanResults)}건`}{calls > 0 && orphanResults > 0 ? ` · 미연결 결과 ${number(orphanResults)}건` : ""} · {chars > 0 || unknown === 0 ? `${number(chars)}자` : "크기 미상"}</span>
              </summary>
              <ul className="space-y-2 pb-3 pl-4">
                {tools.map((tool) => <li key={tool.name} className="flex flex-wrap justify-between gap-x-3 gap-y-0.5">
                  <span className="min-w-0 break-all text-text">{tool.name}</span>
                  <span className="ml-auto shrink-0 tabular-nums">{tool.calls > 0 ? `${number(tool.calls)}회` : `결과 ${number(tool.resultCount)}건`} · {tool.chars > 0 || tool.unknownSizes === 0 ? `${number(tool.chars)}자` : "크기 미상"}{tool.unknownSizes > 0 ? ` · ${number(tool.unknownSizes)}건 크기 미상` : ""}</span>
                </li>)}
              </ul>
              {unknown > 0 && <p className="pb-2 text-xs">결과의 원문 크기를 알 수 없는 호출 {number(unknown)}건은 문자 수에서 제외했습니다.</p>}
            </details>;
          })}
          {groups.length === 0 && <p className="mt-2 text-xs">확보한 도구 호출 기록이 없습니다.</p>}
          <p className="mt-2 text-xs">문자 수는 저장된 대화와 도구 결과의 크기입니다. 토큰으로 환산하지 않습니다.</p>
          {report.limitations.map((note) => <p key={note} className="mt-1 text-xs text-status-awaiting">{note}</p>)}
        </section>

        <section aria-label="서브에이전트의 별도 컨텍스트">
          <h3 className="text-sm font-medium text-text">서브에이전트 <span className="ml-1 text-xs font-normal text-text-secondary">{number(report.subagents.length)}개</span></h3>
          <p className="mt-1 text-xs leading-relaxed">각 에이전트는 별도 컨텍스트를 사용합니다. 개별 사용량은 미제공이며 메인 사용량에 더하지 않습니다.</p>
          {report.subagents.length > 0 ? <ul className="mt-2 divide-y divide-border border-y border-border">
            {report.subagents.map((child) => <li key={child.id} className="py-2 text-xs">
              <div className="flex items-baseline gap-3"><span className="min-w-0 flex-1 break-words text-text">{child.title}</span><span className="shrink-0">{stateLabel(child.state)}</span></div>
              <p className="mt-0.5 break-all">{child.model ?? "실행 모델 미상"}</p>
            </li>)}
          </ul> : <p className="mt-2 text-xs">확보한 서브에이전트 실행 기록이 없습니다.</p>}
        </section>
      </div>
    </section>
  );
}
