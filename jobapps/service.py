#!/usr/bin/env python3
"""Private stdio adapter. No HTTP server, Hermes runtime or third-party Python deps."""
import fcntl
import json
import os
from pathlib import Path
import sys
import time

ROOT=Path(os.environ.get('LORCA_JOBS_ROOT',str(Path.home()/'Library/Application Support/lorca/job-apps')))
os.environ['LORCA_JOBS_ROOT']=str(ROOT)
os.environ['JOB_PIPELINE_DB']=str(ROOT/'pipeline.sqlite3')
import jobtracker as tracker
import research
from research_context import atomic_text
import integrations
import outreach
import google_connections as google
import sheets
import shared_settings
import resumes
import shared_resume


def setup():
    ROOT.mkdir(parents=True,exist_ok=True)
    (ROOT/'profile').mkdir(exist_ok=True)
    for name,default in [('preferences.md',shared_settings.DEFAULT_PREFERENCES),('master_resume.md',shared_settings.DEFAULT_PROFILE)]:
        path=ROOT/'profile'/name
        if not path.exists():atomic_text(path,default)
    tracker.init_db();research.init_research();integrations.init();resumes.init()


def snapshot():
    state=research.status()
    state['preferences']['version']=shared_settings.profile_version()
    return {'jobs':outreach.numbered_jobs(),'dossiers':state['dossiers'],'run':state['run'],
            'preferences':state['preferences'],'profile':(ROOT/'profile/master_resume.md').read_text(),
            'context':state['context'],'root':str(ROOT),'stages':tracker.STATUSES,
            'priorities':tracker.PRIORITIES,'outreach':tracker.OUTREACH,
            'integrations':integrations.status(),'mail':outreach.snapshot(),'resumes':resumes.snapshot()}



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
    if op in ('tailor_resume','compile_resume','compile_master'):
        if resumes.snapshot()['work']['busy']:raise ValueError('A resume is already being prepared')
        run=research.status()['run']
        if op=='tailor_resume' and run and run['status'] in ('starting','running','stopping'):raise ValueError('Pause research on this Mac before tailoring a resume')
        identifier=int(v.get('id',0))
        if op!='compile_master' and not tracker.get_job(identifier):raise ValueError('Job not found')
        if op=='tailor_resume' and not v.get('model'):raise ValueError('Select a local model')
        resumes.start(identifier,v.get('model',''),v.get('source') if op=='compile_resume' else None,op=='compile_master')
        return snapshot()
    if op=='select_resume':resumes.select(int(v['id']),v['choice']);integrations.request_sync();return snapshot()
    if op=='application_submitted':resumes.applied(int(v['id']),v['version']);integrations.request_sync();return snapshot()
    if op=='google_connect':integrations.connect(v.get('path',''));return snapshot()
    if op=='google_settings':integrations.save(v);integrations.request_sync();return snapshot()
    if op=='google_enable':integrations.enable();return snapshot()
    if op=='google_sync':integrations.run(integrations.sync);return snapshot()
    if op=='google_auto_sync':integrations.request_sync();return snapshot()
    if op=='resume_sync_choice':
        if v.get('choice') not in ('local','sheet'):raise ValueError('Unknown resume choice')
        integrations.run(lambda:shared_resume.sync(v['choice']));return snapshot()
    if op=='resume_pdf':
        if integrations.status()['busy']:raise ValueError('Wait for Google activity to finish')
        google.approved_resume(v['path']);return snapshot()
    if op=='resume_source':
        source=v.get('source','')
        shared_settings.validate('master_latex',source)
        if source and '\\documentclass' not in source:raise ValueError('Choose a complete LaTeX resume source')
        with shared_settings.LOCK:shared_settings.apply_values({'master_latex':source})
        integrations.request_sync();return snapshot()
    if op=='save_contact':
        if integrations.status()['busy']:raise ValueError('Wait for the Google operation before editing the recruiter')
        outreach.save_contact(int(v['id']),v.get('name',''),v.get('email',''))
        try:outreach.prepare(int(v['id']))
        except ValueError:pass
        integrations.request_sync();return snapshot()
    if op=='prepare_email':outreach.prepare(int(v['id']),v.get('kind','initial'));return snapshot()
    if op=='load_thread':
        integrations.run(lambda:outreach.load_thread(int(v['id'])));return snapshot()
    if op=='edit_email':
        if integrations.status()['busy']:raise ValueError('Wait for the Google operation before editing the email')
        outreach.edit(v['id'],v['subject'],v['body'],v.get('expected_token'));return snapshot()
    if op=='gmail_draft':
        identifier=v['id'];integrations.run(lambda:outreach.create_gmail(identifier));return snapshot()
    if op=='send_email':
        identifier=v['id'];token=v.get('approval_token','')
        integrations.run(lambda:outreach.send_approved(identifier,token));return snapshot()
    if op=='follow_up':
        value=sheets.validate('follow_up',v.get('value',''));outreach.ensure_ids()
        with tracker.db() as db:db.execute('UPDATE job_meta SET follow_up=? WHERE job_id=?',(value,int(v['id'])))
        return snapshot()
    if op=='resolve_conflict':sheets.resolve(int(v['id']),v['field'],v['choice']);return snapshot()
    if op=='settings_conflict':
        if integrations.status()['busy']:raise ValueError('Wait for Google sync to finish')
        run=research.status()['run']
        if run and run['status'] in ('starting','running','stopping'):raise ValueError('Pause research before resolving settings')
        shared_settings.resolve(v.get('field',''),v.get('choice',''));return snapshot()
    if op=='settings':
        if research.status()['run'] and research.status()['run']['status'] in ('starting','running','stopping'):
            raise ValueError('Pause research before changing preferences or profile')
        text=v.get('profile','')
        if not isinstance(text,str) or not 20<=len(text)<=24000:raise ValueError('Profile must contain 20–24,000 characters')
        shared_settings.save_profile(text,v.get('preferences',''),v.get('version',''))
        integrations.request_sync()
        return snapshot()
    if op=='start':
        if resumes.snapshot()['work']['busy']:raise ValueError('Wait for resume preparation before starting research on this Mac')
        model=v.get('model','')
        if not isinstance(model,str) or not model.strip() or len(model)>200:raise ValueError('Select a local model')
        with shared_settings.LOCK:
            if len(shared_settings.local_values()['candidate_profile'].strip())<20 or 'Paste and review your factual resume' in (ROOT/'profile/master_resume.md').read_text():
                raise ValueError('Add your candidate profile in Settings first')
            os.environ['LORCA_JOBS_MODEL']=model;research.MODEL=model
            research.start()
        return snapshot()
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
        finally:integrations.shutdown();resumes.shutdown();pause_and_wait()

if __name__=='__main__':main()
