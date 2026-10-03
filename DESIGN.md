---
name: Praxis
format: compact
description: >-
  Praxis 디자인 시스템 — Rust+Tauri AI 에이전트 오케스트레이션 데스크톱 IDE.
  "Chromatic Discipline": 절제된 다크, 보더 기반 위계, Teal 단일 신호.
  다크 테마 기본 + 라이트 테마 지원(CSS 변수 `--c-*`, `.dark` 토글, localStorage 지속; 터미널은 다크 유지). 밀도 기본 A(compact) + B(relaxed) 토글.
density:
  default: compact            # 변형 A — High-Density Operator
  toggle: relaxed             # 변형 B — Focused Review (data-density="relaxed")
colors:
  primary: "#14b8a6"          # teal-500 — CTA, focus ring, active tab, active nav
  primaryHover: "#0d9488"     # teal-600
  primaryActive: "#0f766e"    # teal-700
  primaryBright: "#2dd4bf"    # teal-400 — interactive icon, progress, link
  teal:
    "50": "#f0fdfa"
    "100": "#ccfbf1"
    "200": "#99f6e4"
    "300": "#5eead4"
    "400": "#2dd4bf"
    "500": "#14b8a6"
    "600": "#0d9488"
    "700": "#0f766e"
    "800": "#115e59"
    "900": "#134e4a"
  neutral:
    "50": "#fafafa"
    "100": "#f4f4f5"
    "200": "#e4e4e7"
    "300": "#d4d4d8"
    "400": "#a1a1aa"          # muted text, secondary icon
    "500": "#71717a"          # placeholder, disabled (non-interactive only)
    "600": "#52525b"
    "700": "#3f3f46"          # hover overlay on card / strong border
    "800": "#2a2a2a"          # BORDER
    "850": "#1f1f1f"          # popover / dropdown / modal bg
    "900": "#161616"          # CARD / PANEL / SIDEBAR
    "950": "#0d0d0d"          # APP BACKGROUND
  background: "#0d0d0d"
  surface: "#161616"
  surfaceRaised: "#1f1f1f"
  border: "#2a2a2a"
  borderStrong: "#3f3f46"
  text:
    primary: "#f4f4f5"        # 16.8:1 on bg
    secondary: "#a1a1aa"      # 5.9:1 on bg / 4.7:1 on card (AA)
    muted: "#71717a"          # 3.5:1 — non-interactive only (disabled/placeholder)
    inverse: "#0d0d0d"
    link: "#2dd4bf"
  status:
    running:  { bg: "#0c1929", border: "#1d4ed8", text: "#60a5fa" }   # 7.3:1
    awaiting: { bg: "#1c1407", border: "#b45309", text: "#fbbf24" }   # 9.5:1
    question: { bg: "#150c1f", border: "#7e22ce", text: "#c084fc" }   # 7.4:1 — 에이전트가 답을 기다림(검토 대기와 구별)
    done:     { bg: "#071a0f", border: "#15803d", text: "#4ade80" }   # 8.4:1
    failed:   { bg: "#1a0a0a", border: "#b91c1c", text: "#f87171" }   # 5.8:1
  role:
    planner: "#b7a47a"        # plan-board identity strip only
    researcher: "#64748b"     # explore-archive identity strip only
    implementer: "#14b8a6"    # build-pods identity strip only
    tester: "#b78aa5"         # test-bench identity strip only
    reviewer: "#8b7ad1"       # review-core identity strip only
typography:
  fontFamily:
    ui: "'Inter Variable', Inter, -apple-system, BlinkMacSystemFont, sans-serif"
    code: "'JetBrains Mono', 'Fira Code', ui-monospace, monospace"
  fontSize:
    xs: "11px"                # badge, aux label
    sm: "12px"                # caption, metadata, agent action text
    base: "13px"             # code / terminal (JetBrains Mono fixed)
    md: "14px"                # UI body default (Inter) — compact
    lg: "16px"                # section header, modal title
    xl: "20px"                # panel header
    "2xl": "24px"            # page title
  fontWeight:
    regular: 400
    medium: 500              # UI label, button
    semibold: 600            # section header
    bold: 700                # page title, emphasis
  lineHeight:
    tight: 1.25
    normal: 1.5
    relaxed: 1.625
    code: 1.6
spacing:
  "0": "0px"
  "0.5": "2px"
  "1": "4px"
  "1.5": "6px"
  "2": "8px"
  "3": "12px"
  "4": "16px"
  "5": "20px"
  "6": "24px"
  "8": "32px"
  "10": "40px"
  "12": "48px"
  "16": "64px"
rounded:
  none: "0px"
  sm: "4px"                  # badge, input, small button
  md: "6px"                  # button default, card inner
  lg: "8px"                  # card, panel, modal
  xl: "12px"                 # dialog, overlay
  full: "9999px"             # avatar, toggle, chip
elevation:
  "0": { bg: "{colors.background}", border: "none", shadow: "none" }
  "1": { bg: "{colors.surface}", border: "1px solid {colors.border}", shadow: "0 1px 3px rgba(0,0,0,0.4)" }
  "2": { bg: "{colors.surfaceRaised}", border: "1px solid {colors.border}", shadow: "0 4px 12px rgba(0,0,0,0.5), inset 0 1px 0 rgba(255,255,255,0.04)" }
  "3": { bg: "{colors.surfaceRaised}", border: "1px solid {colors.borderStrong}", shadow: "0 8px 32px rgba(0,0,0,0.6), inset 0 1px 0 rgba(255,255,255,0.05)" }
  "4": { bg: "{colors.surfaceRaised}", border: "1px solid {colors.borderStrong}", shadow: "0 16px 48px rgba(0,0,0,0.7), inset 0 1px 0 rgba(255,255,255,0.06)" }
components:
  Button:
    base: { height: "36px", rounded: "{rounded.md}", fontWeight: "{typography.fontWeight.medium}", fontSize: "{typography.fontSize.md}", focusVisible: "outline 2px solid {colors.primary}; outline-offset 2px" }
    primary: { default: { bg: "{colors.primary}", text: "{colors.text.inverse}" }, hover: { bg: "{colors.primaryHover}" }, active: { bg: "{colors.primaryActive}" }, disabled: { bg: "{colors.neutral.800}", text: "{colors.neutral.500}" } }
    outline: { default: { bg: "transparent", border: "1px solid {colors.primary}", text: "{colors.primaryBright}" }, hover: { bg: "rgba(20,184,166,0.10)" } }
    ghost: { default: { bg: "transparent", text: "{colors.text.secondary}" }, hover: { bg: "{colors.neutral.800}", text: "{colors.text.primary}" } }
    destructive: { default: { bg: "{colors.status.failed.bg}", border: "1px solid {colors.status.failed.border}", text: "{colors.status.failed.text}" } }
  IconButton:
    base: { icon: "16px", hitArea: "≥28px(icon + p-1.5)", rounded: "{rounded.sm}", text: "{colors.text.secondary}", focusVisible: "outline 2px solid {colors.primary}; outline-offset 2px" }
    hover: { text: "{colors.text.primary}", bg: "{colors.surfaceRaised}" }
    pressedOn: { text: "{colors.primaryBright}", bg: "rgba(20,184,166,0.10)", rule: "토글이면 aria-pressed 필수 — 눌림은 색이 아니라 배경면으로 만든다" }
    disabled: { text: "{colors.text.muted}" }
    scope: "헤더·툴바·카드 헤더의 아이콘 단독 버튼. 클릭 가능한 기본 상태에 {colors.text.muted} 금지(Don't #3) — muted는 disabled 전용"
  PaneChip:
    base: { fontSize: "{typography.fontSize.xs}", rounded: "{rounded.sm}", padding: "3px 6px", structure: "icon(14px) + label", text: "{colors.text.secondary}" }
    hover: { text: "{colors.text.primary}", bg: "{colors.surfaceRaised}" }
    active: { text: "{colors.primaryBright}", bg: "rgba(20,184,166,0.10)", rule: "눌린 칩의 라벨 = 열리는 탭의 라벨 — 같은 대상은 같은 언어로 부른다" }
    compact: { rule: "중앙 잔여 < 1080px이면 라벨을 접고 아이콘(16px)만 — 행 폭만 바뀌고 레이아웃 모드는 안 바뀌므로 히스테리시스는 두지 않는다" }
    group: { border: "1px solid {colors.border}", rounded: "{rounded.sm}", scope: "코드 열로 가는 칩(파일·프리뷰)만 묶는다 — 트리 열 손잡이(파일 트리·변경)와 터미널은 같은 줄, 묶음 밖. 목적지가 갈리면 묶음도 갈린다(ADR 0173이 개정한 ADR 0112)" }
  SideQuestion:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text.primary}"
    padding: "{spacing.4}"
  QuestionReference:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text.secondary}"
    rounded: "{rounded.sm}"
    padding: "{spacing.2}"
  Card:
    default: { bg: "{colors.surface}", border: "1px solid {colors.border}", rounded: "{rounded.lg}", padding: "{spacing.6}" }
    interactiveHover: { border: "1px solid {colors.neutral.700}", shadow: "{elevation.2.shadow}" }
    densityRelaxed: { padding: "{spacing.8}", shadow: "{elevation.2.shadow}" }
  Input:
    default: { bg: "{colors.surface}", border: "1px solid {colors.border}", text: "{colors.text.primary}", height: "36px", rounded: "{rounded.sm}" }
    focus: { border: "1px solid {colors.primary}", ring: "0 0 0 3px rgba(20,184,166,0.25)" }
    error: { border: "1px solid {colors.status.failed.border}", ring: "0 0 0 3px rgba(185,28,28,0.25)" }
    placeholder: { text: "{colors.text.muted}" }
  Scrollbar:
    base: { hitArea: "12px", thumb: "6px pill(rest), padding-box clip", track: "transparent — 에디터 레지스터는 거터를 그리지 않는다", corner: "transparent" }
    dark: { thumb: "rgba(161,161,170,0.30)", hover: "rgba(161,161,170,0.50)", active: "rgba(161,161,170,0.65)" }
    light: { thumb: "rgba(24,24,27,0.25)", hover: "rgba(24,24,27,0.40)", active: "rgba(24,24,27,0.55)" }
    interaction: { hover: "thumb 8px + hover 농도", active: "drag 시 10px + active 농도. Teal·상태색 금지 — 중립 농도 3단만" }
    scope: "네이티브(webkit)·Monaco slider 동일 규칙. SidePanel은 스크롤바 숨김 유지"
    impl: "::-webkit-scrollbar로만 구현 — 표준 scrollbar-width/scrollbar-color가 non-auto면 webkit 규칙이 전부 무시되어 WKWebView가 네이티브 흰 thumb로 폴백한다. color-scheme(light/.dark→dark)은 :root 토큰과 함께 선언"
  SelectChip:
    scope: "값 하나를 고르는 칩+팝오버 전부. Composer 툴바의 세션 오버라이드(레포·에이전트·모델·Effort)가 첫 사용처이고, 스킬 탭의 정렬 피커처럼 목록 위 단일 선택도 같은 문법을 쓴다 — 네이티브 <select>는 어디서도 쓰지 않는다"
    base: { fontSize: "{typography.fontSize.xs}", rounded: "{rounded.md}", padding: "4px 8px", border: "1px solid {colors.border}", text: "{colors.text.secondary}", structure: "icon(13px) + label + chevronDown(12px)" }
    hover: { border: "1px solid {colors.borderStrong}" }
    overridden: { text: "{colors.primaryBright}", border: "1px solid rgba(20,184,166,0.5)", label: "선택값 병기 (예: Effort: high)" }
    menu: { bg: "{colors.surfaceRaised}", border: "1px solid {colors.borderStrong}", rounded: "{rounded.lg}", shadow: "{elevation.2.shadow}", placement: "칩 위(bottom-full)로 열림", caption: "이름 + '— 이 세션에만 적용' ({typography.fontSize.xs} {colors.text.muted})" }
    menuItem: { fontSize: "{typography.fontSize.sm}", padding: "6px 8px", hover: "bg {colors.surface}", selected: "check(13px) {colors.primaryBright} + text {colors.text.primary}", unselected: "text {colors.text.secondary}", defaultItem: "'기본 (설정값)' 항목을 항상 최상단에" }
    menuSearch: { field: "{components.Input}", height: "28px", fontSize: "{typography.fontSize.sm}", rounded: "{rounded.sm}", placement: "메뉴 최상단 고정 — 목록만 스크롤한다", autoFocus: "열리면 즉시 포커스", rule: "후보 수와 무관하게 항상 같은 자리에 둔다 — 개수에 따라 나타났다 사라지면 '이번엔 검색이 되나'를 매번 확인하게 된다" }
    menuCount: { fontSize: "{typography.fontSize.xs}", text: "{colors.text.muted}", form: "표시/전체 (예: 12/348)", rule: "좁혀졌다는 사실을 숫자로 준다 — 스크롤 막대 길이는 근거가 아니다" }
    menuMatch: { text: "{colors.text.primary}", fontWeight: "{typography.fontWeight.medium}", rest: "{colors.text.secondary}", rule: "질의와 겹친 구간만 밝힌다 — 강조에 새 액센트색을 만들지 않는다(Don't #5)" }
    menuCursor: { bg: "{colors.surface}", text: "{colors.text.primary}", rule: "키보드 커서와 hover는 같은 표현이다(둘은 동시에 존재하지 않는다). 선택값의 check + {colors.primaryBright}와는 다른 채널로 남긴다 — '지금 짚은 것'과 '이미 고른 것'은 다르다" }
    menuEmpty: { text: "{colors.text.muted}", fontSize: "{typography.fontSize.sm}", rule: "'없음'만 쓰지 않는다 — 왜 없는지와 다음 행동을 함께 준다" }
    keyboard: { open: "칩 클릭·Enter → 검색 필드로 포커스", move: "↑/↓ 커서 이동 — 경계에서 순환하지 않는다(끝이 어디인지 보여야 한다)", commit: "Enter로 커서 항목 확정", cancel: "Esc는 팝오버를 닫는다 — 질의를 먼저 비우는 2단계 Esc는 두지 않는다(닫는 법이 상태에 따라 달라진다)", a11y: "combobox + listbox/option + aria-activedescendant, 커서 항목은 scrollIntoView(nearest)" }
  DetailChip:
    scope: "상태 한 줄 + 위로 열리는 상세 팝오버. 승인 바의 준비 점검·자동 해결이 첫 사용처다 — 형태는 {components.SelectChip}과 같지만 목적이 다르다(값을 고르는 것이 아니라 펼쳐 보는 것)"
    base: { fontSize: "{typography.fontSize.xs}", rounded: "{rounded.md}", padding: "4px 8px", border: "1px solid {colors.border}", text: "{colors.text.secondary}", structure: "요약(truncate) + chevronDown(12px)", title: "요약 전문 — 칩은 좁아지면 잘리므로 title로 함께 준다" }
    hover: { border: "1px solid {colors.borderStrong}" }
    attention: { text: "{colors.status.failed.text}", border: "1px solid rgba(185,28,28,0.5)", rule: "확인이 필요하다는 것은 **색으로만** 말한다 — 스스로 펼치지 않는다. 드문 상세가 흔한 작업의 자리를 뺏으면 사용자는 하려던 일을 하기 전에 매번 접어야 한다" }
    panel: { bg: "{colors.surfaceRaised}", border: "1px solid {colors.borderStrong}", rounded: "{rounded.lg}", shadow: "{elevation.2.shadow}", placement: "칩 위(bottom-full), 오른쪽 끝 정렬", width: "min(640px, 100vw - 2rem)", maxHeight: "50vh", a11y: "role=dialog + aria-label(고정 이름), 칩은 aria-expanded/aria-haspopup" }
    scroll: { rule: "팝오버가 유일한 스크롤러다 — 안쪽 블록마다 max-h를 겹쳐 두지 않는다. 겹치면 높이가 합산돼 얼마나 자랄지 아무도 예측하지 못한다(긴 목록 하나 정도는 예외)" }
    close: { rule: "Esc와 바깥 mousedown으로 닫는다. 닫는 클릭을 삼키지 않는다 — 옆 칩을 바로 누르면 그쪽이 열려야 한다. 두 팝오버가 동시에 열리지 않는 것은 이 규칙의 결과이지 따로 둔 상태가 아니다" }
    persistence: { rule: "펼침은 **대상이 바뀔 때만** 접는다. 폴링·갱신에 접힘을 얹으면 대화를 이어가는 동안 매 턴 사용자의 선택이 지워진다" }
    placementRule: "대화 열 아래 바에는 상세를 인라인으로 펼치지 않는다 — 펼친 만큼 대화가 좁아지고 바의 높이가 작업 상태에 따라 달라진다. 팝오버는 대화를 덮되 밀지 않는다"
  Badge:
    base: { fontSize: "{typography.fontSize.xs}", fontWeight: "{typography.fontWeight.medium}", padding: "2px 8px", rounded: "{rounded.full}", iconText: "icon(8px) + label 항상 병기" }
    running:  { bg: "{colors.status.running.bg}",  border: "1px solid {colors.status.running.border}",  text: "{colors.status.running.text}",  icon: "Loader2(spin)" }
    awaiting: { bg: "{colors.status.awaiting.bg}", border: "1px solid {colors.status.awaiting.border}", text: "{colors.status.awaiting.text}", icon: "Clock" }
    question: { bg: "{colors.status.question.bg}", border: "1px solid {colors.status.question.border}", text: "{colors.status.question.text}", icon: "MessageCircleQuestion" }
    done:     { bg: "{colors.status.done.bg}",     border: "1px solid {colors.status.done.border}",     text: "{colors.status.done.text}",     icon: "CheckCircle2" }
    failed:   { bg: "{colors.status.failed.bg}",   border: "1px solid {colors.status.failed.border}",   text: "{colors.status.failed.text}",   icon: "XCircle" }
  Sidebar:
    base: { bg: "{colors.surface}", borderRight: "1px solid {colors.border}", widthExpanded: "240px", widthCollapsed: "56px" }
    navItem: { height: "36px", rounded: "{rounded.md}", text: "{colors.text.secondary}" }
    navItemActive: { text: "{colors.primaryBright}", icon: "{colors.primary}", bg: "rgba(20,184,166,0.10)", leftBar: "2px solid {colors.primary}" }
    sectionLabel: { fontSize: "{typography.fontSize.xs}", fontWeight: "{typography.fontWeight.semibold}", transform: "uppercase", letterSpacing: "0.08em", color: "{colors.text.muted}" }
  DropTarget:
    zone: { border: "2px solid {colors.primary}", bg: "rgba(20,184,166,0.20)", scope: "드롭하면 안에 들어가는 대상(에디터 분할 존·프로젝트 그룹 박스). 값은 EditorSplitView 드롭 존(border-2 border-primary bg-primary/20)에서 승격" }
    caret: { size: "2px", bg: "{colors.primary}", attr: "data-drop-caret", scope: "드롭하면 사이에 끼는 자리(탭 순서·그룹 순서) — 어디에 앉을지 보이는 표시는 필수" }
    rejected: { dropEffect: "none", highlight: "없음" }
  Tabs:
    list: { bg: "{colors.surface}", borderBottom: "1px solid {colors.border}" }
    trigger: { text: "{colors.text.secondary}", borderBottom: "2px solid transparent" }
    triggerActive: { text: "{colors.primaryBright}", borderBottom: "2px solid {colors.primary}", fontWeight: "{typography.fontWeight.medium}" }
    focusVisible: "outline 2px solid {colors.primary}"
    scope: "면을 바꾸는 컨트롤 — 누르면 다른 내용이 온다. 같은 목록의 범위를 좁히는 것은 FilterSegment다"
  FilterSegment:
    sortAxis: { rule: "정렬 축을 세그먼트에 섞지 않는다 — 세그먼트는 **무엇을 보느냐**(범위)이고 정렬은 **어떤 순서로 보느냐**다. 한 줄에 두 축을 두면 눌린 칸이 무엇을 뜻하는지 매번 읽어야 한다. 정렬은 {components.SelectChip}으로 오른쪽에 따로 둔다" }
    lensAxis: { rule: "렌즈 축(누가 쓰는가)이면 `전체`에 groupHeader를 달지 않는다 — 소속은 이미 행마다 가용성 태그가 말하고 있어, 헤더로 또 가르면 같은 내용이 두 자리를 차지한다(Don't #11)" }
    base: { fontSize: "{typography.fontSize.xs}", rounded: "{rounded.sm}", padding: "3px 8px", minHeight: "24px", text: "{colors.text.secondary}", structure: "label + count" }
    hover: { text: "{colors.text.primary}", bg: "{colors.surfaceRaised}" }
    active: { text: "{colors.primaryBright}", bg: "rgba(20,184,166,0.10)", rule: "PaneChip·사이드바 활성과 같은 문법 — '지금 보고 있는 곳'의 언어는 앱 전체에 하나뿐이다(Don't #13)" }
    count: { text: "{colors.text.muted}", activeText: "{colors.primaryBright} 70%", rule: "라벨 뒤에 개수를 병기한다 — 누르기 전에 무엇이 몇 개인지 보여야 빈 칸을 고르지 않는다. 활성 칸에서는 muted를 쓰지 않는다 — teal wash 위에서 읽히지 않는다" }
    group: { border: "1px solid {colors.border}", rounded: "{rounded.md}", padding: "2px", gap: "2px", rule: "세그먼트 전체를 한 테두리로 묶는다 — 배타적 선택임을 형태로 말한다", narrow: "폭이 모자라면 줄바꿈한다 — 가로 스크롤로 밀면 뒤쪽 세그먼트는 존재조차 보이지 않는다" }
    empty: { rule: "개수 0인 세그먼트는 렌더하지 않는다(전체는 예외) — 눌러도 아무것도 없는 칸은 손잡이가 아니다" }
    allView: { rule: "'전체'는 필터를 끄는 모드가 아니라 같은 기준으로 묶어 보는 모드다 — 항목에 groupHeader를 달아 구분을 유지한다" }
    newItem: { rule: "항목을 새로 만들거나 들여오면 '전체'로 되돌린다 — 방금 만든 것이 현재 필터 밖이면 사용자는 실패했다고 읽는다" }
    groupHeader: { fontSize: "{typography.fontSize.xs}", fontWeight: "{typography.fontWeight.semibold}", text: "{colors.text.muted}", rule: "'전체'에서만 나타난다 — 하나로 좁힌 뒤에는 눌린 세그먼트가 이미 그 이름을 말한다" }
    a11y: { role: "tablist/tab + aria-selected", roving: "선택된 세그먼트만 tabIndex 0 — Tab이 칸마다 멈추면 목록까지 가는 데 여섯 번 눌러야 한다", keys: "←/→로 이동, Home/End로 양 끝. 선택과 포커스가 함께 움직인다", focusVisible: "outline 2px solid {colors.primary}; outline-offset 2px", rule: "활성은 색 단독이 아니라 배경면으로도 표시한다" }
    selected: { rule: "선택된 세그먼트는 개수가 0이어도 남긴다 — 검색으로 비는 순간 눌린 칸이 사라지면 지금 어디에 서 있는지 알 수 없다. empty 규칙보다 이쪽이 우선한다" }
    count-source: { rule: "개수는 다른 축의 필터(검색어·프로젝트 범위)를 **먼저 적용한 뒤** 센다 — 누르면 실제로 나올 수가 아니면 병기하는 의미가 없다" }
    scope: "같은 면의 범위를 좁히는 모든 목록. SkillsView 출처 필터(전체/Claude/Codex/Antigravity/GitHub/직접 추가)와 MemoryView 상태 필터(전체/검토 대상/후보/검토 중/stale/이관 대기/승인 완료/휴면). 목록 위 한 줄에 놓고, 선택은 뷰 세션 동안만 유지한다"
    impl: "src/components/FilterSegment.tsx — 계약을 코드로 한 번만 구현한다. 화면마다 손으로 다시 그리면 개수 병기·roving tabIndex가 조용히 빠진다"
  SessionNavigator:
    base: { bg: "{colors.surfaceRaised}", border: "1px solid {colors.borderStrong}", rounded: "{rounded.xl}", density: "compact" }
    item: { text: "{colors.text.secondary}", hover: "{colors.surface}", selectedText: "{colors.primaryBright}", selectedBg: "rgba(20,184,166,0.10)", focus: "2px solid {colors.primary}", disabledText: "{colors.text.muted}" }
    scope: "메인 창의 Shift 두 번 — 기존 프로젝트·그룹·세션 목록을 검색 가능한 트리로 탐색. 그룹 소속·저장 형식은 사이드바와 공유"
    structure: "검색 필드 → 그룹 → 프로젝트 → 세션. 그룹·프로젝트는 펼침/접힘, 세션은 해당 작업 열기. 미소속·빈 프로젝트도 접근 가능"
    search: "일치 항목의 조상 경로를 보존하고 검색 중 접힌 조상을 펼친다. 이름이 같아도 경로와 호스트로 구분하며 선택은 host + id를 유지"
    keyboard: "↑/↓는 보이는 행 순서, ←/→는 계층 탐색, Enter는 행의 동작, Esc는 닫기·이전 포커스 복귀. IME 조합 확정은 선택 동작과 분리"
    compatibility: "⌘K는 기존 통합 검색, ⌘P는 기존 파일 검색. 두 검색 오버레이는 동시에 열지 않는다"
  EditorSearch:
    base: { bg: "{colors.surfaceRaised}", border: "1px solid {colors.borderStrong}", rounded: "{rounded.xl}", tabs: "{components.Tabs}", density: "compact" }
    states: { defaultText: "{colors.text.secondary}", hover: "{colors.surface}", selectedText: "{colors.primaryBright}", selectedBg: "rgba(20,184,166,0.10)", focus: "2px solid {colors.primary}", disabledText: "{colors.text.muted}", loading: "종류별 검색 중 안내", error: "오류 안내 + 재검색 경로", empty: "결과 없음 + 질의 변경 경로" }
    scope: "별도 에디터 창의 Shift 두 번 — 현재 창에 연결된 세션 범위를 표시. 창 사이의 작업·커맨드 이동은 포함하지 않는다"
    structure: "범위 표시 + 전체/파일/내용 탭 + 검색 필드 + 결과 목록. 파일 경로와 내용 결과의 줄 위치를 표시"
    keyboard: "Tab/Shift+Tab은 검색 유형 전환, ↑/↓는 보이는 결과 순서, Enter는 같은 창에서 열기·해당 위치 이동, Esc는 닫기·이전 포커스 복귀"
    availability: "실행할 수 있는 범위만 검색한다. 로컬 전용 내용 검색에 원격 task id를 보내지 않는다. 미지원·로딩·오류·결과 없음을 구분"
    integrity: "검색어·범위·세션이 바뀌면 이전 응답을 선택할 수 없다. 검색 열기·닫기가 미저장 편집을 초기화하지 않는다"
  MetaTag:
    base: { fontSize: "{typography.fontSize.xs}", rounded: "{rounded.sm}", padding: "{spacing.0.5} {spacing.1.5}", border: "1px solid {colors.border}", text: "{colors.text.muted}", fill: "none — 채우지 않는다" }
    accent: { text: "{colors.primaryBright}", border: "1px solid {colors.border}", rule: "형태는 같고 색만 바뀐다. '기본을 덮는다'는 신호에만 쓴다 — project 스킬이 global을, must_apply가 relevance 게이트를 덮는다" }
    scope: "상태가 아닌 분류·범위·정책. 스킬의 global/project, 메모리의 status·휴면·항상 적용"
    rule: "Badge는 작업 상태 4종 전용이다 — 비상태 표시에 끌어 쓰면 '색이 곧 상태'라는 계약이 흐려진다. 아이콘 병기 의무도 여기엔 없다: 상태가 아니므로 WCAG 1.4.1의 색-단독 금지 대상이 아니다"
    text-muted: { rule: "클릭 대상이 아닌 표시이므로 {colors.text.muted} 허용 — Don't #3은 클릭 가능한 텍스트에 대한 규칙이다" }
  SkillCard:
    base: { bg: "{colors.surface}", border: "1px solid {colors.border}", rounded: "{rounded.lg}", padding: "{spacing.3}", density: "두 줄 — 첫 줄 이름·메타, 둘째 줄 설명 2줄 clamp" }
    name: { font: "{typography.fontFamily.code}", fontSize: "{typography.fontSize.sm}", text: "{colors.text.primary}", hint: "argument-hint를 이름 뒤에 {colors.text.muted}로 병기" }
    meta: { order: "크기 → 실적 → 발동 방식 → 가용성", component: "{components.MetaTag}", rule: "첫 줄 오른쪽에 모아 둔다 — 카드마다 같은 자리라야 훑을 때 눈이 흔들리지 않는다" }
    actions: { rule: "**없다.** 스킬의 정본은 에이전트 디렉터리이고 Praxis는 저장하지 않는다 — 뷰어에 추가·삭제·가져오기 버튼을 두면 고칠 수 없는 것을 고칠 수 있는 척하게 된다. 여는 동작은 본문 펼침 하나뿐이다" }
    size: { rule: "크기는 확장 시 프롬프트에 들어가는 양이다. claude 네이티브로 도는 스킬에는 실제 비용이 아니므로, 렌즈가 codex·agy일 때만 accent로 든다" }
  Dialog:
    content: { bg: "{colors.neutral.850}", border: "1px solid {colors.borderStrong}", rounded: "{rounded.xl}", shadow: "{elevation.3.shadow}", width: "min(90vw, 480px)" }
    overlay: { bg: "rgba(0,0,0,0.75)", backdropFilter: "blur(2px)" }
  Toast:
    base: { bg: "{colors.neutral.850}", rounded: "{rounded.lg}", shadow: "{elevation.2.shadow}", structure: "icon + title + description" }
    accentStrip: { default: "{colors.primary}", success: "{colors.status.done.border}", warning: "{colors.status.awaiting.border}", error: "{colors.status.failed.border}" }
  MemoryReviewCard:
    base: { bg: "{colors.surface}", border: "1px solid {colors.border}", rounded: "{rounded.lg}", padding: "{spacing.3}", width: "단일 열 max-w-3xl — 2열 리스트/상세로 가르지 않는다" }
    knowledgeType: { width: "20 고정폭 좌측 라벨", fontSize: "{typography.fontSize.xs}", fontWeight: "{typography.fontWeight.medium}", colorBy: "claim {colors.text.secondary} / observation status.awaiting / decision status.running / convention {colors.primaryBright} / abandoned status.failed / pitfall status.question", rule: "지식 유형은 상태가 아니라 분류다 — 색은 빌려 쓰되 Badge 형태를 쓰지 않는다" }
    content: { fontSize: "{typography.fontSize.md}", wrap: "break-words — 코드·경로가 섞인 본문이 카드를 넘기지 않는다" }
    meta: { structure: "MetaTag 줄(휴면·status·항상 적용) + scope·적용 횟수·최종 사용", codeValue: "{typography.fontFamily.code} {colors.text.muted}", rule: "status는 MetaTag이지 Badge가 아니다 — 메모리 수명주기는 작업 상태 4종과 다른 축이다" }
    readiness: { placement: "본문 아래 인라인 한 줄", tone: "ready / needs_action / blocked / inactive / unknown", rule: "무엇을 해야 승인되는지 사람 말로 적는다 — status 문자열만으로는 다음 손이 정해지지 않는다" }
    actions: { placement: "카드 우측 세로 스택", fontSize: "{typography.fontSize.xs}", base: "{colors.text.muted} → hover {colors.text.primary}", approve: "{colors.primaryBright} — 사람이 확인했다는 유일한 행위라 여기만 강조한다", order: "사용이력 · 근거 · 버전 · 편집 · 승인 · 항상 적용 · 보관" }
    drilldown: { form: "카드 안에서 펼치는 인라인 아코디언(사용이력·근거·버전)", exclusive: "카드마다 하나씩", rule: "상세를 별도 패널로 밀지 않는다 — 검토는 목록을 훑으며 하는 일이고, 면을 갈아 끼우면 방금 본 이웃 항목이 사라진다" }
    selection: { rule: "체크박스는 legacy_unverified에만 — 일괄 처리는 이관이라는 한 국면에서만 정당하다. 승인은 언제나 한 건씩 사람이 한다" }
    destructive: { rule: "보관·복원 같은 상태 변경은 hover 인라인 버튼으로 노출하지 않는다 — 스크롤 중 오조작이 사람 승인 기록을 흔든다. 상시 표시하고 확인을 거친다" }
  WorkContextPanel:
    base: { bg: "{colors.surface}", borderBottom: "1px solid {colors.border}", paddingInline: "{spacing.3}", structure: "environment summary + current activity + subagents + recent activity" }
    section: { borderBottom: "1px solid {colors.border}", paddingBlock: "{spacing.3}", heading: "{typography.fontSize.sm} {colors.text.secondary}" }
    row: { minHeight: "28px", icon: "16px {colors.text.secondary}", label: "{typography.fontSize.sm}", value: "{colors.text.secondary}", codeValue: "{typography.fontFamily.code}" }
    diff: { additions: "{colors.status.done.text}", deletions: "{colors.status.failed.text}", loading: "{colors.text.muted}", error: "{colors.status.awaiting.text}" }
    refresh: { default: "{colors.text.muted}", hover: "{colors.text.primary}", focusVisible: "outline 2px solid {colors.primary}; outline-offset 2px" }
    states: { loading: "inline section status; keep panel shell", empty: "explicit no changes/no activity copy", error: "inline retryable diff status; keep verified task metadata", edge: "omit commit/push/source fields when no verified API exists" }
  SessionWorkspace:
    base: { structure: "header(44px) -> [file tree | session | code] -> terminal dock -> review bar -> composer", rule: "영역마다 자기 사각형을 갖는다 — 겹치는 것은 플로팅 채널 하나뿐이고 그것도 자리를 예약받는다" }
    headerActions: { align: "우측 끝을 세션 콘텐츠 박스의 오른쪽 끝에 맞춘다", rule: "채널이 떠 있으면 320px 물러선다 — 창 끝에 붙으면 바로 아래 채널의 머리처럼 읽혀 세션의 손잡이라는 것이 보이지 않는다", codeOpen: "채널이 코드 열로 흡수되면 예약이 없으므로 창 끝으로 돌아온다", form: "소환 칩은 전부 PaneChip(파일·프리뷰·Diff·에디터 창·터미널) — 눌림은 teal wash 배경면 하나로 말한다. 트리 열 손잡이는 헤더에 없다(ADR 0188)" }
    dismiss: { rule: "소환된 면은 자기 위에 닫기를 갖는다 — 연 손잡이의 재클릭·단축키는 보조 경로다", code: "탭줄 오른쪽 ✕", terminal: "도크 헤더 ✕", channel: "환경 헤더 ✕ + 접힌 자리의 재열기 핸들" }
    tree: { width: "208px", toggle: "헤더의 파일 칩이 코드 열과 함께 연다(ADR 0188); 닫기는 트리 헤더 ✕ 또는 CmdB", scope: "세션 열 왼쪽 바깥의 파일 트리. 변경 목록은 오른쪽 Diff 탭" }
    session: { minWidth: "360px", width: "코드 열이 열리면 저장된 세션 폭(기본 480px) — 코드 열의 탭에 무관(ADR 0188)", owns: "대화/출력/서브 에이전트. 변경 파일 선택 시 이 자리를 diff로 전환하고 입력창도 숨김. 대화 복귀 시 스크롤·작성 중 메시지 복원" }
    code: { minWidth: "480px — 모든 탭이 같은 폭·같은 리사이저(ADR 0188)", default: "닫힘 — 부를 때만 자리를 받는다(ADR 0111)", tabs: "작업정보(고정) / 파일 / 프리뷰 / Diff(변경 파일 목록)", opens: "Diff 버튼은 목록을 열고 행 클릭은 중앙 본문을 선택한다. 파일 열기·CmdP·LSP 이동은 일반 파일 탭을 연다" }
    fallback: { exit: "중앙 잔여 < 840px -> 탭", enter: ">= 900px -> 2열", rule: "두 임계 사이는 직전 모드 유지. Diff 목록도 같은 폴백을 따른다 — 좁은 창에서는 목록과 세션 자리의 본문을 번갈아 본다(ADR 0188). 전환 중 대화 DOM 수명 보존" }
    activityHome: { rule: "작업정보의 거처는 언제나 하나이고, 그것을 코드 열의 상태가 정한다", codeClosed: "세션 열 우상단 플로팅 채널 + 세션이 332px 예약", codeOpen: "코드 열의 고정 작업정보 탭 — 플로팅은 사라진다", narrow: "중앙 잔여 < 692px이면 코드 열 탭만; 컨텍스트 게이지가 그리로 보낸다" }
  FloatingActivityChannel:
    base: { bg: "{colors.surfaceRaised}", border: "1px solid {colors.borderStrong}", rounded: "{rounded.xl}", shadow: "{elevation.3.shadow}", width: "300px 고정 — 조절 핸들 없음", inset: "세션 열 안쪽 16px", structure: "작업정보 카드 + 오늘 할 일 카드의 세로 스택" }
    scope: { host: "세션 열 — 화면이 아니라 그 열의 레이어다", never: "헤더·파일 트리·코드 열·터미널 도크·컴포저 위에 걸치지 않는다" }
    reserve: { width: "332px = 300 + 16(바깥 여백) + 16(콘텐츠 간격)", target: "세션 열 paddingRight + 헤더 액션 줄 marginRight(332 - 헤더 패딩 12 = 320)", rule: "떠 있는 동안에만 건다; absolute 기준이 padding box라 예약은 카드 위치를 밀지 않는다; 헤더는 전폭이라 예약이 안 걸리므로 액션 줄만 같은 선까지 물러선다" }
    content: { density: "rail — 하위 에이전트는 한 줄로 접고 최근 활동은 진입점만 둔다", overflow: "모자라면 작업정보가 먼저 줄어든다(min-h-0 + 내부 스크롤); 할 일은 45%까지", empty: "카드째 사라진다 — 빈 껍데기를 세션에 띄우지 않는다", scrollbar: "숨김" }
    states: { default: "코드 열이 닫혀 있고 중앙 잔여 >= 692px(= 세션 최소 360 + 예약 332)", codeOpen: "숨김; 같은 카드가 코드 열의 고정 작업정보 탭으로 이동", narrow: "자리가 모자라면 숨김 — 창 폭이 아니라 사이드바·트리를 뺀 중앙 잔여로 잰다", collapsed: "환경 헤더의 ✕로 접는다 — 닫기는 닫을 대상 위에 있다. 컨텍스트 게이지는 보조 진입점(잔량은 손끝에, 설계 0044)" }
    handle: { form: "IconButton(panelRight 16px) + {colors.surfaceRaised} bg + 1px {colors.borderStrong} + {rounded.md}", position: "채널이 살던 자리 — 세션 열 우상단 16px", visible: "접혀 있고 다시 뜰 자리가 있을 때만(코드 열 닫힘 + 중앙 잔여 >= 692px) — 눌러도 못 뜨는 자리에는 문을 만들지 않는다", rationale: "여는 곳과 나타나는 곳이 같은 자리여야 인과가 보인다" }
  Village:
    base: { bg: "{colors.surface}", border: "1px solid {colors.border}", rounded: "{rounded.lg}", structure: "header telemetry + one street of four ordered buildings + one alley below" }
    map: { tile: "16px logical; the map is a fixed 24x12 grid (384x192 logical px) — never fluid", scale: "integer 1x/2x/3x chosen from container size; leftover space is letterboxed, never stretched — fractional scale smears pixel edges", ground: "44 procedurally baked tiles; brightness runs in four steps (alley → grass → road → zone floor) so shapes read at all, and material seams get transition tiles (road edges, 8-way water) instead of a ruled line", buildings: "2 tiles tall (roof row + wall row) — one 16px cell gives 6px of roof and 9px of wall, and neither reads", decor: "declared in village-decor.ts; tests assert it never intersects walkable tiles or seats — a tree a character walks over is only findable by eye", order: "prep → work → review → plaza, left to right; the alley sits below the main road, outside the pipeline order" }
    zone: { form: "an OPEN area, not a roofed box — a floor of its own tiles + one structure row at the back + a sign; roofing it would hide the characters inside", border: "1px solid {colors.borderStrong}, tinted by status", seats: "declared tile coordinates, at least one per capacity slot (>= 8); two rows with a gap so the front row does not cover the back", empty: "keeps its slot and explains itself" }
    agent: { rendering: "project-owned pixel-art sheet baked at LOGICAL 1:1 (16x24 per cell, 4 facings x 3 frames x 6 roles); CSS transform: scale() picks the display multiple — baking it in would freeze the scale", position: "zone membership IS the status", identity: "role fixes the sprite and the name for the life of the task", facing: "down while resting — a village of backs of heads has no face to read status from", hitTarget: "the character itself is the button, sized to the drawn sprite (not the logical cell), focusable including mid-travel", maximum: 8, overflow: "surplus is stated as +n 더보기" }
    text: { scale: "labels, signs, bubbles and cards do NOT take the map scale — they are not pixel fonts and blur when multiplied, and the OS text-size setting must keep meaning" }
    card: { trigger: "click or Enter on a character", placement: "beside the character, opening toward the street interior; below it in single-column", dismiss: "Escape, outside pointerdown, re-clicking the character, or the character starting to travel", content: "status + age + current operation + instruction (3-line clamp) + agent/repo/id + primary 작업 열기 button", elevation: "inset top highlight + drop shadow (popover exception)" }
    operation: { source: "main-thread tool use only — subagent calls must not overwrite it", placement: "speech bubble above the working character", overflow: "single-line clamp; a bubble wider than the character breaks the street" }
    motion: { travel: "state transitions only; an axis-aligned waypoint path (never diagonal — no diagonal frames exist) at 180ms per tile, replayed by transform transition per segment", roam: "2-3 tiles WITHIN the character's own zone at 600ms per tile; never leaves the zone, because the zone is the status", hold: "hover, focus, selection, or an in-flight transition stops that character where it stands — it does not walk back to its seat, or it would flee the cursor", reduced: "roam off entirely; travel becomes instant placement" }
    working: { building: "작업 중", text: "{colors.status.running.text}" }
    waiting: { building: "확인 대기", text: "{colors.status.awaiting.text}" }
    idle: { building: "준비 중", text: "{colors.text.secondary}" }
    done: { building: "완료 광장", text: "{colors.status.done.text}" }
    failed: { building: "골목", text: "{colors.status.failed.text}" }
    reducedMotion: { motion: "none", travel: "instant reposition", hover: "no lift; buildings and cards unchanged" }
---

# Praxis Design System

> Canonical design contract. 모든 구현·리뷰·시각 QA는 이 문서와 `docs/designs/wireframes/`를 정본으로 따른다.
> 토큰 정본(YAML front matter)은 normative. 본문은 rationale.

## Overview

**Chromatic Discipline** — Praxis는 에이전트 오케스트레이션 도구다. 사용자는 동시에 수십 개 에이전트 상태를 훑고, 터미널 출력을 읽고, diff를 판단한다. 모든 시각 결정의 목표는 **인지 부하 최소화**다.

- **색은 상태 또는 Role Stations의 역할 정체성을 전달할 때만**, 공간은 정보 위계를 만든다.
- **그림자 대신 보더로 계층**을 쌓는다 (다크에서 그림자는 약하다).
- **단일 Teal 액센트**가 "인터랙션 가능한 것"을 명확히 표시한다.
- 다크 모노크로매틱 기반 위에 상태색 4종과 컴포넌트 한정 역할색만 얇은 신호로 돌출한다.
- 오래 봐도 눈이 피로하지 않게 — 글로우/그라디언트 금지.

레퍼런스: Warp(3단 표면 위계 + left-border 상태 스트립), Linear(액센트 극도 절제), Vercel(semantic token + 텍스트 레이블 병기), Sentry/Datadog(색+아이콘+텍스트 3중 인코딩), Zed(에디터 register vs UI register 분리).

## Colors

| 역할 | 토큰 | 값 |
|------|------|-----|
| App 배경 | `colors.background` / `neutral.950` | `#0d0d0d` |
| 카드/패널/사이드바 | `colors.surface` / `neutral.900` | `#161616` |
| 팝오버/드롭다운/모달 | `colors.surfaceRaised` / `neutral.850` | `#1f1f1f` |
| 보더 | `colors.border` / `neutral.800` | `#2a2a2a` |
| 강조 보더 | `colors.borderStrong` / `neutral.700` | `#3f3f46` |
| **Primary (CTA/포커스/활성)** | `colors.primary` | `#14b8a6` |
| Primary hover / active | `primaryHover` / `primaryActive` | `#0d9488` / `#0f766e` |
| Interactive icon / progress / link | `primaryBright` | `#2dd4bf` |
| 본문 텍스트 | `text.primary` | `#f4f4f5` (16.8:1) |
| 보조 텍스트 | `text.secondary` | `#a1a1aa` (AA) |
| 비활성/플레이스홀더 | `text.muted` | `#71717a` (3.5:1, 비인터랙션 전용) |
| 차트 계열 1 | `--c-series-1` | `var(--c-primary-bright)` |
| 차트 계열 2 | `--c-series-2` | `var(--c-primary-hover)` |
| 차트 계열 3 | `--c-series-3` | `color-mix(in srgb, var(--c-primary-bright) 45%, var(--c-bg))` |
| 차트 계열 4 | `--c-series-4` | `color-mix(in srgb, var(--c-primary-hover) 60%, black)` |
| 채도 높은 액센트 면 위 텍스트 | `--c-on-accent` | `#f4f4f5` |

**상태색** (bg / border / text, 다크 배경 위 AA 충족):

| 상태 | bg | border | text | 명암비 |
|------|-----|--------|------|--------|
| Running | `#0c1929` | `#1d4ed8` | `#60a5fa` | 7.3:1 |
| AwaitingReview | `#1c1407` | `#b45309` | `#fbbf24` | 9.5:1 |
| Done | `#071a0f` | `#15803d` | `#4ade80` | 8.4:1 |
| Failed | `#1a0a0a` | `#b91c1c` | `#f87171` | 5.8:1 |

**규칙**: Teal은 CTA·포커스링·활성 탭 하단선·활성 사이드바 항목에만. 상태색은 상태 표시 전용(버튼/링크/내비 금지). 색 단독으로 상태 전달 금지 — 항상 아이콘+텍스트. 차트 계열은 액센트 명도 계조의 파생이다 — 두 번째 유채색 금지는 차트에도 적용된다.

**Diff 2단계 틴트**: diff 본문은 줄 단위 틴트(`addbg`/`delbg`) 위에 단어 단위 강조(`addbg-strong`/`delbg-strong`)를 겹친다. 한 줄에서 실제로 바뀐 문자 구간만 짚기 위한 것이며 **diff 본문 전용**이다 — 상태 표시나 일반 하이라이트에 쓰지 않는다. 줄 대부분이 바뀐 경우에는 강조를 생략하고 줄 틴트만 남긴다.

**Village 역할색**: planner `#b7a47a`, researcher `#64748b`, implementer `#14b8a6`, tester `#b78aa5`, reviewer `#8b7ad1`. 역할색은 이름표의 1~3px strip에만 사용하며 glyph+역할 텍스트를 항상 병기한다. 건물 상태색과 의미를 섞지 않는다 — 역할은 정체성이고 건물은 상태다.

**파일 타입 아이콘**: 아는 언어는 [material-icon-theme](https://github.com/material-extensions/vscode-material-icon-theme)(MIT)의 **자기 색을 가진 글리프**로 그린다 — VS Code 사용자가 아는 그 세트다. 색이 아이콘 **안에** 있으므로 Don't #5의 브랜드색 예외에 속하고, 틴트를 덧칠하지 않는다(두 색이 싸운다). 모르는 종류는 아래 단색 틴트로 후퇴하므로 **"형태가 주 채널"이라는 계약은 그대로다.** 두 곳에서 컬러 글리프를 끄는 예외가 있다: **디렉터리**(펼침 상태를 실루엣으로 말해야 한다)와 **권한 없는 행**(밝은 로고가 muted보다 살아 보여 상태 신호를 덮는다).

> **라이트 테마에서 노란 계열(js·lock·database)은 대비가 1.4~1.5로 낮다.** 실측했고 받아들인다 — 18px 트리 행에서는 내부 실루엣이 대비를 벌어 읽히고, 이것은 참조 구현(VS Code)과 같은 거동이다. 로고를 우리 손으로 다시 칠하면 그때부터 그것은 그 세트가 아니다. 이 판단이 뒤집히면 그 종류만 후퇴 경로로 내리면 된다 — 후퇴 경로는 이미 있다.

**파일 타입 틴트**(후퇴 경로): 파일 트리·디렉터리 브라우저 **아이콘 전용** 보조 채널. 주 채널은 형태(실루엣)이고 색은 그 위에 얹는 저채도 틴트다 — 색 판별에 실패해도 형태로 후퇴한다. 채도는 상태색의 절반 이하로 눌러 액센트 teal·상태색과 hue가 겹치지 않으며, 상태·인터랙션 의미로 쓰지 않는다. 테마와 무관한 고정 2벌이고 정본은 `index.css`의 `--ft-*`다.

| 카테고리 | 다크 | 라이트 |
|---|---|---|
| 폴더 | `#c9a86a` | `#8a6d2f` |
| 코드·셸 | `#a094d4` | `#6d5fb8` |
| 문서 | `#7f9bbd` | `#4f6d94` |
| 스타일·자산·마크업 | `#c98ba6` | `#a05578` |
| 설정·데이터 | `#8fa878` | `#5f7a48` |

디렉터리는 골드 틴트 + `font-medium` 이중 인코딩으로 격상한다. lock·git류 특수 파일은 무색(muted)을 유지한다 — 전부 칠하면 아무것도 안 칠한 것과 같다.

### 테마 — 같은 규율, 다른 팔레트

위 표는 기본 테마 **Praxis Dark**의 정본인 동시에, 모든 테마가 채워야 할 **슬롯 목록**이다. 테마는 자유 스타일이 아니라 **토큰 세트 교체**다 — 표면 3단·보더 2단·텍스트 3단·액센트 1·상태 5를 자기 팔레트로 채우되, 규칙(액센트는 인터랙션 전용, 상태색은 상태 전용, 색 단독 전달 금지)은 그대로 진다.

| 테마 | 계열 | 성격 |
|------|------|------|
| Praxis Dark / Light | 중성 모노크롬 | 기본. teal 단일 신호 |
| Catppuccin Mocha / Latte | 청보라 파스텔 | 저자극 — 장시간 응시에 가장 편하다 |
| Tokyo Night | 청보라 네온 | 대비가 또렷하다 |
| Nord | 한색 플랫 | 절제 — 이 문서의 철학에 가장 가깝다 |
| Gruvbox Dark | 난색 레트로 | 유일한 따뜻한 배경 |
| One Dark Pro | 저채도 클래식 | 가장 익숙하다 |

정본은 `src/lib/themes.ts`다. 각 테마는 **원색 15개만 선언**하고 diff 틴트·스크롤바 농도·액센트 변형·터미널 커서는 계산으로 파생한다 — 값을 손으로 적으면 테마 8종 × 토큰 25개를 사람이 관리하게 되고, 하나가 어긋나도 눈으로는 안 보인다.

**대비 보정**: 팔레트 원색이 최소 명암비(본문 7:1, 보조 4.5:1, muted 3.5:1, 상태 4.5:1)에 못 미치면 **색상·채도는 두고 명도만** 옮겨 채운다. Nord·Gruvbox처럼 원래 저대비인 팔레트를 성격을 죽이지 않고 규율 안으로 들이기 위한 장치다. Praxis Dark/Light는 여기서 이미 검증된 값이므로 보정하지 않는다.

**액센트 선택**: 팔레트 안에서 **상태색 5종과 겹치지 않는 색**을 고른다. 겹치면 "누를 수 있는 것"과 "상태"를 색으로 구분할 수 없다. Catppuccin이 시그니처 mauve 대신 teal을, One Dark가 blue 대신 cyan을 쓰는 이유다. Gruvbox만 예외로 시그니처 orange를 쓴다 — 그 팔레트에서 orange를 빼면 Gruvbox가 아니다.

## Typography

2-레지스터 시스템:
- **UI 레지스터** = `Inter` — 레이블/버튼/헤더/본문. 기본 `md(14px)`.
- **Code 레지스터** = `JetBrains Mono` — 코드/터미널/파일경로/타임스탬프/Task ID. 기본 `base(13px)`, line-height `code(1.6)`.

스케일: xs(11) badge · sm(12) metadata · base(13) code · md(14) UI body · lg(16) section header · xl(20) panel header · 2xl(24) page title. 웨이트: regular(400)/medium(500, 레이블·버튼)/semibold(600, 헤더)/bold(700, 페이지 타이틀).

같은 카드 안에서 두 폰트 혼용 허용 (예: UI 레이블 + 모노스페이스 값).

- **WIKI 읽기 레지스터**: 렌더링한 문서 본문은 `typography.fontFamily.ui`, `fontSize.lg`, `lineHeight.relaxed`; 보조 목록은 `fontSize.md`, 경로·Markdown 소스는 `fontFamily.code`와 `fontSize.base`를 쓴다. 본문 최대 폭 680px, 절 사이 `spacing.6`, 문단 사이 `spacing.4`. compact/relaxed 전환은 목록·도구 영역 밀도를 바꾸며 긴 문서의 본문을 작은 메타데이터 크기로 축소하지 않는다.

## Layout

- **데스크톱 우선**, 최소 지원 960px. 기본 1280×800+.
- **글로벌 사이드바**(좌측 고정): 확장 240px / 축소 56px 토글. nav 항목 36px.
- 960px 이하: 사이드바 아이콘 전용 자동 축소. diff 본문은 **창 폭이 아니라 자기 본문 폭이 720px 미만일 때** unified로 강제한다.
- spacing 4px 베이스 스케일. 화면 본문 패딩 기본 `spacing.4~6`.
- **세션 워크스페이스**: 아래 표가 영역의 정본이다. 각 영역은 자기 사각형을 갖고, **겹치는 것은 플로팅 채널 하나뿐이며 그것도 세션 열에서 자리를 예약받는다.**

| 영역 | 폭 | 기본 | 소유 |
|---|---|---|---|
| 트리 열 | 208px | 접힘 — 헤더의 파일 칩이 코드 열과 함께 연다 | 파일 트리 |
| 세션 열 | 일반 모드 최소 360px | 중앙 전폭 | 대화/출력·서브 에이전트. 변경 파일 선택 시 diff, 복귀 시 대화·입력 복원 |
| 코드 열 | 최소 480px, 탭에 무관 | **닫힘** | 작업정보(고정) · 파일 · 프리뷰 · Diff 변경 목록 |

코드 열은 탭에 무관하게 중앙 잔여가 840px 미만이면 탭으로 접고 900px 이상에서 2열로 돌아온다. Diff 목록만 좁게 두던 분기는 ADR 0188에서 뺐다. 오른쪽 패널 닫기·다른 탭 선택은 대화 복귀로 이어진다.

**별도 질의** — 같은 코드 열에 `따로 질문` 탭을 추가한다. 기존 480px/360px 최소 폭과 840/900px 전환을 그대로 쓰며 파일·프리뷰와 자리를 공유한다. 좁은 모드에서 질의 중에는 전폭 메인 컴포저를 숨기고, `메인 대화로` 또는 `메인 입력창에 첨부`로 대화와 기존 초안을 복원한다. 세부 배치와 상태는 S-21, 동작·격리 계약은 설계 계약이 정본이다.

- **작업정보의 거처는 언제나 하나이고, 그것을 코드 열의 상태가 정한다.** 코드 열이 닫혀 있으면 세션 열 우상단에 플로팅 채널로 뜨고 세션은 332px을 비운다. 파일·Diff를 부르면 채널은 사라지고 같은 카드가 코드 열의 고정 `작업정보` 탭으로 옮겨 간다. 자리 판정은 **창 폭이 아니라 중앙 잔여 폭(≥ 692px = 세션 최소 360 + 예약 332)** 으로 한다 — 사이드바와 파일 트리를 빼고 나면 같은 창도 세션에 남는 폭이 배 넘게 차이 난다. 자리가 모자라면 코드 열 탭이 유일한 진입로다.
- **밀도(density)**: 기본 **A/compact** — card padding `spacing.4`, nav 32px, UI 13px, 상태 스트립 3px, 카드 갭 `spacing.3`. 토글 **B/relaxed** (`data-density="relaxed"`) — card padding `spacing.8`, 미세 그림자, UI 15px, line-height relaxed, content max-width 680px. Tailwind variant `compact:` / `relaxed:`로 분기.

### WIKI 콘텐츠 영역

> 2026-09-28 Wiki 채널(사이드바 Wiki·세션 입력창의 위키 참고자료)을 제거했다. 이 절과 아래 WIKI 조합·인터랙션 규칙은 적용 대상이 없으며 기록으로만 남긴다. 창고 연결은 설정 › 지식, 메모리는 설정 › 메모리 탭이 맡는다.

WIKI는 전역 N1 안에서 주제 문서를 읽는 표면과 초안을 만드는 표면을 구분한다. 판정 폭은 글로벌 사이드바를 제외한 **WIKI 콘텐츠 영역**이다. 1120px 이상은 문서 탐색 240px + 본문 유동 + 출처 240px, 880~1119px는 탐색 240px + 본문과 접을 수 있는 출처, 880px 미만은 목록 또는 본문 한 면과 명시적 `목록으로` 이동을 사용한다. 패널 간격은 `spacing.4`, 본문 패딩은 `spacing.6`이며 좁은 면은 `spacing.4`다. 이는 로컬 macOS 창의 적응형 배치이며 모바일 지원을 뜻하지 않는다.

자료 추가 폼은 읽기 화면의 상주 카드가 아니라 `Dialog`로 연다. WIKI 문서 탭의 primary는 `새 문서`, 편집 중에는 `문서 저장`이다. `정리 세션 열기`는 보조 행동으로 창고 루트의 에이전트 세션을 연다. 문서 탭은 창고 전체 Markdown의 검색·2D 그래프·렌더링·원문 편집·휴지통 삭제를 제공하고 자료·메모리 탭을 유지한다(파일 위키 설계). 공존 기간의 초안 작성(S-19)은 자료 필터의 선택 행동으로 남기고 현재 단계의 주 행동 한 개만 primary로 표시한다. 본문·목록·출처가 각자 스크롤하며 sticky 도구줄은 포커스된 입력과 본문을 가리지 않는다. 세부 화면 계약은 Appendix B의 S-14·S-19에 둔다.

## Elevation & Depth

보더 우선, 그림자 최소. 5단계:

| level | 용도 | bg | border | shadow |
|-------|------|-----|--------|--------|
| 0 | 앱 배경 | `#0d0d0d` | none | none |
| 1 | 카드/패널 | `#161616` | `1px #2a2a2a` | `0 1px 3px /.4` |
| 2 | 드롭다운/팝오버 | `#1f1f1f` | `1px #2a2a2a` | `0 4px 12px /.5` + inset highlight |
| 3 | 모달/다이얼로그 | `#1f1f1f` | `1px #3f3f46` | `0 8px 32px /.6` + inset |
| 4 | 커맨드 팔레트/최상위 | `#1f1f1f` | `1px #3f3f46` | `0 16px 48px /.7` + inset |

inset `0 1px 0 rgba(255,255,255,0.04~0.06)` 상단 하이라이트는 elevation 2+ 에서만 — 그림자 없이 "들린" 느낌을 내는 유일한 수단.

## Shapes

`rounded`: sm(4) badge/input · md(6) button/card-inner · lg(8) card/panel/modal · xl(12) dialog · full(9999) avatar/chip/toggle. 일관성: 같은 위계의 요소는 같은 radius.

## Components

- **SideQuestion** — 기존 코드 열에 범위·상태 → 독립 대화 → 질의 입력창을 조합한다. `border` 1px 구분선과 `typography.fontSize.md` 본문, PaneChip 진입·Tabs 전환·MetaTag 범위·Button 전송을 사용한다. 질의는 `따로 질문 / 질문 보내기`, 메인은 `메인에 요청 / 메인에 보내기`로 구분한다. 열기→질의 입력, 닫기→진입 버튼, 첨부→메인 입력으로 포커스를 옮긴다. Enter는 포커스한 입력창만 전송하고 IME 조합 중에는 전송·닫기를 하지 않는다. 패널 열기·마운트는 실행 트리거가 아니며 닫힌 패널은 무거운 Markdown·목록 계산을 하지 않는다.
- **QuestionReference** — 메인 입력창에 `참고자료 · 질문 제목 · 선택한 내용 분량`과 펼치기·편집·제거를 둔다. `border` 1px, `typography.fontSize.sm`을 쓴다. 펼치면 실제 전달할 내용과 출처를 그대로 보여준다. 기존 글·캡처를 보존하고 자동 전송하지 않는다. 참고자료와 사용자 적용 지시를 구분하며, 원문·선택 내용은 키보드로 읽고 수정할 수 있다. 제거 버튼의 aria-label에는 자료명을 포함한다. 두 컴포넌트 모두 기존 팔레트·폰트·모션을 따른다.

YAML front matter `components` 가 normative 정의. 핵심 요약:

- **WIKI 조합 규칙** — 기존 `Sidebar`·`Tabs`·`Input`·`Button`·`IconButton`·`Badge`·`MetaTag`·`Dialog`·`Toast`를 재사용한다. 별도 팔레트·버튼 variant는 만들지 않는다. `문서 / 자료`는 FilterSegment 하나, `검토할 초안`은 개수 Badge(공존 기간만), 기존 Wiki 공간·스캔·연결 복구는 `관리` 메뉴 항목이다. 문서와 원자료 목록은 분할선이 있는 행, 제목·내용은 읽기 영역이다. 상태는 색만 쓰지 않고 텍스트로 병기하며 범위는 MetaTag로 표시한다. 목록의 선택 checkbox와 검색·본문 입력은 명시적 label을 가진다.
- **WIKI 인터랙션 8상태** — default는 기존 컴포넌트, hover는 surfaceRaised, focus-visible은 기존 primary outline, active는 기존 선택 배경·aria-pressed/aria-selected, disabled는 기존 disabled 배경·text.muted와 사유, loading은 폭을 유지한 진행 문구와 aria-busy, error는 status.failed 텍스트·아이콘과 해당 동작 재시도, success는 저장 응답 확인 뒤 문서 열기와 status.done 안내로 파생한다. 원자료 추가의 success를 위키 저장 성공으로 표시하지 않는다. 비동작 본문·분할선·설명용 MetaTag에는 8상태를 강제하지 않는다. 라이트 테마의 링크·활성 텍스트는 배경 대비가 확보된 기존 foreground 역할을 사용하고 밝은 Teal을 작은 본문 글자에 강제하지 않는다.

- **SessionNavigator / EditorSearch** — Shift 두 번은 창의 목적을 따른다. 메인 창에서는 **그룹 → 프로젝트 → 세션 트리**로 소속을 찾고, 별도 에디터에서는 **전체·파일·내용 검색**으로 편집 대상을 찾는다. 검색창의 키보드 순서는 실제 화면 순서와 일치해야 하며, 취소는 원래 작업으로 돌아간다. 화면 계약은 S-17·S-18에 둔다. 메인 ⌘K·⌘P의 기존 진입점은 유지한다.
- **SessionHomePicker** — 세션홈 이어받기 선택창은 `SessionNavigator`의 팝오버·행·키보드 문법을 그대로 쓰되 **프로젝트 → 세션** 2단 트리다(그룹 단 없음). 현재 저장소 프로젝트가 맨 위에 펼쳐진 채 열리고, 나머지는 최근 활동순으로 접힌다. 검색은 서버 조회(200ms 디바운스)이고 검색 중에는 전부 펼친다. ↑↓ 순환·트리 행에서 ←→ 접힘/펼침·Enter는 프로젝트를 여닫고 세션을 고른다. IME 조합 확정 Enter로 고르지 않는다. 최근 활동·다른 저장소 경고 단계와 평문 렌더 규칙은 설계를 따르고, 트리 규칙은 설계가 정본이다.

- **Button** — primary(teal-500→hover 600→active 700, disabled neutral-800/500) / outline(teal border+text, hover wash) / ghost(secondary→neutral-800) / destructive(failed 톤). 높이 36px, focus `outline 2px teal-500 offset 2px`.
- **IconButton / PaneChip (헤더 손잡이 2형)** — 세션 헤더·툴바·카드 헤더의 소형 컨트롤. IconButton은 아이콘 단독 토글, PaneChip은 소환 칩(파일·프리뷰·Diff·에디터 창·터미널)으로 **icon(14px)+label을 병기**한다 — 눌린 칩의 라벨이 곧 열리는 탭의 라벨이다. 둘 다 기본 `text.secondary`(muted는 disabled 전용, Don't #3), hover는 `surfaceRaised` 배경, **눌림·활성은 teal 10% wash + `primaryBright`** — 사이드바 활성·코드 열 활성 탭과 같은 문법 하나로 통일한다. 중앙 잔여 < 1080px이면 칩 라벨을 접고 아이콘만 남긴다(행 폭만 바뀌므로 히스테리시스 불요).
- **Card** — `surface` bg + `1px border` + `lg` radius + `spacing.6` padding. 인터랙티브 hover: border `neutral-700` + elevation-2. relaxed: padding `spacing.8` + 미세 그림자.
- **Input** — `surface` bg + `border`. focus: teal border + `3px` teal ring. error: failed border + red ring. 높이 36px, `sm` radius.
- **Scrollbar** — 에디터 레지스터를 따른다: 트랙/거터를 칠하지 않고 콘텐츠 위에 중립 반투명 thumb만 띄운다(VS Code/Zed 문법). 피드백은 **농도 3단(rest→hover→drag) + 두께 확장(6→8→10px)**으로만 — Teal은 CTA·포커스 신호라 스크롤바에 쓰면 신호가 희석된다. 12px 히트 영역, pill radius, Monaco slider에도 동일 토큰(`--c-scrollbar-*`) 적용. SidePanel의 스크롤바 숨김 범위는 유지. 구현은 `::-webkit-scrollbar` 전용 — 표준 `scrollbar-width`/`scrollbar-color`를 non-auto로 선언하면 webkit 커스텀 규칙이 전부 무시되어 WKWebView가 네이티브 흰 thumb로 폴백한다(금지).
- **SelectChip (값 하나를 고르는 칩+팝오버)** — Composer 툴바의 세션 단위 컨트롤(레포·에이전트·모델·Effort)은 전부 같은 칩+팝오버 패턴을 쓴다: `xs` 칩(icon+label+chevron, `border`, hover `borderStrong`) → 칩 위로 열리는 elevation-2 팝오버 메뉴("기본 (설정값)" 항목 최상단, 현재 값에 check 아이콘, "— 이 세션에만 적용" 캡션). 오버라이드 활성 시 칩은 `primaryBright` 텍스트 + `primary/50` 보더로 구분한다. **네이티브 `<select>` 금지** — OS 팝업은 다크 표면 위계·토큰을 따르지 않아 툴바에서 이질적이다. (Orca ADE의 Composer 칩 패턴 흡수, designs/0012) **후보가 수십을 넘을 수 있는 칩**(브랜치·레포·모델)은 메뉴 최상단에 고정 검색 필드를 두고 목록만 스크롤시킨다 — 필드는 후보 수와 무관하게 상시이며(자리가 조건에 따라 움직이면 근육 기억이 서지 않는다), 매치 구간은 색이 아니라 `text.primary` + `medium`으로 밝히고, 헤더에 `표시/전체` 카운트를 병기해 좁혀진 정도를 숫자로 준다. 걸러낸 결과를 **재정렬하지 않는다** — 원래 순서(브랜치는 최근 커밋순)가 곧 사용자가 기대하는 순서다.
- **IsolationPicker (격리 선택 칩)** — SelectChip과 같은 칩+팝오버 문법이되, 각 항목이 **라벨 + 캡션 2줄**이다. 캡션이 이 컴포넌트의 존재 이유다: 격리 실행의 워크트리와 브랜치는 승인하든 버리든 함께 지워지는데 "워크트리"라는 라벨은 그 사실을 전혀 담지 않는다. **결과가 파괴적인 선택은 그 결과를 라벨 옆에 적는다.** 전에는 눌러서 3-state를 도는 순환 칩이었고 문제가 둘이었다 — 다음 상태를 예측할 수 없고(순환은 SelectChip 패턴이 아니다), 삭제를 말하지 않았다. **순환 칩을 새로 만들지 말 것**: 선택지가 셋 이상이면 팝오버다.
- **Badge (상태 4종)** — `xs` 11px, `full` radius, `2px 8px`. **아이콘(8px)+텍스트 항상 병기**. 에이전트 카드 왼쪽 `2px`(compact 3px) 상태 스트립 상시.
- **MetaTag (비상태 태그)** — 상태가 **아닌** 분류·범위·정책을 다는 `xs` 테두리 태그. 스킬의 `global`/`project`, 메모리의 `status`·`휴면`·`항상 적용`이 전부 여기에 속한다. **채우지 않고 테두리로만 구분**하며, `accent`(`primaryBright` 텍스트)는 "기본을 덮는다"는 신호에만 쓴다 — project 스킬이 global을 덮고, `must_apply`가 relevance 게이트를 덮는다. Badge를 끌어 쓰지 않는 이유는 하나다: **색이 곧 작업 상태**라는 계약을 비상태 표시가 갉아먹기 때문이다. 같은 이유로 아이콘 병기 의무도 없다(WCAG 1.4.1은 상태를 색 단독으로 말하지 말라는 규칙이다). 클릭 대상이 아니므로 `text.muted`가 허용된다 — Don't #3은 클릭 가능한 텍스트에 대한 규칙이다.
- **SkillCard (스킬 뷰어 카드)** — 스킬 한 줄을 담는 두 줄 카드. 첫 줄에 `/이름 <argument-hint>`와 오른쪽으로 `MetaTag` 묶음(크기 → 실적 → 발동 방식 → 가용성), 둘째 줄에 설명 2줄 clamp. **액션 버튼이 없다** — 스킬의 정본은 각 에이전트 디렉터리이고 Praxis는 저장하지 않으므로, 추가·삭제·가져오기를 두면 고칠 수 없는 것을 고칠 수 있는 척하게 된다(설계 0052). 여는 동작은 본문 펼침 하나뿐이다. 크기는 **확장 시 프롬프트에 들어가는 양**이라 claude 네이티브로 도는 스킬에는 실제 비용이 아니다 — 렌즈가 codex·agy일 때만 `accent`로 든다.
- **Sidebar** — `surface` bg + right border. active: text `primaryBright`, icon `primary`, bg `teal/10%`, left-bar `2px primary`. section-label: 11px semibold uppercase.
- **Tabs** — active 하단선 `2px primary` + text `primaryBright`.
- **FilterSegment (목록 범위 필터)** — 목록 위 한 줄에 놓이는 배타적 세그먼트 그룹. 스킬 출처(`전체 / Claude / Codex / …`)와 메모리 상태(`전체 / 검토 대상 / 후보 / …`)가 **같은 컴포넌트**다. **탭은 면을 바꾸고 세그먼트는 같은 면의 범위를 좁힌다** — 그래서 하단선(Tabs)이 아니라 **teal 10% wash + `primaryBright`** 문법을 쓴다(PaneChip·사이드바 활성과 같은 언어, Don't #13). 각 세그먼트는 라벨 뒤에 **개수를 병기**하고, **개수 0인 세그먼트는 렌더하지 않는다**(`전체`와 **현재 선택된 칸**은 예외 — 눌린 칸이 검색 중에 사라지면 지금 어디에 서 있는지 알 수 없다). 개수는 **다른 축의 필터를 먼저 적용한 뒤** 센다 — 검색어·프로젝트 범위를 무시한 수를 병기하면 눌렀을 때 빈 목록이 나온다. `전체`는 필터를 끄는 모드가 아니라 **같은 기준으로 묶어 보는 모드**이므로 항목에 `groupHeader`를 달아 구분을 유지하고, 하나로 좁히면 눌린 세그먼트가 이미 그 이름을 말하므로 헤더를 뗀다. 구현은 `src/components/FilterSegment.tsx` 하나다 — 화면마다 손으로 다시 그리면 개수 병기와 roving tabIndex가 조용히 빠진다. **정렬 축을 여기 섞지 않는다** — 세그먼트는 무엇을 보느냐이고 정렬은 어떤 순서로 보느냐다. 정렬은 `SelectChip`으로 오른쪽에 따로 둔다. 축이 **렌즈**(누가 쓰는가)이면 `전체`에 `groupHeader`를 달지 않는다 — 소속은 이미 행마다 가용성 태그가 말한다.
- **Dialog** — `neutral-850` bg, `borderStrong`, `xl` radius, elevation-3, overlay `rgba(0,0,0,.75)+blur(2px)`.
- **Toast** — `neutral-850`, `lg` radius, elevation-2, 좌측 3px accent strip(default teal / success / warning / error).
- **MemoryReviewCard** — 메모리 검토 목록의 카드 한 장. **단일 열**(`max-w-3xl`)이고 리스트/상세 2열로 가르지 않는다 — 검토는 목록을 훑으며 하는 일이라, 상세를 옆 면에 띄우면 방금 본 이웃 항목이 사라진다. 그래서 사용이력·근거·버전은 **카드 안에서 펼치는 인라인 아코디언**이고 카드마다 하나씩만 열린다. 왼쪽에 지식 유형(claim/observation/decision/convention/abandoned/pitfall) 고정폭 라벨, 가운데 본문, 그 아래 `MetaTag` 줄과 readiness 한 줄, 오른쪽에 액션 세로 스택이 놓인다. **status는 Badge가 아니라 MetaTag다** — 메모리 수명주기는 작업 상태 4종과 다른 축이고, 같은 색 언어를 쓰면 둘이 섞인다. 액션 중 `직접 확인 후 승인`만 `primaryBright`로 든다: 사람이 확인했다는 유일한 행위이고 나머지는 조회다. 일괄 선택 체크박스는 `legacy_unverified`에만 둔다 — 일괄 처리는 이관이라는 한 국면에서만 정당하고, 승인은 언제나 한 건씩 사람이 한다. 보관·복원은 hover 인라인으로 노출하지 않는다(스크롤 중 오조작이 승인 기록을 흔든다).
- **WorkContextPanel** — 세션 위 `FloatingActivityChannel`과 우측 `SidePanel`의 `작업 정보` 탭이 공유하는 콘텐츠. 작업 목표 → 환경(로컬/원격·브랜치·base·worktree·상태) → 검증된 `git diff --stat` → 현재 활동 → 하위 에이전트 → 최근 활동 순으로 보여 준다. 경로·브랜치는 code register와 truncate+전체 `title`을 사용한다. 로딩/오류는 해당 행 안에서만 표면화하고 나머지 검증된 메타데이터는 유지한다.
- **SessionWorkspace** — 세션 화면의 영역 계약. 헤더(44px) 아래를 `파일 트리 | 세션 | 코드` 세 열이 나눠 갖고, 그 아래로 터미널 도크·리뷰 바·컴포저가 전폭으로 깔린다. 코드 열은 **상주하지 않고 부름을 받는다** — 헤더 버튼·⌥⌘S·팔레트가 열고, 파일 열기나 "Diff 보기"처럼 코드가 목적지인 동작은 자동으로 연다(ADR 0111). 폭이 모자라면 탭으로 접되 진입 900 / 복귀 840으로 임계를 갈라 경계에서 진동을 막는다. 소환된 면(코드 열·터미널 도크·플로팅 채널)은 **자기 위에 닫기**를 갖는다 — "연 버튼을 다시 누른다"는 기억해야 하는 지식이지 보이는 손잡이가 아니므로, 손잡이 재클릭과 단축키는 보조 경로다.
- **FloatingActivityChannel** — **세션 열 안쪽** 우상단 16px에 뜨는 300px 비모달 오버레이. `surfaceRaised` + `borderStrong` + elevation-3 + `xl` radius로 층을 분리하고, 떠 있는 동안 세션 열이 `paddingRight: 332px`으로 같은 폭을 비운다 — 겹쳐 두면 대화 오른쪽 끝이 카드 아래로 깔린다. 화면이 아니라 **그 열의 레이어**이므로 헤더·파일 트리·코드 열·컴포저 위에는 걸치지 않는다. 코드 열이 열리거나 중앙 잔여가 692px 미만이면 숨고, 같은 카드가 코드 열의 고정 `작업정보` 탭으로 이동한다(승격과 흡수는 대칭이다). 접는 손잡이는 환경 헤더의 ✕이고, 접힌 자리(세션 우상단)에는 **재열기 핸들**이 남는다 — 여는 곳과 나타나는 곳이 같은 자리여야 인과가 보인다. 컨텍스트 게이지는 보조 진입점이다(잔량은 손끝에, 설계 0044).
- **Village** — 홈과 플로팅 창이 공유하는 조망. 최대 8개 작업을 **위에서 내려다보는 마을**에 투영한다. 화면은 고정 타일 그리드(24×12, 타일 16px)이고, 컨테이너 크기에 따라 **정수배 1x/2x/3x**로만 확대하며 남는 공간은 레터박스로 흘린다 — 소수배는 픽셀 경계를 뭉갠다. 준비소·작업장·리뷰 데스크·완료 광장이 왼쪽에서 오른쪽으로 놓이고 골목이 대로 아래 붙으며, 캐릭터가 **어느 구역에 서 있는지가 곧 작업 상태**다. 구역은 지붕 있는 상자가 아니라 **열린 영역**이다 — 지붕을 씌우면 안에 선 캐릭터가 보이지 않아 위치로 상태를 읽을 수 없다. 건물은 구역 뒤쪽 **2행**(지붕 행 + 벽 행)을 차지하고 그 앞이 열린 마당이다. 배경은 명도를 네 단으로 벌려 형태를 만들고(골목 → 잔디 → 도로 → 구역 바닥), 재질 경계에는 전이 타일을, 건물·오브젝트·캐릭터 발밑에는 그림자를 둔다 — 같은 명도에 몰린 면들은 아무리 디테일을 얹어도 서로 녹는다. 자리는 선언된 타일 좌표이고 구역마다 정원(8) 이상을 둔다. 캐릭터는 **4방향**(down·up·left·right) 스프라이트를 가지며 정지 중에는 정면을 본다 — 뒤통수만 보이는 마을에는 상태를 읽을 얼굴이 없다. 이동은 두 가지다: 상태가 바뀌면 **축 정렬 웨이포인트 경로**로 다음 구역까지 걸어가고(대각선은 없다 — 그 프레임이 자산에 없다), 그 외에는 **자기 구역 안에서 2~3타일 배회**한다. 배회는 구역을 절대 벗어나지 않는다. hover·포커스·선택 중이면 **그 자리에 즉시 멈춘다** — 자리로 복귀시키면 누르려는 순간 캐릭터가 도망간다. 정체성은 역할이 정하고 상태와 분리된다: 계획·탐색·구현·테스트·리뷰가 캐릭터 외형과 `프로젝트-role-task번호`(예: `praxis-builder-07`) 이름을 결정하며, 상태가 바뀌어도 **그 둘은 변하지 않는다.** 캐릭터 자체가 포커스 가능한 버튼이고 이동 중에도 누를 수 있다. 작업 중인 캐릭터는 머리 위 말풍선에 지금 쓰는 도구를 띄운다. 상태는 구역 위치 + 색 + 기호 + 텍스트로 중복 인코딩하고, 역할색은 이름표의 1~3px identity strip에만 쓴다. 픽셀 자산은 **논리 배율 1:1로 굽고** 확대는 CSS가 정수배로 한다 — 시트를 미리 키우면 배율이 하나로 고정된다. 이름표·간판·카드 같은 텍스트는 맵 배율을 따르지 않는다: 픽셀 폰트가 아니라 확대하면 흐려지고, OS 텍스트 크기 설정도 존중해야 한다.

## Do's and Don'ts

WIKI에서는 자료를 선택한 문맥을 초안과 저장된 문서까지 이어준다. 자료 추가·초안 생성·MD 저장의 성공 상태를 구분한다. 원자료 목록을 문장별 검증 인용으로 표시하거나, 저장 결과를 모른 채 같은 저장을 반복하는 동작을 만들지 않는다.

**Do**
1. 카드 왼쪽에 2px(compact 3px) 상태 스트립을 항상 표시 — 배지만으론 시선 이동 비용이 크다.
2. 모든 상태 배지에 아이콘 + 텍스트 병기 — 색 단독은 WCAG 1.4.1 위반.
3. 코드/경로/타임스탬프/Task ID는 JetBrains Mono, UI 레이블/버튼/헤더는 Inter로 분리.
4. Teal은 CTA·포커스링·활성 탭·활성 nav에만 사용한다. 단, Village의 implementer identity strip은 컴포넌트 한정 예외다.
5. 드롭다운/팝오버/모달에만 inset 상단 하이라이트 — 다크에서 "들린" 느낌의 유일한 수단.
6. Village 캐릭터는 **자기 구역 안에서만** 움직인다 — 구역이 곧 상태이므로 이탈은 상태 오독이다. 구역 간 이동은 상태 전이에만, 구역 내 배회는 2~3타일까지 허용한다. 배회 중에도 누를 수 있어야 하므로 **hover·포커스·선택 중이면 그 자리에 즉시 멈춘다** — "움직이는 표적은 누를 수 없다"는 원래 근거는 금지가 아니라 정지 조건으로 지킨다(ADR 0113가 ADR 0102를 이렇게 개정했다).
7. 픽셀 자산은 표시 크기에 맞춰 미리 굽는다(`tools/sprites/build-village-scene.py`, `build-village-characters.py`) — `image-rendering: pixelated`는 확대 전용이고, 큰 원본을 런타임에 축소하면 얼굴이 뭉개진다. 저해상도 도트는 코드로 찍는다: 파라미터화되므로 역할 6종 × 프레임 7개가 한 정의에서 나온다.
8. 탑다운 구역은 **열어 두고** 캐릭터를 그 안 선언된 자리에 세워라 — 지붕을 씌우면 안이 안 보여 위치로 상태를 읽는 규율이 무너진다. 자리는 정원 이상을 두고 lint로 지킨다.
9. 떠 있는 것에는 **자리를 예약해 준다** — 세션 위 플로팅 채널은 그 열의 `paddingRight`로 같은 폭(332px)을 비운다. 예약 없는 겹침은 대화의 오른쪽 끝을 영구히 덮는다. absolute 기준이 padding box라 예약해도 카드 위치는 밀리지 않는다. 예약은 그 열 밖에도 미친다 — 전폭 헤더의 액션 줄은 같은 선(320px)까지 물러서 자기 열의 오른쪽 끝에 정렬한다.
10. 플로팅 레이어는 **그것이 속한 영역 안에** 마운트한다 — 작업정보는 세션의 것이므로 세션 열 안에 산다. 화면 기준으로 띄우면 헤더 버튼과 컴포저까지 덮는다.
11. 소환된 표면에는 **그 표면 위에** 닫기를 둔다 — "연 버튼을 다시 누른다"만으로 닫게 하면 되돌아오는 길이 기억에만 존재한다. 재클릭·단축키는 보조 경로다.
12. 범위 필터 줄은 **`FilterSegment` 컴포넌트 하나로** 만든다 — 개수 병기·0건 미렌더·roving tabIndex를 화면마다 손으로 지키게 하면 반드시 갈라진다. 실제로 스킬과 메모리가 한동안 다른 문법으로 갈라져 있었다.
13. 상태가 아닌 분류·범위·정책에는 **`MetaTag`**를 쓴다 — 스킬의 `global`/`project`, 메모리의 `status`·`휴면`·`항상 적용`. 형태(테두리)는 하나, 색은 두 단(기본/accent)뿐이다.
14. 후보 수가 **데이터에 따라 늘어나는** 피커에는 검색을 상시로 둔다 — 스크롤은 이름을 이미 아는 사람에게도 O(n)이다. 필드는 메뉴 최상단에 고정하고 목록만 스크롤시키며, 열리는 즉시 포커스를 준다. 값의 집합이 미리 정해진 범위 필터는 이쪽이 아니라 #12(`FilterSegment`)다 — 셀 수 있으면 세그먼트, 셀 수 없으면 검색이다.

**Don't**
1. CTA·카드에 배경 그라디언트/글로우 금지 — Teal 신호가 희석된다.
2. 상태색(blue/amber/green/red)을 버튼·링크·내비에 쓰지 마라 — 상태 전용. 사용자가 지정한 그룹 식별색은 아래 #5의 장식 범위에 한해 예외이며 상태 토큰을 재사용하지 않는다.
3. `text.muted(#71717a, 3.5:1)`를 클릭 가능 텍스트/레이블에 쓰지 마라 — AA 미달, 비활성/플레이스홀더 전용.
4. 카드 구분에 drop-shadow 금지 — `1px #2a2a2a` 보더 + 배경 명도차로 충분. 그림자는 모달/팝오버 elevation 예외만.
5. 두 번째 유채색 액센트 추가 금지 — 에이전트 브랜드색(Claude/Cursor 로고)과 **파일 타입 아이콘의 언어 브랜드색**은 아이콘 이미지 내부에 한정. 이미지 밖으로(텍스트·보더·배경) 새면 그때부터 위반이다. **사용자가 지정한 사이드바 그룹 식별색**은 별도 예외다: 이름 있는 6개 프리셋을 그룹 왼쪽 3px 띠와 옅은 제목 배경, 해당 선택 메뉴의 색 견본에만 허용한다. 그룹 제목·개수는 중립 `text` 색으로 읽히고 프로젝트 목록 배경·작업 상태·활성 항목 색은 유지한다. 드롭 강조가 식별색 배경보다 우선한다. 기본값 복원과 키보드 선택·취소를 제공한다. 범위·호환성·검증 계약.
6. Village에 네온 글로우·홀로그램 그라디언트를 추가하지 마라 — 미래감은 구조와 데이터 신호로 표현한다. 배회는 허용되지만 **자기 구역 밖으로 나가는 배회는 금지**다: 그 순간 위치가 상태를 말한다는 계약이 깨진다.
7. 맵 배율에 소수배를 쓰지 마라 — 16px 타일이 반쯤 뭉개져 픽셀 아트를 고른 이유가 없어진다. 남는 공간은 늘리지 말고 레터박스로 흘려라.
8. 작업 요약 말풍선을 캐릭터 위로 열지 마라 — 홈은 스크롤 컨테이너라 위로 열면 잘리고 자기 섹션 헤더를 가린다. 방 안쪽을 향해 옆으로 연다.
9. Composer 툴바에 네이티브 `<select>`를 쓰지 마라 — OS 렌더링 팝업은 토큰 밖이다. SelectChip 팝오버 패턴을 따른다.
10. 스크롤바 thumb에 Teal·상태색을 쓰지 마라 — 스크롤 위치는 CTA도 상태도 아니다. 중립 농도 변화(rest→hover→drag)로만 피드백한다.
11. 같은 내용을 두 자리에 동시에 두지 마라 — 작업정보는 플로팅 채널 또는 코드 열 탭 한 곳에 표시한다. 변경 목록은 오른쪽 Diff 탭, 본문은 세션 자리에 표시한다. 목록을 선택해도 목록이 사라지지 않게 두 영역의 목적지를 분리한다(설계 0064).
12. 곁눈으로 보는 상태(작업정보)와 의도를 갖고 여는 도구(파일·프리뷰)를 **닫을 수 있는 같은 슬롯에** 넣지 마라 — 도구를 열 때마다 상태가 밀려난다. 코드 열의 작업정보 탭이 닫히지 않는 이유다.
13. "지금 보고 있는 곳" 표시에 두 언어를 쓰지 마라 — 헤더 소환 칩·코드 열 활성 탭·사이드바 활성 nav는 전부 같은 문법(teal 10% wash + `primaryBright`)이다. 클릭 가능한 요소의 활성 신호가 색 단독이어서도 안 된다 — 배경면이 함께 바뀌어야 눌림이 보인다.
14. **Badge를 상태 아닌 표시에 끌어 쓰지 마라** — 상태 4종(running/awaiting/question/done/failed)에만 쓴다. 지식 유형·수명주기·범위·정책까지 Badge로 달면 "색이 곧 작업 상태"라는 계약이 무너진다. 그 자리는 `MetaTag`다.
15. **면을 바꾸는 컨트롤과 범위를 좁히는 컨트롤에 제3의 문법을 만들지 마라** — 전환은 Tabs(하단선 2px), 범위는 FilterSegment(teal wash)다. pill 배경 같은 중간 형태를 새로 들이면 사용자는 누르기 전에 무엇이 바뀔지 알 수 없다. 메모리 탭의 `메모리 / 자기개선`이 정확히 그 실수였다.
16. 걸러낸 결과를 **조용히 잘라내지 마라** — 상한을 두면 사용자는 그것이 없다고 읽는다. 자를 수밖에 없으면 몇 개를 감췄는지 말하고 좁히는 법을 준다.
17. 필터 결과를 **관련도순으로 재정렬하지 마라** — 같은 목록이 질의마다 다른 순서로 서면, 한 글자 더 치는 동안 방금 눈으로 짚은 항목이 사라진다. 원래 순서를 지키고 매치 구간만 밝힌다.

---

## Appendix A — Tooling Notes

- 2026-09-28 Wiki 채널 제거: 위키 그래프 전용 범주형 식별색 `--c-cat-*` 3종을 토큰과 규칙에서 지웠다. 두 번째 유채색 금지에 남은 예외는 없다.

- 2026-09-08 WIKI 한정 Targeted 갱신: 주제별 MD 결과물 요구를 반영해 읽기·단계형 작성 규칙과 S-14·S-19를 정의했다. YAML 시각 토큰은 동일하므로 DTCG·기존 토큰 preview는 재생성하지 않는다. PATH/local bin에 design.md CLI가 없어 설치 없이 수동 YAML·참조·구조·변경분 검증을 적용한다. 실제 앱·모델 품질·밝은/어두운 테마 렌더 검증은 구현 후 범위다. 결정과 검증 기록은 WIKI 작성 설계에 둔다.
<!-- critique: P5 H4 E4 S5 R5 C5 · pass 1 · WIKI 변경분 계약 검토, 런타임 시각 평가 아님 -->

- 2026-09-08 그룹 식별색 예외는 해당 규칙과 기존 컴포넌트에만 한정해 갱신했다. 설치형 CLI probe는 실행하지 않았으며 토큰·제목 구조·참조의 보존은 diff로 검토했다. 이 컴포넌트의 밝은/어두운 테마 검증 범위와 관측 결과는 그룹 색상 계약에 기록한다.
- **design.md CLI: UNAVAILABLE** (probe 실패) → lint/export는 수동 폴백으로 수행.
  - lint: Contract Validator 수동 체크리스트로 대체 (broken-ref 없음, primary/typography 존재, 상태색 명암비 AA 확인 완료).
  - export: `design-tokens.json`(DTCG)을 수동 작성. CLI 복구 시 `export --format dtcg DESIGN.md`로 재생성 권장.
- 라이트 테마는 범위 외(Phase 2 이후). `preview.html`은 다크 정본 카탈로그.

- 2026-09-08 탐색/검색 한정 갱신: `SessionNavigator`·`EditorSearch` 계약과 S-17·S-18을 추가했다. 설치된 design.md CLI가 없어 기존 수동 폴백을 적용하며, 임시 도구 설치는 하지 않는다. YAML 파싱·새 참조·화면 링크를 검증한다. 기존 색·폰트·간격·라운드·그림자 값은 유지하므로 DTCG 시각 토큰 재추출은 필요하지 않다.
<!-- critique: P5 H4 E4 S5 R5 C5 · pass 1 · 탐색/검색 계약 한정, 기존 시각 토큰 유지 -->

## Appendix B — Key Screens Index

와이어프레임 정본: `docs/designs/wireframes/INDEX.md`

| Screen | Priority | Phase | Wireframe |
|--------|----------|-------|-----------|
| S-01 Task Creation | P0 | 1~2 | 0001-task-creation |
| S-02 Orchestration Dashboard | P0 | 2 | 0002-orchestration-dashboard |
| S-03 Terminal View | P0 | 0~1 | 0003-terminal-view |
| S-04 Diff Viewer | P1 | 2 이후 | 0004-diff-viewer |
| S-05 Memory View | P1 | 3~4 | 0005-memory-view |
| S-06 Repository Settings | P0 | 0~1 | 0006-repository-settings |
| S-07 Self-Improvement Review | P2 | 5~6 | 0007-self-improvement-review |
| S-08 Workspace Task Info Panel | P0 | Current | 0008-workspace-task-info-panel |
| S-09 Skills View | P1 | Current | 0009-skills-view |
| S-10 Branch Picker Search | P1 | Current | 0010-branch-picker-search |
| S-17 Session Navigator | P1 | Current | 0017-session-navigator |
| S-18 Editor Search | P1 | Current | 0018-editor-search |
| S-14 WIKI / Personal Knowledge Vault | P0 | Designed | 0014-personal-knowledge-vault |
| S-19 Wiki Authoring | P0 | Designed | 0019-wiki-authoring |
| S-21 Side Question | P1 | Current | 0021-side-question |
