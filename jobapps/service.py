#!/usr/bin/env python3
"""Private stdio adapter. No HTTP server, Hermes runtime or third-party Python deps."""
import fcntl
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import sys
import time
import traceback

ROOT=Path(os.environ.get('LORCA_JOBS_ROOT',str(Path.home()/'Library/Application Support/lorca/job-apps')))
os.environ['LORCA_JOBS_ROOT']=str(ROOT)
os.environ['JOB_PIPELINE_DB']=str(ROOT/'pipeline.sqlite3')
import jobtracker as tracker
import research
from research_context import atomic_json, atomic_text
import integrations
import outreach
import google_connections as google
import sheets


def setup():
    ROOT.mkdir(parents=True,exist_ok=True)
    (ROOT/'profile').mkdir(exist_ok=True)
    for name,default in [('preferences.md','# Job search preferences\nNew York City startups; full-time junior/new-grad software roles; expected graduation December 2026. Flag eligibility and start-date uncertainties.\n'),('master_resume.md','# Candidate profile\nPaste and review your factual resume before starting research.\n')]:
        path=ROOT/'profile'/name
        if not path.exists():atomic_text(path,default)
    tracker.init_db();research.init_research();integrations.init()


def snapshot():
    state=research.status()
    return {'jobs':tracker.list_jobs(),'dossiers':state['dossiers'],'run':state['run'],
            'preferences':state['preferences'],'profile':(ROOT/'profile/master_resume.md').read_text(),
            'context':state['context'],'root':str(ROOT),'stages':tracker.STATUSES,
            'priorities':tracker.PRIORITIES,'outreach':tracker.OUTREACH,
            'integrations':integrations.status(),'mail':outreach.snapshot()}


def import_tracker(source):
    current=research.status()['run']
    if current and current['status'] in ('starting','running','stopping'):
        raise ValueError('Pause research before importing a tracker')
    source=Path(source).expanduser().resolve()
    db=source/'pipeline.sqlite3'
    if not db.is_file():raise ValueError('Select the m2max-job-pipeline folder containing pipeline.sqlite3')
    if source==ROOT.resolve():raise ValueError('This is already the active tracker')
    if tracker.list_jobs():raise ValueError('This tracker already has records. Import is available only into an empty tracker.')
    # A SQLite snapshot is safe even if the old dashboard is open. Never move or mutate its DB.
    target=ROOT/'import.sqlite3'
    with sqlite3.connect(db.as_uri()+'?mode=ro',uri=True) as old,sqlite3.connect(target) as new:
        old.backup(new)
        for table in ['research_runs']:
            new.execute("UPDATE research_runs SET status='interrupted',pid=0,stop_requested=1 WHERE status IN ('starting','running','stopping')")
    staging=ROOT/'import-files';staging.mkdir(exist_ok=True)
    try:
        for name in ['profile','packets','.research']:
            if (source/name).is_dir():
                shutil.copytree(source/name,staging/name,dirs_exist_ok=True,
                    ignore=shutil.ignore_patterns('*.lock','*.pid','backups','__pycache__'))
        for name in ['profile','packets','.research']:
            if (staging/name).exists():shutil.copytree(staging/name,ROOT/name,dirs_exist_ok=True)
        shutil.copy2(tracker.DB_PATH,ROOT/'before-import.sqlite3')
        target.replace(tracker.DB_PATH)
        integrations.init()
        atomic_json(ROOT/'import.json',{'source':str(source),'at':tracker.now(),'jobs':len(tracker.list_jobs())})
    finally:shutil.rmtree(staging,ignore_errors=True)
    return snapshot()


def pause_and_wait():
    current=research.status()['run']
    research.stop()
    if current and current['status'] in ('starting','running','stopping'):
        deadline=time.monotonic()+7
        while time.monotonic()<deadline:
            if research.status()['run']['status'] not in ('starting','running','stopping'):break
            time.sleep(.2)


def dispatch(v):
    op=v.get('op')
    if op=='snapshot':return snapshot()
    if op=='google_connect':integrations.connect(v.get('path',''));return snapshot()
    if op=='google_settings':integrations.save(v);return snapshot()
    if op=='google_enable':integrations.enable();return snapshot()
    if op=='google_sync':integrations.run(integrations.sync);return snapshot()
    if op=='resume_pdf':
        if integrations.status()['busy']:raise ValueError('Wait for Google activity to finish')
        google.approved_resume(v['path']);return snapshot()
    if op=='prepare_email':outreach.prepare(int(v['id']),v.get('kind','initial'));return snapshot()
    if op=='edit_email':outreach.edit(v['id'],v['subject'],v['body']);return snapshot()
    if op=='gmail_draft':
        identifier=v['id'];integrations.run(lambda:outreach.create_gmail(identifier));return snapshot()
    if op=='follow_up':
        value=sheets.validate('follow_up',v.get('value',''));outreach.ensure_ids()
        with tracker.db() as db:db.execute('UPDATE job_meta SET follow_up=? WHERE job_id=?',(value,int(v['id'])))
        return snapshot()
    if op=='resolve_conflict':sheets.resolve(int(v['id']),v['field'],v['choice']);return snapshot()
    if op=='import':return import_tracker(v.get('path',''))
    if op=='settings':
        if research.status()['run'] and research.status()['run']['status'] in ('starting','running','stopping'):
            raise ValueError('Pause research before changing preferences or profile')
        text=v.get('profile','')
        if not isinstance(text,str) or not 20<=len(text)<=24000:raise ValueError('Profile must contain 20–24,000 characters')
        research.save_preferences(v.get('preferences',''),v.get('version',''))
        atomic_text(ROOT/'profile/master_resume.md',text)
        return snapshot()
    if op=='start':
        model=v.get('model','')
        if not isinstance(model,str) or not model.strip() or len(model)>200:raise ValueError('Select a local model')
        if 'Paste and review your factual resume' in (ROOT/'profile/master_resume.md').read_text():
            raise ValueError('Import your existing tracker or add your candidate profile in Settings first')
        os.environ['LORCA_JOBS_MODEL']=model;research.MODEL=model
        research.start();return snapshot()
    if op=='pause':pause_and_wait();return snapshot()
    if op=='update':
        if v.get('field') not in ('stage','priority','notes','outreach_stage'):raise ValueError('Unsupported field')
        tracker.update_job(int(v['id']),{v['field']:v.get('value','')});return snapshot()
    if op=='detail':return {'job':tracker.get_job(int(v['id'])),'history':tracker.get_history(int(v['id']))}
    if op=='quit':pause_and_wait();return {'closed':True}
    raise ValueError('Unknown Job Apps operation')


def main():
    setup()
    # One adapter owns this local tracker, and is closed when the native app closes.
    with (ROOT/'adapter.lock').open('a') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        integrations.start_background()
        try:
            for line in sys.stdin:
                if len(line)>200000:raise ValueError('Job Apps request too large')
                v=json.loads(line)
                try:response={'ok':True,'result':dispatch(v)}
                except Exception as exc:response={'ok':False,'error':str(exc)[:700]}
                print(json.dumps(response,ensure_ascii=False),flush=True)
                if v.get('op')=='quit':break
        finally:integrations.shutdown();pause_and_wait()

if __name__=='__main__':main()
