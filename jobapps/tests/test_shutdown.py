"""Quit drains confirmed writes and keeps local data when the network stalls."""
from pathlib import Path
import io
import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.request
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import google_http
import google_connections as google
import integrations

class ShutdownTests(unittest.TestCase):
    def test_quit_finishes_current_write_before_final_sync_and_acknowledgement(self):
        entered=threading.Event();release=threading.Event();settled=threading.Event();done=threading.Event()
        stop=threading.Event();wake=threading.Event();lock=threading.Lock();order=[]
        def operation():
            entered.set();release.wait(2);order.append('confirmed write saved');settled.set()
        def final_sync():
            self.assertTrue(settled.is_set());order.append('final sync')
        with patch.object(integrations,'_stop',stop),patch.object(integrations,'_wake',wake),patch.object(integrations,'_busy',lock),patch.object(integrations,'_state',{}),patch.object(google,'config',return_value={'enabled':True}),patch.object(google,'TOKEN') as token,patch.object(integrations,'sync',side_effect=final_sync):
            token.exists.return_value=True
            integrations.run(operation);self.assertTrue(entered.wait(1))
            quit_thread=threading.Thread(target=lambda:(integrations.shutdown(timeout=2),done.set()))
            quit_thread.start();self.assertFalse(done.wait(.03))
            release.set();self.assertTrue(done.wait(2));quit_thread.join()
            self.assertEqual(order,['confirmed write saved','final sync'])
            with patch.object(google_http.urllib.request,'urlopen') as request:
                with self.assertRaisesRegex(google_http.GoogleNetworkError,'closing'):
                    google_http.request_json(urllib.request.Request('https://example.test'))
                request.assert_not_called()

    def test_stuck_worker_has_deadline_and_does_not_start_final_sync(self):
        lock=threading.Lock();lock.acquire()
        with patch.object(integrations,'_stop',threading.Event()),patch.object(integrations,'_wake',threading.Event()),patch.object(integrations,'_busy',lock),patch.object(integrations,'sync') as sync:
            started=time.monotonic()
            self.assertFalse(integrations.shutdown(timeout=.03))
            self.assertLess(time.monotonic()-started,.5);sync.assert_not_called()
        lock.release()

    def test_shutdown_network_requests_share_budget_and_never_retry_a_write(self):
        future=time.monotonic()+.25
        with patch.object(google_http,'shutdown_deadline',future),patch.object(google_http,'stopping',return_value=False),patch.object(google_http.urllib.request,'urlopen',return_value=io.BytesIO(b'{"ok":true}')) as network:
            self.assertTrue(google_http.request_json(urllib.request.Request('https://example.test',data=b'{}'))['ok'])
            self.assertGreater(network.call_args.kwargs['timeout'],0)
            self.assertLessEqual(network.call_args.kwargs['timeout'],.25)
        with patch.object(google_http,'shutdown_deadline',time.monotonic()-1),patch.object(google_http.urllib.request,'urlopen') as network:
            with self.assertRaisesRegex(google_http.GoogleNetworkError,'closing'):
                google_http.request_json(urllib.request.Request('https://example.test'))
            network.assert_not_called()

    def test_offline_final_sync_preserves_local_records_and_returns(self):
        with patch.object(integrations,'_stop',threading.Event()),patch.object(integrations,'_wake',threading.Event()),patch.object(integrations,'_busy',threading.Lock()),patch.object(integrations,'_state',{}),patch.object(google,'config',return_value={'enabled':True}),patch.object(google,'TOKEN') as token,patch.object(integrations,'sync',side_effect=google_http.GoogleNetworkError('Offline')):
            token.exists.return_value=True
            self.assertTrue(integrations.shutdown(timeout=.1))
            self.assertIn('Saved locally; sync pending',integrations._state['message'])

    def test_service_sigterm_closes_without_leaving_a_companion(self):
        with tempfile.TemporaryDirectory() as tmp:
            env=dict(os.environ,LORCA_JOBS_ROOT=tmp,JOB_PIPELINE_DB=str(Path(tmp)/'pipeline.sqlite3'))
            child=subprocess.Popen([sys.executable,'-B',str(Path(__file__).parents[1]/'service.py')],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
            try:
                child.stdin.write('{"op":"snapshot"}\n');child.stdin.flush()
                self.assertTrue(json.loads(child.stdout.readline())['ok'])
                child.send_signal(signal.SIGTERM);self.assertEqual(child.wait(timeout=3),0)
                self.assertTrue((Path(tmp)/'pipeline.sqlite3').is_file())
            finally:
                if child.poll() is None:child.kill();child.wait()
                child.stdin.close();child.stdout.close()

if __name__=='__main__':unittest.main()
