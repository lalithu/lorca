"""No Google traffic: interrupted TLS reads recover; writes remain single-shot."""
import io
from pathlib import Path
import ssl
import sys
import unittest
import urllib.error
import urllib.request
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import google_http
import google_connections as google
import integrations


class GoogleHTTPTests(unittest.TestCase):
    def test_read_recovers_once_from_ssl_eof(self):
        lost=urllib.error.URLError(ssl.SSLEOFError(8,'unexpected EOF'))
        with patch.object(google_http.urllib.request,'urlopen',side_effect=[lost,io.BytesIO(b'{"ok":true}')]) as request,patch.object(google_http.time,'sleep') as delay:
            self.assertEqual(google_http.request_json(urllib.request.Request('https://sheets.googleapis.com/test')),{'ok':True})
            self.assertEqual(request.call_count,2);delay.assert_called_once_with(.4)

    def test_repeated_loss_is_bounded_and_friendly(self):
        with patch.object(google_http.urllib.request,'urlopen',side_effect=TimeoutError('private transport detail')) as request,patch.object(google_http.time,'sleep'):
            with self.assertRaisesRegex(google_http.GoogleNetworkError,'work is kept locally') as failure:
                google_http.request_json(urllib.request.Request('https://sheets.googleapis.com/test'))
            self.assertEqual(request.call_count,2)
            self.assertNotIn('private transport detail',str(failure.exception))

    def test_gmail_creation_update_and_send_are_never_replayed(self):
        for method in ('POST','PUT'):
            with self.subTest(method=method),patch.object(google_http.urllib.request,'urlopen',side_effect=ssl.SSLEOFError('EOF')) as request,patch.object(google_http.time,'sleep') as delay:
                with self.assertRaisesRegex(google_http.GoogleNetworkError,'may have reached Google'):
                    google_http.request_json(urllib.request.Request('https://gmail.googleapis.com/test',data=b'{}',method=method))
                self.assertEqual(request.call_count,1);delay.assert_not_called()

    def test_certificate_validation_failure_is_not_retried_or_bypassed(self):
        lost=urllib.error.URLError(ssl.SSLCertVerificationError(1,'certificate invalid'))
        with patch.object(google_http.urllib.request,'urlopen',side_effect=lost) as request,patch.object(google_http.time,'sleep') as delay:
            with self.assertRaisesRegex(google_http.GoogleNetworkError,'could not be verified'):
                google_http.request_json(urllib.request.Request('https://sheets.googleapis.com/test'))
            self.assertEqual(request.call_count,1);delay.assert_not_called()

    def test_gmail_404_stays_available_for_sent_detection(self):
        lost=urllib.error.HTTPError('https://gmail.googleapis.com/test',404,'Missing',{},io.BytesIO(b'{"error":{"message":"Draft missing"}}'))
        with patch.object(google.oauth,'access_token',return_value='test-only'),patch.object(google_http.urllib.request,'urlopen',side_effect=lost) as request:
            with self.assertRaises(RuntimeError) as failure:google.api('gmail','drafts/example')
            self.assertEqual(failure.exception.status,404);self.assertEqual(request.call_count,1)

    def test_gateway_allows_only_draft_updates_and_still_blocks_send(self):
        with patch.object(google.oauth,'access_token',return_value='test-only'),patch.object(google,'request_json',return_value={}) as request:
            google.api('gmail','drafts/example',{'message':{'raw':'test'}},method='PUT')
            self.assertEqual(request.call_args[0][0].get_method(),'PUT')
            for path,method in [('drafts/send','PUT'),('drafts/send','POST'),('messages/example','PUT')]:
                with self.subTest(path=path,method=method),self.assertRaises(ValueError):google.api('gmail',path,{},method=method)
            self.assertEqual(request.call_count,1)

    def test_connection_loss_stops_sync_without_false_success(self):
        with patch.object(integrations,'_state',{'last_sync':'previous'}),patch.object(integrations.shared_resume,'sync',side_effect=google_http.GoogleNetworkError('Connection interrupted')),patch.object(integrations.shared_settings,'sync') as settings,patch.object(integrations.outreach,'reconcile') as mail,patch.object(integrations.sheets,'sync') as jobs:
            with self.assertRaises(google_http.GoogleNetworkError):integrations.sync()
            self.assertEqual(integrations._state['last_sync'],'previous')
            settings.assert_not_called();mail.assert_not_called();jobs.assert_not_called()
