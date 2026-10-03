import { describe, expect, it } from "vitest";
import { routeTranscript } from "./voice-router";

describe("voice router", () => {
  it("화면 이름 한 마디를 해당 화면으로 보낸다", () => {
    expect(routeTranscript("설정")).toEqual({ type: "view", view: "settings" });
    expect(routeTranscript("앙상블")).toEqual({ type: "view", view: "ensemble" });
  });

  it("조사·서술어가 붙어도 같은 화면으로 보낸다", () => {
    expect(routeTranscript("설정 화면")).toEqual({ type: "view", view: "settings" });
    expect(routeTranscript("홈으로 가")).toEqual({ type: "view", view: "home" });
    expect(routeTranscript("인사이트 열어줘")).toEqual({ type: "view", view: "insights" });
  });

  it("전사에 섞여 오는 문장부호와 공백을 무시한다", () => {
    expect(routeTranscript(" 앙상블. ")).toEqual({ type: "view", view: "ensemble" });
    expect(routeTranscript("설정!")).toEqual({ type: "view", view: "settings" });
  });

  it("영문 화면 이름도 받는다", () => {
    expect(routeTranscript("home")).toEqual({ type: "view", view: "home" });
    expect(routeTranscript("Settings")).toEqual({ type: "view", view: "settings" });
  });

  it("지운 Wiki 화면의 옛 별칭은 화면을 바꾸지 않는다", () => {
    expect(routeTranscript("위키")).toBeNull();
    expect(routeTranscript("메모리")).toBeNull();
  });

  it("커맨드가 화면 별칭보다 먼저 매칭된다", () => {
    // "새 작업"이 "작업"(workspace)으로 흡수되면 새 작업을 영영 못 만든다.
    expect(routeTranscript("새 작업")).toEqual({ type: "newTask" });
    expect(routeTranscript("작업")).toEqual({ type: "view", view: "workspace" });
  });

  it("전송·취소 커맨드를 인식한다", () => {
    expect(routeTranscript("전송")).toEqual({ type: "submit" });
    expect(routeTranscript("보내 줘")).toEqual({ type: "submit" });
    expect(routeTranscript("지워")).toEqual({ type: "clear" });
    expect(routeTranscript("취소")).toEqual({ type: "clear" });
  });

  it("매칭되지 않으면 null 을 준다 — 오인식으로 화면이 튀지 않게", () => {
    expect(routeTranscript("안녕하세요")).toBeNull();
    expect(routeTranscript("")).toBeNull();
    expect(routeTranscript("   ")).toBeNull();
    // 별칭이 문장 한가운데 있을 뿐인 발화는 커맨드가 아니다.
    expect(routeTranscript("위키 문서를 정리해 줘")).toBeNull();
    // 리뷰 채널이 없어져 "리뷰"는 더 이상 화면 별칭이 아니다 — 세션 안 액션이다.
    expect(routeTranscript("리뷰")).toBeNull();
  });
});
