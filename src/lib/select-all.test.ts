import { describe, expect, it, vi } from "vitest";
import { selectAllInFocusedSurface } from "./select-all";

describe("native select-all routing", () => {
  const surface = () => ({ execCommand: vi.fn() }) as unknown as Document;
  const editor = (focused: boolean) => ({ hasTextFocus: () => focused, trigger: vi.fn() });

  it("selects only the focused editor in a split, without selecting its hidden textarea", () => {
    const editors = [editor(false), editor(true), editor(false)];
    const document = surface();
    selectAllInFocusedSurface(editors, document);
    expect(editors[1].trigger).toHaveBeenCalledWith("native-menu", "editor.action.selectAll", null);
    expect(editors[0].trigger).not.toHaveBeenCalled();
    expect(editors[2].trigger).not.toHaveBeenCalled();
    expect(document.execCommand).not.toHaveBeenCalled();
  });

  it("uses native DOM selection when a composer, search field or preview has focus", () => {
    const unfocused = editor(false);
    const document = surface();
    selectAllInFocusedSurface([unfocused], document);
    expect(unfocused.trigger).not.toHaveBeenCalled();
    expect(document.execCommand).toHaveBeenCalledWith("selectAll");
  });

  it("also selects DOM content when no editor is mounted", () => {
    const document = surface();
    selectAllInFocusedSurface([], document);
    expect(document.execCommand).toHaveBeenCalledWith("selectAll");
  });
});
