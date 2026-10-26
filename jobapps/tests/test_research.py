"""Offline regression tests: use a temporary DB; never contact real services."""
import importlib
import json
import os
import sqlite3
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))

class ResearchTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory()
        os.environ['JOB_PIPELINE_DB']=str(Path(self.tmp.name)/'test.sqlite3')
        import jobtracker
        import research
        self.j=importlib.reload(jobtracker);self.r=importlib.reload(research)
        self.j.init_db();self.r.init_research()
        self.root=patch.object(self.r,'ROOT',Path(self.tmp.name));self.root.start()
        self.pref=patch.object(self.r,'PREFERENCES',Path(self.tmp.name)/'preferences.md');self.pref.start()
        self.state=patch.object(self.r,'STATE',Path(self.tmp.name)/'.research');self.state.start()
        (Path(self.tmp.name)/'profile').mkdir(exist_ok=True)
        (Path(self.tmp.name)/'profile'/'master_resume.md').write_text('Python, FastAPI; software engineering internship; expected graduation December 2026')
        self.r.PREFERENCES.write_text('# Search preferences\nNYC new grad software engineering')
        self.data={'role':{'company':'Acme','title':'Software Engineer','location':'New York, NY','job_url':'https://acme.test/jobs/123','summary':'Build APIs.','fit':'Python experience.','gaps':'Confirm start date.','priority':'High','salary':'$100k'},'posting_evidence':{'url':'https://acme.test/jobs/123','quote':'Acme is hiring a Software Engineer in New York.','official':True},'application_evidence':{'url':'https://acme.test/jobs/123','quote':'Apply now for this Software Engineer opportunity.'},'contact':{}}
        self.content='Acme is hiring a Software Engineer in New York. Apply now for this Software Engineer opportunity. Python required.'
        self.events=[{'type':'tool_result','name':'web_extract','output':json.dumps({'results':[{'url':'https://acme.test/jobs/123','content':self.content,'error':None}]})}]
    def tearDown(self):
        self.root.stop();self.pref.stop();self.state.stop();self.tmp.cleanup();os.environ.pop('JOB_PIPELINE_DB',None)
    def test_search_snippet_cannot_verify(self):
        events=[{**self.events[0],'name':'web_search'}]
        with self.assertRaises(ValueError):self.r.prepare_dossier(self.data,events)
    def test_repair_model_quote_from_actual_source(self):
        self.data['posting_evidence']['quote']='This quote never appeared anywhere.'
        _,payload=self.r.prepare_dossier(self.data,self.events)
        self.assertIn(payload['posting_evidence']['quote'],self.content)
    def test_salary_without_quote_is_removed(self):
        fields,payload=self.r.prepare_dossier(self.data,self.events)
        self.assertEqual(fields['salary'],'');self.assertEqual(payload['role']['salary'],'')
        self.assertEqual(fields['verification'],'Verified')
    def test_missing_apply_evidence_downgrades(self):
        self.data['application_evidence']={}
        fields,_=self.r.prepare_dossier(self.data,self.events)
        self.assertEqual(fields['verification'],'Unverified');self.assertEqual(fields['verified_at'],'')
    def test_guessed_email_is_removed(self):
        quote='Jane Doe recruits engineers for Acme in New York.'
        self.events.append({'type':'tool_result','name':'web_extract','output':json.dumps({'results':[{'url':'https://acme.test/team','content':quote+' More company information and recruiting opportunities.'}]})})
        self.data['contact']={'name':'Jane Doe','email':'jane@acme.test','evidence':{'url':'https://acme.test/team','quote':quote}}
        fields,payload=self.r.prepare_dossier(self.data,self.events)
        self.assertFalse(payload['contact']['email']);self.assertNotIn('@',fields['recruiter_contact'])
    def test_duplicate_url_and_preserved_decisions(self):
        fields,payload=self.r.prepare_dossier(self.data,self.events)
        self.assertTrue(self.r.ingest(1,fields,payload));self.assertFalse(self.r.ingest(1,fields,payload))
        self.assertEqual(len(self.j.list_jobs()),1)
        job=self.j.list_jobs()[0];self.j.update_job(job['id'],{'stage':'Applied'})
        self.assertFalse(self.r.ingest(2,fields,payload));self.assertEqual(self.j.get_job(job['id'])['stage'],'Applied')
    def test_existing_lead_enriched_without_approvals(self):
        self.j.create_job({'company':'Acme','title':'Software Engineer','job_url':'https://acme.test/jobs/123?utm_source=test'})
        fields,payload=self.r.prepare_dossier(self.data,self.events)
        self.assertTrue(self.r.ingest(1,fields,payload));self.assertEqual(len(self.j.list_jobs()),1)
        job=self.j.list_jobs()[0];self.assertEqual(job['stage'],'Ready for review');self.assertEqual(job['application_approved'],0)
    def test_approved_record_is_never_overwritten(self):
        job=self.j.create_job({'company':'Acme','title':'Software Engineer','job_url':'https://acme.test/jobs/123','application_approved':True})
        fields,payload=self.r.prepare_dossier(self.data,self.events)
        self.assertFalse(self.r.ingest(1,fields,payload));self.assertEqual(self.j.get_job(job['id'])['application_approved'],1)
    def test_preferences_conflict(self):
        original=self.r.preferences();self.r.PREFERENCES.write_text('# Changed externally\nRemote roles instead of NYC')
        with self.assertRaises(ValueError):self.r.save_preferences('New search preferences for junior NYC jobs',original['version'])
    def test_preferences_roundtrip(self):
        original=self.r.preferences();saved=self.r.save_preferences('# Search preferences\nRemote junior Python roles',original['version'])
        self.assertEqual(saved,self.r.preferences())
    def test_no_match_is_not_saved(self):
        self.assertIsNone(self.r.prepare_dossier({'no_match':True},[]))
    def test_wrong_employer_cannot_be_ingested(self):
        self.data['role']['company']='Different Employer'
        with self.assertRaises(ValueError):self.r.prepare_dossier(self.data,self.events)
    def test_closed_posting_is_rejected(self):
        self.events[0]['output']=json.dumps({'results':[{'url':'https://acme.test/jobs/123','content':self.content+' This position is closed.'}]})
        with self.assertRaises(ValueError):self.r.prepare_dossier(self.data,self.events)
    def test_salary_must_match_quoted_amount(self):
        self.data['salary_evidence']={'url':'https://acme.test/jobs/123','quote':'Salary for this position: $150k - $170k.'}
        self.events[0]['output']=json.dumps({'results':[{'url':'https://acme.test/jobs/123','content':self.content+' Salary for this position: $150k - $170k.'}]})
        fields,_=self.r.prepare_dossier(self.data,self.events);self.assertEqual(fields['salary'],'')
    def test_start_is_singleton_and_stop_does_not_signal_other_processes(self):
        from types import SimpleNamespace
        with patch.object(self.r,'preflight'),patch.object(self.r.subprocess,'Popen',return_value=SimpleNamespace(pid=os.getpid())) as launch:
            first=self.r.start();second=self.r.start()
        self.assertTrue(first['started']);self.assertFalse(second['started']);self.assertEqual(launch.call_count,1)
        self.r.stop();self.assertTrue(self.r.stopped(first['run_id']))
    def test_aggregator_cannot_be_claimed_official(self):
        url='https://wellfound.com/jobs/123'
        self.data['role']['job_url']=url;self.data['posting_evidence']['url']=url;self.data['application_evidence']['url']=url
        self.events[0]['output']=json.dumps({'results':[{'url':url,'content':self.content}]})
        with self.assertRaises(ValueError):self.r.prepare_dossier(self.data,self.events)
    def test_discovery_queue_deduplicates(self):
        self.r.STATE.mkdir()
        lead={'company':'Acme','title':'Software Engineer','job_url':'https://acme.test/jobs/123'}
        self.assertEqual(self.r.queue_leads({'leads':[lead,lead]}),1)
        self.assertEqual(self.r.queue_leads({'leads':[lead]}),0)
        self.assertEqual(self.r.next_lead(1)['company'],'Acme')
        from research_context import checkpoint
        checkpoint(self.r.STATE,1,'research','completed','Done')
        self.assertIsNone(self.r.next_lead(1))
        self.assertEqual(self.r.queue_leads({'leads':[lead]},1),0)
    def test_context_is_bounded_and_readable(self):
        from research_context import checkpoint,read_context,compact_context,MAX_RECENT_TASKS
        for i in range(30):checkpoint(self.r.STATE,1,'research','skipped',f'Skipped lead {i}')
        context=read_context(self.r.STATE)
        self.assertEqual(len(context['recent_tasks']),MAX_RECENT_TASKS)
        self.assertIn('Research handoff',(self.r.STATE/'CONTEXT.md').read_text())
        compact=json.loads(compact_context(self.r.STATE));self.assertEqual(len(compact['recent_outcomes']),4)
    def test_unfinished_lead_resumes_after_stop(self):
        from research_context import checkpoint
        lead={'company':'Acme','title':'Software Engineer','job_url':'https://acme.test/jobs/123'}
        checkpoint(self.r.STATE,1,'research','started','Reading Acme',lead=lead)
        checkpoint(self.r.STATE,1,'run','stopped','Power loss or stop')
        self.assertEqual(self.r.next_lead(2),lead)
    def test_saved_lead_is_not_resumed(self):
        from research_context import checkpoint
        fields,payload=self.r.prepare_dossier(self.data,self.events);self.r.ingest(1,fields,payload)
        lead={'company':'Acme','title':'Software Engineer','job_url':'https://acme.test/jobs/123'}
        checkpoint(self.r.STATE,1,'research','started','Interrupted after saving',lead=lead)
        self.assertIsNone(self.r.next_lead(2))
    def test_prompt_budget_does_not_silently_truncate(self):
        from research_context import bounded_prompt,MAX_PROMPT_CHARS
        with self.assertRaises(ValueError):bounded_prompt('x'*(MAX_PROMPT_CHARS+1))
    def test_corrupt_context_does_not_lose_tracker(self):
        from research_context import read_context
        self.r.STATE.mkdir();(self.r.STATE/'context.json').write_text('{broken')
        self.assertIn('warning',read_context(self.r.STATE))
        self.assertEqual(self.j.list_jobs(),[])
    def test_reboot_pid_reuse_does_not_block_resume(self):
        with self.j.db() as cx:
            cx.execute('INSERT INTO research_runs(status,started_at,heartbeat,model,pid) VALUES(?,?,?,?,?)',('running',self.j.now(),self.j.now(),'test',os.getpid()))
        self.assertEqual(self.r.status()['run']['status'],'interrupted')
    def test_worker_checkpoints_finished_brief(self):
        from research_context import read_context
        from types import SimpleNamespace
        self.j.create_job({'company':'Acme','title':'Software Engineer','job_url':'https://acme.test/jobs/123'})
        with self.j.db() as cx:
            rid=cx.execute('INSERT INTO research_runs(status,started_at,heartbeat,model) VALUES(?,?,?,?)',('starting',self.j.now(),self.j.now(),'test')).lastrowid
        with patch.object(self.r,'run_session',return_value=(self.data,self.events)),patch.object(self.r.signal,'signal'),patch.object(self.r.subprocess,'Popen',return_value=SimpleNamespace(terminate=lambda:None)):
            self.r.worker(rid,once=True)
        memory=read_context(self.r.STATE)
        self.assertIsNone(memory['active_task'])
        self.assertTrue(any(t['phase']=='research' and t['outcome']=='saved' for t in memory['recent_tasks']))
        self.assertEqual(self.j.list_jobs()[0]['stage'],'Ready for review')
    def test_worker_stop_preserves_unfinished_task(self):
        from research_context import read_context
        from types import SimpleNamespace
        self.j.create_job({'company':'Acme','title':'Software Engineer','job_url':'https://acme.test/jobs/123'})
        with self.j.db() as cx:
            rid=cx.execute('INSERT INTO research_runs(status,started_at,heartbeat,model) VALUES(?,?,?,?)',('starting',self.j.now(),self.j.now(),'test')).lastrowid
        with patch.object(self.r,'run_session',side_effect=InterruptedError('Power loss simulation')),patch.object(self.r.signal,'signal'),patch.object(self.r.subprocess,'Popen',return_value=SimpleNamespace(terminate=lambda:None)):
            self.r.worker(rid,once=True)
        self.assertEqual(read_context(self.r.STATE)['active_task']['lead']['company'],'Acme')
        self.assertEqual(self.r.next_lead(rid+1)['company'],'Acme')
    def test_fenced_json_with_commentary(self):
        text='Here is the brief:\n```json\n'+json.dumps(self.data)+'\n```\nSummary: ready for review.'
        self.assertEqual(self.r.parse_answer(text)['role']['company'],'Acme')
    def test_plain_json_with_trailing_commentary(self):
        self.assertEqual(self.r.parse_answer(json.dumps(self.data)+'\nSummary: complete.')['role']['company'],'Acme')
    def test_multiple_json_blocks_are_rejected(self):
        with self.assertRaises(ValueError):self.r.parse_answer('```json\n{}\n```\n```json\n{}\n```')
        with self.assertRaises(ValueError):self.r.parse_answer('{}\n{}')
    def test_partial_json_is_not_repaired(self):
        with self.assertRaises(ValueError):self.r.parse_answer('{"role":{"company":"Acme"')
    def test_nonzero_complete_brief_requires_evidence(self):
        data=self.r.completed_answer({'text':json.dumps(self.data),'exit_code':1},1)
        fields,payload=self.r.prepare_dossier(data,self.events)
        self.assertEqual(fields['company'],'Acme');self.assertTrue(any('nonzero' in w for w in payload['warnings']))
        with self.assertRaises(ValueError):self.r.prepare_dossier(data,[])
    def test_interrupted_or_error_final_is_rejected(self):
        for result,code in [({'text':json.dumps(self.data),'exit_code':130},130),({'text':json.dumps(self.data),'exit_code':1,'error':'Interrupted'},1),({'text':json.dumps(self.data),'exit_code':0},-15)]:
            with self.assertRaises(RuntimeError):self.r.completed_answer(result,code)
        with self.assertRaises(RuntimeError):self.r.completed_answer(None,1)
    def test_nonzero_no_match_is_not_a_recovered_brief(self):
        with self.assertRaises(RuntimeError):self.r.completed_answer({'text':'{"no_match":true}','exit_code':1},1)
    def transcript_fixture(self, rows):
        home=Path(self.tmp.name)/'hermes';home.mkdir(exist_ok=True)
        with sqlite3.connect(home/'state.db') as cx:
            cx.execute('CREATE TABLE sessions(id TEXT,source TEXT)')
            cx.execute('CREATE TABLE messages(id INTEGER PRIMARY KEY,session_id TEXT,role TEXT,tool_name TEXT,content TEXT)')
            cx.execute('INSERT INTO sessions VALUES(?,?)',('test-session','job-research'))
            for name,content in rows:
                cx.execute('INSERT INTO messages(session_id,role,tool_name,content) VALUES(?,?,?,?)',('test-session','tool',name,content))
        return home,[{'type':'system','subtype':'init','session_id':'test-session'}]

    def test_stop_sets_flag(self):
        with self.j.db() as cx:
            rid=cx.execute('INSERT INTO research_runs(status,started_at,heartbeat,model) VALUES(?,?,?,?)',('running',self.j.now(),self.j.now(),'test')).lastrowid
        self.r.stop();self.assertTrue(self.r.stopped(rid))
    def test_url_dedupe_strips_tracking(self):
        self.assertEqual(self.r.canonical('https://acme.test/jobs/123/?utm_source=x#apply'),'https://acme.test/jobs/123')

if __name__=='__main__':unittest.main()
