import { createRoot } from "react-dom/client";
import { ErrorBoundary } from "../components/ErrorBoundary";
import { DesignCanvas, ScreenFrame } from "./DesignCanvas";
import { collectScreens } from "./registry";
import tokens from "virtual:dojang-design-tokens";
import "../index.css";

// This entrypoint is served only by Vite; the production inputs stay unchanged.
const screens = collectScreens(import.meta.glob("./screens/*.screen.tsx", { eager: true }));
const frame = new URLSearchParams(window.location.search).get("view") === "screen";
if (frame && import.meta.hot) import.meta.hot.on("vite:afterUpdate", () => window.parent.dispatchEvent(new CustomEvent("dojang:design-hmr")));
document.body.style.overflow = "auto";
createRoot(document.getElementById("root")!).render(
  <ErrorBoundary>{frame ? <ScreenFrame screens={screens} search={window.location.search} /> : <DesignCanvas screens={screens} tokens={tokens} />}</ErrorBoundary>,
);
