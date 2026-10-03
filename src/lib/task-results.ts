import { invoke } from "@tauri-apps/api/core";
export interface VerificationReceipt { verify_run_id:string; result_revision:number; started_at:number; finished_at:number; outcome:string; freshness:string; logs_ref:string|null; observation_scope:string; command_spec_hash:string; report?:{build:{command:string;exit_code:number;tail:string}|null;test:{command:string;exit_code:number;tail:string}|null}|null; }
export interface ResultReference { result_revision?:number; kind:string; relative_path:string; created_at:number; availability:string; }
export interface TaskResult { task_id:number; source_id:string; result_revision:number; completion_epoch:number; freshness:string; fingerprint:string|null; receipts:VerificationReceipt[]; references:ResultReference[]; }
export interface ReviewRound {id:string;task_id:number;source_id:string;result_revision:number;completion_epoch:number;revision:number;state:string;criteria:{text:string;checked:boolean}[];active:boolean;}
export interface ReviewSnapshot {source_id:string;result_revision:number;completion_epoch:number;active:ReviewRound|null;rounds:ReviewRound[];decisions:{action:string;reason:string;round_id:string;created_at:number}[];deliveries:{mutation_id:string;state:string;error:string|null}[];}
export interface ReviewInput {source_id:string;result_revision:number;round_id:string|null;expected_revision:number;mutation_id:string;action:string;reason:string;criteria:string[];checked:boolean[];}
export const taskResultGet=(id:number)=>invoke<TaskResult>("task_result_get",{id});
export const taskResultReference=(id:number,sourceId:string,expectedRevision:number,kind:string,relativePath:string)=>invoke<TaskResult>("task_result_reference",{id,sourceId,expectedRevision,kind,relativePath});
export const taskReviewGet=(id:number)=>invoke<ReviewSnapshot>("task_review_get",{id});
export const taskReviewDecide=(id:number,input:ReviewInput)=>invoke<ReviewSnapshot>("task_review_decide",{id,input});

export const taskReviewDeliver=(id:number,sourceId:string,mutationId:string)=>invoke<ReviewSnapshot>("task_review_deliver",{id,sourceId,mutationId});
