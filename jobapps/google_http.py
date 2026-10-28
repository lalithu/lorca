"""Small, bounded recovery for Google's secure HTTP connections."""
import http.client
import json
import ssl
import time
import urllib.error
import urllib.request


stopping = lambda: False
shutdown_deadline = None


class GoogleNetworkError(RuntimeError):
    pass


def request_json(request,timeout=25):
    reading=request.get_method()=='GET'
    for attempt in range(2 if reading else 1):
        remaining = shutdown_deadline - time.monotonic() if shutdown_deadline is not None else timeout
        if stopping() or remaining <= 0:raise GoogleNetworkError("Lorca is closing. Unsynced changes are saved locally for the next sync.")
        try:
            with urllib.request.urlopen(request,timeout=min(timeout,remaining)) as response:return json.load(response)
        except urllib.error.HTTPError:
            raise  # Keep provider status codes, especially Gmail draft 404s.
        except (urllib.error.URLError,ssl.SSLError,TimeoutError,ConnectionError,http.client.HTTPException) as exc:
            reason=getattr(exc,'reason',exc)
            if isinstance(reason,ssl.SSLCertVerificationError):
                raise GoogleNetworkError('Google’s secure connection could not be verified. Check this Mac’s date and network.') from None
            if reading and attempt==0:
                time.sleep(.4)
                continue
            message='Google connection interrupted. Your work is kept locally; sync will retry automatically.'
            if not reading:message='Google connection interrupted. The request may have reached Google; check its status before retrying.'
            raise GoogleNetworkError(message) from None
