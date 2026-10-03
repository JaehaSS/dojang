import { invoke } from "@tauri-apps/api/core";
import type { TaskDiffResult } from "../transport";
import type {
  AnnotationSaveInput,
  BranchList,
  BrowseResult,
  ComposeOutcome,
  CrystallizeResult,
  DiffHunk,
  EnsembleMatrix,
  FileContent,
  FsNode,
  GhRepo,
  GithubIssuesResult,
  GrillNote,
  GrillRound,
  HunkRef,
  InterviewAssessment,
  PartialApplyResult,
  QuickOpenTaskCandidate,
  RematchedAnnotation,
  ReviewAnnotation,
  Schedule,
  SessionHomeEntry,
  SessionHomePreview,
  SkillMeta,
  Task,
  TaskRow,
  VerifyPreview,
  VerifyReport,
} from "../ipc";
import { LOCAL_HOST, SessionResumeError, parseSessionResumeConflictTaskId } from "../transport";
import type {
  PraxisTransport,
  ScheduleCreateRequest,
  SessionHomeSession,
  TaskCreateRequest,
} from "../transport";

/** 서버 행을 화면 타입으로 승격 — 로컬 command는 host를 모르므로 여기서 붙인다. */
const local = (row: TaskRow): Task => ({ ...row, host: LOCAL_HOST });

export const tauriTransport: PraxisTransport = {
  kind: "local",
  hostId: LOCAL_HOST,
  taskList: async () => (await invoke<TaskRow[]>("task_list")).map(local),
  taskDiffStat: (id) => invoke<string>("task_diff_stat", { id }),
  taskDiff: (id, range, includeReview) =>
    invoke<TaskDiffResult>("task_diff", { id, range, includeReview }),
  fsTree: (id) => invoke<FsNode[]>("fs_tree", { id }),
  fsTreePath: (repository) => invoke<FsNode[]>("fs_tree_path", { path: repository }),
  fsBrowse: (path) => invoke<BrowseResult>("fs_browse", { path }),
  fsRoots: () => invoke<string[]>("fs_roots"),
  fsCreateFile: (dir, name) => invoke<string>("fs_create_file", { dir, name }),
  fsCreateDir: (dir, name) => invoke<string>("fs_create_dir", { dir, name }),
  fsRename: (path, name) => invoke<string>("fs_rename", { path, name }),
  fsTrash: (path) => invoke<void>("fs_trash", { path }),
  fsCopy: (src, destDir) => invoke<string>("fs_copy", { src, destDir }),
  fsOpenTerminal: (path) => invoke<void>("fs_open_terminal", { path }),
  gitStatus: (path) => invoke<boolean>("git_status_path", { path }),
  gitInit: (path) => invoke<boolean>("git_init_path", { path }),
  gitBranches: (path) => invoke<BranchList>("git_branches_path", { path }),
  gitCheckoutBranch: (path, branch) =>
    invoke<BranchList>("git_checkout_branch_path", { path, branch }),
  repositoryList: async () => [],
  fsRead: (id, path) => invoke<FileContent>("fs_read", { id, path }),
  fsWrite: (id, path, content) => invoke<number>("fs_write", { id, path, content }),
  scheduleList: () => invoke<Schedule[]>("schedule_list"),
  taskCreate: async (request: TaskCreateRequest) => {
    const {
      goal_contract: goalContract,
      reasoning_effort: reasoningEffort,
      service_tier: serviceTier,
      ambiguity,
      base_branch: baseBranch,
      client_ref: clientRef,
      purpose_selection: purposeSelection,
      ...commandArgs
    } = request;
    try {
      return local(await invoke<TaskRow>("task_create", {
        ...commandArgs,
        ...(goalContract != null ? { goalContract } : {}),
        ...(purposeSelection ? { purposeSelection } : {}),
        ...(reasoningEffort?.trim() ? { reasoningEffort: reasoningEffort.trim() } : {}),
        ...(serviceTier != null ? { serviceTier } : {}),
        ...(ambiguity != null ? { ambiguity } : {}),
        // 빈 값은 아예 보내지 않는다 — 백엔드의 "현재 checkout에서 시작" 기본 동작과 같다.
        ...(baseBranch?.trim() ? { baseBranch: baseBranch.trim() } : {}),
        // 없으면 보내지 않는다 — 백엔드는 이 값이 있을 때만 진행 이벤트를 낸다.
        ...(clientRef ? { clientRef } : {}),
      }));
    } catch (error) {
      // 로컬 task_create는 문자열 오류뿐이다(Result<T, String>). 세션 승계 실패는
      // "이어받기에 실패해 작업을 시작하지 않았습니다: <reason>" 접두로만 식별한다 —
      // 다른 검증 오류(빈 instruction 등)까지 승계 거절로 오분류하지 않기 위해서다.
      // taskId는 "이미 #<id> 작업이 이어가고 있습니다" 정규식으로 뽑는다(원격은 409 JSON의
      // task_id를 직접 쓴다, transport/runner.ts).
      if (
        request.resumeSession &&
        typeof error === "string" &&
        error.includes("이어받기에 실패해 작업을 시작하지 않았습니다")
      ) {
        const taskId = parseSessionResumeConflictTaskId(error);
        throw taskId != null
          ? new SessionResumeError(error, "conflict", taskId)
          : new SessionResumeError(error, "not_found");
      }
      throw error;
    }
  },
  taskResume: async (id, message, clientRef) =>
    local(
      await invoke<TaskRow>("task_resume", {
        id,
        message,
        // 없으면 보내지 않는다 — taskCreate와 같은 관례(백엔드는 있을 때만 진행 이벤트를 낸다).
        ...(clientRef ? { clientRef } : {}),
      }),
    ),
  taskInput: (id, data) => invoke("task_write", { id, data }),
  taskMessage: (id, message) => invoke("convo_send", { id, message }),
  taskModelSet: (id, model) => invoke("task_model_set", { id, model }),
  conversationSubmit: (taskId, requestId, message, imagePaths, previewClientRef) =>
    invoke("conversation_submit", { taskId, requestId, message, imagePaths, ...(previewClientRef ? { previewClientRef } : {}) }),
  conversationReceipt: (taskId, requestId) => invoke("conversation_receipt", { taskId, requestId }),
  sideQuestionRead: (taskId) => invoke("side_question_read", { taskId }),
  sideQuestionSend: (taskId, input) => invoke("side_question_send", { taskId, input }),
  sideQuestionCancel: (taskId, turnId) => invoke("side_question_cancel", { taskId, turnId }),
  sideQuestionReset: (taskId, generation) => invoke("side_question_reset", { taskId, generation }),
  taskCancel: (id) => invoke("task_cancel", { id }),
  taskDelete: (id) => invoke("task_delete", { id }),
  taskApprove: (id) => invoke("task_approve", { id }),
  taskApprovalStatus: (id) => invoke("task_approval_status", { id }),
  approvalRepairStatus: (id) => invoke("approval_repair_status", { id }),
  approvalRepairPrepare: (id) => invoke("approval_repair_prepare", { id }),
  approvalRepairRun: (id, sessionId) => invoke("approval_repair_run", { id, sessionId }),
  approvalRepairAccept: (id, sessionId) => invoke("approval_repair_accept", { id, sessionId }),
  approvalRepairCancel: (id, sessionId) => invoke("approval_repair_cancel", { id, sessionId }),
  taskDiscard: (id) => invoke("task_discard", { id }),
  taskRunApprove: (id) => invoke("task_run_approve", { id }),
  taskRunReject: (id) => invoke("task_run_reject", { id }),
  githubIssuesList: (repo) => invoke<GithubIssuesResult>("github_issues_list", { repo }),
  githubReposList: (repos) => invoke<GhRepo[]>("github_repos_list", { repos }),
  githubCreateTaskFromIssue: async (repo, issueNumber, agent) =>
    local(await invoke<TaskRow>("github_create_task_from_issue", { repo, number: issueNumber, agent })),
  githubIssueDelete: (repo, issueNumber) =>
    invoke<void>("github_issue_delete", { repo, number: issueNumber }),
  verifySpec: (id) => invoke<VerifyPreview>("verify_spec", { id }),
  taskVerify: (id, previewToken) =>
    invoke<VerifyReport>("task_verify", { id, previewToken }),
  reviewProcessQuarantines: async () => [],
  reviewProcessReconcile: async () => {
    throw new Error("review process 복구는 원격 Runner 전용입니다");
  },
  scheduleAdd: (request: ScheduleCreateRequest) =>
    invoke<number>("schedule_add", {
      label: request.label,
      cron: request.cron,
      kind: request.kind,
      payload: request.payload,
      tz_offset_secs: request.tzOffsetSecs,
    }),
  scheduleRemove: (id) => invoke("schedule_remove", { id }),
  scheduleSetEnabled: (id, enabled) => invoke("schedule_set_enabled", { id, enabled }),
  reminderAdd: (text, delayMinutes) => invoke<number>("reminder_add", { text, delayMinutes }),
  skillsList: (repo) => invoke<SkillMeta[]>("skills_list", { repo }).catch(() => []),
  quickopenSearch: (query, scopes) =>
    invoke<QuickOpenTaskCandidate[]>("quickopen_search", { query, scopes }),
  sessionHomeIndex: async (repo, all, query): Promise<SessionHomeSession[]> =>
    (await invoke<SessionHomeEntry[]>("session_home_index", { repo, all, query })).map(
      (row) => ({ ...row, host: LOCAL_HOST, vendor: row.vendor ?? "claude", preview_supported: true }),
    ),
  sessionHomePreview: (repo, vendor, sessionId) =>
    invoke<SessionHomePreview>("session_home_preview", { repo, vendor, sessionId }).then((preview) => ({
      ...preview,
      meta: { ...preview.meta, vendor: preview.meta.vendor ?? vendor },
    })),
  diffHunks: (id, range) => invoke<DiffHunk[]>("diff_hunks", { id, range }),
  annotationsList: (taskId, range) =>
    invoke<RematchedAnnotation[]>("annotations_list", { taskId, range }),
  annotationSave: (taskId, input: AnnotationSaveInput) =>
    invoke<ReviewAnnotation>("annotation_save", {
      taskId,
      id: input.id ?? null,
      hunkId: input.hunk_id,
      path: input.path,
      line: input.line,
      side: input.side,
      bodyMd: input.body_md,
    }),
  annotationsResend: (taskId, ids) => invoke("annotations_resend", { taskId, ids }),
  partialApply: (taskId, hunkIds) =>
    invoke<PartialApplyResult>("partial_apply", { id: taskId, hunkIds }),
  partialRollback: (taskId) => invoke("partial_rollback", { id: taskId }),
  ensembleMatrix: (ensemble) => invoke<EnsembleMatrix>("ensemble_matrix", { ensemble }),
  ensembleCompose: (ensemble, winnerTaskId, selections: HunkRef[]) =>
    invoke<ComposeOutcome>("ensemble_compose", { ensemble, winnerTaskId, selections }),
  interviewStart: (repo, instruction, agent) =>
    invoke<InterviewAssessment>("interview_start", { repo, instruction, agent }),
  interviewCrystallize: (repo, instruction, answers, agent) =>
    invoke<CrystallizeResult>("interview_crystallize", { repo, instruction, answers, agent }),
  grillRound: (repo, instruction, transcript, agent) =>
    invoke<GrillRound>("grill_round", { repo, instruction, transcript, agent }),
  grillNote: (repo, instruction, transcript, agent) =>
    invoke<GrillNote>("grill_note", { repo, instruction, transcript, agent }),
  grillSaveNote: (repo, slug, markdown, date) =>
    invoke<string>("grill_save_note", { repo, slug, markdown, date }),
};
