"""Small durable handoffs between fresh research sessions; no transcript replay."""
from __future__ import annotations
from datetime import datetime, timezone
import json
import os
from pathlib import Path

MAX_PROMPT_CHARS = 32000
MAX_RECENT_TASKS = 12


def atomic_text(path, text):
    path=Path(path);path.parent.mkdir(parents=True,exist_ok=True)
    temporary=path.with_name(path.name+'.tmp')
    with temporary.open('w',encoding='utf-8') as handle:
        handle.write(text);handle.flush();os.fsync(handle.fileno())
    temporary.replace(path)
    try:
        directory=os.open(path.parent,os.O_RDONLY)
        try:os.fsync(directory)
        finally:os.close(directory)
    except OSError:pass  # Some filesystems do not support syncing directory handles.


def atomic_json(path, data):
    atomic_text(path,json.dumps(data,indent=2,ensure_ascii=False)+'\n')


def read_context(state):
    path=Path(state)/'context.json'
    if not path.exists():return {'version':1,'updated_at':'','active_task':None,'recent_tasks':[]}
    try:
        data=json.loads(path.read_text(encoding='utf-8'))
        if not isinstance(data,dict) or not isinstance(data.get('recent_tasks'),list):raise ValueError('Invalid context format')
        return data
    except (ValueError,OSError):
        # Saved jobs/packets remain authoritative even if the optional handoff is damaged.
        return {'version':1,'updated_at':'','active_task':None,'recent_tasks':[],
                'warning':'The context file could not be read. Saved briefs remain in the tracker; discovery can resume.'}


def checkpoint(state,run_id,phase,outcome,message,folder=None,lead=None,preferences_version=''):
    state=Path(state);data=read_context(state)
    stamp=datetime.now(timezone.utc).isoformat(timespec='seconds')
    entry={'run_id':run_id,'phase':phase,'outcome':outcome,'message':str(message)[:350],'at':stamp}
    if folder:
        try:entry['evidence_path']=str(Path(folder).relative_to(state.parent))
        except ValueError:entry['evidence_path']=str(folder)
    if lead:entry['company']=str(lead.get('company',''))[:120]
    if phase in ('research','discovery'):
        if outcome=='started':
            data['active_task']={'phase':phase,'lead':lead,'started_at':stamp,'evidence_path':entry.get('evidence_path','')}
        else:data['active_task']=None
    data.update(version=1,updated_at=stamp,run_id=run_id,preferences_version=preferences_version,
                recent_tasks=[*data.get('recent_tasks',[]),entry][-MAX_RECENT_TASKS:])
    atomic_json(state/'context.json',data)
    lines=['# Research handoff','',f'Last checkpoint: {stamp}',
           '', 'Saved SQLite dossiers and source files are the source of truth. This is a compact progress summary.',
           'Fresh model context is used for every discovery batch and every researched opportunity.',
           'Current preferences and the primary master resume override older handoffs.','']
    active=data.get('active_task')
    if active:
        lead=active.get('lead') or {}
        lines += ['## Next task after an interruption','',f"Resume {active['phase']}: {lead.get('company','lead discovery')} {lead.get('title','')}",
                  f"Source: {lead.get('job_url','See lead queue')}",
                  'Click Start research after reopening the dashboard. Already saved briefs are skipped.','']
    else:lines += ['No unfinished task. Continue with the persisted lead queue or discover a new batch.','']
    lines += ['## Recent task checkpoints','']
    for item in reversed(data['recent_tasks']):
        lines.append(f"- {item['at']} | {item['phase']} / {item['outcome']}: {item['message']}")
        if item.get('evidence_path'):lines.append(f"  Evidence: `{item['evidence_path']}`")
    atomic_text(state/'CONTEXT.md','\n'.join(lines)+'\n')
    return data


def compact_context(state):
    data=read_context(state)
    # Exclude full source pages, emails, transcripts, and accumulated model reasoning.
    compact={'unfinished_task':data.get('active_task'),
             'recent_outcomes':[{k:item[k] for k in ('phase','outcome','message') if k in item}
                                for item in data.get('recent_tasks',[])[-4:]]}
    return json.dumps(compact,ensure_ascii=False)[:3000]


def bounded_prompt(text):
    if len(text)>MAX_PROMPT_CHARS:
        raise ValueError('Research input exceeds the 32,000-character prompt budget. Shorten preferences or the master resume; nothing was silently truncated.')
    return text
