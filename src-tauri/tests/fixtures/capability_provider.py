#!/usr/bin/python3
"""Deterministic Codex 0.157.1 app-server fixture.  It never invokes a model."""
import json
import sys

if "--version" in sys.argv:
    print("codex-cli 0.157.1")
    raise SystemExit(0)

assert "app-server" in sys.argv and "--stdio" in sys.argv
assert "features.code_mode_host=true" in sys.argv
assert "features.code_mode=true" in sys.argv
assert "features.default_mode_request_user_input=true" in sys.argv
assert "features.send_message_to_user_async=true" in sys.argv
assert "features.computer_use=false" not in sys.argv

THREAD = "capability-thread"
TURN = "capability-turn"
case = None

def send(value):
    print(json.dumps(value), flush=True)

def response(frame, value):
    send({"id": frame["id"], "result": value})

def event(method, **params):
    send({"method": method, "params": params})

def native(method, request_id, **params):
    send({"id": request_id, "method": method, "params": {
        "threadId": THREAD, "turnId": TURN, **params
    }})

def complete(status="completed"):
    event("item/completed", threadId=THREAD, turnId=TURN,
          item={"type": "agentMessage", "id": "final", "text": "fixture complete"})
    event("turn/completed", threadId=THREAD, turn={"id": TURN, "status": status})

def native_question():
    native("item/tool/requestUserInput", "same-native-wire", itemId="input-item",
           isBlocking=False, questions=[{
               "id": "colour", "header": "Colour", "question": "Choose",
               "isOther": False, "isSecret": False,
               "options": [{"label": "red", "description": ""},
                           {"label": "blue", "description": ""}]
           }])

def approval(method, request_id, decisions):
    params = {"itemId": "approval-item", "startedAtMs": 1, "reason": "fixture",
              "availableDecisions": decisions}
    if method == "item/commandExecution/requestApproval":
        params.update(command="echo fixture", cwd="/tmp")
    elif method == "item/fileChange/requestApproval":
        params.update(grantRoot="/tmp")
    else:
        params.update(permissions={"network": {"enabled": True}})
    native(method, request_id, **params)

def raw(item):
    event("rawResponseItem/completed", threadId=THREAD, turnId=TURN, item=item)

try:
    for line in sys.stdin:
        frame = json.loads(line)
        method = frame.get("method")
        params = frame.get("params", {})
        if method == "initialize":
            assert params["capabilities"]["experimentalApi"] is True
            response(frame, {})
        elif method == "initialized":
            pass
        elif method in ("thread/start", "thread/resume"):
            assert params["approvalPolicy"] == "never"
            assert params["sandbox"] == "danger-full-access"
            if method == "thread/start":
                assert params["experimentalRawEvents"] is True
                tools = params["dynamicTools"]
                assert tools[0]["name"] == "praxis_ui"
                assert tools[0]["tools"][0]["name"] == "ask_user"
            else:
                assert params["threadId"] == THREAD
            response(frame, {"thread": {"id": THREAD}, "model": "fixture-model"})
        elif method == "turn/start":
            assert params["threadId"] == THREAD
            case = params["input"][0]["text"]
            response(frame, {"turn": {"id": TURN}})
            if case == "native-question":
                native_question()
            elif case == "async-message":
                event("item/completed", threadId=THREAD, turnId=TURN, item={
                    "type": "agentMessage", "id": "async-message", "delivery": "async",
                    "text": "Choose a colour", "questions": [{
                        "title": "Colour", "options": ["red", "blue"]
                    }]})
            elif case == "native-unanswered":
                native_question()
                complete()
            elif case.startswith("approval-"):
                approval("item/commandExecution/requestApproval", "same-approval-wire",
                         [case.removeprefix("approval-")])
            elif case == "permission":
                approval("item/permissions/requestApproval", "same-permissions-wire", ["accept"])
            elif case == "file":
                approval("item/fileChange/requestApproval", "same-file-wire", ["accept"])
            elif case == "mcp-form":
                native("mcpServer/elicitation/request", "same-mcp-wire", mode="form",
                       message="Configure", requestedSchema={"type": "object", "required": ["enabled"],
                       "properties": {"enabled": {"type": "boolean"},
                                      "style": {"type": "string", "enum": ["small", "large"]}}})
            elif case == "wrong-owner":
                send({"id": "bad-owner", "method": "item/tool/requestUserInput", "params": {
                    "threadId": THREAD, "turnId": "other-turn", "itemId": "input-item", "isBlocking": True,
                    "questions": [{"id": "x", "header": "X", "question": "X", "isOther": False,
                                   "isSecret": False, "options": [{"label": "x", "description": ""}]}]}})
            elif case == "raw-rich":
                raw({"type": "reasoning", "encrypted_content": "NEVER-SERIALIZE-RAW-REASONING"})
                raw({"type": "custom_tool_call", "call_id": "raw-exec", "name": "exec", "input": "{\"cmd\":\"echo hi\"}"})
                raw({"type": "custom_tool_call_output", "call_id": "raw-exec", "output": [
                    {"type": "input_text", "text": "Script completed"},
                    {"type": "input_image", "image_url": "data:image/png;base64,AQ=="}]})
                event("item/started", threadId=THREAD, turnId=TURN,
                      item={"type": "mcpToolCall", "id": "mcp-rich", "tool": "rich"})
                event("item/completed", threadId=THREAD, turnId=TURN,
                      item={"type": "mcpToolCall", "id": "mcp-rich", "status": "completed",
                            "result": {"content": [{"type": "text", "text": "MCP text"},
                              {"type": "image", "mimeType": "image/png", "data": "AQ=="},
                              {"type": "audio", "mimeType": "audio/wav", "data": "AQ=="}]}})
                event("item/completed", threadId=THREAD, turnId=TURN,
                      item={"type": "imageGeneration", "id": "generated", "result": "AQ=="})
                complete()
            elif case in ("raw-yielded", "raw-wait", "raw-wait-failed", "raw-wait-error", "unknown-raw-tool"):
                name = "unlisted" if case == "unknown-raw-tool" else "exec"
                raw({"type": "custom_tool_call", "call_id": "raw-exec", "name": name,
                     "input": "{\"cmd\":\"echo hi\"}"})
                if case != "unknown-raw-tool":
                    raw({"type": "custom_tool_call_output", "call_id": "raw-exec",
                         "output": "Script running with cell ID abc"})
                    if case in ("raw-wait", "raw-wait-failed", "raw-wait-error"):
                        raw({"type": "function_call", "call_id": "raw-wait", "name": "wait",
                             "arguments": "{\"cell_id\":\"abc\"}"})
                        terminal = {
                            "raw-wait": "Script completed",
                            "raw-wait-failed": "Script failed",
                            "raw-wait-error": "Script error",
                        }[case]
                        raw({"type": "custom_tool_call_output", "call_id": "raw-wait",
                             "output": terminal})
                complete()
            else:
                raise AssertionError("unknown fixture case: " + str(case))
        elif method == "turn/interrupt":
            response(frame, {})
            complete("interrupted")
        elif method == "turn/steer":
            assert case == "async-message"
            assert params["threadId"] == THREAD and params["expectedTurnId"] == TURN
            assert "red" in params["input"][0]["text"]
            response(frame, {"turnId": TURN})
            complete()
        elif method is None and "id" in frame:
            request_id = frame["id"]
            result = frame["result"]
            if case == "native-question":
                assert request_id == "same-native-wire"
                assert result == {"answers": {"colour": {"answers": ["blue"]}}}
                event("serverRequest/resolved", threadId=THREAD, requestId=request_id)
                complete()
            elif case.startswith("approval-"):
                expected = case.removeprefix("approval-")
                assert request_id == "same-approval-wire" and result == {"decision": expected}
                event("serverRequest/resolved", threadId=THREAD, requestId=request_id)
                complete()
            elif case == "permission":
                assert result == {"permissions": {"network": {"enabled": True}}, "scope": "turn"}
                event("serverRequest/resolved", threadId=THREAD, requestId=request_id)
                complete()
            elif case == "file":
                assert result == {"decision": "accept"}
                event("serverRequest/resolved", threadId=THREAD, requestId=request_id)
                complete()
            elif case == "mcp-form":
                assert result == {"action": "accept", "content": {"enabled": True, "style": "large"}}
                event("serverRequest/resolved", threadId=THREAD, requestId=request_id)
                complete()
            else:
                raise AssertionError("unexpected response: " + repr(frame))
finally:
    pass
