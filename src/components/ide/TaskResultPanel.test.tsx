// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { TaskResultPanel } from "./TaskResultPanel";
import { taskResultGet, taskReviewGet, taskReviewDeliver, type TaskResult, type ReviewSnapshot } from "../../lib/task-results";
import { taskCapsule } from "../../lib/ipc";
vi.mock("../../lib/task-results",()=>({taskResultGet:vi.fn(),taskReviewGet:vi.fn(),taskReviewDecide:vi.fn(),taskResultReference:vi.fn(),taskReviewDeliver:vi.fn()}));
vi.mock("../../lib/ipc",()=>({taskCapsule:vi.fn()}));
vi.mock("../../lib/project-plan",()=>({purposeGet:vi.fn(async()=>null)}));
vi.mock("@tauri-apps/api/event",()=>({listen:vi.fn(async()=>()=>{})}));
(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let root:Root;let host:HTMLDivElement;
const result:TaskResult={task_id:1,source_id:"s",result_revision:2,completion_epoch:2,freshness:"current",fingerprint:"hash",references:[],receipts:[{verify_run_id:"r",result_revision:1,started_at:1,finished_at:2,outcome:"passed",freshness:"stale",logs_ref:null,observation_scope:"git",command_spec_hash:"c",report:{build:null,test:{command:"npm test",exit_code:0,tail:"tests passed"}}}]};
const view:ReviewSnapshot={source_id:"s",result_revision:2,completion_epoch:2,active:null,rounds:[],decisions:[],deliveries:[]};
beforeEach(()=>{vi.clearAllMocks();host=document.createElement("div");document.body.append(host);root=createRoot(host);vi.mocked(taskResultGet).mockResolvedValue(structuredClone(result));vi.mocked(taskReviewGet).mockResolvedValue(structuredClone(view));vi.mocked(taskCapsule).mockRejectedValue("capsule unavailable");});
afterEach(()=>{act(()=>root.unmount());host.remove();});
const button=(text:string)=>[...host.querySelectorAll("button")].find(b=>b.textContent===text)!;
it("keeps source validity, command outcome and section errors separate",async()=>{
 await act(async()=>root.render(<TaskResultPanel taskId={1} onClose={()=>{}} onOpenFile={()=>{}} onChanges={()=>{}}/>));
 expect(host.textContent).toContain("현재 소스");expect(host.textContent).toContain("과거 결과");expect(host.textContent).toContain("통과");expect(host.textContent).toContain("npm test");expect(host.textContent).toContain("tests passed");expect(host.textContent).toContain("브리핑 조회 불가");expect(button("검토 시작").disabled).toBe(true);
});
it("does not enable acceptance from a passed receipt without manual checks",async()=>{
 vi.mocked(taskReviewGet).mockResolvedValue({...view,active:{id:"a",task_id:1,source_id:"s",result_revision:2,completion_epoch:2,revision:1,state:"pending",active:true,criteria:[{text:"눈으로 확인",checked:false}]}});
 await act(async()=>root.render(<TaskResultPanel taskId={1} onClose={()=>{}} onOpenFile={()=>{}} onChanges={()=>{}}/>));
 expect((host.querySelector('input[type="checkbox"]') as HTMLInputElement).checked).toBe(false);expect(button("검토 완료").disabled).toBe(true);
});
it("resumes only a queued delivery and does not automatically send unknown requests",async()=>{
 vi.mocked(taskReviewGet).mockResolvedValue({...view,active:{id:"a",task_id:1,source_id:"s",result_revision:2,completion_epoch:2,revision:2,state:"changes_requested",active:true,criteria:[]},deliveries:[{mutation_id:"queued",state:"queued",error:null},{mutation_id:"unknown",state:"delivery_unknown",error:null}]});
 await act(async()=>root.render(<TaskResultPanel taskId={1} onClose={()=>{}} onOpenFile={()=>{}} onChanges={()=>{}}/>));
 expect(taskReviewDeliver).not.toHaveBeenCalled();expect(button("새 실행 결과 재검토").disabled).toBe(true);
 await act(async()=>button("대기 요청 전송 재개").click());expect(taskReviewDeliver).toHaveBeenCalledOnce();expect(taskReviewDeliver).toHaveBeenCalledWith(1,"s","queued");
});
it("discards a previous task's late result and restores focus on close",async()=>{
 let resolve!:(value:TaskResult)=>void;vi.mocked(taskResultGet).mockReturnValueOnce(new Promise(r=>{resolve=r;}));
 const origin=document.createElement("button");document.body.append(origin);origin.focus();const onClose=vi.fn();
 await act(async()=>root.render(<TaskResultPanel taskId={1} onClose={onClose} onOpenFile={()=>{}} onChanges={()=>{}}/>));
 await act(async()=>root.render(<TaskResultPanel taskId={2} onClose={onClose} onOpenFile={()=>{}} onChanges={()=>{}}/>));
 await act(async()=>resolve({...result,result_revision:999}));expect(host.textContent).not.toContain("v999");
 await act(async()=>window.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape"})));expect(onClose).toHaveBeenCalledOnce();
 act(()=>root.render(null));expect(document.activeElement).toBe(origin);origin.remove();
});

it("blocks decisions when the result and review were read from different versions",async()=>{
 vi.mocked(taskReviewGet).mockResolvedValue({...view,result_revision:3});
 await act(async()=>root.render(<TaskResultPanel taskId={1} onClose={()=>{}} onOpenFile={()=>{}} onChanges={()=>{}}/>));
 expect(host.textContent).toContain("조회 중 결과가 변경되었습니다");expect(button("검토 시작").disabled).toBe(true);
});
