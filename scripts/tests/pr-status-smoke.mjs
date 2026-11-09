import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

// Production result/review panels and helpers; only the desktop IPC and external opener are faked.
const directory = path.resolve(".dojang/verification/pr-status");
const fixture = path.join(directory, "fixture");
const source = file => `/@fs/${path.resolve(file)}`;
let server;
let browser;
try {
  await mkdir(fixture, { recursive: true });
  await writeFile(path.join(fixture, "index.html"), '<!doctype html><link rel="icon" href="data:,"><div id="root"></div><script type="module" src="./main.tsx"></script>');
  await writeFile(path.join(fixture, "core.ts"), 'export const invoke=(command,args)=>window.invokeFixture(command,args);');
  await writeFile(path.join(fixture, "event.ts"), 'export const listen=async()=>()=>{};');
  await writeFile(path.join(fixture, "opener.ts"), 'export const openUrl=async url=>{window.opened.push(url);};');
  await writeFile(path.join(fixture, "main.tsx"), `
import React from "react";
import {createRoot} from "react-dom/client";
import "${source("src/index.css")}";
import {TaskResultPanel} from "${source("src/components/ide/TaskResultPanel.tsx")}";
window.mode="ready";window.calls=[];window.decisions=[];window.opened=[];
const review={source_id:"smoke",result_revision:2,completion_epoch:2,active:{id:"round",task_id:7,source_id:"smoke",result_revision:2,completion_epoch:2,revision:1,state:"pending",active:true,criteria:[]},rounds:[],decisions:[],deliveries:[]};
const status={status:"ready",message:null,repository:"owner/repo",branch:"feature",local_head:"b".repeat(40),observed_at:1792960000,
 pr:{number:7,url:"https://github.com/owner/repo/pull/7",title:"PR smoke",state:"open",draft:true,head_sha:"a".repeat(40),head_branch:"feature",base_branch:"main",review_decision:"CHANGES_REQUESTED",review_requests:["reviewer"],checks:[
 {name:"unit-tests",status:"fail",description:"3 tests failed",details_url:"https://github.com/owner/repo/actions/runs/42"},
 {name:"build",status:"pending",description:"",details_url:null},
 {name:"external-check",status:"unknown",description:"",details_url:"javascript:alert(1)"}]}};
window.invokeFixture=async(command,args)=>{
 window.calls.push({command,args});
 if(command==="task_result_get")return {task_id:7,source_id:"smoke",result_revision:2,completion_epoch:2,freshness:"current",fingerprint:"hash",references:[],receipts:[]};
 if(command==="task_review_get")return structuredClone(review);
 if(command==="task_capsule")throw new Error("No capsule in fixture");
 if(command==="purpose_get")return null;
 if(command==="review_artifact_list")return [];
 if(command==="task_spec_list")return [];
 if(command==="task_pr_status_get"){
   if(window.mode==="error")throw new Error("인증 실패 fixture");
   if(window.mode==="none")return {...status,status:"no_pr",message:"이 브랜치의 PR이 없습니다.",pr:null};
   return structuredClone(status);
 }
 if(command==="task_review_decide"){window.decisions.push(args);return structuredClone(review);}
 throw new Error("unexpected command "+command);
};
createRoot(document.getElementById("root")).render(<TaskResultPanel taskId={7} onClose={()=>{}} onOpenFile={()=>{}} onChanges={()=>{}}/>);
`);
  server = await createServer({
    configFile: false, root: fixture, plugins: [react()], logLevel: "error",
    resolve: { alias: [
      { find: "@tauri-apps/api/core", replacement: path.join(fixture, "core.ts") },
      { find: "@tauri-apps/api/event", replacement: path.join(fixture, "event.ts") },
      { find: "@tauri-apps/plugin-opener", replacement: path.join(fixture, "opener.ts") },
    ] },
    server: { host: "127.0.0.1", port: 0 },
  });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.DOJANG_SMOKE_BROWSER || "chrome" });
  const page = await browser.newPage({ viewport: { width: 1400, height: 1100 } });
  page.setDefaultTimeout(15000);
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(server.resolvedUrls.local[0] + "index.html", { waitUntil: "domcontentloaded" });
  const panel = page.getByRole("region", { name: "PR·검사 상태", exact: true });
  await panel.getByText(/unit-tests/).waitFor();
  assert.match(await panel.innerText(), /owner\/repo/);
  assert.match(await panel.innerText(), /초안/);
  assert.match(await panel.innerText(), /reviewer/);
  assert.match(await panel.innerText(), /미확인/);
  assert.match(await panel.innerText(), /다릅니다|불일치/);
  assert.equal(await panel.locator('[href^="javascript:"]').count(), 0);
  const reason = page.getByRole("textbox", { name: "검토 사유", exact: true });
  await reason.fill("기존 검토 의견");
  await panel.getByRole("button", { name: "실패 검사를 검토 사유에 연결", exact: true }).click();
  assert.equal(await page.evaluate(() => window.decisions.length), 0);
  const draft = await reason.inputValue();
  assert.ok(draft.startsWith("기존 검토 의견"));
  assert.ok(draft.includes("a".repeat(40)) && draft.includes("unit-tests"));
  assert.ok(draft.includes("https://github.com/owner/repo/actions/runs/42"));
  const resultPanel = page.getByRole("region", { name: "작업 결과", exact: true });
  await resultPanel.evaluate(node => { node.scrollTop = 0; });
  await resultPanel.screenshot({ path: path.join(directory, "pr-checks.png") });
  await page.evaluate(() => { window.mode = "error"; });
  await panel.getByRole("button", { name: "PR 새로고침", exact: true }).click();
  await panel.getByText(/인증 실패 fixture/).waitFor();
  assert.equal(await panel.getByText(/unit-tests/).count(), 0, "failed refresh must clear stale success");
  assert.equal(await page.getByRole("button", { name: "수정 요청 보내기", exact: true }).isEnabled(), true);
  await page.getByRole("button", { name: "수정 요청 보내기", exact: true }).click();
  await page.waitForFunction(() => window.decisions.length === 1);
  const input = await page.evaluate(() => window.decisions[0].input);
  assert.equal(input.action, "request_changes");
  assert.equal(input.reason, draft);
  await page.evaluate(() => { window.mode = "none"; });
  await panel.getByRole("button", { name: "PR 새로고침", exact: true }).click();
  await panel.getByText(/이 브랜치의 PR이 없습니다/).waitFor();
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ status: "passed", checks: ["production result/review integration", "PR/check/reviewer rendering", "SHA mismatch", "safe link rejection", "explicit draft attachment", "stale success cleared after failure", "existing explicit delivery", "no PR distinct from unavailable"], screenshot: path.join(directory, "pr-checks.png") }, null, 2));
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}
