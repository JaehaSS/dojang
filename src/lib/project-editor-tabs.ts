interface ProjectTabs {
  paths: string[];
  active: string | null;
}

const tabsKey = (root: string) => `praxis-project-editor:${root}`;

export function loadProjectTabs(root: string): ProjectTabs {
  try {
    const saved = JSON.parse(localStorage.getItem(tabsKey(root)) ?? "null");
    if (saved && Array.isArray(saved.paths) && saved.paths.every((path: unknown) => typeof path === "string")) {
      return { paths: saved.paths, active: typeof saved.active === "string" ? saved.active : null };
    }
  } catch { /* 손상된 목록은 파일 접근이나 창 열기를 막지 않는다. */ }
  return { paths: [], active: null };
}

export function saveProjectTabs(root: string, paths: string[], active: string | null): void {
  localStorage.setItem(tabsKey(root), JSON.stringify({ paths, active }));
}
