import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { randomBytes } from 'node:crypto';
import { createServer } from 'vite';
const binary=process.env.DOJANG_NATIVE_PROBE_APP;
if(!binary)throw new Error('DOJANG_NATIVE_PROBE_APP is required');
const root=await mkdtemp(path.join(tmpdir(),'dojang-thinkrail-native-'));
const nonce=randomBytes(16).toString('hex');
const repo=path.join(root,'repo');
let server,child,exit,timer;
try{
  await mkdir(repo);await writeFile(path.join(root,'dojang-probe-owned'),nonce);
  const git=(...args)=>execFileSync('git',['-C',repo,...args],{encoding:'utf8',timeout:10000,stdio:['ignore','pipe','pipe']});
  git('init','-b','main');git('config','user.name','Dojang Probe');git('config','user.email','probe@example.invalid');
  await writeFile(path.join(repo,'README.md'),'# Native fixture\n');git('add','.');git('-c','core.hooksPath=/dev/null','commit','-m','Native base');
  git('checkout','-b','native-probe');git('remote','add','origin','https://github.com/JetBrains/thinkrail.git');
  await writeFile(path.join(repo,'spec.md'),`# Frozen input\nNATIVE_SPEC_${nonce}\n[valid](README.md) [missing](absent.md)\n`);
  git('add','.');git('-c','core.hooksPath=/dev/null','commit','-m','Native feature');
  server=await createServer({server:{host:'127.0.0.1',port:0,strictPort:false}});await server.listen();
  child=spawn(binary,[],{env:{...process.env,DOJANG_NATIVE_PROBE_ROOT:root,DOJANG_NATIVE_PROBE_URL:server.resolvedUrls.local[0],DOJANG_NATIVE_LIVE_VENDOR:process.env.DOJANG_NATIVE_LIVE_VENDOR||'1'},stdio:['ignore','pipe','pipe']});
  let out='',err='';child.stdout.on('data',chunk=>out=(out+chunk).slice(-60000));child.stderr.on('data',chunk=>err=(err+chunk).slice(-12000));
  exit=new Promise((resolve,reject)=>{child.once('exit',(code,signal)=>resolve({code,signal}));child.once('error',reject)});
  const ended=await Promise.race([exit,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('native child deadline')),115000)})]);
  const record=out.split('\n').filter(line=>line.startsWith('{')).map(line=>{try{return JSON.parse(line)}catch{return null}}).find(value=>value?.probe==='thinkrail-native');
  assert.ok(record,`no native evidence: ${out}\n${err}`);
  console.log(JSON.stringify(record,null,2));
  assert.equal(ended.code,0,`native exit ${ended.code}: ${err}`);assert.equal(record.passed,true);
  await mkdir(path.resolve('.dojang/verification/thinkrail-followups'),{recursive:true});
  await writeFile(path.resolve('.dojang/verification/thinkrail-followups/native-evidence.json'),JSON.stringify(record,null,2));
}finally{
  clearTimeout(timer);
  if(child&&child.exitCode===null&&child.signalCode===null){child.kill('SIGTERM');await Promise.race([exit,new Promise(resolve=>setTimeout(resolve,4000))]);if(child.exitCode===null&&child.signalCode===null){child.kill('SIGKILL');await exit;}}
  await server?.close();await rm(root,{recursive:true,force:true});
}
