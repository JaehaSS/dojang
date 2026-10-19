import { createContext, useContext, useMemo, type ReactNode } from "react";
import type { DocumentImageReader } from "./MarkdownImage";

interface SessionImages {
  rootPath?: string | null;
  readFile?: DocumentImageReader;
}

const SessionImageContext = createContext<SessionImages>({});

/** Main, debate and subagent messages share the selected session's host and workspace. */
export function SessionImageScope({ children, rootPath, readFile }: SessionImages & { children: ReactNode }) {
  const images = useMemo(() => ({ rootPath, readFile }), [rootPath, readFile]);
  return <SessionImageContext.Provider value={images}>{children}</SessionImageContext.Provider>;
}

export const useSessionImages = () => useContext(SessionImageContext);
