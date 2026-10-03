import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
} from "react";
import Editor, { type OnMount } from "@monaco-editor/react";
// 타입만 — 런타임 번들은 @monaco-editor/react가 넘겨주는 인스턴스를 그대로 쓴다.
import type * as Monaco from "monaco-editor";
import { isMacPlatform } from "../../lib/hotkey";
import { looksLikeHtml } from "../../lib/looks-like-html";
import { langFromPath } from "../../lib/monaco";
import { useTheme } from "../../lib/use-theme";
import { Icon } from "./icons";
import { fileIcon } from "./file-icons";
import { MarkdownDoc } from "./MarkdownDoc";
import { HtmlDoc } from "./HtmlDoc";
import { DiffTab } from "./DiffTab";
import { TableView } from "./TableView";
import { GotoOverlay, type GotoState } from "./GotoOverlay";
import { ReferencesPanel } from "./ReferencesPanel";
import {
  dedupeTargets,
  resolveOutcome,
  shouldFallbackToReferences,
} from "../../lib/lsp";
import type {
  EditorSettings,
  FileKind,
  LspGotoKind,
  LspStatusInfo,
  LspTarget,
} from "../../lib/ipc";
import type { TabKey } from "../../lib/tab-key";
import { lspSemanticTokens } from "../../lib/ipc";
import { DEFAULT_EDITOR_SETTINGS } from "../../lib/editor-settings";
import { previewBoundsFromRect } from "../../lib/designmode/layout";
import { registerEditorCaptureTarget } from "../../lib/designmode/editor-capture-target";
import { buildSelectionCapture } from "../../lib/designmode/selection-capture";
import type { DesignCaptureRecord } from "../../lib/designmode/types";
import { pushCapture } from "../../lib/designmode/store";
import { requestComposerFocus } from "../../lib/composer-focus";
import { bubblePlacement, shouldShowBubble } from "../../lib/selection-ask";
import { linesOfRange } from "../../lib/md-selection";
import { SelectionAskBubble, type BubbleAnchor } from "./SelectionAskBubble";
import type { SplitAxis } from "./editor-split";

/** 에디터에 열린 탭 한 개. */
export interface OpenFile {
  /**
   * 탭의 정체성. 같은 경로의 파일 탭과 diff 탭이 공존하므로 `path`로는 가를 수 없다.
   * 파일 탭은 `fileTabKey(path)`, diff 탭은 `diffTabKey(path)`다(`lib/tab-key`).
   */
  key: TabKey;
  path: string;
  /** 백엔드 판별 종류 — text만 Monaco 편집 가능. image=data URL, binary/too_large=미리보기 불가.
   *  `diff`는 파일 내용이 아니라 변경분을 그리는 탭이다: `content`·`baseContent`가 빈 문자열,
   *  `mtime`이 0, `dirty`는 언제나 false다(저장할 것이 없다). */
  kind: FileKind | "diff";
  /** text=본문, image=data URL, binary/too_large="" */
  content: string;
  /** 디스크에서 마지막으로 읽은 내용. 편집해도 바뀌지 않는다.
   *
   *  `content`는 편집하는 순간 덮어써지므로 디스크 원본을 가리킬 것이 없어진다. 그 원본이 곧
   *  "에이전트가 만든 마지막 상태"이며, 자동 저장의 충돌 판정과 되돌리기가 여기를 기준점으로 쓴다. */
  baseContent: string;
  /** 디스크에서 읽은 시점의 mtime(ms) — 외부 변경 감지 기준. */
  mtime: number;
  dirty: boolean;
  /** 작업 폴더 밖에서 읽은 파일은 탭 정체성은 그대로 두고 수정만 막는다. */
  readOnly?: boolean;
  /** 훑어보기로 연 탭. 자리를 다음 프리뷰에 물려준다 — 물려주는 판단은 배치가 한다(ADR 0189). */
  preview?: boolean;
}

/** Monaco 컨텍스트 키 — ⇧⏎ 콘솔 실행이 걸리는 조건(Python 파일 + 콘솔 있음). */
const REPL_CONTEXT_KEY = "praxisReplRunnable";

const isMarkdown = (p: string) => /\.(md|markdown)$/i.test(p);
const isHtml = (p: string) => /\.(html?|xhtml)$/i.test(p);

type EditorInst = Parameters<OnMount>[0];
type MonacoInst = Parameters<OnMount>[1];

export interface EditorPaneProps {
  taskId: number | null;
  files: OpenFile[];
  /** 지금 보고 있는 탭의 키. 실경로가 필요한 동작은 그 탭의 `path`를 쓴다. */
  activeKey: TabKey | null;
  dark: boolean;
  onSelect: (key: TabKey) => void;
  onClose: (key: TabKey) => void;
  onChange: (path: string, content: string) => void;
  /** content = 에디터의 라이브 값(키 입력 직후 React state가 늦어도 유실 없게). */
  onSave: (path: string, content: string) => void;
  onReload: (path: string) => void;
  /** 기본 앱으로 열기 (바이너리/이미지/대용량 — OS 연결 프로그램). */
  onOpenPath: (path: string) => void;
  /** Finder에서 보기 (파일 선택된 채 폴더 열기). */
  onRevealPath: (path: string) => void;
  /** Markdown 미리보기 링크 우클릭. 문서 경로는 여기서 고정해 분할이 정확한 대상만 조작한다. */
  onLinkMenu?: (sourceKey: TabKey, sourcePath: string, link: string, at: { x: number; y: number }) => void;
  /** 렌더된 문서의 링크를 왼쪽 클릭했다. 링크를 푸는 것도 여는 것도 배치의 몫이다
   *  — 이 칸은 자기 id를 모르므로 "어디에 열지"를 답할 수 없다. */
  onOpenLink?: (sourceKey: TabKey, sourcePath: string, link: string) => void;
  /** 원격 파일은 클라이언트 OS 경로가 없으므로 기본 앱/Finder 동작을 숨긴다. */
  supportsExternalPath?: boolean;
  /** ⇧⏎ — 선택 영역(없으면 커서 줄)을 Python 콘솔로 보낸다. Python 파일에서만 걸린다.
   *  없으면 키바인딩을 걸지 않는다 — 콘솔은 로컬 에디터 팝아웃에만 있다. */
  onRunSelection?: (code: string) => void;
  /** 표 파일(parquet)을 Python 콘솔에서 pandas로 연다. 없으면 버튼을 그리지 않는다. */
  onOpenInRepl?: (path: string) => void;
  /** 완성된 코드 폰트 스택 문자열(fonts.ts codeFontStack 결과). 기본값 = 기존 하드코딩 값(미설정 시 픽셀 단위 동일). */
  codeFontFamily?: string;
  /** 코드 폰트 크기(px). 기본값 = 기존 하드코딩 값. */
  codeFontSize?: number;
  /** 에디터 동작 설정(미니맵·줄 바꿈·탭 크기). 없으면 기본값 = 이 설정이 생기기 전의 동작. */
  editorSettings?: EditorSettings;
  /** 정의/구현/사용처 조회 (⌘B·⌥⌘B). 없으면 키바인딩을 걸지 않는다 — 원격 워크트리는
   *  언어 서버를 띄울 수 없으므로 App이 로컬에서만 넘긴다. */
  onGoto?: (req: {
    kind: LspGotoKind;
    path: string;
    text: string;
    line: number;
    column: number;
  }) => Promise<LspTarget[]>;
  /** 이동 대상 열기 — 다른 파일이면 탭을 열고 그 줄로 스크롤한다. */
  onOpenTarget?: (
    target: LspTarget,
  ) => Promise<"opened" | "external" | "failed">;
  /** Split navigation controller receives the source cursor before an explicit target move. */
  onNavigateTarget?: (
    target: LspTarget,
    origin: {
      path: string;
      line: number;
      column: number;
      scrollTop: number;
      scrollLeft: number;
    },
  ) => void;
  /** 사용처 결과의 주인은 분할이다. 칸을 바꿔도 결과를 잃지 않게 한다. */
  beginReferences?: () => number;
  onReferences?: (
    targets: LspTarget[],
    sourcePath: string,
    request: number,
    origin: {
      path: string;
      line: number;
      column: number;
      scrollTop: number;
      scrollLeft: number;
    },
  ) => void;
  /** 현재 파일 내용 세대. 지연 LSP 응답은 요청 당시 세대가 아니면 버린다. */
  sourceVersion?: (path: string) => number;
  /** task/host/window tuple. Delayed LSP work must not cross this editor scope. */
  navigationScope?: string;
  /** 이 파일에서 정의 이동을 쓸 수 있는지 물어본다. 없으면 상태 배지를 띄우지 않는다. */
  onLspStatus?: (path: string) => Promise<LspStatusInfo>;
  /** App이 지시하는 커서 이동 지점. 대상 파일 탭이 활성화된 뒤에 적용된다. */
  reveal?: {
    path: string;
    line: number;
    column: number;
    scrollTop?: number;
    scrollLeft?: number;
    restore?: boolean;
    navId?: number;
    epoch?: number;
  } | null;
  /** reveal을 소비했음을 알린다 — 같은 위치로 두 번 점프하는 것을 막는다. */
  onRevealed?: (ack?: {
    navId: number;
    epoch: number;
    path?: string;
    line?: number;
    column?: number;
    scrollTop?: number;
    scrollLeft?: number;
  }) => void;
  /** 선택한 코드로 세션에 질문한다. 없으면 버블을 띄우지 않는다(⌘L 첨부는 그대로 동작). */
  onAsk?: (input: {
    filePath: string;
    selectionText: string;
    startLine: number;
    endLine: number;
    question: string;
  }) => void;
  onAskSeparately?: (input: { filePath: string; selectionText: string; startLine: number; endLine: number }) => void;
  /** ⌘L 선택 첨부의 목적지 재지정 — 팝아웃 창은 로컬 store 대신 메인 창으로 배달해야 한다.
   *  지정되면 컴포저 포커스도 부르지 않는다(사용자의 창을 뺏지 않는다). */
  onAttachCapture?: (record: DesignCaptureRecord) => void;
  /** 세션이 응답 중이면 버블 입력을 잠근다. */
  askBusy?: boolean;
  /** 전송 실패 사유 — 버블을 유지한 채 보여 주고 입력을 잃지 않는다. */
  askError?: string | null;
  /**
   * 모델을 살려 둘 경로 전체. 분할에서 **다른 그룹이 띄우고 있는 파일**까지 포함한다.
   *
   * `files`(이 그룹의 탭)로 판단하면, 같은 파일을 두 그룹이 열었을 때 한쪽에서 탭을 닫는
   * 순간 공유 모델이 dispose돼 옆 그룹의 에디터가 죽은 모델을 물게 된다.
   *
   * **필수다.** `files`로 떨어지는 폴백을 두면 diff 탭의 실경로가 모델을 붙잡는다 —
   * diff 탭은 모델을 만들지 않으므로 살려 둘 이유도 없다.
   */
  retainedPaths: string[];
  /** 분할 손잡이. 없으면 탭 바에 버튼을 두지 않는다(분할을 모르는 호출부). */
  onSplit?: (axis: SplitAxis) => void;
  /** 이 칸 접기. 그룹이 둘 이상일 때만 넘어온다 — 마지막 칸은 접을 곳이 없다. */
  onCloseGroup?: () => void;
  /** 프리뷰를 고정으로 승격 — 탭 더블클릭. 없으면 더블클릭이 아무 일도 하지 않는다. */
  onPinTab?: (key: TabKey) => void;
  /** 탭 우클릭. 좌표는 화면 기준이며 메뉴는 부르는 쪽이 소유한다 — 항목 절반이 배치 조작이라
   *  이 컴포넌트는 답을 갖고 있지 않다(자기가 몇 번째 칸인지도 모른다). */
  onTabMenu?: (key: TabKey, x: number, y: number) => void;
  /**
   * 탭을 집어 들었다. 지정되면 탭이 draggable이 된다 — 없으면 아예 끌리지 않는다.
   *
   * 운반 데이터를 여기서 싣지 않는 이유는 이 칸이 자기 id를 모르기 때문이다. 받는 쪽이
   * "어느 칸에서 왔는가"를 알아야 드롭이 복제가 아니라 이동이 된다(`EditorSplitView`).
   */
  onTabDragStart?: (key: TabKey, event: ReactDragEvent<HTMLElement>) => void;
  /** 지금 보고 있는 칸인지. 캡처·⌘L의 목적지가 taskId당 한 자리라 이 값으로 주인을 고른다. */
  focused?: boolean;
  /** 이 에디터 열이 화면에 보일 때만 Diff 단축키를 소유한다. */
  shortcutsActive?: boolean;
}

const GOTO_LABEL: Record<LspGotoKind, string> = {
  definition: "정의로 이동",
  implementation: "구현으로 이동",
  references: "사용처",
};

type LspInfoState = {
  scopeKey: string;
  path: string | null;
  state: "idle" | "loading" | "ready" | "error";
  value: LspStatusInfo | null;
};

const base = (p: string) => p.split("/").pop() ?? p;

/**
 * 살아 있는 칸들 — 마운트 순서대로.
 *
 * 언어 기능(정의·구현·사용처)은 Monaco **전역**에 걸리고, Monaco는 등록된 모든 프로바이더의
 * 결과를 이어 붙인다. 분할에서 같은 파일을 두 칸이 띄우면 그 순간 ⌘클릭 결과가 두 벌이 되고,
 * 정의가 하나뿐인 심볼조차 Peek 목록으로 열린다. 등록은 칸마다 하되 **답하는 칸은 하나**로
 * 정해 그것을 막는다 — 기준은 마운트 순서다(포커스로 고르면 클릭이 포커스를 옮기는 그 프레임에
 * 주인이 바뀌어 결과가 흔들린다).
 */
const paneOrder: symbol[] = [];
const paneFiles = new Map<symbol, () => OpenFile[]>();

/** 이 칸이 그 경로의 조회를 맡는가. 아무 칸도 안 들고 있으면 누구도 답하지 않는다.
 *  모델을 가진 칸만 후보다 — diff 탭은 같은 경로를 갖지만 Monaco 모델이 없다. */
function ownsGoto(token: symbol, path: string): boolean {
  return (
    paneOrder.find(
      (t) =>
        paneFiles
          .get(t)?.()
          .some((f) => f.kind === "text" && !f.readOnly && f.path === path) ?? false,
    ) === token
  );
}

/** 탭 바 + Monaco 멀티파일 에디터. Cmd/Ctrl+S 저장. */
export function EditorPane({
  taskId,
  files,
  activeKey,
  dark,
  onSelect,
  onClose,
  onChange,
  onSave,
  onReload,
  onOpenPath,
  onRevealPath,
  onLinkMenu,
  onOpenLink,
  supportsExternalPath = true,
  onRunSelection,
  onOpenInRepl,
  codeFontFamily = "JetBrains Mono, Fira Code, ui-monospace, monospace",
  codeFontSize = 13,
  editorSettings,
  onGoto,
  onOpenTarget,
  onNavigateTarget,
  onReferences,
  beginReferences,
  sourceVersion,
  navigationScope = String(taskId),
  onLspStatus,
  reveal = null,
  onRevealed,
  onAsk,
  onAskSeparately,
  onAttachCapture,
  askBusy = false,
  askError = null,
  retainedPaths,
  onSplit,
  onCloseGroup,
  onPinTab,
  onTabMenu,
  onTabDragStart,
  focused = true,
  shortcutsActive = true,
}: EditorPaneProps) {
  // Monaco 테마 이름 = 활성 테마 id (lib/monaco.ts가 테마별로 defineTheme 해둔다).
  const themeId = useTheme().id;
  const active = files.find((f) => f.key === activeKey) ?? null;
  // 렌더 가능한 문서(마크다운·HTML)의 표시 모드(경로별) — 기본 프리뷰. true면 Monaco 편집.
  const [sourceMode, setSourceMode] = useState<Record<string, boolean>>({});
  /** 내용 기반 HTML 판정을 경로마다 한 번만 하고 굳힌다.
   *
   *  라이브 버퍼로 매 렌더 다시 판정하면 편집 중에 화면이 튄다: 빈 `scratch.txt`에 `<p>hi</p>`를
   *  치다 마지막 `>`를 누르는 순간 판정이 뒤집혀 Monaco가 언마운트되고 iframe이 자리를 차지한다.
   *  포커스가 body로 날아가 **뒤이은 키 입력이 조용히 사라진다.** 처음 열었을 때의 내용으로
   *  결정하고 그대로 둔다 — 부수적으로 타이핑마다의 재판정 비용도 없어진다. */
  const contentHtmlByPath = useRef(new Map<string, boolean>());
  const contentLooksHtml = (path: string, content: string) => {
    const seen = contentHtmlByPath.current.get(path);
    if (seen !== undefined) return seen;
    const verdict = looksLikeHtml(content);
    contentHtmlByPath.current.set(path, verdict);
    return verdict;
  };
  const showSource = active ? (sourceMode[active.path] ?? false) : false;
  // 확장자가 아니어도 내용 전체가 HTML이면 HTML로 본다(`<table>`로 시작하는 .md·무확장자 파일).
  const activeIsHtml =
    active?.kind === "text" &&
    (isHtml(active.path) || contentLooksHtml(active.path, active.content));
  /** HTML로 판정되면 마크다운이 아니다 — 두 플래그는 배타적이어야 한다.
   *  프리뷰가 샌드박스 iframe이면 부모는 그 안의 선택도 `data-md-line` 표식도 읽을 수 없어,
   *  마크다운 첨부(⌘L) 경로가 켜져 있으면 빈 선택으로 동작한다. */
  const activeIsMarkdown =
    active?.kind === "text" && isMarkdown(active.path) && !activeIsHtml;
  /** 프리뷰를 가진 종류인가 — 그렇지 않은 텍스트는 언제나 Monaco다. */
  const activeIsRendered = activeIsMarkdown || activeIsHtml;
  // Monaco를 띄우는 조건: 텍스트이고, (프리뷰가 없는 종류거나) 프리뷰가 있어도 소스 모드일 때.
  const showEditor =
    active?.kind === "text" && (!activeIsRendered || showSource);
  /** 렌더된 문서를 보고 있는가 — Monaco가 없는 화면이라 선택·첨부 경로가 따로 필요하다. */
  const showDoc = active?.kind === "text" && !showEditor;

  // 키바인딩/저장 콜백이 항상 최신 핸들러·활성파일을 보도록 ref 경유.
  const saveRef = useRef(onSave);
  saveRef.current = onSave;
  /** 활성 탭의 **실경로**. 저장·첨부·정의 이동은 키가 아니라 파일을 다룬다. */
  const activeRef = useRef<string | null>(active?.path ?? null);
  activeRef.current = active?.path ?? null;
  const runSelectionRef = useRef(onRunSelection);
  runSelectionRef.current = onRunSelection;
  /** ⇧⏎ 게이트 — 활성 파일이 Python이고 콘솔이 있을 때만 참. */
  const replEnabled = onRunSelection != null && active?.kind === "text" && langFromPath(active.path) === "python";
  const replEnabledRef = useRef(replEnabled);
  replEnabledRef.current = replEnabled;
  const replContextRef = useRef<{ set: (v: boolean) => void } | null>(null);
  useEffect(() => {
    replContextRef.current?.set(replEnabled);
  }, [replEnabled]);
  const linkMenuRef = useRef(onLinkMenu);
  linkMenuRef.current = onLinkMenu;
  const openMarkdownLinkMenu = useCallback(
    (link: string, at: { x: number; y: number }) => {
      if (active == null) return;
      linkMenuRef.current?.(active.key, active.path, link, at);
    },
    [active?.key, active?.path],
  );
  const openLinkRef = useRef(onOpenLink);
  openLinkRef.current = onOpenLink;
  const openMarkdownLink = useCallback(
    (link: string) => {
      if (active == null) return;
      openLinkRef.current?.(active.key, active.path, link);
    },
    [active?.key, active?.path],
  );
  const taskIdRef = useRef(taskId);
  taskIdRef.current = taskId;
  const attachCaptureRef = useRef(onAttachCapture);
  attachCaptureRef.current = onAttachCapture;

  const editorRef = useRef<EditorInst | null>(null);
  const monacoRef = useRef<MonacoInst | null>(null);
  /** 시맨틱 토큰 프로바이더를 이미 건 언어 — 언어당 한 번만 등록한다. */
  const semanticLangsRef = useRef<Set<string>>(new Set());
  const paneRef = useRef<HTMLDivElement>(null);
  /** 마크다운 프리뷰의 스크롤 컨테이너. ⌘L이 "이 문서 안의 선택인가"를 여기로 판정한다. */
  const docRef = useRef<HTMLDivElement>(null);
  // 우리가 만든 모델 URI 집합 — 닫힌 파일 모델만 안전하게 dispose.
  const ownedRef = useRef<Set<string>>(new Set());
  const filesRef = useRef(files);
  filesRef.current = files;
  // 지연 생성 — useRef(Symbol())는 렌더마다 심볼을 새로 만든다(쓰이지 않아도).
  const tokenRef = useRef<symbol | null>(null);
  if (tokenRef.current === null) tokenRef.current = Symbol("editor-pane");
  const token = tokenRef.current;
  const lspScopeKey = String(taskId);
  const [lspState, setLspState] = useState<LspInfoState>({
    scopeKey: lspScopeKey,
    path: null,
    state: "idle",
    value: null,
  });
  const lspRequestRef = useRef(0);
  const lspPath = active?.kind === "text" && !active.readOnly ? active.path : null;
  const lspCurrent =
    lspState.scopeKey === lspScopeKey && lspState.path === lspPath;
  const lspInfo =
    lspCurrent && lspState.state === "ready" ? lspState.value : null;

  useEffect(() => {
    paneFiles.set(token, () => filesRef.current);
    paneOrder.push(token);
    return () => {
      paneFiles.delete(token);
      const at = paneOrder.indexOf(token);
      if (at >= 0) paneOrder.splice(at, 1);
    };
  }, [token]);

  /** prop 이 없으면 이 설정이 생기기 전의 동작으로 둔다 — 팝아웃·분할 등 아직 넘기지 않는
   *  경로가 있어도 화면이 달라지지 않게 한다. */
  const settings = editorSettings ?? DEFAULT_EDITOR_SETTINGS;

  const [gotoState, setGotoState] = useState<GotoState | null>(null);
  const [references, setReferences] = useState<LspTarget[] | null>(null);
  const gotoRef = useRef(onGoto);
  gotoRef.current = onGoto;
  const openTargetRef = useRef(onOpenTarget);
  openTargetRef.current = onOpenTarget;
  const navigateTargetRef = useRef(onNavigateTarget);
  navigateTargetRef.current = onNavigateTarget;
  const referencesRef = useRef(onReferences);
  referencesRef.current = onReferences;
  const beginReferencesRef = useRef(beginReferences);
  beginReferencesRef.current = beginReferences;
  const sourceVersionRef = useRef(sourceVersion);
  sourceVersionRef.current = sourceVersion;
  const navigationScopeRef = useRef(navigationScope);
  navigationScopeRef.current = navigationScope;
  const gotoRequestRef = useRef(0);
  const providerRequestRef = useRef(0);
  const revealRef = useRef(reveal);
  revealRef.current = reveal;
  const revealedRef = useRef(onRevealed);
  revealedRef.current = onRevealed;

  const navigateTarget = useCallback((target: LspTarget) => {
    const ed = editorRef.current;
    const path = activeRef.current;
    const position = ed?.getPosition();
    if (ed && path && position && navigateTargetRef.current) {
      navigateTargetRef.current(target, {
        path,
        line: position.lineNumber,
        column: position.column,
        scrollTop: ed.getScrollTop?.() ?? 0,
        scrollLeft: ed.getScrollLeft?.() ?? 0,
      });
      return;
    }
    void openTargetRef.current?.(target);
  }, []);

  /** App이 지시한 위치로 커서를 옮긴다. 대상 탭이 아직 활성이 아니면 아무것도 하지 않고,
   *  다음 렌더(탭 전환 후)에 다시 불린다. */
  const applyReveal = useCallback(() => {
    const target = revealRef.current;
    const ed = editorRef.current;
    if (!target || !ed || target.path !== activeRef.current) return;
    const model = ed.getModel();
    const line = Math.max(
      1,
      Math.min(target.line, model?.getLineCount() ?? target.line),
    );
    const column = Math.max(
      1,
      Math.min(target.column, model?.getLineMaxColumn(line) ?? target.column),
    );
    ed.setPosition({ lineNumber: line, column });
    if (target.restore && target.scrollTop != null && target.scrollLeft != null)
      ed.setScrollPosition({
        scrollTop: target.scrollTop,
        scrollLeft: target.scrollLeft,
      });
    else ed.revealLineInCenter(line);
    ed.focus();
    if (target.navId != null && target.epoch != null)
      revealedRef.current?.({
        navId: target.navId,
        epoch: target.epoch,
        path: activeRef.current ?? undefined,
        line: ed.getPosition()?.lineNumber,
        column: ed.getPosition()?.column,
        scrollTop: ed.getScrollTop?.(),
        scrollLeft: ed.getScrollLeft?.(),
      });
    else revealedRef.current?.();
  }, []);

  /** 고른 텍스트를 세션 컴포저에 붙인다 — Monaco든 마크다운 프리뷰든 여기로 모인다.
   *  두 화면이 각자 첨부하면 팝아웃 배달·포커스 규칙이 곧 갈라진다. */
  const attachLines = useCallback(
    (filePath: string, text: string, startLine: number, endLine: number) => {
      const taskId = taskIdRef.current;
      if (taskId == null) return;
      const record = buildSelectionCapture({
        taskId,
        filePath,
        text,
        startLine,
        endLine,
      });
      if (!record) return;
      const attach = attachCaptureRef.current;
      if (attach) {
        // 팝아웃 창 — 캡처는 메인 창으로 배달되고 포커스는 이 창에 남는다.
        attach(record);
        return;
      }
      pushCapture(taskId, record);
      requestComposerFocus(taskId);
    },
    [],
  );

  /** ⌘L — 드래그한 코드를 세션 컴포저에 첨부하고 입력창으로 포커스를 넘긴다.
   *  선택이 없으면 첨부할 것이 없으므로 아무 일도 하지 않는다(파일 전체 참조는 @멘션의 몫). */
  const attachSelection = useCallback(() => {
    const ed = editorRef.current;
    const path = activeRef.current;
    const selection = ed?.getSelection();
    if (!ed || !path || !selection || selection.isEmpty()) return;
    attachLines(
      path,
      ed.getModel()?.getValueInRange(selection) ?? "",
      selection.startLineNumber,
      selection.endLineNumber,
    );
  }, [attachLines]);
  const attachSelectionRef = useRef(attachSelection);
  attachSelectionRef.current = attachSelection;

  /** 드래그한 자리에 떠 있는 질문 버블. 선택이 없거나 공백뿐이면 null. */
  const [bubble, setBubble] = useState<BubbleAnchor | null>(null);
  const askRef = useRef(onAsk);
  askRef.current = onAsk;

  /** 선택 끝 위치를 화면 좌표로 옮긴다. 스크롤로 화면 밖에 나가면 버블을 거둔다. */
  const syncBubble = useCallback(() => {
    const ed = editorRef.current;
    if (!ed || !askRef.current) return setBubble(null);
    const selection = ed.getSelection();
    const text = selection
      ? (ed.getModel()?.getValueInRange(selection) ?? "")
      : "";
    if (!selection || selection.isEmpty() || !shouldShowBubble(text))
      return setBubble(null);
    const at = ed.getScrolledVisiblePosition({
      lineNumber: selection.endLineNumber,
      column: selection.endColumn,
    });
    const height = ed.getLayoutInfo().height;
    if (!at || at.top < 0 || at.top > height) return setBubble(null);
    setBubble({
      top: at.top + at.height + 4,
      left: at.left,
      placement: bubblePlacement(height - (at.top + at.height), 76),
      startLine: selection.startLineNumber,
      endLine: selection.endLineNumber,
    });
  }, []);

  /** 버블에서 온 질문 — 지금 선택된 범위를 그대로 실어 보낸다. */
  const askFromBubble = useCallback((question: string) => {
    const ed = editorRef.current;
    const path = activeRef.current;
    const selection = ed?.getSelection();
    if (!ed || !path || !selection || selection.isEmpty()) return;
    askRef.current?.({
      filePath: path,
      selectionText: ed.getModel()?.getValueInRange(selection) ?? "",
      startLine: selection.startLineNumber,
      endLine: selection.endLineNumber,
      question,
    });
  }, []);

  /**
   * ⌘B(정의) · ⌥⌘B(구현). 선언 위에서 ⌘B를 누르면 제자리 대신 사용처를 띄운다.
   *
   * `declarationOnly`는 ⌘클릭이 부르는 경로다. 다른 곳으로 가는 이동은 Monaco의 클릭
   * 동선이 이미 처리하므로, 여기서는 선언 위에서 누른 경우의 사용처 목록만 맡는다.
   */
  const runGoto = useCallback(async (
    kind: LspGotoKind,
    options: { at?: Monaco.IPosition; declarationOnly?: boolean } = {},
  ) => {
    const ed = editorRef.current;
    const path = activeRef.current;
    const goto = gotoRef.current;
    if (!ed || !path || !goto || filesRef.current.find((file) => file.path === path)?.readOnly) return;
    const pos = options.at ?? ed.getPosition();
    if (!pos) return;
    const declarationOnly = options.declarationOnly === true;
    const version = sourceVersionRef.current?.(path) ?? 0;
    const scope = navigationScopeRef.current;
    // ⌘클릭 보조 조회는 사용처로 넘어가기로 정해지기 전까지 번호를 올리지 않는다. 먼저 올리면
    // 진행 중인 ⌘B가 결과를 버리고, 상태를 쓰지 않는 이 조회 탓에 "조회 중"이 남는다.
    let request = declarationOnly ? gotoRequestRef.current : ++gotoRequestRef.current;

    const referenceRequest = kind === "references" ? (beginReferencesRef.current?.() ?? 0) : null;
    const ask = (k: LspGotoKind) =>
      goto({
        kind: k,
        path,
        text: ed.getValue(),
        line: pos.lineNumber,
        column: pos.column,
      });

    if (!declarationOnly) setGotoState({ phase: "loading", label: GOTO_LABEL[kind] });
    try {
      let effective = kind;
      let targets = await ask(kind);
      if (
        kind === "definition" &&
        shouldFallbackToReferences(targets, { path, line: pos.lineNumber })
      ) {
        if (declarationOnly) {
          if (gotoRequestRef.current !== request) return;
          request = ++gotoRequestRef.current;
        }
        effective = "references";
        targets = await ask("references");
      } else if (declarationOnly) return;
      if (
        navigationScopeRef.current !== scope ||
        gotoRequestRef.current !== request ||
        (sourceVersionRef.current?.(path) ?? 0) !== version
      )
        return;
      const label = GOTO_LABEL[effective];
      const outcome = resolveOutcome(targets);
      if (effective === "references") {
        setGotoState(null);
        const results = dedupeTargets(targets);
        const request = referenceRequest ?? (beginReferencesRef.current?.() ?? 0);
        if (referencesRef.current)
          referencesRef.current(results, path, request, {
            path,
            line: pos.lineNumber,
            column: pos.column,
            scrollTop: ed.getScrollTop?.() ?? 0,
            scrollLeft: ed.getScrollLeft?.() ?? 0,
          });
        else setReferences(results);
        return;
      }
      if (outcome.kind === "none") {
        setGotoState({ phase: "empty", label });
        return;
      }
      if (outcome.kind === "choose") {
        setGotoState({ phase: "choose", label, targets: outcome.targets });
        return;
      }
      setGotoState(null);
      navigateTarget(outcome.target);
    } catch (e) {
      // ⌘클릭은 Monaco의 이동과 함께 도는 보조 조회다 — 여기서 실패를 띄우면 정상 이동 위에 배너가 겹친다.
      if (declarationOnly) return;
      setGotoState({
        phase: "error",
        label: GOTO_LABEL[kind],
        message: String(e),
      });
    }
  }, []);
  const runGotoRef = useRef(runGoto);
  runGotoRef.current = runGoto;

  /** Monaco에 넘긴 결과 위치(URI) → 원본 LspTarget. 다른 파일로 점프할 때 되찾는다. */
  const uriTargetsRef = useRef(new Map<string, LspTarget>());
  const disposablesRef = useRef<{ dispose: () => void }[]>([]);

  /**
   * LSP를 Monaco의 **언어 기능**으로 등록한다 — ⌘클릭, ⌘를 누른 채 hover했을 때의 밑줄,
   * F12, Peek(⌥F12)가 여기서 따라온다.
   *
   * 사용처는 등록하지 않는다. Monaco의 사용처 UI는 결과가 한 건이면 목록 없이 바로 이동하고,
   * 여러 건이면 열려 있지 않은 파일의 미리보기를 만들지 못한다. 프로바이더가 없으면 우클릭
   * 메뉴의 Monaco 항목도 숨으므로, 사용처 요청은 모두 앱의 사용처 목록(runGoto)으로 모은다.
   *
   * 키바인딩을 하나씩 흉내 내는 대신 Monaco가 이미 가진 동선에 백엔드를 꽂는 쪽을 택했다.
   * 대신 Monaco가 결과를 URI로만 다루므로, 워크트리 밖 위치나 아직 열지 않은 파일로 가는
   * 마지막 한 걸음은 editorOpener로 가로채 App의 탭 열기에 넘긴다.
   */
  const registerLanguageFeatures = useCallback(
    (m: MonacoInst) => {
      if (disposablesRef.current.length > 0) return; // 재마운트 시 중복 등록 방지.

      // 워크트리 안이면 탭 모델과 **같은 URI 규약**을 써야 Monaco가 "이미 열린 파일"로 본다.
      const toLocation = (t: LspTarget) => {
        const uri =
          t.external || t.path == null
            ? m.Uri.file(t.abs_path)
            : m.Uri.parse(t.path);
        uriTargetsRef.current.set(uri.toString(), t);
        return { uri, range: new m.Range(t.line, t.column, t.line, t.column) };
      };

      const pathOfModel = (model: Monaco.editor.ITextModel) =>
        filesRef.current.find(
          (f) => m.Uri.parse(f.path).toString() === model.uri.toString(),
        )?.path ?? null;

      const query = async (
        model: Monaco.editor.ITextModel,
        position: Monaco.Position,
        kind: LspGotoKind,
      ) => {
        const path = pathOfModel(model);
        const goto = gotoRef.current;
        if (!path || !goto || !ownsGoto(token, path)) return [];
        const version = sourceVersionRef.current?.(path) ?? 0;
        const scope = navigationScopeRef.current;
        const request = ++providerRequestRef.current;
        try {
          const targets = await goto({
            kind,
            path,
            text: model.getValue(),
            line: position.lineNumber,
            column: position.column,
          });
          if (
            navigationScopeRef.current !== scope ||
            providerRequestRef.current !== request ||
            (sourceVersionRef.current?.(path) ?? 0) !== version
          )
            return [];
          return dedupeTargets(targets).map(toLocation);
        } catch {
          // hover마다 배너를 띄울 수는 없다 — 원인 표시는 ⌘B(GotoOverlay)가 맡는다.
          return [];
        }
      };

      // 지원 언어에만 건다. 백엔드가 모르는 형식은 어차피 에러로 떨어져 빈 결과가 된다.
      const languages = ["typescript", "javascript", "rust", "python", "java"];
      for (const language of languages) {
        disposablesRef.current.push(
          m.languages.registerDefinitionProvider(language, {
            provideDefinition: (
              model: Monaco.editor.ITextModel,
              position: Monaco.Position,
            ) => query(model, position, "definition"),
          }),
          m.languages.registerImplementationProvider(language, {
            provideImplementation: (
              model: Monaco.editor.ITextModel,
              position: Monaco.Position,
            ) => query(model, position, "implementation"),
          }),
        );
      }

      // 다른 파일로의 착지 — 모델이 없는 파일도 여기서 App의 탭 열기로 넘어간다.
      if (typeof m.editor.registerEditorOpener === "function") {
        disposablesRef.current.push(
          m.editor.registerEditorOpener({
            openCodeEditor: (
              _source: Monaco.editor.ICodeEditor,
              resource: Monaco.Uri,
            ) => {
              const target = uriTargetsRef.current.get(resource.toString());
              if (!target || !openTargetRef.current) return false;
              openTargetRef.current(target);
              return true;
            },
          }),
        );
      }
    },
    [token],
  );

  // 언어 기능 등록은 전역이다 — 이 창이 사라지면 반드시 걷어낸다.
  useEffect(
    () => () => {
      disposablesRef.current.forEach((d) => d.dispose());
      disposablesRef.current = [];
      uriTargetsRef.current.clear();
    },
    [],
  );

  const handleMount = useCallback<OnMount>(
    (ed, m) => {
      editorRef.current = ed;
      monacoRef.current = m;
      registerLanguageFeatures(m);
      ed.addCommand(m.KeyMod.CtrlCmd | m.KeyCode.KeyS, () => {
        const p = activeRef.current;
        if (p) saveRef.current(p, ed.getValue());
      });
      // Monaco 커맨드로 등록하면 에디터에 포커스가 있을 때만 발동하고, 그때 Monaco가
      // 이벤트를 소비하므로 App의 전역 ⌘B(파일 트리)·⌥⌘B(사이드바)까지 올라가지 않는다.
      ed.addCommand(m.KeyMod.CtrlCmd | m.KeyCode.KeyB, () => {
        void runGotoRef.current("definition");
      });
      ed.addCommand(m.KeyMod.CtrlCmd | m.KeyMod.Alt | m.KeyCode.KeyB, () => {
        void runGotoRef.current("implementation");
      });
      ed.addCommand(m.KeyMod.Alt | m.KeyCode.KeyB, () => {
        void runGotoRef.current("references");
      });
      // 참조 찾기 — VS Code·IntelliJ 공통 바인딩(⇧F12). 우클릭 메뉴 항목도 같은 액션이다.
      ed.addAction({
        id: "praxis.findReferences",
        label: "사용처 보기",
        keybindings: [m.KeyMod.Shift | m.KeyCode.F12],
        contextMenuGroupId: "navigation",
        contextMenuOrder: 1.45,
        run: () => void runGotoRef.current("references"),
      });
      // 선언 위 ⌘클릭 → 사용처 목록. 다른 곳으로 가는 이동은 Monaco의 클릭 동선이 그대로 맡는다.
      // Monaco처럼 누른 자리에서 뗐을 때만 클릭으로 본다 — ⌘-드래그는 선택이다.
      let pressed: Monaco.IPosition | null = null;
      const clickAt = (e: Monaco.editor.IEditorMouseEvent) => {
        const trigger = isMacPlatform() ? e.event.metaKey : e.event.ctrlKey;
        if (!trigger || e.target.type !== m.editor.MouseTargetType.CONTENT_TEXT) return null;
        return e.target.position;
      };
      ed.onMouseDown((e) => {
        pressed = e.event.leftButton ? clickAt(e) : null;
      });
      ed.onMouseUp((e) => {
        const at = clickAt(e);
        const from = pressed;
        pressed = null;
        if (at && from && at.lineNumber === from.lineNumber && at.column === from.column)
          void runGotoRef.current("definition", { at, declarationOnly: true });
      });
      ed.addCommand(m.KeyMod.Alt | m.KeyCode.F7, () => {
        void runGotoRef.current("references");
      });
      // ⌘L은 Monaco 기본 expandLineSelection을 덮는다 — 커맨드로 등록해야 확실히 우선한다.
      ed.addCommand(m.KeyMod.CtrlCmd | m.KeyCode.KeyL, () => {
        attachSelectionRef.current();
      });
      // ⇧⏎ — Python 파일의 선택 영역(없으면 커서 줄)을 콘솔로 보낸다. Jupyter·VS Code의
      // "Run Selection/Line in Interactive Window"와 같은 손동작. 컨텍스트 키로 게이트해
      // 다른 언어·콘솔 없는 창에서는 Monaco 기본 동작(줄 바꿈)이 그대로 남는다.
      replContextRef.current = ed.createContextKey<boolean>(REPL_CONTEXT_KEY, replEnabledRef.current);
      ed.addCommand(
        m.KeyMod.Shift | m.KeyCode.Enter,
        () => {
          const run = runSelectionRef.current;
          const model = ed.getModel();
          const sel = ed.getSelection();
          if (!run || !model || !sel) return;
          const code = sel.isEmpty()
            ? model.getLineContent(sel.positionLineNumber)
            : model.getValueInRange(sel);
          if (code.trim()) run(code);
          // 줄 실행이면 다음 줄로 내려간다 — 연속 ⇧⏎로 한 줄씩 흘려보내는 흐름.
          if (sel.isEmpty() && sel.positionLineNumber < model.getLineCount()) {
            ed.setPosition({ lineNumber: sel.positionLineNumber + 1, column: 1 });
          }
        },
        REPL_CONTEXT_KEY,
      );
      // 드래그가 끝난 자리에 질문 버블을 띄운다. 스크롤하면 따라가고, 화면 밖으로 나가면 거둔다.
      ed.onDidChangeCursorSelection(() => syncBubble());
      ed.onDidScrollChange(() => syncBubble());
      applyReveal(); // 새 탭이 이번 마운트로 열렸다면 여기서 착지한다.
    },
    [applyReveal, syncBubble],
  );

  useEffect(applyReveal, [reveal, activeKey, applyReveal]);

  /**
   * 마크다운 프리뷰의 ⌘L.
   *
   * Monaco 화면에서는 에디터 커맨드로 걸지만(`addCommand`) 프리뷰에는 걸 자리가 없다 —
   * 렌더된 DOM일 뿐이다. 그래서 창에서 듣되 **선택이 이 문서 안에 있을 때만** 가져간다.
   * 줄 번호는 렌더할 때 블록에 심어 둔 표식에서 되찾는다(`lib/md-selection`).
   */
  useEffect(() => {
    // HTML 프리뷰는 제외한다 — 샌드박스 iframe 안의 선택은 부모가 읽을 수 없고 줄 표식도 없다.
    if (!showDoc || !activeIsMarkdown) return;
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || !(event.metaKey || event.ctrlKey)) return;
      if (event.altKey || event.shiftKey || event.code !== "KeyL") return;
      const host = docRef.current;
      const path = activeRef.current;
      const selection = window.getSelection();
      if (host == null || path == null) return;
      if (
        selection == null ||
        selection.rangeCount === 0 ||
        selection.isCollapsed
      )
        return;
      const range = selection.getRangeAt(0);
      if (!host.contains(range.commonAncestorContainer)) return;
      const lines = linesOfRange(range);
      if (lines == null) return;
      event.preventDefault();
      attachLines(path, range.toString(), lines.startLine, lines.endLine);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showDoc, activeIsMarkdown, attachLines]);

  // 파일을 바꾸면 이전 파일의 결과 패널은 의미가 없다. 버블도 같이 거둔다 —
  // 다른 파일의 선택을 물고 있으면 엉뚱한 코드를 보내게 된다.
  useEffect(() => {
    setGotoState(null);
    if (!referencesRef.current) setReferences(null);
    setBubble(null);
  }, [activeKey]);

  // "왜 ⌘클릭이 안 되는가"에 답할 수 있어야 한다 — 서버 미설치인지, 애초에 지원 언어가
  // 아닌지를 파일마다 물어 배지로 남긴다.
  // diff 탭은 파일을 편집하는 자리가 아니다 — 키로 물으면 `diff:` 접두가 그대로 IPC로 나간다.
  useEffect(() => {
    const request = ++lspRequestRef.current;
    if (!onLspStatus || !lspPath) {
      setLspState({
        scopeKey: lspScopeKey,
        path: lspPath,
        state: "idle",
        value: null,
      });
      return;
    }
    setLspState({
      scopeKey: lspScopeKey,
      path: lspPath,
      state: "loading",
      value: null,
    });
    onLspStatus(lspPath)
      .then((info) => {
        if (request !== lspRequestRef.current) return;
        setLspState({
          scopeKey: lspScopeKey,
          path: lspPath,
          state: "ready",
          value: info,
        });
      })
      .catch(() => {
        if (request !== lspRequestRef.current) return;
        setLspState({
          scopeKey: lspScopeKey,
          path: lspPath,
          state: "error",
          value: null,
        });
      });
    return () => {
      if (request === lspRequestRef.current) lspRequestRef.current += 1;
    };
  }, [lspPath, lspScopeKey, onLspStatus]);

  /**
   * 시맨틱 토큰 프로바이더 — **활성 파일의 언어에 대해, legend를 받아 본 뒤에** 건다.
   *
   * Monarch(정규식 렉서)는 `foo`가 변수인지 함수인지 클래스인지 구조적으로 알 수 없다.
   * VS Code가 그것을 아는 것은 언어 서버의 시맨틱 토큰 덕이고, 여기가 그 통로다.
   *
   * 두 가지를 지킨다.
   * - **legend가 없으면 등록하지 않는다.** 빈 프로바이더를 걸면 Monaco가 Monarch 결과까지
   *   덮어 색이 오히려 사라진다 — 언어 서버가 없는 환경에서 화면이 더 나빠진다.
   * - **활성 파일이 정해진 뒤에 묻는다.** 마운트 시점에는 모델이 없을 수 있고, legend는
   *   서버(=언어)마다 다르므로 물어볼 대상 파일이 있어야 한다.
   */
  useEffect(() => {
    const m = monacoRef.current;
    if (!m || !active || active.kind !== "text" || active.readOnly) return;
    const language = langFromPath(active.path);
    if (semanticLangsRef.current.has(language)) return;

    let cancelled = false;
    const path = active.path;
    void (async () => {
      try {
        const taskId = taskIdRef.current;
        if (taskId == null) return;
        const first = await lspSemanticTokens(
          taskId,
          path,
          active.content ?? "",
        );
        if (cancelled || !first || first.legend.token_types.length === 0)
          return;
        if (semanticLangsRef.current.has(language)) return;
        semanticLangsRef.current.add(language);
        const legend = {
          tokenTypes: first.legend.token_types,
          tokenModifiers: first.legend.token_modifiers,
        };
        disposablesRef.current.push(
          m.languages.registerDocumentSemanticTokensProvider(language, {
            getLegend: () => legend,
            provideDocumentSemanticTokens: async (
              model: Monaco.editor.ITextModel,
            ) => {
              const file = filesRef.current.find(
                (f) => m.Uri.parse(f.path).toString() === model.uri.toString(),
              );
              if (!file || file.readOnly) return null;
              try {
                const nextTaskId = taskIdRef.current;
                if (nextTaskId == null) return null;
                const res = await lspSemanticTokens(
                  nextTaskId,
                  file.path,
                  model.getValue(),
                );
                return res
                  ? { data: new Uint32Array(res.data), resultId: undefined }
                  : null;
              } catch {
                // 서버가 죽었거나 파일이 지원 밖 — Monarch 색으로 후퇴한다.
                return null;
              }
            },
            releaseDocumentSemanticTokens: () => {},
          }),
        );
      } catch {
        // 조회 실패는 조용히 넘어간다. 색이 없는 것이 화면이 깨지는 것보다 낫다.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [active?.path, active?.kind]);

  // 모델 누수 방지: 닫힌 파일의 Monaco 모델을 dispose.
  //
  // "닫혔다"의 기준은 이 그룹이 아니라 **전 그룹**(retainedPaths)이다 — 같은 파일을 두 칸이
  // 띄우면 모델은 URI 하나를 공유하므로, 한쪽 탭을 닫았다고 버리면 옆 칸이 죽은 모델을 문다.
  useEffect(() => {
    const m = monacoRef.current;
    if (!m) return;
    const want = new Set(retainedPaths.map((p) => m.Uri.parse(p).toString()));
    files
      .filter((f) => f.kind === "text")
      .forEach((f) => ownedRef.current.add(m.Uri.parse(f.path).toString()));
    for (const model of m.editor.getModels()) {
      const id = model.uri.toString();
      if (ownedRef.current.has(id) && !want.has(id)) {
        model.dispose();
        ownedRef.current.delete(id);
      }
    }
  }, [files, retainedPaths]);

  // bounds는 스크린샷 캡처에만 필요하다 — 화면 밖이라 계산할 수 없어도 선택 텍스트(⌘L)는
  // 그대로 첨부할 수 있어야 하므로 null로 넘기고 판단은 읽는 쪽에 맡긴다.
  useEffect(() => {
    // 캡처 목적지는 taskId당 한 자리다. 칸마다 등록하면 마지막 칸이 자리를 덮고, 그 칸을
    // 접는 순간 자리가 통째로 비어 캡처가 소리 없이 죽는다 — 보고 있는 칸만 등록한다.
    if (!focused) return;
    if (taskId == null) return;
    return registerEditorCaptureTarget(taskId, () => {
      // diff 탭에는 캡처할 에디터가 없다. 그래도 경로를 돌려주면 그 파일의 에디터 화면인
      // 것처럼 귀속되므로 자리를 비운다.
      if (!active || active.kind !== "text" || !paneRef.current) return null;
      const bounds =
        previewBoundsFromRect(paneRef.current.getBoundingClientRect()) ?? null;
      const selection = showEditor ? editorRef.current?.getSelection() : null;
      const hasSelection = selection != null && !selection.isEmpty();
      return {
        bounds,
        file_path: active.path,
        selection_text: hasSelection
          ? (editorRef.current?.getModel()?.getValueInRange(selection) ?? null)
          : null,
        selection_start_line: hasSelection ? selection.startLineNumber : null,
        selection_end_line: hasSelection ? selection.endLineNumber : null,
      };
    });
  }, [active, focused, showEditor, taskId]);

  const saveActive = () => {
    if (!active) return;
    // Monaco가 없는 화면(프리뷰)에서 `editorRef`는 언마운트된 옛 인스턴스를 계속 문다 —
    // dispose된 에디터의 `getValue()`는 빈 문자열이라 그대로 저장하면 파일이 날아간다.
    onSave(
      active.path,
      showEditor
        ? (editorRef.current?.getValue() ?? active.content)
        : active.content,
    );
  };

  return (
    <div
      ref={paneRef}
      className="relative flex-1 flex flex-col min-h-0 min-w-0"
    >
      {/* 탭 바 */}
      <div className="h-8 flex items-stretch bg-raised border-b border-border overflow-x-auto shrink-0">
        {files.map((f, i) => (
          <div
            key={f.key}
            data-tab-key={f.key}
            data-tab-path={f.path}
            // 프리뷰 탭은 기울여 그린다 — VS Code의 관례이고, "이 자리는 곧 다른 파일에게
            // 넘어간다"를 탭 하나로 말할 다른 채널이 없다.
            className={`group flex items-center gap-2 pl-3 pr-2 text-sm border-r border-border cursor-pointer whitespace-nowrap ${
              f.key === activeKey
                ? "bg-bg text-text"
                : "text-text-secondary hover:text-text"
            } ${f.preview ? "italic" : ""}`}
            onClick={() => onSelect(f.key)}
            onDoubleClick={() => onPinTab?.(f.key)}
            onContextMenu={(event) => {
              if (onTabMenu == null) return;
              event.preventDefault();
              onTabMenu(f.key, event.clientX, event.clientY);
            }}
            // 앞 아홉 탭에는 번호를 일러 준다 — 단축키가 있다는 사실 자체가 여기 말고는
            // 드러날 자리가 없다. 열 번째부터는 짚을 번호가 없으므로 경로만 남는다.
            title={`${f.kind === "diff" ? `${f.path} — 변경분` : f.path}${i < 9 ? `\n⌘${i + 1}` : ""}${
              f.preview ? "\n미리보기 — 더블클릭하면 고정" : ""
            }`}
            // 탭을 끌어 옆·아래에 놓으면 분할이 된다(EditorSplitView가 받는다). 분할을 모르는
            // 호출부에서는 손잡이가 없으므로 끌리지도 않는다.
            draggable={onTabDragStart != null}
            onDragStart={(event) => onTabDragStart?.(f.key, event)}
          >
            <span
              className={
                f.key === activeKey ? "text-primary-bright" : "text-text-muted"
              }
            >
              <Icon
                name={f.kind === "diff" ? "diff" : fileIcon(base(f.path))}
                size={13}
              />
            </span>
            <span>{base(f.path)}</span>
            {f.dirty && (
              <span
                className="text-primary-bright text-xs"
                aria-label="저장 안 됨"
              >
                ●
              </span>
            )}
            <button
              className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100 text-text-muted hover:text-text"
              onClick={(e) => {
                e.stopPropagation();
                onClose(f.key);
              }}
              aria-label="닫기"
            >
              <Icon name="x" size={13} />
            </button>
          </div>
        ))}
        {active && (
          <div className="ml-auto flex items-center gap-1 px-2 shrink-0">
            {active.readOnly && (
              <span className="text-xs text-text-muted" role="status">읽기 전용</span>
            )}
            {/* 언어 서버 상태 — 정의 이동이 되는 파일인지 눈으로 확인되는 자리. */}
            {lspInfo?.server && (
              <span
                className={`flex items-center gap-1 px-1 text-xs ${
                  lspInfo.available ? "text-text-muted" : "text-status-awaiting"
                }`}
                title={
                  lspInfo.available
                    ? `${lspInfo.server} — ⌘클릭·F12로 정의, ⌥⌘B로 구현, ⇧F12·선언 위 ⌘클릭으로 사용처`
                    : (lspInfo.detail ?? "정의 이동을 쓸 수 없습니다")
                }
                aria-label={
                  lspInfo.available ? "언어 서버 연결됨" : "언어 서버 사용 불가"
                }
              >
                <Icon name="plug" size={13} />
                {!lspInfo.available && <span>정의 이동 불가</span>}
              </span>
            )}
            {/* 마크다운·HTML: 프리뷰 ⇄ 소스 토글 */}
            {activeIsRendered && (
              <button
                className={`h-6 px-2 rounded text-xs ${
                  showSource
                    ? "text-text-muted hover:text-text"
                    : "bg-bg text-text"
                }`}
                onClick={() =>
                  setSourceMode((m) => ({
                    ...m,
                    [active.path]: !(m[active.path] ?? false),
                  }))
                }
                title={showSource ? "프리뷰로 보기" : "소스 편집"}
              >
                {showSource ? "프리뷰" : "소스"}
              </button>
            )}
            {/* 기본 앱 / Finder 로 열기 — 모든 파일에 제공(바이너리·이미지·대용량은 유일한 열람 수단). */}
            {supportsExternalPath && (
              <button
                className="text-text-muted hover:text-text p-1"
                onClick={() => onOpenPath(active.path)}
                title="기본 앱으로 열기"
                aria-label="기본 앱으로 열기"
              >
                <Icon name="desktop" size={15} />
              </button>
            )}
            {supportsExternalPath && (
              <button
                className="text-text-muted hover:text-text p-1"
                onClick={() => onRevealPath(active.path)}
                title="Finder에서 보기"
                aria-label="Finder에서 보기"
              >
                <Icon name="folder" size={15} />
              </button>
            )}
            {/* 다시 불러오기·저장은 디스크 내용을 가진 탭의 것이다 — diff 탭에는 없다. */}
            {active.kind !== "diff" && (
              <button
                className="text-text-muted hover:text-text p-1"
                onClick={() => onReload(active.path)}
                title="디스크에서 다시 불러오기"
                aria-label="다시 불러오기"
              >
                <Icon name="refresh" size={15} />
              </button>
            )}
            {/* 저장은 텍스트 편집 대상에만 (이미지/바이너리/대용량은 편집 불가). */}
            {active.kind === "text" && (
              <button
                className={`p-1 ${active.dirty ? "text-primary-bright" : "text-text-muted hover:text-text"}`}
                onClick={saveActive}
                title="저장 (⌘S)"
                aria-label="저장"
              >
                <Icon name="save" size={15} />
              </button>
            )}
            {/* 분할 — 파일이 아니라 **이 칸**에 거는 동작이라 파일 손잡이 뒤에 둔다. */}
            {onSplit && (
              <>
                <button
                  className="text-text-muted hover:text-text p-1"
                  onClick={() => onSplit("row")}
                  title="오른쪽으로 분할 (⌘\\)"
                  aria-label="오른쪽으로 분할"
                >
                  <Icon name="splitRight" size={15} />
                </button>
                <button
                  className="text-text-muted hover:text-text p-1"
                  onClick={() => onSplit("column")}
                  title="아래로 분할 (⇧⌘\\)"
                  aria-label="아래로 분할"
                >
                  <Icon name="splitDown" size={15} />
                </button>
              </>
            )}
            {onCloseGroup && (
              <button
                className="text-text-muted hover:text-text p-1"
                onClick={onCloseGroup}
                title="이 칸 접기 — 열린 탭은 옆 칸으로 옮겨진다"
                aria-label="편집 칸 접기"
              >
                <Icon name="x" size={15} />
              </button>
            )}
          </div>
        )}
      </div>

      {references && !onReferences && (
        <ReferencesPanel
          targets={references}
          onOpen={navigateTarget}
          onClose={() => setReferences(null)}
        />
      )}

      {/* 에디터 본문 — 종류별 분기 */}
      {!active ? (
        <div className="flex-1 flex items-center justify-center text-text-muted text-sm">
          좌측 파일 트리에서 파일을 선택하세요
        </div>
      ) : showEditor ? (
        <div className="flex-1 min-h-0 relative">
          <Editor
            path={active.path}
            value={active.content}
            /* 언마운트가 모델을 dispose하지 않게 막는다(@monaco-editor/react 기본값은 dispose).
               분할에서 옆 칸이 같은 모델을 보고 있을 수 있고, 수명은 위의 GC 하나가 소유한다. */
            keepCurrentModel

            language={langFromPath(active.path)}
            theme={themeId}
            onMount={handleMount}
            onChange={(v) => onChange(active.path, v ?? "")}
            options={{
              fontSize: codeFontSize,
              fontFamily: codeFontFamily,
              minimap: { enabled: settings.minimap },
              scrollBeyondLastLine: false,
              automaticLayout: true,
              tabSize: settings.tab_size,
              // Monaco 는 "off" | "on" | "wordWrapColumn" | "bounded" 를 받는다.
              wordWrap: settings.word_wrap ? "on" : "off",
              renderWhitespace: "selection",
              // ⌘클릭을 정의 이동에 내준다 — 멀티커서는 ⌥클릭으로(VS Code 기본과 동일).
              multiCursorModifier: "alt",
              // 선언 위 ⌘클릭의 대체 동작(Monaco 사용처 UI)을 끈다 — 사용처는 앱 목록이 띄운다.
              gotoLocation: { alternativeDefinitionCommand: "" },
              // 에디터로서의 기본기 — 들여쓰기 가이드, 활성 줄 강조, 괄호 짝 색.
              guides: { indentation: true, bracketPairs: true },
              renderLineHighlight: "all",
              bracketPairColorization: { enabled: true },
              stickyScroll: { enabled: true },
              smoothScrolling: true,
              readOnly: active.readOnly === true,
              cursorBlinking: "smooth",
              padding: { top: 6 },
            }}
          />
          {gotoState && (
            <GotoOverlay
              state={gotoState}
              onPick={(target) => {
                setGotoState(null);
                navigateTarget(target);
              }}
              onClose={() => setGotoState(null)}
            />
          )}
          {bubble && onAsk && (
            <SelectionAskBubble
              anchor={bubble}
              busy={askBusy}
              error={askError}
              onSubmit={askFromBubble}
              onAskSeparately={onAskSeparately ? () => {
                const editor = editorRef.current;
                const selection = editor?.getSelection();
                const path = activeRef.current;
                if (!editor || !selection || selection.isEmpty() || !path) return;
                onAskSeparately({
                  filePath: path,
                  selectionText: editor.getModel()?.getValueInRange(selection) ?? "",
                  startLine: selection.startLineNumber,
                  endLine: selection.endLineNumber,
                });
                setBubble(null);
              } : undefined}
              onAttachOnly={() => {
                attachSelectionRef.current();
                setBubble(null);
              }}
              onClose={() => setBubble(null)}
            />
          )}
        </div>
      ) : active.kind === "text" && activeIsHtml ? (
        // HTML 프리뷰 — iframe이 자체 스크롤을 가지므로 바깥에 overflow·패딩을 두지 않는다.
        <div className="flex-1 min-h-0">
          <HtmlDoc html={active.content} title={base(active.path)} retainKeyboardFocus={shortcutsActive} />
        </div>
      ) : active.kind === "text" ? (
        // 마크다운 프리뷰 (활성 마크다운 + 소스 모드 아님)
        <div ref={docRef} className="flex-1 min-h-0 overflow-auto px-6 py-4">
          <MarkdownDoc
            text={active.content}
            dark={dark}
            onLinkMenu={onLinkMenu ? openMarkdownLinkMenu : undefined}
            onOpenLink={onOpenLink ? openMarkdownLink : undefined}
          />
        </div>
      ) : active.kind === "table" ? (
        <TableView
          content={active.content}
          name={base(active.path)}
          onOpenInRepl={onOpenInRepl ? () => onOpenInRepl(active.path) : undefined}
        />
      ) : active.kind === "diff" ? (
        <DiffTab
          path={active.path}
          active={focused && shortcutsActive}
          preview={active.preview === true}
          onClose={() => onClose(active.key)}
        />
      ) : active.kind === "image" ? (
        <div className="flex-1 min-h-0 overflow-auto flex items-center justify-center p-4 bg-surface">
          <img
            src={active.content}
            alt={base(active.path)}
            className="max-w-full max-h-full object-contain"
          />
        </div>
      ) : (
        // binary / too_large — 미리보기 불가 폴백 패널
        <div className="flex-1 flex flex-col items-center justify-center gap-3 text-center px-6">
          <div className="text-text-muted">
            <Icon
              name={active.kind === "too_large" ? "database" : "code"}
              size={40}
            />
          </div>
          <div className="text-sm text-text-secondary max-w-md">
            {active.kind === "too_large"
              ? "이 파일은 미리보기하기에 너무 큽니다 (2MB 초과)."
              : "이 형식은 앱에서 미리보기할 수 없습니다."}
            <br />
            기본 앱이나 Finder에서 열어 확인하세요.
          </div>
          {supportsExternalPath && (
            <div className="flex items-center gap-2 mt-1">
              <button
                className="h-8 px-3 rounded-md bg-primary text-bg text-sm font-medium hover:opacity-90 flex items-center gap-1.5"
                onClick={() => onOpenPath(active.path)}
              >
                <Icon name="desktop" size={15} />
                기본 앱으로 열기
              </button>
              <button
                className="h-8 px-3 rounded border border-border text-text-secondary text-sm hover:text-text hover:border-border-strong flex items-center gap-1.5"
                onClick={() => onRevealPath(active.path)}
              >
                <Icon name="folder" size={15} />
                Finder에서 보기
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
