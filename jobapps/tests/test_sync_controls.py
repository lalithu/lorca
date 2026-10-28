"""Sync fault isolation, automatic triggers and serialized wakeups; no Google access."""
from pathlib import Path
import sys
import threading
import unittest
from unittest.mock import Mock, patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import google_connections as google
import integrations
import outreach
import research
import service
import shared_settings
import sheets
import shared_resume

class SyncControlTests(unittest.TestCase):
    def setUp(self):
        self.resume_sync=patch.object(shared_resume,'sync')
        self.resume_sync.start();self.addCleanup(self.resume_sync.stop)
    def test_bad_settings_do_not_block_gmail_or_jobs(self):
        with patch.object(integrations,'_state',{'last_sync':'previous'}),patch.object(shared_settings,'sync',side_effect=ValueError('Bad header')),patch.object(outreach,'reconcile') as mail,patch.object(sheets,'sync',return_value={'jobs':49,'conflicts':0}) as jobs:
            integrations.sync()
            mail.assert_called_once();jobs.assert_called_once()
            self.assertIn('Synced 49 jobs',integrations._state['message'])
            self.assertIn('Settings need attention: Bad header',integrations._state['message'])
            self.assertNotEqual(integrations._state['last_sync'],'previous')

    def test_jobs_failure_keeps_last_success_and_reports_other_errors(self):
        with patch.object(integrations,'_state',{'last_sync':'previous'}),patch.object(shared_settings,'sync',side_effect=ValueError('Bad settings')),patch.object(outreach,'reconcile',side_effect=ValueError('Gmail offline')),patch.object(sheets,'sync',side_effect=ValueError('Sheet offline')):
            integrations.sync()
            self.assertEqual(integrations._state['last_sync'],'previous')
            for label in ['Settings need attention','Gmail needs attention','Jobs need attention']:self.assertIn(label,integrations._state['message'])

    def test_automatic_sync_requires_connection_and_enablement(self):
        wake=threading.Event()
        with patch.object(integrations,'_wake',wake),patch.object(google,'TOKEN') as token,patch.object(google,'config',return_value={'enabled':False}):
            token.exists.return_value=True;integrations.request_sync();self.assertFalse(wake.is_set())
        with patch.object(integrations,'_wake',wake),patch.object(google,'TOKEN') as token,patch.object(google,'config',return_value={'enabled':True}):
            token.exists.return_value=False;integrations.request_sync();self.assertFalse(wake.is_set())
            token.exists.return_value=True;integrations.request_sync();self.assertTrue(wake.is_set())

    def test_requests_wait_for_inflight_google_work_and_coalesce(self):
        stop=threading.Event();wake=threading.Event();busy=threading.Lock();finished=threading.Event();worker=None
        with patch.object(integrations,'_stop',stop),patch.object(integrations,'_wake',wake),patch.object(integrations,'_busy',busy),patch.object(integrations,'_state',{}),patch.object(google,'TOKEN') as token,patch.object(google,'config',return_value={'enabled':True}),patch.object(integrations,'auto_drafts'),patch.object(integrations,'sync',side_effect=finished.set) as sync:
            token.exists.return_value=True
            busy.acquire();owned=True
            try:
                for _ in range(5):integrations.request_sync()
                worker=integrations.start_background()
                self.assertFalse(finished.wait(.05))
                busy.release();owned=False
                self.assertTrue(finished.wait(2),'Queued sync was dropped')
            finally:
                if owned:busy.release()
                integrations.shutdown(flush=False)
                if worker:worker.join(2)
            self.assertFalse(worker.is_alive());self.assertEqual(sync.call_count,1)

    def test_successful_settings_saves_and_open_request_existing_sync(self):
        with patch.object(service,'snapshot',return_value={}),patch.object(research,'status',return_value={'run':None}),patch.object(integrations,'request_sync') as sync,patch.object(integrations,'save') as templates,patch.object(shared_settings,'save_profile') as profile:
            service.dispatch({'op':'google_settings'})
            service.dispatch({'op':'settings','profile':'A reviewed factual profile for testing','preferences':'Useful search preferences for testing','version':'v'})
            service.dispatch({'op':'google_auto_sync'})
            self.assertEqual(sync.call_count,3);templates.assert_called_once();profile.assert_called_once()
            sync.reset_mock();profile.side_effect=ValueError('Stale editor')
            with self.assertRaises(ValueError):service.dispatch({'op':'settings','profile':'A reviewed factual profile for testing'})
            sync.assert_not_called()

if __name__=='__main__':unittest.main()
