import base64
from datetime import datetime,timezone
from email import message_from_bytes
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import jobtracker as tracker
import google_connections as google
import outreach
import research
import sheets
import resumes

class GoogleTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.root=Path(self.tmp.name)
        self.patches=[patch.object(tracker,'ROOT',self.root),patch.object(tracker,'DB_PATH',self.root/'pipeline.sqlite3'),patch.object(google,'ROOT',self.root),patch.object(google,'CONFIG',self.root/'google.json')]
        for p in self.patches:p.start()
        tracker.init_db();research.init_research();outreach.init()
        self.job=tracker.create_job({'company':'Acme','title':'Engineer','job_url':'https://acme.test/job','verification':'Verified','recruiter_name':'Jane Doe','recruiter_contact':'jane@acme.test'})
        with tracker.db() as db:db.execute('INSERT INTO research_dossiers VALUES(?,?,?,?)',(self.job['id'],0,tracker.now(),json.dumps({'role':{'fit':'Real profile facts'},'contact':{'name':'Jane Doe','email':'jane@acme.test','email_evidence':{'quote':'Jane Doe jane@acme.test','url':'https://acme.test/team'}}})))
        outreach.ensure_ids()
    def tearDown(self):
        for p in reversed(self.patches):p.stop()
        self.tmp.cleanup()
    def configure(self):
        google.save_config({'initial_template':'Subject: {{role}} at {{company}}\nHello {{first_name}},\nFixed wording. {{job_url}}\nLalith','followup_template':'Subject: Follow up\nHello {{first_name}}, following up.','sheet_url':'https://docs.google.com/spreadsheets/d/abcdefghijklmnopqrstuvwx/edit'})
        pdf=self.root/'resume.pdf';pdf.write_bytes(b'%PDF-1.7\nApproved test PDF');google.approved_resume(str(pdf))
    def test_templates_can_be_missing_and_later_preserve_wording(self):
        with self.assertRaisesRegex(ValueError,'template'):outreach.prepare(self.job['id'])
        self.configure();row=outreach.prepare(self.job['id'])
        self.assertEqual(row['subject'],'Engineer at Acme');self.assertEqual(row['body'],'Hello Jane,\nFixed wording. https://acme.test/job\nLalith')
        self.assertEqual(outreach.prepare(self.job['id'])['id'],row['id'])
        with patch.object(google,'api') as api:self.assertFalse(api.called)
    def test_gmail_draft_has_pdf_and_cannot_be_duplicated(self):
        self.configure();row=outreach.prepare(self.job['id']);captured=[]
        def api(service,path,body=None):
            captured.append((service,path,body));return {'id':'draft','message':{'id':'message','threadId':'thread'}}
        with patch.object(google,'api',side_effect=api):
            outreach.create_gmail(row['id'])
            with self.assertRaises(ValueError):outreach.create_gmail(row['id'])
        self.assertEqual([x[1] for x in captured],['drafts'])
        mime=message_from_bytes(base64.urlsafe_b64decode(captured[0][2]['message']['raw']))
        self.assertEqual(mime['To'],'jane@acme.test');self.assertEqual(len(mime.get_payload()),2)
        self.assertEqual(mime.get_payload()[1].get_content_type(),'application/pdf')
    def test_body_only_template_gets_subject_without_changing_wording(self):
        self.configure();config=google.config()
        config['initial_template']='Hello {{first_name}},\n\nMy original wording.\nLalith'
        google.save_config(config)
        record=outreach.prepare(self.job['id'])
        self.assertEqual(record['subject'],'Engineer @ Acme')
        self.assertEqual(record['body'],'Hello Jane,\n\nMy original wording.\nLalith')
    def test_prepare_outreach_creates_only_one_draft_even_after_ambiguous_failure(self):
        self.configure()
        with patch.object(google,'api',side_effect=TimeoutError('Network lost')) as api:
            with self.assertRaises(TimeoutError):outreach.prepare_gmail(self.job['id'])
            with self.assertRaisesRegex(ValueError,'needs checking'):outreach.prepare_gmail(self.job['id'])
            self.assertEqual(api.call_count,1)
        self.assertEqual(outreach.snapshot()['messages'][0]['state'],'Needs reconciliation')
    def test_gmail_handoff_and_automatic_followup_survive_second_mac(self):
        self.configure();config=google.config();config['enabled']=True;google.save_config(config)
        state,sheet_api=self.fake_sheet();created=[]
        def api(service,path,body=None):
            if service=='sheets':return sheet_api(service,path,body)
            self.assertEqual(path,'drafts');created.append(body)
            return {'id':'gmail-draft','message':{'id':'draft-message','threadId':'linked-thread'}}
        with patch.object(google,'api',side_effect=api):
            outreach.prepare_gmail(self.job['id']);sheets.sync()
        record=outreach.snapshot()['messages'][0]
        self.assertEqual(record['state'],'Gmail draft')
        self.assertEqual(outreach.snapshot()['meta'][0]['follow_up'],'')
        with patch.object(tracker,'DB_PATH',self.root/'second-mac.sqlite3'):
            tracker.init_db();research.init_research();outreach.init()
            with patch.object(google,'api',side_effect=api):
                sheets.sync();other=tracker.list_jobs()[0]
                outreach.prepare_gmail(other['id'])
            self.assertEqual(len(created),1)
            self.assertEqual(outreach.snapshot()['messages'][0]['gmail_draft'],'gmail-draft')
            sent_time=int(datetime.now(timezone.utc).timestamp()*1000)+1000
            sent={'id':'sent-message','labelIds':['SENT'],'internalDate':str(sent_time),
                  'payload':{'headers':[{'name':'Message-ID','value':record['rfc_id']},{'name':'From','value':google.ACCOUNT}]}}
            def sent_api(service,path,body=None):
                if service=='sheets':return sheet_api(service,path,body)
                if path.startswith('drafts/'):
                    exc=RuntimeError('Draft was sent');exc.status=404;raise exc
                self.assertTrue(path.startswith('threads/'))
                return {'id':'linked-thread','messages':[sent]}
            with patch.object(google,'api',side_effect=sent_api):
                outreach.reconcile();sheets.sync()
            expected=outreach.business_due(datetime.fromtimestamp(sent_time/1000,timezone.utc).isoformat())
            self.assertEqual(outreach.snapshot()['meta'][0]['follow_up'],expected)
        with patch.object(google,'api',side_effect=sheet_api):sheets.sync()
        self.assertEqual(outreach.snapshot()['messages'][0]['state'],'Sent')
        self.assertEqual(outreach.snapshot()['meta'][0]['follow_up'],expected)
    def broken_template_draft(self):
        self.configure();config=google.config()
        config['initial_template']='Hi ___,\nMy name is [NAME]. Interested in [ROLE] at [COMPANY].\n[ATTACH RESUME]'
        google.save_config(config);record=outreach.prepare(self.job['id'])
        with tracker.db() as db:db.execute('UPDATE outreach_messages SET body=? WHERE id=?',(config['initial_template'],record['id']))
        captured=[]
        def create(service,path,body=None):
            captured.append(body['message']['raw']);return {'id':'draft','message':{'id':'message','threadId':'thread'}}
        with patch.object(google,'api',side_effect=create):outreach.create_gmail(record['id'])
        return outreach.snapshot()['messages'][0],captured[0]

    def test_fill_existing_gmail_draft_preserves_id_pdf_and_custom_subject(self):
        from email import policy
        record,raw=self.broken_template_draft();mime=message_from_bytes(base64.urlsafe_b64decode(raw),policy=policy.default)
        mime.replace_header('Subject','My custom subject');raw=base64.urlsafe_b64encode(mime.as_bytes()).decode();calls=[]
        def api(service,path,body=None,method=None):
            calls.append((path,method,body))
            if body is None:return {'id':'draft','message':{'raw':raw}}
            return {'id':'draft','message':{'id':'updated-message','threadId':'thread'}}
        with patch.object(google,'api',side_effect=api):outreach.fill_gmail(record)
        self.assertEqual([call[1] for call in calls],[None,'PUT'])
        updated=message_from_bytes(base64.urlsafe_b64decode(calls[1][2]['message']['raw']),policy=policy.default)
        self.assertEqual(str(updated['Subject']),'My custom subject')
        self.assertIn('Hi Jane,',updated.get_body(preferencelist=('plain',)).get_content())
        self.assertIn('Lalith Uriti',updated.get_body(preferencelist=('plain',)).get_content())
        self.assertEqual(list(updated.iter_attachments())[0].get_payload(decode=True),(self.root/'profile/approved-resume.pdf').read_bytes())
        saved=outreach.snapshot()['messages'][0]
        self.assertEqual(saved['gmail_draft'],'draft');self.assertEqual(saved['provider_message'],'updated-message')
        self.assertNotIn('[NAME]',saved['body'])

    def test_fill_refuses_to_overwrite_gmail_body_edits(self):
        from email import policy
        record,raw=self.broken_template_draft();mime=message_from_bytes(base64.urlsafe_b64decode(raw),policy=policy.default)
        mime.get_body(preferencelist=('plain',)).set_content('My own reviewed email')
        with patch.object(google,'api',return_value={'id':'draft','message':{'raw':base64.urlsafe_b64encode(mime.as_bytes()).decode()}}) as api:
            with self.assertRaisesRegex(ValueError,'edited'):outreach.fill_gmail(record)
            self.assertEqual(api.call_count,1)
        self.assertEqual(outreach.snapshot()['messages'][0]['body'],record['body'])

    def test_ambiguous_draft_update_keeps_tracking_without_replay(self):
        record,raw=self.broken_template_draft()
        with patch.object(google,'api',side_effect=[{'id':'draft','message':{'raw':raw}},TimeoutError('Connection lost')]) as api:
            with self.assertRaises(TimeoutError):outreach.fill_gmail(record)
            self.assertEqual(api.call_count,2)
        self.assertEqual(outreach.snapshot()['messages'][0]['state'],'Needs reconciliation')
        self.assertEqual(outreach.snapshot()['messages'][0]['gmail_draft'],'draft')

    def test_changed_pdf_is_not_uploaded(self):
        self.configure();row=outreach.prepare(self.job['id']);(self.root/'profile/approved-resume.pdf').write_bytes(b'%PDF-different')
        with patch.object(google,'api') as api:
            with self.assertRaisesRegex(ValueError,'Resume changed'):outreach.create_gmail(row['id'])
            api.assert_not_called()
    def test_oversized_or_non_pdf_resume_keeps_approved_file(self):
        self.configure()
        approved=(self.root/'profile/approved-resume.pdf').read_bytes()
        fingerprint=google.config()['resume_sha256']
        invalid=self.root/'not-a-pdf.pdf';invalid.write_text('Wrong content')
        with self.assertRaisesRegex(ValueError,'PDF'):google.approved_resume(str(invalid))
        large=self.root/'large.pdf'
        with large.open('wb') as file:
            file.write(b'%PDF-1.7');file.truncate(15_000_001)
        with self.assertRaisesRegex(ValueError,'15 MB'):google.approved_resume(str(large))
        self.assertEqual((self.root/'profile/approved-resume.pdf').read_bytes(),approved)
        self.assertEqual(google.config()['resume_sha256'],fingerprint)

    def test_ambiguous_creation_never_retries(self):
        self.configure();row=outreach.prepare(self.job['id'])
        with patch.object(google,'api',side_effect=TimeoutError('network timeout')):
            with self.assertRaises(TimeoutError):outreach.create_gmail(row['id'])
        with patch.object(google,'api') as api:
            with self.assertRaises(ValueError):outreach.create_gmail(row['id'])
            api.assert_not_called()
        self.assertEqual(outreach.snapshot()['messages'][0]['state'],'Needs reconciliation')
    def test_no_email_send_api_exists(self):
        for path in ['messages/send','drafts/send','threads/delete']:
            with self.assertRaises(ValueError):google.api('gmail',path,{})
    def test_five_weekdays_and_no_false_send_on_deleted_draft(self):
        self.assertEqual(outreach.business_due('2026-10-23T15:00:00+00:00'),'2026-10-30')
        self.configure();row=outreach.prepare(self.job['id'])
        with patch.object(google,'api',return_value={'id':'draft','message':{'id':'msg','threadId':'thread'}}):outreach.create_gmail(row['id'])
        def api(service,path,*_):
            if path.startswith(('drafts/','threads/')):
                e=RuntimeError('not found');e.status=404;raise e
            return {'messages':[]}
        with patch.object(google,'api',side_effect=api):outreach.reconcile()
        self.assertEqual(outreach.snapshot()['messages'][0]['state'],'Needs reconciliation')
        self.assertEqual(tracker.get_job(self.job['id'])['outreach_stage'],'Drafted')
    def test_sent_and_reply_clear_followup(self):
        self.configure();row=outreach.prepare(self.job['id'])
        with patch.object(google,'api',return_value={'id':'draft','message':{'id':'msg','threadId':'thread'}}):outreach.create_gmail(row['id'])
        row=outreach.snapshot()['messages'][0];sent_time=int(datetime.now(timezone.utc).timestamp()*1000)+1000
        sent={'id':'sent','labelIds':['SENT'],'internalDate':str(sent_time),'payload':{'headers':[{'name':'Message-ID','value':row['rfc_id']},{'name':'From','value':google.ACCOUNT}]}}
        thread={'id':'thread','messages':[sent]}
        def api(service,path,*_):
            if path.startswith('drafts/'):
                e=RuntimeError('not found');e.status=404;raise e
            return thread
        with patch.object(google,'api',side_effect=api):outreach.reconcile()
        self.assertEqual(tracker.get_job(self.job['id'])['outreach_stage'],'Sent')
        self.assertTrue(outreach.snapshot()['meta'][0]['follow_up'])
        thread['messages'].append({'id':'reply','labelIds':['INBOX'],'internalDate':str(sent_time+1000),'payload':{'headers':[{'name':'From','value':'Jane <jane@acme.test>'}]}})
        with patch.object(google,'api',side_effect=api):outreach.reconcile()
        self.assertEqual(outreach.snapshot()['messages'][0]['state'],'Needs review')
        self.assertEqual(outreach.snapshot()['meta'][0]['follow_up'],'')
        with self.assertRaises(ValueError):outreach.prepare(self.job['id'],'followup')
    def test_gmail_rewritten_message_ids_require_a_unique_confirmed_send_to_the_recipient(self):
        self.configure();record=outreach.prepare(self.job['id'])
        with patch.object(google,'api',return_value={'id':'draft','message':{'id':'draft-message','threadId':'linked-thread'}}):outreach.create_gmail(record['id'])
        sent_time=int(datetime.now(timezone.utc).timestamp()*1000)+1000
        sent={'id':'new-sent-id','labelIds':['SENT'],'internalDate':str(sent_time),
              'payload':{'headers':[{'name':'Message-ID','value':'<rewritten@gmail.test>'},{'name':'From','value':google.ACCOUNT},{'name':'To','value':'someone-else@example.test'}]}}
        thread={'id':'linked-thread','messages':[sent]}
        def api(service,path,*_):
            if path.startswith('drafts/'):
                exc=RuntimeError('Draft no longer exists');exc.status=404;raise exc
            return thread
        with patch.object(google,'api',side_effect=api):outreach.reconcile()
        self.assertEqual(outreach.snapshot()['messages'][0]['state'],'Gmail draft')
        self.assertEqual(outreach.snapshot()['meta'][0]['follow_up'],'')
        sent['payload']['headers'][-1]['value']='jane@acme.test'
        thread['messages'].append({**sent,'id':'ambiguous-second-send'})
        with patch.object(google,'api',side_effect=api):outreach.reconcile()
        self.assertEqual(outreach.snapshot()['messages'][0]['state'],'Gmail draft')
        thread['messages']=thread['messages'][:1]
        with patch.object(google,'api',side_effect=api):outreach.reconcile()
        saved=outreach.snapshot()['messages'][0]
        self.assertEqual(saved['state'],'Sent');self.assertEqual(saved['provider_message'],'new-sent-id')
        self.assertEqual(saved['rfc_id'],'<rewritten@gmail.test>')
        expected=outreach.business_due(datetime.fromtimestamp(sent_time/1000,timezone.utc).isoformat())
        self.assertEqual(outreach.snapshot()['meta'][0]['follow_up'],expected)
    def test_reviewed_reply_is_portable_and_a_new_reply_requires_review_again(self):
        self.configure();row=outreach.prepare(self.job['id'])
        with tracker.db() as db:
            db.execute("UPDATE outreach_messages SET state='Needs review',sent_at='2026-10-01T12:00:00+00:00',reply_revision='r1' WHERE id=?",(row['id'],))
        with self.assertRaisesRegex(ValueError,'changed'):outreach.review_reply(self.job['id'],'stale')
        receipt=outreach.reply_receipt(self.job['id']);self.assertEqual(receipt,'initial:r1')
        outreach.review_reply(self.job['id'],receipt)
        state,api=self.fake_sheet()
        with patch.object(google,'api',side_effect=api):sheets.sync()
        with patch.object(tracker,'DB_PATH',self.root/'review-mac.sqlite3'):
            tracker.init_db();research.init_research();outreach.init()
            with patch.object(google,'api',side_effect=api):sheets.sync()
            job=tracker.list_jobs()[0]
            self.assertEqual(outreach.reply_receipt(job['id']),receipt)
            self.assertEqual(outreach.snapshot()['meta'][0]['reviewed_reply'],receipt)
        with tracker.db() as db:db.execute("UPDATE outreach_messages SET reply_revision='r2' WHERE id=?",(row['id'],))
        self.assertNotEqual(outreach.reply_receipt(self.job['id']),receipt)
        self.assertEqual(outreach.snapshot()['meta'][0]['reviewed_reply'],receipt)
    def test_three_way_merge_preserves_conflicts(self):
        self.assertEqual(sheets.merge_value('old','old','sheet'),('sheet',False))
        self.assertEqual(sheets.merge_value('old','local','old'),('local',False))
        self.assertEqual(sheets.merge_value('old','local','sheet'),('local',True))
        self.assertEqual(sheets.merge_value(None,'local','sheet'),('local',True))
        for field,value in [('stage','Bogus'),('priority','urgent'),('follow_up','tomorrow')]:
            with self.assertRaises(ValueError):sheets.validate(field,value)
    def test_manual_recruiter_can_prepare_without_fabricated_evidence(self):
        self.configure()
        outreach.save_contact(self.job['id'],'New Recruiter','new@acme.test')
        record=outreach.prepare(self.job['id'])
        self.assertEqual(record['recipient'],'new@acme.test')
        who,_=outreach.contact(tracker.get_job(self.job['id']))
        self.assertEqual(who['origin'],'Provided by you')
        with self.assertRaises(ValueError):outreach.save_contact(self.job['id'],'New Recruiter','not-an-email')
        self.assertEqual(tracker.get_job(self.job['id'])['recruiter_contact'],'new@acme.test')
    def test_manual_contact_and_mail_survive_a_second_mac_sync(self):
        self.configure();state,api=self.fake_sheet()
        outreach.save_contact(self.job['id'],'New Recruiter','new@acme.test');draft=outreach.prepare(self.job['id'])
        with patch.object(google,'api',side_effect=api):sheets.sync()
        self.assertEqual(state['rows'][1][26],'Provided by you')
        self.assertEqual(json.loads(state['rows'][1][27])[0]['id'],draft['id'])
        # Simulate another Mac: separate database, same remote Sheet.
        other=self.root/'second.sqlite3'
        with patch.object(tracker,'DB_PATH',other):
            tracker.init_db();research.init_research();outreach.init()
            with patch.object(google,'api',side_effect=api):sheets.sync()
            job=tracker.list_jobs()[0]
            self.assertEqual(job['recruiter_contact'],'new@acme.test')
            self.assertEqual(outreach.prepare(job['id'])['id'],draft['id'])
            who,_=outreach.contact(job);self.assertEqual(who['origin'],'Provided by you')
            outreach.save_contact(job['id'],'New Recruiter','changed@acme.test');outreach.prepare(job['id'])
            with patch.object(google,'api',side_effect=api):sheets.sync()
        with patch.object(google,'api',side_effect=api):sheets.sync()
        self.assertEqual(tracker.get_job(self.job['id'])['recruiter_contact'],'changed@acme.test')
        self.assertEqual(outreach.snapshot()['messages'][0]['recipient'],'changed@acme.test')
    def test_application_source_and_choice_survive_second_mac_sync(self):
        import uuid
        self.configure();state,api=self.fake_sheet();resumes.init()
        source='\\documentclass{article}\\begin{document}Approved facts\\end{document}'
        version=str(uuid.uuid4())
        record={'source':source,'master_source':source,'version':version,'selection':'tailored','saved_version':'','submitted_version':version,'updated_at':tracker.now()}
        with tracker.db() as db:resumes.import_record(db,self.job['id'],json.dumps(record))
        with patch.object(google,'api',side_effect=api):sheets.sync()
        with patch.object(tracker,'DB_PATH',self.root/'another.sqlite3'):
            tracker.init_db();research.init_research();outreach.init()
            with patch.object(google,'api',side_effect=api):sheets.sync()
            downloaded=tracker.list_jobs()[0]
            data=json.loads(resumes.portable(downloaded['id']))
            self.assertEqual(data['source'],source);self.assertEqual(data['submitted_version'],version)
            resumes.select(downloaded['id'],'master')
            with patch.object(google,'api',side_effect=api):sheets.sync()
        with patch.object(google,'api',side_effect=api):sheets.sync()
        self.assertEqual(json.loads(resumes.portable(self.job['id']))['selection'],'master')

    def test_shared_resume_mail_metadata_imports_over_legacy_baseline_and_survives_both_macs(self):
        self.configure();draft=outreach.prepare(self.job['id']);state,api=self.fake_sheet()
        with patch.object(google,'api',side_effect=api):sheets.sync()
        legacy=json.loads(outreach.portable_mail(self.job['id']))
        for record in legacy:
            for key in ('resume_choice','resume_sha256','resume_version'):record.pop(key)
        with tracker.db() as db:
            baseline=json.loads(db.execute('SELECT baseline FROM job_meta WHERE job_id=?',(self.job['id'],)).fetchone()[0])
            baseline['mail_data']=json.dumps(legacy)
            db.execute('UPDATE job_meta SET baseline=? WHERE job_id=?',(json.dumps(baseline),self.job['id']))
        shared=[{**legacy[0],'resume_choice':'tailored','resume_sha256':'a'*64,'resume_version':'saved-version'}]
        state['rows'][1][27]=json.dumps(shared)
        with patch.object(google,'api',side_effect=api):result=sheets.sync()
        self.assertEqual(result['conflicts'],0)
        current=json.loads(outreach.portable_mail(self.job['id']))[0]
        for key in ('resume_choice','resume_sha256','resume_version'):self.assertEqual(current[key],shared[0][key])
        self.assertEqual(current['id'],draft['id'])
        with patch.object(tracker,'DB_PATH',self.root/'metadata-other-mac.sqlite3'):
            tracker.init_db();research.init_research();outreach.init()
            with patch.object(google,'api',side_effect=api):result=sheets.sync()
            self.assertEqual(result['conflicts'],0)
            other=tracker.list_jobs()[0]
            self.assertEqual(json.loads(outreach.portable_mail(other['id']))[0],current)
        # A notes update must not strip the other Mac's attachment tracking.
        tracker.update_job(self.job['id'],{'notes':'A later local edit'})
        with patch.object(google,'api',side_effect=api):sheets.sync()
        self.assertEqual(json.loads(state['rows'][1][27])[0],current)

    def test_legacy_mail_schema_migrates_without_losing_draft_or_adding_actions(self):
        self.configure();draft=outreach.prepare(self.job['id'])
        with tracker.db() as db:
            for key in ('resume_choice','resume_sha256','resume_version'):db.execute('ALTER TABLE outreach_messages DROP COLUMN '+key)
        with patch.object(google,'api') as api:
            outreach.init();outreach.init()
            api.assert_not_called()
        record=json.loads(outreach.portable_mail(self.job['id']))[0]
        self.assertEqual(record['id'],draft['id']);self.assertEqual(record['body'],draft['body'])
        self.assertEqual(record['resume_choice'],'')

    def test_shared_mail_still_rejects_unknown_fields_and_does_not_export_local_columns(self):
        self.configure();outreach.prepare(self.job['id'])
        with tracker.db() as db:db.execute("ALTER TABLE outreach_messages ADD COLUMN local_secret TEXT DEFAULT 'not-portable'")
        record=json.loads(outreach.portable_mail(self.job['id']))[0]
        self.assertNotIn('local_secret',record)
        with self.assertRaisesRegex(ValueError,'Unsupported shared mail fields'):
            outreach.decode_mail(json.dumps([{**record,'unexpected_action':'send'}]))
        with self.assertRaisesRegex(ValueError,'Invalid shared mail values'):
            outreach.decode_mail(json.dumps([{**record,'resume_choice':True}]))
    def test_send_requires_current_review_and_never_retries_ambiguity(self):
        self.configure();row=outreach.prepare(self.job['id'])
        with patch.object(google,'api') as api:
            with self.assertRaisesRegex(ValueError,'Review'):outreach.send_approved(row['id'],'stale')
            api.assert_not_called()
        token=outreach.approval_token(row)
        calls=[]
        def api(service,path,body=None,**kwargs):
            calls.append(path)
            if path=='drafts':return {'id':'draft','message':{'id':'msg','threadId':'thread'}}
            if path=='drafts/send':
                self.assertTrue(kwargs['approved_send']);raise TimeoutError('Uncertain send')
        with patch.object(google,'api',side_effect=api):
            with self.assertRaises(TimeoutError):outreach.send_approved(row['id'],token)
            with self.assertRaises(ValueError):outreach.send_approved(row['id'],token)
        self.assertEqual(calls,['drafts','drafts/send'])
        self.assertEqual(outreach.snapshot()['messages'][0]['state'],'Needs send reconciliation')
    def test_approved_send_uses_provider_receipt_for_followup(self):
        self.configure();row=outreach.prepare(self.job['id'])
        stamp=int(datetime.now(timezone.utc).timestamp()*1000)+1000
        def api(service,path,body=None,**kwargs):
            if path=='drafts':return {'id':'draft','message':{'id':'msg','threadId':'thread'}}
            if path=='drafts/send':return {'id':'sent','threadId':'thread','labelIds':['SENT']}
            if path.startswith('threads/'):
                return {'id':'thread','messages':[{'id':'sent','labelIds':['SENT'],'internalDate':str(stamp),'payload':{'headers':[{'name':'Message-ID','value':'<lorca-'+row['id']+'@lorca.local>'}]}}]}
            self.fail(path)
        with patch.object(google,'api',side_effect=api):outreach.send_approved(row['id'],outreach.approval_token(row))
        self.assertEqual(outreach.snapshot()['messages'][0]['state'],'Sent')
        self.assertTrue(outreach.snapshot()['meta'][0]['follow_up'])
    def test_bootstrap_keeps_stable_identity_and_never_trusts_approval(self):
        import uuid
        identity=str(uuid.uuid4());row=['']*25;row[4]='Ready for review';row[19]='Phone note';row[20]='High';row[22]=identity
        row[23]=json.dumps({'job':{'company':'Other','title':'Role','job_url':'https://other.test/role','application_approved':True,'outreach_approved':True,'outreach_stage':'Sent'},'dossier':{'role':{'summary':'Source-backed summary'}}})
        job_id=sheets.bootstrap_row(row)
        job=tracker.get_job(job_id)
        self.assertEqual(job['notes'],'Phone note');self.assertFalse(job['application_approved']);self.assertEqual(job['outreach_stage'],'Not started')
        with tracker.db() as db:self.assertEqual(db.execute('select sync_id from job_meta where job_id=?',(job_id,)).fetchone()[0],identity)
    def test_conflict_resolution_validates_and_keeps_other_fields(self):
        with tracker.db() as db:db.execute('INSERT INTO sync_conflicts VALUES(?,?,?,?)',(self.job['id'],'notes','local','sheet'))
        sheets.resolve(self.job['id'],'notes','sheet')
        self.assertEqual(tracker.get_job(self.job['id'])['notes'],'sheet');self.assertEqual(outreach.snapshot()['conflicts'],[])

    def fake_sheet(self):
        import copy, re, urllib.parse
        from monitor_data import HEADERS
        state={'rows':[HEADERS.copy()], 'writes':0, 'layout':[]}
        def api(service,path,body=None):
            if '?fields=' in path:return {'sheets':[{'properties':{'sheetId':i,'title':n,'gridProperties':{'rowCount':1000,'columnCount':26}}} for i,n in enumerate(['Opportunities','Status'])]}
            if path.endswith(':batchUpdate') and '/values:' not in path:
                for request in body['requests']:
                    state['layout'].append(request)
                    if 'sortRange' in request:state['rows'][1:]=sorted(state['rows'][1:],key=lambda row:int(row[0]),reverse=True)
                return {}
            if '/values/' in path:
                requested=urllib.parse.unquote(path.split('/values/',1)[1])
                match=re.search(r'!A:([A-Z]+)$',requested)
                end=sum((ord(c)-64)*26**i for i,c in enumerate(reversed(match[1]))) if match else 28
                return {'values':[row[:end] for row in copy.deepcopy(state['rows'])]}
            if path.endswith('/values:batchUpdate'):
                state['writes']+=1
                for change in body['data']:
                    cell=change['range'].split('!')[-1]
                    if not change['range'].startswith("'Opportunities'!"):continue
                    match=re.match(r'([A-Z]+)(\d+)',cell);col=sum((ord(c)-64)*26**i for i,c in enumerate(reversed(match[1])))-1;number=int(match[2])-1
                    while len(state['rows'])<=number:state['rows'].append([])
                    for offset,value in enumerate(change['values'][0]):
                        row=state['rows'][number]
                        while len(row)<=col+offset:row.append('')
                        row[col+offset]=value
                return {}
            return {}
        return state,api
    def test_sheet_round_trip_and_conflict_without_overwrite(self):
        self.configure();state,api=self.fake_sheet()
        with patch.object(google,'api',side_effect=api):
            self.assertEqual(sheets.sync()['conflicts'],0)
            self.assertEqual(len(state['rows']),2)
            state['rows'][1][19]='Edited on phone'
            sheets.sync();self.assertEqual(tracker.get_job(self.job['id'])['notes'],'Edited on phone')
            tracker.update_job(self.job['id'],{'notes':'Local edit'})
            state['rows'][1][19]='Concurrent phone edit'
            self.assertEqual(sheets.sync()['conflicts'],1)
            self.assertEqual(state['rows'][1][19],'Concurrent phone edit')
            self.assertEqual(tracker.get_job(self.job['id'])['notes'],'Local edit')
            sheets.resolve(self.job['id'],'notes','local');sheets.sync()
            self.assertEqual(state['rows'][1][19],'Local edit')
            state['rows'][1][5]='Made-up verification';sheets.sync()
            self.assertEqual(state['rows'][1][5],'Verified')
    def test_legacy_missing_contact_labels_clear_false_conflicts(self):
        self.configure();state,api=self.fake_sheet()
        with patch.object(google,'api',side_effect=api):sheets.sync()
        tracker.update_job(self.job['id'],{'recruiter_contact':''})
        with tracker.db() as db:
            meta=db.execute('SELECT baseline FROM job_meta WHERE job_id=?',(self.job['id'],)).fetchone()
            baseline=json.loads(meta[0]);baseline.pop('recruiter_contact',None)
            db.execute('UPDATE job_meta SET baseline=? WHERE job_id=?',(json.dumps(baseline),self.job['id']))
            db.execute('INSERT INTO sync_conflicts VALUES(?,?,?,?)',(self.job['id'],'recruiter_contact','','Not publicly established'))
        state['rows'][1][12]='Not publicly established'
        with patch.object(google,'api',side_effect=api):result=sheets.sync()
        self.assertEqual(result['conflicts'],0)
        self.assertEqual(outreach.snapshot()['conflicts'],[])
        self.assertEqual(state['rows'][1][12],'')
        self.assertEqual(tracker.get_job(self.job['id'])['recruiter_contact'],'')
    def test_contact_migration_keeps_real_contacts_and_real_conflicts(self):
        self.configure();state,api=self.fake_sheet()
        with patch.object(google,'api',side_effect=api):sheets.sync()
        with tracker.db() as db:
            baseline=json.loads(db.execute('SELECT baseline FROM job_meta WHERE job_id=?',(self.job['id'],)).fetchone()[0]);baseline.pop('recruiter_contact',None)
            db.execute('UPDATE job_meta SET baseline=? WHERE job_id=?',(json.dumps(baseline),self.job['id']))
        state['rows'][1][12]='Not publicly established'
        with patch.object(google,'api',side_effect=api):self.assertEqual(sheets.sync()['conflicts'],0)
        self.assertEqual(state['rows'][1][12],'jane@acme.test')
        tracker.update_job(self.job['id'],{'recruiter_contact':'local@acme.test'})
        state['rows'][1][12]='remote@acme.test'
        with patch.object(google,'api',side_effect=api):self.assertEqual(sheets.sync()['conflicts'],1)
        self.assertEqual(state['rows'][1][12],'remote@acme.test')
    def test_new_jobs_sort_above_existing_rows_and_expand_filter(self):
        self.configure();state,api=self.fake_sheet()
        with patch.object(google,'api',side_effect=api):
            sheets.sync()
            second=tracker.create_job({'company':'New company','title':'New job'})
            sheets.sync()
            self.assertEqual([int(row[0]) for row in state['rows'][1:]],[second['id'],self.job['id']])
            self.assertEqual(state['rows'][1][1],'New company')
            self.assertEqual(state['rows'][2][1],'Acme')
            filters=[r['setBasicFilter']['filter']['range'] for r in state['layout'] if 'setBasicFilter' in r]
            self.assertEqual(filters[-1]['endRowIndex'],3)
            self.assertEqual(filters[-1]['endColumnIndex'],30)

    def test_second_mac_keeps_original_numbers_and_repairs_existing_import(self):
        import uuid
        self.configure();state,api=self.fake_sheet()
        original=str(uuid.uuid4());row=['']*26
        row[0]=49;row[4]='Ready for review';row[20]='High';row[22]=original;row[25]=49
        row[23]=json.dumps({'job':{'company':'Latest','title':'Engineer','job_url':'https://latest.test/job'}})
        identifier=sheets.bootstrap_row(row)
        self.assertNotEqual(identifier,49)
        # Reproduce a previously imported M3 record with the wrong local number.
        with tracker.db() as db:db.execute('UPDATE job_meta SET discovery_number=? WHERE job_id=?',(identifier,identifier))
        state['rows'].append(row)
        with patch.object(google,'api',side_effect=api):
            sheets.sync()
            self.assertEqual(state['rows'][1][0],49)
            self.assertEqual(state['rows'][1][1],'Latest')
            self.assertEqual(next(j['number'] for j in outreach.numbered_jobs() if j['id']==identifier),49)
            third=tracker.create_job({'company':'Even newer','title':'Engineer'})
            sheets.sync()
            self.assertEqual(state['rows'][1][0],50)
            self.assertEqual(next(j['number'] for j in outreach.numbered_jobs() if j['id']==third['id']),50)

    def test_backup_column_is_read_when_visible_number_was_overwritten(self):
        import uuid
        self.configure();state,api=self.fake_sheet()
        row=['']*26;row[0]=2;row[4]='Ready for review';row[20]='High'
        row[22]=str(uuid.uuid4());row[25]=49
        row[23]=json.dumps({'job':{'company':'Latest','title':'Engineer','job_url':'https://latest.test/job'}})
        identifier=sheets.bootstrap_row(row)
        with tracker.db() as db:db.execute('UPDATE job_meta SET discovery_number=2 WHERE job_id=?',(identifier,))
        state['rows'].append(row)
        with patch.object(google,'api',side_effect=api):sheets.sync()
        self.assertEqual(next(j['number'] for j in outreach.numbered_jobs() if j['id']==identifier),49)
        self.assertEqual(state['rows'][1][0],49)
        self.assertEqual(state['rows'][1][25],49)

    def test_49_reversed_imports_recover_original_numbers_and_stay_newest_first(self):
        import uuid
        self.configure();state,api=self.fake_sheet()
        with tracker.db() as db:existing=db.execute('SELECT sync_id FROM job_meta WHERE job_id=?',(self.job['id'],)).fetchone()[0]
        for index,number in enumerate(range(49,0,-1)):
            row=['']*26;row[0]=index+1;row[25]=index+1
            row[4]='Ready for review';row[19]='Phone note '+str(number);row[20]='High'
            row[22]=existing if index==0 else str(uuid.uuid4())
            job={k:self.job[k] for k in ('company','title','job_url')} if index==0 else {'company':'Company '+str(number),'title':'Engineer','job_url':'https://company.test/job/'+str(number)}
            row[23]=json.dumps({'job':job,'discovery_number':number,'created_at':'2026-10-01T12:00:00+00:00','dossier':{'researched_at':'2026-10-01T12:00:00+00:00'}})
            sheets.bootstrap_row(row);state['rows'].append(row)
        with tracker.db() as db:db.execute('UPDATE job_meta SET discovery_number=job_id')
        with patch.object(google,'api',side_effect=api):
            sheets.sync();sheets.sync()
            jobs=outreach.numbered_jobs()
            self.assertEqual([j['number'] for j in jobs],list(range(49,0,-1)))
            self.assertEqual([j['id'] for j in jobs],list(range(1,50)))
            self.assertEqual([r[0] for r in state['rows'][1:]],list(range(49,0,-1)))
            self.assertEqual([r[25] for r in state['rows'][1:]],list(range(49,0,-1)))
            self.assertEqual(len({r[22] for r in state['rows'][1:]}),49)
            self.assertEqual(tracker.get_job(49)['notes'],'Phone note 1')
            self.assertEqual(tracker.get_job(49)['created_at'],'2026-10-01T12:00:00+00:00')
            self.assertEqual(json.loads(state['rows'][1][23])['discovery_number'],49)
            latest=tracker.create_job({'company':'Newest','title':'Engineer'})
            sheets.sync()
            self.assertEqual(outreach.numbered_jobs()[0]['number'],50)
            self.assertEqual(outreach.numbered_jobs()[0]['id'],latest['id'])
            self.assertEqual(state['rows'][1][1],'Newest')

    def test_sheet_changes_during_read_are_not_overwritten(self):
        self.configure();state,api=self.fake_sheet();reads=0
        def racing(service,path,body=None):
            nonlocal reads
            result=api(service,path,body)
            if '/values/' in path:
                reads+=1
                if reads==2:result['values'].append(['new phone row'])
            return result
        with patch.object(google,'api',side_effect=racing):
            with self.assertRaisesRegex(ValueError,'Sheet changed'):sheets.sync()
        self.assertEqual(state['writes'],0)
    def test_bootstrap_is_idempotent_and_reuses_imported_job(self):
        import uuid
        self.configure();row=['']*25;row[4]='Ready for review';row[20]='High';row[22]=str(uuid.uuid4())
        row[23]=json.dumps({'job':{k:self.job[k] for k in ('company','title','job_url')}})
        self.assertEqual(sheets.bootstrap_row(row),self.job['id'])
        self.assertEqual(sheets.bootstrap_row(row),self.job['id'])
        self.assertEqual(len(tracker.list_jobs()),1)
        row[22]=str(uuid.uuid4());row[21]='invalid-date'
        with self.assertRaises(ValueError):sheets.bootstrap_row(row)
        self.assertEqual(len(tracker.list_jobs()),1)
    def test_gmail_methods_cannot_delete_or_send(self):
        for method,path in [('DELETE','drafts/abc'),('POST','messages'),('PUT','threads/abc')]:
            with self.assertRaises(ValueError):google.api('gmail',path,method=method)

if __name__=='__main__':unittest.main()
