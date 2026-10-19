import selectorSource from "../../src-tauri/src/designmode/inject.js?raw";
import type { DesignSelection } from "../lib/design-canvas/selection";

export interface CanvasSelector {
  setEnabled(enabled: boolean): void;
  selectSuggestedAncestor(): void;
  getSelectedElement?(): Element | null;
  reselectSource?(source: string, instanceSource?: string | null): boolean;
  dispose(): void;
}

type CanvasWindow = Window & {
  __dojangCanvasNative?: boolean;
  __praxisDesignMode?: { bindCanvasDocument(document: Document, onSelection: (selection: DesignSelection) => void): CanvasSelector };
};

export function bindCanvasDocument(document: Document, onSelection: (selection: DesignSelection) => void): CanvasSelector {
  const host = window as CanvasWindow;
  // Native previews already initialize this shared selector. Browser smoke uses the
  // identical script; the fallback does not grant native IPC or filesystem access.
  if (!host.__praxisDesignMode?.bindCanvasDocument) new Function(selectorSource)();
  if (!host.__praxisDesignMode?.bindCanvasDocument) throw new Error("요소 선택기를 불러오지 못했습니다.");
  return host.__praxisDesignMode.bindCanvasDocument(document, onSelection);
}

export function sendSelectionToApp(selection: DesignSelection): boolean {
  if (!(window as CanvasWindow).__dojangCanvasNative) return false;
  window.location.href = `praxis-designmode:inspect?data=${encodeURIComponent(JSON.stringify(selection))}`;
  return true;
}
