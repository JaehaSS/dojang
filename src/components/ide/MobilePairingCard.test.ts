import { describe, expect, it } from "vitest";
import { pairingUrl } from "./MobilePairingCard";

const CODE = "ab".repeat(32);

describe("pairingUrl", () => {
  it("스킴이 없으면 https를 붙인다 — 사용자는 보통 호스트만 적는다", () => {
    expect(pairingUrl("my-mac.example.ts.net", CODE)).toBe(
      `https://my-mac.example.ts.net/m/#pair=${CODE}`,
    );
  });

  it("스킴이 있으면 그대로 쓴다", () => {
    expect(pairingUrl("https://my-mac.example.ts.net", CODE)).toBe(
      `https://my-mac.example.ts.net/m/#pair=${CODE}`,
    );
  });

  it("끝 슬래시와 경로를 흡수한다 — origin만 남긴다", () => {
    expect(pairingUrl("https://host.ts.net/", CODE)).toBe(`https://host.ts.net/m/#pair=${CODE}`);
    expect(pairingUrl("https://host.ts.net/m/", CODE)).toBe(`https://host.ts.net/m/#pair=${CODE}`);
  });

  it("포트를 보존한다", () => {
    expect(pairingUrl("host.ts.net:8443", CODE)).toBe(`https://host.ts.net:8443/m/#pair=${CODE}`);
  });

  it("앞뒤 공백을 무시한다", () => {
    expect(pairingUrl("  host.ts.net  ", CODE)).toBe(`https://host.ts.net/m/#pair=${CODE}`);
  });

  it("주소나 코드가 비면 null — QR을 그리지 않는다", () => {
    expect(pairingUrl("", CODE)).toBeNull();
    expect(pairingUrl("   ", CODE)).toBeNull();
    expect(pairingUrl("host.ts.net", "")).toBeNull();
  });

  it("파싱할 수 없는 입력은 null", () => {
    expect(pairingUrl("http://", CODE)).toBeNull();
  });
});
