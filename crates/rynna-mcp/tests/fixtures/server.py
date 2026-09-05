import json
import os
import sys
import subprocess

child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)"]) if os.environ.get("SPAWN_CHILD") else None

for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion': request['params']['protocolVersion'], 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'fixture', 'version': '1'}}
    elif method == 'tools/list':
        result = {'tools': [{'name': 'echo/path', 'description': 'Echo arguments', 'inputSchema': {'type': 'object', 'properties': {'text': {'type': 'string'}}}}]}
    elif method == 'tools/call':
        result = {'content': [{'type': 'text', 'text': request['params']['arguments']['text']}], 'structuredContent': {'profile': os.environ.get('PROFILE_MARKER'), 'isolated': 'CARGO_MANIFEST_DIR' not in os.environ, 'pid': os.getpid(), 'child_pid': child.pid if child else None}, 'isError': False}
    else:
        continue
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
