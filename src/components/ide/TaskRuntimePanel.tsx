import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { observationLabel, taskObservationGet, type TaskObservation } from "../../lib/task-observation";
const labels: Record<string,string> = { resume:"대화 재개", interrupt:"중단", questions:"구조화 질문", approvals:"도구 승인", subagents:"서브 에이전트", tokens:"토큰 계측", cost:"달러 비용 계측", isolated_questions:"별도 질의", agent_switch:"벤더 전환" };
const supportLabels: Record<string,string> = { supported:"지원", unsupported:"미지원", unknown:"미확인" };
export function TaskRuntimePanel({ taskId }: { taskId:number }) {
  const [value,setValue]=useState<TaskObservation|null>(null);
  const [error,setError]=useState<string|null>(null);
  const generation=useRef(0);
  const reload=useCallback(async()=>{
    const request=++generation.current;
    try {
      const next=await taskObservationGet(taskId);
      if(request!==generation.current)return;
      if(next.task_id!==taskId)throw new Error("다른 작업의 실행 상태 응답입니다.");
      setValue(next);setError(null);
    }catch(cause){if(request===generation.current){setValue(null);setError(String(cause));}}
  },[taskId]);
  useEffect(()=>{
    setValue(null);setError(null);void reload();
    const stops:Array<()=>void>=[];let active=true;
    for(const event of ["task://state","convo-interaction://changed","task://archive-changed","task-result://changed"]){
      void listen(event,()=>void reload()).then(stop=>{if(active)stops.push(stop);else stop();}).catch(()=>{});
    }
    const timer=window.setInterval(()=>void reload(),3000);
    return()=>{active=false;generation.current++;window.clearInterval(timer);stops.forEach(stop=>stop());};
  },[reload]);
  return <section aria-label="실행 상태·벤더 지원" className="border border-border rounded p-2 my-2 text-xs">
    <div className="flex gap-2 items-center"><strong>{value?observationLabel[value.status]:error?"관측 불가":"실행 상태 확인 중"}</strong><button className="ml-auto text-primary-bright" onClick={()=>void reload()}>실행 상태 새로고침</button></div>
    {error&&<p role="status" className="text-status-failed">상태 조회 실패: {error}</p>}
    {value&&<>
      <p className="text-text-muted mt-1">{new Date(value.checked_at*1000).toLocaleString()} · 작업 상태 {value.task_state}</p>
      <p>등록 실행 {value.registered_execution?"있음":"없음"} · 입력 대기 {value.pending_questions} · 검증 프로세스 {value.review_processes} · 별도 질의 {value.side_questions} · 전송 {value.deliveries} · 승인 처리 {value.approvals}</p>
      {value.cleanup_required&&<p className="text-status-failed">실행 프로세스 정리 확인이 필요합니다.</p>}
      <p>자식 프로세스: {value.child_processes==null?"관측 불가":value.child_processes}</p>
      <details className="mt-1"><summary className="cursor-pointer">벤더 기능 지원표 · {value.capabilities.agent} · {value.capabilities.route}</summary>
        <p className="text-text-muted">연결 경로의 지원 계약입니다. CLI 설치·인증과 실제 사용 가능 여부는 실행 시 확인합니다.</p>
        <table className="w-full mt-1 text-left"><thead><tr><th>기능</th><th>지원</th><th>조건·관측 범위</th></tr></thead><tbody>{value.capabilities.features.map(feature=><tr key={feature.key} className="border-t border-border"><td className="py-1">{labels[feature.key]??feature.key}</td><td>{supportLabels[feature.state]??"미확인"}{feature.enabled===false&&feature.state==="supported"?" · 꺼짐":""}</td><td className="text-text-muted">{feature.reason}</td></tr>)}</tbody></table>
      </details>
      <p className="text-text-muted mt-1">등록된 실행·질문·전송을 조회합니다. 벤더가 보고하지 않는 자식 작업의 전체 종료는 판정하지 않습니다.</p>
    </>}
  </section>;
}
