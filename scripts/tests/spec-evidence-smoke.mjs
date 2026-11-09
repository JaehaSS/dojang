import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

// Real result/spec components and reference helper. Desktop IPC and external opener are faked.
const directory = path.resolve(".dojang/verification/spec-evidence");
const fixture = path.join(directory, "fixture");
const source = file => `/@fs/${path.resolve(file)}`;
let browser;
let server;
try {
  await mkdir(fixture, { recursive: true });
  await writeFile(path.join(fixture, "index.html"), '<!doctype html><link rel="icon" href="data:,"><div id="root"></div><script type="module" src="./main.tsx"></script>');
  await writeFile(path.join(fixture, "core.ts"), 'export const invoke=(command,args)=>window.invokeFixture(command,args);');
  await writeFile(path.join(fixture, "event.ts"), 'export const listen=async()=>()=>{};');
  await writeFile(path.join(fixture, "opener.ts"), 'export const openUrl=async()=>{};');
  await writeFile(path.join(fixture, "main.tsx"), `
import React,{useState,useCallback} from "react";
import {createRoot} from "react-dom/client";
import "${source("src/index.css")}";
import {TaskResultPanel} from "${source("src/components/ide/TaskResultPanel.tsx")}";
import {appendSpecReference} from "${source("src/lib/task-specs.ts")}";
const original='# 인증 조건\\n기존 세션을 보존한다.\\n<img src="https://spec.invalid/beacon" onerror="window.pwned=true">';
let live={relative_path:"docs/auth.md",title:"인증 조건",content_hash:"a".repeat(64),byte_length:128,content:original};
window.calls=[];window.saved=[];window.decisions=[];window.opened=[];window.pwned=false;
const review={source_id:"smoke",result_revision:2,completion_epoch:2,active:{id:"round",task_id:7,source_id:"smoke",result_revision:2,completion_epoch:2,revision:1,state:"pending",active:true,criteria:[]},rounds:[],decisions:[],deliveries:[]};
const meta=({content,...row})=>row;
window.changeSource=()=>{live={...live,content:"# 바뀐 조건",content_hash:"b".repeat(64)};};
window.deleteSource=()=>{live=null;};
window.invokeFixture=async(command,args)=>{
 window.calls.push({command,args});
 if(command==="task_result_get")return {task_id:7,source_id:"smoke",result_revision:2,completion_epoch:2,freshness:"current",fingerprint:"hash",references:[],receipts:[]};
 if(command==="task_review_get")return structuredClone(review);
 if(command==="task_capsule")throw new Error("capsule fixture");
 if(command==="purpose_get")return null;
 if(command==="review_artifact_list")return [];
 if(command==="task_pr_status_get")return {status:"no_pr",message:null,repository:"owner/repo",branch:"feature",local_head:"a".repeat(40),observed_at:1792960000,pr:null};
 if(command==="task_spec_search")return {hits:[{relative_path:"docs/auth.md",title:"인증 조건",snippet:"기존 세션을 보존한다.",line:2}],truncated:false,scanned_files:1,skipped_files:0};
 if(command==="task_spec_read"){if(!live)throw new Error("source deleted");return structuredClone(live);}
 if(command==="task_spec_save"){
  if(live?.content_hash!==args.input.expected_hash)throw new Error("문서 변경 fixture");
  const row={...live,id:window.saved.length+1,task_id:args.id,created_at:1792960000,base_revision:"base"};window.saved.push(row);return structuredClone(row);
 }
 if(command==="task_spec_list")return window.saved.map(meta);
 if(command==="task_spec_get")return structuredClone(window.saved.find(row=>row.id===args.snapshotId&&row.task_id===args.id));
 if(command==="task_review_decide"){window.decisions.push(args);return structuredClone(review);}
 throw new Error("unexpected command "+command);
};
function Harness(){
 const [shown,setShown]=useState(true);const [draft,setDraft]=useState("기존 작업 초안");
 const close=useCallback(()=>setShown(false),[]);const open=useCallback(path=>window.opened.push(path),[]);const changes=useCallback(()=>{},[]);
 const attach=useCallback(reference=>setDraft(value=>appendSpecReference(value,reference)),[]);
 window.hide=()=>setShown(false);window.show=()=>setShown(true);
 return <><label>작업 초안<textarea aria-label="작업 초안" value={draft} onChange={event=>setDraft(event.target.value)}/></label>{shown&&<TaskResultPanel taskId={7} onClose={close} onOpenFile={open} onChanges={changes} onAttachInput={attach}/>}</>;
}
createRoot(document.getElementById("root")).render(<Harness/>);
`);
  server = await createServer({ configFile: false, root: fixture, plugins: [react()], logLevel: "error",
    resolve: { alias: [
      { find: "@tauri-apps/api/core", replacement: path.join(fixture, "core.ts") },
      { find: "@tauri-apps/api/event", replacement: path.join(fixture, "event.ts") },
      { find: "@tauri-apps/plugin-opener", replacement: path.join(fixture, "opener.ts") },
    ] }, server: { host: "127.0.0.1", port: 0 } });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.DOJANG_SMOKE_BROWSER || "chrome" });
  const page = await browser.newPage({ viewport: { width: 1400, height: 1400 } });
  page.setDefaultTimeout(15000);
  const errors = [];
  const remote = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("request", request => { if (request.url().startsWith("https://spec.invalid/")) remote.push(request.url()); });
  await page.goto(server.resolvedUrls.local[0] + "index.html", { waitUntil: "domcontentloaded" });
  const specs = page.getByRole("region", { name: "명세 근거", exact: true });
  await specs.getByRole("textbox", { name: "명세 검색어", exact: true }).fill("인증");
  await specs.getByRole("button", { name: "명세 검색", exact: true }).click();
  await specs.getByRole("button", { name: /인증 조건.*docs\/auth.md/ }).click();
  await specs.getByRole("region", { name: "현재 문서", exact: true }).locator("pre").waitFor();
  await specs.getByRole("button", { name: "현재 문서 열기", exact: true }).click();
  assert.deepEqual(await page.evaluate(() => window.opened), ["docs/auth.md"]);
  await page.evaluate(() => window.changeSource());
  await specs.getByRole("button", { name: "명세 버전 보존", exact: true }).click();
  await specs.getByText(/문서 변경 fixture/).waitFor();
  assert.equal(await page.evaluate(() => window.saved.length), 0);
  await specs.getByRole("button", { name: /인증 조건.*docs\/auth.md/ }).click();
  await specs.getByRole("region", { name: "현재 문서", exact: true }).getByText("# 바뀐 조건", { exact: true }).waitFor();
  await specs.getByRole("button", { name: "명세 버전 보존", exact: true }).click();
  await page.waitForFunction(() => window.saved.length === 1);
  const frozen = specs.getByRole("region", { name: "보존 명세", exact: true });
  await frozen.locator("pre").getByText("# 바뀐 조건", { exact: true }).waitFor();
  await specs.getByRole("button", { name: "작업 입력에 연결", exact: true }).click();
  const draft = await page.getByRole("textbox", { name: "작업 초안", exact: true }).inputValue();
  assert.ok(draft.startsWith("기존 작업 초안"));
  assert.ok(draft.includes("[[dojang-spec:1:" + "b".repeat(64) + "]]") && !draft.includes("# 바뀐 조건"));
  assert.equal(await page.evaluate(() => window.decisions.length), 0);
  const readCount = await page.evaluate(() => window.calls.filter(row => row.command === "task_spec_read").length);
  await page.evaluate(() => { window.deleteSource(); window.hide(); });
  await specs.waitFor({ state: "detached" });
  await page.evaluate(() => window.show());
  await specs.getByRole("region", { name: "보존 명세", exact: true }).getByRole("button", { name: /docs\/auth.md.*#1/ }).click();
  await frozen.locator("pre").getByText("# 바뀐 조건", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.calls.filter(row => row.command === "task_spec_read").length), readCount);
  await specs.getByRole("button", { name: "검토 사유에 연결", exact: true }).click();
  const reason = await page.getByRole("textbox", { name: "검토 사유", exact: true }).inputValue();
  assert.ok(reason.includes("[[dojang-spec:1:" + "b".repeat(64) + "]]"));
  assert.equal(await page.evaluate(() => window.decisions.length), 0);
  await page.getByRole("button", { name: "수정 요청 보내기", exact: true }).click();
  await page.waitForFunction(() => window.decisions.length === 1);
  assert.equal(await page.evaluate(() => window.decisions[0].input.reason), reason);
  assert.deepEqual(remote, []);
  assert.equal(await page.evaluate(() => window.pwned), false);
  assert.deepEqual(errors, []);
  await page.getByRole("region", { name: "작업 결과", exact: true }).evaluate(node => { node.scrollTop = 0; });
  await page.getByRole("region", { name: "작업 결과", exact: true }).screenshot({ path: path.join(directory, "spec-evidence.png") });
  console.log(JSON.stringify({ status: "passed", checks: ["production result/spec panel", "search/preview and original navigation", "stale preview hash rejected", "immutable snapshot after source deletion", "reopen without live file reads", "preserved input draft with explicit reference", "explicit review request only", "inert source text"], screenshot: path.join(directory, "spec-evidence.png") }, null, 2));
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}
