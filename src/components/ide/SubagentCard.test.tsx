import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { SubagentThread } from "../../lib/activity";
import { SubagentCard } from "./SubagentCard";
import type { ConvoItem } from "./ConversationView";

const thread: SubagentThread<ConvoItem> = {
  id: "toolu_task1",
  title: "스크롤 원인 조사",
  state: "done",
  lastOp: "Read · ConversationView.tsx",
  model: null,
  items: [{ role: "text", text: "상세 조사 결과" }],
};

describe("SubagentCard", () => {
  it("기본 접힘 상태에서 상태와 요약만 보여준다", () => {
    const html = renderToStaticMarkup(
      <SubagentCard thread={thread} parentBusy={false} onOpenSubagent={() => {}} />,
    );

    expect(html).toContain('aria-expanded="false"');
    expect(html).toContain("스크롤 원인 조사");
    expect(html).toContain("완료");
    expect(html).not.toContain("상세 조사 결과");
    expect(html).not.toContain("탭에서 열기");
  });

  it("펼치면 상세 응답과 전용 탭 진입점을 보여준다", () => {
    const html = renderToStaticMarkup(
      <SubagentCard
        thread={thread}
        parentBusy={false}
        defaultExpanded
        onOpenSubagent={() => {}}
      />,
    );

    expect(html).toContain('aria-expanded="true"');
    expect(html).toContain("상세 조사 결과");
    expect(html).toContain("max-h-80");
    expect(html).toContain("탭에서 열기");
  });
});

describe("SubagentCard — 실행 모델 칩", () => {
  it("관측된 모델이 있으면 접힘 상태에서도 헤더에 그대로 이름한다", () => {
    const html = renderToStaticMarkup(
      <SubagentCard thread={{ ...thread, model: "claude-sonnet-4-5" }} parentBusy={false} />,
    );

    expect(html).toContain("claude-sonnet-4-5");
  });

  it("관측 전(null)에는 칩을 그리지 않는다 — 빈 칩은 모른다는 사실을 감춘다", () => {
    const html = renderToStaticMarkup(<SubagentCard thread={thread} parentBusy={false} />);

    expect(html).not.toContain("text-[10px]");
  });
});

describe("SubagentCard — ConvoItem role 확장 내성", () => {
  it("서브에이전트 스레드에 없는 role이 섞여도 토큰 줄로 그리지 않는다", () => {
    // 이 카드의 마지막 분기는 한때 "나머지는 전부 meta"였다. 공유 union에 divider가 늘자
    // 그 분기가 divider를 토큰 줄로 그리려 했다(타입체크가 잡아 줬을 뿐이다).
    // divider는 메인 스레드 전용이지만 두 소비자가 같은 union을 쓰므로, 여기서 못을 박는다.
    const mixed: SubagentThread<ConvoItem> = {
      ...thread,
      items: [
        { role: "text", text: "본문" },
        { role: "divider", text: "컨텍스트를 비웠습니다" },
        { role: "meta", cost: 0, turns: 1, tokensIn: 11, tokensOut: 22 },
      ],
    };

    const html = renderToStaticMarkup(
      <SubagentCard thread={mixed} parentBusy={false} onOpenSubagent={() => {}} defaultExpanded />,
    );

    expect(html).toContain("본문");
    expect(html).toContain("11 → 22 tok");
    // divider는 조용히 빠진다 — 토큰 줄로 둔갑하지 않는 것이 핵심이다.
    expect(html).not.toContain("컨텍스트를 비웠습니다");
  });
});

describe("SubagentCard — Codex 관측 상태", () => {
  it("부모 턴이 끝나도 running을 중단으로 추정하지 않고, 상세 기록 불가와 완료 미확인을 구분한다", () => {
    const html = renderToStaticMarkup(
      <SubagentCard
        thread={{
          ...thread,
          state: "unknown",
          vendor: "codex",
          observationStatus: "unreadable",
          items: [],
        }}
        parentBusy={false}
        defaultExpanded
      />,
    );

    expect(html).toContain("완료 확인 불가");
    expect(html).toContain("상세 기록을 확인할 수 없습니다.");
    expect(html).not.toContain("중단됨");
  });

  it("공개 결과가 있어도 관측 상한으로 빠진 상세 기록을 표시한다", () => {
    const html = renderToStaticMarkup(
      <SubagentCard thread={{ ...thread, vendor: "codex", observationStatus: "limited" }} parentBusy={false} defaultExpanded />,
    );
    expect(html).toContain("완료");
    expect(html).toContain("상세 조사 결과");
    expect(html).toContain("일부 상세 기록을 확인할 수 없습니다.");
  });

  it("명시적으로 stopped인 Codex 자식만 중단됨으로 표시한다", () => {
    const html = renderToStaticMarkup(
      <SubagentCard thread={{ ...thread, state: "stopped", vendor: "codex" }} parentBusy />,
    );
    expect(html).toContain("중단됨");
  });
});
