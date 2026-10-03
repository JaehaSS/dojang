// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { QuestionCard, QuestionSession } from "./QuestionSession";
import { answerQuestion, interactionSnapshot, questionReceipt, saveQuestionDraft, type Interaction } from "../../lib/conversation-interaction";
import { appendConvoEvent, coalesceConvoEvent, type ConvoEventLike } from "./ConversationView";
vi.mock("@tauri-apps/api/event",()=>({listen:vi.fn(async()=>()=>{})}));
const opener=vi.hoisted(()=>({openUrl:vi.fn(async()=>undefined)}));
vi.mock("@tauri-apps/plugin-opener",()=>opener);
vi.mock("../../lib/conversation-interaction",async(importOriginal)=>({
  ...await importOriginal<typeof import("../../lib/conversation-interaction")>(),
  answerQuestion:vi.fn(),questionReceipt:vi.fn(),saveQuestionDraft:vi.fn(),interactionSnapshot:vi.fn(),retryQuestionCleanup:vi.fn(),
}));
(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
let container:HTMLDivElement;let root:Root;
const refresh=vi.fn(async()=>{});
const item=():Interaction=>({id:"question-one",execution_id:"execution-one",call_id:"call-one",questions:{kind:"clarification",questions:[{id:"color",question:"어떤 색상?",options:[{id:"blue",label:"파랑",description:""}],allow_free_text:true,is_secret:false}]},state:"pending",reason:null,revision:1,expires_at:Date.now()/1000+100,draft:[],draft_revision:0,receipt:null});
beforeEach(()=>{vi.clearAllMocks();container=document.createElement("div");document.body.append(container);root=createRoot(container);vi.mocked(saveQuestionDraft).mockResolvedValue(1);vi.mocked(answerQuestion).mockImplementation(async(_task,_item,id)=>({request_id:id,state:"claimed"}));});
afterEach(async()=>{await act(async()=>root.unmount());container.remove();});
async function card(value=item()){await act(async()=>root.render(<QuestionCard key={value.id} taskId={7} item={value} phase="running" refresh={refresh}/>));}
async function choose(){await act(async()=>{(container.querySelector('input[type="radio"]') as HTMLInputElement).click();});}
function button(label:string){return [...container.querySelectorAll("button")].find((b)=>b.textContent===label)!;}
it("persists a question draft separately, then submits a single immutable receipt on double click",async()=>{
  await card();await choose();expect(saveQuestionDraft).toHaveBeenCalledWith(7,expect.objectContaining({id:"question-one"}),[{question_id:"color",option_id:"blue",text:null}],0);
  let finish!:(receipt:{request_id:string;state:string})=>void;
  vi.mocked(answerQuestion).mockImplementation((_task,_item,id)=>new Promise((resolve)=>{finish=(r)=>resolve({...r,request_id:id});}));
  await act(async()=>{button("답변 보내기").click();button("답변 보내기").click();});
  expect(answerQuestion).toHaveBeenCalledTimes(1);
  await act(async()=>finish({request_id:"id",state:"claimed"}));
  expect(container.textContent).toContain("답변 접수됨");expect(container.querySelector("fieldset")?.disabled).toBe(true);
});
it("queries an uncertain receipt without resending the answer",async()=>{
  await card();await choose();vi.mocked(answerQuestion).mockRejectedValue(new Error("connection lost"));
  await act(async()=>button("답변 보내기").click());
  expect(container.querySelector("fieldset")?.disabled).toBe(true);
  const id=vi.mocked(answerQuestion).mock.calls[0][2];vi.mocked(questionReceipt).mockResolvedValue({request_id:id,state:"written"});
  await act(async()=>button("접수 상태 확인").click());
  expect(questionReceipt).toHaveBeenCalledWith(7,id);expect(answerQuestion).toHaveBeenCalledTimes(1);expect(container.textContent).toContain("답변 전송됨");
});
it("keeps definitive pre-claim validation failures editable and retries with a new receipt",async()=>{
  await card();await choose();vi.mocked(answerQuestion).mockRejectedValueOnce(new Error("INPUT_VALIDATION: 범위를 확인하세요"));
  await act(async()=>button("답변 보내기").click());
  const first=vi.mocked(answerQuestion).mock.calls[0][2];
  expect(container.querySelector("fieldset")?.disabled).toBe(false);expect(container.textContent).toContain("범위를 확인하세요");expect(button("접수 상태 확인")).toBeUndefined();
  await act(async()=>button("답변 보내기").click());
  expect(answerQuestion).toHaveBeenCalledTimes(2);expect(vi.mocked(answerQuestion).mock.calls[1][2]).not.toBe(first);expect(container.textContent).toContain("답변 접수됨");
});
it("IME composition does not submit and a normal modifier Enter targets this question only",async()=>{
  const value=item();value.draft=[{question_id:"color",option_id:null,text:"직접 답변"}];await card(value);
  const input=container.querySelector("textarea")!;
  await act(async()=>{input.dispatchEvent(new KeyboardEvent("keydown",{bubbles:true,key:"Enter",ctrlKey:true,isComposing:true}));});expect(answerQuestion).not.toHaveBeenCalled();
  await act(async()=>{input.dispatchEvent(new KeyboardEvent("keydown",{bubbles:true,key:"Enter",ctrlKey:true}));});
  expect(answerQuestion).toHaveBeenCalledWith(7,expect.objectContaining({id:value.id,execution_id:value.execution_id}),expect.any(String),value.draft);
});
it("restores a submitted answer as read-only after returning to the task",async()=>{
  const value=item();value.draft=[{question_id:"color",option_id:"blue",text:null}];value.receipt={request_id:"old",state:"acknowledged"};value.state="closed";value.reason="answered";
  await card(value);expect((container.querySelector("input") as HTMLInputElement).checked).toBe(true);expect(container.textContent).toContain("도구 응답 반영됨");expect(button("답변 보내기")).toBeUndefined();
});
it("requires an explicit native approval choice and never defaults to accept",async()=>{
  const value=item();value.questions.questions=[{id:"decision",question:"권한을 허용할까요?",options:[{id:"accept",label:"Accept",description:""},{id:"decline",label:"Decline",description:""}],allow_free_text:false,is_secret:false}];value.request={kind:"approval",title:"파일 접근 승인",details:"이 도구가 파일을 읽습니다.",blocking:true};
  await card(value);const submit=button("승인 여부 보내기");expect(submit.disabled).toBe(true);expect((container.querySelectorAll('input[type="radio"]')[0] as HTMLInputElement).checked).toBe(false);expect(answerQuestion).not.toHaveBeenCalled();
  await act(async()=>{(container.querySelectorAll('input[type="radio"]')[1] as HTMLInputElement).click();});expect(submit.disabled).toBe(false);
  await act(async()=>submit.click());expect(answerQuestion).toHaveBeenCalledWith(7,expect.objectContaining({id:value.id}),expect.any(String),[{question_id:"decision",option_id:"decline",text:null}]);
});
it("shows native form context and permits an explicit optional-field skip",async()=>{
  const value=item();value.questions.questions=[{id:"note",question:"추가 메모",options:[{id:"skip",label:"Skip",description:"Leave this optional field empty"}],allow_free_text:true,is_secret:false}];value.request={kind:"form",title:"MCP input",details:"선택 사항은 건너뛸 수 있습니다.",blocking:true};
  await card(value);const submit=button("입력 보내기");expect(container.textContent).toContain("MCP input");expect(container.textContent).toContain("선택 사항은 건너뛸 수 있습니다.");expect(submit.disabled).toBe(true);
  await act(async()=>{(container.querySelector('input[type="radio"]') as HTMLInputElement).click();});expect(submit.disabled).toBe(false);
  await act(async()=>submit.click());expect(answerQuestion).toHaveBeenCalledWith(7,expect.objectContaining({id:value.id}),expect.any(String),[{question_id:"note",option_id:"skip",text:null}]);
});
it("shows async native URL context but refuses a non-http external opener",async()=>{
  const value=item();value.request={kind:"url",title:"외부 페이지",details:"열어 확인한 뒤 선택하세요.",blocking:false,url:"javascript:alert(1)"};await card(value);
  expect(container.textContent).toContain("외부 페이지");expect(container.textContent).toContain("다른 작업은 계속 진행될 수 있습니다.");expect(container.textContent).toContain("열 수 없는 주소입니다.");expect(button("외부 링크 열기")).toBeUndefined();expect(opener.openUrl).not.toHaveBeenCalled();
});
it("opens an approved native URL only after an explicit click",async()=>{
  const value=item();value.request={kind:"url",title:"외부 페이지",details:"링크를 확인하세요.",blocking:true,url:"https://example.com/check"};await card(value);
  expect(opener.openUrl).not.toHaveBeenCalled();await act(async()=>button("외부 링크 열기").click());expect(opener.openUrl).toHaveBeenCalledWith("https://example.com/check");
});
it("declines an MCP form without requiring the requested information",async()=>{
  const value=item();value.request={kind:"form",title:"MCP 입력",details:"필수 이름",blocking:true};
  value.questions.questions=[{id:"name",question:"이름",options:[],allow_free_text:true,is_secret:false},{id:"__dojang_action",question:"제출할까요?",options:[{id:"accept",label:"허용",description:""},{id:"decline",label:"거절",description:""}],allow_free_text:false,is_secret:false}];
  await card(value);expect(button("입력 보내기").disabled).toBe(true);
  await act(async()=>{(container.querySelectorAll('input[type="radio"]')[1] as HTMLInputElement).click();});
  expect(button("입력 보내기").disabled).toBe(false);await act(async()=>button("입력 보내기").click());
  expect(answerQuestion).toHaveBeenCalledWith(7,value,expect.any(String),[{question_id:"__dojang_action",option_id:"decline",text:null}]);
});
it("does not send previously drafted form fields when the user declines",async()=>{
  const value=item();value.request={kind:"form",title:"MCP 입력",details:"",blocking:true};
  value.questions.questions=[{id:"name",question:"이름",options:[],allow_free_text:true,is_secret:false},{id:"__dojang_action",question:"제출할까요?",options:[{id:"decline",label:"거절",description:""}],allow_free_text:false,is_secret:false}];
  value.draft=[{question_id:"name",option_id:null,text:"draft only"}];await card(value);await choose();
  await act(async()=>button("입력 보내기").click());
  expect(answerQuestion).toHaveBeenCalledWith(7,value,expect.any(String),[{question_id:"__dojang_action",option_id:"decline",text:null}]);
});
it("identifies nonblocking questions and keeps resolved written receipts unconfirmed",async()=>{
  const value=item();value.request={kind:"question",title:"Codex 질문",details:"",blocking:false};
  await card(value);expect(container.textContent).toContain("답변을 기다리는 동안 작업은 계속됩니다.");
  value.state="closed";value.reason="resolved";value.receipt={request_id:"native",state:"written"};
  await card(value);expect(container.textContent).toContain("반영 확인 전");expect(container.textContent).toContain("Codex가 더 이상 답변을 기다리지 않음");expect(container.textContent).not.toContain("도구 응답 반영됨");
});
it("remote task views do not read local question IPC",async()=>{
  await act(async()=>root.render(<QuestionSession taskId={null} linkedIds={[]}>{(_render,status)=><div>{status}</div>}</QuestionSession>));expect(interactionSnapshot).not.toHaveBeenCalled();
});
it("ignores an old task snapshot arriving after task selection changes",async()=>{
  let old!:(v:Awaited<ReturnType<typeof interactionSnapshot>>)=>void;
  vi.mocked(interactionSnapshot).mockImplementation((id)=>id===1?new Promise((resolve)=>{old=resolve;}):Promise.resolve({enabled:true,execution_id:null,phase:"idle",items:[]}));
  await act(async()=>root.render(<QuestionSession key="one" taskId={1} linkedIds={[]}>{(_render,status)=><div>{status}</div>}</QuestionSession>));
  await act(async()=>root.render(<QuestionSession key="two" taskId={2} linkedIds={[]}>{(_render,status)=><div>{status}</div>}</QuestionSession>));
  await act(async()=>old({enabled:true,execution_id:"old",phase:"running",items:[item()]}));expect(container.textContent).not.toContain("어떤 색상?");
});
it("coalesces streamed Markdown before and after a question without duplicating completed text",()=>{
  const first:ConvoEventLike={kind:"text_update",item_id:"m",text:"# Title",complete:false};
  let items=appendConvoEvent([],first);items=appendConvoEvent(items,{kind:"interaction",interaction_id:"q"});
  const completed={...first,text:"# Title\n\n```mermaid\ngraph LR; A-->B\n```",complete:true};items=appendConvoEvent(items,completed);
  expect(items).toHaveLength(2);expect(items[0]).toMatchObject({role:"text",complete:true,text:completed.text});expect(items[1]).toEqual({role:"interaction",interactionId:"q"});
  expect(coalesceConvoEvent([first],completed)).toEqual([completed]);
});
