import { useEffect, useState } from "react";
import { useHostScope } from "../lib/host-scope";
import { taskList } from "../lib/ipc";
import { inputCls } from "./ide/formStyles";

interface Props {
  kind: "task" | "reminder";
  repo: string;
  instruction: string;
  agent: string;
  text: string;
  onApply: (payload: { repo: string; instruction: string; agent: string } | { text: string }) =>
    void;
}

const INSTRUCTION_TEMPLATES = [
  "git pull && npm test",
  "npm run build",
  "npm run lint && npm run type-check",
  "docs 업데이트",
  "커스텀 명령",
];

export function TaskPayloadTemplate({
  kind,
  repo,
  instruction,
  agent,
  text,
  onApply,
}: Props) {
  const [repos, setRepos] = useState<string[]>([]);
  // 세션에 속하지 않는 화면이라 호스트를 스코프에서 받는다 (ADR 0133).
  const host = useHostScope();

  useEffect(() => {
    taskList(host)
      .then((tasks) => {
        const uniqueRepos = Array.from(new Set(tasks.map((t) => t.repo))).filter(Boolean);
        setRepos(uniqueRepos);
      })
      .catch(() => {});
  }, []);

  if (kind === "reminder") {
    return (
      <div className="flex flex-col gap-2">
        <textarea
          className={`${inputCls} min-h-16 resize-y font-code text-xs`}
          placeholder="리마인더 내용"
          value={text}
          onChange={(e) => onApply({ text: e.target.value })}
        />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="text-xs text-text-muted">프로젝트 선택</div>
      <select
        className={inputCls}
        value={repo}
        onChange={(e) => onApply({ repo: e.target.value, instruction, agent })}
      >
        <option value="">프로젝트 선택…</option>
        {repos.map((r) => (
          <option key={r} value={r}>
            {r}
          </option>
        ))}
      </select>

      <div className="text-xs text-text-muted">명령어</div>
      <div className="flex gap-2 flex-wrap">
        {INSTRUCTION_TEMPLATES.map((tmpl) => (
          <button
            key={tmpl}
            onClick={() =>
              onApply({
                repo,
                instruction: tmpl,
                agent,
              })
            }
            className="h-8 px-3 rounded-md bg-raised text-text-secondary text-sm hover:text-text text-center truncate max-w-xs"
          >
            {tmpl}
          </button>
        ))}
      </div>
      <textarea
        className={`${inputCls} min-h-20 resize-y font-code text-xs`}
        placeholder="명령어 또는 지시사항"
        value={instruction}
        onChange={(e) =>
          onApply({
            repo,
            instruction: e.target.value,
            agent,
          })
        }
      />

      <div className="text-xs text-text-muted">에이전트 (선택)</div>
      <select
        className={inputCls}
        value={agent}
        onChange={(e) =>
          onApply({
            repo,
            instruction,
            agent: e.target.value,
          })
        }
      >
        <option value="">기본 에이전트</option>
        <option value="claude">claude</option>
        <option value="codex">codex</option>
        <option value="agy">agy</option>
      </select>
    </div>
  );
}
