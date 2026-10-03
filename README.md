# Dojang

**English** · [한국어](README.ko.md)

**A desktop IDE for running several AI coding agents at once, then reviewing and approving what they changed.**

You can already hand work to an agent from a terminal. Dojang takes over from there. It gives each
task its own **isolated git worktree**, keeps the agent's edits inside it, shows you the diff when the
agent finishes, and **leaves the decision to approve or discard with you**. Your original branch does
not change until you approve.

<!-- TODO: screenshot slot — one home screen + one review/diff screen. Not captured yet; add before publishing. -->

```
prompt  →  isolated worktree  →  agent runs  →  review diff  →  approve or discard
           dojang/<task> branch                  (partial apply supported)
```

## Why Dojang

- **Your original branch stays untouched.** Every task gets a worktree and a dedicated branch under
  `.praxis/worktrees/`. Discarding removes the whole worktree; commits and merges happen only on approval.
- **Run many tasks at once.** Eight by default (configurable from 1 to 64), each in its own worktree
  and terminal session.
- **Mix vendors.** Pick Claude Code, Codex, or Antigravity (Gemini) per task. Send the same prompt to
  several agents to compare results, or have them review each other's work.
- **See what matters before you approve.** The review bar shows protected paths, in-progress git
  operations, expected conflicts, and verification results. Apply only the **hunks you choose**; if
  applying fails, Dojang rolls back to a checkpoint.
- **Pick up where your terminal left off.** Adopt a Claude Code session that is already running and
  continue it as a new task without re-explaining the context. Opening the same session in two places
  at once is refused.
- **Everything runs locally.** The app, repositories, memory files, and index database all live on
  your machine. No server account is required.

## Features

The full list is in the [feature catalog](docs/guide/features.md) (Korean). The UI is Korean-only
for 1.0; there is no i18n yet.

| Area | Features |
|---|---|
| Task orchestration | Worktree isolation · concurrent runs · vendor choice · direct-run mode · terminal session adoption |
| Conversation | Prompt queue · side questions alongside the main thread · checkpoint rewind · agent follow-up questions |
| Review and approval | Diff viewer · per-hunk apply · line comments · cross-vendor review · approval readiness · conflict resolution |
| Verification | Build/test gates · goal contracts (protected paths enforced) · decision ledger (optional) |
| Code | Monaco editor · go to definition/references · Quick Open · parquet tables · IPython console · preview window the agent can drive |
| Knowledge | File-based memory injection · code wiki generation · review quizzes while you wait |
| Agent environment | Vendor-neutral `/skills` · MCP · LSP bridge · prompt interview |
| Mobile and more | Mobile PWA with Web Push · voice input · 25 themes |

## Installation

### Prerequisites

| Requirement | How to check |
|---|---|
| git and a locally cloned repository | `git --version` |
| At least one agent CLI | `claude --version` · `codex --version` · `agy --version` |

Each agent CLI must already be logged in or have its API key configured. **If it does not work in your
terminal, it will not work in Dojang either.** Dojang does not install the CLIs for you.

### macOS (Apple Silicon)

1.0 supports **macOS on Apple Silicon only** — see [Platform support](#platform-support).

1. Download `Dojang_<version>_aarch64.dmg` from the [releases page](https://github.com/JaehaSS/dojang/releases/latest).
2. Open the dmg and drag `Dojang.app` into `Applications`.
3. The first time, do not double-click. **Right-click → Open**, then click **Open** again in the warning dialog.

Step 3 is needed because this build is ad-hoc signed only — no Apple code signature or notarization,
and it is distributed only through GitHub Releases. From the command line,
`xattr -dr com.apple.quarantine /Applications/Dojang.app` does the same thing. You can verify the
download with `shasum -a 256` against the `SHA256SUMS.txt` attached to the release.

### Windows (x64, preview)

Windows builds are produced by CI and **have not been verified on a real Windows machine yet** — see
[Platform support](#platform-support) for what is known not to work.

1. Install [Git for Windows](https://git-scm.com/download/win). Dojang drives git, and runs
   `.praxis-env-setup.sh` with the `sh.exe` that ships with it.
2. Download `Dojang_<version>_x64-setup.exe` (or the `.msi`) from the [releases page](https://github.com/JaehaSS/dojang/releases/latest)
   and run it. The installer fetches WebView2 if it is missing.
3. The installer is not code-signed, so SmartScreen warns on first run — click **More info → Run anyway**.

### Build from source

Build it yourself on other platforms, or if you would rather avoid the signing warning — this path is
unverified outside macOS Apple Silicon. In addition to the prerequisites above, you need:

| Requirement | Notes |
|---|---|
| Node.js 22 or 24 | Tests may fail on 25 |
| Rust stable + Clippy | |
| [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) | Per-OS system libraries |

```sh
npm ci
npm run tauri build
```

On macOS this produces `macos/Dojang.app` and `dmg/` under `src-tauri/target/release/bundle/`.
On Windows it produces `nsis/*-setup.exe` and `msi/*.msi` in the same place (`src-tauri/tauri.windows.conf.json`).

## Your first task

The [user guide](docs/guide/README.md) walks you through it. The guides are currently written in Korean.

- [Getting started](docs/guide/getting-started.md) — from prerequisites to approving your first task
- [Feature catalog](docs/guide/features.md) — what Dojang can do
- [Workflows & FAQ](docs/guide/workflows-faq.md) — common patterns and troubleshooting

## Platform support

1.0 targets **macOS on Apple Silicon.** Windows x64 is a preview; Linux and Intel Macs are not supported.

| Item | Status |
|---|---|
| macOS (Apple Silicon) | Where Dojang is developed, built, and used. Distributed as a dmg on the releases page |
| macOS (Intel) | Not supported. No prebuilt binary; building from source is unverified |
| Windows (x64) | Preview. Installers are built by CI (`.github/workflows/build.yml`). The core loop — worktree, agent session, review, approve — has Windows code paths, but has **not been verified on a real machine**. Disabled on Windows: knowledge vault, side questions (⌘J), Codex question sessions, durable review recovery, turn process cleanup, "Open in Terminal", design-mode screenshots. Shortcut labels still show ⌘ (Ctrl works) |
| Linux | Not supported. Platform branches exist, but there is no record of a build or verification |
| Where tasks run | Locally only. The remote Linux Runner path was removed on 2026-09-19 |
| Auto-update | None for the app itself. Download new versions from the releases page. The CLI vendor auto-update setting (설정 > 연결) defaults to off |

Known limitations are collected under ["범위 밖" (out of scope)](docs/guide/features.md#범위-밖--주의할-것) in the feature catalog.

## Network & privacy

None of the outbound calls below are telemetry. Dojang does not phone home.

| Call | When | Purpose |
|---|---|---|
| `api.anthropic.com/api/oauth/usage` | On demand | Usage display, using the Claude CLI OAuth token read from the macOS Keychain item "Claude Code-credentials" (falls back to `~/.claude/.credentials.json`) |
| npm registry (`registry.npmjs.org`) | When installing/updating the Codex CLI | Installs `@openai/codex` into a staging directory |
| CLI auto-update check/install | Off by default (설정 > 연결) | Checks and updates agent CLI versions when enabled |
| Hugging Face model download (~130MB, BGE-small) | First use of an embedding-based feature | Not downloaded at launch; only when that feature actually runs |
| Gmail OAuth/API | Opt-in only | Only if you connect a Gmail knowledge source |
| Web Push | Only when the mobile surface is enabled | Task notifications to the paired phone |
| `npx` of `jonrad/lsp-mcp` (pinned commit) | When the LSP bridge starts | Bridges the editor to a language server |

## Mobile PWA (beta)

The mobile companion is served directly from the desktop app (설정 > 모바일) and is **beta** —
real-device pairing has not been run through end-to-end verification yet.

## Development

`npm run tauri dev` is the only development entry point. There is no separate script that serves the
frontend on its own.

There is no CI; run checks locally.

```sh
npm run check       # frontend (tsc · vitest) + Rust tests + clippy
```

## Further reading

[User guide](docs/guide/README.md) (Korean) · [docs/architecture.md](docs/architecture.md) ·
[DESIGN.md](DESIGN.md) (design system) · [THIRD-PARTY-ASSETS.md](THIRD-PARTY-ASSETS.md)

Design notes, decision records, and the work ledger are not published in this repository. Where the
documents here referred to them, the title is kept without a link.

## License

[MIT](LICENSE). Sources and copyright notices for the file-type icons are in [THIRD-PARTY-ASSETS.md](THIRD-PARTY-ASSETS.md).
