import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Icon, type IconName } from "./icons";
import { RepoPicker, type RecentRepo } from "./RepoPicker";
import { BranchPicker } from "./BranchPicker";
import { AgentPicker } from "./AgentPicker";
import { ModelPicker } from "./ModelPicker";
import { SpeedPicker } from "./SpeedPicker";
import { EffortPicker } from "./EffortPicker";
import { IsolationPicker } from "./IsolationPicker";
import { reasoningEffortsForModel } from "../../lib/models";
import {
  skillsList,
  fsTreePath,
  flattenFiles,
  gitStatus,
  gitInit,
  gitBranches,
  gitCheckoutBranch,
  type BranchList,
  useWorktreeOverrideGet,
  useWorktreeOverrideClear,
  useWorktreeSet,
  type SkillMeta,
} from "../../lib/ipc";
import {
  matchMentionToken,
  filterMentionFiles,
  applyMention,
  handleMenuKey,
  filterSkills,
} from "../../lib/mention";
import {
  mergeMentionItems,
  mentionInsertText,
  type MentionItem,
} from "../../lib/mention-knowledge";
import { useKnowledgeMentions } from "../use-knowledge-mentions";
import { pasteImageSave } from "../../lib/ipc";
import { findImageFile, pastedImageRef, insertAtCaret, blobToBase64 } from "../../lib/paste-image";
import { MentionDropdown } from "./MentionDropdown";
import { SkillDropdown } from "./SkillDropdown";
import {
  LOCAL_HOST,
  hasHost,
  type HostId,
  type SessionHomeSession,
} from "../../lib/transport";
import { hostCapabilities } from "../../lib/host-capabilities";
import { SessionHomePicker } from "./SessionHomePicker";
import type { InterviewState } from "../../lib/interview";
import type { GrillState } from "../../lib/grill";
import { InterviewPanel } from "./InterviewPanel";
import { useTextareaUndo } from "./useTextareaUndo";
import { EnglishPromptAssistPanel, useEnglishPromptAssist } from "./useEnglishPromptAssist";
import {
  isolationChipState,
  isolationForced,
  choiceFromOverride,
  overrideFromChoice,
  type IsolationChoice,
} from "./new-task-isolation";
import { stageLabel, ensembleLabel, elapsedLabel, type CreationState } from "../../lib/creation-stage";
import { QUESTION_AGENTS } from "../../lib/conversation-interaction";
import type { SessionStyle } from "../../lib/session-style";

/** off: 대화(턴마다 `-p`) · questions: 대화 + 질문 응답 · approvals: 대화 + 질문 응답 + 실행 전 승인(Claude만).
 *  터미널/대화 자체는 설정 › 작업 실행의 세션 방식이 정한다(`session-style.ts`). */
type InteractionMode = "off" | "questions" | "approvals";

interface Props {
  serviceTier?: "default" | "fast";
  onServiceTierChange?: (value: "default" | "fast") => void;
  questionsEnabled?: boolean;
  onQuestionsEnabledChange?: (value: boolean) => void;
  approvalsEnabled?: boolean;
  onApprovalsEnabledChange?: (value: boolean) => void;
  /** 설정의 세션 방식. 터미널이면 대화 세부 선택 대신 터미널 표시만 둔다 — 이어받기는 대화로만 잇는다. */
  sessionStyle?: SessionStyle;
  /** 새 세션이 살 환경. 생성 시점에 고정되므로 여기서만 고를 수 있다 (ADR 0133). */
  host: HostId;
  setHost: (host: HostId) => void;
  repo: string;
  setRepo: (s: string) => void;
  recentRepos: RecentRepo[];
  agents: string[];
  setAgents: (a: string[]) => void;
  /** 세션홈에서 고른 벤더 세션 — 이번 작업이 이어받을 대상(설계 2026-09-17). 선택 시 고정된다. */
  resumeSession: SessionHomeSession | null;
  onResumeSessionChange: (session: SessionHomeSession | null) => void;
  /** 등록된 프로젝트 경로 — 세션 이어받기 트리가 세션의 cwd를 프로젝트에 묶는 기준(설계 2026-09-18). */
  projects?: readonly string[];
  /** 세션 단위 모델 오버라이드 ("" = 설정의 벤더 기본). 단일 에이전트일 때만 UI 노출. */
  model: string;
  setModel: (m: string) => void;
  /** Codex 세션 단위 reasoning override ("" = Codex 설정 기본값). */
  reasoningEffort: string;
  setReasoningEffort: (effort: string) => void;
  instruction: string;
  setInstruction: (s: string) => void;
  /** 인터뷰 상태·핸들러 — 로컬 전용(원격 Runner에서는 패널 숨김). */
  interview: InterviewState;
  onInterviewStart: () => void;
  onInterviewAnswer: (questionId: string, answer: string) => void;
  onInterviewCrystallize: () => void;
  onInterviewRetry: () => void;
  /** 인터뷰 패널 ✕ — 두 단계를 모두 idle로 되돌린다. 없으면 ✕를 그리지 않는다. */
  onInterviewClose?: () => void;
  /** 인터뷰 1단계(깊게 파기) 상태·핸들러 — 채점과 한 패널에서 순서대로 진행한다. */
  grill: GrillState;
  onGrillStart: () => void;
  onGrillDraft: (value: string) => void;
  onGrillAnswer: () => void;
  onGrillAcceptRecommendation: () => void;
  onGrillDontKnow: () => void;
  onGrillEndNow: () => void;
  /** 개선된 지시문을 적용하고 바로 2단계(채점)로 이어간다. */
  onGrillApplyAndScore: () => void;
  onGrillApplyInstruction: () => void;
  /** 지시문을 적용한 뒤에만 주어진다 — 되돌릴 원문이 없으면 undefined. */
  onGrillUndoInstruction?: () => void;
  onGrillSave: () => void;
  onGrillRetry: () => void;
  /** 로컬 단일 작업의 워크트리 격리 설정. null은 설정 로드 전 상태. */
  useWorktree: boolean | null;
  /** 새 작업의 시작 브랜치. 격리는 분기 기준, 직접 실행은 checkout 대상이다. */
  baseBranch: string;
  /** base 최신화 결과 한 줄. 알릴 값이 있을 때만 온다(정상은 백엔드가 보내지 않는다). */
  baseRefreshNotice?: string | null;
  onDismissBaseRefresh?: () => void;
  setBaseBranch: (branch: string) => void;
  busy: boolean;
  /** busy 동안 "어디까지 왔는가" — null이면 이벤트가 아직/전혀 오지 않은 생성(설계 0059). */
  creating: CreationState | null;
  onCreate: () => void;
}

/** 조회 전·실패·원격의 공통 상태 — 상수라 effect 의존성으로 새 객체가 새지 않는다. */
const emptyBranches: BranchList = { current: "", branches: [] };

const chip = "flex items-center gap-1 text-xs text-text-secondary border border-border rounded-md px-2 py-1";
const Chip = ({ icon, label }: { icon: IconName; label: string }) => (
  <span className={chip}>
    <Icon name={icon} size={13} />
    {label}
  </span>
);

/** 홈 하단 컴포저 — 레포/브랜치/워크트리 칩 + 지시문 입력으로 새 작업 생성. */
export function Composer({
  host,
  repo,
  setRepo,
  recentRepos,
  agents,
  setAgents,
  resumeSession,
  onResumeSessionChange,
  projects,
  model,
  setModel,
  reasoningEffort,
  setReasoningEffort,
  serviceTier = "default",
  onServiceTierChange,
  questionsEnabled = false,
  onQuestionsEnabledChange,
  approvalsEnabled = false,
  onApprovalsEnabledChange,
  sessionStyle = "conversation",
  instruction,
  setInstruction,
  interview,
  onInterviewStart,
  onInterviewAnswer,
  onInterviewCrystallize,
  onInterviewRetry,
  onInterviewClose,
  grill,
  onGrillStart,
  onGrillDraft,
  onGrillAnswer,
  onGrillAcceptRecommendation,
  onGrillDontKnow,
  onGrillEndNow,
  onGrillApplyAndScore,
  onGrillApplyInstruction,
  onGrillUndoInstruction,
  onGrillSave,
  onGrillRetry,
  useWorktree,
  baseBranch,
  baseRefreshNotice,
  onDismissBaseRefresh,
  setBaseBranch,
  busy,
  creating,
  onCreate,
}: Props) {
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const undo = useTextareaUndo({
    value: instruction,
    setValue: setInstruction,
    ref: inputRef,
    resetKey: host,
  });
  // 호스트를 바꾸면 초안이 갈리므로 진행 중인 ⌘J 결과를 버린다 — 되돌리기와 같은 좌표.
  const assist = useEnglishPromptAssist({ value: instruction, onChange: setInstruction, draftKey: host });
  const [skillOpen, setSkillOpen] = useState(false);
  const [skills, setSkills] = useState<SkillMeta[]>([]);
  const [skillSel, setSkillSel] = useState(0);
  const skillCacheRef = useRef<{ host: HostId; repo: string; list: SkillMeta[] } | null>(null);

  // @파일 멘션 드롭다운 — 레포 파일을 lazy-fetch + 캐시.
  const [fileMenuOpen, setFileMenuOpen] = useState(false);
  /** 파일 경로만 — 지식 결과는 별도로 와서 렌더 직전에 합쳐진다. */
  const [fileNames, setFileNames] = useState<string[]>([]);
  const [fileSel, setFileSel] = useState(0);
  /** 활성 @토큰. null이면 멘션 중이 아니다. */
  const [mentionToken, setMentionToken] = useState<string | null>(null);
  const [sessionPickerOpen, setSessionPickerOpen] = useState(false);
  /** 선택된 레포가 git 저장소인지 — null은 아직 조회 전(또는 레포 미선택). */
  const [isRepo, setIsRepo] = useState<boolean | null>(null);
  const [initBusy, setInitBusy] = useState(false);
  const [initError, setInitError] = useState<string | null>(null);
  const [pasteError, setPasteError] = useState<string | null>(null);
  // 경과초 — 낭독되는 단계 문구와 분리된 aria-hidden 보조 정보라 250ms마다 갱신해도 된다.
  const [elapsedMs, setElapsedMs] = useState(0);
  useEffect(() => {
    if (!busy) {
      setElapsedMs(0);
      return;
    }
    const start = Date.now();
    const id = window.setInterval(() => setElapsedMs(Date.now() - start), 250);
    return () => window.clearInterval(id);
  }, [busy]);
  // 붙여넣기 저장(await) 중 사용자가 계속 타이핑할 수 있다 — 삽입은 최신 지시문 기준.
  const instructionRef = useRef(instruction);
  instructionRef.current = instruction;
  const fileCacheRef = useRef<{ host: HostId; repo: string; files: string[] } | null>(null);
  const fileReqRef = useRef(0); // fetch 세대 카운터 — stale 응답 무시용
  // 저장된 원격 선택을 로컬로 바꾸지 않는다. 준비가 끝날 때까지 전송만 막는다.
  const activeHost = host;
  const connected = hasHost(activeHost);
  // 이 프로젝트 전용 격리 설정 — null은 "전역 기본 따름"(로드 전에도 같은 값이라
  // 강제 격리가 아닌 동안에는 아래 chipState가 표시 여부를 최종 판단한다).
  const [wtOverride, setWtOverride] = useState<boolean | null>(null);
  const forced = isolationForced("local", agents.length);
  const chipState = isolationChipState(wtOverride, useWorktree);
  const [branches, setBranches] = useState<BranchList>(emptyBranches);
  const [checkingOut, setCheckingOut] = useState(false);
  const [checkoutError, setCheckoutError] = useState<string | null>(null);
  const repoRef = useRef(repo);
  repoRef.current = repo;
  // 격리 실행은 선택 base에서 분기하고, 직접 실행은 선택 브랜치로 체크아웃한 뒤 시작한다.
  const canPickBranch = isRepo === true && branches.branches.length > 0;

  // 새 작업 지시도 세션 대화창처럼 여러 줄을 그대로 보여주되 최대 높이를 제한한다.
  useLayoutEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    input.style.height = "auto";
    input.style.height = `${Math.min(input.scrollHeight, 208)}px`;
  }, [instruction]);

  // /로 시작하고 공백 없으면 → 스킬 드롭다운.
  useEffect(() => {
    const m = instruction.match(/^\/([^\s]*)$/);
    // 목록은 작업이 사는 호스트에서 온다.
    // repo는 프로젝트 레이어 스킬에만 필요하다 — 글로벌 스킬은 레포를 고르기 전에도 있다.
    if (!m) {
      setSkillOpen(false);
      return;
    }
    const prefix = m[1].toLowerCase();

    const applyFilter = (list: SkillMeta[]) => {
      const filtered = filterSkills(list, prefix);
      setSkills(filtered);
      setSkillSel(0);
      setSkillOpen(filtered.length > 0);
    };

    const cached = skillCacheRef.current;
    if (cached && cached.host === activeHost && cached.repo === repo) {
      applyFilter(cached.list);
      return;
    }
    skillsList(activeHost, repo).then((list) => {
      skillCacheRef.current = { host: activeHost, repo, list };
      applyFilter(list);
    });
  }, [instruction, activeHost, repo]);

  // 캐럿 앞 @토큰 감지 → 레포 파일 멘션 메뉴 (lazy-fetch + 캐시).
  useEffect(() => {
    const el = inputRef.current;
    const caret = el?.selectionStart ?? instruction.length;
    const tk = matchMentionToken(instruction.slice(0, caret));
    setMentionToken(tk);
    // repo 미선택은 **파일 결과만** 비운다. 지식은 프로젝트에 매이지 않으므로
    // repo 없이도 계속 검색된다 (설계 0020 DR-13).
    if (tk === null || !repo.trim() || !connected) {
      setFileNames([]);
      return;
    }
    const show = (files: string[]) => {
      setFileNames(filterMentionFiles(files, tk));
      setFileSel(0);
    };
    if (fileCacheRef.current?.host === activeHost && fileCacheRef.current.repo === repo) {
      show(fileCacheRef.current.files);
      return;
    }
    const reqId = ++fileReqRef.current;
    fsTreePath(activeHost, repo)
      .then((tree) => {
        if (reqId !== fileReqRef.current) return; // 이후 다른 repo fetch가 시작됨 → stale 무시
        const files = flattenFiles(tree);
        fileCacheRef.current = { host: activeHost, repo, files };
        show(files);
      })
      .catch(() => {
        if (reqId === fileReqRef.current) setFileNames([]);
      });
  }, [instruction, repo, activeHost, connected]);

  // 외부 지식 — 파일 결과를 기다리지 않고 따로 온다. 늦거나 실패해도 파일 멘션은 그대로다.
  const { hits: knowledgeHits, pending: knowledgePending } = useKnowledgeMentions(mentionToken);
  const mentionItems = useMemo(
    () => mergeMentionItems(fileNames, knowledgeHits),
    [fileNames, knowledgeHits],
  );

  // 열림 여부는 항목이 실제로 생겼을 때만 바뀐다 — Esc로 닫은 뒤 같은 토큰에서
  // 지식 결과가 늦게 도착해도 메뉴가 되살아나지 않게 한다.
  useEffect(() => {
    setFileMenuOpen(mentionToken !== null && mentionItems.length > 0);
  }, [mentionToken, mentionItems]);

  // repo 변경 시 캐시 무효화.
  useEffect(() => {
    if (skillCacheRef.current && (skillCacheRef.current.repo !== repo || skillCacheRef.current.host !== activeHost)) {
      skillCacheRef.current = null;
      setSkillOpen(false);
    }
    if (fileCacheRef.current && (fileCacheRef.current.repo !== repo || fileCacheRef.current.host !== activeHost)) {
      fileCacheRef.current = null;
      setFileMenuOpen(false);
    }
  }, [repo, activeHost]);

  // 선택된 레포가 격리 실행이 가능한 상태인지 — 준비 안내 노출을 가른다. 저장소 여부만이
  // 아니라 커밋 유무까지 본다(백엔드 `is_ready_repository`).
  useEffect(() => {
    setInitError(null);
    if (!repo.trim() || !connected) {
      setIsRepo(null);
      return;
    }
    let stale = false;
    gitStatus(activeHost, repo)
      .then((value) => {
        if (!stale) setIsRepo(value);
      })
      // 조회 실패 시 안내를 띄우지 않는다 — 서버가 판단하게 두고 기존 UI를 유지.
      .catch(() => {
        if (!stale) setIsRepo(null);
      });
    return () => {
      stale = true;
    };
  }, [repo, activeHost, connected]);

  // base 후보 브랜치 — 레포를 바꿀 때마다 다시 읽는다.
  useEffect(() => {
    setCheckoutError(null);
    if (!repo.trim() || !connected) {
      setBranches(emptyBranches);
      return;
    }
    let stale = false;
    gitBranches(repo)
      .then((list) => {
        if (!stale) setBranches(list);
      })
      // git 저장소가 아니거나 조회에 실패하면 피커를 숨긴다 — 백엔드는 현재 checkout을 쓴다.
      .catch(() => {
        if (!stale) setBranches(emptyBranches);
      });
    return () => {
      stale = true;
    };
  }, [repo, connected]);

  // 피커에서 바로 체크아웃한다. 전환 뒤에는 고른 base를 비워 새 현재 브랜치를 따르게 한다.
  const checkoutBranch = async (branch: string) => {
    const target = repo;
    setCheckingOut(true);
    setCheckoutError(null);
    try {
      const list = await gitCheckoutBranch(target, branch);
      if (repoRef.current !== target) return;
      setBranches(list);
      setBaseBranch("");
    } catch (error) {
      if (repoRef.current === target) setCheckoutError(String(error));
    } finally {
      setCheckingOut(false);
    }
  };

  // 레포별 격리 오버라이드 — 레포를 바꿀 때마다 다시 읽는다.
  useEffect(() => {
    if (!repo.trim() || !connected) {
      setWtOverride(null);
      return;
    }
    let stale = false;
    useWorktreeOverrideGet(repo)
      .then((value) => {
        if (!stale) setWtOverride(value);
      })
      // 조회 실패는 "오버라이드 없음"과 같게 다룬다 — 전역 기본이 그대로 적용된다.
      .catch(() => {
        if (!stale) setWtOverride(null);
      });
    return () => {
      stale = true;
    };
  }, [repo, connected]);

  // 칩 클릭: 기본 따름 → 이 프로젝트만 켬 → 이 프로젝트만 끔 → 기본 따름.
  // 낙관적으로 먼저 반영하고 저장에 실패하면 되돌린다(설정 하나라 롤백이 안전하다).
  const setIsolation = async (choice: IsolationChoice) => {
    const prev = wtOverride;
    const next = overrideFromChoice(choice);
    setWtOverride(next);
    try {
      if (next === null) await useWorktreeOverrideClear(repo);
      else await useWorktreeSet(next, repo);
    } catch {
      setWtOverride(prev);
    }
  };

  const initRepo = async () => {
    setInitBusy(true);
    setInitError(null);
    try {
      await gitInit(activeHost, repo);
      setIsRepo(true);
    } catch (error) {
      setInitError(String(error));
    } finally {
      setInitBusy(false);
    }
  };

  const insertSkill = (name: string) => {
    const next = instruction.replace(/^\/[^\s]*$/, `/${name} `);
    setInstruction(next);
    setSkillOpen(false);
    requestAnimationFrame(() => inputRef.current?.focus());
  };

  const insertMention = (item: MentionItem) => {
    const el = inputRef.current;
    const caret = el?.selectionStart ?? instruction.length;
    // 파일은 경로 그대로(기존 계약), 지식은 출처가 보이는 라벨로 들어간다.
    const { value: next, caret: pos } = applyMention(instruction, caret, mentionInsertText(item));
    setInstruction(next);
    setFileMenuOpen(false);
    requestAnimationFrame(() => {
      inputRef.current?.setSelectionRange(pos, pos);
      inputRef.current?.focus();
    });
  };

  // 클립보드 이미지 붙여넣기 — 레포의 `.praxis/pasted/`에 저장하고 경로 참조를 지시문에 삽입
  // (에이전트가 그 경로의 파일을 읽는다). 작업 생성 전이라 캡처 칩 파이프라인(worktree)은 없다.
  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const file = findImageFile(e.clipboardData);
    if (!file || !repo.trim() || busy) return;
    e.preventDefault();
    const caret = inputRef.current?.selectionStart ?? instruction.length;
    void (async () => {
      try {
        const path = await pasteImageSave(repo, await blobToBase64(file), file.type);
        const { value: next, caret: pos } = insertAtCaret(
          instructionRef.current,
          caret,
          pastedImageRef(path),
        );
        setInstruction(next);
        setPasteError(null);
        requestAnimationFrame(() => {
          inputRef.current?.setSelectionRange(pos, pos);
          inputRef.current?.focus();
        });
      } catch (error) {
        setPasteError(String(error));
      }
    })();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (undo.onKeyDown(e)) return;
    // ⌘J 영어로 다듬기 — 조합 중이면 스스로 무시한다. 생성 중(readOnly)에는 초안을 바꾸지 않는다.
    if (!busy && assist.onKeyDown(e)) return;
    // Shift+Enter는 메뉴가 열려 있어도 브라우저 기본 줄바꿈을 그대로 허용한다.
    if (e.key === "Enter" && e.shiftKey) return;
    // /스킬 드롭다운 (↑↓/Tab/Esc — Enter는 태스크 생성으로 fallthrough).
    if (
      skillOpen &&
      handleMenuKey(e, {
        items: skills,
        sel: skillSel,
        setSel: setSkillSel,
        onSelect: (sk) => insertSkill(sk.name),
        close: () => setSkillOpen(false),
      })
    )
      return;
    // @파일 메뉴 열림 시 Enter가 태스크 생성으로 새지 않도록 먼저 가로챈다.
    if (
      fileMenuOpen &&
      handleMenuKey(e, {
        items: mentionItems,
        sel: fileSel,
        setSel: setFileSel,
        onSelect: insertMention,
        close: () => setFileMenuOpen(false),
        enterSelects: true,
      })
    )
      return;
    if (e.key !== "Enter") return;
    e.preventDefault();
    if (canCreate) onCreate();
  };

  const canCreate = !busy && connected && !!repo.trim() && !!instruction.trim();
  const showResume = hostCapabilities(activeHost).sessionHomeResume;
  const showInteraction = host === LOCAL_HOST && agents.length === 1 && QUESTION_AGENTS.includes(agents[0]) && !!onQuestionsEnabledChange;
  const canApprove = agents[0] === "claude" && !!onApprovalsEnabledChange;
  // 이어받기는 대화 세션만 이을 수 있다 — 설정이 터미널이어도 이어받기는 대화 선택을 보인다.
  const terminalSession = showInteraction && sessionStyle === "terminal" && !resumeSession;
  // 승인은 질문 응답 위에서만 성립하므로 두 체크박스 대신 한 선택으로 묶는다.
  const interactionMode: InteractionMode = !questionsEnabled ? "off" : canApprove && approvalsEnabled ? "approvals" : "questions";
  const setInteractionMode = (mode: InteractionMode) => {
    onQuestionsEnabledChange?.(mode === "questions" || mode === "approvals");
    onApprovalsEnabledChange?.(mode === "approvals");
  };
  return (
    <div className="border-t border-border px-4 pt-2 pb-3 bg-bg shrink-0">
      <div className="composer-toolbar mb-2">
        <RepoPicker
          repo={repo}
          recentRepos={recentRepos}
          onPick={setRepo}
        />
        {/* 격리 실행의 분기 기준 또는 직접 실행이 체크아웃할 시작 브랜치. */}
        {canPickBranch && (
          <BranchPicker
            value={baseBranch}
            current={branches.current}
            branches={branches.branches}
            onPick={setBaseBranch}
            direct={!forced && chipState?.on === false}
            onCheckout={checkoutBranch}
            checkingOut={checkingOut || busy}
          />
        )}
        <AgentPicker agents={agents} onChange={setAgents} />
        {/* 세션 모델 오버라이드 — 앙상블(2개+)은 벤더별 기본을 따르므로 단일 선택일 때만 노출 */}
        {agents.length === 1 && (
          <ModelPicker agent={agents[0]} model={model} onChange={setModel} />
        )}
        {host === LOCAL_HOST && agents.length === 1 && agents[0] === "codex" && onServiceTierChange && (
          <SpeedPicker model={model} value={serviceTier} onChange={onServiceTierChange} disabled={busy} />
        )}
        {/* 세션 reasoning effort 오버라이드 — 모델이 지원할 때만 노출 */}
        {agents.length === 1 && reasoningEffortsForModel(agents[0], model).length > 0 && (
          <EffortPicker
            agent={agents[0]}
            model={model}
            effort={reasoningEffort}
            onChange={setReasoningEffort}
          />
        )}
        {/* 격리(브랜치·워크트리)는 커밋이 있는 git 저장소에서만 가능하다 — 아니면 폴더에서 직접
            실행된다. `git init`만 한 폴더도 여기 걸린다: 저장소지만 HEAD가 없어 격리가 안 선다. */}
        {isRepo === false ? (
          <>
            <Chip icon="folder" label="직접 실행 · 격리 불가" />
            <button
              className="flex items-center gap-1 text-xs text-text-secondary border border-border rounded-md px-2 py-1 hover:border-border-strong disabled:opacity-50"
              disabled={initBusy}
              onClick={() => void initRepo()}
              title="저장소가 없으면 만들고, 커밋이 없으면 현재 내용을 초기 커밋으로 남깁니다"
            >
              <Icon name="branch" size={13} />
              {initBusy ? "준비 중…" : "git 준비하기"}
            </button>
          </>
        ) : forced ? (
          // 원격·앙상블은 설정과 무관하게 항상 격리된다 — 사실만 알리고 누를 수 없다.
          <>
            <Chip icon="branch" label="새 브랜치" />
            <Chip icon="branch" label="워크트리" />
          </>
        ) : (
          chipState && (
            <>
              {chipState.on && <Chip icon="branch" label="새 브랜치" />}
              <IsolationPicker
                choice={choiceFromOverride(wtOverride)}
                globalDefault={useWorktree ?? true}
                onPick={(next) => void setIsolation(next)}
              />
            </>
          )
        )}
      </div>
      {!connected && activeHost !== LOCAL_HOST && (
        <p className="mb-2 text-xs text-text-muted" role="status">서버에 연결 중입니다. 연결될 때까지 전송할 수 없습니다.</p>
      )}
      {/* 최신화 결과는 status 색을 쓰지 않는다 — 최신화가 안 됐어도 **작업은 성공했다**.
          상태색은 상태 표시 전용이다(DESIGN.md). */}
      {baseRefreshNotice && (
        <div className="mb-2 flex items-start gap-2 text-xs text-text-secondary">
          <Icon name="branch" size={12} />
          <span className="flex-1">{baseRefreshNotice}</span>
          <button
            className="text-text-muted hover:text-text-secondary"
            aria-label="최신화 알림 닫기"
            onClick={onDismissBaseRefresh}
          >
            ✕
          </button>
        </div>
      )}
      {initError && <div className="mb-2 text-xs text-status-failed">{initError}</div>}
      {checkoutError && (
        <div className="mb-2 text-xs text-status-failed">브랜치 전환 실패: {checkoutError}</div>
      )}
      {pasteError && (
        <div className="mb-2 text-xs text-status-failed">이미지 첨부 실패: {pasteError}</div>
      )}
      <InterviewPanel
        grill={grill}
        interview={interview}
        currentInstruction={instruction}
        currentRepo={repo}
        disabled={!repo.trim() || !instruction.trim()}
        onGrillStart={onGrillStart}
        onGrillDraft={onGrillDraft}
        onGrillAnswer={onGrillAnswer}
        onGrillAcceptRecommendation={onGrillAcceptRecommendation}
        onGrillDontKnow={onGrillDontKnow}
        onGrillEndNow={onGrillEndNow}
        onGrillApplyAndScore={onGrillApplyAndScore}
        onGrillApplyInstruction={onGrillApplyInstruction}
        onGrillUndoInstruction={onGrillUndoInstruction}
        onGrillSave={onGrillSave}
        onGrillRetry={onGrillRetry}
        onInterviewStart={onInterviewStart}
        onInterviewAnswer={onInterviewAnswer}
        onInterviewCrystallize={onInterviewCrystallize}
        onInterviewRetry={onInterviewRetry}
        onClose={onInterviewClose}
      />
      <EnglishPromptAssistPanel state={assist.state} onDismiss={assist.clear} />
      <div className="relative">
        <SkillDropdown open={skillOpen} items={skills} sel={skillSel} onHover={setSkillSel} onSelect={insertSkill} />
        <MentionDropdown
          open={fileMenuOpen}
          items={mentionItems}
          sel={fileSel}
          token={mentionToken ?? ""}
          knowledgePending={knowledgePending}
          onHover={setFileSel}
          onSelect={insertMention}
        />
        <div className="flex items-end gap-2 border border-border-strong rounded-md px-3 py-2 focus-within:border-primary">
          <textarea
            ref={inputRef}
            rows={1}
            className="flex-1 max-h-52 resize-none bg-transparent outline-none text-md leading-relaxed text-text placeholder:text-text-muted"
            placeholder="작업을 설명하세요 — Enter 생성 · Shift+Enter 줄바꿈 · @파일 · /스킬 · 이미지 붙여넣기 · ⌘J 영어로"
            value={instruction}
            onChange={undo.onChange}
            onKeyDown={onKeyDown}
            onSelect={undo.onSelect}
            onCompositionStart={undo.onCompositionStart}
            onCompositionEnd={undo.onCompositionEnd}
            onPaste={onPaste}
            readOnly={busy}
            aria-busy={busy}
          />
          <button
            className={`shrink-0 ${canCreate ? "text-primary-bright" : "text-text-muted"}`}
            disabled={!canCreate}
            onClick={onCreate}
            aria-label="작업 생성"
            title="작업 생성 (Enter)"
          >
            <span className={busy ? "inline-flex animate-spin" : "inline-flex"}>
              <Icon name={busy ? "refresh" : "send"} size={18} />
            </span>
          </button>
        </div>
        {/* 무엇을·어디서(위 도구줄)와 달리, 세션이 어떻게 흘러갈지를 정하는 옵션은 입력창 아래에 둔다.
            한 줄에 모두 두면 Claude 질문 세션에서 도구줄이 두 줄로 넘어갔다. */}
        {(showResume || showInteraction) && (
          <div className="composer-toolbar mt-1.5">
            {showResume && (
              resumeSession ? (
                <span className={chip}>
                  <Icon name="clock" size={13} />
                  <span
                    className="max-w-[160px] truncate"
                    title={resumeSession.title?.trim() || resumeSession.first_message?.trim() || resumeSession.session_id}
                  >
                    {resumeSession.title?.trim() ||
                      resumeSession.first_message?.trim() ||
                      resumeSession.session_id.slice(0, 8)}
                  </span>
                  <button
                    type="button"
                    className="text-text-muted hover:text-text"
                    aria-label="이어받기 세션 해제"
                    onClick={() => onResumeSessionChange(null)}
                  >
                    <Icon name="x" size={12} />
                  </button>
                </span>
              ) : (
                <button
                  type="button"
                  className="flex items-center gap-1 text-xs text-text-secondary border border-border rounded-md px-2 py-1 hover:border-border-strong"
                  onClick={() => setSessionPickerOpen(true)}
                >
                  <Icon name="clock" size={13} />
                  세션 이어받기
                </button>
              )
            )}
            {terminalSession ? (
              <span
                className="text-xs text-text-secondary"
                title="실제 CLI를 앱 터미널에서 그대로 씁니다. 작업 화면에서는 터미널 안에서만 입력합니다. 설정 › 작업 실행 › 세션 방식에서 바꿉니다."
              >
                터미널 세션
              </span>
            ) : showInteraction && (
              <label title="대화: 턴마다 CLI를 -p로 실행해 말풍선으로 보여 줍니다. 질문 응답: 에이전트가 선택지 질문으로 답을 받아 이어갑니다. 실행 전 승인: 파일 수정·명령 실행 전에 허용 여부를 묻습니다(Claude만).">
                <select
                  aria-label="세션 방식"
                  value={interactionMode}
                  disabled={busy}
                  onChange={(e) => setInteractionMode(e.target.value as InteractionMode)}
                >
                  <option value="off">대화</option>
                  <option value="questions">대화 · 질문 응답</option>
                  {/* 실행 전 승인 — Claude 질문 세션만 CLI의 권한 프롬프트를 앱으로 받을 수 있다. */}
                  {canApprove && <option value="approvals">대화 · 질문 응답 · 실행 전 승인</option>}
                </select>
              </label>
            )}
          </div>
        )}
        {busy && (
          <div className="mt-1 flex items-center gap-2 text-xs text-text-secondary">
            <span role="status" aria-live="polite">
              {creating?.candidates != null
                ? ensembleLabel(creating.candidates)
                : stageLabel(creating?.stage ?? null, baseBranch)}
            </span>
            <span aria-hidden="true">{elapsedLabel(elapsedMs)}</span>
          </div>
        )}
      </div>
      {sessionPickerOpen && (
        <SessionHomePicker
          host={activeHost}
          repo={repo}
          projects={projects}
          onSelect={(session) => {
            onResumeSessionChange(session);
            // 공급자 간 문맥 이관은 하지 않는다. 고른 세션의 원래 에이전트로만 이어받는다.
            setAgents([session.vendor ?? "claude"]);
            setSessionPickerOpen(false);
          }}
          onClose={() => setSessionPickerOpen(false)}
        />
      )}
    </div>
  );
}
