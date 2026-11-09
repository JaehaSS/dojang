// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { TaskRuntimePanel } from "./TaskRuntimePanel";
const api=vi.hoisted(()=>({get:vi.fn()}));
vi.mock("../../lib/task-observation",async()=>({...await vi.importActual("../../lib/task-observation"),taskObservationGet:api.get}));
vi.mock("@tauri-apps/api/event",()=>({listen:async()=>()=>{}}));
(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let host:HTMLDivElement,root:Root;
const observation=(id=1,status="remaining_work")=>({task_id:id,checked_at:10,revision:"rev",task_state:"AwaitingReview",status,registered_execution:false,pending_questions:0,review_processes:0,side_questions:1,deliveries:0,approvals:0,child_processes:null,capabilities:{agent:"agy",route:"stream",features:[{key:"cost",state:"unsupported",enabled:null,reason:"달러 비용을 계측하지 않습니다."}]}});
beforeEach(()=>{vi.clearAllMocks();host=document.createElement("div");document.body.append(host);root=createRoot(host)});
afterEach(()=>{act(()=>root.unmount());host.remove()});
it("등록된 잔여 작업과 관측 불가·비용 미지원을 구분한다",async()=>{
 api.get.mockResolvedValue(observation());await act(async()=>{root.render(<TaskRuntimePanel taskId={1}/>)});
 expect(host.textContent).toContain("잔여 작업 존재");expect(host.textContent).toContain("별도 질의 1");expect(host.textContent).toContain("자식 프로세스: 관측 불가");expect(host.textContent).toContain("미지원");expect(host.textContent).not.toContain("$0");
});
it("실패한 갱신에서 과거 성공 상태를 현재 상태로 유지하지 않는다",async()=>{
 api.get.mockResolvedValueOnce(observation(1,"ended")).mockRejectedValueOnce(new Error("offline"));
 await act(async()=>{root.render(<TaskRuntimePanel taskId={1}/>)});expect(host.textContent).toContain("작업 기록 종결");
 await act(async()=>{host.querySelector("button")!.click()});expect(host.textContent).toContain("관측 불가");expect(host.textContent).not.toContain("작업 기록 종결");
});
it("작업을 바꾼 뒤 도착한 이전 응답을 무시한다",async()=>{
 let resolveOld!:(v:unknown)=>void;api.get.mockReturnValueOnce(new Promise(resolve=>{resolveOld=resolve})).mockResolvedValueOnce(observation(2,"review_needed"));
 await act(async()=>{root.render(<TaskRuntimePanel taskId={1}/>)});
 await act(async()=>{root.render(<TaskRuntimePanel taskId={2}/>)});
 await act(async()=>{resolveOld(observation(1,"ended"))});expect(host.textContent).toContain("결과 검토 필요");expect(host.textContent).not.toContain("작업 기록 종결");
});
