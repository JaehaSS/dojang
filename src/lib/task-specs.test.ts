import { beforeEach, expect, it, vi } from "vitest";
import { appendSpecReference, taskSpecGet, taskSpecList, taskSpecRead, taskSpecReference, taskSpecSave, taskSpecSearch, taskSpecValidateLinks } from "./task-specs";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const hash = "a".repeat(64);

beforeEach(() => invoke.mockReset());

it("uses the task-spec IPC commands and exact argument names", async () => {
  invoke.mockResolvedValue({});
  await taskSpecSearch(4, "인증");
  await taskSpecRead(4, "docs/auth.md");
  await taskSpecSave(4, { relative_path: "docs/auth.md", expected_hash: hash });
  await taskSpecList(4);
  await taskSpecGet(4, 9);
  await taskSpecValidateLinks(4, "docs/auth.md", hash);
  expect(invoke.mock.calls).toEqual([
    ["task_spec_search", { id: 4, query: "인증" }],
    ["task_spec_read", { id: 4, relativePath: "docs/auth.md" }],
    ["task_spec_save", { id: 4, input: { relative_path: "docs/auth.md", expected_hash: hash } }],
    ["task_spec_list", { id: 4 }],
    ["task_spec_get", { id: 4, snapshotId: 9 }],
    ["task_spec_validate_links", { id: 4, relativePath: "docs/auth.md", expectedHash: hash }],
  ]);
});

it("creates only a validated readable reference and standalone token", () => {
  const reference = taskSpecReference({ id: 9, relative_path: "docs/auth.md", content_hash: hash, created_at: 1 });
  expect(reference).toContain("명세 근거: docs/auth.md");
  expect(reference).toContain(`[[dojang-spec:9:${hash}]]`);
  expect(() => taskSpecReference({ id: 0, relative_path: "docs/auth.md", content_hash: hash, created_at: 1 })).toThrow("ID");
  expect(() => taskSpecReference({ id: 9, relative_path: "docs/auth.md", content_hash: "bad", created_at: 1 })).toThrow("해시");
  expect(() => taskSpecReference({ id: 9, relative_path: "docs\nauth.md", content_hash: hash, created_at: 1 })).toThrow("경로");
});

it("preserves the whole draft and rejects an oversized UTF-8 result", () => {
  expect(appendSpecReference("기존 초안", "[[dojang-spec:9:" + hash + "]]"))
    .toBe(`기존 초안\n\n[[dojang-spec:9:${hash}]]`);
  const draft = "가".repeat(43690);
  expect(() => appendSpecReference(draft, "x")).toThrow("128KiB");
  expect(draft).toHaveLength(43690);
});
