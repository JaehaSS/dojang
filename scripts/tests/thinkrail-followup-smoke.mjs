import assert from 'node:assert/strict';
import {mkdir,rm,writeFile} from 'node:fs/promises';
import path from 'node:path';
import {chromium} from 'playwright';
import {createServer} from 'vite';
import react from '@vitejs/plugin-react';
const directory=path.resolve('.dojang/verification/thinkrail-followups'),fixture=path.join(directory,'fixture');
const source=file=>`/@fs/${path.resolve(file)}`;
let server,browser;
try{
 await mkdir(fixture,{recursive:true});
 await writeFile(path.join(fixture,'index.html'),'<div id="root"></div><script type="module" src="/main.tsx"></script>');
 await writeFile(path.join(fixture,'core.ts'),'export const invoke=(command,args)=>window.invokeFixture(command,args);');
 await writeFile(path.join(fixture,'event.ts'),'export const listen=async()=>()=>{};');
 await writeFile(path.join(fixture,'main.tsx'),`
import React from 'react';import {createRoot} from 'react-dom/client';import '${source('src/index.css')}';
import {TaskRuntimePanel} from '${source('src/components/ide/TaskRuntimePanel.tsx')}';
import {SpecEvidencePanel} from '${source('src/components/ide/SpecEvidencePanel.tsx')}';
import {PullRequestComposer} from '${source('src/components/ide/PullRequestComposer.tsx')}';
window.calls=[];window.mode='ready';
const hash='a'.repeat(64);
window.invokeFixture=async(command,args)=>{
 window.calls.push({command,args});
 if(command==='task_observation_get')return {task_id:7,revision:'one',checked_at:1792960000,task_state:'AwaitingReview',status:'remaining_work',registered_execution:false,pending_questions:0,review_processes:0,side_questions:1,deliveries:0,approvals:0,child_processes:null,capabilities:{agent:'agy',route:'stream',features:[{key:'cost',state:'unsupported',enabled:null,reason:'달러 비용을 계측하지 않습니다.'}]}};
 if(command==='task_spec_list')return [];
 if(command==='task_spec_search')return {hits:[{relative_path:'spec.md',title:'Fixture spec',snippet:'links',line:1}],scanned_files:1,skipped_files:0,truncated:false};
 if(command==='task_spec_read')return {relative_path:'spec.md',title:'Fixture spec',content_hash:hash,byte_length:20,content:'# Fixture spec\\n[missing](missing.md)'};
 if(command==='task_spec_validate_links')return {relative_path:'spec.md',content_hash:hash,checked_at:1792960000,links:[{source_line:2,target:'missing.md',state:'missing',resolved_path:'missing.md'}],summary:{total:1,ok:0,missing:1,outside_scope:0,unsupported:0,anchor_missing:0,unreadable:0,external:0},truncated:false};
 if(command==='task_pr_preview')return {token:'preview',repository:'acme/repo',branch:'feature',base:'main',head:'b'.repeat(40),title:args.input.title||'Actual commit subject',body:args.input.body??'Template and saved validation',commits:[{sha:'b'.repeat(40),subject:'Actual commit subject'}],verification_summary:'Saved tests passed',template_path:'.github/pull_request_template.md',expires_at:Date.now()/1000+60};
 if(command==='task_pr_execute')return {action:args.action.action,repository:'acme/repo',branch:'feature',head:'b'.repeat(40),pr_number:null,url:null,message:'Explicit stub action complete'};
 throw new Error('Unexpected IPC '+command);
};
createRoot(document.getElementById('root')).render(<div className="bg-bg text-text p-4"><TaskRuntimePanel taskId={7}/><SpecEvidencePanel taskId={7} onOpenFile={()=>{}} onAttachReview={()=>{}}/><PullRequestComposer taskId={7}/></div>);
`);
 server=await createServer({root:fixture,configFile:false,plugins:[react()],resolve:{alias:{'@tauri-apps/api/core':path.join(fixture,'core.ts'),'@tauri-apps/api/event':path.join(fixture,'event.ts')}},server:{host:'127.0.0.1',port:0,fs:{allow:[process.cwd()]}}});await server.listen();
 browser=await chromium.launch({channel:process.env.DOJANG_SMOKE_BROWSER||'chrome'});const page=await browser.newPage({viewport:{width:1200,height:1050}});page.setDefaultTimeout(15000);
 const errors=[];page.on('pageerror',e=>errors.push(e.message));await page.goto(server.resolvedUrls.local[0]+'index.html',{waitUntil:'domcontentloaded'});
 await page.getByText('잔여 작업 존재',{exact:true}).waitFor();await page.getByText('자식 프로세스: 관측 불가',{exact:true}).waitFor();
 await page.getByText(/벤더 기능 지원표/).click();await page.getByRole('cell',{name:'미지원',exact:true}).waitFor();
 await page.getByRole('textbox',{name:'명세 검색어',exact:true}).fill('fixture');await page.getByRole('button',{name:'명세 검색',exact:true}).click();
 await page.getByRole('button',{name:/Fixture spec/}).click();await page.getByRole('button',{name:'문서 링크 검사',exact:true}).click();await page.getByText(/파일 없음 1개/).waitFor();
 const prepare=page.getByRole('button',{name:'PR 미리보기 만들기',exact:true});await prepare.click();await page.getByRole('textbox',{name:'PR 제목',exact:true}).evaluate(node=>{if(node.value!=='Actual commit subject')throw new Error('initial default title missing')});
 assert.equal(await page.evaluate(()=>window.calls.filter(call=>call.command==='task_pr_execute').length),0);
 await page.getByRole('textbox',{name:'PR 본문',exact:true}).fill('Edited exact body');assert.equal(await page.getByRole('button',{name:'브랜치 push',exact:true}).isDisabled(),true);
 await prepare.click();await page.getByRole('button',{name:'브랜치 push',exact:true}).click();await page.getByText('Explicit stub action complete',{exact:true}).waitFor();
 const mutations=await page.evaluate(()=>window.calls.filter(call=>call.command==='task_pr_execute'));assert.equal(mutations.length,1);assert.equal(mutations[0].args.action.action,'push');
 assert.equal(await page.getByRole('button',{name:'PR 만들기',exact:true}).isDisabled(),true);assert.deepEqual(errors,[]);
 await page.screenshot({path:path.join(directory,'followups.png')});console.log(JSON.stringify({status:'passed',checks:['observed residual work','unknown children','vendor support table','current document link validation','default PR preview','no automatic remote action','edit invalidates preview','explicit isolated push action'],screenshot:path.join(directory,'followups.png')},null,2));
}finally{await browser?.close();await server?.close();await rm(fixture,{recursive:true,force:true});}
