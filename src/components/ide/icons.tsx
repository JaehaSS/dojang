// 의존성 없는 인라인 SVG 아이콘 (Tabler outline 근사). currentColor 상속.
type Name =
  | "grid"
  | "folder"
  | "database"
  | "bulb"
  | "plug"
  | "settings"
  | "plus"
  | "x"
  | "chevronRight"
  | "chevronLeft"
  | "chevronDown"
  | "terminal"
  | "refresh"
  | "save"
  | "diff"
  | "home"
  | "branch"
  | "send"
  | "sparkle"
  | "desktop"
  | "check"
  | "chart"
  | "play"
  | "scale"
  | "stop"
  | "at"
  | "more"
  | "code"
  | "copy"
  | "chat"
  | "clock"
  | "search"
  | "folderOpen"
  | "file"
  | "fileText"
  | "fileCode"
  | "braces"
  | "image"
  | "lock"
  | "markdown"
  | "palette"
  | "eye"
  | "eyeOff"
  | "mic"
  | "popout"
  | "panelRight"
  | "splitRight"
  | "splitDown";

// 각 아이콘은 서브패스 배열 — 문자열 split 파서 없이 그대로 <path>로 렌더.
const PATHS: Record<Name, string[]> = {
  grid: ["M4 4h6v6H4z", "M14 4h6v6h-6z", "M4 14h6v6H4z", "M14 14h6v6h-6z"],
  folder: ["M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"],
  database: [
    "M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3z",
    "M4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6",
    "M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6",
  ],
  bulb: ["M9 18h6", "M10 21h4", "M12 3a6 6 0 0 1 4 10c-.7.7-1 1.5-1 2H9c0-.5-.3-1.3-1-2a6 6 0 0 1 4-10z"],
  plug: ["M9 7V3", "M15 7V3", "M7 7h10v4a5 5 0 0 1-10 0z", "M12 16v5"],
  settings: [
    "M12 9.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z",
    "M11 3h2l.5 2.6 1.8.8 2.3-1.4 1.4 1.4-1.4 2.3.8 1.8L21 11v2l-2.3.5-.8 1.8 1.4 2.3-1.4 1.4-2.3-1.4-1.8.8L13 21h-2l-.5-2.3-1.8-.8-2.3 1.4-1.4-1.4 1.4-2.3-.8-1.8L3 13v-2l2.3-.5.8-1.8L4.7 6.4 6.1 5l2.3 1.4 1.8-.8z",
  ],
  plus: ["M12 5v14", "M5 12h14"],
  x: ["M6 6l12 12", "M18 6L6 18"],
  chevronRight: ["M9 6l6 6-6 6"],
  chevronLeft: ["M15 6l-6 6 6 6"],
  chevronDown: ["M6 9l6 6 6-6"],
  terminal: ["M5 7l5 5-5 5", "M13 17h6"],
  refresh: ["M20 11a8 8 0 1 0-2.3 5.6", "M20 5v6h-6"],
  save: ["M6 4h10l4 4v12H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z", "M8 4v5h7V4", "M8 14h8v6H8z"],
  diff: ["M12 4v16", "M6 8H3", "M6 8l-3 3", "M6 8l-3-3", "M18 16h3", "M18 16l3 3", "M18 16l3-3"],
  home: ["M4 12l8-8 8 8", "M6 10v9h12v-9"],
  branch: [
    "M6 4a2 2 0 1 0 0 4 2 2 0 0 0 0-4z",
    "M6 16a2 2 0 1 0 0 4 2 2 0 0 0 0-4z",
    "M18 6a2 2 0 1 0 0 4 2 2 0 0 0 0-4z",
    "M6 8v8",
    "M18 10c0 4-6 3-6 6",
  ],
  send: ["M12 5v14", "M6 11l6-6 6 6"],
  sparkle: ["M12 4l1.6 4.4L18 10l-4.4 1.6L12 16l-1.6-4.4L6 10l4.4-1.6z"],
  desktop: ["M3 5h18v11H3z", "M8 20h8", "M10 16v4", "M14 16v4"],
  check: ["M5 12l5 5L20 7"],
  more: ["M12 6h0", "M12 12h0", "M12 18h0"],
  code: ["M8 7l-5 5 5 5", "M16 7l5 5-5 5"],
  // 복사 — 앞장 위에 겹친 뒷장. 뒷장은 가려지는 변을 그리지 않아 겹침이 드러난다.
  copy: [
    "M9 9h10a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V10a1 1 0 0 1 1-1z",
    "M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3",
  ],
  chart: ["M4 4v16h16", "M8 16v-4", "M12 16V8", "M16 16v-6"],
  play: ["M7 5l12 7-12 7z"],
  scale: ["M12 4v16", "M6 8h12", "M6 8l-3 6a3 3 0 0 0 6 0z", "M18 8l-3 6a3 3 0 0 0 6 0z", "M8 20h8"],
  stop: ["M7 7h10v10H7z"],
  at: ["M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z", "M16 12v1.5a2.5 2.5 0 0 0 5 0V12a9 9 0 1 0-3.5 7.1"],
  chat: ["M4 5h16v10H8l-4 4z"],
  clock: ["M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16z", "M12 8v4l3 2"],
  search: ["M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14z", "M20 20l-4.35-4.35"],
  // 파일 트리 — 접힌 폴더와 펼친 폴더는 실루엣이 달라야 한 눈에 갈린다.
  folderOpen: ["M3 8a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2v1", "M3 10h18l-2.2 8H5.2z"],
  file: ["M6 3h7l5 5v13H6z", "M13 3v5h5"],
  fileText: ["M6 3h7l5 5v13H6z", "M13 3v5h5", "M9 13h6", "M9 17h4"],
  fileCode: ["M6 3h7l5 5v13H6z", "M13 3v5h5", "M10 13l-2 2 2 2", "M14 13l2 2-2 2"],
  braces: [
    "M9 4c-2 0-2 2.5-2 4s0 2-2 2 2 .5 2 2 0 4 2 4",
    "M15 4c2 0 2 2.5 2 4s0 2 2 2-2 .5-2 2 0 4-2 4",
  ],
  image: ["M4 5h16v14H4z", "M9.5 10a1 1 0 1 0 0-.01", "M4 16.5l4-4 3 3 3.5-3.5L20 16"],
  lock: ["M6 11h12v9H6z", "M9 11V8a3 3 0 0 1 6 0v3"],
  markdown: ["M3 6h18v12H3z", "M7 15V9l2.5 3L12 9v6", "M16.5 9v4.5", "M14.5 12.5l2 2 2-2"],
  palette: [
    "M12 3a9 9 0 1 0 0 18 1.9 1.9 0 0 0 1.4-3.2 1.9 1.9 0 0 1 1.4-3.2h1.7A4.5 4.5 0 0 0 21 10.2C20.5 6.1 16.7 3 12 3z",
    "M7.5 12h0",
    "M9.5 8h0",
    "M14 7.5h0",
  ],
  eye: ["M2 12s3.6-6 10-6 10 6 10 6-3.6 6-10 6-10-6-10-6z", "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z"],
  eyeOff: [
    "M10.6 6.2A9.9 9.9 0 0 1 12 6c6.4 0 10 6 10 6a17 17 0 0 1-3.3 3.8",
    "M6.3 7.8A16.9 16.9 0 0 0 2 12s3.6 6 10 6c1.7 0 3.2-.4 4.5-1",
    "M9.9 9.9a3 3 0 0 0 4.2 4.2",
    "M4 4l16 16",
  ],
  mic: [
    "M12 3a3 3 0 0 1 3 3v5a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3z",
    "M5 11a7 7 0 0 0 14 0",
    "M12 18v3",
  ],
  popout: ["M12 6H7a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h9a2 2 0 0 0 2-2v-5", "M11 13 20 4", "M15 4h5v5"],
  // 우측 패널 — 플로팅 채널 재열기 핸들 전용 (layout-sidebar-right).
  panelRight: ["M4 5h16v14H4z", "M15 5v14"],
  // 에디터 분할 — 나뉜 자리를 그대로 보여 준다(layout-columns / layout-rows).
  splitRight: ["M4 5h16v14H4z", "M12 5v14"],
  splitDown: ["M4 5h16v14H4z", "M4 12h16"],
};

export function Icon({ name, size = 18 }: { name: Name; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {PATHS[name].map((d, i) => (
        <path key={i} d={d} />
      ))}
    </svg>
  );
}

export type IconName = Name;
