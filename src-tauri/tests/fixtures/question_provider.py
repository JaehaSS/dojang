#!/usr/bin/python3
"""Deterministic stdio provider; never invokes network tools or reads credentials."""
import json
import os
import subprocess
import sys
import time
if '--version' in sys.argv:
    print('codex-cli 0.154.0')
    sys.exit()
assert "features.computer_use=false" in sys.argv and "features.code_mode_host=true" in sys.argv
speed = next((arg.split('=', 1)[1].strip('"') for arg in sys.argv if arg.startswith('service_tier=')), None)
if speed == 'fast': assert 'features.fast_mode=true' in sys.argv
def send(value):
    print(json.dumps(value), flush=True)
def response(frame, value):
    send({'id': frame['id'], 'result': value})
def event(method, **params):
    send({'method': method, 'params': params})
thread = 'test-thread'
turn = 'test-turn'
args = {'kind': 'clarification', 'questions': [{'id': 'color', 'question': 'Choose a color', 'options': [{'id':'blue', 'label':'Blue', 'description':''}], 'allow_free_text':True, 'is_secret':False}]}
question = {'type':'dynamicToolCall','id':'call-one','namespace':'praxis_ui','tool':'ask_user','arguments':args}
case = ''
child = None
try:
    for line in sys.stdin:
        frame = json.loads(line)
        method = frame.get('method')
        p = frame.get('params', {})
        if method == 'initialize':
            assert p['capabilities']['experimentalApi']
            response(frame, {})
        elif method == 'thread/start' or method == 'thread/resume':
            assert p.get('serviceTier') == speed
            assert p['approvalPolicy'] == 'never' and p['sandbox'] == 'danger-full-access'
            if method == 'thread/start':
                tool=p['dynamicTools'][0]
                assert tool['name']=='praxis_ui' and tool['tools'][0]['name']=='ask_user'
            else:
                assert p['threadId']==thread and 'dynamicTools' not in p
                assert p.get('excludeTurns') is True, 'resume must omit image-heavy historical turns'
            response(frame, {'thread':{'id':thread},'model':'fixture-model'})
        elif method == 'mcpServerStatus/list':
            response(frame, {'data':[], 'nextCursor':None})
        elif method == 'turn/start':
            assert p.get('serviceTier') == speed
            case=p['input'][0]['text']
            if case in ['large-event', 'oversized-event']:
                size = 5 if case == 'large-event' else 17
                event('fixture/image', image='a' * (size * 1024 * 1024))
            response(frame, {'turn':{'id':turn}})
            request={'id':9007199254740993,'method':'item/tool/call','params':{'threadId':thread,'turnId':turn,'callId':'call-one','namespace':'praxis_ui','tool':'ask_user','arguments':args}}
            if case == 'secret':
                args['questions'][0]['is_secret']=True
                args['questions'][0]['question']='NEVER-PERSIST-SECRET'
            if case == 'wrong-owner': request['params']['turnId']='another-turn'
            if case == 'native': request['method']='item/commandExecution/requestApproval'
            if case == 'unknown-tool': request['params']['namespace']='other'
            if case == 'reverse':
                send(request)
                event('item/started',threadId=thread,turnId=turn,item=question)
            else:
                event('item/started',threadId=thread,turnId=turn,item=question)
                send(request)
            if case == 'eof': break
        elif method == 'turn/interrupt':
            response(frame,{})
            event('turn/completed',threadId=thread,turn={'id':turn,'status':'interrupted'})
        elif method is None and frame.get('id') == 9007199254740993:
            assert case not in ['native','secret','unknown-tool','wrong-owner','cancel','expired']
            assert frame['result']['success']
            output=frame['result']['contentItems']
            assert json.loads(output[0]['text'])['answers'][0]['option_id']=='blue'
            if case in ['survivor','spoofed-helper']:
                child=subprocess.Popen([sys.executable if case=='survivor' else 'SkyComputerUseClient','-c','import time;time.sleep(30)'],executable=sys.executable,start_new_session=True)
                time.sleep(.1)
            if case == 'pending-command':
                event('item/started',threadId=thread,turnId=turn,item={'type':'commandExecution','id':'cmd-one','command':'unfinished'})
            event('item/completed',threadId=thread,turnId=turn,item={**question,'success':True,'contentItems':output})
            event('item/agentMessage/delta',threadId=thread,turnId=turn,itemId='message-one',delta='# Accepted\n')
            event('item/agentMessage/delta',threadId=thread,turnId=turn,itemId='message-one',delta='**Blue**')
            event('item/completed',threadId=thread,turnId=turn,item={'type':'agentMessage','id':'message-one','text':'# Accepted\n**Blue**'})
            event('turn/completed',threadId=thread,turn={'id':turn,'status':'completed'})
finally:
    # In the survivor case deliberately let the adapter own cleanup, rather than hiding the evidence.
    pass
