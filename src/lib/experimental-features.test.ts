// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";
import {
  experimentalFeaturesEnabled,
  resetExperimentalFeaturesForTest,
  setExperimentalFeaturesEnabled,
} from "./experimental-features";

beforeEach(() => {
  resetExperimentalFeaturesForTest(false);
});

describe("고급·실험 기능 표시 스토어", () => {
  it("기본값은 꺼짐이다 — 처음 쓰는 사람에게 실험 지표부터 보이면 안 된다", () => {
    expect(experimentalFeaturesEnabled()).toBe(false);
  });

  it("켜면 localStorage에 남아 다음 조회에도 이어진다", () => {
    setExperimentalFeaturesEnabled(true);

    expect(experimentalFeaturesEnabled()).toBe(true);
    expect(window.localStorage.getItem("praxis:experimental-features")).toBe("1");
  });

  it("끄면 값도 저장소도 함께 되돌아간다", () => {
    setExperimentalFeaturesEnabled(true);
    setExperimentalFeaturesEnabled(false);

    expect(experimentalFeaturesEnabled()).toBe(false);
    expect(window.localStorage.getItem("praxis:experimental-features")).toBe("0");
  });
});
