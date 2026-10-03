import { ApprovalReadinessPanel } from "./components/ide/ApprovalReadinessPanel";
import { ApprovalRepairPanel } from "./components/ide/ApprovalRepairPanel";
import { QuestionSession } from "./components/ide/QuestionSession";
import { useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState } from "react";
import { emitTo, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { homeDir } from "@tauri-apps/api/path";
import { TerminalView } from "./components/TerminalView";
import { vaultLocalComposerSend } from "./lib/knowledge-vault-ipc";
import { DiffSessionScope } from "./components/DiffSessionContext";
import { PreviewTab } from "./components/ide/PreviewTab";
import { usePreviewWorkbenchHost } from "./components/use-preview-workbench-host";
import { usePreviewActivation } from "./components/use-preview-activation";
import {
  PREVIEW_TOOLBAR_REQUEST,
  type ToolbarMessage,
} from "./lib/preview-workbench/window-events";
import { ToolbarHost } from "./lib/preview-workbench/toolbar-host";
import { Sidebar, type View } from "./components/ide/Sidebar";
import { HomeView } from "./components/ide/HomeView";
import { projectEditorOpen } from "./lib/project-editor-ipc";
import { EnsembleView } from "./components/ide/EnsembleView";
import { ConflictResolver } from "./components/ide/ConflictResolver";
import { CheckpointMenu } from "./components/ide/CheckpointMenu";
import {
  ConversationView,
  eventToItems,
  appendConvoEvent,
  coalesceConvoEvent,
  type ConvoItem,
  type ConvoEventLike,
} from "./components/ide/ConversationView";
import { ActivityColumnTab, ActivityRail } from "./components/ide/ActivityRail";
import { DebateView } from "./components/ide/DebateView";
import { endsSequence } from "./components/ide/debate-rounds";
import { applyStateOverrides, noteStateOverride, type StateOverrides } from "./lib/task-state-overlay";
import type { DebateEventLike } from "./components/ide/debate-rounds";
import { debateSessionWidth, MIN_SESSION_WIDTH } from "./components/ide/workspace-split-width";
import {
  FLOATING_CHANNEL_HEADER_INSET,
  FLOATING_CHANNEL_RESERVED,
  PANE_LABELS_MIN_CENTER,
  channelHandleVisible,
  channelPlacement,
  floatingChannelMinCenter,
} from "./components/ide/activity-rail";
import { ContextGauge } from "./components/ide/ContextGauge";
import { ContextResetButton } from "./components/ide/ContextResetButton";
import { WorkspaceSplit } from "./components/ide/WorkspaceSplit";
import { SessionDiffSurface } from "./components/ide/SessionDiffSurface";
import { TaskVendorReview } from "./components/ide/TaskVendorReview";
import { WorkspacePaneButtons } from "./components/ide/WorkspacePaneButtons";
import { useSessionPanels } from "./components/ide/useSessionPanels";
import { CodeColumnTabs, type CodeTab } from "./components/ide/CodeColumnTabs";
import { SessionModelSwitch } from "./components/ide/SessionModelSwitch";
import {
  EMPTY_CONTEXT_OBSERVATION,
  foldContextObservation,
  MODEL_CHANGE_INVALIDATION,
} from "./lib/context-observation";
import { EMPTY_MODEL_SNAPSHOT, foldModelSnapshot } from "./lib/model-snapshot";
import {
  loadProjectGroups,
  saveProjectGroups,
  unassignProject,
  type ProjectGroups,
} from "./lib/project-groups";
import type { WorkContextDiff } from "./components/ide/WorkContextPanel";
import { SubagentView } from "./components/ide/SubagentView";
import { subagentRootMap, subagentThreadRootOf, subagentThreads } from "./lib/activity";
import { TerminalDock } from "./components/ide/TerminalDock";
import { UsageBar } from "./components/ide/UsageBar";
import { InsightsView } from "./components/ide/InsightsView";
import { VocabView } from "./components/ide/VocabView";
import { PipelineView } from "./components/ide/pipeline/PipelineView";
import { Composer } from "./components/ide/Composer";
import { refreshMessage, type BaseRefreshEvent } from "./lib/base-refresh";
import type { CreatingEvent, CreationState } from "./lib/creation-stage";
import { isDirectRun } from "./components/ide/discard-confirm";
import { parseCodeItemId } from "./lib/quickopen";
import { QUESTION_AGENTS } from "./lib/conversation-interaction";
import { onKeyDown as shiftDown, onKeyUp as shiftUp, type ShiftTapState } from "./lib/shift-double-tap";
import { AgentComposer } from "./components/ide/AgentComposer";
import { SideQuestionPanel } from "./components/ide/SideQuestionPanel";
import { QuestionReferenceAttachments } from "./components/ide/QuestionReferenceAttachments";
import { useQuestionReferences } from "./components/ide/useQuestionReferences";
import { ConversationSubmitter } from "./lib/conversation-submit";
import { useConversationQueue } from "./components/use-conversation-queue";
import { ConversationQueue } from "./components/ide/ConversationQueue";
import { contextSourceHash, formatQuestionReferences, type SideQuestionApi, type SideQuestionContext } from "./lib/side-question";
import type { SplitMode } from "./components/ide/workspace-split-width";
import { requestComposerFocus } from "./lib/composer-focus";
import { useSessionDraft } from "./components/ide/useSessionDraft";
import { FileTree } from "./components/ide/FileTree";
import { ChangedFileCount, ChangesList } from "./components/ide/ChangesList";
import { previewTabsEnabled, setPreviewTabsEnabled } from "./lib/preview-tabs";
import { useExperimentalFeatures } from "./lib/experimental-features";
import { useSessionStyle } from "./lib/session-style";
import { useQuestionThreads } from "./lib/use-question-threads";
import { fileTabKey } from "./lib/tab-key";
import { useTreeFileOps } from "./components/ide/useTreeFileOps";
import { ContextMenuShell } from "./components/ide/ContextMenuShell";
import { FilePromptDialog } from "./components/ide/FilePromptDialog";
import { targetDir } from "./components/ide/tree-file-ops";
import { EditorSplitView } from "./components/ide/EditorSplitView";
import { useWorkspaceFiles } from "./components/ide/useWorkspaceFiles";
import { useEditorActions } from "./components/ide/useEditorActions";
import { useEditorPopOutShortcut } from "./components/ide/useEditorPopOutShortcut";
import { useEditorWindowHost } from "./components/use-editor-window-host";
import { AutosaveUndoBar, type AutosaveNotice } from "./components/ide/AutosaveUndoBar";
import {
  EDITOR_REVERTED_EVENT,
  EDITOR_WINDOW_LABEL,
  type EditorAskPayload,
  type EditorNotificationPayload,
  type EditorStatus,
} from "./lib/editor-window-events";
import { buildSelectionCapture } from "./lib/designmode/selection-capture";
import { formatCapturesPrompt } from "./lib/designmode/prompt";
import { pushCapture } from "./lib/designmode/store";
import { buildAskPayload } from "./lib/selection-ask";
import { SettingsPanel, type SettingsTab } from "./components/ide/SettingsPanel";
import { TaskResultPanel } from "./components/ide/TaskResultPanel";
import { CapsulePanel } from "./components/ide/CapsulePanel";
import { Menu } from "./components/ide/Menu";
import { Icon } from "./components/ide/icons";
import { QuickOpen } from "./components/QuickOpen";
import { SessionNavigator } from "./components/SessionNavigator";
import type { QuickOpenScope, RankedQuickOpenItem } from "./lib/quickopen";
import {
  ACTIVE_STATES,
  taskCreate,
  taskResume,
  taskListAll,
  taskWrite,
  taskDiffStat,
  taskApprove,
  taskDiscard,
  conflictPathsFromError,
  tasksDiscardOrphans,
  defaultShell,
  flattenFiles,
  fsWrite,
  openLocalFile,
  resolveAbsPath,
  editorWindowFocus,
  taskCapsule,
  capsuleInject,
  ensembleList,
  convoHistory,
  convoStatus,
  convoInterrupt,
  debateRoundCapGet,
  debateSides,
  type DebateSide,
  type ConvoStatus,
  type ConvoEvent,
  type Capsule,
  type Task,
  fontSettingsGet,
  editorSettingsGet,
  type FontSettings,
  type EditorSettings,
  interviewStart,
  interviewCrystallize,
  grillRound,
  grillNote,
  grillSaveNote,
  useWorktreeGet,
  taskRef,
  insightCards,
} from "./lib/ipc";
import { countUnseen } from "./lib/insight-seen";
import type { HostFailure } from "./lib/task-list-merge";
import { hostCapabilities, LOCAL_ONLY_REASON } from "./lib/host-capabilities";
import { HostScopeProvider } from "./lib/host-scope";
import { codeFontStack, uiFontStack } from "./lib/fonts";
import {
  DEFAULT_EDITOR_SETTINGS,
  applyTreeMetrics,
  normalizeEditorSettings,
} from "./lib/editor-settings";
import { singleAgentSelection } from "./lib/agents";
import { isTerminalState, taskStatusLabel, taskTextClass, taskTone } from "./lib/task-status";
import { inferAgentRole } from "./lib/agent-role";
import { normalizeReasoningEffort } from "./lib/models";
import {
  LOCAL_HOST,
  getTransport,
  hasHost,
  listHosts,
  subscribeTransportChange,
  taskKey,
  SessionResumeError,
  type HostId,
  type PraxisTransport,
  type SessionHomeSession,
  type TaskRef,
} from "./lib/transport";
import { RunnerTransport } from "./lib/transport/runner";
import { describeSessionResumeError } from "./lib/session-resume";
import { resolveAgentLink, type AgentLinkTarget } from "./lib/agent-link";
import {
  LinkContextMenu,
  type LinkMenuAction,
  type LinkMenuKind,
  type LinkMenuState,
} from "./components/ide/LinkContextMenu";
import { removeTask, transportRemovalActions } from "./lib/task-removal";
import { applyTheme, counterpartOf } from "./lib/themes";
import { useTheme } from "./lib/use-theme";
import { useDeferredTaskRemoval } from "./components/ide/useDeferredTaskRemoval";
import { useNotificationCollector, useNotificationSnapshot } from "./lib/use-notification-snapshot";
import { usePendingQuestions } from "./lib/use-pending-questions";
import { notificationAcknowledge, type InboxItem } from "./lib/notifications";
import { RemovalUndoBar } from "./components/ide/RemovalUndoBar";
import { VoiceHUD, type VoiceHudState } from "./components/ide/VoiceHUD";
import { actionLabel, routeTranscript, type VoiceAction } from "./lib/voice-router";
import {
  initialInterviewState,
  interviewReducer,
  isStale as interviewIsStale,
  runAssessmentFlow,
  runCrystallizeFlow,
  type InterviewFlowDeps,
} from "./lib/interview";
import {
  grillReducer,
  initialGrillState,
  runGrillNote,
  runGrillRound,
  type GrillFlowDeps,
} from "./lib/grill";

const stateText: Record<string, string> = {
  Running: "text-status-running",
  AwaitingReview: "text-status-awaiting",
  PendingApproval: "text-status-awaiting",
  Finalizing: "text-status-awaiting",
  Done: "text-status-done",
  Failed: "text-status-failed",
  Discarded: "text-status-failed",
  Created: "text-text-secondary",
};

/** 검토 대기는 대기 성격에 따라 색이 갈린다 — 그 외 상태는 위 표를 그대로 쓴다. */
const stateTextFor = (task: Pick<Task, "state" | "awaiting_kind">): string =>
  task.state === "AwaitingReview"
    ? taskTextClass(task)
    : (stateText[task.state] ?? "text-text-secondary");

/** 리뷰 바 안내 한 줄. 바가 한 줄로 고정되며 잘릴 수 있어 title로도 같이 준다.
 *  답을 기다리는 턴에 "변경을 검토해 머지하세요"를 띄우면, 정작 필요한 행동(대답)이 아니라
 *  하지 않아도 될 행동을 안내하게 된다. */
const reviewBarMessage = (tone: string, direct: boolean): string =>
  tone === "question"
    ? "에이전트가 답을 기다리고 있습니다 — 아래에 답하면 이어서 진행합니다."
    : direct
      ? "이 대화 턴이 끝났습니다 — 변경을 검토해 승인하거나, 계속 대화하거나 버리세요."
      : "이 대화 턴이 끝났습니다 — 변경을 검토해 머지하거나, 계속 대화하거나 버리세요.";

const repoBase = (p: string) => p.split("/").filter(Boolean).pop() ?? p;
const tabBtn = (on: boolean) =>
  `h-6 px-2 rounded text-sm ${on ? "bg-bg text-text" : "text-text-secondary hover:text-text"}`;

/** Runner task output 한 행을 convo 이벤트로 해석 — kind 태그 JSON이 아니면(터미널 조각) null. */
const parseConvoEvent = (data: string): ConvoEventLike | null => {
  try {
    const value: unknown = JSON.parse(data);
    if (
      typeof value === "object" &&
      value !== null &&
      typeof (value as { kind?: unknown }).kind === "string"
    )
      return value as ConvoEventLike;
  } catch {
    /* convo 이벤트 아님 */
  }
  return null;
};

/** 원격 스트림 append — durable user 이벤트가 낙관적 user 말풍선과 겹치면 한 번만 남긴다. */
const appendConvoItems = (prev: ConvoItem[], items: ConvoItem[]): ConvoItem[] => {
  const last = prev[prev.length - 1];
  const first = items[0];
  if (last?.role === "user" && first?.role === "user" && last.text === first.text)
    return [...prev, ...items.slice(1)];
  return [...prev, ...items];
};

function App() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [notificationLoadRequest, setNotificationLoadRequest] = useState<string | null>(null);
  const [notificationRenderReady, setNotificationRenderReady] = useState<string | null>(null);
  const notificationResultRef = useRef<{
    host: HostId; taskId: number; sequence: number; requestId: string; resolve: (loaded: boolean) => void;
  } | null>(null);
  const popupNotificationRef = useRef<(payload: EditorNotificationPayload) => void>(() => {});
  const notifications = useNotificationSnapshot();
  const pendingQuestionIds = usePendingQuestions();
  const { reconcile: reconcileNotifications } = useNotificationCollector(
    notifications.snapshot,
    notifications.setSnapshot,
    notifications.setError,
  );
  // 선택은 (host, id) 좌표다 — 로컬 3번과 원격 3번은 서로 다른 작업이라 숫자 하나로는
  // 가리킬 수 없다 (ADR 0133 결정 4). 아래 파생값은 화면 코드가 그대로 쓰던 이름이다.
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const selected = tasks.find((task) => taskKey(task) === selectedKey) ?? null;
  const selectedId = selected?.id ?? null;
  const selectedHost = selected?.host ?? LOCAL_HOST;
  const selectedConnected = hasHost(selectedHost) && !selected?.stale;
  /** 지금 선택된 작업의 라우팅 좌표. ipc 래퍼에 넘기는 값이다. */
  const selectedCoord = selected ? taskRef(selected) : null;
  /** 선택된 작업의 호스트가 무엇을 할 수 있는가 — 손잡이를 누르기 전에 판정한다. */
  const selectedCaps = hostCapabilities(selectedHost);
  /** 승인이 머지 충돌로 막힌 작업 — 해소 화면을 띄운다. */
  /** 승인이 충돌로 막힌 작업의 좌표 — 해소 화면이 그 호스트로 다시 승인한다. */
  const [approvalRefresh, setApprovalRefresh] = useState(0);
  const [conflictTask, setConflictTask] = useState<TaskRef | null>(null);
  /** 이번 조회에서 응답하지 않은 호스트. 목록에서 빠진 이유를 사이드바가 남긴다. */
  const [hostFailures, setHostFailures] = useState<HostFailure[]>([]);
  /** 새 세션을 만들 환경. 생성 시점에 그 세션에 고정된다 (ADR 0133). */
  const [composerHost, setComposerHost] = useState<HostId>(() => {
    try {
      return window.localStorage.getItem("praxis-composer-host") ?? LOCAL_HOST;
    } catch {
      return LOCAL_HOST;
    }
  });
  const [view, setView] = useState<View>("home");
  /** 퀵오픈이 스킬 항목으로 설정을 열 때 지목하는 탭 — ADR 0191 결정 4. */
  const [settingsTab, setSettingsTab] = useState<SettingsTab | undefined>();
  const viewRef = useRef<View>(view);
  viewRef.current = view;
  // 키보드 핸들러(빈 deps)에서 현재 선택 작업의 mode를 읽기 위한 ref.
  const selectedRef = useRef<Task | null>(null);
  /** task://state 로컬 패치용 최신 tasks 미러 — 마운트 시 1회 등록되는 리스너의 stale 클로저 회피. */
  const tasksRef = useRef<Task[]>([]);
  /** 재조회가 도는 사이 도착한 로컬 상태 전이 — 늦게 온 스냅샷이 되돌리지 못하게 덧씌운다(task-state-overlay.ts). */
  const stateOverridesRef = useRef<StateOverrides>(new Map());
  /** 마지막으로 화면에 반영한 재조회의 시작 시각 — 더 먼저 시작한 조회가 더 늦게 끝나면 버린다. */
  const refreshAppliedAtRef = useRef(0);
  const [activeEnsemble, setActiveEnsemble] = useState<string | null>(null);
  const [repo, setRepo] = useState("");
  // 새 작업의 시작 브랜치. ""는 "고르지 않음" — 레포의 현재 체크아웃을 따른다.
  // 레포가 바뀌면 다른 레포의 브랜치명이 남지 않도록 비운다.
  const [baseBranch, setBaseBranch] = useState("");
  /** base 최신화 결과 한 줄. 이벤트 콜백이 최신 base 이름을 봐야 해서 ref로 따라 둔다. */
  const [baseRefreshNotice, setBaseRefreshNotice] = useState<string | null>(null);
  const baseBranchRef = useRef(baseBranch);
  baseBranchRef.current = baseBranch;
  useEffect(() => {
    setBaseBranch("");
  }, [repo]);
  // 리드 에이전트(pluggable). localStorage(JSON) 영속. 앙상블(2개+) 생성 진입점은 1.0에서 닫았다 — 항상 1개.
  const [agents, setAgentsState] = useState<string[]>(() => {
    try {
      const parsed = JSON.parse(localStorage.getItem("praxis-agents") || "null");
      if (Array.isArray(parsed) && parsed.length && parsed.every((x) => typeof x === "string"))
        return singleAgentSelection(parsed);
    } catch {
      /* ignore */
    }
    return ["claude"];
  });
  // 세션 단위 모델 오버라이드 ("" = 설정의 벤더 기본). 다음 작업 생성에만 쓰이므로 비영속.
  const [model, setModel] = useState("");
  // "고급·실험 기능 표시"가 꺼져 있으면 Design Mode 등을 숨긴다(설정 › 모양새).
  const advancedFeatures = useExperimentalFeatures();
  // 질문 응답 — 에이전트가 선택지 질문을 던지고 답을 받아 이어가는 질문형 세션. 정식 기능이라 토글과 무관하다.
  const [questionsEnabled,setQuestionsEnabled]=useState(false);
  // 실행 전 승인 — Claude 질문 세션에서 파일 수정·명령 실행 전에 CLI처럼 허용 여부를 묻는다.
  const [approvalsEnabled,setApprovalsEnabled]=useState(false);
  // 세션 방식(설정 › 작업 실행) — 터미널이면 턴마다 `-p`로 도는 대화 대신 실제 CLI를 PTY로 띄운다.
  const sessionStyle = useSessionStyle();
  const [serviceTier, setServiceTier] = useState<"default" | "fast">("default");
  // Codex 세션 단위 reasoning override ("" = Codex 설정 기본값).
  const [reasoningEffort, setReasoningEffort] = useState("");
  // 세션홈에서 고른 이어받기 대상 — 다음 작업 생성에만 쓰이므로 비영속(설계 2026-09-17).
  const [resumeSession, setResumeSession] = useState<SessionHomeSession | null>(null);
  const setSessionModel = (nextModel: string) => {
    setModel(nextModel);
    if (nextModel !== model) setServiceTier("default");
    setReasoningEffort((current) =>
      normalizeReasoningEffort(agents[0] ?? "", nextModel, current),
    );
  };
  const setAgents = (a: string[]) => {
    const next = singleAgentSelection(a);
    // 에이전트 선택이 바뀌면 모델 오버라이드 초기화 — 다른 벤더에 무효한 모델이 넘어가는 것을 방지.
    if (next[0] !== agents[0] || next.length !== agents.length) {
      setModel("");
      setReasoningEffort("");
      setServiceTier("default");
      // 원래 공급자와 다른 에이전트로는 같은 벤더 세션을 승계할 수 없다.
      setResumeSession((current) =>
        current && (next.length !== 1 || next[0] !== (current.vendor ?? "claude")) ? null : current,
      );
    }
    setAgentsState(next);
    localStorage.setItem("praxis-agents", JSON.stringify(next));
  };
  // 사이드바에 항상 표시할 프로젝트(레포) 목록 — 작업이 전부 종료돼도 유지, 명시 제거 전까지 남는다.
  const loadStrArray = (key: string): string[] => {
    try {
      const parsed = JSON.parse(localStorage.getItem(key) || "null");
      if (Array.isArray(parsed) && parsed.every((x) => typeof x === "string")) return parsed;
    } catch {
      /* ignore */
    }
    return [];
  };
  const [projects, setProjectsState] = useState<string[]>(() => loadStrArray("praxis-projects"));
  // 사이드바 프로젝트 묶음 — 조작은 사이드바가 하고 여기는 소유와 저장만 한다(설계 0062 D-8).
  const [projectGroups, setProjectGroups] = useState<ProjectGroups>(loadProjectGroups);
  const changeProjectGroups = (next: ProjectGroups) => {
    setProjectGroups(next);
    saveProjectGroups(next);
  };
  // 사용자가 명시 제거한 레포 — 이력이 남아있어도 tasks 기반 union으로 되살아나지 않도록 기억.
  const [removedProjects, setRemovedProjects] = useState<string[]>(() =>
    loadStrArray("praxis-projects-removed"),
  );
  const [instruction, setInstruction] = useState("");
  const [vaultDraftVersion, setVaultDraftVersion] = useState(0);
  const vaultClientRef = useMemo(() => crypto.randomUUID(), [composerHost, repo, vaultDraftVersion, agents, view, selectedKey, activeEnsemble]);
  const composerDraftsRef = useRef(new Map<HostId, {
    repo: string; instruction: string; agents: string[]; model: string; reasoningEffort: string; serviceTier: "default" | "fast";
  }>());
  // 인터뷰 2단계(채점) — 상태 전이는 순수 reducer, 호출 결과만 액션으로 전달.
  const [interview, dispatchInterview] = useReducer(
    interviewReducer,
    undefined,
    initialInterviewState,
  );
  // 인터뷰 1단계(깊게 파기) — 화면은 채점과 한 패널이지만 상태는 별개다. 산출물이 계약이 아니라
  // 노트라 phase 집합도 종료 조건도 달라, 한 reducer에 합치면 모든 전이에 모드 분기가 붙는다(Plan 0039 DR-P4).
  const [grill, dispatchGrill] = useReducer(grillReducer, undefined, initialGrillState);
  const [busy, setBusy] = useState(false);
  // 진행 중인 생성 하나 — busy가 잠금이라면 이쪽은 "어디까지 왔는가"다(설계 0059).
  const [creating, setCreating] = useState<CreationState | null>(null);
  const [err, setErr] = useState<string | null>(null);
  /** 세션 이어받기가 409(충돌)로 거절됐을 때만 채워진다 — 오류 배너에 그 작업으로 가는 링크를 낸다. */
  const [resumeConflictTaskId, setResumeConflictTaskId] = useState<number | null>(null);
  // 테마는 themes.ts가 소유한다(CSS 변수·.dark 클래스·localStorage). 여기서는 구독만 한다.
  const theme = useTheme();
  const light = theme.kind === "light";
  // 폰트 설정 — 미로드(null) 시 EditorPane/TerminalView는 자체 기본값(기존 하드코딩 값)을 사용.
  const [fontSettings, setFontSettings] = useState<FontSettings | null>(null);
  const [editorSettings, setEditorSettings] = useState<EditorSettings>(DEFAULT_EDITOR_SETTINGS);
  // 새 작업 격리 표시 — null 동안에는 로컬 단일 작업에 워크트리를 광고하지 않는다.
  const [useWorktree, setUseWorktree] = useState<boolean | null>(null);
  const shellRef = useRef<{ cmd: string; args: string[] }>({ cmd: "/bin/zsh", args: ["-l"] });

  // 기본은 에이전트 대화(출력) 중심. 에디터/파일트리는 평소 축소 — Cmd+B 또는 폴더 아이콘으로 표시.
  const [centerTab, setCenterTab] = useState<
    "editor" | "output" | "conversation" | `sub:${string}`
  >("output");

  /**
   * 패널 열림 상태는 세션의 속성이다(ADR 0163). 코드 열·터미널 도크·파일 트리·채널 핀이
   * 세션 좌표별로 갈려 살고, 전환과 같은 프레임에 그 세션의 모습으로 되돌아온다.
   *
   * 코드 열의 기본은 닫힘이고 헤더의 코드 버튼(⌥⌘S)이 연다(ADR 0111·0169). 사용자가 연
   * 상태는 이번 실행의 해당 세션에만 남고, 처음 여는 세션은 플로팅 채널에서 시작한다.
   */
  const panels = useSessionPanels(selectedKey);
  const { terminalDock, codeOpen, codeTab, centralDiffPath, showTree, channelPinned } = panels;
  const { setTerminalDock, setCodeTab, setShowTree, setChannelPinned, setCentralDiffPath } = panels;
  const { openDiffPanel } = panels;
  // 훅의 setter는 마운트 내내 identity가 고정이라, 단축키 핸들러가 한 번만 붙어도 최신을 본다.
  const showCode = panels.setCodeOpen;
  const toggleCode = useCallback(() => showCode((v) => !v), [showCode]);
  /**
   * 코드 열을 특정 탭으로 연다. 파일 열기·Diff 보기가 이걸 거치는 이유는, 닫힌 열 뒤에서
   * 탭만 바뀌면 사용자에게는 클릭이 아무 일도 하지 않은 것으로 보이기 때문이다.
   */
  const openCodeColumn = useCallback(
    (tab: CodeTab) => {
      setCodeTab(tab);
      showCode(true);
    },
    [setCodeTab, showCode],
  );
  const openSessionDiff = useCallback(
    (path: string) => {
      openDiffPanel();
      setCentralDiffPath(path);
    },
    [openDiffPanel, setCentralDiffPath],
  );
  // 워크스페이스(파일/에디터) 상태 — 에디터를 별도 창으로 빼낼 때 두 창이 같은 훅을 쓴다.
  // 파일이 실제로 열렸을 때만 중앙 탭을 에디터로 돌린다(읽기 실패 시엔 그대로 둔다).
  const workspaceSourceChangeRef = useRef<(path: string) => void>(() => undefined);
  const {
    tree,
    openFiles,
    activeKey,
    treeOpen,
    consumeTreeOpen,
    setActiveKey,
    activeFile,
    refreshTree,
    openFile,
    pinTab,
    pinAll,
    changeFile,
    saveFile,
    reloadFile,
    closeTab,
    closeTabsForPath,
    flushDirty,
  } = useWorkspaceFiles({
    task: selectedCoord,
    onError: setErr,
    onSourceChange: (path) => workspaceSourceChangeRef.current(path),
    onFileOpened: () => {
      setCenterTab("editor");
      openCodeColumn("file");
    },
  });
  // 파일 내용 밖의 동작 — OS로 넘기기·Finder·심볼 이동. 에디터 창도 같은 훅을 쓴다.
  const {
    revealTarget,
    setRevealTarget,
    revealAt,
    openPathExternal,
    revealPathInFinder,
    copyAbsPath,
    gotoSymbol,
    askLspStatus,
    openLspTarget,
  } = useEditorActions({ taskId: selectedId, host: selectedHost, onError: setErr, openFile });
  // 열려 있는 서브 에이전트 탭(Task tool_id) — 작업 전환 시 리셋 (openTask가 centerTab도 복구).
  const [subTabs, setSubTabs] = useState<string[]>([]);
  // 대화 모드(Phase 2) — claude stream-json 이벤트를 누적한 트랜스크립트.
  const [convoItems, setConvoItems] = useState<ConvoItem[]>([]);
  const questionRuntime=useRef(false);
  const updateQuestionBusy=useCallback((busy:boolean)=>{questionRuntime.current=true;setConvoBusy(busy);},[]);
  useEffect(()=>{questionRuntime.current=false;},[selectedKey]);
  /** 같은 이벤트의 원본 — 아이템에는 `speaker`가 남지 않아 토론 뷰가 라운드를 못 가른다. */
  const [convoEvents, setConvoEvents] = useState<DebateEventLike[]>([]);
  /** 우측 자리 — 있으면 이 작업은 토론 중이다. 좌측은 여전히 `tasks`가 원천이다. */
  /** 비좌측 자리들. `null`이면 토론이 아니다 — 빈 배열을 두지 않아 기존 `!= null` 판정이 그대로 선다. */
  const [debateSeats, setDebateSeats] = useState<DebateSide[] | null>(null);
  const vaultFollowupClientRef = useMemo(() => `vault-followup:${selectedId}:${crypto.randomUUID()}`, [selectedKey, selectedId, view, debateSeats, activeEnsemble]);
  const [debateRoundCap, setDebateRoundCap] = useState(3);
  const completeNotificationResult = (taskId: number, requestId: string | null, loaded: boolean) => {
    const pending = notificationResultRef.current;
    if (!pending || pending.requestId !== requestId || pending.host !== selectedHost || pending.taskId !== taskId) return;
    if (loaded) setNotificationRenderReady(requestId);
    else {
      notificationResultRef.current = null;
      pending.resolve(false);
    }
  };
  useLayoutEffect(() => {
    const pending = notificationResultRef.current;
    if (!pending || pending.requestId !== notificationRenderReady) return;
    if (selectedKey !== taskKey({ host: pending.host, id: pending.taskId })) return;
    if (view !== "workspace" || centerTab !== "conversation") return;
    notificationResultRef.current = null;
    pending.resolve(true);
  }, [centerTab, convoItems, notificationRenderReady, selectedKey, view]);
  /** 최근 본 세션의 트랜스크립트 — 재진입 때 조회를 기다리는 동안 빈 화면을 보이지 않게.
   *  삽입 순서를 최근 사용 순으로 유지해 오래된 것부터 버린다(세션당 수천 아이템이라 무한 보관 불가). */
  const convoCacheRef = useRef(new Map<string, ConvoItem[]>());
  // 원본 이벤트 캐시 — 토론 뷰는 speaker가 필요해서 표시 아이템만으로는 다시 그릴 수 없다.
  const convoEventsCacheRef = useRef(new Map<string, DebateEventLike[]>());
  const [convoBusy, setConvoBusy] = useState(false);
  const [convoLastEventAt, setConvoLastEventAt] = useState<number | null>(null);
  const [convoActivity, setConvoActivity] = useState<ConvoStatus | null>(null);
  const previewWorkbenchTasks = useMemo(
    () => tasks.map((task) => {
      const terminal = !ACTIVE_STATES.includes(task.state);
      const supported = task.host === LOCAL_HOST && !task.stale && !terminal && task.mode === "conversation" && (task.agent === "claude" || task.agent === "codex");
      let unsupportedReason: string | null = null;
      if (task.host !== LOCAL_HOST || task.stale) unsupportedReason = "원격 Runner에서는 프리뷰 질문을 지원하지 않습니다.";
      else if (!ACTIVE_STATES.includes(task.state)) unsupportedReason = "완료되었거나 삭제된 작업에서는 프리뷰 질문을 보낼 수 없습니다.";
      else if (task.mode !== "conversation") unsupportedReason = "터미널 작업에서는 프리뷰 질문을 지원하지 않습니다.";
      else if (task.agent !== "claude" && task.agent !== "codex") unsupportedReason = "Claude 또는 Codex 대화 작업에서만 프리뷰 질문을 지원합니다.";
      return { key: taskKey(task), taskId: task.id, supported, unsupportedReason, terminal };
    }),
    [tasks],
  );
  const previewWorkbench = usePreviewWorkbenchHost({
    tasks: previewWorkbenchTasks,
    onAccepted: (taskId, message, running) => {
      if (selectedHost !== LOCAL_HOST || selectedId !== taskId) return;
      setConvoItems((items) => appendConvoItems(items, [{ role: "user", text: message }]));
      setConvoEvents((events) => [...events, { kind: "user", text: message }]);
      setConvoBusy(running);
      setConvoLastEventAt(running ? Date.now() : null);
    },
  });
  const previewWorkbenchRef = useRef(previewWorkbench);
  const toolbarHostRef = useRef<ToolbarHost | null>(null);
  if (!toolbarHostRef.current) toolbarHostRef.current = new ToolbarHost(previewWorkbenchRef);
  previewWorkbenchRef.current = previewWorkbench;
  useEffect(() => {
    const unlisten = listen<ToolbarMessage>(PREVIEW_TOOLBAR_REQUEST, ({ payload }) => {
      void toolbarHostRef.current?.handle(payload);
    });
    return () => {
      toolbarHostRef.current?.clear();
      void unlisten.then((dispose) => dispose());
    };
  }, []);
  useEffect(() => {
    const unlisten = listen<number>("designmode://closed", ({ payload }) => toolbarHostRef.current?.closeTask(payload));
    return () => void unlisten.then((dispose) => dispose());
  }, []);
  useEffect(() => {
    void toolbarHostRef.current?.publish();
  });
  const refreshPreviewWorkbench = previewWorkbench.refresh;
  useEffect(() => {
    if (selectedId == null) return;
    const key = taskKey({ host: selectedHost, id: selectedId });
    void refreshPreviewWorkbench(key, selectedId);
  }, [refreshPreviewWorkbench, selectedHost, selectedId]);
  // 마지막 호출의 원자적 컨텍스트 관측. 모델 전환 중 늦게 온 이전 턴 관측은 다음 invocation까지 막는다.
  const [contextObservation, setContextObservation] = useState(EMPTY_CONTEXT_OBSERVATION);
  const contextTokens = contextObservation.observation?.contextTokens ?? null;
  // 선택 세션의 관측 실행 모델 — `model_snapshot`을 누적한 것. 두 소비자가 서로 다른 필드를
  // 본다: 모델 칩은 실제로 도는 것만 말할 수 있어야 해서 `resolved`만 보고, CTX % 게이지는
  // 윈도(1M 여부)만 알면 되므로 `requested`까지 폴백한다.
  const [modelSnapshot, setModelSnapshot] = useState(EMPTY_MODEL_SNAPSHOT);
  // 파일 트리(⌘B)와 터미널 도크(⌃`)는 위 useSessionPanels가 세션별로 들고 있다.
  // 도크는 우측 패널의 터미널 탭과 같은 백엔드 셸을 공유하므로, 열려 있으면 그쪽 탭은 셸 대신
  // 안내를 띄운다(같은 PTY를 두 xterm이 서로 다른 크기로 리사이즈하는 것을 막는다).
  /** 파일 트리에 도트 파일(.claude·.github…)을 낼지. 기본은 감춤 — 매일 여는 디렉터리가
   *  목록 위쪽을 차지해야 한다. */
  const [showHidden, setShowHidden] = useState(false);
  /** 프리뷰 탭 — 트리를 한 번 클릭해서 연 파일이 자리 하나를 돌려 쓴다. 익숙하지 않으면
   *  성가실 수 있어 끌 수 있게 두었다(localStorage에 남는다). */
  const [previewTabs, setPreviewTabs] = useState(previewTabsEnabled);

  /** 활성 **파일 탭**의 실경로. diff 탭이 활성이면 `null` — 이름 변경 복원과 팝아웃이 이것을 본다. */
  const activeFilePath = activeFile != null && activeFile.kind !== "diff" ? activeFile.path : null;

  /** @멘션·Quick Open이 보는 파일 경로 목록. 렌더마다 새로 만들면 QuickOpen이 닫혀 있어도
   *  렌더마다 워크트리 파일 전체를 다시 정렬한다 — 큰 원격 저장소에서 렌더 한 번이 초 단위가
   *  되어 세션을 열자마자 앱이 멈추던 원인(원장 #448). */
  const workspaceFiles = useMemo(() => flattenFiles(tree), [tree]);

  /** 파일 트리의 우클릭 메뉴와 새 파일·폴더. 팝아웃 에디터 창도 같은 훅을 쓴다. */
  const treeOps = useTreeFileOps({
    taskId: selectedId,
    tree,
    refreshTree,
    openFile,
    canMutate: selectedCaps.fileOperations,
    openFiles,
    activePath: activeFilePath,
    closeTabsForPath,
    onError: setErr,
    onRevealPath: (path) => void revealPathInFinder(path),
    onCopyAbsPath: (path) => void copyAbsPath(path),
  });
  /**
   * 세션에 속하지 않는 화면(파일·메모리·일정·GitHub·Quick Open)이 보는 호스트.
   * 세션은 각자 호스트를 갖지만 이 화면들은 그렇지 않다 — 마지막 연결이 조용히
   * 결정하게 두지 않고 사이드바에서 고르게 한다 (ADR 0133 결정 1).
   */
  const [browsingHost, setBrowsingHost] = useState<HostId>(() => {
    try {
      return window.localStorage.getItem("praxis-browsing-host") ?? LOCAL_HOST;
    } catch {
      return LOCAL_HOST;
    }
  });
  const browsingHostRef = useRef(browsingHost);
  browsingHostRef.current = browsingHost;
  /**
   * 저장된 선택이 더는 등록되지 않은 호스트일 수 있다(프로필 삭제·미연결 기동).
   * 그때는 로컬로 되돌아간다 — 화면이 볼 호스트는 **이 값 하나**여야 한다.
   * 사이드바 표시와 진입 게이트가 서로 다른 값을 보면 보이는데 눌리지 않는다.
   */
  const scopedHost = hasHost(browsingHost) ? browsingHost : LOCAL_HOST;
  const pickBrowsingHost = useCallback((host: HostId) => {
    setBrowsingHost(host);
    try {
      window.localStorage.setItem("praxis-browsing-host", host);
    } catch {
      // 저장소가 막힌 환경에서도 이번 세션의 선택은 유지된다.
    }
  }, []);
  // 좌측 내비 접힘 — 풀화면으로 대화에 집중할 때. ⌥⌘B 또는 사이드바 상단 버튼으로 토글.
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  // 인사이트 사이드바 점 — 아직 보지 않은 발견 카드 수(설계 2026-09-28 §4, 슬라이스 T5).
  // 예전 회고 신선도 점(retroUnread, #565에서 회고와 함께 폐기)과 같은 자리이지만, 켜지는
  // 조건이 "새 다이제스트"가 아니라 "아직 안 본 카드가 있다"이다. `insightCards("all", tz)`는
  // 파일 무거운 지출 스캔을 건너뛰어(설계 §5.3 "all"은 S1·S6을 내지 않는다) 앱 시작마다
  // 불러도 싸다 — 이 배지는 사실상 S2·L1만 센다.
  const [insightsUnread, setInsightsUnread] = useState(0);
  const refreshInsightsUnread = useCallback(() => {
    insightCards("all", -new Date().getTimezoneOffset() * 60)
      .then((cards) => setInsightsUnread(countUnseen(cards)))
      .catch(() => setInsightsUnread(0));
  }, []);
  useEffect(() => {
    refreshInsightsUnread();
  }, [refreshInsightsUnread]);
  // 인사이트 화면을 나가면 다시 센다 — 그 사이 InsightsView가 보여준 카드를 "봤다"로
  // 남겼을 것이므로 점이 꺼져야 한다.
  const prevViewForInsightsRef = useRef(view);
  useEffect(() => {
    if (prevViewForInsightsRef.current === "insights" && view !== "insights") {
      refreshInsightsUnread();
    }
    prevViewForInsightsRef.current = view;
  }, [view, refreshInsightsUnread]);
  // 우측 도구 패널 — 열린 도구 탭과 활성 탭을 분리해 데스크톱 앱처럼 전환한다.
  // 작업정보 플로팅 채널의 펼침 여부 — 컨텍스트 게이지가 토글한다(설계 0044).
  // 상태는 위 useSessionPanels가 세션별로 들고 있다.
  /**
   * 중앙(세션 + 코드)에 남은 폭 — `WorkspaceSplit`이 재서 올려 준다.
   *
   * 첫 프레임에는 아직 관측값이 없어 창 폭에서 크롬 몫을 뺀 추정으로 시작한다. 넉넉히 빼는
   * 쪽이 안전하다 — 실제보다 넓게 잡으면 자리도 없는데 채널이 한 프레임 떴다 사라진다.
   */
  const [centerWidth, setCenterWidth] = useState(() =>
    typeof window === "undefined" ? 0 : Math.max(0, window.innerWidth - 448),
  );
  // 컴포저 입력은 세션마다 따로 산다 — 옮겼다 돌아와도 쓰던 문장이 남아 있게.
  const [agentInput, setAgentInput, clearSubmittedDraft] = useSessionDraft(selectedKey);
  const questionReferences = useQuestionReferences(selectedKey);
  const conversationSubmitter = useRef(new ConversationSubmitter());
  const queuedOptimistic = useRef(new Map<string, { item: ConvoItem; event: DebateEventLike }>());
  const { queue: promptQueue, flush: flushPromptQueue } = useConversationQueue({
    submitter: conversationSubmitter.current,
    tasks,
    onSending: (ref, prompt) => {
      if (selectedRef.current == null || taskKey(selectedRef.current) !== taskKey(ref)) return;
      // Use the same optimistic ordering as manual sends; failures remove these exact objects.
      const item: ConvoItem = { role: "user", text: prompt.message };
      const event = { kind: "user", text: prompt.message };
      queuedOptimistic.current.set(prompt.id, { item, event });
      setConvoItems((items) => [...items, item]);
      setConvoEvents((events) => [...events, event]);
      setConvoBusy(true);
      if (ref.host === LOCAL_HOST) {
        setConvoLastEventAt(Date.now());
        setConvoActivity(null);
      }
    },
    onAccepted: (ref, prompt) => {
      queuedOptimistic.current.delete(prompt.id);
      if (selectedRef.current != null && taskKey(selectedRef.current) === taskKey(ref)) {
        if (prompt.uncertain) {
          // An accepted turn may already have finished before its receipt was recovered.
          // Restore durable ordering instead of appending the question after its answer.
          if (ref.host === LOCAL_HOST) void convoHistory(ref.id).then((history) => {
            if (selectedRef.current == null || taskKey(selectedRef.current) !== taskKey(ref)) return;
            const events = history.items as ConvoEventLike[];
            setConvoItems(events.flatMap(eventToItems));
            setConvoEvents(events as DebateEventLike[]);
            setConvoBusy(history.busy);
          }).catch((error) => setErr(String(error)));
        }
      }
      void refresh();
    },
    onFailed: (ref, prompt) => {
      const optimistic = queuedOptimistic.current.get(prompt.id);
      queuedOptimistic.current.delete(prompt.id);
      if (selectedRef.current != null && taskKey(selectedRef.current) === taskKey(ref)) {
        setConvoBusy(false);
        if (optimistic) {
          setConvoItems((items) => items.filter((item) => item !== optimistic.item));
          setConvoEvents((events) => events.filter((event) => event !== optimistic.event));
        }
      }
    },
  });
  const queuedPrompts = promptQueue.snapshot(selectedKey ?? "");
  const shouldQueuePrompt = selected?.mode === "conversation" && (
    convoBusy || ["Running", "Starting", "Queued", "Finalizing"].includes(selected.state) || queuedPrompts.items.length > 0
  );
  const [splitMode, setSplitMode] = useState<SplitMode>("tabs");
  const [questionContext, setQuestionContext] = useState<{ owner: string; id: string; context: SideQuestionContext } | null>(null);
  const sideQuestionApi = useMemo<SideQuestionApi | null>(() => selectedId == null ? null : ({
    read: () => getTransport(selectedHost).sideQuestionRead(selectedId),
    send: (input) => getTransport(selectedHost).sideQuestionSend(selectedId, input),
    cancel: (turnId) => getTransport(selectedHost).sideQuestionCancel(selectedId, turnId),
    reset: (generation) => getTransport(selectedHost).sideQuestionReset(selectedId, generation),
  }), [selectedHost, selectedId]);
  const questionThreads = useQuestionThreads(selectedKey, selected?.mode === "conversation", sideQuestionApi);
  const hasQuestionThread = questionThreads.has;
  const agentInputRef = useRef(agentInput);
  agentInputRef.current = agentInput;
  const [diffStat, setDiffStat] = useState<string | null>(null);
  const [contextDiff, setContextDiff] = useState<WorkContextDiff>({ state: "loading" });
  const [resultTask, setResultTask] = useState<number | null>(null);
  const closeResult = useCallback(() => setResultTask(null), []);
  const [capsule, setCapsule] = useState<Capsule | null>(null);
  const [capsuleBusy, setCapsuleBusy] = useState(false);
  // Quick Open(⌘K/⌘P) 오버레이 — fileScopeOnly=true면 file 스코프로 고정(⌘P 프리셋).
  const [quickOpen, setQuickOpen] = useState<{ open: boolean; scopes?: QuickOpenScope[] }>({
    open: false,
  });
  const [sessionNavigator, setSessionNavigator] = useState(false);
  const sessionNavigatorRef = useRef(sessionNavigator);
  sessionNavigatorRef.current = sessionNavigator;
  // 에이전트 입력 히스토리 (↑/↓ 탐색). pos == 길이면 미탐색(현재 입력).
  const histRef = useRef<string[]>([]);
  const histPosRef = useRef<number>(0);
  // 대화 모드 작업 생성 시 초기 지시를 convoItems에 시딩하기 위한 전달용(선택 전환 리셋보다 뒤에 적용).
  const pendingConvoRef = useRef<{ id: number; text: string } | null>(null);

  // ⌘ 팔레트의 "테마 전환"은 계열을 유지한 채 명암만 뒤집는다 —
  // Mocha에서 누르면 흰 배경이 아니라 Latte로 간다.
  const toggleTheme = () => applyTheme(counterpartOf(theme));

  const refresh = useCallback(async () => {
    // 병렬 조회는 가장 느린 호스트까지 기다린 뒤 한 번에 반영된다. 로컬 스냅샷은 시작 시점의
    // 것이라, 원격 터널이 타임아웃(10초)까지 늘어지면 그 사이 끝난 턴의 검토 대기 전이가
    // 낡은 "실행 중"으로 덮였다. 시작 시각을 기준으로 이후 도착한 전이를 덧씌우고, 먼저 시작한
    // 조회가 더 늦게 끝나면 그 스냅샷은 통째로 버린다.
    const startedAt = performance.now();
    try {
      // 호스트별 독립 조회 — 느린 호스트 하나가 목록 전체를 잡지 않게 한다.
      const merged = await taskListAll();
      if (startedAt < refreshAppliedAtRef.current) return;
      refreshAppliedAtRef.current = startedAt;
      setTasks(applyStateOverrides(merged.tasks, stateOverridesRef.current, startedAt));
      setHostFailures(merged.failures);
      const failedHosts = new Set(merged.failures.map((failure) => failure.host));
      for (const host of listHosts()) {
        if (failedHosts.has(host)) continue;
        reconcileNotifications(host);
      }
    } catch (e) {
      setErr(String(e));
    }
  }, [reconcileNotifications]);

  // 원격 구독은 sequence 0부터 이벤트를 replay하므로, 이벤트당 refresh를 걸면 부팅 순간
  // 같은 taskList 요청이 수십 번 몰린다(측정: 15회 이상). 짧게 모아 마지막 한 번만 보낸다.
  const refreshTimerRef = useRef<number | undefined>(undefined);
  const refreshSoon = useCallback(() => {
    if (refreshTimerRef.current) clearTimeout(refreshTimerRef.current);
    refreshTimerRef.current = window.setTimeout(() => {
      refreshTimerRef.current = undefined;
      void refresh();
    }, 120);
  }, [refresh]);

  /**
   * 사이드바 실패 카드의 "다시 연결". 목록을 다시 훑어 카드 문구를 최신 상태로 돌려놓는다.
   */
  const retryHost = useCallback(async (_host: HostId) => {
    await refresh();
  }, [refresh]);

  // 폰트 설정을 CSS 변수(Tailwind font-ui/font-code 반영)와 컴포넌트 상태(Monaco/xterm prop)에 함께 적용.
  const applyFonts = useCallback((s: FontSettings) => {
    document.documentElement.style.setProperty("--font-ui", uiFontStack(s.ui_family));
    document.documentElement.style.setProperty("--font-code", codeFontStack(s.code_family));
    document.documentElement.style.setProperty("--font-ui-size", `${s.ui_size}px`);
    setFontSettings(s);
  }, []);

  // 에디터 설정 적용. 트리 치수는 CSS 변수로 내리고(글자 하나에서 다섯을 파생), 나머지는
  // state로 EditorPane에 넘어간다 — 미니맵·줄 바꿈·탭은 Monaco 옵션이라 CSS가 닿지 않는다.
  const applyEditorSettings = useCallback((s: EditorSettings) => {
    const next = normalizeEditorSettings(s);
    applyTreeMetrics(document.documentElement, next.tree_font_size);
    setEditorSettings(next);
  }, []);

  useEffect(() => {
    void refresh();
    defaultShell()
      .then((s) => {
        shellRef.current = s;
      })
      .catch(() => {});
    // 상태 전이는 페이로드 {id, state}로 해당 작업만 로컬 패치 — 전이마다 taskList 전체
    // 재조회(생성 직후 Created→Running 연쇄로 재조회 폭주)를 하지 않는다. 모르는 id(외부
    // 기원 생성 등 목록에 없는 작업)만 전체 재조회로 폴백.
    const un = listen<{ id: number; state: string; awaiting_kind?: string | null }>(
      "task://state",
      (e) => {
        const { id, state: next, awaiting_kind: nextKind = null } = e.payload;
        // 진행 중인 재조회보다 이 전이가 새롭다 — 그 결과가 돌아올 때 되돌리지 않도록 남긴다.
        noteStateOverride(stateOverridesRef.current, id, next, nextKind, performance.now());
        const current = tasksRef.current.find((t) => t.host === LOCAL_HOST && t.id === id);
        if (!current) {
          void refresh();
          return;
        }
        // 대기 성격만 바뀌는 전이(검토 대기 ↔ 답변 대기)도 반영해야 하므로 state만 비교하지 않는다.
        if (current.state === next && (current.awaiting_kind ?? null) === nextKind) return;
        setTasks((prev) =>
          prev.map((t) =>
            t.host === LOCAL_HOST && t.id === id
              ? { ...t, state: next, awaiting_kind: nextKind }
              : t,
          ),
        );
      },
    );
    // 호스트마다 구독을 하나씩 연다. **증분으로** 붙이고 뗀다 — 변경마다 전부 끊고 다시
    // 붙이면 재연결 커서가 튀어 이미 받은 이벤트를 다시 받거나 건너뛴다.
    const remoteSubs = new Map<string, { transport: PraxisTransport; stop: () => void }>();
    const bindTransports = () => {
      const hosts = new Set(listHosts().filter((host) => host !== LOCAL_HOST));
      for (const [host, sub] of remoteSubs) {
        // 호스트가 내려갔거나 재연결로 인스턴스가 바뀌었으면 옛 구독을 뗀다.
        if (hosts.has(host) && getTransport(host) === sub.transport) continue;
        sub.stop();
        remoteSubs.delete(host);
      }
      let opened = false;
      for (const host of hosts) {
        if (remoteSubs.has(host)) continue;
        const transport = getTransport(host);
        if (!(transport instanceof RunnerTransport)) continue;
        const stop = transport.subscribeEvents(
          0,
          (event) => {
            if (event.kind !== "output") refreshSoon();
          },
          () => {},
        );
        remoteSubs.set(host, { transport, stop });
        opened = true;
      }
      if (opened) void refresh();
    };
    const unsubscribeTransport = subscribeTransportChange(() => {
      bindTransports();
    });
    bindTransports();
    return () => {
      void un.then((f) => f());
      remoteSubs.forEach((sub) => sub.stop());
      remoteSubs.clear();
      if (refreshTimerRef.current) clearTimeout(refreshTimerRef.current);
      unsubscribeTransport();
    };
  }, [refresh, refreshSoon]);

  // 부팅 시 저장된 폰트 설정 로드 → 적용. 실패(미저장/IPC 오류)는 조용히 무시(기본 체인 유지).
  useEffect(() => {
    fontSettingsGet().then(applyFonts).catch(() => {});
  }, [applyFonts]);

  // 에디터 설정도 같은 자리에서 읽는다. 실패하면 기본값(= 이 설정이 생기기 전의 동작)에
  // 머무르므로 화면이 깨지지 않는다.
  useEffect(() => {
    editorSettingsGet().then(applyEditorSettings).catch(() => {});
  }, [applyEditorSettings]);


  // 설정 화면에서 값을 바꾼 뒤 홈으로 돌아올 때마다 저장된 워크트리 상태를 다시 읽는다.
  // 값은 프로젝트별이므로 레포를 바꿀 때도 다시 읽는다 — 오버라이드가 걸린 레포로 전환하면
  // 칩 표시가 그 레포의 유효값을 따라야 한다.
  useEffect(() => {
    if (view !== "home") return;
    let current = true;
    useWorktreeGet(repo)
      .then((on) => {
        if (current) setUseWorktree(on);
      })
      .catch(() => {});
    return () => {
      current = false;
    };
  }, [view, repo]);

  // tasks에 등장한 모든 레포를 projects에 합집합(마이그레이션/시드) — 단, 명시 제거된 레포는 되살리지 않음.
  useEffect(() => {
    const distinct = new Set(tasks.map((t) => t.repo));
    setProjectsState((prev) => {
      const next = [...prev];
      let changed = false;
      for (const r of distinct) {
        if (!next.includes(r) && !removedProjects.includes(r)) {
          next.push(r);
          changed = true;
        }
      }
      if (!changed) return prev;
      localStorage.setItem("praxis-projects", JSON.stringify(next));
      return next;
    });
  }, [tasks, removedProjects]);

  // 프로젝트 명시 제거 — 목록에서만 제거(작업 이력은 유지), 이후 이력 union으로 되살아나지 않도록 기억.
  const removeProject = (r: string) => {
    // 배정도 함께 지운다 — 다시 등록한 프로젝트가 옛 그룹으로 되살아나면 설명할 수 없다.
    setProjectGroups((prev) => {
      const next = unassignProject(prev, r);
      if (next !== prev) saveProjectGroups(next);
      return next;
    });
    setProjectsState((prev) => {
      if (!prev.includes(r)) return prev;
      const next = prev.filter((p) => p !== r);
      localStorage.setItem("praxis-projects", JSON.stringify(next));
      return next;
    });
    setRemovedProjects((prev) => {
      if (prev.includes(r)) return prev;
      const next = [...prev, r];
      localStorage.setItem("praxis-projects-removed", JSON.stringify(next));
      return next;
    });
  };

  // 새 작업 생성 = 명시적 재등록 의도 — 제거 목록에서 빼고 projects에 다시 추가.
  const registerProject = (r: string) => {
    setRemovedProjects((prev) => {
      if (!prev.includes(r)) return prev;
      const next = prev.filter((p) => p !== r);
      localStorage.setItem("praxis-projects-removed", JSON.stringify(next));
      return next;
    });
    setProjectsState((prev) => {
      if (prev.includes(r)) return prev;
      const next = [...prev, r];
      localStorage.setItem("praxis-projects", JSON.stringify(next));
      return next;
    });
  };

  // Cmd/Ctrl+B: 에디터 사이드의 파일 트리를 토글한다(평소 숨김, 필요 시만 표시).
  //
  // 에디터에 포커스가 있으면 Monaco가 ⌘B를 "정의로 이동"으로 먼저 소비한다(EditorPane).
  // Monaco는 그때 stopPropagation까지 하므로 보통 여기 오지도 않지만, 버전에 따라
  // 전파가 남을 수 있어 defaultPrevented를 한 번 더 본다 — 점프와 트리 토글이 겹치면
  // 파일이 열리면서 패널이 같이 접히는 기묘한 동작이 된다.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if ((e.metaKey || e.ctrlKey) && (e.key === "b" || e.key === "B")) {
        e.preventDefault();
        setShowTree((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Cmd/Ctrl+W: 메인 홈에선 앱 종료(PTY 정리 경유), 그 외 화면에선 홈으로 복귀.
  // macOS 기본 메뉴의 Close Window 가속기를 제거했기에 여기서 Cmd+W를 받는다(ADR 0004).
  //
  // 파일을 보고 있으면 `EditorSplitView`가 캡처 단계에서 먼저 가져가 그 파일만 닫는다.
  // 닫을 파일이 다 떨어졌을 때에야 여기로 내려온다 — "탭을 하나씩 닫다가 화면을 나간다"는
  // 사다리가 되도록, 소비된 이벤트는 반드시 흘려보낸다.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if ((e.metaKey || e.ctrlKey) && (e.key === "w" || e.key === "W")) {
        e.preventDefault();
        if (viewRef.current === "home") {
          void getCurrentWindow().close(); // CloseRequested → PTY 정리 → 종료
        } else {
          escapeToHome();
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);


  // 하단 터미널 도크 토글 — ⌃` (VS Code와 같은 자리). 터미널의 유일한 거처다(설계 0044).
  // 캡처 단계로 받는다: 도크에 포커스가 있으면 xterm이 keydown을 먼저 소비해
  // 같은 키로 닫지 못하게 되기 때문이다.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (viewRef.current !== "workspace") return;
      if (!e.ctrlKey || e.metaKey || e.altKey || e.shiftKey || e.code !== "Backquote") return;
      e.preventDefault();
      setTerminalDock((v) => !v);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  // 좌측 내비 토글 — ⌥⌘B (코드 열 ⌥⌘S와 대칭). 사이드바는 전 뷰 공통이라 뷰 게이트 없음.
  // 에디터 포커스 시에는 "구현으로 이동"이 가져간다 (위 ⌘B와 같은 이유로 가드).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const meta = e.metaKey || e.ctrlKey;
      if (meta && e.altKey && !e.shiftKey && e.code === "KeyB") {
        e.preventDefault();
        setSidebarCollapsed((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // 코드 열 토글 — ⌥⌘S. #75가 우측 패널과 함께 비워 둔 자리를 코드 열이 물려받는다.
  // 워크스페이스에서만 듣는다 — 다른 화면에는 열 자체가 없다.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      if (viewRef.current !== "workspace") return;
      const meta = e.metaKey || e.ctrlKey;
      if (meta && e.altKey && !e.shiftKey && e.code === "KeyS") {
        e.preventDefault();
        toggleCode();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleCode]);

  // Quick Open — ⌘K(전역, 전체 스코프) · ⌘P(파일 스코프 프리셋, worktree 선택 시에만).
  // 뷰/포커스 무관하게 항상 활성 — Composer/Agent 입력 중에도 e.key가 k/p인 것만으로는
  // 기존 컴포저 단축키(Enter 전송, Tab/Arrow 메뉴 내비)와 겹치지 않는다.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const meta = e.metaKey || e.ctrlKey;
      if (!meta || e.shiftKey || e.altKey) return;
      if (e.code === "KeyK") {
        e.preventDefault();
        setSessionNavigator(false);
        setQuickOpen({ open: true, scopes: undefined });
      } else if (e.code === "KeyP") {
        if (selectedRef.current == null) return; // worktree 미선택 — 파일 스코프 비활성
        e.preventDefault();
        setSessionNavigator(false);
        setQuickOpen({ open: true, scopes: ["file"] });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  /**
   * ⌘N — 새 파일, ⇧⌘N — 새 폴더.
   *
   * 만들 자리는 **지금 보고 있는 파일의 폴더**다(없으면 워크트리 루트). 키보드에는 "트리에서
   * 짚은 곳"이 없어서인데, 대신 다이얼로그가 대상 경로를 부제로 보여 주므로 만들기 전에 확인된다.
   * 우클릭 경로는 짚은 노드를 그대로 쓴다 — 손이 가리킨 것이 있으면 그것이 이긴다.
   */
  const newItemRef = useRef({
    dir: "",
    canMutate: false,
    hasTask: false,
    busy: false,
    startNew: treeOps.startNew,
  });
  newItemRef.current = {
    // 키가 아니라 실경로다 — diff 탭이 활성이어도 새 파일은 그 파일의 폴더에 만들어야 한다.
    dir: targetDir(activeFile == null ? null : { path: activeFile.path, is_dir: false }),
    canMutate: selectedCaps.fileOperations,
    hasTask: selectedId != null,
    // 이미 이름을 입력하는 중이면 갈아치우지 않는다 — 치던 것을 잃는다.
    busy: treeOps.promptProps.open,
    startNew: treeOps.startNew,
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.repeat) return;
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.code !== "KeyN") return;
      const now = newItemRef.current;
      if (!now.hasTask || !now.canMutate || now.busy) return;
      e.preventDefault();
      now.startNew(e.shiftKey ? "dir" : "file", now.dir);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Shift 더블탭은 세션의 소속을 한 화면에서 훑는 트리를 연다. ⌘K/⌘P 검색과는 목적이 달라
  // 동시에 겹치지 않게 닫는다.
  useEffect(() => {
    let tap: ShiftTapState = { lastUp: 0 };
    const down = (e: KeyboardEvent) => {
      if (sessionNavigatorRef.current) return;
      tap = shiftDown(tap, e);
    };
    const up = (e: KeyboardEvent) => {
      if (sessionNavigatorRef.current) return;
      const { next, fire } = shiftUp(tap, e, performance.now());
      tap = next;
      if (fire) {
        setQuickOpen((current) => ({ ...current, open: false }));
        setSessionNavigator(true);
      }
    };
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    return () => {
      window.removeEventListener("keydown", down, true);
      window.removeEventListener("keyup", up, true);
    };
  }, []);

  const refreshContextDiff = useCallback(() => {
    if (selectedId == null || !selectedConnected) {
      setContextDiff({ state: "loading" });
      return;
    }
    const ref = selectedCoord;
    if (!ref) return;
    const taskId = ref.id;
    setContextDiff({ state: "loading" });
    void taskDiffStat(ref)
      .then((value) => {
        if (selectedRef.current?.id === taskId) setContextDiff({ state: "ready", value });
      })
      .catch(() => {
        if (selectedRef.current?.id === taskId) setContextDiff({ state: "error" });
      });
  }, [selectedId, selectedConnected]);

  useLayoutEffect(() => {
    // 열린 파일·활성 경로는 useWorkspaceFiles가 taskId 변화에 맞춰 스스로 비운다.
    // 컴포저 입력은 여기서 건드리지 않는다 — useSessionDraft가 세션별로 보관했다 되돌린다.
    setDiffStat(null);
    setCapsule(null);
    setSubTabs([]);
    // 대화 모드 새 작업이면 초기 지시를 유저 버블로 시딩 + busy; 아니면 비운다.
    const seed = pendingConvoRef.current;
    const cached = selectedKey == null ? undefined : convoCacheRef.current.get(selectedKey);
    let restoredFromCache = false;
    if (seed && selectedHost === LOCAL_HOST && seed.id === selectedId) {
      setConvoItems([{ role: "user", text: seed.text }]);
      setConvoEvents([{ kind: "user", text: seed.text }]);
      setConvoBusy(true);
      setConvoLastEventAt(Date.now());
      setConvoActivity(null);
      pendingConvoRef.current = null;
    } else {
      // 재진입은 직전에 본 트랜스크립트를 먼저 그린다 — 조회가 끝날 때까지 빈 화면을 두지 않게.
      setConvoItems(cached ?? []);
      setConvoEvents(selectedKey == null ? [] : convoEventsCacheRef.current.get(selectedKey) ?? []);
      restoredFromCache = cached != null;
      setConvoBusy(false);
      setConvoLastEventAt(null);
      setConvoActivity(null);
    }
    setContextObservation(EMPTY_CONTEXT_OBSERVATION);
    setModelSnapshot(EMPTY_MODEL_SNAPSHOT);
    histRef.current = [];
    histPosRef.current = 0;
    if (selectedConnected) {
      refreshTree();
      refreshContextDiff();
    }
    // 저장된 대화 복원 — 재진입/앱 재시작에도 트랜스크립트·busy 유지 (터미널 작업은 빈 결과).
    if (selectedId == null || !selectedConnected) return;
    // 원격 작업의 트랜스크립트는 Runner output에서 복원한다(아래 원격 대화 effect) —
    // 로컬 DB의 convoHistory는 원격 task id와 무관한 로컬 작업을 돌려줄 수 있다.
    if (getTransport(selectedHost).kind === "remote") return;
    let cancelled = false;
    void convoHistory(selectedId)
      .then((h) => {
        if (cancelled) return;
        const raw = h.items as ConvoEventLike[];
        if (raw.length === 0) {
          // 캐시로 그렸는데 원장이 비어 있으면 그 캐시가 낡은 것(되감기 등) — 지운다.
          // 시딩(새 작업)이면 유지: 첫 턴이 아직 원장에 닿지 않은 정상 상태다.
          if (restoredFromCache && selectedKey != null) {
            setConvoItems([]);
            setConvoEvents([]);
            convoCacheRef.current.delete(selectedKey);
            convoEventsCacheRef.current.delete(selectedKey);
          }
          return;
        }
        setConvoItems(raw.flatMap(eventToItems));
        setConvoEvents(raw as DebateEventLike[]);
        completeNotificationResult(selectedId, notificationLoadRequest, true);
        // 관측은 하나의 이벤트 객체로 순서대로 접는다. 분자와 분모가 다른 턴에서 섞이지 않는다.
        setContextObservation(foldContextObservation(EMPTY_CONTEXT_OBSERVATION, raw, selected?.agent));
        // 마지막 하나만 집으면 `invocation`(requested 전용)으로 끝난 열에서 resolved를 잃는다.
        setModelSnapshot(foldModelSnapshot(EMPTY_MODEL_SNAPSHOT, raw));
        // 마지막 이벤트로 시퀀스가 끝났으면 턴은 종료 — inflight 플래그의 짧은 잔상은 무시.
        const last = raw[raw.length - 1] as DebateEventLike | undefined;
        const ended = last != null && endsSequence(last);
        const active = h.busy && !ended;
        setConvoBusy(active);
        setConvoLastEventAt(active ? Date.now() : null);
      })
      .catch((e) => {
        // 삼키면 빈 트랜스크립트가 "대화가 없다"로 보인다 — 토론은 라운드 번호까지 어긋난다.
        if (!cancelled) setErr(`대화 이력을 불러오지 못했습니다: ${e}`);
        completeNotificationResult(selectedId, notificationLoadRequest, false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId, selectedHost, selectedKey, selectedConnected, notificationLoadRequest, refreshTree, refreshContextDiff]);

  /** 비좌측 자리 조회 — "토론 중"은 컬럼이 아니라 이 행의 존재로 파생한다. 원격은 토론을 돌리지 않는다. */
  const refreshDebateSide = useCallback(() => {
    // 상한은 전역 설정이라 세션을 따라다니지 않는다 — 자리를 읽는 김에 분모도 다시 읽는다.
    void debateRoundCapGet().then(setDebateRoundCap).catch(() => {});
    // 끊긴 호스트에서는 transport가 이미 map에서 빠져 있다 — `getTransport`는 그때 **throw**한다.
    // 이 콜백은 effect라 그 예외가 커밋 단계로 올라가 루트 ErrorBoundary까지 간다: 세션을 클릭한
    // 것뿐인데 창 전체가 폴백으로 바뀐다(원장 #450의 "먹통"). 이웃 effect들(위 복원, 아래 원격
    // 리플레이)은 모두 `selectedConnected`를 먼저 보므로 여기만 어긋나 있었다.
    if (selectedId == null || !selectedConnected || getTransport(selectedHost).kind !== "local") {
      setDebateSeats(null);
      return;
    }
    const taskId = selectedId;
    void debateSides(taskId)
      .then((seats) => {
        if (selectedRef.current?.id === taskId) setDebateSeats(seats.length > 0 ? seats : null);
      })
      .catch(() => setDebateSeats(null));
  }, [selectedId, selectedHost, selectedConnected]);
  useEffect(() => refreshDebateSide(), [refreshDebateSide]);

  useEffect(() => {
    const pending = notificationResultRef.current;
    if (!pending || (pending.host === selectedHost && pending.taskId === selectedId)) return;
    notificationResultRef.current = null;
    pending.resolve(false);
  }, [selectedHost, selectedId]);

  // 트랜스크립트 캐시 적재 — 스트리밍으로 늘어난 분까지 담아 재진입 때 즉시 그린다.
  const convoCacheOwnerRef = useRef<string | null>(null);
  useEffect(() => {
    if (selectedKey !== convoCacheOwnerRef.current) {
      // 선택이 바뀐 렌더의 convoItems는 아직 직전 세션의 것이다 — 이 키에 적재하면 캐시가 없던
      // 세션을 열 때 다른 세션의 트랜스크립트가 먼저 그려진다. 선택 효과가 곧 다시 set하므로 건너뛴다.
      convoCacheOwnerRef.current = selectedKey;
      return;
    }
    if (selectedKey == null || convoItems.length === 0) return;
    const cache = convoCacheRef.current;
    cache.delete(selectedKey); // 재삽입으로 최근 사용 순서를 만든다
    cache.set(selectedKey, convoItems);
    convoEventsCacheRef.current.set(selectedKey, convoEvents);
    // 세션당 수천 아이템이라 무한 보관은 메모리를 먹는다 — 최근 8세션만 남긴다.
    while (cache.size > 8) {
      const oldest = cache.keys().next().value;
      if (oldest == null) break;
      cache.delete(oldest);
      convoEventsCacheRef.current.delete(oldest);
    }
  }, [selectedKey, convoItems, convoEvents]);

  useEffect(()=>{
    const un=listen<{message:string}>("convo-interaction://shutdown-blocked",({payload})=>setErr(payload.message));
    return ()=>{void un.then((f)=>f());};
  },[]);

  // 대화 모드 스트리밍 이벤트 수신 → 트랜스크립트 누적 (선택 작업으로 필터).
  useEffect(() => {
    if (selectedId == null || selectedHost !== LOCAL_HOST) return;
    const un = listen<ConvoEvent>("convo://event", (e) => {
      const ev = e.payload;
      if (ev.id !== selectedId) return;
      if (ev.kind !== "other") {
        setConvoLastEventAt(Date.now());
        setConvoActivity(null);
      }
      setContextObservation((prev) => foldContextObservation(prev, [ev], selected?.agent));
      if (ev.kind === "model_snapshot" || ev.kind === "context_cleared")
        setModelSnapshot((prev) => foldModelSnapshot(prev, [ev]));
      setConvoItems((p) => appendConvoEvent(p,ev));
      setConvoEvents((p) => coalesceConvoEvent(p,ev as DebateEventLike));
      if(ev.kind==="model_snapshot" && ev.source==="app_server") questionRuntime.current=true;
      if (!questionRuntime.current && endsSequence(ev as DebateEventLike)) {
        setConvoBusy(false);
        setConvoLastEventAt(null);
      }
      if (ev.kind === "result") {
        // 턴 동안 에이전트가 파일을 바꿨을 수 있다 — 트리 + (열려 있으면) diff stat 갱신.
        refreshTree();
        refreshContextDiff();
        setDiffStat((prev) => {
          if (prev != null)
            void (selectedCoord ? taskDiffStat(selectedCoord) : Promise.reject())
              .then((s) => setDiffStat((p2) => (p2 != null ? s : p2)))
              .catch(() => {});
          return prev;
        });
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, [selectedId, selectedHost, selected?.agent, refreshTree, refreshContextDiff]);

  useEffect(() => {
    if (selectedId == null || !convoBusy || convoLastEventAt == null) return;
    let cancelled = false;
    let interval: number | undefined;
    const check = () => {
      void convoStatus(selectedId)
        .then((status) => {
          if (!cancelled) setConvoActivity(status);
        })
        .catch(() => {
          if (!cancelled)
            setConvoActivity({ state: "unknown", started_at: 0, last_event_at: 0, last_operation: null, checked_at: Math.floor(Date.now() / 1000) });
        });
    };
    const wait = Math.max(0, 20_000 - (Date.now() - convoLastEventAt));
    const timeout = window.setTimeout(() => {
      check();
      interval = window.setInterval(check, 5_000);
    }, wait);
    return () => {
      cancelled = true;
      window.clearTimeout(timeout);
      if (interval != null) window.clearInterval(interval);
    };
  }, [selectedId, convoBusy, convoLastEventAt]);

  selectedRef.current = selected;
  const selectedMode = selected?.mode ?? null;

  /** 자동 저장을 되돌릴 수 있는 시간. 지나면 지점이 사라진다. */
  const AUTOSAVE_UNDO_GRACE_MS = 8000;
  /** 응답이 끝난 뒤 "완료"를 남겨 두는 시간. */
  const DONE_BADGE_MS = 4000;
  const [autosaveNotice, setAutosaveNotice] = useState<AutosaveNotice | null>(null);
  /** 전송 함수가 아래에서 정의되므로 ref로 잇는다 — 버블은 두 창 모두에서 여기로 들어온다. */
  const askFromSelectionRef = useRef<(payload: EditorAskPayload) => void>(() => {});

  /**
   * 코드 창에 흘려보낼 진행 상태. 본문은 보내지 않는다 — 답은 이 창에서 읽는다.
   *
   * 응답이 끝나면 곧장 대기로 돌아가지 않고 잠시 "완료"를 남긴다. 다른 모니터를 보고 있다가
   * 고개를 돌렸을 때 방금 끝났다는 것을 알 수 있어야, 질문이 갔는지 확인하러 오지 않는다.
   */
  const [editorStatus, setEditorStatus] = useState<EditorStatus>("idle");
  useEffect(() => {
    if (selectedMode !== "conversation") return setEditorStatus("idle");
    if (convoBusy) return setEditorStatus("busy");
    setEditorStatus((prev) => (prev === "busy" ? "done" : prev));
    const timer = window.setTimeout(() => setEditorStatus("idle"), DONE_BADGE_MS);
    return () => window.clearTimeout(timer);
  }, [convoBusy, selectedMode]);

  // 에디터를 두 번째 모니터로 빼낸 동안 이 창의 코드 탭은 사라진다 —
  // 한 세션의 에디터 상태는 한 곳에만 산다. 창이 닫히면 파일 목록과 함께 돌아온다.
  const editorWindow = useEditorWindowHost({
    taskId: selectedId,
    host: selectedHost,
    branch: selected?.branch ?? null,
    worktreePath: selected?.worktree_path ?? null,
    supportsLsp: selectedCaps.lsp,
    // diff 탭은 저쪽 창이 열 수 없다 — 실경로만, 그것도 파일 탭의 것만 보낸다(F-12).
    openPaths: openFiles.filter((f) => f.kind !== "diff").map((f) => f.path),
    activePath: activeFilePath,
    status: editorStatus,
    onPopIn: async (payload) => {
      const scopeCurrent = () =>
        selectedRef.current?.id === payload.task_id && selectedRef.current?.host === payload.host;
      if (!scopeCurrent()) return;
      setCenterTab("editor");
      for (const path of payload.open_paths) {
        if (!scopeCurrent()) return;
        await openFile(path);
      }
      if (!scopeCurrent() || !payload.active_path) return;
      setActiveKey(fileTabKey(payload.active_path));
    },
    onAutosaved: (payload) =>
      setAutosaveNotice({
        taskId: payload.task_id,
        entries: payload.entries,
        deadline: Date.now() + AUTOSAVE_UNDO_GRACE_MS,
      }),
    onAsk: (payload) => askFromSelectionRef.current(payload),
    // 캡처 store는 창마다 따로라 레코드를 배달받아 이 창의 store에 넣는다.
    // 컴포저 포커스는 부르지 않는다: 사용자는 팝아웃에서 코드를 읽는 중이다(설계 0050).
    onCapture: (payload) => {
      if (typeof payload?.task_id !== "number") return; // 창 경계에서 온 값이다 — 주소를 검증한다.
      pushCapture(payload.task_id, payload);
    },
    onNotification: (payload) => popupNotificationRef.current(payload),
    onError: setErr,
  });

  // 파일 탭을 보던 코드 열은 팝아웃과 함께 닫는다. 파일 탭만 빠지면 열이 첫 탭(작업정보)으로
  // 물러나, 떠 있던 채널이 부르지도 않은 옆 열로 옮겨 가고 세션 폭까지 바뀐다. 이 열이 보여 주던
  // 것은 창으로 나갔으니 코드 열을 열기 전의 모양으로 돌아간다. 팝아웃되는 순간과, 나가 있는
  // 동안 다른 세션으로 옮겨 그 세션의 열 상태가 복원되는 순간에만 본다 — 나가 있는 동안 ⌥⌘S로
  // 다시 여는 것까지 막으면 토글이 먹지 않는 것처럼 보인다.
  // 세션은 `selectedKey`가 아니라 패널 값의 주인으로 본다 — 전환 직후 한 렌더는 옛 세션의 값이다.
  const panelKey = panels.stateKey;
  const popOutSeenRef = useRef({ poppedOut: editorWindow.poppedOut, key: panelKey });
  useLayoutEffect(() => {
    const seen = popOutSeenRef.current;
    const arrived = editorWindow.poppedOut && (!seen.poppedOut || seen.key !== panelKey);
    popOutSeenRef.current = { poppedOut: editorWindow.poppedOut, key: panelKey };
    if (arrived && codeOpen && codeTab === "file") showCode(false);
  }, [editorWindow.poppedOut, panelKey, codeOpen, codeTab, showCode]);

  /** 링크 배달 함수만 따로 잡는다. `editorWindow` 객체째 의존성에 넣으면 매 렌더 참조가 바뀌어
   *  `openAgentLink`가 다시 만들어지고, 그것을 받는 Markdown memo가 전부 깨진다. */
  const revealInEditorWindow = editorWindow.revealFile;

  /** 나가 있으면 그 창을 앞으로, 아니면 빼낸다 — 되돌리기는 그 창을 닫는 쪽이 담당한다. */
  const togglePopOut = useCallback(() => {
    if (!editorWindow.poppedOut) return void editorWindow.popOut();
    // focus 실패는 창이 죽었다는 뜻이다. 에러만 띄우고 두면 poppedOut이 true로 굳어 파일 탭이
    // 영영 돌아오지 않는다 — 회수 경로가 editor://closed뿐이다. 다시 열어 상태를 맞춘다.
    void editorWindowFocus().catch(() => editorWindow.popOut());
  }, [editorWindow.poppedOut, editorWindow.popOut]);

  useEditorPopOutShortcut({
    active: () => viewRef.current === "workspace",
    onToggle: togglePopOut,
  });

  // 원격 conversation 작업 — durable Runner output(직렬화 convo 이벤트)에서 트랜스크립트를
  // 복원하고, output 이벤트가 올 때마다 이어서 읽는다. 후속 턴의 user 이벤트는 낙관적
  // 말풍선과 겹칠 수 있어 append 시 중복을 접는다.
  useEffect(() => {
    if (selectedId == null || selectedMode !== "conversation" || !selectedConnected) return;
    // 선택된 작업의 호스트로 읽는다 — 기본 호스트를 읽으면 다른 머신의 출력을 붙이게 된다.
    const transport = getTransport(selectedHost);
    if (!(transport instanceof RunnerTransport)) return;
    let cancelled = false;
    let after = 0;
    /** 선택 효과가 재진입 때 먼저 그린 캐시. 화면에서 이 뒤에 붙은 것만 드레인 중 보낸 낙관적 말풍선이다. */
    const base = (selectedKey == null ? undefined : convoCacheRef.current.get(selectedKey)) ?? [];
    /** 첫 드레인이 끝났는가. 끝나기 전 화면은 캐시(또는 빈 화면)라, 이력을 그 위에 붙이면 안 된다. */
    let replaced = false;
    let drained: ConvoItem[] = [];
    let observation = EMPTY_CONTEXT_OBSERVATION;
    let snapshot = EMPTY_MODEL_SNAPSHOT;
    const agent = selected?.agent;
    /** 이력 전체를 한 번에 **대체**한다. 페이지마다 붙이면 재진입 캐시 위에 이력이 한 벌 더 쌓여
     *  세션을 다시 열 때마다 트랜스크립트가 배로 늘었고, 페이지마다 대체하면 긴 세션이 처음으로
     *  되감겼다가 다시 자란다. */
    const commitReplay = () => {
      replaced = true;
      setContextObservation(observation);
      setModelSnapshot(snapshot);
      const history = drained;
      drained = [];
      setConvoItems((prev) => {
        // 드레인 중 보낸 낙관적 말풍선은 캐시 뒤에 붙어 있다 — 이력 뒤로 옮기고, 이미 durable로
        // 실려 왔으면 append 규칙이 한 번만 남긴다.
        const tail =
          base.length === 0 ? prev : prev.length >= base.length && prev[0] === base[0] ? prev.slice(base.length) : [];
        return appendConvoItems(history, tail);
      });
    };
    let chain = Promise.resolve();
    const load = () => {
      // 직렬화 체인 — output 이벤트 burst에 동시 fetch로 같은 구간을 두 번 append 방지.
      chain = chain.then(async () => {
        try {
          // task별 필터 endpoint라 빈 응답 = 더 없음. REPLAY_LIMIT 초과분은 반복 조회.
          for (;;) {
            if (cancelled) return;
            const rows = await transport.taskOutput(selectedId, after);
            if (cancelled) return;
            if (rows.length === 0) {
              if (!replaced) commitReplay();
              return;
            }
            after = rows[rows.length - 1].sequence;
            const events = rows
              .map((row) => parseConvoEvent(row.data))
              .filter((ev): ev is ConvoEventLike => ev !== null);
            if (events.length === 0) continue;
            const items = events.flatMap(eventToItems);
            if (!replaced) {
              observation = foldContextObservation(observation, events, agent);
              snapshot = foldModelSnapshot(snapshot, events);
              drained = drained.concat(items);
            } else {
              setContextObservation((prev) => foldContextObservation(prev, events, agent));
              // 이어읽기라 직전 배치의 스냅샷 위에 접는다 — 새 배치가 한 필드만 실어도 잃지 않는다.
              setModelSnapshot((prev) => foldModelSnapshot(prev, events));
              if (items.length) setConvoItems((prev) => appendConvoItems(prev, items));
            }
            if (events.some((ev) => ev.kind === "result")) setConvoBusy(false);
          }
        } catch (error) {
          // 타임아웃/중단(AbortSignal.timeout)은 SSH 터널이 반쯤 죽었다는 신호라 구분해 남긴다 —
          // 그 외 오류와 콘솔에서 한눈에 갈라 보이도록 마커를 다르게 붙인다(원장 #448).
          const isStale = error instanceof DOMException && (error.name === "TimeoutError" || error.name === "AbortError");
          if (isStale) {
            console.error("[remote-drain-stale] taskOutput 응답 없음 — SSH 터널이 반쯤 죽었을 수 있다", error);
          } else {
            console.error("[remote-drain-error] taskOutput 실패", error);
          }
          /* 연결 오류 — 다음 output 이벤트/재선택에서 재시도 */
        }
      });
    };
    load();
    const stop = transport.subscribeEvents(
      0,
      (event) => {
        if (event.task_id !== selectedId) return;
        if (event.kind === "output") load();
        if (event.kind === "completed" || event.kind === "failed" || event.kind === "cancelled")
          setConvoBusy(false);
      },
      () => {},
    );
    return () => {
      cancelled = true;
      stop();
    };
  }, [selectedId, selectedMode, selectedConnected, selected?.agent]);
  tasksRef.current = tasks;
  // 최근 레포 = distinct repo + 레포별 마지막 작업 시각(최신순).
  const repoLast = new Map<string, number>();
  for (const t of tasks) {
    if (t.created_at > (repoLast.get(t.repo) ?? 0)) repoLast.set(t.repo, t.created_at);
  }
  const recentRepos = [...repoLast.entries()]
    .sort((a, b) => b[1] - a[1])
    .map(([path, lastUsed]) => ({ path, lastUsed }));

  // 인터뷰 오케스트레이션은 lib/interview.ts의 flow 함수가 소유 — App은 의존성만 연결한다.
  const interviewReqRef = useRef(0);

  const interviewFlowDeps = (): InterviewFlowDeps => ({
    api: { start: interviewStart, crystallize: interviewCrystallize },
    dispatch: dispatchInterview,
  });

  // 인자를 받는 꼴이 따로 있는 이유: 파기 노트를 적용한 직후 채점으로 이어갈 때 setInstruction이
  // 아직 반영되지 않아, 컴포저 상태 대신 방금 적용한 지시문을 직접 넘겨야 한다.
  const startInterviewWith = (instr: string, r: string) => {
    if (!r || !instr) return;
    void runAssessmentFlow(interviewFlowDeps(), {
      repo: r,
      instruction: instr,
      agent: agents[0] ?? "claude",
      requestId: ++interviewReqRef.current,
    });
  };
  const startInterview = () => startInterviewWith(instruction.trim(), repo.trim());

  // 그릴 인터뷰 — 벤더 선택 규칙을 인터뷰와 공유한다.
  const grillReqRef = useRef(0);
  // 진행 중인 호출 키 — StrictMode 이중 마운트와 phase 재진입으로 같은 라운드가
  // 두 번 나가지 않게 막는다. 이 가드가 없으면 개발 모드에서 호출 수가 조용히 2배가 된다.
  const grillInFlightRef = useRef<string | null>(null);
  const [instructionUndo, setInstructionUndo] = useState<string | null>(null);

  // 노트는 상태에만 담고 지시문은 건드리지 않는다 — 적용은 사용자가 버튼을 눌렀을 때만.
  const grillFlowDeps = (): GrillFlowDeps => ({
    api: { round: grillRound, note: grillNote, save: grillSaveNote },
    dispatch: dispatchGrill,
  });

  const startGrill = () => {
    const instr = instruction.trim();
    const r = repo.trim();
    if (!r || !instr) return;
    grillInFlightRef.current = null;
    dispatchGrill({
      type: "start",
      instruction: instr,
      repo: r,
      requestId: ++grillReqRef.current,
    });
  };

  // 답변이 transcript에 들어가면 다음 라운드를 자동 호출한다 — 사용자가 매번 "다음"을 누르지 않게.
  useEffect(() => {
    if (grill.phase !== "asking") return;
    const key = `round:${grill.requestId}:${grill.transcript.length}:${grill.retries}`;
    if (grillInFlightRef.current === key) return;
    grillInFlightRef.current = key;
    void runGrillRound(
      grillFlowDeps(),
      {
        repo: grill.repoSnapshot,
        instruction: grill.instructionSnapshot,
        agent: agents[0] ?? "claude",
        requestId: grill.requestId,
      },
      grill.transcript,
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [grill.phase, grill.transcript.length, grill.retries, grill.requestId]);

  useEffect(() => {
    if (grill.phase !== "noting") return;
    const key = `note:${grill.requestId}:${grill.retries}`;
    if (grillInFlightRef.current === key) return;
    grillInFlightRef.current = key;
    void runGrillNote(
      grillFlowDeps(),
      {
        repo: grill.repoSnapshot,
        instruction: grill.instructionSnapshot,
        agent: agents[0] ?? "claude",
        requestId: grill.requestId,
      },
      grill.transcript,
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [grill.phase, grill.retries, grill.requestId]);

  const applyGrillInstruction = () => {
    if (!grill.note) return;
    setInstructionUndo(instruction);
    setInstruction(grill.note.revised_instruction);
  };

  // 통합 패널의 주 경로 — 개선된 지시문을 넣고 곧바로 2단계(채점)를 시작한다.
  const applyGrillAndScore = () => {
    if (!grill.note) return;
    applyGrillInstruction();
    startInterviewWith(grill.note.revised_instruction.trim(), repo.trim());
  };

  // 패널 ✕·작업 생성 뒤 — 두 단계를 함께 비운다. 되돌릴 원문도 그 지시문과 함께 의미를 잃는다.
  const resetInterviewPanel = () => {
    dispatchInterview({ type: "reset" });
    dispatchGrill({ type: "reset" });
    setInstructionUndo(null);
  };

  const saveGrillNote = async () => {
    if (!grill.note) return;
    // 날짜는 프론트가 넘긴다 — 백엔드가 시스템 시계를 읽으면 결과가 시각에 의존한다.
    const date = new Date().toISOString().slice(0, 10);
    try {
      const path = await grillSaveNote(
        grill.repoSnapshot,
        grill.note.slug,
        grill.note.markdown,
        date,
      );
      dispatchGrill({ type: "saved", path });
    } catch (error) {
      // 저장 실패로 노트를 화면에서 지우지 않는다 — phase는 done에 머문다.
      dispatchGrill({ type: "saveFailed", error: String(error) });
    }
  };

  const crystallizeInterview = () => {
    const answers = Object.entries(interview.answers).map(([question_id, answer]) => ({
      question_id,
      answer,
    }));
    void runCrystallizeFlow(
      interviewFlowDeps(),
      {
        repo: interview.repoSnapshot,
        instruction: interview.instructionSnapshot,
        agent: agents[0] ?? "claude",
        requestId: ++interviewReqRef.current,
      },
      answers,
    );
  };

  const switchComposerHost = (next: HostId) => {
    if (next === composerHost) return;
    composerDraftsRef.current.set(composerHost, {
      repo,
      instruction,
      agents,
      model,
      reasoningEffort,
      serviceTier,
    });
    const draft = composerDraftsRef.current.get(next);
    setComposerHost(next);
    setRepo(draft?.repo ?? "");
    setInstruction(draft?.instruction ?? "");
    setAgentsState(draft?.agents ?? ["claude"]);
    setModel(draft?.model ?? "");
    setReasoningEffort(draft?.reasoningEffort ?? "");
    setServiceTier(draft?.serviceTier ?? "default");
    try {
      window.localStorage.setItem("praxis-composer-host", next);
    } catch {
      // 저장소가 막힌 환경에서도 이번 세션의 선택은 유지된다.
    }
  };

  const create = async () => {
    const list = agents.length ? agents : ["claude"];
    const ref = vaultClientRef;
    const resume = resumeSession;
    setErr(null);
    setResumeConflictTaskId(null);
    // 여기까지가 동기다 — Enter 다음 렌더 한 프레임 안에 컴포저가 잠기고 문구가 뜬다(R-1).
    // 앙상블은 후보마다 단계가 따로 흘러 하나로 표시할 수 없다 — 개수만 싣는다.
    setCreating(
      list.length === 1 ? { ref, stage: null } : { ref: null, stage: null, candidates: list.length },
    );
    setBusy(true);
    try {
      const r = repo.trim();
      const instr = instruction.trim();
      const { cmd, args } = shellRef.current;
      // 선택한 원격이 준비되지 않았다고 로컬 작업을 만들지 않는다. 선택 host의 정체성은
      // 연결 오류를 보여 준 뒤에도 유지돼야 하며, 재시도가 다른 머신에 작업을 만들면 안 된다.
      const host = composerHost;
      if (!hasHost(host)) {
        setErr("선택한 서버에 연결되지 않았습니다. 연결을 완료한 뒤 다시 전송해 주세요.");
        return;
      }
      if (resume) {
        // 결정 11의 두 번째 방어선 — 세션을 고른 뒤 호스트를 바꿨을 수 있다.
        // 서버도 다시 판정하지만, 애초에 다른 호스트로 보내지 않는다.
        if (resume.host !== host) {
          setErr("이어받을 세션을 고른 뒤 서버가 바뀌었습니다. 세션을 다시 선택해 주세요.");
          return;
        }
        const vendor = resume.vendor ?? "claude";
        if (list.length !== 1 || list[0] !== vendor) {
          setErr(`세션 이어받기는 ${vendor} 단일 에이전트에서만 지원합니다.`);
          return;
        }
      }
      const remote = getTransport(host).kind === "remote";
      // 원격도 대화 모드다. Runner는 conversation task를 그대로 실행하고(queue.rs),
      // user/text/tool 이벤트를 task_output에 직렬화해 두므로 데스크톱이 같은 말풍선
      // 트랜스크립트를 복원한다(ADR 0023). terminal로 만들면 그 구조가 있는데도
      // 화면에는 PTY 스크롤백만 남아 내 말과 에이전트 말이 구분되지 않는다.
      const single = !resume && !remote && list.length===1 && QUESTION_AGENTS.includes(list[0]);
      // 터미널은 백엔드의 기존 PTY 경로(`claude <지시문>`)를 탄다. 권한은 CLI 설정을 그대로 따른다.
      const mode = single && sessionStyle === "terminal" ? "terminal" : single && questionsEnabled ? "conversation_questions" : "conversation";
      // 인터뷰 점수는 지시문·레포가 결정화 시점과 같을 때만 기록 — stale 점수를 새 입력에 붙이지 않는다.
      const ambiguity =
        interview.result && !interviewIsStale(interview, instr, r)
          ? interview.result.ambiguity
          : null;
      // 사용자가 고르지 않고 지시문에서 역할을 자동 추론한다.
      const role = inferAgentRole(instr);
      if (list.length === 1) {
        // 단일 인터랙티브 — 작업으로 진입해 출력 탭에서 대화.
        const t = await taskCreate({
          host,
          repo: r,
          instruction: instr,
          agent: list[0],
          role,
          headless: false,
          ensemble: "",
          mode,
          cmd,
          args,
          cols: 100,
          rows: 30,
          model: model.trim(),
          reasoning_effort: reasoningEffort.trim(),
          ...(!remote && list[0] === "codex" && mode !== "terminal" ? { service_tier: serviceTier } : {}),
          ambiguity,
          // 원격은 러너 머신의 체크아웃을 따른다 — base를 고르는 UI도 그때는 뜨지 않는다.
          base_branch: remote ? "" : baseBranch,
          // 원격(Runner)은 무시한다 — tauriTransport만 실어 보낸다(transport.ts).
          client_ref: ref,
          ...(resume ? { resumeSession: resume.session_id, resumeVendor: resume.vendor ?? "claude" } : {}),
          ...(mode === "conversation_questions" && list[0] === "claude" && approvalsEnabled ? { approvals: true } : {}),
        });
        if (!remote && mode !== "terminal") pendingConvoRef.current = { id: t.id, text: instr };
        registerProject(r);
        setInstruction("");
        setResumeSession(null);
        resetInterviewPanel();
        // 낙관적 전환 — 생성된 작업을 로컬 목록에 즉시 반영하고 화면을 전환한 뒤,
        // 전체 재조회는 백그라운드로 수행(전환이 taskList 왕복을 기다리지 않게).
        setTasks((prev) => (prev.some((x) => x.id === t.id) ? prev : [t, ...prev]));
        setSelectedKey(taskKey(t));
        setCenterTab("conversation");
        setView("workspace");
        void refresh();
      } else {
        // 앙상블 — 같은 지시문을 N개 벤더가 각자 대화 모드로 자율수행 → 트랜스크립트·diff 비교/심판.
        // (대화 모드: 구조화 스트림 + 영속 트랜스크립트 → EnsembleView에서 벤더별 응답 비교 가능.)
        const ens = `ens-${Date.now()}`;
        const created: Task[] = [];
        for (const a of list) {
          created.push(await taskCreate({
            host,
            repo: r,
            instruction: instr,
            agent: a,
            role,
            headless: true,
            ensemble: ens,
            mode,
            cmd,
            args,
            cols: 100,
            rows: 30,
            model: "",
            reasoning_effort: "",
            ambiguity,
            // 앙상블 후보는 모두 같은 base에서 갈라져야 비교가 성립한다.
            base_branch: remote ? "" : baseBranch,
          }));
        }
        registerProject(r);
        setInstruction("");
        resetInterviewPanel();
        // 낙관적 전환 — 생성된 앙상블 작업들을 로컬 목록에 즉시 반영, 재조회는 백그라운드.
        setTasks((prev) => [...created.filter((t) => !prev.some((x) => x.id === t.id)), ...prev]);
        setSelectedKey(null);
        setActiveEnsemble(ens);
        setView("ensemble");
        void refresh();
      }
      setVaultDraftVersion(version => version + 1);
    } catch (e) {
      if (e instanceof SessionResumeError) {
        const description = describeSessionResumeError(e);
        setErr(description.message);
        setResumeConflictTaskId(description.taskId ?? null);
      } else {
        setErr(String(e));
      }
    } finally {
      setBusy(false);
      setCreating(null);
    }
  };

  // task://creating은 client_ref가 일치하는 이벤트만 반영한다 — today_start·앙상블처럼
  // client_ref 없이 만든 생성이나 다른 생성의 잔여 이벤트가 이 화면에 새지 않게(설계 0059 §5.1).
  useEffect(() => {
    const un = listen<CreatingEvent>("task://creating", (event) => {
      setCreating((prev) =>
        prev && prev.ref === event.payload.client_ref ? { ...prev, stage: event.payload.stage } : prev,
      );
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const openTask = (task: Task) => {
    setSelectedKey(taskKey(task));
    setView("workspace");
    setCenterTab(task.mode === "conversation" ? "conversation" : "output");
  };

  usePreviewActivation({
    selectedTaskId: view === "workspace" && selectedHost === LOCAL_HOST ? selectedId : null,
    selectTask: (id) => {
      const task = tasks.find((candidate) => candidate.id === id && candidate.host === LOCAL_HOST && !candidate.stale);
      if (!task) return false;
      openTask(task);
      return true;
    },
    showPreview: () => openCodeColumn("preview"),
  });

  const openNotificationResult = async (item: InboxItem, requestId: string = crypto.randomUUID()): Promise<boolean> => {
    const previous = notificationResultRef.current;
    if (previous) {
      notificationResultRef.current = null;
      previous.resolve(false);
    }
    setNotificationRenderReady(null);
    const task = tasks.find((candidate) => candidate.host === item.host && candidate.id === item.task_id);
    const source = notifications.snapshot?.sources.find((candidate) => candidate.host === item.host);
    if (!task || task.stale || !hasHost(item.host) || source?.source_id !== item.source_id) {
      setErr("알림의 작업을 찾을 수 없습니다. 확인은 수동으로 하세요.");
      return false;
    }
    const saved = await flushDirty();
    if (!saved.ok) {
      setErr(`${saved.path} 를 저장하지 못해 결과를 열지 않았습니다.`);
      return false;
    }
    setNotificationLoadRequest(requestId);
    openTask(task);
    if (task.mode !== "conversation" || task.host !== LOCAL_HOST) return false;
    return new Promise<boolean>((resolve) => {
      const timeout = window.setTimeout(() => {
        if (notificationResultRef.current?.requestId !== requestId) return;
        notificationResultRef.current = null;
        resolve(false);
      }, 10_000);
      notificationResultRef.current = {
        host: item.host,
        taskId: item.task_id,
        sequence: item.sequence,
        requestId,
        resolve: (loaded) => {
          window.clearTimeout(timeout);
          resolve(loaded);
        },
      };
    });
  };

  const openNotificationChanges = async (item: InboxItem): Promise<void> => {
    const task = tasks.find((candidate) => candidate.host === item.host && candidate.id === item.task_id);
    const source = notifications.snapshot?.sources.find((candidate) => candidate.host === item.host);
    if (!task || task.stale || !hasHost(item.host) || source?.source_id !== item.source_id) {
      const message = "알림의 작업을 찾을 수 없습니다.";
      setErr(message);
      throw new Error(message);
    }
    const saved = await flushDirty();
    if (!saved.ok) {
      const message = `${saved.path} 를 저장하지 못해 변경을 열지 않았습니다.`;
      setErr(message);
      throw new Error(message);
    }
    openTask(task);
    window.setTimeout(openDiffPanel, 0);
  };
  popupNotificationRef.current = (payload) => {
    void (async () => {
      if (payload.action === "changes") return openNotificationChanges(payload.item);
      if (!(await openNotificationResult(payload.item, payload.request_id))) return;
      notifications.setSnapshot(await notificationAcknowledge(
        payload.item.host,
        payload.item.source_id,
        payload.item.task_id,
        payload.item.sequence,
      ));
    })().catch((reason) => setErr(`알림 이동에 실패했습니다. 확인은 수동으로 하세요: ${String(reason)}`));
  };
  /**
   * 유예가 끝난 뒤에야 실제로 지운다 — 여기서 처음으로 중단·워크트리 정리가 일어난다.
   * 예약 중에는 세션이 그대로 돌고 있으므로 ⌘Z는 아무것도 복원할 필요가 없다.
   */
  const commitRemoval = async (task: Task): Promise<void> => {
    try {
      await removeTask(task, transportRemovalActions(getTransport(task.host), task.id));
    } catch (e) {
      setErr(String(e));
    } finally {
      // 성공이든 실패든 서버 목록으로 되맞춘다. 유예 창은 이미 닫혀 화면 필터가 풀린 뒤이므로,
      // 여기서 갱신하지 않으면 지우지 못한 세션이 낡은 상태 그대로 남는다.
      await refresh();
    }
  };

  // 일부만 정리되는 것이 정상이다 — 진행 중인 턴을 붙들었거나 미완료 approval journal이 있는
  // 작업은 남는다. 남은 것을 알리지 않으면 배너가 왜 그대로인지 알 수 없다.
  const discardOrphans = async (): Promise<void> => {
    try {
      const { retired, failed } = await tasksDiscardOrphans();
      if (failed.length) {
        setErr(
          `${retired.length}건을 정리했습니다. ${failed.length}건은 남았습니다 — `
            + failed.map((f) => `#${f.id}: ${f.reason}`).join(" · "),
        );
      }
    } catch (e) {
      setErr(String(e));
    } finally {
      await refresh();
    }
  };

  const removal = useDeferredTaskRemoval({
    commit: commitRemoval,
    onSchedule: (task) => {
      // 목록에서 감췄으니 열려 있던 화면도 접는다 — 지워진 세션을 계속 보고 있을 수는 없다.
      if (selectedKey === taskKey(task)) {
        setSelectedKey(null);
        setView("home");
      }
    },
    onUndo: (task) => openTask(task),
  });
  // 예약된 세션은 목록에서 빠진다. 실제 정리는 아직이므로 tasks 자체는 건드리지 않는다.
  const visibleTasks = tasks.filter((task) => !removal.isPending(task));
  // 구조화 질문은 상태 전이가 없어 `tasks`의 state로는 드러나지 않는다 — 질문 원장에서 답변을
  // 기다리는 작업을 읽어 사이드바 점·라벨에 겹친다. 답이 반영되어 질문이 닫히면 함께 사라진다.
  // 원장은 로컬 전용이다.
  const questionTasks = useMemo(
    () => new Set(pendingQuestionIds.map((id) => taskKey({ host: LOCAL_HOST, id }))),
    [pendingQuestionIds],
  );

  const goHome = () => setView("home");
  const openQuickLink = (nextView: View): void => {
    setSettingsTab(undefined);
    setView(nextView);
  };

  // ── 음성 입력(Plan 0043) ────────────────────────────────────────────────
  const [voiceHud, setVoiceHud] = useState<VoiceHudState>({
    phase: "idle",
    mode: null,
    message: null,
    isError: false,
  });
  const voiceFlashRef = useRef<number | null>(null);
  const clearVoiceFlash = useCallback(() => {
    if (voiceFlashRef.current != null) {
      window.clearTimeout(voiceFlashRef.current);
      voiceFlashRef.current = null;
    }
  }, []);
  /** 결과·에러를 잠깐 띄웠다 지운다. 다음 녹음이 시작되면 타이머도 함께 갈린다. */
  const flashVoice = useCallback(
    (message: string, isError: boolean) => {
      clearVoiceFlash();
      setVoiceHud({ phase: "idle", mode: null, message, isError });
      voiceFlashRef.current = window.setTimeout(() => {
        voiceFlashRef.current = null;
        setVoiceHud({ phase: "idle", mode: null, message: null, isError: false });
      }, 2500);
    },
    [clearVoiceFlash],
  );

  /**
   * UI 를 움직이는 단일 실행기. 지금은 음성만 부르지만, 3단계에서 에이전트가
   * 같은 액션 타입으로 화면을 조작한다 — 브리지만 붙이면 되도록 여기 하나로 모은다.
   */
  const dispatchUiAction = (action: VoiceAction): void => {
    switch (action.type) {
      case "view":
        openQuickLink(action.view);
        break;
      case "newTask":
        goHome();
        break;
      case "submit":
        // 오인식된 한 마디가 에이전트를 실행시키지 않도록 Composer 와 같은 가드를 건다.
        if (!busy && repo.trim() && instruction.trim()) void create();
        break;
      case "clear":
        setInstruction("");
        break;
    }
  };
  // 핸들러는 매 렌더 새로 만들어지지만 구독은 한 번만 건다 — ref 로 최신 것을 본다.
  const dispatchUiActionRef = useRef(dispatchUiAction);
  dispatchUiActionRef.current = dispatchUiAction;

  useEffect(() => {
    const subscriptions = [
      listen<{ phase: VoiceHudState["phase"]; mode: VoiceHudState["mode"] }>(
        "voice://state",
        (e) => {
          // Idle 복귀는 무시한다 — 방금 띄운 결과 플래시를 즉시 지워버리기 때문.
          if (e.payload.phase === "idle") return;
          clearVoiceFlash();
          setVoiceHud({ phase: e.payload.phase, mode: e.payload.mode, message: null, isError: false });
        },
      ),
      // base 최신화 결과 — 작업은 이미 만들어졌고 이건 부가 정보다. 백엔드가 정상
      // (Skipped·AlreadyCurrent)은 아예 보내지 않으므로 여기 오는 것은 전부 알릴 값이 있다.
      listen<BaseRefreshEvent>("worktree://base-refresh", (e) => {
        const msg = refreshMessage(baseBranchRef.current || "base", e.payload.outcome);
        if (msg) setBaseRefreshNotice(msg);
      }),
      listen<{ mode: "command" | "dictation"; text: string }>("voice://transcript", (e) => {
        const { mode, text } = e.payload;
        if (mode === "dictation") {
          // 자동 전송하지 않는다 — 전송은 "전송" 커맨드나 Enter 로만(설계 0043 §Business Rules).
          setInstruction((prev) => (prev.trim() ? `${prev} ${text}` : text));
          flashVoice(`받아쓰기: ${text}`, false);
          return;
        }
        const action = routeTranscript(text);
        if (!action) {
          flashVoice(`인식 못함: ${text}`, true);
          return;
        }
        dispatchUiActionRef.current(action);
        flashVoice(actionLabel(action), false);
      }),
      listen<{ message: string }>("voice://error", (e) => flashVoice(e.payload.message, true)),
    ];
    return () => {
      subscriptions.forEach((subscription) => void subscription.then((un) => un()));
      clearVoiceFlash();
    };
  }, [clearVoiceFlash, flashVoice]);

  // 서브 에이전트 탭 열기/닫기 — Codex 스타일: 패널·대화 카드 클릭 → 탭 스트립에 추가.
  const openSubagent = useCallback((toolId: string) => {
    setSubTabs((p) => (p.includes(toolId) ? p : [...p, toolId]));
    setCenterTab(`sub:${toolId}`);
  }, []);
  const closeSubagent = useCallback((toolId: string) => {
    setSubTabs((p) => p.filter((t) => t !== toolId));
    setCenterTab((p) => (p === `sub:${toolId}` ? "conversation" : p));
  }, []);
  // 루트 귀속 가능한 서브 이벤트와 최상위 Task 결과는 인라인 토글/전용 탭으로 이동한다.
  // 중첩 스폰의 손자도 루트에 귀속하고, 부모 미상 이벤트는 접근 불가로 사라지지 않게 메인에 남긴다.
  // 매 렌더 다시 만들면 참조가 바뀌어 ConversationView와 그 안 Markdown의 memo가 전부
  // 무효화된다 — 대화 1020건에서 재렌더 300ms 대 53ms 차이다. convoItems가 바뀔 때만 만든다.
  const subThreads = useMemo(() => subagentThreads(convoItems), [convoItems]);
  const subThreadById = useMemo(
    () => new Map(subThreads.map((thread) => [thread.id, thread])),
    [subThreads],
  );
  const mainConvoItems = useMemo(() => {
    const subRoots = subagentRootMap(convoItems);
    return convoItems.filter((it) => subagentThreadRootOf(it, subRoots) == null);
  }, [convoItems]);
  // Cmd+W용 — 어떤 서브 화면에서든 깨끗한 메인 홈으로 복귀.
  const escapeToHome = () => {
    setSelectedKey(null);
    setActiveEnsemble(null);
    setView("home");
  };

  const closeQuickOpen = () => setQuickOpen((q) => ({ ...q, open: false }));

  // Quick Open 항목 선택 라우팅 — task/session은 작업 열기, file은 에디터로, skill은 스킬 화면,
  // command는 정적 레지스트리 액션(quickopen.ts QUICK_OPEN_COMMANDS)을 그대로 실행.
  const handleQuickOpenSelect = (item: RankedQuickOpenItem) => {
    closeQuickOpen();
    if (item.scope === "task" || item.scope === "session") {
      // Quick Open 항목은 id만 들고 온다 — 호스트는 목록에서 되찾는다. 같은 id가 두 호스트에
      // 있으면 목록 순서(최신 우선)를 따르고, 없으면 아무것도 열지 않는다.
      const target = tasks.find((task) => task.id === Number(item.id));
      if (target) openTask(target);
      return;
    }
    if (item.scope === "file") {
      setView("workspace");
      void openFile(item.id);
      return;
    }
    if (item.scope === "code") {
      // 코드 히트는 `경로:줄:열`을 id에 싣는다 — LSP 점프와 같은 착지 경로를 쓴다.
      const at = parseCodeItemId(item.id);
      if (!at) return;
      setView("workspace");
      void openFile(at.path).then((opened) => {
        if (opened) revealAt(at.path, at.line, at.column);
      });
      return;
    }
    if (item.scope === "skill") {
      setSettingsTab("skills");
      setView("settings");
      return;
    }
    switch (item.action) {
      case "new-task":
        goHome();
        break;
      case "toggle-theme":
        toggleTheme();
        break;
      case "toggle-activity":
        setChannelPinned((v) => !v);
        break;
      case "toggle-code":
        toggleCode();
        break;
      case "go-home":
        escapeToHome();
        break;
    }
  };

  /** `~/…` 링크를 펴려면 클라이언트 홈이 필요하다. 값이 바뀌지 않으니 한 번만 읽는다.
   *  state가 아니라 ref인 이유는 아래 openAgentLink가 고정된 콜백이어서다 — 홈이 도착할 때
   *  참조가 바뀌면 대화의 Markdown memo가 그 순간 통째로 깨진다. */
  const homePathRef = useRef<string | null>(null);
  useEffect(() => {
    void homeDir()
      .then((path) => {
        homePathRef.current = path;
      })
      .catch(() => {});
  }, []);

  /** 링크 해석은 클릭과 우클릭 메뉴가 같은 답을 봐야 한다 — 옵션을 한 곳에서만 만든다.
   *  caps를 task에서 다시 계산하는 것은 바깥의 selectedCaps가 고정 콜백에 stale closure로
   *  잡히기 때문이다. 신선한 task로 계산하면 값도 맞고 deps에서도 사라진다. */
  const resolveLink = useCallback(
    (task: Task, value: string): AgentLinkTarget | null =>
      resolveAgentLink(value, task.worktree_path, {
        homePath: homePathRef.current,
        externalPaths: hostCapabilities(task.host).fileOperations,
        remotePaths: task.host !== LOCAL_HOST,
      }),
    [],
  );

  /** 대화의 모든 Markdown 블록이 이 함수를 prop으로 받는다 — 참조가 매 렌더 바뀌면 memo가
   *  전부 깨져 트랜스크립트 전체를 다시 파싱한다. 그래서 고정한다(askLspStatus와 같은 이유). */
  const openAgentLink = useCallback(
    async (value: string) => {
      const task = selectedRef.current;
      if (!task) return;
      const target = resolveLink(task, value);
      if (!target) return setErr(`허용되지 않은 에이전트 링크입니다: ${value}`);
      try {
        if (target.kind === "external-url") return await openUrl(target.url);
        // 에디터가 나가 있으면 이 창에는 파일 탭이 없다(`CodeColumnTabs.tsx:67`) — 링크도
        // 그 창으로 따라가지 않으면 열리기는 해도 어디에도 보이지 않는다.
        if (
          await revealInEditorWindow({
            task_id: task.id,
            host: task.host,
            path: target.path,
            line: target.line ?? null,
            column: target.column ?? null,
          })
        ) {
          return;
        }
        if (selectedRef.current?.id !== task.id || selectedRef.current.host !== task.host) return;
        if (!(await openFile(target.path))) return;
        if (selectedRef.current?.id !== task.id || selectedRef.current.host !== task.host) return;
        // `src/App.tsx:1187` 표기로 온 링크는 그 줄에 착지시킨다 — LSP 이동과 같은 손잡이.
        // 규칙(기본 열·0줄 처리)은 두 창이 공유한다.
        revealAt(target.path, target.line, target.column);
      } catch (e) {
        setErr(String(e));
      }
    },
    [openFile, revealAt, revealInEditorWindow, resolveLink],
  );

  /** 링크 우클릭 메뉴 — 화면 좌표에 뜨므로 대화 트리 밖, 다른 좌표 메뉴들과 같은 자리에서 그린다. */
  const [linkMenu, setLinkMenu] = useState<LinkMenuState | null>(null);
  /** Markdown이 memo라 이 핸들러도 고정해야 한다 — 매 렌더 새 참조면 트랜스크립트가 다시 파싱된다. */
  const onLinkMenu = useCallback(
    (link: string, at: { x: number; y: number }) => setLinkMenu({ x: at.x, y: at.y, link }),
    [],
  );

  /** 항목 구성은 링크가 실제로 무엇을 가리키느냐로 갈린다 — 열려 있는 동안만 해석한다. */
  const linkMenuTarget = useMemo(
    () => (linkMenu && selected ? resolveLink(selected, linkMenu.link) : null),
    [linkMenu, selected, resolveLink],
  );
  const linkMenuKind: LinkMenuKind =
    linkMenuTarget == null ? "unresolved" : linkMenuTarget.kind === "external-url" ? "url" : "file";

  /** 절대경로를 얻는 길이 종류마다 다르다. `os-path`는 이미 절대경로이고, worktree 안 파일은
   *  IPC가 worktree 하위로 제한해 풀어 준다. */
  const linkAbsPath = useCallback(async (task: Task | null, target: AgentLinkTarget | null): Promise<string | null> => {
    if (target?.kind === "os-path") return target.path;
    if (target?.kind !== "task-file") return null;
    return task ? await resolveAbsPath(task.id, target.path) : null;
  }, []);

  /** 메뉴는 ContextMenuShell이 항목을 고른 직후 스스로 닫는다 — 여기서 또 닫지 않는다. */
  const runLinkMenu = useCallback(
    (action: LinkMenuAction) => {
      const link = linkMenu?.link;
      if (link == null) return;
      void (async () => {
        try {
          const task = selectedRef.current;
          const target = task ? resolveLink(task, link) : null;
          // 열기는 클릭과 같은 경로로 보낸다 — 갈라지면 두 손잡이가 서로 다르게 동작한다.
          if (action === "open") return await openAgentLink(link);
          // 복사는 링크 원문 그대로다. 사용자가 본 텍스트와 붙여넣은 것이 달라지지 않게.
          if (action === "copyLink") return await navigator.clipboard.writeText(link);
          if (action === "openExternal") {
            if (task == null || target?.kind === "external-url" || target == null) {
              return setErr(`절대 경로를 알 수 없는 링크입니다: ${link}`);
            }
            return await openLocalFile(taskRef(task), target.path);
          }
          const path = await linkAbsPath(task, target);
          if (path == null) return setErr(`절대 경로를 알 수 없는 링크입니다: ${link}`);
          if (action === "copyAbsPath") return await navigator.clipboard.writeText(path);
          if (action === "reveal") return await revealItemInDir(path);
        } catch (e) {
          setErr(String(e));
        }
      })();
    },
    [linkMenu, linkAbsPath, openAgentLink, resolveLink],
  );

  // 하단 컴포저 → 실행 중인 에이전트(PTY)에 한 줄 전송. 원격은 Runner input API 경유.
  /** 에이전트 모드 전송. 텍스트를 인자로 받는다 — 호출자는 컴포저일 수도, 에디터 창일 수도 있다.
   *  busy 개념이 없다(대화 모드의 convoBusy에 대응하는 것이 없음) — PTY에 그대로 쓴다. */
  const sendAgent = async (line: string): Promise<boolean> => {
    if (selectedId == null || selectedKey == null || !selectedConnected || !line.trim()) return false;
    const draft = agentInput;
    // 히스토리 누적(연속 중복 제외) + 탐색 포인터 리셋.
    const h = histRef.current;
    if (h[h.length - 1] !== line) h.push(line);
    histPosRef.current = h.length;
    try {
      // 로컬은 서버가 검토된 자료를 붙이고 전달 기록을 남긴다. Runner는 기존 입력 경로를 쓴다.
      if (selectedHost === LOCAL_HOST) {
        await vaultLocalComposerSend(selectedId, line, vaultFollowupClientRef, selectedHost);
      } else {
        const data = line.includes("\n") ? `\x1b[200~${line}\x1b[201~\r` : `${line}\r`;
        await getTransport(selectedHost).taskInput(selectedId, data);
      }
      clearSubmittedDraft(selectedKey, draft);
      return true;
    } catch (e) {
      setErr(String(e));
      return false;
    }
  };

  // 인터럽트 — 진행 중인 에이전트 턴 중단 (Ctrl-C = \x03).
  const interruptAgent = () => {
    if (selectedId == null || !selectedConnected) return;
    if (selectedKey != null) promptQueue.pause(selectedKey, "응답을 중단했습니다. 대기 요청은 계속 보내기를 누를 때까지 보존합니다.");
    if (getTransport(selectedHost).kind === "remote") {
      // 원격 terminal은 PTY stdin으로 Ctrl-C 전달(로컬과 동일 의미), 대화 작업은
      // 턴 프로세스 경로가 없어 cancel API로 중단한다.
      if (selected?.mode === "conversation") {
        getTransport(selectedHost)
          .taskCancel(selectedId)
          .then(() => void refresh())
          .catch((e) => setErr(String(e)));
      } else {
        getTransport(selectedHost)
          .taskInput(selectedId, "\x03")
          .catch((e) => setErr(String(e)));
      }
      return;
    }
    // 대화 작업은 PTY가 없다 — 실행 중인 턴의 프로세스 그룹을 죽인다(합성 중단 result가 busy 해제).
    if (selected?.mode === "conversation") {
      convoInterrupt(selectedId).catch((e) => setErr(String(e)));
    } else {
      taskWrite(selectedId, "\x03").catch((e) => setErr(String(e)));
    }
  };

  // 대화 모드 전송 — claude stream-json 한 턴(멀티턴 resume). 사용자 말풍선 즉시 추가.
  // 원격은 Runner message API가 작업을 재큐잉하고 convo_session_id로 resume한다.
  /** 대화 모드 전송. 텍스트를 인자로 받아 컴포저 state를 경유하지 않는다. */
  /** 보냈으면 true. 초안을 지워도 되는지는 호출자가 이 값으로 판단한다. */
  const sendConvo = async (line: string, imagePaths: string[] = []): Promise<boolean> => {
    if (selectedId == null || selectedKey == null || !selectedConnected || !line.trim() || convoBusy) return false;
    if (promptQueue.has(selectedKey)) {
      const pending = conversationSubmitter.current.inspect(selectedKey);
      const resolvingDirect = pending?.message === line && JSON.stringify(pending.images) === JSON.stringify(imagePaths)
        && !promptQueue.snapshot(selectedKey).items.some((item) => item.uncertain || item.sending);
      if (!resolvingDirect) {
        setErr("대기 중인 요청이 있습니다. 대기열을 먼저 보내거나 삭제해주세요.");
        return false;
      }
    }
    const current = () => selectedRef.current?.host === selectedHost && selectedRef.current.id === selectedId;
    const h = histRef.current;
    if (h[h.length - 1] !== line) h.push(line);
    histPosRef.current = h.length;
    const optimisticItem: ConvoItem = { role: "user", text: line };
    const optimisticEvent = { kind: "user", text: line };
    setConvoItems((p) => [...p, optimisticItem]);
    setConvoEvents((p) => [...p, optimisticEvent]);
    setConvoBusy(true);
    const transport = getTransport(selectedHost);
    const remote = transport.kind === "remote";
    if (!remote) {
      // 로컬 turn liveness 워치독(convoStatus)은 로컬 DB 전용 — 원격에서는 켜지 않는다.
      setConvoLastEventAt(Date.now());
      setConvoActivity(null);
    }
    try {
      await conversationSubmitter.current.send(selectedKey, {
        submit: (requestId, message, images) => transport.conversationSubmit(selectedId, requestId, message, images, vaultFollowupClientRef),
        receipt: (requestId) => transport.conversationReceipt(selectedId, requestId),
      }, line, imagePaths, remote ? undefined : vaultFollowupClientRef);
      if (remote) void refresh();
      return true;
    } catch (e) {
      if (current()) {
        setErr(String(e));
        setConvoBusy(false);
        setConvoItems((p) => p.filter((item) => item !== optimisticItem));
        setConvoEvents((p) => p.filter((item) => item !== optimisticEvent));
      }
      return false;
    }
  };

  /**
   * 끝난(Done/Failed/Discarded) 대화에 보내는 메시지는 이 작업이 아니라 **새 작업**으로 간다 —
   * 그 새 작업이 원본의 벤더 세션을 이어받는다(`task_resume`). 원본 행은 건드리지 않는다.
   * taskCreate 성공 경로(1688행 근방)와 같은 관례로 새 작업을 즉시 선택하고 시딩한다 —
   * convoHistory가 아직 첫 턴을 못 봤을 때 pendingConvoRef가 사용자 버블 + busy를 대신 그린다.
   */
  const resumeConvo = async (line: string, imagePaths: string[] = []): Promise<boolean> => {
    if (selectedId == null || selectedKey == null || !selectedConnected || !line.trim() || convoBusy) return false;
    if (getTransport(selectedHost).kind === "remote") {
      setErr("원격 작업은 대화 이어받기를 지원하지 않습니다 — 로컬 세션에서 이어받아 주세요.");
      return false;
    }
    // 이어받기의 첫 턴은 작업 생성 지시문으로 들어가므로 첨부를 실을 자리가 없다. 조용히
    // 버리면 false가 아니라 true가 나가 컴포저가 첨부까지 비운다 — 사용자는 보낸 줄 안다.
    if (imagePaths.length > 0) {
      setErr("첨부는 이어받기 첫 메시지에 실을 수 없습니다 — 먼저 메시지만 보내 대화를 이어받은 뒤 첨부해 주세요.");
      return false;
    }
    const from = { host: selectedHost, id: selectedId };
    const stillHere = () =>
      selectedRef.current?.host === from.host && selectedRef.current.id === from.id;
    setConvoBusy(true);
    try {
      const t = await taskResume(from.host, from.id, line, vaultFollowupClientRef);
      pendingConvoRef.current = { id: t.id, text: line };
      setTasks((prev) => (prev.some((x) => x.id === t.id) ? prev : [t, ...prev]));
      setSelectedKey(taskKey(t));
      setCenterTab("conversation");
      void refresh();
      return true;
    } catch (e) {
      // 기다리는 동안 사용자가 다른 작업으로 옮겼을 수 있다. 그쪽에 오류를 띄우거나 busy를
      // 풀면 남의 진행 중인 턴이 입력 가능한 것처럼 보인다.
      if (stillHere()) {
        setErr(String(e));
        setConvoBusy(false);
      }
      return false;
    }
  };

  /**
   * 대화 한 줄을 보낸다 — **끝난 대화면 이어받기로 간다.** 대화로 나가는 모든 경로가 이것을
   * 거쳐야 한다: 한 곳이라도 `sendConvo`를 직접 부르면 그 경로만 종결 가드에 막혀,
   * 컴포저로는 되는 일이 재시도·선택 질문에서는 안 되는 화면이 된다.
   */
  const deliverConvo = async (message: string, imagePaths: string[] = []): Promise<boolean> =>
    selected?.mode === "conversation" && isTerminalState(selected.state)
      ? await resumeConvo(message, imagePaths)
      : await sendConvo(message, imagePaths);

  const sendMainConvo = async (line: string, imagePaths: string[] = []): Promise<boolean> => {
    if (selectedKey == null || !agentInput.trim()) return false;
    const key = selectedKey;
    const draft = agentInput;
    const submittedReferences = structuredClone(questionReferences.references);
    let referenceText: string;
    try {
      referenceText = formatQuestionReferences(submittedReferences);
    } catch (error) {
      setErr(String(error));
      return false;
    }
    const message = referenceText ? `${line}\n\n${referenceText}` : line;
    if (!selectedConnected) return false;
    const accepted = shouldQueuePrompt && selected != null
      ? promptQueue.enqueue(taskRef(selected), message, imagePaths)
      : await deliverConvo(message, imagePaths);
    if (accepted) {
      clearSubmittedDraft(key, draft);
      questionReferences.consume(key, submittedReferences);
      if (shouldQueuePrompt) void flushPromptQueue();
    }
    return accepted;
  };

  const askSeparately = async (context?: SideQuestionContext) => {
    if (selectedKey == null || selected?.mode !== "conversation") return;
    if (context?.path) {
      try {
        const source = await getTransport(selectedHost).fsRead(selected.id, context.path);
        if (source.kind === "text") context = { ...context, source_hash: contextSourceHash(source.content) };
      } catch {
        // The selected text remains an explicit snapshot even if the source cannot be read.
      }
      if (selectedRef.current?.host !== selectedHost || selectedRef.current.id !== selected.id) return;
    }
    if (context) setQuestionContext({ owner: selectedKey, id: crypto.randomUUID(), context });
    questionThreads.mark(selectedKey);
    openCodeColumn("question");
  };

  const pendingConversation = selectedKey == null ? null : conversationSubmitter.current.inspect(selectedKey);
  const retryPendingConversation = async () => {
    if (!pendingConversation) return;
    if (await deliverConvo(pendingConversation.message, pendingConversation.images)) {
      setErr("이전 요청의 접수를 확인했습니다. 현재 초안과 첨부는 남겨두었으니 다음 전송 전에 확인하세요.");
    }
  };

  /**
   * 선택 버블에서 온 질문을 세션으로 보낸다.
   *
   * 캡처를 컴포저 칩(`pushCapture`)에 올리지 않는다 — 올리는 순간 사용자가 미리 붙여 둔
   * 다른 캡처와 섞인다. 이 질문은 자기 선택만 싣고 곧장 나간다.
   */
  askFromSelectionRef.current = (payload) => {
    const record = buildSelectionCapture({
      taskId: payload.task_id,
      filePath: payload.file_path,
      text: payload.selection_text,
      startLine: payload.start_line,
      endLine: payload.end_line,
    });
    if (!record) return;
    const composed = `${payload.question}\n\n${formatCapturesPrompt([record])}`;
    void (selectedMode === "conversation" ? deliverConvo(composed) : sendAgent(composed));
  };

  // ↑/↓ 로 입력 히스토리 탐색 (shell 스타일).
  const navHistory = (dir: -1 | 1) => {
    const h = histRef.current;
    if (h.length === 0) return;
    const pos = Math.max(0, Math.min(h.length, histPosRef.current + dir));
    histPosRef.current = pos;
    setAgentInput(pos === h.length ? "" : h[pos]);
  };

  const showDiffStat = async () => {
    if (!selectedCoord) return;
    try {
      setDiffStat(await taskDiffStat(selectedCoord));
    } catch (e) {
      setErr(String(e));
    }
  };

  // 입력창 프리필 — 작성 중 텍스트가 있으면 덮어쓰기 확인.
  const prefillInput = (text: string) => {
    if (agentInput.trim() && !window.confirm("입력창의 작성 중 내용을 덮어쓸까요?")) return;
    setAgentInput(text);
  };

  // Capsule 브리핑 (read-only).
  const runCapsule = async () => {
    if (selectedId == null || capsuleBusy) return;
    setErr(null);
    setCapsuleBusy(true);
    try {
      setCapsule(await taskCapsule(selectedId));
    } catch (e) {
      setErr(String(e));
    } finally {
      setCapsuleBusy(false);
    }
  };

  const act = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setErr(null);
    try {
      await fn();
      await refresh();
      // 성공 시에만 선택 해제 + 홈으로. 실패 시 워크스페이스에 머물러 에러 맥락 유지.
      setSelectedKey(null);
      setView("home");
    } catch (e) {
      setErr(String(e));
      // 승인이 충돌로 막힌 것이면 해소 화면을 연다 — 폐기가 유일한 길이 아니다.
      if (conflictPathsFromError(e) && selectedCoord?.host === LOCAL_HOST) setConflictTask(selectedCoord);
    } finally {
      setBusy(false);
      setApprovalRefresh((value) => value + 1);
    }
  };

  /**
   * 사이드바 우클릭 "머지" — 리뷰 바의 승인과 같은 `taskApprove` 경로다. 선택되지 않은 작업에도
   * 쓰이므로 `act`의 선택 좌표 대신 대상 작업의 좌표로 충돌 해소를 열고, 열려 있던 작업일 때만 홈으로 간다.
   */
  /** 승인 본체. 실패는 호출자가 표시하도록 그대로 던진다(충돌이면 해소 창만 연다). */
  const approveTaskOrThrow = async (task: Task) => {
    const ref = taskRef(task);
    setBusy(true);
    setErr(null);
    try {
      await taskApprove(ref);
      await refresh();
      if (selectedKey === taskKey(ref)) {
        setSelectedKey(null);
        setView("home");
      }
    } catch (e) {
      if (conflictPathsFromError(e) && ref.host === LOCAL_HOST) setConflictTask(ref);
      throw e;
    } finally {
      setBusy(false);
      setApprovalRefresh((value) => value + 1);
    }
  };

  const mergeTask = async (task: Task) => {
    const ref = taskRef(task);
    if (busy) return;
    if (!hasHost(ref.host)) {
      setErr("호스트에 연결되지 않아 머지할 수 없습니다.");
      return;
    }
    // 유예 중인 폐기가 뒤따라 터지지 않게 리뷰 바와 같은 잠금을 따른다.
    if (removal.isPending(task)) return;
    try {
      await approveTaskOrThrow(task);
    } catch (e) {
      setErr(String(e));
    }
  };

  /** 파이프라인 화면의 티켓·통합 작업 이동. 로컬 호스트의 작업만 다룬다. */
  const findLocalTask = (id: number) =>
    tasks.find((task) => task.id === id && task.host === LOCAL_HOST && !task.stale);
  const openPipelineTask = (id: number) => {
    const task = findLocalTask(id);
    if (task) openTask(task);
    else setErr("작업을 찾을 수 없습니다. 목록을 새로고침한 뒤 다시 시도하세요.");
  };
  /**
   * G2 승인 — 사이드바 머지와 같은 `taskApprove` 경로라 승인 가드가 그대로 적용된다.
   * 전역 오류 배너로 삼키지 않고 실패를 던져 파이프라인 화면(FinalApproval)이 표시한다.
   */
  const approvePipelineIntegration = async (id: number) => {
    const task = findLocalTask(id);
    if (!task) throw new Error("통합 작업을 찾을 수 없습니다. 목록을 새로고침한 뒤 다시 시도하세요.");
    if (busy) throw new Error("다른 작업이 진행 중입니다. 잠시 후 다시 시도하세요.");
    if (removal.isPending(task)) throw new Error("폐기 유예 중인 작업입니다. 유예가 끝난 뒤 다시 시도하세요.");
    await approveTaskOrThrow(task);
  };

  const openEnsemble = (ens: string) => {
    setActiveEnsemble(ens);
    setSelectedKey(null);
    setView("ensemble");
  };

  /**
   * 폐기 — **10초 유예 창을 거친다.**
   *
   * 폐기는 워크트리를 지우고 커밋하지 않은 변경을 되돌릴 수 없으므로 되무를 자리가 필요한데,
   * 확인 모달과 유예 창은 그 일을 두 번 한다. 더 파괴적인 "삭제"(실행 중 세션 중단까지 포함)조차
   * 모달 없이 유예 창만 쓰므로, 버리기만 모달을 두면 방향이 거꾸로다.
   * 모달이 유일하게 담고 있던 "무엇이 남는가"는 `removalDetail`이 유예 배너에서 말한다.
   *
   * 두 진입점(헤더 메뉴·리뷰 바)이 여기 한 곳을 공유하는 것은 그대로다.
   */
  const discardSelected = () => {
    if (!selected) return;
    removal.schedule(selected);
  };

  // 앙상블 결판 — 추천 후보를 머지하고 나머지 후보(자율수행 잔재)를 정리(Discard).
  const resolveEnsemble = async (winnerId: number) => {
    if (!activeEnsemble) return;
    setErr(null);
    try {
      const cands = await ensembleList(activeEnsemble);
      const losers = cands.filter(
        (c) => c.id !== winnerId && !["Done", "Discarded", "Failed"].includes(c.state),
      );
      const msg = losers.length
        ? `추천 후보를 머지하고 나머지 ${losers.length}개 후보를 버릴까요?`
        : "이 후보를 머지할까요?";
      if (!window.confirm(msg)) return;
      const winner = cands.find((c) => c.id === winnerId);
      if (!winner) return;
      await taskApprove(taskRef(winner));
      const failed: number[] = [];
      for (const l of losers) {
        try {
          await taskDiscard(taskRef(l));
        } catch {
          failed.push(l.id);
        }
      }
      await refresh();
      if (failed.length) {
        // 정리 실패를 숨기지 않는다 — 앙상블 뷰에 머물러 수동 정리 가능하게.
        setErr(`머지 완료. 단, ${failed.length}개 후보를 버리지 못했습니다 — 이 화면에서 직접 정리하세요.`);
      } else {
        setActiveEnsemble(null);
        setView("home");
      }
    } catch (e) {
      setErr(String(e));
    }
  };

  // 완성된 코드 폰트 스택/크기 — 미로드 시 undefined로 두어 EditorPane/TerminalView 자체 기본값(기존 하드코딩 값) 사용.
  const codeFontFamily = fontSettings ? codeFontStack(fontSettings.code_family) : undefined;
  const codeFontSize = fontSettings?.code_size;
  // 유예 중인 작업은 확정 액션을 모두 잠근다 — Quick Open·알림은 예약된 세션도 다시 열 수 있는데,
  // 거기서 `승인`이 나가면 10초 뒤 예약된 폐기가 뒤따라 터져 영문 모를 오류만 남는다.
  const canFinalize = selected?.state === "AwaitingReview" && selectedConnected && !selected?.stale
    && !convoBusy && !removal.isPending(selected);
  const selectedIsDirect = selected ? isDirectRun(selected) : false;

  /**
   * 작업정보 채널의 재료 — 플로팅이든 코드 열 탭이든 같은 것을 받는다.
   * 자리가 둘이어도 내용은 하나여야 어느 쪽이 최신인지 묻지 않게 된다.
   */
  const channelProps =
    selected != null && selectedConnected
      ? {
          task: selected,
          runtime: getTransport(selectedHost).kind,
          diff: contextDiff,
          items: convoItems,
          busy: convoBusy,
          activity: convoActivity,
          onRefreshDiff: refreshContextDiff,
          onOpenSubagent: openSubagent,
          // 채널이 진입점만 두었던 최근 활동 — 목록째 있는 코드 열 탭으로 승격한다.
          onOpenRecentActivity: () => openCodeColumn("activity"),
          tasks,
        }
      : null;

  /** 코드 열 — 2열 레이아웃의 오른쪽. 탭 폴백에서도 같은 것을 쓴다. */
  const codeColumn = selected != null && channelProps != null && (
    <CodeColumnTabs
      active={codeTab}
      onActivate={setCodeTab}
      onClose={() => showCode(false)}
      previewAvailable={advancedFeatures && selectedCaps.designPreview}
      editorPoppedOut={editorWindow.poppedOut}
      activity={<ActivityColumnTab {...channelProps} />}
      question={selected.mode === "conversation" && debateSeats == null && hasQuestionThread && sideQuestionApi && selectedKey != null ? (
        <SideQuestionPanel
          key={selectedKey}
          sessionKey={selectedKey}
          api={sideQuestionApi}
          active={codeOpen && codeTab === "question"}
          initialContext={questionContext?.owner === selectedKey ? questionContext : null}
          files={workspaceFiles}
          readFile={async (path) => {
            const file = await getTransport(selectedHost).fsRead(selected.id, path);
            if (file.kind !== "text") throw new Error("텍스트 파일만 질문에 첨부할 수 있습니다.");
            return file.content;
          }}
          onAttach={(reference) => {
            if (reference.sourceKey !== selectedKey) return;
            questionReferences.attach(reference);
            if (splitMode === "tabs") showCode(false);
            requestAnimationFrame(() => requestComposerFocus(selected.id));
          }}
          onBack={() => {
            showCode(false);
            requestAnimationFrame(() => requestComposerFocus(selected.id));
          }}
        />
      ) : undefined}
      file={
        <EditorSplitView
          taskId={selected.id}
          host={selectedHost}
          rootPath={selected.worktree_path}
          windowId="main"
          sourceChangeRef={workspaceSourceChangeRef}
          files={openFiles}
          activeKey={activeKey}
          treeOpen={treeOpen}
          onTreeOpenHandled={consumeTreeOpen}
          dark={!light}
          // 파일 탭을 실제로 보고 있을 때만 ⌘W·⌘\가 이 에디터의 것이다. 팝아웃 중에는 코드가
          // 저쪽 창에 있으므로(CodeColumnTabs가 파일 탭을 뺀다) 여기서 듣지 않는다.
          shortcutsActive={codeOpen && codeTab === "file" && !editorWindow.poppedOut}
          onSelect={setActiveKey}
          onClose={closeTab}
          // 트리에서 칸 위로 끌어다 놓은 파일 — 아직 열리지 않았으므로 여는 것부터 해야 한다.
          onOpenFile={openFile}
          onChange={changeFile}
          onSave={saveFile}
          onReload={reloadFile}
          onOpenPath={openPathExternal}
          onRevealPath={revealPathInFinder}
          onCopyAbsPath={(path) => void copyAbsPath(path)}
          onPinTab={pinTab}
          supportsExternalPath={selectedCaps.fileOperations}
          codeFontFamily={codeFontFamily}
          codeFontSize={codeFontSize}
          editorSettings={editorSettings}
          onGoto={selectedCaps.lsp ? gotoSymbol : undefined}
          onLspStatus={selectedCaps.lsp ? askLspStatus : undefined}
          onOpenTarget={openLspTarget}
          reveal={revealTarget}
          onRevealed={() => setRevealTarget(null)}
          onNavigationError={setErr}
          askBusy={selectedMode === "conversation" && convoBusy}
          onAskSeparately={selected.mode === "conversation" ? (input) => askSeparately({
            label: `${input.filePath} L${input.startLine}–${input.endLine}`,
            text: input.selectionText,
            path: input.filePath,
          }) : undefined}
          onAsk={(input) => {
            const payload = buildAskPayload({ taskId: selected.id, ...input });
            if (payload) askFromSelectionRef.current(payload);
          }}
        />
      }
      preview={
        <PreviewTab
          taskId={selected.id}
          active={codeOpen && codeTab === "preview"}
          // diff 탭은 Monaco가 아니다 — 캡처할 에디터가 없으므로 활성 **파일** 탭만 센다.
          editorAvailable={activeFilePath !== null}
          workbench={{
            state: previewWorkbench.stateFor(taskKey(selected), selected.id),
            onDraftChange: (draft) => previewWorkbench.setDraft(taskKey(selected), selected.id, draft),
            onSubmit: (message) => void previewWorkbench.submit(taskKey(selected), selected.id, message),
            onCancelPending: () => previewWorkbench.cancelPending(taskKey(selected), selected.id),
            onTakeOver: () => void previewWorkbench.takeOver(taskKey(selected), selected.id).catch((cause) => setErr(String(cause))),
            onRelease: () => void previewWorkbench.release(taskKey(selected), selected.id).catch((cause) => setErr(String(cause))),
            onRefresh: () => void previewWorkbench.refresh(taskKey(selected), selected.id),
          }}
          onPreviewChanged={() => void previewWorkbench.refresh(taskKey(selected), selected.id)}
          key={`code-preview-${selected.id}`}
        />
      }
      diff={<ChangesList />}
    />
  );

  /**
   * 작업정보의 거처. 코드 열이 닫혀 있으면 세션이 오른쪽을 비워 준 자리에 뜨고, 파일·Diff를
   * 부르면 코드 열의 고정 탭으로 들어간다 — 한 번에 한 곳이다(설계 0018 · ADR 0066).
   */
  const sessionMin = debateSeats != null ? debateSessionWidth(debateSeats.length + 1) : MIN_SESSION_WIDTH;
  const placement = channelPlacement({ centerWidth, pinned: channelPinned, codeOpen, sessionMin });

  /**
   * 컨텍스트 게이지 클릭 — 뜻은 언제나 "작업정보를 보여 줘" 하나이고, 손잡이만 자리를 따른다.
   *
   * 보고 있는 것을 다시 누르면 치운다. 좁은 창에서는 뜰 자리가 없으므로 코드 열 탭이 유일한
   * 진입로가 된다 — 게이지가 그리로 데려가지 않으면 그 창에서는 작업정보에 닿을 길이 없다.
   */
  const toggleActivity = () => {
    if (placement === "code") {
      if (codeTab === "activity") showCode(false);
      else setCodeTab("activity");
      return;
    }
    if (placement === "floating") {
      setChannelPinned(false);
      return;
    }
    if (centerWidth < floatingChannelMinCenter(sessionMin)) openCodeColumn("activity");
    else setChannelPinned(true);
  };

  return (
    <HostScopeProvider value={scopedHost}>
    <div className="flex h-screen bg-bg text-text font-ui text-[length:var(--font-ui-size)]">
      <Sidebar
        view={view}
        insightsUnread={insightsUnread}
        onNewTask={goHome}
        onQuickLink={openQuickLink}
        browsingHost={scopedHost}
        onPickBrowsingHost={pickBrowsingHost}
        collapsed={sidebarCollapsed}
        onToggleCollapse={() => setSidebarCollapsed((v) => !v)}
        tasks={visibleTasks}
        selectedKey={selectedKey}
        projects={projects}
        onOpenTask={openTask}
        onNewInRepo={(projectRepo) => {
          setRepo(projectRepo);
          setView("home");
        }}
        onDeleteTask={removal.schedule}
        onMergeTask={(task) => void mergeTask(task)}
        onRemoveProject={removeProject}
        groups={projectGroups}
        onProjectGroupsChange={changeProjectGroups}
        questionTasks={questionTasks}
        hostFailures={hostFailures}
        onRetryHost={retryHost}
        onDiscardOrphans={discardOrphans}
      />

      <main className="flex-1 flex flex-col min-w-0 min-h-0">
        {err && (
          <div className="bg-dangerbg border-b border-dangerborder text-status-failed text-sm px-3 py-1 font-code shrink-0">
            {err}
            {resumeConflictTaskId != null && (
              <button
                className="ml-2 text-primary-bright hover:underline"
                onClick={() => {
                  setSelectedKey(taskKey({ host: composerHost, id: resumeConflictTaskId }));
                  setView("workspace");
                  setCenterTab("conversation");
                  setErr(null);
                  setResumeConflictTaskId(null);
                  void refresh();
                }}
              >
                #{resumeConflictTaskId} 작업으로 이동
              </button>
            )}
            <button
              className="ml-2 text-text-muted hover:text-text"
              onClick={() => {
                setErr(null);
                setResumeConflictTaskId(null);
              }}
            >
              닫기
            </button>
          </div>
        )}
        {view === "workspace" && selectedHost === LOCAL_HOST && selectedId != null && resultTask === selectedId && (
          <TaskResultPanel key={selectedId} taskId={selectedId} onClose={closeResult} onOpenFile={(path) => { void openFile(path); }} onChanges={() => { setResultTask(null); openDiffPanel(); }} />
        )}
        {conflictTask != null && (
          <ConflictResolver
            taskId={conflictTask.id}
            onClose={(resolved) => {
              setConflictTask(null);
              if (resolved) {
                setErr(null);
                setApprovalRefresh((value) => value + 1);
                void refresh();
              }
            }}
          />
        )}

        {view === "insights" ? (
          <InsightsView
            onOpenHome={goHome}
            tasks={visibleTasks}
            onOpenTask={openTask}
            onStartResearch={(instruction, repo) => {
              if (repo) setRepo(repo);
              // 쓰던 초안을 지우지 않는다 — 받아쓰기처럼 뒤에 덧붙이고, 전송은 사용자가 한다.
              setInstruction((prev) => (prev.trim() ? `${prev}\n\n${instruction}` : instruction));
              setView("home");
            }}
          />
        ) : view === "vocab" ? (
          <VocabView />
        ) : view === "pipeline" ? (
          <PipelineView repo={repo} onOpenTask={openPipelineTask} onApproveIntegration={approvePipelineIntegration} />
        ) : view === "settings" ? (
          <SettingsPanel
            key={settingsTab ?? "default"}
            repo={repo}
            initialTab={settingsTab}
            agent={agents[0] ?? "claude"}
            fontSettings={fontSettings}
            onFontSettings={applyFonts}
            editorSettings={editorSettings}
            onEditorSettings={applyEditorSettings}
            onUseWorktreeChange={setUseWorktree}
          />
        ) : view === "ensemble" && activeEnsemble ? (
          <EnsembleView
            ensemble={activeEnsemble}
            onOpenTask={openTask}
            onApprove={resolveEnsemble}
            onHome={goHome}
          />
        ) : view === "workspace" && selected && !selectedConnected ? (
          <div className="flex flex-1 items-center justify-center text-sm text-text-secondary" role="status">
            {selected.host} 연결이 끊겼습니다. 마지막 확인 작업은 유지되며, 다시 연결한 뒤 열 수 있습니다.
          </div>
        ) : view === "workspace" && selected ? (
          // diff 스냅샷·확인함·부분 적용 선택은 세션당 하나다. 변경 목록(트리 열)과 diff 탭(코드
          // 열)이 같은 값을 봐야 하므로 두 열보다 위에 산다(설계 DR-7).
          //
          // Provider가 아니라 Scope다 — 작업이 바뀌면 세션을 통째로 다시 만들어야 앞 작업의
          // 확인함·스냅샷·부분 적용 선택이 넘어오지 않는다(설계 F-5).
          <DiffSessionScope task={taskRef(selected)} openDiff={openSessionDiff}>
          <div className="flex-1 flex min-h-0 relative">
            {/* 작업정보 채널은 이 컨테이너 위에 독립 레이어로 뜬다 — 세션 열은 그만큼 오른쪽을 비워 겹침을 막는다. */}
            <div
              className="flex-1 flex flex-col min-w-0 min-h-0"
            >
              {/* 상단바: 제목 + 상태 + 우측 액션 (이미지 chrome) */}
              <header className="h-11 border-b border-border flex items-center gap-2 px-3 shrink-0">
                <button className="text-text-secondary hover:text-text shrink-0" onClick={goHome} title="홈" aria-label="홈">
                  <Icon name="home" size={16} />
                </button>
                {/* 프로젝트·에이전트 칩은 여기 없다 — 둘 다 이 화면 안 다른 곳이 이미 말한다
                    (프로젝트는 사이드바 그룹 머리와 트리 열 머리, 에이전트는 사이드바 작업 배지와
                    바로 오른쪽 전환 칩). 같은 말을 세 번 하느라 정작 한 줄뿐인 제목이 34%에서
                    잘리고 있었다 — 그 폭을 제목에 돌려준다. */}
                <span className="font-medium truncate max-w-[60%]" title={selected.instruction}>
                  {selected.instruction || selected.branch}
                </span>
                <span className={`text-xs shrink-0 ${stateTextFor(selected)}`}>
                  ● {taskStatusLabel(selected)}
                </span>
                {/* 손잡이 줄은 세션의 것이다 — 채널이 떠 있으면 그 예약선까지만 오고, 창 끝에 붙어
                    채널의 머리처럼 읽히지 않는다.
                    `shrink-0`: 폭이 모자랄 때 물러서는 쪽은 손잡이가 아니라 제목이다. 없으면
                    칩이 눌려 한글 라벨이 글자 단위로 접힌다(제목은 이미 `truncate`라 0까지 준다). */}
                <div
                  className="ml-auto flex shrink-0 items-center gap-1"
                  style={
                    placement === "floating"
                      ? { marginRight: FLOATING_CHANNEL_HEADER_INSET }
                      : undefined
                  }
                >
                  {/* 모델 교체는 이제 원격에서도 된다 — 호출이 `TaskRef`로 transport를 타므로
                      원격 3번이 로컬 3번을 고치던 좌표 혼동이 구조적으로 없다. 이 줄의 손잡이 중
                      에이전트 전환·토론만 로컬 전용이고, 그 판정은 컴포넌트가 능력 표로 한다. */}
                  <SessionModelSwitch
                    inDebate={debateSeats != null}
                    key={selected.id}
                    task={selected}
                    contextTokens={contextTokens}
                    observedModel={modelSnapshot.resolved}
                    // 응답 행은 명령이 돈 시점의 상태를 싣는다 — 그 사이 온 `task://state` 전이를
                    // 되돌리지 않도록 상태 필드는 화면의 것을 지킨다(후속 전이는 이벤트가 다시 패치한다).
                    onServiceTierChanged={(updated) => setTasks((prev) => prev.map((task) =>
                      task.host === selected.host && task.id === selected.id
                        ? { ...updated, host: task.host, state: task.state, awaiting_kind: task.awaiting_kind }
                        : task,
                    ))}
                    onDebateStarted={refreshDebateSide}
                    onChanged={(nextModel, updatedTask) => {
                      setTasks((prev) =>
                        prev.map((t) =>
                          t.host === selected.host && t.id === selected.id
                            ? updatedTask
                              ? { ...updatedTask, host: t.host, state: t.state, awaiting_kind: t.awaiting_kind }
                              : { ...t, model: nextModel, service_tier: t.service_tier != null && (t.model ?? "") !== nextModel ? "default" : t.service_tier }
                            : t,
                        ),
                      );
                      // 관측은 직전 턴의 것이라 전환과 함께 낡는다. 지우지 않으면 게이지가 옛
                      // 모델의 윈도로 잔량을 계산하고, 칩이 바뀐 적 없다는 듯 옛 모델을 계속 건다.
                      if (selectedRef.current?.host === selected.host && selectedRef.current.id === selected.id) {
                        setModelSnapshot(EMPTY_MODEL_SNAPSHOT);
                        setContextObservation(MODEL_CHANGE_INVALIDATION);
                      }
                    }}
                  />
                  {/* 코드 열·터미널의 손잡이 줄 — 목적지마다 버튼을 준다(ADR 0112, 트리 칩은 ADR 0188에서 제거). */}
                  <ChangedFileCount>
                    {(diffCount) => <WorkspacePaneButtons
                    open={codeOpen}
                    active={codeTab}
                    diffCount={diffCount}
                    previewAvailable={advancedFeatures && selectedCaps.designPreview}
                    questionAvailable={selected.mode === "conversation" && debateSeats == null && hasQuestionThread}
                    // "파일" 칩은 코드 열 파일 탭과 함께 닫혀 있던 트리도 연다 — 트리·에디터가 한 손잡이다(ADR 0188).
                    // 닫기는 트리 헤더의 ✕·⌘B. 파일 열기 자체(⌘P·링크)는 트리를 건드리지 않는다.
                    onOpen={(tab) => {
                      openCodeColumn(tab);
                      if (tab === "file" && !showTree) setShowTree(true);
                    }}
                    onClose={() => showCode(false)}
                    popOut={{
                      poppedOut: editorWindow.poppedOut,
                      onToggle: togglePopOut,
                    }}
                    terminal={{
                      available: selectedCaps.workspaceShell,
                      open: terminalDock,
                      reason: selectedCaps.workspaceShell ? undefined : LOCAL_ONLY_REASON,
                      onToggle: () => setTerminalDock((v) => !v),
                    }}
                    showLabels={centerWidth >= PANE_LABELS_MIN_CENTER}
                  />
                    }
                  </ChangedFileCount>
                  {/* 체크포인트·되감기는 저빈도 파괴적 동선이라 툴바 팝오버로 둔다 — 대화 위에 상시 펼치지 않는다. */}
                  {selected.mode === "conversation" && (
                    <CheckpointMenu taskId={selected.id} onRewound={() => void refresh()} />
                  )}
                  <Menu
                    items={[
                      ...(selected.host === LOCAL_HOST ? [{ label: "작업 결과·검토", onClick: () => setResultTask(selected.id) }] : []),
                      { label: "작업 브리핑", onClick: runCapsule },
                      { label: "Diff Stat", onClick: showDiffStat },
                      ...(canFinalize
                        ? [{ label: "Discard", danger: true, onClick: discardSelected }]
                        : []),
                    ]}
                  />
                </div>
              </header>

              {/* 컬럼: 파일 트리(토글) + 중앙 2열(세션 | 코드) — 좁으면 탭 폴백(설계 0044) */}
              <div className="flex-1 flex min-h-0">
                {showTree && (
                  <aside className="w-52 shrink-0 bg-surface border-r border-border flex flex-col min-h-0">
                    <>
                    <div className="h-8 flex items-center justify-between px-3 border-b border-border shrink-0">
                      <span className="text-xs uppercase tracking-wide text-text-muted truncate">
                        {repoBase(selected.repo)}
                      </span>
                      <div className="flex items-center gap-1.5 shrink-0">
                        <button
                          className={previewTabs ? "text-primary-bright" : "text-text-secondary hover:text-text"}
                          onClick={() => {
                            const next = !previewTabs;
                            setPreviewTabsEnabled(next);
                            setPreviewTabs(next);
                            // 끄는 순간 지금 훑어보던 탭은 붙박이가 된다 — 끈 뒤에 열리는
                            // 파일이 그것을 밀어내면 "껐는데도 탭이 사라진다"가 된다.
                            if (!next) pinAll();
                          }}
                          title={
                            previewTabs
                              ? "미리보기 탭 켬 — 한 번 클릭한 파일은 탭 하나를 돌려 씁니다 (더블클릭하면 고정)"
                              : "미리보기 탭 끔 — 클릭할 때마다 탭이 새로 열립니다"
                          }
                          aria-label="미리보기 탭"
                          aria-pressed={previewTabs}
                        >
                          <Icon name="file" size={14} />
                        </button>
                        <button
                          className={showHidden ? "text-primary-bright" : "text-text-secondary hover:text-text"}
                          onClick={() => setShowHidden((v) => !v)}
                          title={showHidden ? "숨김 항목 감추기" : "숨김 항목 보기 (.*)"}
                          aria-label="숨김 항목"
                          aria-pressed={showHidden}
                        >
                          <Icon name={showHidden ? "eye" : "eyeOff"} size={14} />
                        </button>
                        <button
                          className="text-text-secondary hover:text-text"
                          onClick={refreshTree}
                          title="트리 새로고침"
                          aria-label="새로고침"
                        >
                          <Icon name="refresh" size={14} />
                        </button>
                        {/* 소환된 면은 자기 위에 닫기를 갖는다(Do #11). 헤더의 폴더 칩과
                            ⌘B는 보조 경로다 — 트리를 보고 있는 손은 트리 곁에 있다. */}
                        <button
                          className="text-text-secondary hover:text-text"
                          onClick={() => setShowTree(false)}
                          title="파일 트리 닫기 (⌘B)"
                          aria-label="파일 트리 닫기"
                        >
                          <Icon name="x" size={14} />
                        </button>
                      </div>
                    </div>
                    <div className="flex flex-1 flex-col overflow-auto py-1">
                      <FileTree
                        nodes={tree}
                        activePath={activeFile?.path ?? null}
                        onOpen={(path) => void openFile(path, { preview: true, tree: true })}
                        onPin={(path) => pinTab(fileTabKey(path))}
                        onContextMenu={treeOps.openMenu}
                        showHidden={showHidden}
                      />
                    </div>
                    </>
                  </aside>
                )}

                <WorkspaceSplit
                  codeOpen={codeOpen}
                  onCloseCode={() => showCode(false)}
                  onAvailableWidth={setCenterWidth}
                  onModeChange={setSplitMode}
                  session={
                    <SessionDiffSurface
                      path={centralDiffPath}
                      onBack={() => {
                        setCentralDiffPath(null);
                        requestAnimationFrame(() => requestComposerFocus(selected.id));
                      }}
                      onOpenPath={openSessionDiff}
                    >
                    <div
                      className="relative flex min-h-0 flex-1 flex-col"
                      style={
                        placement === "floating"
                          ? { paddingRight: FLOATING_CHANNEL_RESERVED }
                          : undefined
                      }
                    >
                      {/* 서브 에이전트 탭은 세션 열 안에 산다 — 헤더가 아니라 그 대화가 속한 자리다. */}
                      {selected.mode === "conversation" && subTabs.length > 0 && (
                        <div className="flex shrink-0 items-center gap-1 border-b border-border px-2 py-1">
                          <button
                            className={tabBtn(!centerTab.startsWith("sub:"))}
                            onClick={() => setCenterTab("conversation")}
                          >
                            대화
                          </button>
                          {subTabs.map((tid) => {
                            const title = subThreadById.get(tid)?.title ?? "서브 에이전트";
                            return (
                              <span key={tid} className="flex items-center">
                                <button
                                  className={`${tabBtn(centerTab === `sub:${tid}`)} max-w-[140px] truncate`}
                                  onClick={() => setCenterTab(`sub:${tid}`)}
                                  title={title}
                                >
                                  ⑂ {title}
                                </button>
                                <button
                                  className="px-0.5 text-text-muted hover:text-text"
                                  onClick={() => closeSubagent(tid)}
                                  title="서브 에이전트 탭 닫기"
                                  aria-label="서브 에이전트 탭 닫기"
                                >
                                  <Icon name="x" size={11} />
                                </button>
                              </span>
                            );
                          })}
                        </div>
                      )}
                      {centerTab.startsWith("sub:") ? (
                        <SubagentView
                          entry={subThreadById.get(centerTab.slice(4)) ?? null}
                          items={subThreadById.get(centerTab.slice(4))?.items ?? []}
                          parentBusy={convoBusy}
                          onOpenLink={openAgentLink}
                          onLinkMenu={onLinkMenu}
                          hidden={centralDiffPath != null}
                          conversationId={`${selectedKey ?? selected.id}:${centerTab.slice(4)}`}
                          key={centerTab}
                        />
                      ) : selected.mode === "conversation" && debateSeats != null ? (
                        /* 세션 열만 갈린다 — 헤더·코드 열·플로팅 채널의 기존 배치는 그대로다. */
                        <QuestionSession key={selectedKey} taskId={selectedHost === LOCAL_HOST ? selected.id : null} linkedIds={convoItems.flatMap((item)=>item.role==="interaction"?[item.interactionId]:[])} onBusyChange={updateQuestionBusy}>
                          {(renderQuestion,interactionStatus)=><DebateView
                            taskId={selected.id}
                            events={convoEvents}
                            roundCap={debateRoundCap}
                            panes={[{ agent: selected.agent ?? "", model: selected.model ?? null }, ...debateSeats]}
                            busy={convoBusy}
                            onSend={sendConvo}
                            onEnded={refreshDebateSide}
                            onOpenLink={openAgentLink}
                            renderQuestion={renderQuestion}
                            interactionStatus={interactionStatus}
                          />}
                        </QuestionSession>
                      ) : selected.mode === "conversation" ? (
                        <QuestionSession key={selectedKey} taskId={selectedHost === LOCAL_HOST ? selected.id : null} linkedIds={mainConvoItems.flatMap((item)=>item.role==="interaction"?[item.interactionId]:[])} onBusyChange={updateQuestionBusy}>
                          {(renderQuestion,interactionStatus)=><ConversationView
                          renderQuestion={renderQuestion}
                          interactionStatus={interactionStatus}
                          conversationId={selectedKey ?? selected.id}
                          onAskSeparately={(text) => askSeparately({ label: "선택한 대화", text })}
                          translatable
                          items={mainConvoItems}
                          busy={convoBusy}
                          subagents={subThreadById}
                          activity={convoActivity}
                          onOpenSubagent={openSubagent}
                          onOpenLink={openAgentLink}
                          onLinkMenu={onLinkMenu}
                          // 좁은 창에서는 코드 열이 세션 열을 display:none으로 가린다. 그동안 스크롤 위치가
                          // 사라지므로, 가려지는 것을 알려야 돌아올 때 보던 자리로 되돌린다.
                          hidden={centralDiffPath != null || (codeOpen && splitMode === "tabs")}
                          waitTaskId={selectedHost === LOCAL_HOST ? selected.id : null}
                        />}
                        </QuestionSession>
                      ) : (
                        <TerminalView
                          host={selectedHost}
                          taskId={selected.id}
                          // 단일 에이전트는 실제 CLI라 키 입력을 그대로 받는다. 앙상블은 `-p` 출력뿐이다.
                          readOnly={!!selected.ensemble}
                          onOpenLink={openAgentLink}
                          codeFontFamily={codeFontFamily}
                          codeFontSize={codeFontSize}
                          key={`out-${selected.id}`}
                        />
                      )}

                      {/* 작업정보 채널 — 세션 열 안에서만 뜬다. 예약한 자리에 놓이므로 대화를 덮지 않고,
                          코드 열이 열리면 그쪽 고정 탭으로 옮겨 간다. */}
                      {placement === "floating" && channelProps != null && (
                        <ActivityRail {...channelProps} onCollapse={() => setChannelPinned(false)} />
                      )}

                      {/* 재열기 핸들 — 채널이 살던 자리에 남는 문(FloatingActivityChannel.handle).
                          핀을 켜면 실제로 뜰 수 있는 조건에서만 보인다. */}
                      {channelProps != null &&
                        channelHandleVisible({ centerWidth, pinned: channelPinned, codeOpen, sessionMin }) && (
                          <button
                            className="absolute top-4 right-4 z-30 rounded-md border border-border-strong bg-raised p-1.5 text-text-secondary shadow-[0_4px_12px_rgba(0,0,0,0.5)] hover:text-text"
                            onClick={() => setChannelPinned(true)}
                            title="작업정보 열기"
                            aria-label="작업정보 열기"
                          >
                            <Icon name="panelRight" size={16} />
                          </button>
                        )}
                    </div>
                    </SessionDiffSurface>
                  }
                  code={codeColumn}
                />
              </div>

              {/* 리뷰 바 — 대화가 워크트리를 변경해 검토 대기일 때 인라인으로 승인/폐기 동선 (대화→검토→머지 루프).
                  이 바는 대화 열의 높이를 깎는 자리다 — 준비 점검·자동 해결의 상세를 여기에 인라인으로
                  펼치면 그만큼 대화가 좁아지고, 높이가 작업 상태에 따라 들쭉날쭉해진다. 둘은 칩 한 줄로
                  두고 상세는 위로 뜨는 팝오버가 맡는다(DetailChip) — 대화를 덮되 밀지 않으므로 이 바는
                  창이 아주 좁아 줄바꿈이 나기 전까지 항상 한 줄이다. */}
              {selected.mode === "conversation" && selected.state === "AwaitingReview" && (
                <div className="border-t border-border bg-raised px-3 py-2 shrink-0 flex flex-wrap items-center gap-2 text-sm">
                  <span className={stateTextFor(selected)}>●</span>
                  <span className="min-w-0 flex-1 truncate text-text-secondary" title={reviewBarMessage(taskTone(selected), selectedIsDirect)}>
                    {reviewBarMessage(taskTone(selected), selectedIsDirect)}
                  </span>
                  <div className="ml-auto flex min-w-0 max-w-full flex-wrap items-center justify-end gap-1">
                    {!selectedIsDirect && (
                      <ApprovalReadinessPanel
                        key={taskKey(taskRef(selected))}
                        task={taskRef(selected)}
                        base={selected.base}
                        refreshKey={`${selected.updated_at}:${approvalRefresh}`}
                        disabled={busy || !canFinalize || convoBusy}
                        onRepair={deliverConvo}
                        onResolve={selectedHost === LOCAL_HOST ? () => setConflictTask(taskRef(selected)) : undefined}
                      />
                    )}
                    {!selectedIsDirect && (
                      <ApprovalRepairPanel key={`repair:${taskKey(taskRef(selected))}`} task={taskRef(selected)} disabled={busy || !canFinalize || convoBusy} onAccepted={() => { setApprovalRefresh((value) => value + 1); void refresh(); }} />
                    )}
                    {!selectedIsDirect && (
                      <span className="min-w-0 max-w-[180px] truncate font-code text-xs text-text-muted" title={selected.base}>
                        머지 대상: {selected.base}
                      </span>
                    )}
                    {/* 멀티벤더 리뷰는 채널이 아니라 이 작업에 붙는다 — 이력도 여기 남는다. */}
                    <TaskVendorReview repo={selected.repo} taskId={selected.id} host={selected.host} />
                    <button
                      className="h-7 px-2.5 rounded text-text-secondary hover:text-text"
                      onClick={openDiffPanel}
                    >
                      Diff 보기
                    </button>
                    <button
                      className="h-7 px-2.5 rounded text-status-failed hover:opacity-80 disabled:opacity-40"
                      disabled={busy || !canFinalize}
                      onClick={discardSelected}
                    >
                      버리기
                    </button>
                    <button
                      className="flex h-7 min-w-0 max-w-[260px] items-center rounded px-3 font-medium text-status-done hover:opacity-80 disabled:opacity-40"
                      disabled={busy || !canFinalize}
                      onClick={() => act(() => taskApprove(taskRef(selected)))}
                    >
                      {selectedIsDirect ? (
                        "승인"
                      ) : (
                        <>
                          <span className="min-w-0 truncate" title={selected.base}>{selected.base}</span>
                          <span className="shrink-0">에 승인하고 머지</span>
                        </>
                      )}
                    </button>
                  </div>
                </div>
              )}

              {/* 종결 대화 안내 — 이 상태에서 보내는 메시지는 이 작업이 아니라 새 작업으로 간다
                  (task_resume). 리뷰 바와 자리가 겹치지 않는다 — 저쪽은 AwaitingReview 전용. */}
              {selected.mode === "conversation" && isTerminalState(selected.state) && (
                <div className="border-t border-border bg-raised px-3 py-2 shrink-0 flex items-center gap-2 text-sm">
                  <span className={stateTextFor(selected)}>●</span>
                  <span className="min-w-0 flex-1 text-text-secondary">
                    {getTransport(selectedHost).kind === "remote"
                      ? "끝난 대화입니다 — 원격 세션은 대화 이어받기를 지원하지 않습니다."
                      : "끝난 대화입니다 — 메시지를 보내면 이 세션을 이어받아 새 작업으로 계속합니다."}
                  </span>
                </div>
              )}

              {/* 하단 에이전트 질의 컴포저 (멀티라인 + @파일 멘션 + 인터럽트).
                  토론 중에는 분할 뷰가 자기 컴포저를 갖는다 — 둘을 함께 두지 않는다.
                  터미널 세션도 두지 않는다 — xterm이 곧 입력창이라 CLI 입력창과 둘로 보인다.
                  원격·앙상블 PTY는 xterm이 입력을 받지 않으므로 컴포저가 유일한 입력 경로로 남는다. */}
              {debateSeats == null &&
                !(selected.mode === "terminal" && !selected.ensemble && getTransport(selectedHost).kind === "local") && (
                <footer
                  className="border-t border-border px-3 py-2 shrink-0"
                  hidden={centralDiffPath != null || (codeOpen && codeTab === "question" && splitMode === "tabs")}
                  inert={centralDiffPath != null || (codeOpen && codeTab === "question" && splitMode === "tabs")}
                >
                  <AgentComposer
                    host={selectedHost}
                    value={agentInput}
                    onChange={(v) => {
                      setAgentInput(v);
                      histPosRef.current = histRef.current.length; // 직접 입력 시 탐색 리셋
                    }}
                    onSend={selected.mode === "conversation" ? sendMainConvo : sendAgent}
                    label={selected.mode === "conversation" ? "메인에 요청" : undefined}
                    sendLabel={selected.mode === "conversation" ? (shouldQueuePrompt ? "대기열에 추가" : "메인에 보내기") : undefined}
                    disabled={
                      selected.mode === "conversation" &&
                      (!selectedConnected ||
                        // 원격 종결 대화는 이어받기가 없다 — 눌러 보고서야 막히지 않게 미리 잠근다.
                        (isTerminalState(selected.state) && getTransport(selectedHost).kind === "remote"))
                    }
                    attachments={<>
                      <ConversationQueue queue={queuedPrompts} connected={selectedConnected}
                        onRemove={(id) => { if (selectedKey != null) promptQueue.remove(selectedKey, id); }}
                        onPause={() => { if (selectedKey != null) promptQueue.pause(selectedKey); }}
                        onResume={() => { if (selectedKey != null) { promptQueue.resume(selectedKey); void flushPromptQueue(); } }} />
                      {pendingConversation && !convoBusy && !queuedPrompts.items.some((item) => item.uncertain) && <details className="mb-2 rounded border border-border p-2 text-xs text-text-secondary">
                        <summary>접수 확인이 필요한 이전 요청</summary>
                        <p className="my-1">아래 원본을 같은 요청 ID로 확인·재시도합니다. 작성 중인 초안은 유지합니다.</p>
                        <pre className="max-h-32 overflow-auto whitespace-pre-wrap break-words">{pendingConversation.message}</pre>
                        {pendingConversation.images.length > 0 && <p>이미지 {pendingConversation.images.length}개 포함</p>}
                        <button type="button" className="mt-1 rounded border border-border px-2 py-1 hover:border-primary disabled:opacity-50" disabled={!selectedConnected} onClick={() => void retryPendingConversation()}>이전 요청 확인·재시도</button>
                      </details>}
                      <QuestionReferenceAttachments key={selectedKey} references={questionReferences.references} onChange={questionReferences.setReferences} />
                    </>}
                    onInterrupt={interruptAgent}
                    onHistory={navHistory}
                    files={workspaceFiles}
                    repo={selected.repo}
                    taskId={selected.id}
                    draftKey={selectedKey}
                    active={centralDiffPath == null && !(codeOpen && codeTab === "question" && splitMode === "tabs")}
                    gauge={
                      <>
                        {selected.mode === "conversation" && (
                          <ContextGauge
                            task={selected}
                            observation={contextObservation.observation}
                            model={modelSnapshot.resolved ?? modelSnapshot.requested ?? selected.model}
                            busy={convoBusy}
                            onOpenDetails={toggleActivity}
                          />
                        )}
                        {/* 대화 세션에만 의미가 있고(끊을 벤더 세션이 있어야 한다), 캡슐 주입은
                            로컬 worktree에 파일을 쓰므로 원격 작업에는 걸 수 없다. */}
                        {selected.mode === "conversation" &&
                          getTransport(selected.host).kind === "local" && (
                            <ContextResetButton
                              task={selected}
                              onReset={() => {
                                // 마지막 관측은 끊어낸 세션의 것이다. 지우지 않으면 바로 옆 게이지가
                                // 비우기 전 포화도를 그대로 보여줘 "눌렀는데 그대로"로 읽힌다.
                                // 구분선 자체는 백엔드가 convo://event로 흘려보낸다.
                                setContextObservation(EMPTY_CONTEXT_OBSERVATION);
                                void refresh();
                              }}
                            />
                          )}
                      </>
                    }
                  />
                </footer>
              )}
            </div>

            {/* Capsule 브리핑 플로팅 패널 */}
            {capsule !== null && (
              <CapsulePanel
                capsule={capsule}
                onForward={(na) => {
                  prefillInput(na);
                  setCapsule(null);
                }}
                onInject={async () => {
                  if (selectedId == null) return;
                  try {
                    const files = await capsuleInject(selectedId);
                    setErr(null);
                    window.alert(`브리핑 저장 완료: ${files.join(", ")}`);
                    setCapsule(null);
                  } catch (e) {
                    setErr(String(e));
                  }
                }}
                onClose={() => setCapsule(null)}
              />
            )}


            {/* Diff Stat 플로팅 패널 (⋮ 메뉴) */}
            {diffStat !== null && (
              <div className="absolute right-4 bottom-20 z-30 w-96 max-h-64 overflow-auto rounded-lg border border-border-strong bg-raised shadow-xl p-3">
                <div className="flex items-center justify-between mb-1">
                  <span className="text-xs uppercase tracking-wide text-text-muted">Diff Stat</span>
                  <button
                    className="text-text-muted hover:text-text"
                    onClick={() => setDiffStat(null)}
                    aria-label="닫기"
                  >
                    <Icon name="x" size={14} />
                  </button>
                </div>
                <pre className="text-xs font-code text-text-secondary whitespace-pre-wrap">
                  {diffStat.trim() || "(변경 없음)"}
                </pre>
              </div>
            )}
          </div>
          </DiffSessionScope>
        ) : (
          // 홈: 대시보드 + 하단 컴포저
          <>
            <HomeView tasks={visibleTasks} onOpenTask={openTask} onOpenEnsemble={openEnsemble} onRefresh={refresh} onOpenProject={async (root) => (await projectEditorOpen(root)).root} />
            <Composer
              host={composerHost}
              setHost={switchComposerHost}
              repo={repo}
              setRepo={setRepo}
              recentRepos={recentRepos}
              agents={agents}
              setAgents={setAgents}
              resumeSession={resumeSession}
              onResumeSessionChange={setResumeSession}
              projects={projects}
              serviceTier={serviceTier}
              onServiceTierChange={setServiceTier}
              questionsEnabled={questionsEnabled}
              onQuestionsEnabledChange={setQuestionsEnabled}
              approvalsEnabled={approvalsEnabled}
              onApprovalsEnabledChange={setApprovalsEnabled}
              sessionStyle={sessionStyle}
              model={model}
              setModel={setSessionModel}
              reasoningEffort={reasoningEffort}
              setReasoningEffort={setReasoningEffort}
              instruction={instruction}
              setInstruction={setInstruction}
              interview={interview}
              onInterviewStart={startInterview}
              onInterviewAnswer={(questionId, answer) =>
                dispatchInterview({ type: "answer", questionId, answer })
              }
              onInterviewCrystallize={crystallizeInterview}
              onInterviewRetry={() => dispatchInterview({ type: "retry" })}
              onInterviewClose={resetInterviewPanel}
              grill={grill}
              onGrillStart={startGrill}
              onGrillDraft={(value) => dispatchGrill({ type: "draft", value })}
              onGrillAnswer={() => dispatchGrill({ type: "answer" })}
              onGrillAcceptRecommendation={() => dispatchGrill({ type: "acceptRecommendation" })}
              onGrillDontKnow={() => dispatchGrill({ type: "dontKnow" })}
              onGrillEndNow={() => dispatchGrill({ type: "endNow" })}
              onGrillApplyAndScore={applyGrillAndScore}
              onGrillApplyInstruction={applyGrillInstruction}
              onGrillUndoInstruction={
                instructionUndo === null
                  ? undefined
                  : () => {
                      setInstruction(instructionUndo);
                      setInstructionUndo(null);
                    }
              }
              onGrillSave={saveGrillNote}
              onGrillRetry={() => dispatchGrill({ type: "retry" })}
              useWorktree={useWorktree}
              baseBranch={baseBranch}
              baseRefreshNotice={baseRefreshNotice}
              onDismissBaseRefresh={() => setBaseRefreshNotice(null)}
              setBaseBranch={setBaseBranch}
              busy={busy}
              creating={creating}
              onCreate={create}
            />
          </>
        )}

        {/* 최하단 사용량 상태바 — 뷰와 무관하게 항상 보이도록 컴포저 아래에 둔다. */}
        <UsageBar />

        {/* 터미널 도크 — 사용량 상태바보다 아래, 창의 최하단에 붙는다 (⌃`).
            작업 워크트리 셸이므로 작업이 선택된 워크스페이스 뷰에서만 뜬다. */}
        {view === "workspace" && selected != null && terminalDock && (
          <TerminalDock
            taskId={selected.id}
            available={selectedCaps.workspaceShell}
            label={repoBase(selected.worktree_path)}
            codeFontFamily={codeFontFamily}
            codeFontSize={codeFontSize}
            onClose={() => setTerminalDock(false)}
          />
        )}
      </main>

      <QuickOpen
        open={quickOpen.open}
        scopes={quickOpen.scopes}
        files={workspaceFiles}
        repo={selected?.repo ?? repo}
        // projectSearch는 로컬 transport만 쓴다 — 원격 작업의 내용 검색은 열지 않는다.
        taskId={selectedHost === LOCAL_HOST ? selectedId : null}
        onClose={closeQuickOpen}
        onSelect={handleQuickOpenSelect}
      />

      <SessionNavigator
        open={sessionNavigator}
        tasks={visibleTasks}
        projects={projects}
        groups={projectGroups}
        onClose={() => setSessionNavigator(false)}
        onOpenTask={openTask}
      />

      <RemovalUndoBar pending={removal.pending} onUndo={removal.undo} />

      <AutosaveUndoBar
        notice={autosaveNotice}
        onDismiss={() => setAutosaveNotice(null)}
        onUndo={() => {
          const notice = autosaveNotice;
          if (!notice) return;
          setAutosaveNotice(null);
          void (async () => {
            const reverted: string[] = [];
            for (const entry of notice.entries) {
              if (entry.content === null) continue; // 버퍼를 잡지 않은 큰 파일
              try {
                await fsWrite({ host: selectedHost, id: notice.taskId }, entry.path, entry.content);
                reverted.push(entry.path);
              } catch (e) {
                setErr(`${entry.path} 되돌리기 실패: ${String(e)}`);
              }
            }
            if (reverted.length === 0) return;
            // 디스크가 바뀌었으니 열려 있는 쪽이 다시 읽어야 한다.
            for (const path of reverted) void reloadFile(path);
            void emitTo(EDITOR_WINDOW_LABEL, EDITOR_REVERTED_EVENT, reverted).catch(() => {});
          })();
        }}
      />
      <VoiceHUD state={voiceHud} />

      {/* 파일 트리의 우클릭 메뉴와 새 파일·폴더 다이얼로그 — 트리 안에 두면 트리를 접는 순간
          함께 사라져, 만드는 중에 ⌘B를 누르면 입력하던 이름을 잃는다. */}
      <ContextMenuShell
        at={treeOps.menu}
        ariaLabel={treeOps.menu?.node ? `${treeOps.menu.node.name} 조작` : "워크트리 루트 조작"}
        header={treeOps.menu?.node?.name ?? "워크트리 루트"}
        rows={treeOps.rows}
        onClose={treeOps.closeMenu}
        resetKey={treeOps.menu?.node?.path ?? ""}
      />
      <FilePromptDialog {...treeOps.promptProps} />
      <LinkContextMenu
        menu={linkMenu}
        kind={linkMenuKind}
        supportsExternalPath={selectedCaps.fileOperations}
        onAction={runLinkMenu}
        onClose={() => setLinkMenu(null)}
      />
    </div>
    </HostScopeProvider>
  );
}

export default App;
