#!/usr/bin/env python3
"""Bounded, foreground diagnostic of the installed app-server protocol.

Uses the existing Codex login. No automatic approvals, installation, or config writes.
Prints event shapes and assistant text, never raw tool content or auth payloads.
"""
import argparse
import json
import os
import selectors
import signal
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--codex", default="codex")
    parser.add_argument("--timeout", type=int, default=90)
    parser.add_argument("--raw", action="store_true", help="Inspect version-pinned raw tool events; never print arguments")
    parser.add_argument("--prompt", default="Use Code Mode once to calculate 21 * 2 and print the result. Then report which Computer Use, browser, image and audio tools are actually callable. Do not call those tools yet. Do not install anything, modify files, or start background processes.")
    args = parser.parse_args()
    marker = f"dojang-capability-probe-{os.getpid()}"
    with tempfile.TemporaryDirectory(prefix="dojang-capability-probe-") as cwd:
        with tempfile.TemporaryFile() as errors:
            env = {**os.environ, "DOJANG_CAPABILITY_PROBE": marker}
            command = [args.codex, "app-server", "--stdio", "-c", "features.computer_use=true", "-c", "features.code_mode=true", "-c", "features.code_mode_host=true"]
            proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, text=False, start_new_session=True, cwd=cwd, env=env)
            stream = selectors.DefaultSelector()
            stream.register(proc.stdout, selectors.EVENT_READ)
            pending = b""
            seen = set()
            terminal = False
            success = False
            thread = None
            turn = None
            def send(value):
                proc.stdin.write((json.dumps(value)+"\n").encode())
                proc.stdin.flush()
            try:
                send({"id":"init","method":"initialize","params":{"clientInfo":{"name":"dojang-capability-probe","version":"1"},"capabilities":{"experimentalApi":True}}})
                deadline = time.monotonic() + args.timeout
                while time.monotonic() < deadline and not terminal:
                    if not stream.select(.2):
                        if proc.poll() is not None:
                            break
                        continue
                    chunk = os.read(proc.stdout.fileno(), 65536)
                    if not chunk:
                        break
                    pending += chunk
                    while b"\n" in pending:
                        line, pending = pending.split(b"\n",1)
                        event = json.loads(line)
                        method = event.get("method")
                        params = event.get("params",{})
                        if event.get("error"):
                            print(json.dumps({"id":event.get("id"),"error":event["error"]}),flush=True)
                            terminal=True
                        if event.get("id") == "init" and "result" in event:
                            send({"method":"initialized","params":{}})
                            send({"id":"thread","method":"thread/start","params":{"cwd":cwd,"ephemeral":True,"experimentalRawEvents":args.raw,"approvalPolicy":"never","sandbox":"danger-full-access","developerInstructions":"Run only the user's bounded local diagnostic. Do not read unrelated files, messages, or credentials. Collect all commands before ending. Report actual tool availability, not assumptions."}})
                        elif event.get("id") == "thread" and "result" in event:
                            thread=event["result"]["thread"]["id"]
                            print(json.dumps({"threadStarted":True,"model":event["result"].get("model")}),flush=True)
                            send({"id":"turn","method":"turn/start","params":{"threadId":thread,"input":[{"type":"text","text":args.prompt,"text_elements":[]}]}})
                        elif event.get("id") == "turn" and "result" in event:
                            turn=event["result"]["turn"]["id"]
                        elif method:
                            item=params.get("item",{})
                            msg=params.get("msg",{})
                            shape=(method,item.get("type"),msg.get("type"))
                            if shape not in seen:
                                seen.add(shape)
                                print(json.dumps({"method":method,"itemType":item.get("type"),"messageType":msg.get("type"),"keys":list(params),"itemKeys":list(item),"messageKeys":list(msg)}),flush=True)
                            if 'rawResponseItem' in method:
                                print(json.dumps({"rawType":item.get("type"),"namespace":item.get("namespace"),"name":item.get("name"),"callId":item.get("call_id"),"outputTypes":[c.get('type') for c in item.get('output',[]) if isinstance(c,dict)] if isinstance(item.get('output'),list) else type(item.get('output')).__name__}),flush=True)
                            if "id" in event:
                                print(json.dumps({"requestBlocked":method}),flush=True)
                                terminal=True
                            if method == "item/completed" and item.get("type")=="agentMessage":
                                print(json.dumps({"assistant":item.get("text","")[:6000]}),flush=True)
                            if method == "turn/completed":
                                print(json.dumps({"terminal":params.get("turn",{}).get("status")}),flush=True)
                                terminal=True
                                success=params.get("turn",{}).get("status")=="completed"
                if not terminal:
                    print(json.dumps({"timeoutOrEof":True}),flush=True)
            finally:
                if thread and turn and not terminal and proc.poll() is None:
                    try:
                        send({"id":"interrupt","method":"turn/interrupt","params":{"threadId":thread,"turnId":turn}})
                    except (OSError,ValueError):
                        pass
                proc.stdin.close()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid,signal.SIGTERM)
                    try:
                        proc.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        os.killpg(proc.pid,signal.SIGKILL)
                        proc.wait()
                stream.close()
                # Reap only descendants carrying this probe's unique environment marker.
                # ps is used here for bounded diagnostics; product cleanup verifies OS identity.
                ps=subprocess.run(["ps","eww","-axo","pid=,command="],capture_output=True,text=True,check=True)
                survivors=[]
                for row in ps.stdout.splitlines():
                    if f"DOJANG_CAPABILITY_PROBE={marker}" in row:
                        pid=int(row.strip().split(None,1)[0])
                        if pid!=os.getpid():
                            survivors.append(pid)
                            try: os.kill(pid,signal.SIGKILL)
                            except ProcessLookupError: pass
                print(json.dumps({"providerExit":proc.returncode,"cleanedDescendants":len(survivors)}),flush=True)
            return 0 if success else 1


if __name__ == "__main__":
    raise SystemExit(main())
