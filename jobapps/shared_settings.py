"""Allowlisted profile/template sync. Credentials and the approved PDF stay local."""
import hashlib
import json
import threading
import urllib.parse
import google_connections as google
import jobtracker as tracker
from research_context import atomic_text

DEFAULT_PROFILE = '# Candidate profile\nPaste and review your factual resume before starting research.\n'
DEFAULT_PREFERENCES = '# Job search preferences\nNew York City startups; full-time junior/new-grad software roles; expected graduation December 2026. Flag eligibility and start-date uncertainties.\n'
FIELDS = {'candidate_profile': 24000, 'search_preferences': 16000,
          'initial_template': 20000, 'followup_template': 20000}
TAB = 'Lorca Settings'
LOCK = threading.RLock()


def local_values():
    profile = tracker.ROOT / 'profile'
    def read(name, default):
        path = profile / name
        value = path.read_text() if path.exists() else default
        return '' if value.strip() == default.strip() or (name == 'master_resume.md' and 'Paste and review your factual resume' in value) else value
    config = google.config()
    return {'candidate_profile': read('master_resume.md', DEFAULT_PROFILE),
            'search_preferences': read('preferences.md', DEFAULT_PREFERENCES),
            **{key: config.get(key, '') for key in ('initial_template', 'followup_template')}}


def profile_version():
    values = local_values()
    return hashlib.sha256(json.dumps([values['candidate_profile'], values['search_preferences']]).encode()).hexdigest()


def templates_version():
    config = google.config()
    return hashlib.sha256(json.dumps([config.get(key, '') for key in ('initial_template', 'followup_template', 'sheet_url')]).encode()).hexdigest()


def save_profile(profile, preferences, version):
    validate('candidate_profile', profile); validate('search_preferences', preferences)
    if len(profile.strip()) < 20 or len(preferences.strip()) < 20:
        raise ValueError('Profile and preferences must each contain at least 20 characters')
    with LOCK:
        if version != profile_version():
            raise ValueError('Profile or preferences changed. Reopen settings before saving your edits.')
        atomic_text(tracker.ROOT / 'profile/master_resume.md', profile)
        atomic_text(tracker.ROOT / 'profile/preferences.md', preferences)


def validate(field, value):
    if field not in FIELDS or not isinstance(value, str) or len(value) > FIELDS[field]:
        raise ValueError('Invalid shared setting: ' + field)
    if field in ('candidate_profile', 'search_preferences') and value and len(value.strip()) < 20:
        raise ValueError(field.replace('_', ' ') + ' must contain at least 20 characters, or be empty')


def state_path():
    return tracker.ROOT / 'private/settings-sync.json'


def state(sheet_id):
    path = state_path()
    data = json.loads(path.read_text()) if path.exists() else {}
    if data.get('sheet_id') != sheet_id:
        return {'sheet_id': sheet_id, 'baseline': {}, 'conflicts': []}
    return data


def conflicts():
    try:
        return state(google.sheet_id(google.config().get('sheet_url', ''))).get('conflicts', [])
    except (ValueError, OSError):
        return []


def write_state(data):
    google.oauth.write_json(state_path(), data, private=True)


def apply_values(values):
    config = google.config()
    for field, value in values.items():
        validate(field, value)
        if field == 'candidate_profile':
            atomic_text(tracker.ROOT / 'profile/master_resume.md', value or DEFAULT_PROFILE)
        elif field == 'search_preferences':
            atomic_text(tracker.ROOT / 'profile/preferences.md', value or DEFAULT_PREFERENCES)
        else:
            config[field] = value
    if any(key in values for key in ('initial_template', 'followup_template')):
        google.save_config(config)


def merge(base, local, remote, known):
    if local == remote: return local, False
    if not known:
        if not local: return remote, False
        if not remote: return local, False
    elif local == base: return remote, False
    elif remote == base: return local, False
    return local, True


def sync(research_active=False):
    sid = google.sheet_id(google.config().get('sheet_url', ''))
    with LOCK:
        saved = state(sid)
        local = local_values()
    for field, value in local.items(): validate(field, value)
    metadata = google.api('sheets', sid + '?fields=sheets(properties(title))')
    if TAB not in {s['properties']['title'] for s in metadata.get('sheets', [])}:
        google.api('sheets', sid + ':batchUpdate', {'requests': [{'addSheet': {'properties': {'title': TAB}}}]})
    path = sid + '/values/' + urllib.parse.quote("'" + TAB + "'!A:B", safe='')
    rows = google.api('sheets', path).get('values', [])
    if rows and rows[0] != ['Setting', 'Value']:
        raise ValueError('The Lorca Settings tab has an unexpected header; no settings were overwritten')
    remote, positions = {}, {}
    for index, row in enumerate(rows[1:], 2):
        if not row or row[0] not in FIELDS: continue
        field = row[0]
        if field in remote: raise ValueError('Duplicate shared setting: ' + field)
        remote[field] = row[1] if len(row) > 1 else ''
        validate(field, remote[field]); positions[field] = index
    baseline = dict(saved.get('baseline', {}))
    pending, changes, found, deferred = {}, [], [], 0
    if not rows: changes.append({'range': "'Lorca Settings'!A1:B1", 'values': [['Setting', 'Value']]})
    next_row = max(2, len(rows) + 1)
    for field, value in local.items():
        chosen, conflict = (value, False) if field not in remote else merge(baseline.get(field), value, remote[field], field in baseline)
        if conflict:
            found.append({'field': field, 'local_value': value, 'remote_value': remote[field]}); continue
        if research_active and field in ('candidate_profile', 'search_preferences') and chosen != value:
            deferred += 1; continue
        pending[field] = chosen
        if field not in remote or chosen != remote[field]:
            index = positions.get(field, next_row)
            if field not in positions: next_row += 1
            changes.append({'range': f"'Lorca Settings'!A{index}:B{index}", 'values': [[field, chosen]]})
    if google.api('sheets', path).get('values', []) != rows:
        raise ValueError('Shared settings changed during sync; retry without overwriting them')
    with LOCK:
        if local_values() != local:
            raise ValueError('Local settings changed during sync; retry without overwriting them')
        if changes:
            google.api('sheets', sid + '/values:batchUpdate', {'valueInputOption': 'RAW', 'data': changes})
        updates = {key: value for key, value in pending.items() if value != local[key]}
        if updates:
            google.oauth.write_json(tracker.ROOT / 'private/settings-before-sync.json', local, private=True)
            apply_values(updates)
        baseline.update(pending)
        write_state({'sheet_id': sid, 'baseline': baseline, 'conflicts': found})
    return {'conflicts': len(found), 'deferred': deferred}


def resolve(field, choice):
    if field not in FIELDS or choice not in ('local', 'sheet'):
        raise ValueError('Invalid shared-settings choice')
    with LOCK:
        saved = state(google.sheet_id(google.config().get('sheet_url', '')))
        conflict = next((c for c in saved.get('conflicts', []) if c['field'] == field), None)
        if not conflict: raise ValueError('This settings conflict is no longer pending')
        if local_values()[field] != conflict['local_value']:
            raise ValueError('Local settings changed; sync again before resolving')
        value = conflict['local_value'] if choice == 'local' else conflict['remote_value']
        apply_values({field: value})
        saved['baseline'][field] = conflict['remote_value']
        saved['conflicts'] = [c for c in saved['conflicts'] if c['field'] != field]
        write_state(saved)
