#!/usr/bin/env python3
"""Finish a successful rebuild by replacing Lorca's UI and shared backend."""
import json
import os
from pathlib import Path
import plistlib
import signal
import subprocess
import sys
import time
import urllib.request
import urllib.parse


def output(*args):
    result = subprocess.run(args, capture_output=True, text=True, check=False)
    return result.stdout


def executable(pid):
    for line in output('/usr/sbin/lsof', '-a', '-p', str(pid), '-d', 'txt', '-Fn').splitlines():
        if line.startswith('n'):
            return Path(line[1:])
    return None


def ui_processes():
    result = {}
    for line in output('/bin/ps', '-axo', 'pid=,uid=,comm=').splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) != 3 or fields[1] != str(os.getuid()):
            continue
        path = Path(fields[2])
        if path.name != 'lorca' or path.parent.name != 'MacOS':
            continue
        try:
            info = plistlib.loads((path.parent.parent / 'Info.plist').read_bytes())
        except (OSError, ValueError, plistlib.InvalidFileException):
            continue
        if info.get('CFBundleIdentifier') == 'dev.lorca.macos':
            result[int(fields[0])] = path
    return result


def alive(pid):
    state = output('/bin/ps', '-p', str(pid), '-o', 'stat=').strip()
    return bool(state) and not state.startswith('Z')


def wait_for(predicate, seconds, description):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.1)
    raise RuntimeError(f'Timed out waiting for {description}; no forced kill or data reset was performed')


def backend_pid(root):
    lock = root / 'runtime/backend.lock'
    if not lock.exists():
        return None
    owners = {int(p) for p in output('/usr/sbin/lsof', '-t', str(lock)).split() if p.isdecimal()}
    if not owners:
        return None
    if len(owners) != 1:
        raise RuntimeError('Multiple processes hold the backend lock; refusing an ambiguous restart')
    pid = owners.pop()
    path = executable(pid)
    if path is None or path.name not in ('lorca', 'lorca-core'):
        raise RuntimeError('Backend lock owner is not a recognized Lorca executable')
    return pid


def mapped_to(pid, path):
    # lsof matches the inode, not just the filename. A replaced executable can
    # have the same path while an old process still maps its previous inode.
    data = output('/usr/sbin/lsof', '-a', '-p', str(pid), '-d', 'txt', '-Fi', str(path))
    return f'i{path.stat().st_ino}' in data.splitlines()


def cooperative_ui(pid, path, root):
    try:
        marker = json.loads((root / f'runtime/ui-{pid}.json').read_text())
        if marker.get('pid') != pid or marker.get('protocol') != 1:
            return False
        if Path(marker['executable']).resolve() != path.resolve():
            return False
        mappings = output('/usr/sbin/lsof', '-a', '-p', str(pid), '-d', 'txt', '-Fi')
        return f"i{int(marker['inode'])}" in mappings.splitlines()
    except (OSError, ValueError, KeyError, TypeError):
        return False


def ready(root, pid):
    try:
        endpoint = json.loads((root / 'runtime/endpoint.json').read_text())
        url = urllib.parse.urlsplit(endpoint['url'])
        if url.scheme != 'http' or url.hostname != '127.0.0.1' or not url.port:
            return None
        request = urllib.request.Request(endpoint['url'] + '/health', headers={'Authorization': 'Bearer ' + endpoint['token']})
        with urllib.request.urlopen(request, timeout=0.5) as response:
            health = json.load(response)
        if health.get('pid') == pid and health.get('protocol') == endpoint.get('protocol'):
            return url.port
    except (OSError, ValueError, KeyError):
        pass
    return None


def restart(app, root):
    app = app.resolve()
    core = app / 'Contents/MacOS/lorca-core'
    ui = app / 'Contents/MacOS/lorca'
    old_ui = ui_processes()
    old_backend = backend_pid(root)
    for pid, path in old_ui.items():
        if not cooperative_ui(pid, path, root):
            raise RuntimeError('An older or different-data Lorca app is running. Quit it normally, then rebuild again; no process was stopped')
    print('Restarting Lorca: saving and closing the UI…', flush=True)
    for pid in old_ui:
        os.kill(pid, signal.SIGTERM)
    wait_for(lambda: all(not alive(pid) for pid in old_ui), 45, 'Lorca to save and close')
    if old_backend is not None and alive(old_backend):
        if backend_pid(root) != old_backend:
            raise RuntimeError('Backend ownership changed during restart; refusing to stop a different process')
        os.kill(old_backend, signal.SIGTERM)
        wait_for(lambda: not alive(old_backend), 25, 'the previous backend to stop')
    if backend_pid(root) is not None:
        raise RuntimeError('A backend still owns the lock; the rebuilt app was not launched')
    environment = dict(os.environ, LORCA_DATA_DIR=str(root))
    backend = subprocess.Popen([str(core), 'serve'], env=environment, stdin=subprocess.DEVNULL,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    def new_backend_ready():
        if backend.poll() is not None:
            raise RuntimeError(f'Rebuilt backend exited with code {backend.returncode}')
        return ready(root, backend.pid)
    port = wait_for(new_backend_ready, 10, 'the rebuilt backend health check')
    if backend_pid(root) != backend.pid or not mapped_to(backend.pid, core):
        raise RuntimeError('Backend executable does not match this rebuilt bundle')
    subprocess.run(['/usr/bin/open', '-n', '--env', 'LORCA_DATA_DIR=' + str(root), str(app)], env=environment, check=True)
    def connected_ui():
        for pid, path in ui_processes().items():
            if pid in old_ui or path.resolve() != ui or not mapped_to(pid, ui):
                continue
            sockets = output('/usr/sbin/lsof', '-a', '-p', str(pid), f'-iTCP:{port}', '-sTCP:ESTABLISHED', '-t')
            if str(pid) in sockets.split():
                return pid
        return None
    new_ui = wait_for(connected_ui, 20, 'the rebuilt UI to connect to the fresh backend')
    print(f'Fresh restart verified: UI PID {new_ui}; backend PID {backend.pid}. Data preserved.', flush=True)


if __name__ == '__main__':
    try:
        data = Path(os.environ.get('LORCA_DATA_DIR', str(Path.home() / 'Library/Application Support/lorca')))
        restart(Path(sys.argv[1]), data.resolve())
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        raise SystemExit(f'Lorca rebuild restart failed: {error}')
