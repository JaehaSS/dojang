interface TextEditor {
  hasTextFocus(): boolean;
  trigger(source: string, command: string, payload: null): void;
}

/** Native menu selectors only see Monaco's hidden textarea, not its document. */
export function selectAllInFocusedSurface(editors: readonly TextEditor[], document: Document): void {
  const editor = editors.find((candidate) => candidate.hasTextFocus());
  if (editor) {
    editor.trigger("native-menu", "editor.action.selectAll", null);
    return;
  }
  // Keep the browser's native selection for inputs, contenteditable and previews.
  document.execCommand("selectAll");
}
