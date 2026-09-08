"""Personal localhost control panel for native coding agents (stdlib only)."""
from __future__ import annotations

import argparse
import fcntl
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import tempfile
import threading
import time
from urllib.error import URLError
from urllib.parse import parse_qs, urlsplit, quote
from urllib.request import Request, build_opener, ProxyHandler

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'harness/src'))
sys.path.insert(0, str(ROOT / 'scripts'))
from harness.sessions import Sessions, SessionError, WORKFLOWS, MAX_AGENTS, DEFAULT_AGENT_COUNT, fleet_runtime
from harness import clash, sdd
from harness.attachments import MAX_JSON_BYTES
from harness.skills import SkillManager
from harness.knowledge import KnowledgeManager
from harness.setup import SetupManager
from harness.creator import CreatorManager
from harness.project_browser import browse_projects
from build_kit3_catalog import build_site
from install_accelerator import EDITIONS


class HarnessServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, address, state_dir, projects, overrides=None, timeout=900):
        super().__init__(address, Handler)
        try:
            self.sessions = Sessions(Path(state_dir), projects, overrides, timeout)
            self.skills = SkillManager(self.sessions)
            self.knowledge = KnowledgeManager(self.sessions)
            self.sessions.knowledge = self.knowledge
            self.setup_manager = SetupManager(self.sessions)
            self.creator = CreatorManager(self.sessions)
            self.token = secrets.token_urlsafe(32)
            self.instance = secrets.token_hex(16)
            self.catalog_dir = tempfile.TemporaryDirectory(prefix='harness-catalog-')
            self.catalog = build_site(Path(self.catalog_dir.name)).read_bytes()
            self.page = (ROOT / 'harness/web/index.html').read_bytes()
        except Exception:
            if hasattr(self, 'setup_manager'):
                self.setup_manager.close()
            if hasattr(self, 'knowledge'):
                self.knowledge.close()
            if hasattr(self, 'sessions'):
                self.sessions.close()
            self.server_close()
            raise

    def close(self):
        self.shutdown()
        self.skills.close()
        self.setup_manager.close()
        self.knowledge.close()
        self.sessions.close()
        self.catalog_dir.cleanup()
        self.server_close()

    def bootstrap(self):
        return {
            'csrf': self.token,
            'projects': self.sessions.list_projects(),
            'providers': [{k: v for k, v in p.items() if k != 'executable'} for p in self.sessions.providers.values()],
            'workflows': WORKFLOWS, 'sdd_phases': sdd.PHASES, 'sessions': self.sessions.list(),
            'clash': {'workflows': list(clash.WORKFLOWS), 'stages': clash.STAGES, 'max_rounds': clash.MAX_ROUNDS, 'default_rounds': clash.DEFAULT_ROUNDS},
            'accelerators': [
                {'id': 'kit1', 'name': 'Kit 1 · Infrastructure Creator', 'description': 'Scan, review, generate and apply a bespoke accelerator; update manifest-owned files.'},
                {'id': 'kit2', 'name': 'Kit 2 · Ready-made editions', 'description': 'Preview and install an edition through Projects & Setup.', 'editions': list(EDITIONS)},
                {'id': 'kit3', 'name': 'Kit 3 · Open Source Kit', 'description': 'Discover community tools and their installation commands.'},
            ],
            'runtime': {'timeout_seconds': self.sessions.timeout, 'max_active': 1,
                        'max_agents': MAX_AGENTS, 'default_agent_count': DEFAULT_AGENT_COUNT,
                        'fleet': {key: value for key, value in fleet_runtime().items() if key != 'executable'}},
        }


class Handler(BaseHTTPRequestHandler):
    server_version = 'InfrastructureHarness/1'

    def setup(self):
        super().setup()
        self.connection.settimeout(15)

    def log_message(self, *_):
        pass  # Prompts, session IDs and tokens do not belong in access logs.

    def reply(self, status, payload, content_type='application/json; charset=utf-8', filename=None):
        body = payload if isinstance(payload, bytes) else json.dumps(payload, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(body)))
        if filename is not None:
            self.send_header('Content-Disposition', "attachment; filename*=UTF-8''" + quote(filename, safe=''))
        elif content_type.startswith('text/markdown'):
            self.send_header('Content-Disposition', 'attachment; filename="fleet-review.md"')
        elif content_type == 'application/zip':
            self.send_header('Content-Disposition', 'attachment; filename="project-knowledge.zip"')
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Referrer-Policy', 'no-referrer')
        self.send_header('Content-Security-Policy', "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-src 'self'; frame-ancestors 'self'; base-uri 'none'; object-src 'none'; form-action 'self'")
        self.end_headers()
        if self.command != 'HEAD':
            self.wfile.write(body)

    def error(self, status, message):
        self.reply(status, {'error': message})

    def trusted_request(self):
        port = self.server.server_port
        hosts = (f'127.0.0.1:{port}', f'localhost:{port}')
        if self.headers.get('Host') not in hosts:
            self.error(403, 'Use the localhost server address.')
            return False
        origin = self.headers.get('Origin')
        if origin and origin not in tuple('http://' + host for host in hosts):
            self.error(403, 'Cross-origin requests are disabled.')
            return False
        if self.headers.get('Sec-Fetch-Site') == 'cross-site' and self.path.startswith('/api/'):
            self.error(403, 'Cross-site API requests are disabled.')
            return False
        return True

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        if not self.trusted_request():
            return
        parsed = urlsplit(self.path)
        path = parsed.path
        store = self.server.sessions
        try:
            if path == '/':
                self.reply(200, self.server.page, 'text/html; charset=utf-8')
            elif path in ('/kit3/', '/kit3/index.html'):
                self.reply(200, self.server.catalog, 'text/html; charset=utf-8')
            elif path == '/api/health':
                self.reply(200, {'ok': True, 'instance': self.server.instance})
            elif path == '/api/bootstrap':
                self.reply(200, self.server.bootstrap())
            elif path == '/api/sessions':
                self.reply(200, {'sessions': store.list()})
            elif path == '/api/projects':
                if parsed.query:
                    raise SessionError('Invalid project listing request.')
                self.reply(200, {'projects': store.list_projects()})
            elif path.startswith('/api/projects/') and path.endswith('/setup') and len(path.split('/')) == 5:
                if parsed.query:
                    raise SessionError('Invalid project setup request.')
                self.reply(200, self.server.setup_manager.status(path.split('/')[3]))
            elif path == '/api/creator':
                query = parse_qs(parsed.query, keep_blank_values=True)
                if set(query) != {'project_id'} or len(query['project_id']) != 1:
                    raise SessionError('Select one Creator project.')
                self.reply(200, self.server.creator.list(query['project_id'][0]))
            elif path.startswith('/api/creator/') and len(path.split('/')) == 4:
                if parsed.query: raise SessionError('Invalid Creator request.')
                self.reply(200, self.server.creator.get(path.split('/')[3]))
            elif path == '/api/skills':
                if parsed.query:
                    raise SessionError('Invalid skill catalog request.')
                self.reply(200, self.server.skills.catalog())
            elif path.startswith('/api/projects/') and path.endswith('/skills') and len(path.split('/')) == 5:
                if parsed.query:
                    raise SessionError('Invalid skill listing request.')
                self.reply(200, self.server.skills.installed(path.split('/')[3]))
            elif path.startswith('/api/sessions/') and len(path.split('/')) == 4:
                sid = path.split('/')[3]
                query = parse_qs(parsed.query, strict_parsing=True) if parsed.query else {}
                if set(query) - {'after'} or len(query.get('after', ['0'])) != 1:
                    raise SessionError('Invalid event cursor.')
                after = int(query.get('after', ['0'])[0])
                if after < 0:
                    raise SessionError('Invalid event cursor.')
                self.reply(200, {'session': store.get(sid), 'events': store.events(sid, after)})
            elif path.startswith('/api/sessions/') and len(path.split('/')) == 6 and path.split('/')[4] == 'attachments':
                if parsed.query:
                    raise SessionError('Invalid attachment request.')
                _, _, _, sid, _, identifier = path.split('/')
                store.get(sid)
                item, body, _ = store.attachments.read(sid, identifier)
                self.reply(200, body, 'application/octet-stream', filename=item['name'])
            elif path.startswith('/api/sessions/') and path.endswith('/sdd') and len(path.split('/')) == 5:
                if parsed.query:
                    raise SessionError('Invalid SDD document request.')
                session = store.get(path.split('/')[3])
                if not session['sdd']:
                    raise SessionError('This session does not use SDD.')
                self.reply(200, {'session_id': session['id'], 'sdd': session['sdd'],
                                 'files': sdd.artifacts(store._workspace(session), session['sdd'])})
            elif path.startswith('/api/sessions/') and path.endswith('/results') and len(path.split('/')) == 5:
                query=parse_qs(parsed.query,keep_blank_values=True)
                if query not in ({},{'diff':['0']}): raise SessionError('Invalid result request.')
                self.reply(200,store.results.get(path.split('/')[3],include_diff=not query))
            elif path.startswith('/api/sessions/') and path.endswith('/delivery') and len(path.split('/')) == 5:
                if parsed.query: raise SessionError('Invalid delivery request.')
                self.reply(200,store.delivery.get(path.split('/')[3]))
            elif path.startswith('/api/sessions/') and path.endswith('/report') and len(path.split('/')) == 5:
                if parsed.query:
                    raise SessionError('Invalid report request.')
                sid = path.split('/')[3]
                self.reply(200, store.report(sid).encode('utf-8'), 'text/markdown; charset=utf-8')
            elif path.startswith('/api/sessions/') and path.endswith('/brain') and len(path.split('/')) == 5:
                if parsed.query:
                    raise SessionError('Invalid session knowledge request.')
                self.reply(200, store.brain_info(path.split('/')[3]))
            elif path.startswith('/api/projects/') and path.endswith('/context') and len(path.split('/')) == 5:
                self.reply(200, store.context(path.split('/')[3]))
            elif path.startswith('/api/projects/') and path.endswith('/git') and len(path.split('/')) == 5:
                if parsed.query:
                    raise SessionError('Invalid Git status request.')
                self.reply(200, store.git(path.split('/')[3]))
            elif path.startswith('/api/projects/') and path.endswith('/memory') and len(path.split('/')) == 5:
                query = parse_qs(parsed.query, strict_parsing=True, keep_blank_values=True) if parsed.query else {}
                if (set(query) - {'bank', 'path'} or any(len(values) != 1 or not values[0] for values in query.values())
                        or ('path' in query and 'bank' not in query)):
                    raise SessionError('Invalid memory request.')
                self.reply(200, store.memory(path.split('/')[3], query.get('bank', [None])[0], query.get('path', [None])[0]))
            elif path.startswith('/api/projects/') and path.split('/')[-1] in ('brain', 'knowledge') and len(path.split('/')) == 5:
                query = parse_qs(parsed.query, strict_parsing=True, keep_blank_values=True) if parsed.query else {}
                section = path.split('/')[-1]
                allowed = {'bank', 'path'} if section == 'brain' else {'bank'}
                if (set(query) - allowed or any(len(values) != 1 or not values[0] for values in query.values())
                        or ('path' in query and 'bank' not in query)):
                    raise SessionError('Invalid project knowledge request.')
                project_id = path.split('/')[3]
                bank = query.get('bank', [None])[0]
                if section == 'brain':
                    self.reply(200, self.server.knowledge.brain(project_id, bank, query.get('path', [None])[0]))
                else:
                    self.reply(200, self.server.knowledge.info(project_id, bank))
            elif path.startswith('/api/knowledge/exports/') and len(path.split('/')) == 5:
                if parsed.query:
                    raise SessionError('Invalid export download request.')
                self.reply(200, self.server.knowledge.download(path.split('/')[-1]), 'application/zip')
            else:
                self.error(404, 'Not found.')
        except (SessionError, ValueError, TypeError):
            self.error(400, 'Invalid request, project or session.')
        except (BrokenPipeError, ConnectionResetError):
            pass
        except Exception:
            self.error(500, 'The server could not complete this request.')

    def read_json(self, limit=65536):
        if self.headers.get_content_type() != 'application/json' or self.headers.get('Transfer-Encoding'):
            raise SessionError('Use an application/json body with Content-Length.')
        length = int(self.headers.get('Content-Length', '-1'))
        if not 0 < length <= limit:
            raise SessionError(f'JSON body must be between 1 and {limit} bytes.')
        def pairs(items):
            data = {}
            for key, value in items:
                if key in data:
                    raise SessionError('Duplicate JSON field.')
                data[key] = value
            return data
        def invalid_constant(_):
            raise SessionError('Invalid JSON number.')
        data = json.loads(self.rfile.read(length), object_pairs_hook=pairs, parse_constant=invalid_constant)
        if not isinstance(data, dict):
            raise SessionError('JSON body must be an object.')
        return data

    def do_POST(self):
        if not self.trusted_request():
            return
        token = self.headers.get('X-Harness-Token', '')
        if not token.isascii() or not secrets.compare_digest(token, self.server.token):
            self.error(403, 'Reload the page to renew the session token.')
            return
        store = self.server.sessions
        try:
            path = urlsplit(self.path).path
            # Quoted Markdown at the creator's byte limit can double in its JSON envelope.
            upload = path == '/api/sessions' or path.startswith('/api/sessions/') and path.endswith('/messages') and len(path.split('/')) == 5
            data = self.read_json(MAX_JSON_BYTES if upload else 131072 if path == '/api/skills/create-preview' else 65536)
            if path.startswith('/api/skills/') and urlsplit(self.path).query:
                raise SessionError('Invalid skill request.')
            if path == '/api/creator':
                if urlsplit(self.path).query: raise SessionError('Invalid Creator request.')
                self.reply(201, self.server.creator.start(data))
            elif path.startswith('/api/creator/') and len(path.split('/')) == 4:
                if urlsplit(self.path).query: raise SessionError('Invalid Creator action.')
                self.reply(200, self.server.creator.act(path.split('/')[3],data))
            elif path == '/api/sessions':
                self.reply(201, {'session': store.create(data)})
            elif path == '/api/projects':
                if urlsplit(self.path).query:
                    raise SessionError('Invalid project registration request.')
                project = store.add_project(data)
                self.reply(201, {'project': project, 'projects': store.list_projects()})
            elif path == '/api/projects/browse':
                if urlsplit(self.path).query:
                    raise SessionError('Invalid folder browsing request.')
                self.reply(200, browse_projects(data))
            elif path.startswith('/api/projects/') and path.endswith('/knowledge') and len(path.split('/')) == 5:
                if urlsplit(self.path).query:
                    raise SessionError('Invalid project knowledge request.')
                self.reply(200, self.server.knowledge.run(path.split('/')[3], data))
            elif path == '/api/skills/discover' and 'source_id' in data and not set(data) - {'source_id', 'refresh'}:
                self.reply(200, self.server.skills.discover(data['source_id'], data.get('refresh', False)))
            elif path == '/api/skills/change-preview':
                self.reply(200, self.server.skills.change_preview(data))
            elif path == '/api/skills/apply' and set(data) == {'preview_id'}:
                self.reply(200, self.server.skills.apply_change(data['preview_id']))
            elif path == '/api/skills/preview':
                self.reply(200, self.server.skills.preview(data))
            elif path == '/api/skills/create-preview':
                self.reply(200, self.server.skills.create_preview(data))
            elif path == '/api/skills/install' and set(data) == {'preview_id'}:
                self.reply(200, self.server.skills.install(data['preview_id']))
            elif path.startswith('/api/skills/'):
                raise SessionError('Invalid skill request.')
            elif path.startswith('/api/sessions/') and len(path.split('/')) == 5:
                if urlsplit(self.path).query:
                    raise SessionError('Invalid session action query.')
                _, _, _, sid, action = path.split('/')
                if action == 'messages' and 'prompt' in data and not set(data) - {'prompt', 'model', 'thinking_effort', 'sdd', 'model_routing', 'mode', 'attachments', 'agents_enabled', 'agent_count', 'clash'}:
                    self.reply(200, {'session': store.send(sid, data['prompt'], {k: v for k, v in data.items() if k != 'prompt'})})
                elif action == 'check':
                    self.reply(202,{'session':store.results.start_check(sid,data)})
                elif action == 'delivery':
                    self.reply(200,store.delivery.act(sid,data))
                elif action == 'budgets':
                    self.reply(200, {'session':store.set_budgets(sid,data)})
                elif action == 'cancel' and not data:
                    self.reply(200, {'session': store.cancel(sid)})
                elif action == 'decision' and set(data) == {'approve'}:
                    self.reply(200, {'session': store.decide(sid, data['approve'])})
                elif action == 'resume' and not data:
                    self.reply(200, {'session': store.resume(sid)})
                elif action == 'context' and set(data) == {'query'}:
                    self.reply(200, {'session': store.prepare_context(sid, data['query'])})
                elif action == 'run' and set(data) == {'context_id'}:
                    self.reply(200, {'session': store.run_context(sid, data['context_id'])})
                elif action == 'brain':
                    self.reply(200, store.brain_action(sid, data))
                else:
                    raise SessionError('Invalid session action.')
            elif path == '/api/accelerators/preview':
                if urlsplit(self.path).query:
                    raise SessionError('Invalid accelerator preview request.')
                self.reply(200, self.server.setup_manager.preview(data))
            elif path == '/api/accelerators/install':
                if urlsplit(self.path).query:
                    raise SessionError('Invalid accelerator installation request.')
                self.reply(200, self.server.setup_manager.install(data))
            elif path == '/api/shutdown' and not data:
                self.reply(200, {'ok': True})
                threading.Thread(target=self.server.shutdown, daemon=True).start()
            else:
                self.error(404, 'Not found.')
        except (SessionError, ValueError, TypeError, RecursionError) as error:
            self.error(400, str(error) if isinstance(error, SessionError) else 'Invalid JSON request.')
        except subprocess.TimeoutExpired:
            self.error(408, 'Installation preview timed out.')
        except (BrokenPipeError, ConnectionResetError):
            pass
        except Exception:
            self.error(500, 'The server could not complete this request.')


def private_dir(path):
    path = Path(path).expanduser().absolute()
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    if path.is_symlink():
        raise SessionError('State directory must not be a symbolic link.')
    os.chmod(path, 0o700)
    return path


def metadata(state):
    path = state / 'server.json'
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(descriptor) as handle:
            data = json.load(handle)
        if (type(data.get('port')) is int and 0 < data['port'] < 65536
                and all(isinstance(data.get(key), str) for key in ('instance', 'token'))):
            return data
    except (OSError, ValueError, AttributeError):
        pass
    return None


def call_server(info, path='/api/health', body=None):
    request = Request(f"http://127.0.0.1:{info['port']}{path}",
                      data=json.dumps(body).encode() if body is not None else None,
                      headers={'Content-Type': 'application/json', 'X-Harness-Token': info['token']})
    with build_opener(ProxyHandler({})).open(request, timeout=2) as response:
        return json.load(response)


def running(state):
    info = metadata(state)
    if info:
        try:
            if call_server(info).get('instance') == info['instance']:
                return info
        except (OSError, ValueError, URLError):
            pass
    return None


def serve(args, state):
    lock_fd = os.open(state / 'server.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        os.close(lock_fd)
        raise SessionError('A server already owns this state directory.')
    server = None
    try:
        overrides = {name: getattr(args, name + '_bin') for name in ('claude', 'codex', 'cursor') if getattr(args, name + '_bin')}
        server = HarnessServer(('127.0.0.1', args.port), state, args.project or [Path.cwd()], overrides, args.timeout)
        info = {'pid': os.getpid(), 'port': server.server_port, 'instance': server.instance, 'token': server.token}
        descriptor, temporary = tempfile.mkstemp(prefix='server-', dir=state)
        with os.fdopen(descriptor, 'w') as handle:
            json.dump(info, handle)
        os.replace(temporary, state / 'server.json')
        def stop_signal(*_):
            threading.Thread(target=server.shutdown, daemon=True).start()
        signal.signal(signal.SIGTERM, stop_signal)
        signal.signal(signal.SIGINT, stop_signal)
        print(f"Harness: http://127.0.0.1:{server.server_port}", flush=True)
        server.serve_forever(poll_interval=.2)
    finally:
        if server:
            server.skills.close()
            server.setup_manager.close()
            server.knowledge.close()
            server.sessions.close()
            server.catalog_dir.cleanup()
            server.server_close()
            current = metadata(state)
            if current and current['instance'] == server.instance:
                (state / 'server.json').unlink(missing_ok=True)
        os.close(lock_fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('start', 'serve', 'status', 'stop'), nargs='?', default='start')
    parser.add_argument('--project', action='append', type=Path, help='Register an existing project; repeat for multiple projects (default: current directory).')
    parser.add_argument('--port', type=int, default=8766)
    parser.add_argument('--state-dir', type=Path, default=Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state')) / 'ai-infrastructure-harness')
    parser.add_argument('--timeout', type=int, default=900, help='Initial time budget for API requests without a budgets object (default: 900). Explicit null means no time limit.')
    for name in ('claude', 'codex', 'cursor'):
        parser.add_argument('--' + name + '-bin', help='Explicit native CLI executable path.')
    args = parser.parse_args()
    try:
        for name in ('claude', 'codex', 'cursor'):
            executable = getattr(args, name + '_bin')
            if executable and ('/' in executable or executable.startswith('~')):
                setattr(args, name + '_bin', str(Path(executable).expanduser().resolve()))
        if not 0 <= args.port < 65536 or not 1 <= args.timeout <= 86400:
            raise SessionError('Invalid port or timeout (1–86400 seconds).')
        state = private_dir(args.state_dir)
        if args.command == 'serve':
            serve(args, state)
            return 0
        info = running(state)
        if args.command == 'status':
            print(f"Running: http://127.0.0.1:{info['port']}" if info else 'Stopped')
            return 0 if info else 1
        if args.command == 'stop':
            if not info:
                print('Already stopped')
                return 0
            call_server(info, '/api/shutdown', {})
            for _ in range(50):
                if not metadata(state):
                    print('Stopped')
                    return 0
                time.sleep(.2)
            raise SessionError('Shutdown is still in progress; check status shortly.')
        if info:
            print(f"Already running: http://127.0.0.1:{info['port']} (stop before changing projects or options)")
            return 0
        command = [sys.executable, str(Path(__file__).resolve()), 'serve', '--state-dir', str(state), '--port', str(args.port), '--timeout', str(args.timeout)]
        for project in args.project or [Path.cwd()]:
            command.extend(['--project', str(project.expanduser().resolve(strict=True))])
        for name in ('claude', 'codex', 'cursor'):
            if getattr(args, name + '_bin'):
                command.extend(['--' + name + '-bin', getattr(args, name + '_bin')])
        log_fd = os.open(state / 'server.log', os.O_WRONLY | os.O_APPEND | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        with os.fdopen(log_fd, 'ab') as log:
            process = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
        for _ in range(175):
            info = running(state)
            if info:
                print(f"Started: http://127.0.0.1:{info['port']}")
                return 0
            if process.poll() is not None:
                raise SessionError(f'Server could not start. See {state / "server.log"}.')
            time.sleep(.2)
        raise SessionError(f'Startup is still in progress. Check ./harness-server status and {state / "server.log"}.')
    except (SessionError, OSError) as error:
        print(f'harness-server: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
