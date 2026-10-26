"""Two isolated Mac profiles sharing a mock Sheet; no real Google access."""
import copy
from contextlib import contextmanager
import json
from pathlib import Path
import re
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import google_connections as google
import jobtracker as tracker
import shared_settings as settings


class SharedSettingsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.rows, self.tabs, self.writes = [], [], 0
        self.roots = {name: Path(self.tmp.name) / name for name in ('A', 'B')}
        for name in self.roots:
            with self.mac(name):
                settings.apply_values({'candidate_profile': '', 'search_preferences': '',
                                       'initial_template': '', 'followup_template': ''})
                google.save_config({**google.config(), 'sheet_url': 'https://docs.google.com/spreadsheets/d/abcdefghijklmnopqrstuvwx/edit',
                                    'api_key': 'PRIVATE_KEY_' + name, 'client_secret': 'PRIVATE_CLIENT_' + name,
                                    'resume_name': 'PRIVATE_RESUME.pdf', 'resume_sha256': 'PRIVATE_PDF_HASH'})
                (tracker.ROOT / 'profile/approved-resume.pdf').write_bytes(b'%PDF-PRIVATE_FILE')

    def tearDown(self):
        self.tmp.cleanup()

    @contextmanager
    def mac(self, name):
        root = self.roots[name]
        with patch.object(tracker, 'ROOT', root), patch.object(google, 'ROOT', root), \
             patch.object(google, 'CONFIG', root / 'google.json'), patch.object(google, 'api', side_effect=self.api):
            yield

    def api(self, service, path, body=None):
        self.assertEqual(service, 'sheets')
        if '?fields=' in path:
            return {'sheets': [{'properties': {'title': title}} for title in self.tabs]}
        if path.endswith(':batchUpdate') and '/values:' not in path:
            self.tabs.extend(r['addSheet']['properties']['title'] for r in body['requests'])
            return {}
        if '/values/' in path:
            return {'values': copy.deepcopy(self.rows)}
        if path.endswith('/values:batchUpdate'):
            self.assertEqual(body['valueInputOption'], 'RAW')
            self.writes += 1
            for entry in body['data']:
                index = int(re.search(r'!A(\d+):B\d+', entry['range'])[1]) - 1
                while len(self.rows) <= index: self.rows.append([])
                self.rows[index] = entry['values'][0].copy()
            return {}
        self.fail('Unexpected API request: ' + path)

    def publish(self):
        values = {'master_latex': '\\documentclass{article}\\begin{document}Resume\\end{document}',
                  'candidate_profile': 'Factual test profile: Python and backend projects.',
                  'search_preferences': 'Test search: junior backend engineering roles.',
                  'initial_template': 'Subject: {{role}}\nHello {{first_name}}, exact template wording.',
                  'followup_template': 'Subject: Follow up\nExact follow-up wording.'}
        with self.mac('A'):
            settings.apply_values(values); settings.sync()
        return values

    def test_two_macs_round_trip_profile_preferences_and_templates_without_secrets(self):
        original = self.publish()
        with self.mac('B'):
            settings.sync()
            self.assertEqual(settings.local_values(), original)
            self.assertEqual(google.config()['client_secret'], 'PRIVATE_CLIENT_B')
            self.assertEqual((tracker.ROOT / 'profile/approved-resume.pdf').read_bytes(), b'%PDF-PRIVATE_FILE')
            updated = {**original, 'candidate_profile': 'Updated test profile: Python, Swift, and backend projects.',
                       'initial_template': 'Subject: Updated\nKeep this wording exactly.'}
            settings.apply_values(updated); settings.sync()
        with self.mac('A'):
            settings.sync(); self.assertEqual(settings.local_values(), updated)
            self.assertEqual(google.config()['api_key'], 'PRIVATE_KEY_A')
            self.assertTrue((tracker.ROOT / 'private/settings-before-sync.json').exists())
        cloud = json.dumps(self.rows)
        self.assertNotIn('PRIVATE_', cloud)
        self.assertNotIn('client_secret', cloud)
        self.assertEqual({row[0] for row in self.rows[1:]}, set(settings.FIELDS))

    def test_initial_different_profiles_require_review_without_overwrite(self):
        original = self.publish()
        with self.mac('B'):
            other = 'Different reviewed candidate profile on the second Mac.'
            settings.apply_values({'candidate_profile': other})
            self.assertEqual(settings.sync()['conflicts'], 1)
            self.assertEqual(settings.local_values()['candidate_profile'], other)
            self.assertEqual(settings.conflicts()[0]['remote_value'], original['candidate_profile'])
            settings.resolve('candidate_profile', 'sheet'); settings.sync()
            self.assertEqual(settings.local_values()['candidate_profile'], original['candidate_profile'])
            self.assertEqual(settings.conflicts(), [])

    def test_concurrent_edits_are_preserved_until_local_choice_is_synced(self):
        self.publish()
        with self.mac('B'): settings.sync()
        with self.mac('A'):
            settings.apply_values({'initial_template': 'Subject: Mac A\nNew wording from A.'}); settings.sync()
        with self.mac('B'):
            text = 'Subject: Mac B\nDifferent wording from B.'
            settings.apply_values({'initial_template': text})
            self.assertEqual(settings.sync()['conflicts'], 1)
            self.assertEqual(settings.local_values()['initial_template'], text)
            settings.resolve('initial_template', 'local'); settings.sync()
        with self.mac('A'):
            settings.sync(); self.assertEqual(settings.local_values()['initial_template'], text)

    def test_research_defers_profile_updates_but_receives_templates(self):
        original = self.publish()
        with self.mac('B'): settings.sync()
        with self.mac('A'):
            settings.apply_values({'candidate_profile': 'New factual profile with additional projects and skills.',
                                   'followup_template': 'Subject: Update\nUpdated follow-up.'}); settings.sync()
        with self.mac('B'):
            self.assertEqual(settings.sync(research_active=True)['deferred'], 1)
            self.assertEqual(settings.local_values()['candidate_profile'], original['candidate_profile'])
            self.assertIn('Updated follow-up', settings.local_values()['followup_template'])
            settings.sync()
            self.assertTrue(settings.local_values()['candidate_profile'].startswith('New factual'))

    def test_research_started_during_sync_defers_profile_and_preferences(self):
        import integrations, outreach, research, service, sheets
        original = self.publish()
        with self.mac('B'): settings.sync()
        with self.mac('A'):
            settings.apply_values({'candidate_profile': 'A new factual profile with additional reviewed projects.',
                                   'search_preferences': 'Updated preferences for junior backend engineering.',
                                   'initial_template': 'Subject: New template\nPreserve this new wording.'})
            settings.sync()
        active = {'value': False}
        with self.mac('B'):
            def api(name, path, body=None):
                if not active['value']: service.dispatch({'op': 'start', 'model': 'test-model'})
                return self.api(name, path, body)
            def begin(): active['value'] = True
            with patch.object(service, 'ROOT', self.roots['B']), patch.object(service, 'snapshot', return_value={}), \
                 patch.object(research, 'status', side_effect=lambda: {'run': {'status': 'running'} if active['value'] else None}), \
                 patch.object(research, 'start', side_effect=begin), patch.object(google, 'api', side_effect=api), \
                 patch.object(outreach, 'reconcile'), patch.object(sheets, 'sync', return_value={'jobs': 49, 'conflicts': 0}):
                integrations.sync()
            self.assertTrue(active['value'])
            self.assertEqual(settings.local_values()['candidate_profile'], original['candidate_profile'])
            self.assertEqual(settings.local_values()['search_preferences'], original['search_preferences'])
            self.assertIn('New template', settings.local_values()['initial_template'])
            settings.sync(research_active=False)
            self.assertTrue(settings.local_values()['candidate_profile'].startswith('A new factual'))
            self.assertTrue(settings.local_values()['search_preferences'].startswith('Updated preferences'))

    def test_sheet_race_does_not_advance_baseline_or_write_local_files(self):
        self.publish()
        with self.mac('B'):
            before = settings.local_values(); writes = self.writes; reads = 0
            def changing(service, path, body=None):
                nonlocal reads
                response = self.api(service, path, body)
                if '/values/' in path:
                    reads += 1
                    if reads == 2: response['values'][1][1] += ' Changed remotely'
                return response
            with patch.object(google, 'api', side_effect=changing):
                with self.assertRaisesRegex(ValueError, 'changed during sync'): settings.sync()
            self.assertEqual(self.writes, writes)
            self.assertEqual(settings.local_values(), before)
            self.assertFalse(settings.state_path().exists())

    def test_blank_templates_can_be_cleared_and_placeholder_profile_is_not_uploaded(self):
        self.publish()
        with self.mac('B'):
            settings.sync(); settings.apply_values({'initial_template': ''}); settings.sync()
        with self.mac('A'):
            settings.sync(); self.assertEqual(settings.local_values()['initial_template'], '')
        self.assertNotIn(settings.DEFAULT_PROFILE, json.dumps(self.rows))

    def test_profile_editor_rejects_stale_profile_only_changes(self):
        with self.mac('A'):
            version = settings.profile_version()
            settings.apply_values({'candidate_profile': 'A remotely updated candidate profile with reviewed facts.'})
            with self.assertRaisesRegex(ValueError, 'changed'):
                settings.save_profile('Old editor profile with unsaved edits.', 'Valid search preferences for testing.', version)

    def test_template_editor_rejects_changes_received_after_opening(self):
        import integrations
        with self.mac('A'):
            version = settings.templates_version()
            text = 'Subject: Received remotely\nNew template wording.'
            settings.apply_values({'initial_template': text})
            with self.assertRaisesRegex(ValueError, 'Templates changed'):
                integrations.save({'templatesVersion': version, 'initial_template': 'Old editor contents'})
            self.assertEqual(settings.local_values()['initial_template'], text)
            integrations.save({'templatesVersion': settings.templates_version(), 'initial_template': 'Subject: Reviewed\nUser-approved wording.'})

    def test_malformed_and_unknown_remote_settings_do_not_import_credentials(self):
        self.publish(); self.rows.append(['api_key', 'REMOTE_PRIVATE_SECRET'])
        with self.mac('B'):
            settings.sync(); self.assertEqual(google.config()['api_key'], 'PRIVATE_KEY_B')
            self.assertNotIn('REMOTE_PRIVATE_SECRET', settings.state_path().read_text())
            self.rows[1][1] = 'x' * 24001
            before = settings.local_values()
            with self.assertRaisesRegex(ValueError, 'Invalid shared setting'): settings.sync()
            self.assertEqual(settings.local_values(), before)


if __name__ == '__main__': unittest.main()
