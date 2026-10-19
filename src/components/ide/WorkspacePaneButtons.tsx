import type { ReactElement } from "react";
import { Icon, type IconName } from "./icons";
import type { CodeTab } from "./CodeColumnTabs";

interface Props {
  /** 코드 열이 자리를 받고 있는지. */
  open: boolean;
  /** 열려 있을 때 보이는 탭. 닫혀 있어도 다음에 열릴 탭이다. */
  active: CodeTab;
  /**
   * 변경 렌즈 손잡이 — 트리 열의 `변경` 렌즈를 연다(ADR 2026-10-08 결정 3).
   * 코드 열 칩과 묶여 있지만 목적지는 트리 열이다: 변경 목록은 "무엇을 볼까"를 고르는 자리라
   * 파일 트리와 같은 열에 산다. `count`는 변경 파일 수 — 0 또는 미로드면 배지를 그리지 않는다.
   * 눌림(`on`)은 트리가 열려 있고 렌즈가 변경일 때다. 눌린 채 누르면 트리를 닫는다.
   */
  changes: { on: boolean; count: number | null; onToggle: () => void };
  /** 프리뷰는 로컬 작업에서만 열린다 — 원격에서는 버튼 자체를 만들지 않는다. */
  previewAvailable: boolean;
  designAvailable?: boolean;
  /** 그 탭으로 코드 열을 연다. */
  onOpen: (tab: CodeTab) => void;
  /** 보고 있던 탭을 다시 눌렀을 때 — 여는 버튼이 닫기도 겸한다(보조 경로). */
  onClose: () => void;
  /**
   * 에디터 팝아웃 손잡이 — 나가 있으면 그 창을 앞으로, 아니면 빼낸다.
   * 호출부가 하나뿐이라 선택 prop으로 두지 않는다.
   * 파일 버튼을 낼지도 이 `poppedOut` 하나로 판단한다 — 나가 있으면 파일은 여기 없다
   * (`CodeColumnTabs`와 같은 규칙). 같은 사실을 두 prop으로 받으면 어긋날 수 있다.
   * 칩은 코드 열 묶음 안 마지막 자리다 — 에디터 창은 코드 열의 파일 탭을 빼내는 목적지라
   * 같은 묶음이다. 라벨 없는 muted 아이콘 버튼이던 시절은 Don't #3 위반이었다.
   */
  popOut: { poppedOut: boolean; onToggle: () => void };
  /**
   * 하단 터미널 도크의 손잡이 — 코드 열 탭줄에 있던 중복 버튼을 여기로 합쳤다(Don't #11).
   * 원격 등 불가면 눌리지 않고 사유를 title로 말한다.
   */
  terminal: { available: boolean; open: boolean; reason?: string; onToggle: () => void };
  /**
   * 라벨을 보여 줄 폭인지(`PANE_LABELS_MIN_CENTER`). 좁으면 아이콘으로 접는다 —
   * 아이콘만으로는 folder/file, desktop이 목적지를 말하지 못해서 라벨이 기본이다.
   */
  showLabels: boolean;
  questionAvailable?: boolean;
}

const CODE_BUTTONS: { tab: CodeTab; label: string; icon: IconName }[] = [
  { tab: "file", label: "파일", icon: "file" },
  { tab: "preview", label: "프리뷰", icon: "desktop" },
  { tab: "design", label: "Design", icon: "code" },
];

/**
 * 소환 칩 한 개 — PaneChip 계약(DESIGN.md).
 *
 * 눌린 칩의 라벨이 곧 열리는 탭의 라벨이다. 활성은 색 단독이 아니라 teal wash 배경면 +
 * `primaryBright`로 표시한다 — 코드 열 활성 탭·사이드바 활성 nav와 같은 문법 하나다(Don't #13).
 * 기본색은 `text-secondary` — muted는 클릭 가능한 요소에 쓰지 않는다(Don't #3).
 *
 * 칩은 줄지 않고(`shrink-0`) 라벨은 접히지 않는다(`whitespace-nowrap`). 한글은 CJK라 어느
 * 글자 사이에서든 끊기므로, 헤더 폭이 모자라면 "파일"이 두 줄로 서서 세로로 읽힌다 —
 * 영문 `Diff`만 멀쩡해 보였던 이유다. 자리가 없으면 칩이 아니라 헤더 제목이 물러선다.
 */
function PaneChip(props: {
  on: boolean;
  label: string;
  icon: IconName;
  showLabel: boolean;
  title: string;
  ariaLabel: string;
  onClick: () => void;
  badge?: number | null;
  disabled?: boolean;
}): ReactElement {
  return (
    <button
      className={`flex shrink-0 items-center gap-1 whitespace-nowrap rounded px-1.5 py-1 text-xs disabled:cursor-not-allowed disabled:opacity-50 ${
        props.on
          ? "bg-primary/10 text-primary-bright"
          : "text-text-secondary hover:bg-raised hover:text-text"
      }`}
      onClick={props.onClick}
      disabled={props.disabled}
      title={props.title}
      aria-label={props.ariaLabel}
      aria-pressed={props.on}
    >
      <Icon name={props.icon} size={props.showLabel ? 14 : 16} />
      {props.showLabel && <span>{props.label}</span>}
      {props.badge != null && props.badge > 0 && (
        <span className="tabular-nums text-[10px] text-text-muted">{props.badge}</span>
      )}
    </button>
  );
}

/**
 * 세션 헤더의 패널 손잡이 줄 — 파일·프리뷰·변경·에디터 창·터미널.
 *
 * `변경` 칩은 코드 열 탭이 아니라 트리 열 렌즈의 손잡이다(ADR 2026-10-08). 코드 열의 `Diff`
 * 탭이던 시절에는 480px 열 하나가 목록만 받았다. 라벨은 목적지인 렌즈의 이름 `변경`을 따른다 —
 * 눌린 칩의 라벨이 곧 열리는 면의 라벨이다(ADR 0119).
 *
 * 트리 열 손잡이는 2026-09-09(ADR 0188)에 이 줄에서 뺐다. "파일" 칩이 코드 열 파일 탭을
 * 열면서 닫혀 있던 트리도 함께 여니 폴더 칩은 같은 목적지의 두 번째 문이었다(배선은 App).
 * 트리를 닫는 손잡이는 트리 헤더의 ✕와 ⌘B다.
 *
 * ADR 0112가 목적지마다 버튼을 갈랐지만 이름 없는 아이콘은 여전히 툴팁을 기다려야
 * 알 수 있었고, 같은 목적지가 헤더에선 그림·열리면 글자로 두 언어였다. 그래서 칩에
 * **라벨을 병기**한다 — 누른 칩의 이름이 곧 열리는 탭의 이름이다(ADR 0119).
 *
 * 열림 상태는 여전히 하나다(`open` + `active`). 보고 있던 탭을 다시 누르면 닫히지만
 * 이것은 보조 경로다 — 주 닫기는 코드 열 탭줄의 ✕에 있다.
 *
 * 터미널은 코드 열이 아니라 하단 도크에 살지만(설계 0044) 같은 줄에 둔다 — 사용자에게는
 * "세션 옆에 무엇을 더 띄울까"라는 하나의 선택이고, 자리가 다르다는 것은 구현 사정이다.
 * 코드 열 탭줄에 있던 같은 버튼은 2026-09-09에 지웠으므로 이 칩이 도크의 유일한 마우스
 * 손잡이다(docs/discovery/editor-tab-terminal-button-brief.md).
 *
 * 에디터 팝아웃도 같은 이유로 여기로 왔다. 코드 열 탭줄에 있을 때는 열을 먼저 열어야 손잡이가
 * 보였다 — 창을 빼내는 데 2단계가 필요했다. 여기서는 코드 열과 무관하게 상시 보인다.
 */
export function WorkspacePaneButtons({
  open,
  active,
  changes,
  previewAvailable,
  designAvailable = false,
  onOpen,
  onClose,
  popOut,
  terminal,
  showLabels,
  questionAvailable = false,
}: Props): ReactElement {
  const buttons = CODE_BUTTONS.filter(
    (b) => (b.tab !== "preview" || previewAvailable) && (b.tab !== "design" || (previewAvailable && designAvailable)) && (b.tab !== "file" || !popOut.poppedOut),
  );
  if (questionAvailable) buttons.push({ tab: "question", label: "따로 질문", icon: "chat" });
  return (
    <>
      <div className="flex shrink-0 items-center gap-0.5 rounded border border-border/60 px-0.5 py-0.5">
        {buttons.map(({ tab, label, icon }) => {
          const on = open && active === tab;
          return (
            <PaneChip
              key={tab}
              on={on}
              label={label}
              icon={icon}
              showLabel={showLabels}
              title={on ? `${label} 닫기 (⌥⌘S)` : `${label} 보기`}
              ariaLabel={on ? `${label} 닫기` : `${label} 보기`}
              onClick={() => (on ? onClose() : onOpen(tab))}
            />
          );
        })}
        <PaneChip
          on={changes.on}
          label="변경"
          icon="diff"
          showLabel={showLabels}
          badge={changes.count}
          title={changes.on ? "변경 목록 닫기" : "변경 목록 보기"}
          ariaLabel={changes.on ? "변경 닫기" : "변경 보기"}
          onClick={changes.onToggle}
        />
        <PaneChip
          on={popOut.poppedOut}
          label="에디터 창"
          icon="popout"
          showLabel={showLabels}
          title={popOut.poppedOut ? "코드 창 앞으로 (⌥⌘E)" : "에디터를 새 창으로 (⌥⌘E)"}
          ariaLabel={popOut.poppedOut ? "코드 창 앞으로" : "에디터를 새 창으로"}
          onClick={popOut.onToggle}
        />
      </div>
      <PaneChip
        on={terminal.open}
        label="터미널"
        icon="terminal"
        showLabel={showLabels}
        disabled={!terminal.available}
        title={
          terminal.available
            ? terminal.open
              ? "터미널 닫기 (⌃`)"
              : "터미널 열기 (⌃`)"
            : (terminal.reason ?? "터미널을 사용할 수 없습니다")
        }
        ariaLabel={
          terminal.available
            ? terminal.open
              ? "터미널 닫기"
              : "터미널 열기"
            : `터미널 사용 불가: ${terminal.reason ?? "현재 작업에서는 사용할 수 없습니다"}`
        }
        onClick={terminal.onToggle}
      />
    </>
  );
}
