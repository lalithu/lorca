"""Template-preserving local drafts and provider-confirmed Gmail thread tracking."""
import base64
from datetime import datetime,timedelta,timezone
from email.message import EmailMessage
from email import message_from_bytes, policy
from email.utils import getaddresses
import hashlib
import json
from pathlib import Path
import re
import urllib.parse
import uuid
import html
from zoneinfo import ZoneInfo
import google_connections as google
import outreach_template
import jobtracker as tracker

_threads={}
MAIL_REQUIRED={'id','kind','recipient','subject','body','state','created_at'}
MAIL_DEFAULTS={key:'' for key in ('gmail_draft','thread_id','rfc_id','sent_at','provider_message','error','reply_revision','resume_choice','resume_sha256','resume_version')}
MAIL_FIELDS=MAIL_REQUIRED|set(MAIL_DEFAULTS)

def plain_body(payload):
    parts=[payload,*payload.get('parts',[])]
    for part in parts:
        if part.get('mimeType')=='text/plain' and part.get('body',{}).get('data'):
            data=part['body']['data'];return base64.urlsafe_b64decode(data+'='*(-len(data)%4)).decode(errors='replace')[:30000]
    for part in payload.get('parts',[]):
        nested=plain_body(part)
        if nested:return nested
    if payload.get('mimeType')=='text/html' and payload.get('body',{}).get('data'):
        data=payload['body']['data'];text=base64.urlsafe_b64decode(data+'='*(-len(data)%4)).decode(errors='replace')
        return html.unescape(re.sub('<[^>]+>',' ',text))[:30000]
    return ''

def load_thread(job_id):
    with tracker.db() as db:
        ids={r[0] for r in db.execute("SELECT thread_id FROM outreach_messages WHERE job_id=? AND thread_id!=''",(job_id,))}
    records={}
    for identity in ids:
        thread=google.api('gmail','threads/'+urllib.parse.quote(identity,safe='')+'?format=full')
        for message in thread.get('messages',[]):
            if 'DRAFT' in message.get('labelIds',[]):continue
            h=headers(message)
            records[message['id']]={'id':message['id'],'from':h.get('from',''),'date':h.get('date',''),
                'body':plain_body(message.get('payload',{})),'sent':'SENT' in message.get('labelIds',[]),'sort':int(message.get('internalDate',0))}
    _threads[str(job_id)]=[{key:value for key,value in row.items() if key!='sort'} for row in sorted(records.values(),key=lambda row:row['sort'])]


def init():
    with tracker.db() as db:
        db.executescript('''
        CREATE TABLE IF NOT EXISTS job_meta(job_id INTEGER PRIMARY KEY,sync_id TEXT NOT NULL UNIQUE,follow_up TEXT NOT NULL DEFAULT '',baseline TEXT NOT NULL DEFAULT '{}');
        CREATE TABLE IF NOT EXISTS outreach_messages(id TEXT PRIMARY KEY,job_id INTEGER NOT NULL,kind TEXT NOT NULL,recipient TEXT NOT NULL,subject TEXT NOT NULL,body TEXT NOT NULL,state TEXT NOT NULL,created_at TEXT NOT NULL,gmail_draft TEXT NOT NULL DEFAULT '',thread_id TEXT NOT NULL DEFAULT '',rfc_id TEXT NOT NULL DEFAULT '',sent_at TEXT NOT NULL DEFAULT '',provider_message TEXT NOT NULL DEFAULT '',error TEXT NOT NULL DEFAULT '',UNIQUE(job_id,kind));
        CREATE TABLE IF NOT EXISTS sync_conflicts(job_id INTEGER NOT NULL,field TEXT NOT NULL,local_value TEXT NOT NULL,remote_value TEXT NOT NULL,PRIMARY KEY(job_id,field));
        ''')
    with tracker.db() as db:
        columns={row[1] for row in db.execute('PRAGMA table_info(outreach_messages)')}
        for field in ('resume_choice','resume_sha256','resume_version'):
            if field not in columns:db.execute(f"ALTER TABLE outreach_messages ADD COLUMN {field} TEXT NOT NULL DEFAULT ''")
        if 'reply_revision' not in columns:
            db.execute("ALTER TABLE outreach_messages ADD COLUMN reply_revision TEXT NOT NULL DEFAULT ''")
        if 'reviewed_reply' not in {row[1] for row in db.execute('PRAGMA table_info(job_meta)')}:
            db.execute("ALTER TABLE job_meta ADD COLUMN reviewed_reply TEXT NOT NULL DEFAULT ''")
        if 'application_data' not in {row[1] for row in db.execute('PRAGMA table_info(job_meta)')}:
            db.execute("ALTER TABLE job_meta ADD COLUMN application_data TEXT NOT NULL DEFAULT '{}'")
        if 'mail_data' not in {row[1] for row in db.execute('PRAGMA table_info(job_meta)')}:
            db.execute("ALTER TABLE job_meta ADD COLUMN mail_data TEXT NOT NULL DEFAULT '[]'")
        if 'contact_origin' not in {row[1] for row in db.execute('PRAGMA table_info(job_meta)')}:
            db.execute("ALTER TABLE job_meta ADD COLUMN contact_origin TEXT NOT NULL DEFAULT ''")
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
    with tracker.db() as db:meta=db.execute('SELECT contact_origin FROM job_meta WHERE job_id=?',(job['id'],)).fetchone()
    if meta and meta[0]=='Provided by you':
        email=job['recruiter_contact']
        if not re.fullmatch(r'[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+',email):raise ValueError('Add recruiter email to prepare outreach')
        return {'name':job['recruiter_name'],'email':email,'origin':'Provided by you'},data
    if job['verification']!='Verified' or not re.fullmatch(r'[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+',email):raise ValueError('A verified role and source-backed published contact email are required')
    if not value.get('email_evidence',{}).get('quote'):raise ValueError('Published email source evidence is missing')
    return value,data


def save_contact(job_id,name,email):
    if not tracker.get_job(job_id):raise ValueError('Job not found')
    if not isinstance(name,str) or not name.strip() or len(name)>200:raise ValueError('Enter the recruiter name')
    if not isinstance(email,str) or len(email)>254 or (email and not re.fullmatch(r'[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+',email.strip())):raise ValueError('Enter a valid email address, or leave it empty')
    ensure_ids()
    with tracker.db() as db:
        # Existing Gmail drafts/sent messages remain bound to their original recipient.
        db.execute('UPDATE jobs SET recruiter_name=?,recruiter_contact=?,updated_at=? WHERE id=?',(name.strip(),email.strip(),tracker.now(),job_id))
        db.execute("UPDATE job_meta SET contact_origin='Provided by you' WHERE job_id=?",(job_id,))
        db.execute("DELETE FROM outreach_messages WHERE job_id=? AND state='Local draft'",(job_id,))
        db.execute('INSERT INTO history(job_id,action,happened_at) VALUES(?,?,?)',(job_id,'Recruiter supplied by you; unsent local drafts reset',tracker.now()))


def draft_content(job,kind):
    settings=google.config();template=settings.get('initial_template' if kind=='initial' else 'followup_template','').strip()
    if not template:raise ValueError('Add your '+kind+' template in Job Apps settings first')
    who,data=contact(job)
    profile=tracker.ROOT/'profile/master_resume.md'
    subject,body=outreach_template.render(template,job,who,data,profile.read_text() if profile.exists() else '',kind)
    return subject,body,who['email']


def prepare(job_id,kind='initial'):
    if kind not in ('initial','followup'):raise ValueError('Unknown draft type')
    job=tracker.get_job(job_id)
    if not job:raise ValueError('Job not found')
    with tracker.db() as db:
        existing=db.execute('SELECT * FROM outreach_messages WHERE job_id=? AND kind=?',(job_id,kind)).fetchone()
        if existing:return dict(existing)
        if kind=='followup':
            initial=db.execute("SELECT * FROM outreach_messages WHERE job_id=? AND kind='initial'",(job_id,)).fetchone()
            if not initial or initial['state']!='Sent':raise ValueError('Follow-up requires an actually sent initial message without a detected reply')
    subject,body,recipient=draft_content(job,kind)
    with tracker.db() as db:identity=db.execute('SELECT sync_id FROM job_meta WHERE job_id=?',(job_id,)).fetchone()[0]
    record={'id':str(uuid.uuid5(uuid.UUID(identity),kind)),'job_id':job_id,'kind':kind,'recipient':recipient,'subject':subject,'body':body,'state':'Local draft','created_at':tracker.now()}
    with tracker.db() as db:
        db.execute('INSERT INTO outreach_messages('+','.join(record)+') VALUES('+','.join('?' for _ in record)+')',list(record.values()))
    return record


def prepare_gmail(job_id,kind='initial',fill=False):
    """Explicit user action: create a draft, never send; reuse shared receipts."""
    import sheets
    if google.config().get('enabled'):sheets.sync()
    with tracker.db() as db:
        conflict=db.execute("SELECT 1 FROM sync_conflicts WHERE job_id=? AND field IN ('mail_data','recruiter_name','recruiter_contact')",(job_id,)).fetchone()
    if conflict:raise ValueError('Resolve the Sheet changes for this job before preparing outreach')
    record=prepare(job_id,kind)
    if record['state']=='Local draft':
        if outreach_template.has_markers(record['body']):
            subject,body,_=draft_content(tracker.get_job(job_id),kind)
            with tracker.db() as db:db.execute('UPDATE outreach_messages SET subject=?,body=? WHERE id=?',(subject,body,record['id']))
        create_gmail(record['id'])
    elif record['state']=='Gmail draft' and fill:
        fill_gmail(record)
    elif record['state']!='Gmail draft':
        raise ValueError('This email is already sent or needs checking. Refresh its Gmail status before preparing another draft.')
    # Publish the confirmed draft identity before handing off to Gmail, so a
    # switch to the other Mac reuses it rather than preparing another email.
    if google.config().get('enabled'):sheets.sync()


def fill_gmail(record):
    """Repair only an unchanged, unsent Gmail draft; preserve its attachments."""
    if record['state']!='Gmail draft':raise ValueError('Only an unsent Gmail draft can be filled')
    subject,body,_=draft_content(tracker.get_job(record['job_id']),record['kind'])
    draft=google.api('gmail','drafts/'+urllib.parse.quote(record['gmail_draft'],safe='')+'?format=raw')
    if draft.get('id')!=record['gmail_draft']:raise ValueError('Gmail did not confirm this draft. Refresh its status before retrying.')
    raw=draft.get('message',{}).get('raw','')
    message=message_from_bytes(base64.urlsafe_b64decode(raw+'='*(-len(raw)%4)),policy=policy.default)
    plain=message.get_body(preferencelist=('plain',))
    recipients={address.lower() for _,address in getaddresses(message.get_all('To',[]))}
    if plain is None or plain.get_content().replace('\r\n','\n').strip()!=record['body'].replace('\r\n','\n').strip() or recipients!={record['recipient'].lower()}:
        raise ValueError('This Gmail draft has been edited. Lorca left it untouched; finish editing it in Gmail.')
    # Keep an explicitly edited subject and every attachment already in Gmail.
    if str(message.get('Subject',''))!=record['subject']:subject=str(message.get('Subject',''))
    message.replace_header('Subject',subject)
    html_part=message.get_body(preferencelist=('html',))
    plain.set_content(body)
    if html_part is not None:html_part.set_content('<div>'+html.escape(body).replace('\n','<br>')+'</div>',subtype='html')
    payload={'message':{'raw':base64.urlsafe_b64encode(message.as_bytes()).decode()}}
    try:
        result=google.api('gmail','drafts/'+urllib.parse.quote(record['gmail_draft'],safe=''),payload,method='PUT')
        if result.get('id')!=record['gmail_draft'] or not result.get('message',{}).get('threadId'):
            raise ValueError('Gmail did not confirm the updated draft. Refresh its status before retrying.')
        with tracker.db() as db:
            db.execute("UPDATE outreach_messages SET subject=?,body=?,thread_id=?,provider_message=?,error='' WHERE id=?",(subject,body,result['message']['threadId'],result['message'].get('id',''),record['id']))
    except Exception as exc:
        with tracker.db() as db:db.execute("UPDATE outreach_messages SET state='Needs reconciliation',error=? WHERE id=?",(str(exc)[:500],record['id']))
        raise


def edit(identifier,subject,body,expected_token=None):
    if not isinstance(subject,str) or not isinstance(body,str) or not subject.strip() or '\n' in subject or len(subject)>240 or not 1<=len(body)<=20000:raise ValueError('Invalid subject or body')
    with tracker.db() as db:
        row=db.execute('SELECT * FROM outreach_messages WHERE id=?',(identifier,)).fetchone()
        if not row or row['state']!='Local draft':raise ValueError('Only a local draft can be edited here; edit a Gmail draft in Gmail')
        if expected_token is not None and expected_token!=approval_token(row):raise ValueError('This draft changed on another Mac. Reopen it before saving; your text is retained.')
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


def approval_token(row):
    return hashlib.sha256(json.dumps([row['id'],row['recipient'],row['subject'],row['body'],google.config().get('resume_sha256','') if row['kind']=='initial' else '',google.ACCOUNT],ensure_ascii=False).encode()).hexdigest()


def send_approved(identifier,token):
    # Only a reviewed local draft may use this path. Gmail-edited drafts stay in Gmail.
    if google.config().get('enabled'):
        import sheets
        sheets.sync()
    with tracker.db() as db:conflict=db.execute("SELECT 1 FROM sync_conflicts WHERE job_id=(SELECT job_id FROM outreach_messages WHERE id=?) AND field IN ('mail_data','recruiter_contact','recruiter_name')",(identifier,)).fetchone()
    if conflict:raise ValueError('Resolve the conflicting Sheet changes before sending this email')
    with tracker.db() as db:row=db.execute('SELECT * FROM outreach_messages WHERE id=?',(identifier,)).fetchone()
    if not row or row['state']!='Local draft' or token!=approval_token(row):raise ValueError('Draft or master resume changed. Review it again before sending.')
    job=tracker.get_job(row['job_id'])
    if job['stage'] in ('Archived','Rejected') or job['verification']=='Expired':raise ValueError('This job is closed for outreach')
    # Inspect any existing linked thread before following up.
    if row['kind']=='followup':
        reconcile()
        with tracker.db() as db:initial=db.execute("SELECT state FROM outreach_messages WHERE job_id=? AND kind='initial'",(row['job_id'],)).fetchone()
        if not initial or initial[0]!='Sent':raise ValueError('A reply or uncertain send needs review before following up')
    create_gmail(identifier)
    with tracker.db() as db:
        row=dict(db.execute('SELECT * FROM outreach_messages WHERE id=?',(identifier,)).fetchone())
        if token!=approval_token(row):raise ValueError('Resume changed; review the Gmail draft before sending')
        db.execute("UPDATE outreach_messages SET state='Sending' WHERE id=?",(identifier,))
    try:
        result=google.api('gmail','drafts/send',{'id':row['gmail_draft']},approved_send=True)
        if not result.get('id') or 'SENT' not in result.get('labelIds',[]):raise ValueError('Gmail has not confirmed sending; checking the linked thread is required')
        # Gmail confirmation is re-read for the actual send date/follow-up clock.
        with tracker.db() as db:db.execute("UPDATE outreach_messages SET state='Needs send reconciliation',provider_message=?,thread_id=? WHERE id=?",(result['id'],result.get('threadId',row['thread_id']),identifier))
        reconcile()
    except Exception as exc:
        with tracker.db() as db:db.execute("UPDATE outreach_messages SET state='Needs send reconciliation',error=? WHERE id=?",(str(exc)[:500],identifier))
        raise

def reconcile():
    with tracker.db() as db:rows=[dict(r) for r in db.execute("SELECT * FROM outreach_messages WHERE state!='Local draft'")]
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
        sent=[];linked=[]
        for message in thread.get('messages',[]):
            if 'SENT' not in message.get('labelIds',[]):continue
            if int(message.get('internalDate','0'))<int(datetime.fromisoformat(row['created_at']).timestamp()*1000):continue
            if headers(message).get('message-id')==row['rfc_id'] or message.get('id')==row['provider_message']:sent.append(message)
            h=headers(message)
            recipients={address.lower() for _,address in getaddresses([h.get('to','')])}
            senders={address.lower() for _,address in getaddresses([h.get('from','')])}
            if row['recipient'].lower() in recipients and google.ACCOUNT.lower() in senders:linked.append(message)
        # Gmail can replace both the API ID and RFC Message-ID on send. An
        # unambiguous new SENT message in our linked thread is still evidence.
        if not sent and len(linked)==1:sent=linked
        if not sent:continue
        message=sent[0];stamp=datetime.fromtimestamp(int(message['internalDate'])/1000,timezone.utc).isoformat()
        incoming=[m for m in thread.get('messages',[]) if int(m.get('internalDate',0))>int(message['internalDate']) and not {'SENT','DRAFT','TRASH'}.intersection(m.get('labelIds',[])) and google.ACCOUNT not in [a.lower() for _,a in getaddresses([headers(m).get('from','')])]]
        state='Needs review' if incoming else 'Sent'
        revision=hashlib.sha256(json.dumps(sorted((item['id'],str(item.get('internalDate',''))) for item in incoming)).encode()).hexdigest() if incoming else ''
        with tracker.db() as db:
            db.execute('UPDATE outreach_messages SET state=?,sent_at=?,thread_id=?,rfc_id=?,provider_message=?,error=?,reply_revision=? WHERE id=?',(state,stamp,thread['id'],headers(message).get('message-id',row['rfc_id']),message['id'],'',revision,row['id']))
            db.execute('UPDATE jobs SET outreach_stage=?,updated_at=? WHERE id=?',('Replied' if incoming else 'Sent',tracker.now(),row['job_id']))
            if incoming:db.execute("UPDATE job_meta SET follow_up='' WHERE job_id=?",(row['job_id'],))
            elif row['kind']=='initial' and not row['sent_at']:db.execute('UPDATE job_meta SET follow_up=? WHERE job_id=?',(business_due(stamp),row['job_id']))
            elif row['kind']=='followup':db.execute("UPDATE job_meta SET follow_up='' WHERE job_id=?",(row['job_id'],))


def snapshot():
    ensure_ids()
    with tracker.db() as db:
        return {'messages':[{**dict(r),'approval_token':approval_token(r)} for r in db.execute('SELECT * FROM outreach_messages ORDER BY created_at DESC')],
                'meta':[dict(r) for r in db.execute('SELECT job_id,follow_up,reviewed_reply FROM job_meta')],
                'conflicts':[dict(r) for r in db.execute('SELECT * FROM sync_conflicts')],'threads':dict(_threads)}

def numbered_jobs():
    ensure_ids()
    with tracker.db() as db:numbers={r[0]:r[1] for r in db.execute('SELECT job_id,discovery_number FROM job_meta')}
    return sorted(({**job,'number':numbers[job['id']]} for job in tracker.list_jobs()),key=lambda job:job['number'],reverse=True)


def portable_mail(job_id):
    with tracker.db() as db:
        rows=[{k:row[k] for k in MAIL_FIELDS} for row in db.execute('SELECT * FROM outreach_messages WHERE job_id=? ORDER BY kind',(job_id,))]
    return json.dumps(rows,sort_keys=True,separators=(',',':'))


def decode_mail(text):
    rows=json.loads(text)
    if not isinstance(rows,list) or len(rows)>2:raise ValueError('Invalid shared mail records')
    kinds=set();result=[]
    for row in rows:
        if not isinstance(row,dict):raise ValueError('Invalid shared mail record')
        unknown=set(row)-MAIL_FIELDS
        if unknown:raise ValueError('Unsupported shared mail fields: '+', '.join(sorted(unknown))+'. Update lorca on both Macs.')
        if MAIL_REQUIRED-set(row) or row.get('kind') not in ('initial','followup') or row['kind'] in kinds:raise ValueError('Invalid shared mail record')
        if not all(isinstance(v,str) for v in row.values()) or len(row.get('body',''))>20000:raise ValueError('Invalid shared mail values')
        kinds.add(row['kind']);uuid.UUID(row['id']);datetime.fromisoformat(row['created_at'])
        if not re.fullmatch(r'[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+',row.get('recipient','')):raise ValueError('Invalid shared recipient')
        if row.get('state') not in ('Local draft','Creating','Gmail draft','Needs reconciliation','Sending','Needs send reconciliation','Sent','Needs review'):raise ValueError('Invalid shared mail state')
        if any(len(row.get(key,''))>200 for key in ('resume_choice','resume_sha256','resume_version')):raise ValueError('Invalid shared resume metadata')
        result.append({**MAIL_DEFAULTS,**row})
    return sorted(result,key=lambda row:row['kind'])


def normalize_mail(text):
    return json.dumps(decode_mail(text or '[]'),sort_keys=True,separators=(',',':'))


def import_mail(db,job_id,text):
    rows=decode_mail(text);kinds={row['kind'] for row in rows}
    for row in rows:
        # Import is tracking data only. It never grants permission to create/send mail.
        existing=db.execute('SELECT id FROM outreach_messages WHERE job_id=? AND kind=?',(job_id,row['kind'])).fetchone()
        if existing:row={**row,'id':existing[0]}
        conflict=db.execute('SELECT job_id FROM outreach_messages WHERE id=?',(row['id'],)).fetchone()
        if conflict and conflict[0]!=job_id:raise ValueError('Shared mail identity belongs to another job')
        row={**row,'job_id':job_id}
        fields=list(row)
        db.execute('INSERT INTO outreach_messages('+','.join(fields)+') VALUES('+','.join('?' for _ in fields)+') ON CONFLICT(job_id,kind) DO UPDATE SET '+','.join(k+'=excluded.'+k for k in fields if k not in ('job_id','kind')),list(row.values()))
    for kind in ('initial','followup'):
        if kind not in kinds:db.execute("DELETE FROM outreach_messages WHERE job_id=? AND kind=? AND state='Local draft'",(job_id,kind))


def reply_receipt(job_id):
    with tracker.db() as db:rows=db.execute("SELECT kind,reply_revision,sent_at FROM outreach_messages WHERE job_id=? AND state='Needs review'",(job_id,)).fetchall()
    return '|'.join(sorted(row['kind']+':'+(row['reply_revision'] or row['sent_at']) for row in rows))


def review_reply(job_id,receipt):
    current=reply_receipt(job_id)
    if not current or receipt!=current:raise ValueError('The reply changed. Refresh the conversation before marking it reviewed.')
    with tracker.db() as db:
        db.execute('UPDATE job_meta SET reviewed_reply=? WHERE job_id=?',(current,job_id))
        db.execute('INSERT INTO history(job_id,action,happened_at) VALUES(?,?,?)',(job_id,'Reply reviewed by you',tracker.now()))
