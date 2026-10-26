"""Bounded background Google sync and local draft preparation while Lorca is open."""
import json
import os
from pathlib import Path
import subprocess
import threading
import time
import google_connections as google
import jobtracker as tracker
import outreach
import sheets

_stop=threading.Event()
_busy=threading.Lock()
_state={'message':'Google not connected','busy':False,'last_sync':'','account':''}


def init():
    outreach.init()
    if google.TOKEN.exists():
        _state['message']='Google connected · '+('Sync enabled' if google.config().get('enabled') else 'Enable Sheets sync when ready')

def status():
    config=google.config()
    return {**_state,'connected':google.TOKEN.exists(),'enabled':config.get('enabled',False),'sheet_url':config.get('sheet_url',''),
            'account':config.get('account',''),'initial_template':config.get('initial_template',''),'followup_template':config.get('followup_template',''),
            'resume_name':config.get('resume_name','')}


def run(operation):
    if not _busy.acquire(blocking=False):raise ValueError('Google operation in progress; try again shortly')
    _state.update(busy=True,message='Working…')
    def work():
        try:operation()
        except Exception as exc:_state.update(message=str(exc)[:600])
        finally:_state['busy']=False;_busy.release()
    threading.Thread(target=work,daemon=True).start()


def connect(client_path=''):
    def action():
        account=google.authorize(client_path);_state.update(account=account,message='Google connected. Enable sync when ready.')
    run(action)


def save(values):
    if _busy.locked():raise ValueError('Wait for the Google operation to finish before editing settings')
    config=google.config()
    for key in ('initial_template','followup_template','sheet_url'):
        if key in values:
            value=values[key]
            if not isinstance(value,str) or len(value)>20000:raise ValueError('Invalid '+key)
            config[key]=value
    if config.get('sheet_url'):google.sheet_id(config['sheet_url'])
    google.save_config(config)


def enable():
    if not google.TOKEN.exists():raise ValueError('Connect Google first')
    config=google.config();google.sheet_id(config.get('sheet_url',''))
    # Replace the old writer, otherwise it would overwrite the editable fields.
    label=f'gui/{os.getuid()}/com.lalith.job-search.google-sync'
    subprocess.run(['/bin/launchctl','bootout',label],capture_output=True)
    result=subprocess.run(['/bin/launchctl','disable',label],capture_output=True)
    if result.returncode not in (0,):raise ValueError('Could not disable the legacy Sheets writer; sync was not enabled')
    config['enabled']=True;google.save_config(config)
    run(sync)


def sync():
    gmail_error=''
    try:outreach.reconcile()
    except Exception as exc:gmail_error=' · Gmail needs attention: '+str(exc)[:250]
    result=sheets.sync()
    _state.update(message=f"Synced {result['jobs']} jobs · {result['conflicts']} conflicts"+gmail_error,last_sync=tracker.now())


def auto_drafts():
    config=google.config()
    if not config.get('initial_template'):return
    with tracker.db() as db:
        ids=[row[0] for row in db.execute("SELECT id FROM jobs WHERE verification='Verified' AND id NOT IN (SELECT job_id FROM outreach_messages WHERE kind='initial') LIMIT 5")]
    for identifier in ids:
        try:outreach.prepare(identifier)
        except ValueError:continue  # Missing published contact is normal.


def start_background():
    def loop():
        while not _stop.wait(60):
            if not _busy.acquire(blocking=False):continue
            try:
                auto_drafts()
                if google.config().get('enabled') and google.TOKEN.exists():
                    _state['busy']=True;sync()
            except Exception as exc:_state['message']=str(exc)[:600]
            finally:_state['busy']=False;_busy.release()
    threading.Thread(target=loop,daemon=True).start()


def shutdown():_stop.set()
