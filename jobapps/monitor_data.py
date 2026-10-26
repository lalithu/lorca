"""Read-only projection of the local tracker for the private Google monitor."""
import json
import sqlite3
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo

HEADERS = ['ID', 'Company', 'Role', 'Location', 'Stage', 'Verification', 'Published compensation',
           'Role summary', 'Why it could fit', 'Check before applying', 'Recruiter / hiring contact',
           'Contact / outreach status', 'Recruiter email / contact', 'Public profile', 'Job posting', 'Contact source',
           'Email source', 'Researched (Eastern)', 'Evidence notes']


def eastern(value):
    if not value:
        return ''
    return datetime.fromisoformat(value).astimezone(ZoneInfo('America/New_York')).strftime('%Y-%m-%d %I:%M:%S %p ET')


def snapshot(root, sync_label='Snapshot only — Google authorization needed for automatic sync'):
    root = Path(root)
    with sqlite3.connect((root / 'pipeline.sqlite3').as_uri() + '?mode=ro', uri=True) as cx:
        cx.row_factory = sqlite3.Row
        jobs = [dict(r) for r in cx.execute('SELECT * FROM jobs ORDER BY updated_at DESC, id DESC')]
        dossiers = {r['job_id']: dict(r) for r in cx.execute('SELECT * FROM research_dossiers')}
        row = cx.execute('SELECT * FROM research_runs ORDER BY id DESC LIMIT 1').fetchone()
        run = dict(row) if row else {}
    context_path = root / '.research' / 'context.json'
    try:
        context = json.loads(context_path.read_text())
    except (OSError, ValueError):
        context = {}
    return build_snapshot(jobs, dossiers, run, context, sync_label)


def snapshot_http(sync_label='Automatic sync enabled — refreshes about once per minute'):
    """Use existing loopback read APIs; never access the project folder from launchd."""
    import urllib.request
    def read(path):
        # Ignore proxies and disallow redirects to keep local research on this Mac.
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, *args, **kwargs):
                return None
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        with opener.open('http://127.0.0.1:8765' + path, timeout=15) as response:
            return json.loads(response.read(16_000_000))
    jobs = read('/api/jobs')['jobs']
    research = read('/api/research')
    dossiers = {r['job_id']: {**r, 'payload': json.dumps(r['payload'])} for r in research['dossiers']}
    return build_snapshot(jobs, dossiers, research.get('run') or {}, research.get('context') or {}, sync_label)


def build_snapshot(jobs, dossiers, run, context, sync_label):
    values = [HEADERS]
    for job in jobs:
        dossier = dossiers.get(job['id'], {})
        data = json.loads(dossier.get('payload', '{}'))
        role, contact = data.get('role', {}), data.get('contact', {})
        values.append([job['id'], job['company'], job['title'], job['location'], job['stage'],
                       job['verification'], job['salary'] or 'Not established',
                       role.get('summary', job['description']), role.get('fit', ''), role.get('gaps', ''),
                       job['recruiter_name'] or contact.get('name', ''), job.get('outreach_stage') if job.get('outreach_stage') in ('Sent','Replied') else contact.get('status', 'Not researched'),
                       job.get('recruiter_contact') or contact.get('email') or 'Not publicly established', contact.get('profile_url', ''),
                       job['job_url'], (contact.get('evidence') or {}).get('url', ''),
                       (contact.get('email_evidence') or {}).get('url', ''),
                       eastern(dossier.get('researched_at')), '\n'.join(data.get('warnings', []))])
    updated = datetime.now(ZoneInfo('America/New_York')).strftime('%Y-%m-%d %I:%M:%S %p ET')
    status = [
        ["Lalith's Job Search", 'Private phone monitor'],
        ['Last successful sync (Eastern)', updated],
        ['Sync connection', sync_label],
        ['Research state', run.get('status', 'Not started')],
        ['Current activity', run.get('message', '')],
        ['Research heartbeat (Eastern)', eastern(run.get('heartbeat'))],
        ['Opportunities tracked', len(jobs)],
        ['Research briefs saved', len(dossiers)],
        ['Saved this shift', run.get('saved', 0)],
        ['Sessions this shift', run.get('attempts', 0)],
        ['Retries this shift', run.get('failures', 0)],
        ['Last checkpoint (Eastern)', eastern(context.get('updated_at'))],
        ['Monitoring', 'Phone view only. Review and apply through the official posting.'],
        ['Freshness', 'If the sync timestamp stops advancing, the Mac or its connection may be offline.'],
        ['Research memory', 'Recent checkpoints below.'],
    ]
    for task in context.get('recent_tasks', [])[-6:][::-1]:
        status.append([f"{task.get('phase', '')}: {task.get('outcome', '')}", task.get('message', '')])
    return {'Status': status, 'Opportunities': values}


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument('root')
    parser.add_argument('output')
    args = parser.parse_args()
    Path(args.output).write_text(json.dumps(snapshot(args.root), indent=2))
