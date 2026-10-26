import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import urllib.parse
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import google_connections as google
import jobtracker as tracker
import shared_resume

class SharedResumeTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name);self.rows=[];self.reads=[]
    def tearDown(self):self.tmp.cleanup()
    def api(self,service,path,body=None):
        if '?fields=' in path:return {'sheets':[{'properties':{'title':shared_resume.TAB}}]}
        if '/values/' in path:
            span=urllib.parse.unquote(path.split('/values/',1)[1]).split('!')[1];self.reads.append(span)
            if span=='A1:C2':return {'values':copy.deepcopy(self.rows[:2])}
            count=int(span.split(':A')[1]);return {'values':[[r[0]] for r in self.rows[2:count]]}
        if '/values:batchUpdate' in path:
            for change in body['data']:
                span=change['range'].split('!')[1];start=int(''.join(c for c in span.split(':')[0] if c.isdigit()))-1
                for offset,row in enumerate(change['values']):
                    while len(self.rows)<=start+offset:self.rows.append([])
                    self.rows[start+offset]=row.copy()
            return {}
        self.fail(path)
    def mac(self,name):
        from contextlib import ExitStack
        stack=ExitStack();root=self.root/name;root.mkdir(exist_ok=True)
        stack.enter_context(patch.object(tracker,'ROOT',root));stack.enter_context(patch.object(google,'ROOT',root));stack.enter_context(patch.object(google,'CONFIG',root/'google.json'))
        stack.enter_context(patch.object(google,'api',side_effect=self.api))
        if not google.CONFIG.exists():google.save_config({'sheet_url':'https://docs.google.com/spreadsheets/d/abcdefghijklmnopqrstuvwx/edit'})
        return stack
    def approve(self,text):
        path=tracker.ROOT/'incoming.pdf';path.write_bytes(b'%PDF-1.7\n'+text)
        google.approved_resume(str(path))
    def test_pdf_moves_between_macs_and_unchanged_sync_only_reads_manifest(self):
        with self.mac('A'):
            self.approve(b'Master A');shared_resume.sync();first=shared_resume.fingerprint()
            self.reads=[];shared_resume.sync();self.assertEqual(self.reads,['A1:C2'])
        with self.mac('B'):
            shared_resume.sync();self.assertEqual(shared_resume.fingerprint(),first)
            self.assertEqual(google.config()['resume_name'],'incoming.pdf')
            self.approve(b'Master B');shared_resume.sync();second=shared_resume.fingerprint()
        with self.mac('A'):
            shared_resume.sync();self.assertEqual(shared_resume.fingerprint(),second)
    def test_concurrent_changes_require_choice_and_corrupt_pdf_never_replaces_local(self):
        with self.mac('A'):self.approve(b'Original');shared_resume.sync()
        with self.mac('B'):shared_resume.sync();self.approve(b'B edit');shared_resume.sync()
        with self.mac('A'):
            self.approve(b'A edit');before=shared_resume.fingerprint()
            with self.assertRaisesRegex(ValueError,'Both Macs'):shared_resume.sync()
            self.assertTrue(shared_resume.state()['conflict']);self.assertEqual(shared_resume.fingerprint(),before)
            shared_resume.sync('sheet');self.assertFalse(shared_resume.state()['conflict'])
        with self.mac('B'):self.approve(b'Another B edit');shared_resume.sync()
        self.rows[2]=['corrupted']
        with self.mac('A'):
            before=shared_resume.fingerprint()
            with self.assertRaises(ValueError):shared_resume.sync()
            self.assertEqual(shared_resume.fingerprint(),before)
