#!/usr/bin/env python3
"""Claude 자리에 서는 대역. stream-json을 내보내면서, 질문은 인앱 MCP 툴로 건다.

인자는 실제 claude와 같은 모양으로 받는다 — 마지막 위치인자가 프롬프트, `--mcp-config`가
서버 정의 파일, Bearer 토큰은 `PRAXIS_PREVIEW_TOKEN` 환경변수다.
"""
import json
import os
import sys
import urllib.request

argv = sys.argv[1:]
config = None
prompt = ""
for i, a in enumerate(argv):
    if a == "--mcp-config":
        config = argv[i + 1]
    elif a == "-p" and i + 1 < len(argv):
        prompt = argv[i + 1]
# The user message is not polluted by the clarification contract on each resume.
assert "# Praxis clarification contract" not in prompt
assert argv.count("--append-system-prompt") == 1
system_prompt = argv[argv.index("--append-system-prompt") + 1]
assert "# Praxis synchronous turn contract" in system_prompt
# The case remains the last user line.
case = prompt.strip().splitlines()[-1].strip() if prompt.strip() else ""


def emit(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def call(name, arguments, rid=1):
    with open(config, encoding="utf-8") as fh:
        servers = json.load(fh)["mcpServers"]
    (key,) = servers.keys()
    url = servers[key]["url"]
    body = json.dumps(
        {"jsonrpc": "2.0", "id": rid, "method": "tools/call",
         "params": {"name": name, "arguments": arguments}}
    ).encode()
    req = urllib.request.Request(
        url, data=body,
        headers={"Content-Type": "application/json",
                 "Authorization": "Bearer " + os.environ["PRAXIS_PREVIEW_TOKEN"]},
    )
    with urllib.request.urlopen(req, timeout=300) as resp:
        return json.load(resp)


def listed():
    with open(config, encoding="utf-8") as fh:
        servers = json.load(fh)["mcpServers"]
    (key,) = servers.keys()
    body = json.dumps({"jsonrpc": "2.0", "id": 9, "method": "tools/list"}).encode()
    req = urllib.request.Request(
        servers[key]["url"], data=body,
        headers={"Content-Type": "application/json",
                 "Authorization": "Bearer " + os.environ["PRAXIS_PREVIEW_TOKEN"]},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return [t["name"] for t in json.load(resp)["result"]["tools"]]


emit({"type": "system", "subtype": "init", "session_id": "local-session-1"})

QUESTION = {
    "kind": "clarification",
    "questions": [{
        "id": "color", "question": "어떤 색으로 갈까요?",
        "options": [{"id": "blue", "label": "파랑", "description": "차갑다"},
                    {"id": "red", "label": "빨강", "description": "뜨겁다"}],
        "allow_free_text": False, "is_secret": False,
    }],
}

text = ""
if case == "listing":
    text = "tools=" + ",".join(sorted(listed()))
elif case == "abandon":
    # 질문을 걸어만 두고 턴을 끝낸다 — 호스트가 미응답으로 보고 실패시켜야 한다.
    import threading
    import time
    threading.Thread(target=call, args=("ask_user", QUESTION), daemon=True).start()
    time.sleep(1.5)
    text = "abandoned"
elif case == "no_session":
    text = json.dumps(call("ask_user", QUESTION))
else:
    answer = call("ask_user", QUESTION)
    result = answer.get("result", {})
    body = result.get("content", [{}])[0].get("text", "")
    if result.get("isError"):
        text = "tool_error:" + body
    else:
        text = "answered:" + body

emit({"type": "result", "subtype": "success", "result": text,
      "is_error": False, "session_id": "local-session-1",
      "total_cost_usd": 0.0, "num_turns": 1,
      "usage": {"input_tokens": 1, "output_tokens": 1}})
