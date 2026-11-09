import { useState } from "react";
import type { FileDiff } from "../lib/ipc";
import { Icon } from "../components/ide/icons";

// 모바일 diff 표시 — 읽기 전용. (설계 0013 §10 M1)
// 좁은 화면에서 split view는 불가능하므로 unified만 쓰고, 파일 단위로 접어둔다.
// 가로 스크롤은 각 파일 안쪽에서만 일어나야 한다 — 페이지 전체가 흔들리면 읽을 수 없다.

function lineClass(line: string): string {
  if (line.startsWith("+++") || line.startsWith("---")) return "text-text-muted";
  if (line.startsWith("@@")) return "text-primary-bright";
  if (line.startsWith("+")) return "bg-addbg text-text";
  if (line.startsWith("-")) return "bg-delbg text-text";
  return "text-text-secondary";
}

/** 변경 줄 수 — 파일을 펼치기 전에 규모를 가늠할 근거를 준다. */
export function countChanges(patch: string): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const line of patch.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) added += 1;
    else if (line.startsWith("-") && !line.startsWith("---")) removed += 1;
  }
  return { added, removed };
}

function FileBlock({ file, defaultOpen }: { file: FileDiff; defaultOpen: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  const { added, removed } = countChanges(file.patch);
  return (
    <div className="border-b border-border">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        className="flex min-h-[48px] w-full items-center gap-2 px-4 py-2 text-left active:bg-raised"
      >
        <span className="inline-flex w-4 shrink-0 text-text-muted"><Icon name={open ? "chevronDown" : "chevronRight"} size={12} /></span>
        <span className="min-w-0 flex-1 truncate text-sm text-text" dir="rtl">
          {file.path}
        </span>
        <span className="shrink-0 text-xs text-status-done">+{added}</span>
        <span className="shrink-0 text-xs text-status-failed">−{removed}</span>
      </button>
      {open ? (
        <div className="overflow-x-auto">
          <pre className="w-max min-w-full font-code text-xs leading-5">
            {file.patch.split("\n").map((line, index) => (
              <div key={index} className={`px-4 ${lineClass(line)}`}>
                {line || " "}
              </div>
            ))}
          </pre>
        </div>
      ) : null}
    </div>
  );
}

export function DiffPane({ files }: { files: FileDiff[] }) {
  if (files.length === 0) {
    return <div className="px-6 py-12 text-center text-sm text-text-muted">변경된 파일이 없습니다.</div>;
  }
  return (
    <div>
      {files.map((file, index) => (
        // 첫 파일만 펼쳐 둔다 — 전부 펼치면 큰 diff에서 스크롤이 감당되지 않는다.
        <FileBlock key={file.path} file={file} defaultOpen={index === 0 && files.length <= 3} />
      ))}
    </div>
  );
}
