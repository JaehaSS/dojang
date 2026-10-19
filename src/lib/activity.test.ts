import { describe, expect, it } from "vitest";
import {
  activityEntries,
  currentOperation,
  subagentEntries,
  subagentRootMap,
  subagentRootOf,
  subagentThreadRootOf,
  subagentThreads,
  type ActivityItem,
} from "./activity";

const taskSpawn: ActivityItem = {
  role: "tool",
  name: "Task",
  summary: "버그 조사",
  toolId: "toolu_task1",
};
const subTool: ActivityItem = {
  role: "tool",
  name: "Read",
  summary: "src/main.rs",
  parentId: "toolu_task1",
};
const subToolResult: ActivityItem = {
  role: "tool_result",
  summary: "ok",
  is_error: false,
  toolUseId: "toolu_sub1",
  parentId: "toolu_task1",
};
const subModel: ActivityItem = {
  role: "subagent_model",
  parentId: "toolu_task1",
  model: "claude-sonnet-4-5",
};
const taskDone: ActivityItem = {
  role: "tool_result",
  summary: "조사 완료",
  is_error: false,
  toolUseId: "toolu_task1",
};

describe("subagentEntries", () => {
  it("Task 스폰을 running으로 등록하고 parented 도구로 lastOp를 갱신한다", () => {
    const entries = subagentEntries([taskSpawn, subTool]);
    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({
      id: "toolu_task1",
      title: "버그 조사",
      state: "running",
      lastOp: "Read · src/main.rs",
    });
  });

  it("메인 스레드 tool_result가 오면 done/failed로 전이한다", () => {
    const done = subagentEntries([taskSpawn, subTool, taskDone]);
    expect(done[0].state).toBe("done");
    const failed = subagentEntries([taskSpawn, { ...taskDone, is_error: true }]);
    expect(failed[0].state).toBe("failed");
  });

  it("parented tool_result는 Task 완료로 오인하지 않는다", () => {
    const entries = subagentEntries([
      taskSpawn,
      { ...subToolResult, toolUseId: "toolu_task1" }, // 비정상 입력 방어
    ]);
    expect(entries[0].state).toBe("running");
  });

  it("Task가 아닌 도구·id 없는 Task는 등록하지 않는다", () => {
    expect(
      subagentEntries([{ role: "tool", name: "Bash", summary: "ls", toolId: "t1" }]),
    ).toHaveLength(0);
    expect(subagentEntries([{ role: "tool", name: "Task", summary: "x" }])).toHaveLength(0);
  });

  it("parented 모델 관측을 해당 서브 에이전트의 실행 모델로 기록한다", () => {
    const entries = subagentEntries([taskSpawn, subModel]);
    expect(entries[0].model).toBe("claude-sonnet-4-5");
  });

  it("관측 전에는 model이 null이고, 여러 번 관측되면 마지막 값이 남는다", () => {
    expect(subagentEntries([taskSpawn])[0].model).toBeNull();
    const entries = subagentEntries([
      taskSpawn,
      subModel,
      { ...subModel, model: "claude-opus-4-1" },
    ]);
    expect(entries[0].model).toBe("claude-opus-4-1");
  });

  it("중첩 스폰의 모델 관측은 루트 카드를 덮지 않는다", () => {
    // 루트가 sonnet을 돌리며 opus 서브를 스폰하는 것이 정상 사용이다.
    // 손자의 모델이 루트 칩을 덮으면 칩이 도는 모델을 잘못 이름한다.
    const entries = subagentEntries([
      taskSpawn,
      subModel,
      { role: "tool", name: "Task", summary: "중첩 조사", toolId: "toolu_nested", parentId: "toolu_task1" },
      { role: "subagent_model", parentId: "toolu_nested", model: "claude-opus-4-1" },
    ]);
    expect(entries).toHaveLength(1);
    expect(entries[0].model).toBe("claude-sonnet-4-5");
  });

  it("여러 서브 에이전트는 최신 스폰 먼저 반환한다", () => {
    const entries = subagentEntries([
      taskSpawn,
      { ...taskSpawn, toolId: "toolu_task2", summary: "성능 리뷰" },
    ]);
    expect(entries.map((e) => e.id)).toEqual(["toolu_task2", "toolu_task1"]);
  });
});

describe("subagentRootMap / subagentRootOf", () => {
  const nestedSpawn: ActivityItem = {
    role: "tool",
    name: "Task",
    summary: "중첩 조사",
    toolId: "toolu_nested",
    parentId: "toolu_task1",
  };
  const grandchild: ActivityItem = {
    role: "tool",
    name: "Bash",
    summary: "cargo test",
    parentId: "toolu_nested",
  };

  it("중첩 스폰의 손자 이벤트를 루트 서브 에이전트로 귀속한다", () => {
    const roots = subagentRootMap([taskSpawn, nestedSpawn, grandchild]);
    expect(subagentRootOf(grandchild, roots)).toBe("toolu_task1");
    expect(subagentRootOf(subTool, roots)).toBe("toolu_task1");
  });

  it("메인 스레드·부모 미상 이벤트는 null(메인에 남김)", () => {
    const roots = subagentRootMap([taskSpawn]);
    expect(subagentRootOf(taskSpawn, roots)).toBeNull();
    expect(subagentRootOf({ role: "tool", name: "Read", parentId: "toolu_unknown" }, roots)).toBeNull();
  });
});

describe("subagentThreads", () => {
  const nestedSpawn: ActivityItem = {
    role: "tool",
    name: "Task",
    summary: "중첩 조사",
    toolId: "toolu_nested",
    parentId: "toolu_task1",
  };
  const grandchild: ActivityItem = {
    role: "text",
    text: "중첩 조사 결과",
    parentId: "toolu_nested",
  };

  it("서브 내부 이벤트와 최상위 Task 결과를 같은 토글 스레드에 묶는다", () => {
    const items = [taskSpawn, subTool, nestedSpawn, grandchild, taskDone];
    const threads = subagentThreads(items);

    expect(threads).toHaveLength(1);
    expect(threads[0]).toMatchObject({
      id: "toolu_task1",
      title: "버그 조사",
      state: "done",
    });
    expect(threads[0].items).toEqual([subTool, nestedSpawn, grandchild, taskDone]);
  });

  it("모델 관측은 트랜스크립트에 섞이지 않고 엔트리 필드로만 남는다", () => {
    // items에 들어가면 이것뿐인 스레드가 "기다리는 중…" 대신 빈 줄을 그린다.
    const threads = subagentThreads([taskSpawn, subModel]);

    expect(threads[0].items).toEqual([]);
    expect(threads[0].model).toBe("claude-sonnet-4-5");
  });

  it("최상위 Task 결과만 메인 대화가 아닌 해당 토글에 귀속한다", () => {
    const roots = subagentRootMap([taskSpawn]);

    expect(subagentThreadRootOf(taskDone, roots)).toBe("toolu_task1");
    expect(
      subagentThreadRootOf(
        { role: "tool_result", summary: "일반 도구 결과", toolUseId: "toolu_other" },
        roots,
      ),
    ).toBeNull();
  });
});

describe("Codex 서브에이전트 카드 투영", () => {
  const rootId = "codex:root-a:child-a";
  const nestedId = "codex:root-a:child-b";

  it("중첩 메타데이터가 먼저 와도 부모 체인을 따라 루트 카드와 탭에 묶는다", () => {
    const items: ActivityItem[] = [
      { role: "subagent_state", subagentId: nestedId, parentId: rootId, state: "done", summary: "중첩 작업", model: "gpt-6.1-mini" },
      { role: "text", parentId: nestedId, text: "중첩 결과" },
      { role: "tool", parentId: nestedId, name: "Read", summary: "nested.ts" },
      { role: "subagent_state", subagentId: rootId, state: "running", title: "관측한 루트 이름", summary: "루트 작업", model: "gpt-6.1-sol", observationStatus: "available" },
      { role: "text", parentId: rootId, text: "루트 결과" },
    ];

    expect(subagentRootMap(items).get(nestedId)).toBe(rootId);
    const threads = subagentThreads(items);
    expect(threads).toHaveLength(1);
    expect(threads[0]).toMatchObject({
      id: rootId,
      title: "관측한 루트 이름",
      state: "running",
      vendor: "codex",
      model: "gpt-6.1-sol",
      lastOp: "Read · nested.ts",
    });
    expect(threads[0].items.filter((item) => item.role === "text").map((item) => item.text)).toEqual(["중첩 결과", "루트 결과"]);
  });

  it("Codex의 명시 상태는 부모 busy 해제와 무관하게 보존한다", () => {
    const entries = subagentEntries([
      { role: "subagent_state", subagentId: rootId, state: "unknown", summary: "관측 부족", observationStatus: "unreadable" },
    ]);
    expect(entries[0]).toMatchObject({ state: "unknown", vendor: "codex", observationStatus: "unreadable" });
  });
});

describe("activityEntries (서브 에이전트 제외)", () => {
  it("서브 에이전트 소속(parented) 이벤트는 메인 타임라인에서 제외한다", () => {
    const entries = activityEntries([taskSpawn, subTool, subToolResult, taskDone]);
    expect(entries).toHaveLength(2); // Task 스폰 + Task 결과만
    expect(entries.map((e) => e.title)).toEqual(["도구 실행 완료", "Task"]);
  });
});

describe("activityEntries", () => {
  it("도구 호출과 결과만 최신순 활동 이력으로 만든다", () => {
    expect(
      activityEntries([
        { role: "user" },
        { role: "tool", name: "shell", summary: "npm test" },
        { role: "tool_result", summary: "12 passed", is_error: false },
        { role: "text" },
      ]),
    ).toEqual([
      { index: 2, state: "done", title: "도구 실행 완료", detail: "12 passed" },
      { index: 1, state: "started", title: "shell", detail: "npm test" },
    ]);
  });

  it("도구·에이전트 실패를 failed 상태로 보존한다", () => {
    expect(
      activityEntries([
        { role: "tool_result", summary: "exit 1", is_error: true },
        { role: "error", text: "quota exceeded" },
      ]),
    ).toEqual([
      { index: 1, state: "failed", title: "에이전트 오류", detail: "quota exceeded" },
      { index: 0, state: "failed", title: "도구 실행 실패", detail: "exit 1" },
    ]);
  });
});

describe("currentOperation", () => {
  it("상태 조회의 마지막 작업을 우선한다", () => {
    expect(
      currentOperation([{ role: "tool", name: "Edit", summary: "src/App.tsx" }], "Bash cargo test"),
    ).toBe("Bash cargo test");
  });

  it("상태 조회 값이 없으면 가장 최근 도구를 요약한다", () => {
    expect(
      currentOperation([
        { role: "tool", name: "Read", summary: "README.md" },
        { role: "text" },
        { role: "tool", name: "Edit", summary: "src/App.tsx" },
      ], null),
    ).toBe("Edit · src/App.tsx");
  });

  it("서브 에이전트 내부(parented) 도구는 현재 작업 폴백에서 제외한다", () => {
    expect(
      currentOperation(
        [
          { role: "tool", name: "Edit", summary: "src/App.tsx" },
          { role: "tool", name: "Read", summary: "sub.rs", parentId: "toolu_task1" },
        ],
        null,
      ),
    ).toBe("Edit · src/App.tsx");
  });

  it("마지막 도구 결과 뒤에는 완료된 작업을 현재 작업으로 오인하지 않는다", () => {
    expect(
      currentOperation(
        [
          { role: "tool", name: "shell", summary: "npm test" },
          { role: "tool_result", summary: "passed", is_error: false },
        ],
        null,
      ),
    ).toBeNull();
  });
});
