(async () => {
  if(window.__DOJANG_NATIVE_PROBE_RUNNING__)return;
  window.__DOJANG_NATIVE_PROBE_RUNNING__=true;
  const c=window.__DOJANG_NATIVE_PROBE__;
  const invoke=(command,args={})=>window.__TAURI_INTERNALS__.invoke(command,args);
  const check=(value,message)=>{if(!value)throw new Error(message)};
  const until=async(fn,message,timeout=15000)=>{const end=Date.now()+timeout;while(Date.now()<end){if(await fn())return;await new Promise(resolve=>setTimeout(resolve,150));}throw new Error(message)};
  const evidence={agent:c.agent};
  try {
    await until(()=>document.querySelector('aside'),"production App did not mount");
    await until(()=>document.body.innerText.includes("Native archive record"),"seeded task not visible");
    await invoke('task_archive',{id:c.archiveId});
    await until(()=>document.querySelector(`[data-archive-task-key="local:${c.archiveId}"]`),"archive did not reach production sidebar");
    evidence.archiveUi=true;
    await invoke('plugin:event|emit',{event:'task://state',payload:{id:c.archiveId,state:'AwaitingReview',awaiting_kind:null}});
    await new Promise(resolve=>setTimeout(resolve,600));
    check(document.querySelector(`[data-archive-task-key="local:${c.archiveId}"]`),"delayed state event lost archive");
    evidence.staleStatePreserved=true;
    document.querySelector(`[data-archive-task-key="local:${c.archiveId}"]`).click();
    await until(()=>document.body.innerText.includes('Native preserved answer'),"missing-worktree transcript did not open");
    evidence.missingWorktreeHistory=true;
    await invoke('task_restore',{id:c.archiveId});
    await until(()=>!document.querySelector(`[data-archive-task-key="local:${c.archiveId}"]`),"restore did not reach sidebar");
    evidence.restoreUi=true;
    const doc=await invoke('task_spec_read',{id:c.vendorId,relativePath:'spec.md'});
    const report=await invoke('task_spec_validate_links',{id:c.vendorId,relativePath:'spec.md',expectedHash:doc.content_hash});
    check(report.summary.ok===1 && report.summary.missing===1,"native link report mismatch");
    evidence.specLinks=true;
    const saved=await invoke('task_spec_save',{id:c.vendorId,input:{relative_path:'spec.md',expected_hash:doc.content_hash}});
    await invoke('fs_write',{id:c.vendorId,path:'spec.md',content:'Changed current document'});
    const frozen=await invoke('task_spec_get',{id:c.vendorId,snapshotId:saved.id});
    check(frozen.content===doc.content,"native frozen spec changed");evidence.frozenSpec=true;
    const gh=await invoke('task_pr_status_get',{id:c.vendorId});
    evidence.liveGithub={status:gh.status,repository:gh.repository,message:gh.message};
    check(gh.status==='no_pr'||gh.status==='ready',`live GitHub read failed: ${gh.message}`);
    const observed=await invoke('task_observation_get',{id:c.vendorId});
    check(observed.status==='review_needed' && observed.child_processes===null,"observation claimed false completion");
    evidence.observation=true;
    if(c.liveVendor){
      const reference=`[[dojang-spec:${saved.id}:${saved.content_hash}]]`;
      await invoke('convo_send',{id:c.vendorId,message:`This is a read-only integration check. Do not use tools or modify files. Reply only with the NATIVE_SPEC hexadecimal marker from this quoted frozen reference. ${reference}`,imagePaths:[]});
      await until(async()=>{
        const history=await invoke('convo_history',{id:c.vendorId});
        if(history.busy)return false;
        const expanded=history.items.filter(item=>item.kind==='user_expanded').map(item=>item.text??'').join('');
        const reply=history.items.filter(item=>item.kind==='text' && !item.parent_id).map(item=>item.text??'').join('');
        check(expanded.includes(doc.content),'frozen content absent in transmitted record');
        check(reply.includes(`NATIVE_SPEC_${c.nonce}`),`real vendor did not return frozen marker: ${reply.slice(-300)}`);
        return true;
      },'real vendor did not settle',65000);
      const after=await invoke('task_observation_get',{id:c.vendorId});
      check(!after.registered_execution,'real vendor execution lease survived');
      evidence.realVendorFrozenInput=true;
    }
    await invoke('thinkrail_probe_finish',{nonce:c.nonce,evidence});
  } catch(error) {
    await invoke('thinkrail_probe_finish',{nonce:c.nonce,evidence:{...evidence,error:String(error)}});
  }
})();
