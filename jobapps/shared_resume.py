"""Share the approved PDF through the existing private Sheet, only when changed."""
import base64
import hashlib
import json
import tempfile
from pathlib import Path
import urllib.parse
import google_connections as google
import jobtracker as tracker
import shared_settings

TAB='Lorca Resume'
LIMIT=15_000_000

def state_path():return tracker.ROOT/'private/resume-sync.json'
def state():
    try:return json.loads(state_path().read_text())
    except (OSError,ValueError):return {}
def save(value):google.oauth.write_json(state_path(),value,private=True)
def fingerprint():
    path=tracker.ROOT/'profile/approved-resume.pdf'
    if not path.exists():return ''
    with path.open('rb') as file:data=file.read(LIMIT+1)
    if len(data)>LIMIT or not data.startswith(b'%PDF-'):raise ValueError('Approved resume PDF is invalid')
    return hashlib.sha256(data).hexdigest()

def sync(choice=None):
    sid=google.sheet_id(google.config().get('sheet_url',''))
    meta=google.api('sheets',sid+'?fields=sheets(properties(title))')
    if TAB not in {x['properties']['title'] for x in meta.get('sheets',[])}:
        google.api('sheets',sid+':batchUpdate',{'requests':[{'addSheet':{'properties':{'title':TAB,'gridProperties':{'rowCount':550,'columnCount':3},'hidden':True}}}]})
    def read(range):return google.api('sheets',sid+'/values/'+urllib.parse.quote("'"+TAB+"'!"+range,safe='')).get('values',[])
    remote=read('A1:C2')
    if remote and remote[0]!=['Name','SHA256','Chunks']:raise ValueError('Unexpected master resume sheet; nothing overwritten')
    row=remote[1] if len(remote)>1 else []
    remote_hash=row[1] if len(row)>1 else ''
    if remote_hash and (len(remote_hash)!=64 or any(c not in '0123456789abcdef' for c in remote_hash)):raise ValueError('Invalid shared resume fingerprint')
    local=fingerprint();saved=state();base=saved.get('hash') if saved.get('sheet')==sid else None
    if local==remote_hash:
        save({'sheet':sid,'hash':local,'conflict':False});return
    pull=bool(remote_hash and (not local or local==base or choice=='sheet'))
    push=bool(local and (not remote_hash or remote_hash==base or choice=='local'))
    if not pull and not push:
        save({'sheet':sid,'hash':base,'conflict':True});raise ValueError('Both Macs changed the master PDF. Choose a version in Candidate profile.')
    if pull:
        count=int(row[2])
        if not 1<=count<=501:raise ValueError('Shared PDF exceeds limit')
        chunks=read('A3:A'+str(count+2))
        if len(chunks)!=count:raise ValueError('Shared PDF upload is incomplete; retry next sync')
        encoded=''.join(str(part[0]) if part else '' for part in chunks)
        data=base64.b64decode(encoded,validate=True)
        if len(data)>LIMIT or not data.startswith(b'%PDF-') or hashlib.sha256(data).hexdigest()!=remote_hash:raise ValueError('Shared PDF verification failed; local resume kept')
        if read('A1:C2')!=remote:raise ValueError('Shared PDF changed during sync; retry')
        with shared_settings.LOCK:
            if fingerprint()!=local:raise ValueError('Master PDF changed on this Mac during sync; retry')
            directory=tracker.ROOT/'profile';directory.mkdir(exist_ok=True)
            with tempfile.NamedTemporaryFile(dir=directory,suffix='.pdf') as file:
                file.write(data);file.flush();google.approved_resume(file.name)
            config=google.config();config['resume_name']=Path(row[0]).name;google.save_config(config)
        save({'sheet':sid,'hash':remote_hash,'conflict':False})
    else:
        data=(tracker.ROOT/'profile/approved-resume.pdf').read_bytes()
        if hashlib.sha256(data).hexdigest()!=local:raise ValueError('Master changed during sync; retry')
        text=base64.b64encode(data).decode();chunks=[text[i:i+40000] for i in range(0,len(text),40000)]
        if read('A1:C2')!=remote:raise ValueError('Shared PDF changed during sync; retry')
        # Publish chunks before the manifest; partial uploads never become visible.
        for start in range(0,len(chunks),40):
            part=chunks[start:start+40]
            google.api('sheets',sid+'/values:batchUpdate',{'valueInputOption':'RAW','data':[{'range':f"'{TAB}'!A{start+3}:A{start+len(part)+2}",'values':[[chunk] for chunk in part]}]})
        google.api('sheets',sid+'/values:batchUpdate',{'valueInputOption':'RAW','data':[{'range':f"'{TAB}'!A1:C2",'values':[['Name','SHA256','Chunks'],[google.config().get('resume_name','Resume.pdf'),local,len(chunks)]]}]})
        save({'sheet':sid,'hash':local,'conflict':False})
