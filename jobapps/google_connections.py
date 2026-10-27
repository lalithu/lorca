"""Google connection for Job Apps. Sending requires an explicitly approved draft."""
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import google_oauth as oauth
from google_http import request_json, GoogleNetworkError
from jobtracker import ROOT

ACCOUNT='lalithuriti@gmail.com'
PRIVATE=ROOT/'private'
TOKEN=PRIVATE/'google_token.json'
CONFIG=ROOT/'google.json'
SCOPES=' '.join(['https://www.googleapis.com/auth/spreadsheets','https://www.googleapis.com/auth/gmail.compose','https://www.googleapis.com/auth/gmail.readonly'])
oauth.TOKEN=TOKEN

def config():
    if CONFIG.exists():return json.loads(CONFIG.read_text())
    old=Path.home()/'.config/lalith-job-search/google_sync.json'
    previous=json.loads(old.read_text()) if old.exists() else {}
    return {'sheet_url':previous.get('sheet_url',''),'enabled':False,'initial_template':'','followup_template':'','resume_sha256':''}

def save_config(value):
    oauth.write_json(CONFIG,value,private=True)

def authorize(client_path=''):
    PRIVATE.mkdir(parents=True,exist_ok=True);PRIVATE.chmod(0o700)
    pending=PRIVATE/'pending_token.json'
    client_file=None
    try:
        if client_path:
            source=Path(client_path).expanduser()
        else:
            legacy=Path.home()/'.config/lalith-job-search/google_token.json'
            if not legacy.exists():raise ValueError('Select your Google Desktop OAuth client JSON to connect this Mac')
            old=json.loads(legacy.read_text())
            if not old.get('client_id'):raise ValueError('A Google Desktop OAuth client JSON is required')
            client_file=PRIVATE/'oauth_client_pending.json'
            oauth.write_json(client_file,{'installed':{k:old.get(k,'') for k in ['client_id','client_secret']}},private=True)
            source=client_file
        oauth.TOKEN=pending;oauth.SCOPE=SCOPES
        # OAuth helper prints must not enter the native stdio protocol.
        oauth.authorize(str(source),quiet=True)
        profile=api('gmail','profile')
        if profile.get('emailAddress','').lower()!=ACCOUNT:raise ValueError('Please connect '+ACCOUNT)
        pending.replace(TOKEN)
        value=config();value['account']=ACCOUNT;save_config(value)
        return ACCOUNT
    finally:
        oauth.TOKEN=TOKEN
        pending.unlink(missing_ok=True)
        if client_file:client_file.unlink(missing_ok=True)


def api(service,path,body=None,method=None,approved_send=False):
    # API origins are fixed. Sending is gated separately from ordinary API calls.
    if service=='gmail':
        verb=method or ('POST' if body is not None else 'GET')
        sending=path=='drafts/send' and verb=='POST' and approved_send and isinstance(body,dict) and bool(body.get('id'))
        updating=verb=='PUT' and path!='drafts/send' and re.fullmatch(r'drafts/[\w-]+',path) and isinstance(body,dict)
        if verb!='GET' and not sending and not updating and not (path=='drafts' and verb=='POST' and body is not None):
            raise ValueError('Only Gmail draft creation or updates are allowed')
        if not sending and not re.match(r'^(profile|drafts(?:/[\w-]+)?|threads/[\w-]+|messages(?:/[\w-]+)?)(?:\?|$)',path):raise ValueError('Unsupported Gmail operation')
        url='https://gmail.googleapis.com/gmail/v1/users/me/'+path
    elif service=='sheets':url='https://sheets.googleapis.com/v4/spreadsheets/'+path
    else:raise ValueError('Unsupported Google service')
    data=None if body is None else json.dumps(body).encode()
    try:
        request=urllib.request.Request(url,data,{'Authorization':'Bearer '+oauth.access_token(),'Content-Type':'application/json'},method=method)
        return request_json(request)
    except urllib.error.HTTPError as exc:
        try:message=json.loads(exc.read()).get('error',{}).get('message','Google request failed')
        except Exception:message='Google request failed'
        error=RuntimeError(f'Google {exc.code}: {message[:350]}');error.status=exc.code;raise error from None


def approved_resume(path):
    source=Path(path).expanduser()
    with source.open('rb') as file:data=file.read(15_000_001)
    if not data.startswith(b'%PDF-') or not 1<=len(data)<=15_000_000:raise ValueError('Choose a PDF no larger than 15 MB')
    target=ROOT/'profile/approved-resume.pdf'
    target.parent.mkdir(exist_ok=True)
    fd,tmp=tempfile.mkstemp(dir=target.parent)
    try:
        with os.fdopen(fd,'wb') as f:f.write(data)
        Path(tmp).replace(target)
    finally:Path(tmp).unlink(missing_ok=True)
    value=config();value['resume_sha256']=hashlib.sha256(data).hexdigest();value['resume_name']=source.name;save_config(value)


def sheet_id(value):
    match=re.search(r'/spreadsheets/d/([\w-]+)',value)
    if match:return match[1]
    if re.fullmatch(r'[\w-]{20,}',value):return value
    raise ValueError('Enter the Google Sheet URL')
