"""One-minute, field-aware two-way sync for the existing phone monitor."""
import json
import re
import urllib.parse
import uuid
from datetime import date,datetime
import google_connections as google
import jobtracker as tracker
import outreach
import resumes
from monitor_data import snapshot as monitor_snapshot, HEADERS

FIELDS={'stage':4,'notes':19,'priority':20,'follow_up':21,'recruiter_name':10,'recruiter_contact':12,'contact_origin':26,'mail_data':27,'application_data':28,'reviewed_reply':29}
EXTRA=['Notes','Priority','Follow-up date','Lorca ID','Lorca record','Outreach detail','Found number','Contact origin','Mail workflow','Application resume','Reviewed reply']
META_FIELDS={'follow_up','contact_origin','mail_data','application_data','reviewed_reply'}

def contact_value(field,value):
    # The legacy phone monitor used a display label for a missing contact.
    # It carries no contact data and must compare equal to an empty cell.
    if field=='recruiter_contact' and isinstance(value,str) and value.strip()=='Not publicly established':return ''
    if field=='mail_data':
        # Optional metadata added on one Mac must not look like simultaneous
        # edits to the whole email record on an older baseline.
        try:return outreach.normalize_mail(value)
        except (ValueError,TypeError):return value
    return value

def column(index):
    result='';index+=1
    while index:index,remainder=divmod(index-1,26);result=chr(65+remainder)+result
    return result


def merge_value(base,local,remote):
    if local==remote:return local,False
    if base is not None and local==base:return remote,False
    if base is not None and remote==base:return local,False
    return local,True


def validate(field,value):
    if not isinstance(value,str):raise ValueError('Sheet values must be text')
    if field=='stage' and value not in tracker.STATUSES:raise ValueError('Invalid stage in Google Sheet')
    if field=='priority' and value not in tracker.PRIORITIES:raise ValueError('Invalid priority in Google Sheet')
    if field=='follow_up' and value:date.fromisoformat(value)
    if field=='contact_origin' and value not in ('','Provided by you'):raise ValueError('Invalid contact origin')
    if field=='recruiter_contact' and value and len(value)>1024:raise ValueError('Contact is too long')
    if len(value)>(45000 if field in ('mail_data','application_data') else 20000):raise ValueError('Sheet value exceeds limit')
    if field=='mail_data':
        return outreach.normalize_mail(value)
    if field=='application_data' and not isinstance(json.loads(value or '{}'),dict):raise ValueError('Invalid application resume')
    return contact_value(field,value)


def resolve(job_id,field,choice):
    if field not in FIELDS or choice not in ('local','sheet'):raise ValueError('Invalid conflict choice')
    with tracker.db() as db:
        row=db.execute('SELECT * FROM sync_conflicts WHERE job_id=? AND field=?',(job_id,field)).fetchone()
        if not row:raise ValueError('Conflict no longer exists')
        value=validate(field,row['local_value'] if choice=='local' else row['remote_value'])
        if field in META_FIELDS:
            if field=='mail_data':outreach.import_mail(db,job_id,value or '[]')
            if field=='application_data':resumes.import_record(db,job_id,value or '{}')
            db.execute('UPDATE job_meta SET '+field+'=? WHERE job_id=?',(value,job_id))
        else:db.execute('UPDATE jobs SET '+field+'=?,updated_at=? WHERE id=?',(value,tracker.now(),job_id))
        meta=db.execute('SELECT baseline FROM job_meta WHERE job_id=?',(job_id,)).fetchone()
        baseline=json.loads(meta[0]);baseline[field]=row['remote_value']
        db.execute('UPDATE job_meta SET baseline=? WHERE job_id=?',(json.dumps(baseline),job_id))
        db.execute('DELETE FROM sync_conflicts WHERE job_id=? AND field=?',(job_id,field))


def found_number(row):
    # Record metadata is portable identity data, independent of row position or
    # a Mac's SQLite IDs. Retain Z/A fallback for older Sheet records.
    try:record=json.loads(row[23]) if len(row)>23 and row[23] else {}
    except (ValueError,TypeError):record={}
    original=record.get('discovery_number') if isinstance(record,dict) else None
    for value in (original,row[25] if len(row)>25 else None,row[0] if row else None):
        if str(value).isdigit() and int(value)>0:return int(value)
    return None


def cloud_record(job,dossier,number):
    # This job record carries facts/evidence only. Profile/templates use the
    # dedicated settings tab; credentials, PDFs and local paths are excluded.
    fields={k:v for k,v in job.items() if k in tracker.EDITABLE and k not in ('resume_file','cover_letter_file','application_approved','outreach_approved')}
    value={'job':fields,'dossier':dossier,'discovery_number':number,'created_at':job.get('created_at','')}
    text=json.dumps(value,ensure_ascii=False)
    if len(text)>45000:raise ValueError('Job record is too large for a Sheets cell')
    return text


def bootstrap_row(row):
    if len(row)<24 or not row[22] or not row[23]:return None
    identity=row[22]
    uuid.UUID(identity)
    data=json.loads(row[23]);job=tracker.validate(data['job'])
    # Provider status from a sheet is descriptive, never a Gmail send receipt.
    job['outreach_stage']='Not started'
    job['application_approved']=False;job['outreach_approved']=False
    for field in ('stage','notes','priority','recruiter_name','recruiter_contact'):
        if len(row)>FIELDS[field]:job[field]=validate(field,row[FIELDS[field]])
    follow_up=validate('follow_up',row[21])
    dossier=data.get('dossier')
    payload=json.dumps(dossier) if isinstance(dossier,dict) else None
    # Keep a downloaded record and its identity/evidence in one transaction.
    # The same tracker imported on another Mac must not duplicate the same URL.
    with tracker.db() as db:
        db.execute('BEGIN IMMEDIATE')
        existing=db.execute('SELECT job_id FROM job_meta WHERE sync_id=?',(identity,)).fetchone()
        if existing:return existing[0]
        same=db.execute('SELECT id FROM jobs WHERE job_url=? AND company=? AND title=?',(job.get('job_url',''),job['company'],job['title'])).fetchall() if job.get('job_url') else []
        if len(same)>1:raise ValueError('Multiple local jobs match the Sheet record; resolve duplicate postings first')
        if same:
            identifier=same[0][0]
            db.execute("INSERT INTO job_meta(job_id,sync_id) VALUES(?,?) ON CONFLICT(job_id) DO UPDATE SET sync_id=excluded.sync_id,baseline='{}'",(identifier,identity))
            return identifier
        stamp=tracker.now()
        original_time=data.get('created_at') or (dossier.get('researched_at') if isinstance(dossier,dict) else None)
        try:original_time=datetime.fromisoformat(original_time).isoformat()
        except (TypeError,ValueError):original_time=stamp
        job.update(created_at=original_time,updated_at=stamp)
        identifier=db.execute('INSERT INTO jobs('+','.join(job)+') VALUES('+','.join('?' for _ in job)+')',list(job.values())).lastrowid
        db.execute('INSERT INTO history(job_id,action,happened_at) VALUES(?,?,?)',(identifier,'Downloaded job summary from Google Sheets',stamp))
        db.execute('INSERT INTO job_meta(job_id,sync_id,follow_up,discovery_number) VALUES(?,?,?,?)',(identifier,identity,follow_up,found_number(row) or identifier))
        if payload:
            db.execute('INSERT INTO research_dossiers(job_id,run_id,researched_at,payload) VALUES(?,?,?,?)',(identifier,0,dossier.get('researched_at') or original_time,payload))
    return identifier



def sync():
    resumes.init()
    config=google.config();sid=google.sheet_id(config.get('sheet_url',''))
    prefix=sid
    def api(path,body=None):return google.api('sheets',prefix+path,body)
    metadata=api('?fields=sheets(properties(sheetId,title,gridProperties),basicFilter)')
    tabs={s['properties']['title']:s['properties'] for s in metadata.get('sheets',[])}
    missing=[name for name in ('Opportunities','Status') if name not in tabs]
    if missing:
        api(':batchUpdate',{'requests':[{'addSheet':{'properties':{'title':name}}} for name in missing]})
        metadata=api('?fields=sheets(properties(sheetId,title,gridProperties),basicFilter)')
        tabs={s['properties']['title']:s['properties'] for s in metadata['sheets']}
    remote=api('/values/'+urllib.parse.quote("'Opportunities'!A:AD",safe='')).get('values',[])
    by_uuid={};legacy={}
    for number,row in enumerate(remote[1:],2):
        if len(row)>22 and row[22]:
            if row[22] in by_uuid:raise ValueError('Duplicate Lorca ID in Sheet; fix the duplicate row before syncing')
            by_uuid[row[22]]=(number,row)
        elif row and str(row[0]).isdigit():legacy[int(row[0])]=(number,row)
    outreach.ensure_ids()
    with tracker.db() as db:known={r['sync_id'] for r in db.execute('SELECT sync_id FROM job_meta')}
    remote_urls=set()
    for identity,(_,row) in by_uuid.items():
        if identity not in known:
            record=json.loads(row[23]) if len(row)>23 and row[23] else {}
            posting=record.get('job',{}).get('job_url','')
            if posting and posting in remote_urls:raise ValueError('Duplicate posting identities in Sheet; resolve duplicate rows first')
            if posting:remote_urls.add(posting)
            bootstrap_row(row)
        number=found_number(row)
        if number is not None:
            with tracker.db() as db:db.execute('UPDATE job_meta SET discovery_number=? WHERE sync_id=?',(number,identity))
    with tracker.db() as db:
        for row in db.execute('SELECT job_id FROM job_meta').fetchall():
            db.execute('UPDATE job_meta SET mail_data=? WHERE job_id=?',(outreach.portable_mail(row[0]),row[0]))
            db.execute('UPDATE job_meta SET application_data=? WHERE job_id=?',(resumes.portable(row[0]),row[0]))
    jobs=tracker.list_jobs()
    with tracker.db() as db:
        metas={r['job_id']:dict(r) for r in db.execute('SELECT * FROM job_meta')}
        dossiers={r['job_id']:json.loads(r['payload']) for r in db.execute('SELECT job_id,payload FROM research_dossiers')}
    patches=[];baselines=[];next_row=max(2,len(remote)+1);conflict_count=0
    header=HEADERS+EXTRA
    if not remote or remote[0]!=header:patches.append({'range':"'Opportunities'!A1:AD1",'values':[header]})
    projection=monitor_snapshot(tracker.ROOT)
    monitor={int(r[0]):r for r in projection['Opportunities'][1:]}
    for job in jobs:
        meta=metas[job['id']];identity=meta['sync_id'];pair=by_uuid.get(identity)
        if not pair and job['id'] in legacy:
            number,row=legacy[job['id']]
            if len(row)>2 and (row[1],row[2])==(job['company'],job['title']):pair=(number,row)
        if pair:number,remote_row=pair
        else:number,remote_row=next_row,[];next_row+=1
        baseline={key:contact_value(key,value) for key,value in json.loads(meta['baseline']).items()};next_base=baseline.copy();blocked=set()
        for field,col in FIELDS.items():
            raw_local=meta[field] if field in META_FIELDS else job[field]
            local=contact_value(field,raw_local)
            # Old monitor has no notes/priority/follow-up columns. Missing cells
            # are absent on first adoption, not requests to erase local values.
            remote_value=str(remote_row[col]) if len(remote_row)>col else (local if field not in baseline else ('[]' if field=='mail_data' else '{}' if field=='application_data' else ''))
            if field=='mail_data' and not remote_value:remote_value='[]'
            if field=='application_data' and not remote_value:remote_value='{}'
            placeholder=field=='recruiter_contact' and remote_value.strip()=='Not publicly established'
            try:remote_value=validate(field,remote_value)
            except (ValueError,TypeError):
                remote_value=str(remote_value);conflict=True;chosen=local
            else:
                if field not in baseline and placeholder:chosen,conflict=local,False
                elif field not in baseline and local in ('','[]','{}') and field in ('mail_data','contact_origin','application_data','reviewed_reply'):chosen,conflict=remote_value,False
                else:chosen,conflict=merge_value(baseline.get(field),local,remote_value)
            if conflict:
                with tracker.db() as db:db.execute('INSERT OR REPLACE INTO sync_conflicts(job_id,field,local_value,remote_value) VALUES(?,?,?,?)',(job['id'],field,local,remote_value))
                blocked.add(field);conflict_count+=1
            else:
                with tracker.db() as db:
                    db.execute('DELETE FROM sync_conflicts WHERE job_id=? AND field=?',(job['id'],field))
                    # Do not replace a local edit made while the API read was in flight.
                    current=outreach.portable_mail(job['id']) if field=='mail_data' else resumes.portable(job['id']) if field=='application_data' else db.execute('SELECT '+(field+' FROM job_meta' if field in META_FIELDS else field+' FROM jobs')+' WHERE '+('job_id' if field in META_FIELDS else 'id')+'=?',(job['id'],)).fetchone()[0]
                    if contact_value(field,current)!=local:
                        blocked.add(field);continue
                    if chosen!=raw_local:
                        if field in META_FIELDS:
                            if field=='mail_data':outreach.import_mail(db,job['id'],chosen)
                            if field=='application_data':resumes.import_record(db,job['id'],chosen)
                            db.execute('UPDATE job_meta SET '+field+'=? WHERE job_id=?',(chosen,job['id']))
                        else:db.execute('UPDATE jobs SET '+field+'=?,updated_at=? WHERE id=?',(chosen,tracker.now(),job['id']))
                next_base[field]=chosen
                if field in META_FIELDS:meta[field]=chosen
                else:job[field]=chosen
        row=list(monitor[job['id']])
        row[0]=meta['discovery_number']
        row[4]=job['stage'];row[10]=job['recruiter_name'];row[12]=job['recruiter_contact'];row+=[job['notes'],job['priority'],meta['follow_up'],identity,cloud_record(job,dossiers.get(job['id']),meta['discovery_number']),job['outreach_stage'],meta['discovery_number'],meta['contact_origin'],meta['mail_data'],meta['application_data'],meta['reviewed_reply']]
        # Write only changed cells. Conflicted editable cells are never overwritten.
        for col,value in enumerate(row):
            field=next((key for key,index in FIELDS.items() if index==col),None)
            if field in blocked:continue
            old=str(remote_row[col]) if len(remote_row)>col else ''
            if str(value)==old:continue
            letter=column(col)
            patches.append({'range':f"'Opportunities'!{letter}{number}",'values':[[value]]})
        baselines.append((json.dumps(next_base),job['id']))
    # Keep existing phone-monitor status useful; only its timestamp changes each minute.
    status=projection['Status'];status[2]=['Sync connection','Lorca two-way sync · notes, priority, stage, follow-up date']
    patches.append({'range':f"'Status'!A1:B{len(status)}",'values':status})
    requests=[]
    for name,rows,cols in [('Opportunities',next_row+1,30),('Status',len(status)+1,2)]:
        properties=tabs[name];grid=properties.get('gridProperties',{})
        if grid.get('rowCount',0)<rows or grid.get('columnCount',0)<cols:
            requests.append({'updateSheetProperties':{'properties':{'sheetId':properties['sheetId'],'gridProperties':{'rowCount':max(rows,grid.get('rowCount',0)),'columnCount':max(cols,grid.get('columnCount',0))}},'fields':'gridProperties'}})
    if requests:api(':batchUpdate',{'requests':requests})
    # Sheets has no compare-and-swap. Recheck before writing so a phone edit
    # made during this sync is retried instead of overwritten from an old read.
    latest=api('/values/'+urllib.parse.quote("'Opportunities'!A:AD",safe='')).get('values',[])
    if latest!=remote:raise ValueError('Sheet changed during sync; retrying on the next cycle without overwriting it')
    if patches:api('/values:batchUpdate',{'valueInputOption':'RAW','data':patches})
    # Sort complete rows after cell writes so UUIDs, evidence and phone edits
    # stay attached to their job. New records must not sit below the old filter.
    properties=tabs['Opportunities'];end_row=next_row-1
    current_ids=[int(row[0]) for row in remote[1:] if row and str(row[0]).isdigit()]
    desired_ids=sorted((meta['discovery_number'] for meta in metas.values()),reverse=True)
    layout=[]
    if current_ids!=desired_ids:
        layout.append({'sortRange':{'range':{'sheetId':properties['sheetId'],'startRowIndex':1,'endRowIndex':end_row,'startColumnIndex':0,'endColumnIndex':max(30,properties.get('gridProperties',{}).get('columnCount',30))},'sortSpecs':[{'dimensionIndex':0,'sortOrder':'DESCENDING'}]}})
    old_filter=next((sheet.get('basicFilter',{}) for sheet in metadata['sheets'] if sheet['properties']['title']=='Opportunities'),{})
    filter_range={'sheetId':properties['sheetId'],'startRowIndex':0,'endRowIndex':end_row,'startColumnIndex':0,'endColumnIndex':30}
    if old_filter.get('range')!=filter_range:
        layout.append({'setBasicFilter':{'filter':{**old_filter,'range':filter_range}}})
    if layout:api(':batchUpdate',{'requests':layout})
    with tracker.db() as db:db.executemany('UPDATE job_meta SET baseline=? WHERE job_id=?',baselines)
    return {'jobs':len(jobs),'changed_cells':len(patches)-1,'conflicts':conflict_count}
