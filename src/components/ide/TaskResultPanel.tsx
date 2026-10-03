import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { purposeGet, type PurposeSnapshot } from "../../lib/project-plan";
import { taskCapsule, type Capsule } from "../../lib/ipc";
import { taskResultGet, taskResultReference, taskReviewGet, taskReviewDecide, taskReviewDeliver, type TaskResult, type ReviewSnapshot, type ReviewInput } from "../../lib/task-results";

const stateLabel:Record<string,string>={pending:"검토 중",changes_requested:"수정 요청",accepted:"검토 완료",cancelled:"취소",closed:"작업 종료",current:"현재 소스",stale:"과거 결과",unknown:"미확인",passed:"통과",failed:"실패",not_run:"미실행",infrastructure_error:"실행 환경 오류",cancelled_check:"취소",sent:"접수됨",queued:"접수 대기",dispatching:"접수 확인 중",delivery_unknown:"전송 상태 미확인"};
export function TaskResultPanel({taskId,onClose,onOpenFile,onChanges}:{taskId:number;onClose:()=>void;onOpenFile:(path:string)=>void;onChanges:()=>void}) {
  const [result,setResult]=useState<TaskResult|null>(null);
  const [review,setReview]=useState<ReviewSnapshot|null>(null);
  const [purpose,setPurpose]=useState<PurposeSnapshot|null>(null);
  const [purposeError,setPurposeError]=useState<string|null>(null);
  const [capsule,setCapsule]=useState<Capsule|null>(null);
  const [error,setError]=useState<string|null>(null);
  const [capsuleError,setCapsuleError]=useState<string|null>(null);
  const [busy,setBusy]=useState(false);
  const [reason,setReason]=useState("");
  const [criteria,setCriteria]=useState("");
  const [checked,setChecked]=useState<boolean[]>([]);
  const [path,setPath]=useState("");
  const [kind,setKind]=useState("file");
  const generation=useRef(0);
  const mutation=useRef<{payload:string;id:string}|null>(null);
  const close=useRef<HTMLButtonElement>(null);
  const reload=useCallback(async()=>{
    const g=++generation.current;
    // Serial reads keep revision initialization from contending with itself.
    const r=(await Promise.allSettled([taskResultGet(taskId)]))[0];
    const v=(await Promise.allSettled([taskReviewGet(taskId)]))[0];
    const c=(await Promise.allSettled([taskCapsule(taskId)]))[0];
    const p=(await Promise.allSettled([purposeGet(taskId)]))[0];
    if(g!==generation.current)return;
    if(p.status==="fulfilled"){setPurpose(p.value);setPurposeError(null);}else setPurposeError(String(p.reason));
    if(r.status==="fulfilled")setResult(r.value);
    if(v.status==="fulfilled")setReview(v.value);
    setError(r.status==="rejected"?String(r.reason):v.status==="rejected"?String(v.reason):(r.value.source_id!==v.value.source_id||r.value.result_revision!==v.value.result_revision)?"조회 중 결과가 변경되었습니다. 새로고침 후 검토하세요.":null);
    if(c.status==="fulfilled"){setCapsule(c.value);setCapsuleError(null);}else setCapsuleError(String(c.reason));
  },[taskId]);
  useEffect(()=>{
    setResult(null);setReview(null);setCapsule(null);setPurpose(null);setReason("");setChecked([]);mutation.current=null;
    const origin=document.activeElement as HTMLElement|null;
    void reload();close.current?.focus();
    const key=(event:KeyboardEvent)=>{if(event.key==="Escape")onClose();};
    const refresh=()=>void reload();
    window.addEventListener("keydown",key);window.addEventListener("focus",refresh);
    const interval=window.setInterval(refresh,30000);
    let active=true;let stop:(()=>void)|undefined;
    void listen("task-result://changed",refresh).then(fn=>{if(active)stop=fn;else fn();}).catch(()=>{});
    return ()=>{active=false;generation.current++;stop?.();window.clearInterval(interval);window.removeEventListener("keydown",key);window.removeEventListener("focus",refresh);origin?.focus();};
  },[taskId,reload,onClose]);
  const current=review?.active;
  useEffect(()=>setChecked(current?.criteria.map(c=>c.checked)??[]),[current?.id,current?.revision]);
  const act=async(action:string)=>{
    if(!result||!review)return;
    const payload={source_id:review.source_id,result_revision:review.result_revision,round_id:current?.id??null,expected_revision:current?.revision??0,action,reason,criteria:criteria.split("\n").map(s=>s.trim()).filter(Boolean),checked};
    const serialized=JSON.stringify(payload);
    if(mutation.current?.payload!==serialized)mutation.current={payload:serialized,id:crypto.randomUUID()};
    const input:ReviewInput={...payload,mutation_id:mutation.current.id};
    setBusy(true);setError(null);
    try{await taskReviewDecide(taskId,input);mutation.current=null;setReason("");await reload();}catch(e){setError(String(e));}finally{setBusy(false);}
  };
  const deliver=async(id:string)=>{
    if(!review)return;setBusy(true);
    try{await taskReviewDeliver(taskId,review.source_id,id);await reload();}catch(e){setError(String(e));}finally{setBusy(false);}
  };
  const add=async()=>{
    if(!result)return;setBusy(true);
    try{await taskResultReference(taskId,result.source_id,result.result_revision,kind,path);setPath("");await reload();}catch(e){setError(String(e));}finally{setBusy(false);}
  };
  return <section aria-label="작업 결과" className="shrink-0 max-h-[55vh] overflow-auto border-b border-border bg-surface p-3 text-sm">
    <div className="flex items-center gap-3"><h2 className="font-medium">작업 #{taskId} 결과</h2><button onClick={()=>void reload()} disabled={busy}>새로고침</button><button ref={close} onClick={onClose} className="ml-auto" aria-label="결과 닫기">닫기</button></div>
    {error&&<p role="alert" className="text-status-failed">상태 확인 필요: {error}</p>}
    {!result?<p aria-live="polite">결과를 불러오는 중</p>:<>
      <p className="text-text-muted">결과 v{result.result_revision} · {stateLabel[result.freshness]??result.freshness} · 현재 소스 판정은 외부 환경의 재현성을 보장하지 않습니다.</p>
      <div className="grid gap-4 md:grid-cols-2 mt-3">
        <div><h3 className="font-medium">산출물과 변경</h3>{result.references.length===0?<p>대표 결과 미지정</p>:result.references.map(ref=><p key={`${ref.kind}:${ref.relative_path}`}><button disabled={ref.availability!=null&&ref.availability!=="ready"&&ref.availability!=="available"} onClick={()=>onOpenFile(ref.relative_path)} className="text-primary-bright">{ref.relative_path}</button>{ref.result_revision!=null&&<span className="text-text-muted"> · 지정 v{ref.result_revision}</span>}{ref.availability&&ref.availability!=="ready"&&ref.availability!=="available"&&` · ${ref.availability}`}</p>)}
          <div className="flex flex-wrap gap-2 mt-2"><select aria-label="산출물 종류" value={kind} onChange={e=>setKind(e.target.value)}><option value="file">파일</option><option value="report">보고서</option></select><input aria-label="산출물 상대 경로" placeholder="작업 폴더 안 상대 경로" value={path} onChange={e=>setPath(e.target.value)} className="bg-bg border border-border px-2"/><button disabled={busy||!!error||!path.trim()} onClick={()=>void add()}>대표 결과 지정</button></div>
          <button onClick={onChanges} className="mt-2 text-primary-bright">변경 검토·검증 열기</button>
          {capsuleError?<p role="status">브리핑 조회 불가: {capsuleError}</p>:capsule&&<><p>{capsule.goal_contract?.objective??capsule.instruction}</p><p>{capsule.next_action}</p><p className="text-text-muted">변경 {capsule.changed.length}개</p></>}
          <h3 className="font-medium mt-3">이 회차가 사용한 목적</h3>{purposeError?<p>목적 조회 불가: {purposeError}</p>:purpose?<p>계획 v{purpose.plan_revision} · {purpose.project_objective} / {purpose.phase_objective}</p>:<p>연결된 목적 없음</p>}
          <h3 className="font-medium mt-3">검증 이력</h3>{result.receipts.length===0?<p>실행된 검사 기록 없음</p>:result.receipts.map(receipt=><article key={receipt.verify_run_id} className="border-t border-border py-2"><p>v{receipt.result_revision} · {stateLabel[receipt.outcome]??receipt.outcome} · {stateLabel[receipt.freshness]??receipt.freshness}</p><p className="text-text-muted">{new Date(receipt.finished_at*1000).toLocaleString()} · {receipt.observation_scope}</p>{[receipt.report?.build,receipt.report?.test].filter(c=>c!=null).map((check,i)=><details key={i}><summary>{check!.command} · 종료 {check!.exit_code}</summary><pre className="whitespace-pre-wrap break-words">{check!.tail||"저장된 출력 없음"}</pre></details>)}</article>)}
        </div>
        <div><h3 className="font-medium">사람 검토 · {current?stateLabel[current.state]??current.state:"시작하지 않음"}</h3><p className="text-text-muted">검토 완료는 Git 적용을 실행하지 않습니다.</p>
          {current?.criteria.map((c,i)=><label key={i} className="flex gap-2"><input type="checkbox" checked={checked[i]??false} disabled={busy||current.state!=="pending"} onChange={e=>setChecked(values=>values.map((v,j)=>j===i?e.target.checked:v))}/>{c.text}</label>)}
          {!current&&<textarea aria-label="검토 완료 기준" placeholder="추가 완료 기준 (한 줄에 하나)" value={criteria} onChange={e=>setCriteria(e.target.value)} className="w-full bg-bg border border-border mt-2"/>}
          <textarea aria-label="검토 사유" placeholder="검토 시작·결정·수정 요청 사유" value={reason} onChange={e=>setReason(e.target.value)} className="w-full bg-bg border border-border mt-2"/>
          <div className="flex flex-wrap gap-2"><button disabled={busy||!!error||!reason.trim()} onClick={()=>void act(current?"cancel":"start")}>{current?"검토 취소":"검토 시작"}</button>
            {current?.state==="pending"&&<><button disabled={busy||!!error||!reason.trim()||result.freshness!=="current"||checked.some(v=>!v)} onClick={()=>void act("accept")}>검토 완료</button><button disabled={busy||!!error||!reason.trim()} onClick={()=>void act("request_changes")}>수정 요청 보내기</button></>}
            {current?.state==="changes_requested"&&<><button disabled={busy||!!error||!reason.trim()||review!.completion_epoch<=current.completion_epoch} onClick={()=>void act("resubmit")}>새 실행 결과 재검토</button>{review?.deliveries.some(d=>d.state==="delivery_unknown"||d.state==="dispatching")&&<button disabled={busy||!!error||!reason.trim()} onClick={()=>void act("request_changes")}>전송 상태 확인 후 새 요청 보내기</button>}</>}
          </div>
          {review?.deliveries.map(d=><p key={d.mutation_id} role="status">수정 요청: {stateLabel[d.state]??d.state}{d.error&&` · ${d.error}`}{d.state==="queued"&&current?.state==="changes_requested"&&<button disabled={busy||!!error} onClick={()=>void deliver(d.mutation_id)}>대기 요청 전송 재개</button>}</p>)}
          <details className="mt-2"><summary>검토 결정 이력 ({review?.decisions.length??0})</summary>{review?.decisions.map((d,i)=><p key={i}>{stateLabel[d.action]??d.action} · {d.reason}</p>)}</details>
        </div>
      </div>
    </>}
  </section>;
}
