"""Desktop OAuth authorization and refresh helpers; tokens never enter stdio logs."""
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import secrets
import time
import urllib.error
import urllib.parse
import urllib.request
import webbrowser
from google_http import request_json

PRIVATE = Path.home() / '.config' / 'lalith-job-search'
TOKEN = PRIVATE / 'google_token.json'
SCOPE = 'https://www.googleapis.com/auth/spreadsheets'
TOKEN_URL = 'https://oauth2.googleapis.com/token'


def write_json(path, value, private=False):
    path.parent.mkdir(parents=True, exist_ok=True)
    if private:
        path.parent.chmod(0o700)
    temporary = path.with_name(path.name + '.tmp')
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600 if private else 0o644)
    with os.fdopen(descriptor, 'w') as handle:
        json.dump(value, handle, indent=2)
        handle.flush()
        os.fsync(handle.fileno())
    if private:
        temporary.chmod(0o600)
    temporary.replace(path)


def oauth_post(values):
    request = urllib.request.Request(TOKEN_URL, data=urllib.parse.urlencode(values).encode())
    return request_json(request,timeout=30)


def authorize(client_path, quiet=False):
    client = json.loads(Path(client_path).expanduser().read_text()).get('installed')
    if not isinstance(client, dict) or not client.get('client_id', '').endswith('.apps.googleusercontent.com'):
        raise ValueError('Use a Google Desktop app OAuth client JSON file.')
    verifier = secrets.token_urlsafe(48)
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip('=')
    expected_state = secrets.token_urlsafe(24)
    result = {}

    class Callback(BaseHTTPRequestHandler):
        def do_GET(self):
            query = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
            valid = urllib.parse.urlsplit(self.path).path == '/oauth/callback' and secrets.compare_digest(query.get('state', [''])[0], expected_state)
            if not valid:
                self.send_error(400, 'Invalid OAuth state')
                return
            if query.get('error'):
                result['error'] = query['error'][0]
            elif query.get('code'):
                result['code'] = query['code'][0]
            self.send_response(200)
            self.send_header('Content-Type', 'text/html; charset=utf-8')
            self.end_headers()
            self.wfile.write(b'<h2>Authorization received</h2><p>You can close this tab. The sync setup will verify access next.</p>')

        def log_message(self, *_):
            pass  # Authorization codes must never enter logs.

    with HTTPServer(('127.0.0.1', 0), Callback) as server:
        redirect = f'http://127.0.0.1:{server.server_port}/oauth/callback'
        params = {'client_id': client['client_id'], 'redirect_uri': redirect, 'response_type': 'code',
                  'scope': SCOPE, 'access_type': 'offline', 'prompt': 'consent', 'state': expected_state,
                  'code_challenge': challenge, 'code_challenge_method': 'S256'}
        url = 'https://accounts.google.com/o/oauth2/v2/auth?' + urllib.parse.urlencode(params)
        if not quiet: print('Authorize Google access in the browser.', flush=True)
        webbrowser.open(url)
        server.timeout = 1
        deadline = time.monotonic() + 600
        while not result and time.monotonic() < deadline:
            server.handle_request()
    if not result.get('code'):
        raise RuntimeError('Authorization was not completed: ' + result.get('error', 'timed out'))
    token = oauth_post({'grant_type': 'authorization_code', 'code': result['code'], 'redirect_uri': redirect,
                        'client_id': client['client_id'], 'client_secret': client.get('client_secret', ''),
                        'code_verifier': verifier})
    if not token.get('refresh_token'):
        raise RuntimeError('Google did not return an offline refresh token. Consent must allow offline access.')
    token.update(client_id=client['client_id'], client_secret=client.get('client_secret', ''),
                 expires_at=time.time() + token.get('expires_in', 3600))
    write_json(TOKEN, token, private=True)
    if not quiet: print('Google authorization stored securely; no credentials were printed.', flush=True)


def access_token():
    if not TOKEN.exists():
        raise RuntimeError('Google authorization is not connected on this Mac yet.')
    token = json.loads(TOKEN.read_text())
    if token.get('expires_at', 0) < time.time() + 120:
        fresh = oauth_post({'grant_type': 'refresh_token', 'refresh_token': token['refresh_token'],
                            'client_id': token['client_id'], 'client_secret': token.get('client_secret', '')})
        token.update(fresh)
        token['expires_at'] = time.time() + fresh.get('expires_in', 3600)
        write_json(TOKEN, token, private=True)
    return token['access_token']
