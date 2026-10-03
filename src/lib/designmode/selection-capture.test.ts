import { describe, expect, it } from "vitest";
import {
  buildSelectionCapture,
  isLocalCapture,
  scopeLocalCaptureId,
  truncateSelection,
  LOCAL_CAPTURE_PREFIX,
  MAX_SELECTION_TEXT,
} from "./selection-capture";

describe("truncateSelection", () => {
  it("빈 선택은 null이다", () => {
    expect(truncateSelection("")).toBeNull();
  });

  it("상한 이하는 원문을 그대로 둔다", () => {
    expect(truncateSelection("const answer = 42;")).toBe("const answer = 42;");
  });

  it("상한을 넘으면 잘라내고 원본 길이를 알린다", () => {
    const result = truncateSelection("x".repeat(MAX_SELECTION_TEXT + 10));
    expect(result).toContain("truncated");
    expect(result).toContain(`${MAX_SELECTION_TEXT + 10}자`);
  });

  it("코드포인트 기준으로 세어 백엔드 chars().count()와 상한이 어긋나지 않는다", () => {
    // 이모지는 UTF-16 코드유닛 2개 = 코드포인트 1개. length 기준이면 상한의 절반에서 잘린다.
    const emoji = "🙂".repeat(MAX_SELECTION_TEXT);
    expect(truncateSelection(emoji)).toBe(emoji);
  });
});

describe("buildSelectionCapture", () => {
  const input = {
    taskId: 7,
    filePath: "src/App.tsx",
    text: "const answer = 42;",
    startLine: 10,
    endLine: 12,
  };

  it("에디터 캡처 레코드로 변환한다", () => {
    const record = buildSelectionCapture(input);
    expect(record?.source).toBe("editor");
    expect(record?.task_id).toBe(7);
    expect(record?.file_path).toBe("src/App.tsx");
    expect(record?.selection_text).toBe("const answer = 42;");
    expect(record?.selection_start_line).toBe(10);
    expect(record?.selection_end_line).toBe(12);
  });

  it("스크린샷이 없으므로 image_path는 null이다", () => {
    expect(buildSelectionCapture(input)?.image_path).toBeNull();
  });

  it("빈 선택은 레코드를 만들지 않는다", () => {
    expect(buildSelectionCapture({ ...input, text: "" })).toBeNull();
  });

  it("id는 로컬 접두어를 갖고 호출마다 달라진다", () => {
    const first = buildSelectionCapture(input);
    const second = buildSelectionCapture(input);
    expect(first?.id.startsWith(LOCAL_CAPTURE_PREFIX)).toBe(true);
    expect(first?.id).not.toBe(second?.id);
    expect(isLocalCapture(first!.id)).toBe(true);
  });

  it("백엔드 캡처 id는 로컬로 판별하지 않는다", () => {
    expect(isLocalCapture("1-0")).toBe(false);
  });
});

describe("scopeLocalCaptureId", () => {
  it("팝아웃 몫의 로컬 id에 창 스코프를 심는다", () => {
    // 메인·팝아웃의 seq가 둘 다 0부터라 스코프가 없으면 두 창의 첫 캡처가 같은 id를 받는다.
    expect(scopeLocalCaptureId("local-42-0")).toBe("local-w42-0");
  });

  it("두 번 걸어도 스코프가 겹쳐 붙지 않는다", () => {
    // 배달 경로에 재시도가 끼어도 id가 `local-ww…`로 자라지 않아야 한다.
    expect(scopeLocalCaptureId("local-w42-0")).toBe("local-w42-0");
  });

  it("로컬이 아닌 id는 손대지 않는다", () => {
    // 백엔드가 발급한 id를 고치면 `removeCapture`의 IPC 왕복 대상이 사라진다.
    expect(scopeLocalCaptureId("7")).toBe("7");
  });
});

