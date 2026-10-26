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
import shared_settings
import shared_resume

_stop=threading.Event()
_wake=threading.Event()
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
            'resume_name':config.get('resume_name',''),'resume_sha256':config.get('resume_sha256',''),'latex_source':shared_settings.local_values()['master_latex'],
            'resume_conflict':shared_resume.state().get('conflict',False),
            'settings_conflicts':shared_settings.conflicts(),'templates_version':shared_settings.templates_version()}


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
    with shared_settings.LOCK:
        if 'templatesVersion' in values and values['templatesVersion']!=shared_settings.templates_version():
            raise ValueError('Templates changed. Reopen settings before saving your edits.')
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
    import research
    def research_active():
        current=research.status()['run']
        return bool(current and current['status'] in ('starting','running','stopping'))
    warnings=[]
    try:shared_resume.sync()
    except Exception as exc:warnings.append('Master resume: '+str(exc)[:200])
    try:
        settings=shared_settings.sync(research_active=research_active)
        if settings['conflicts']:warnings.append(f"{settings['conflicts']} profile/template conflicts: review Google settings")
        if settings['deferred']:warnings.append('Pause research to receive shared profile changes')
    except Exception as exc:warnings.append('Settings need attention: '+str(exc)[:250])
    try:outreach.reconcile()
    except Exception as exc:warnings.append('Gmail needs attention: '+str(exc)[:250])
    try:result=sheets.sync()
    except Exception as exc:
        _state['message']=' · '.join([*warnings,'Jobs need attention: '+str(exc)[:250]])
        return
    message=f"Synced {result['jobs']} jobs · {result['conflicts']} conflicts"
    _state.update(message=' · '.join([message,*warnings]),last_sync=tracker.now())


def request_sync():
    # Reuse the same worker/sync path; coalesce repeated opens and saves.
    if google.config().get('enabled') and google.TOKEN.exists():_wake.set()


def auto_drafts():
    config=google.config()
    if not config.get('initial_template'):return
    with tracker.db() as db:
        ids=[row[0] for row in db.execute("SELECT id FROM jobs WHERE verification='Verified' AND id NOT IN (SELECT job_id FROM outreach_messages WHERE kind='initial') LIMIT 5")]
    for identifier in ids:
        try:outreach.prepare(identifier)
        except ValueError:continue  # Missing published contact is normal.
    with tracker.db() as db:
        due=[r[0] for r in db.execute("SELECT job_id FROM job_meta WHERE follow_up!='' AND follow_up<=? AND job_id NOT IN (SELECT job_id FROM outreach_messages WHERE kind='followup')",(time.strftime('%Y-%m-%d'),))]
    for identifier in due:
        try:outreach.prepare(identifier,'followup')
        except ValueError:continue


def start_background():
    def loop():
        while not _stop.is_set():
            _wake.wait(60);_wake.clear()
            if _stop.is_set():break
            # A requested sync waits for an in-flight Google operation rather
            # than getting dropped. All Google work stays serialized.
            with _busy:
                if _stop.is_set():break
                try:
                    if google.config().get('enabled') and google.TOKEN.exists():
                        _state['busy']=True;sync()
                    import research
                    current=research.status()['run']
                    if current and current['status'] in ('starting','running'):auto_drafts()
                except Exception as exc:_state['message']=str(exc)[:600]
                finally:_state['busy']=False
    thread=threading.Thread(target=loop,daemon=True);thread.start()
    return thread


def shutdown():
    _stop.set();_wake.set()
