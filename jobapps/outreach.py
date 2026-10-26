"""Template-preserving local drafts and provider-confirmed Gmail thread tracking."""
import base64
from datetime import datetime,timedelta,timezone
from email.message import EmailMessage
from email.utils import getaddresses
import hashlib
import json
from pathlib import Path
import re
import urllib.parse
import uuid
from zoneinfo import ZoneInfo
import google_connections as google
import jobtracker as tracker


def init():
    with tracker.db() as db:
        db.executescript('''
        CREATE TABLE IF NOT EXISTS job_meta(job_id INTEGER PRIMARY KEY,sync_id TEXT NOT NULL UNIQUE,follow_up TEXT NOT NULL DEFAULT '',baseline TEXT NOT NULL DEFAULT '{}');
        CREATE TABLE IF NOT EXISTS outreach_messages(id TEXT PRIMARY KEY,job_id INTEGER NOT NULL,kind TEXT NOT NULL,recipient TEXT NOT NULL,subject TEXT NOT NULL,body TEXT NOT NULL,state TEXT NOT NULL,created_at TEXT NOT NULL,gmail_draft TEXT NOT NULL DEFAULT '',thread_id TEXT NOT NULL DEFAULT '',rfc_id TEXT NOT NULL DEFAULT '',sent_at TEXT NOT NULL DEFAULT '',provider_message TEXT NOT NULL DEFAULT '',error TEXT NOT NULL DEFAULT '',UNIQUE(job_id,kind));
        CREATE TABLE IF NOT EXISTS sync_conflicts(job_id INTEGER NOT NULL,field TEXT NOT NULL,local_value TEXT NOT NULL,remote_value TEXT NOT NULL,PRIMARY KEY(job_id,field));
        ''')
    with tracker.db() as db:
        if 'discovery_number' not in {row[1] for row in db.execute('PRAGMA table_info(job_meta)')}:
            db.execute('ALTER TABLE job_meta ADD COLUMN discovery_number INTEGER NOT NULL DEFAULT 0')
            db.execute('UPDATE job_meta SET discovery_number=job_id')
    ensure_ids()


def ensure_ids():
    with tracker.db() as db:
        maximum=db.execute('SELECT COALESCE(MAX(discovery_number),0) FROM job_meta').fetchone()[0]
        for row in db.execute('SELECT id FROM jobs ORDER BY id').fetchall():
            if db.execute('SELECT 1 FROM job_meta WHERE job_id=?',(row[0],)).fetchone():continue
            maximum=max(maximum+1,row[0])
            db.execute('INSERT INTO job_meta(job_id,sync_id,discovery_number) VALUES(?,?,?)',(row[0],str(uuid.uuid4()),maximum))


def business_due(sent):
    date=datetime.fromisoformat(sent).astimezone(ZoneInfo('America/New_York')).date()
    days=0
    while days<5:
        date+=timedelta(days=1)
        if date.weekday()<5:days+=1
    return date.isoformat()


def contact(job):
    with tracker.db() as db:row=db.execute('SELECT payload FROM research_dossiers WHERE job_id=?',(job['id'],)).fetchone()
    data=json.loads(row[0]) if row else {}
    value=data.get('contact',{});email=value.get('email','')
    if job['verification']!='Verified' or not re.fullmatch(r'[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+',email):raise ValueError('A verified role and source-backed published contact email are required')
    if not value.get('email_evidence',{}).get('quote'):raise ValueError('Published email source evidence is missing')
    return value,data


def prepare(job_id,kind='initial'):
    if kind not in ('initial','followup'):raise ValueError('Unknown draft type')
    job=tracker.get_job(job_id)
    if not job:raise ValueError('Job not found')
    settings=google.config();template=settings.get('initial_template' if kind=='initial' else 'followup_template','').strip()
    if not template:raise ValueError('Add your '+kind+' template in Job Apps settings first')
    with tracker.db() as db:
        existing=db.execute('SELECT * FROM outreach_messages WHERE job_id=? AND kind=?',(job_id,kind)).fetchone()
        if existing:return dict(existing)
        if kind=='followup':
            initial=db.execute("SELECT * FROM outreach_messages WHERE job_id=? AND kind='initial'",(job_id,)).fetchone()
            if not initial or initial['state']!='Sent':raise ValueError('Follow-up requires an actually sent initial message without a detected reply')
    who,data=contact(job)
    fields={'first_name':who.get('name','').split(' ')[0],'contact_name':who.get('name',''),
        'company':job['company'],'role':job['title'],'job_url':job['job_url'],
        'job_specific':data.get('role',{}).get('fit',''),'your_name':'Lalith Uriti'}
    unknown=set(re.findall(r'\{\{\s*(\w+)\s*\}\}',template))-set(fields)
    if unknown:raise ValueError('Unknown template placeholders: '+', '.join(sorted(unknown)))
    rendered=re.sub(r'\{\{\s*(\w+)\s*\}\}',lambda m:fields[m[1]],template)
    first,sep,body=rendered.partition('\n')
    if not sep or not first.lower().startswith('subject:'):raise ValueError('Start the template with Subject: followed by the subject, then the email body')
    subject=first[8:].strip();body=body.strip()
    if not subject or not body or len(subject)>240 or len(body)>20000:raise ValueError('Template subject/body is empty or too long')
    record={'id':str(uuid.uuid4()),'job_id':job_id,'kind':kind,'recipient':who['email'],'subject':subject,'body':body,'state':'Local draft','created_at':tracker.now()}
    with tracker.db() as db:
        db.execute('INSERT INTO outreach_messages('+','.join(record)+') VALUES('+','.join('?' for _ in record)+')',list(record.values()))
    return record


def edit(identifier,subject,body):
    if not isinstance(subject,str) or not isinstance(body,str) or not subject.strip() or '\n' in subject or len(subject)>240 or not 1<=len(body)<=20000:raise ValueError('Invalid subject or body')
    with tracker.db() as db:
        row=db.execute('SELECT state FROM outreach_messages WHERE id=?',(identifier,)).fetchone()
        if not row or row['state']!='Local draft':raise ValueError('Only a local draft can be edited here; edit a Gmail draft in Gmail')
        db.execute('UPDATE outreach_messages SET subject=?,body=? WHERE id=?',(subject.strip(),body,identifier))


def create_gmail(identifier):
    with tracker.db() as db:
        row=db.execute('SELECT * FROM outreach_messages WHERE id=?',(identifier,)).fetchone()
        if not row:raise ValueError('Draft not found')
        row=dict(row)
        if row['state']!='Local draft':raise ValueError('This draft was already created or needs reconciliation; it will not be resent')
        rfc=f'<lorca-{identifier}@lorca.local>'
        db.execute("UPDATE outreach_messages SET state='Creating',rfc_id=?,error='' WHERE id=?",(rfc,identifier))
    try:
        msg=EmailMessage();msg['To']=row['recipient'];msg['From']=google.ACCOUNT;msg['Subject']=row['subject'];msg['Message-ID']=rfc
        msg.set_content(row['body'])
        if row['kind']=='initial':
            settings=google.config();path=tracker.ROOT/'profile/approved-resume.pdf'
            if not settings.get('resume_sha256'):raise ValueError('Select and approve a resume PDF in settings first')
            data=path.read_bytes()
            if hashlib.sha256(data).hexdigest()!=settings['resume_sha256']:raise ValueError('Resume changed; reselect it before creating a draft')
            msg.add_attachment(data,maintype='application',subtype='pdf',filename=settings.get('resume_name','resume.pdf'))
        else:
            with tracker.db() as db:initial=dict(db.execute("SELECT * FROM outreach_messages WHERE job_id=? AND kind='initial'",(row['job_id'],)).fetchone())
            if initial['state']!='Sent':raise ValueError('Reply detected or initial email not sent; review before following up')
            msg['In-Reply-To']=initial['rfc_id'];msg['References']=initial['rfc_id']
            if not row['subject'].lower().startswith('re:'):msg.replace_header('Subject','Re: '+initial['subject'])
        payload={'message':{'raw':base64.urlsafe_b64encode(msg.as_bytes()).decode()}}
        if row['kind']=='followup' and initial.get('thread_id'):payload['message']['threadId']=initial['thread_id']
    except Exception:
        with tracker.db() as db:db.execute("UPDATE outreach_messages SET state='Local draft' WHERE id=?",(identifier,))
        raise
    try:
        result=google.api('gmail','drafts',payload)
        if not result.get('id') or not result.get('message',{}).get('threadId'):raise ValueError('Gmail did not confirm the created draft')
        with tracker.db() as db:
            db.execute("UPDATE outreach_messages SET state='Gmail draft',gmail_draft=?,thread_id=?,provider_message=? WHERE id=?",(result['id'],result['message']['threadId'],result['message'].get('id',''),identifier))
            db.execute("UPDATE jobs SET outreach_stage='Drafted',updated_at=? WHERE id=?",(tracker.now(),row['job_id']))
    except Exception as exc:
        # Network ambiguity is not safe to retry: the draft may already exist.
        with tracker.db() as db:db.execute("UPDATE outreach_messages SET state='Needs reconciliation',error=? WHERE id=?",(str(exc)[:500],identifier))
        raise


def headers(message):return {h['name'].lower():h['value'] for h in message.get('payload',{}).get('headers',[])}

def reconcile():
    with tracker.db() as db:rows=[dict(r) for r in db.execute("SELECT * FROM outreach_messages WHERE state NOT IN ('Local draft','Needs review')")]
    for row in rows:
        # Only an already-linked thread or our unique RFC message ID is queried.
        thread=None
        if row['state'] in ('Creating','Needs reconciliation'):
            query=urllib.parse.urlencode({'q':'rfc822msgid:'+row['rfc_id'].strip('<>'),'maxResults':5})
            candidates=google.api('gmail','drafts?'+query).get('drafts',[])
            recovered=None
            for candidate in candidates:
                draft=google.api('gmail','drafts/'+candidate['id']+'?format=metadata')
                if headers(draft.get('message',{})).get('message-id')==row['rfc_id']:recovered=draft;break
            if recovered:
                with tracker.db() as db:db.execute("UPDATE outreach_messages SET state='Gmail draft',gmail_draft=?,thread_id=?,error='' WHERE id=?",(recovered['id'],recovered['message']['threadId'],row['id']))
                continue
        if row['gmail_draft'] and row['state']=='Gmail draft':
            try:
                draft=google.api('gmail','drafts/'+row['gmail_draft']+'?format=metadata')
                h=headers(draft.get('message',{}))
                with tracker.db() as db:db.execute('UPDATE outreach_messages SET thread_id=?,rfc_id=? WHERE id=?',(draft['message']['threadId'],h.get('message-id',row['rfc_id']),row['id']))
                continue
            except RuntimeError as exc:
                if getattr(exc,'status',None)!=404:raise
        if row['thread_id']:
            try:thread=google.api('gmail','threads/'+row['thread_id']+'?format=metadata')
            except RuntimeError as exc:
                if getattr(exc,'status',None)!=404:raise
        if not thread:
            query=urllib.parse.urlencode({'q':'rfc822msgid:'+row['rfc_id'].strip('<>'),'maxResults':5})
            found=google.api('gmail','messages?'+query).get('messages',[])
            for item in found:
                candidate=google.api('gmail','messages/'+item['id']+'?format=metadata')
                if headers(candidate).get('message-id')==row['rfc_id']:
                    thread=google.api('gmail','threads/'+candidate['threadId']+'?format=metadata');break
        if not thread:
            with tracker.db() as db:db.execute("UPDATE outreach_messages SET state='Needs reconciliation',error='Draft missing; no linked sent message was confirmed' WHERE id=?",(row['id'],))
            continue
        sent=[]
        for message in thread.get('messages',[]):
            if 'SENT' not in message.get('labelIds',[]):continue
            if int(message.get('internalDate','0'))<int(datetime.fromisoformat(row['created_at']).timestamp()*1000):continue
            if headers(message).get('message-id')==row['rfc_id'] or message.get('id')==row['provider_message']:sent.append(message)
        if not sent:continue
        message=sent[0];stamp=datetime.fromtimestamp(int(message['internalDate'])/1000,timezone.utc).isoformat()
        incoming=[m for m in thread.get('messages',[]) if int(m.get('internalDate',0))>int(message['internalDate']) and not {'SENT','DRAFT','TRASH'}.intersection(m.get('labelIds',[])) and google.ACCOUNT not in [a.lower() for _,a in getaddresses([headers(m).get('from','')])]]
        state='Needs review' if incoming else 'Sent'
        with tracker.db() as db:
            db.execute('UPDATE outreach_messages SET state=?,sent_at=?,thread_id=?,rfc_id=?,error=? WHERE id=?',(state,stamp,thread['id'],headers(message).get('message-id',row['rfc_id']),'',row['id']))
            db.execute('UPDATE jobs SET outreach_stage=?,updated_at=? WHERE id=?',('Replied' if incoming else 'Sent',tracker.now(),row['job_id']))
            if incoming:db.execute("UPDATE job_meta SET follow_up='' WHERE job_id=?",(row['job_id'],))
            elif row['kind']=='initial' and not row['sent_at']:db.execute('UPDATE job_meta SET follow_up=? WHERE job_id=?',(business_due(stamp),row['job_id']))
            elif row['kind']=='followup':db.execute("UPDATE job_meta SET follow_up='' WHERE job_id=?",(row['job_id'],))


def snapshot():
    ensure_ids()
    with tracker.db() as db:
        return {'messages':[dict(r) for r in db.execute('SELECT * FROM outreach_messages ORDER BY created_at DESC')],
                'meta':[dict(r) for r in db.execute('SELECT job_id,follow_up FROM job_meta')],
                'conflicts':[dict(r) for r in db.execute('SELECT * FROM sync_conflicts')]}

def numbered_jobs():
    ensure_ids()
    with tracker.db() as db:numbers={r[0]:r[1] for r in db.execute('SELECT job_id,discovery_number FROM job_meta')}
    return [{**job,'number':numbers[job['id']]} for job in tracker.list_jobs()]
