"""Isolated stdio integration, migration and Lorca session transport tests."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))

class ServiceTests(unittest.TestCase):
    def test_migration_history_preferences_and_idempotence(self):
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp);source=base/'source';source.mkdir();(source/'profile').mkdir()
            (source/'profile/master_resume.md').write_text('Real reviewed profile with Python and SQL.')
            (source/'profile/preferences.md').write_text('# Search preferences\nNew York junior engineering roles')
            env=dict(os.environ,LORCA_JOBS_ROOT=str(source),JOB_PIPELINE_DB=str(source/'pipeline.sqlite3'))
            seed="import jobtracker as j;import research as r;j.init_db();r.init_research();j.create_job({'company':'Acme','title':'Engineer','job_url':'https://acme.test/job'});j.update_job(1,{'notes':'Keep this history'})"
            subprocess.run([sys.executable,'-B','-c',seed],env=env,cwd=Path(__file__).parents[1],check=True)
            root=base/'target';env.update(LORCA_JOBS_ROOT=str(root),JOB_PIPELINE_DB=str(root/'pipeline.sqlite3'))
            child=subprocess.Popen([sys.executable,'-B',str(Path(__file__).parents[1]/'service.py')],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
            def call(v):child.stdin.write(json.dumps(v)+'\n');child.stdin.flush();return json.loads(child.stdout.readline())
            try:
                self.assertEqual(call({'op':'snapshot'})['result']['jobs'],[])
                imported=call({'op':'import','path':str(source)})
                self.assertTrue(imported['ok'],imported)
                self.assertEqual(imported['result']['jobs'][0]['notes'],'Keep this history')
                self.assertEqual(len(call({'op':'detail','id':1})['result']['history']),2)
                self.assertFalse(call({'op':'import','path':str(source)})['ok'])
                state=call({'op':'snapshot'})['result']
                self.assertFalse(call({'op':'settings','version':'stale','preferences':'valid preferences for testing','profile':'Real factual candidate profile'})['ok'])
                self.assertTrue(call({'op':'settings','version':state['preferences']['version'],'preferences':'Updated search preferences for testing','profile':'Real factual candidate profile'})['ok'])
                self.assertEqual(call({'op':'update','id':1,'field':'stage','value':'Applied'})['result']['jobs'][0]['stage'],'Applied')
                self.assertFalse(call({'op':'update','id':1,'field':'email_send','value':'yes'})['ok'])
                self.assertTrue(call({'op':'quit'})['ok'])
                child.wait(timeout=10)
                with sqlite3.connect(source/'pipeline.sqlite3') as db:self.assertEqual(db.execute('select stage from jobs').fetchone()[0],'Discovered')
            finally:
                if child.poll() is None:child.terminate();child.wait()
                child.stdin.close();child.stdout.close()

    def test_early_core_exit_reports_actionable_error_and_cleans_up(self):
        import research
        class ClosedInput:
            def write(self, _):raise BrokenPipeError('child exited')
        class ExitedCore:
            stdin=ClosedInput()
            returncode=7
            def wait(self, **_):return 7
            def poll(self):return 7
        with tempfile.TemporaryDirectory() as tmp:
            with patch.dict(os.environ,{'LORCA_CORE':'fake-core'}), patch.object(research,'prompt',return_value='Research one role'), patch.object(research,'compact_context',return_value='{}'), patch.object(research.subprocess,'Popen',return_value=ExitedCore()):
                with self.assertRaisesRegex(RuntimeError,'exited before accepting input .*exit 7'):
                    research.run_session(1,1,Path(tmp))

    def test_exa_envelope_and_private_source_rejection(self):
        import web
        for url in ['http://example.com','https://localhost/a','https://127.0.0.1/x','https://10.0.0.1','https://[::1]']:
            with self.assertRaises(ValueError):web.public_url(url)
        self.assertEqual(web.public_url('https://jobs.ashbyhq.com/company/job'),'https://jobs.ashbyhq.com/company/job')
        class Response:
            def __enter__(self):return self
            def __exit__(self,*_):pass
            def read(self,*_):return b'data: {"result":{"content":[{"text":"Exact source page"}]}}\n\n'
        with patch.object(web.urllib.request,'urlopen',return_value=Response()):
            result=web.execute('web_extract',{'urls':['https://employer.test/job']})
            self.assertEqual(result['results'][0]['content'],'Exact source page')
        with self.assertRaises(ValueError):web.execute('run_command',{})

if __name__=='__main__':unittest.main()
