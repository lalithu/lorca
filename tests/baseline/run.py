#!/usr/bin/env python3
"""Record Checkpoint A evidence. A successful recorder exit is NOT a healthy-agent verdict."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path, help='New private artifact directory, outside the repository')
    parser.add_argument('--model', required=True, help='Exact installed Ollama model name')
    parser.add_argument('--live', action='store_true', help='Run serial live Core and opt-in Qwen tests')
    parser.add_argument('--native', action='store_true', help='Run actual isolated native acceptance; opens test windows')
    args = parser.parse_args()
    out = args.output.expanduser().resolve()
    if out == ROOT or ROOT in out.parents:
        parser.error('Keep runtime evidence outside the repository; do not publish endpoint credentials or private logs.')
    out.mkdir(parents=True, exist_ok=False, mode=0o700)
    results = []

    def write(name, value):
        (out / name).write_text(json.dumps(value, indent=2))

    def capture(command):
        try:
            p = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=20)
            return {'exit':p.returncode, 'stdout':p.stdout.strip(), 'stderr':p.stderr.strip()}
        except (OSError, subprocess.TimeoutExpired) as e:
            return {'unavailable':str(e)}

    def digest(path):
        return hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None

    def run(name, command, cwd=ROOT, timeout=300, extra_env=None):
        start=time.monotonic()
        env=dict(os.environ, PYTHONDONTWRITEBYTECODE='1', **(extra_env or {}))
        with (out / (name+'.log')).open('w') as log:
            try:
                p=subprocess.Popen(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                try: code=p.wait(timeout=timeout)
                except subprocess.TimeoutExpired:
                    import signal
                    os.killpg(p.pid, signal.SIGTERM)
                    try:p.wait(timeout=10)
                    except subprocess.TimeoutExpired:os.killpg(p.pid, signal.SIGKILL);p.wait()
                    code='timeout'
            except OSError as e:log.write(str(e));code='unavailable'
        entry={'name':name,'command':command,'cwd':str(cwd),'exit':code,'seconds':round(time.monotonic()-start,2)}
        results.append(entry);write('checks.json',results);print(name,code,flush=True)
        return code

    def ollama(path, data=None):
        try:
            req=urllib.request.Request('http://127.0.0.1:11434/api/'+path, data=json.dumps(data).encode() if data is not None else None, headers={'Content-Type':'application/json'})
            with urllib.request.urlopen(req, timeout=20) as r:return json.load(r)
        except Exception as e:return {'unavailable':str(e)}

    source_files = [p for d in ['src','macos/Sources','jobapps'] for p in (ROOT/d).rglob('*') if p.is_file() and p.suffix in ['.rs','.swift','.py'] and '__pycache__' not in str(p)]
    before={str(p.relative_to(ROOT)):digest(p) for p in source_files}
    write('production-sources-before.json',before)
    show=ollama('show',{'model':args.model})
    # Model metadata is retained, not account credentials or the local bearer token.
    metadata={'recordedAt':time.strftime('%Y-%m-%dT%H:%M:%S%z'), 'commit':capture(['git','rev-parse','HEAD']), 'workingTree':capture(['git','status','--short']), 'diffSha256':hashlib.sha256(capture(['git','diff']) .get('stdout','').encode()).hexdigest(), 'hardware':capture(['sysctl','hw.model','hw.memsize','hw.ncpu']), 'macOS':capture(['sw_vers']), 'architecture':platform.machine(), 'rust':capture(['rustc','--version']), 'swift':capture(['xcrun','swift','--version']), 'tools':{x:shutil.which(x) for x in ['cargo','node','python3','sandbox-exec']}, 'ollamaVersion':ollama('version'), 'model':args.model, 'modelMetadata':{k:show[k] for k in ['details','model_info','parameters','capabilities','thinking'] if k in show}, 'modelUnavailable':show.get('unavailable'), 'installedModelDigests':ollama('tags'), 'loadedModelsBefore':ollama('ps'), 'executionSettings':{'effort':'nominal','liveModelSeconds':60,'liveCommandSeconds':30,'liveIterations':10,'liveSerial':True}, 'optionalConfigurationPresent':{'LORCA_WEB_SEARCH_URL':bool(os.environ.get('LORCA_WEB_SEARCH_URL'))}, 'runningEndpoint':{}}
    endpoint=Path.home()/'Library/Application Support/lorca/runtime/endpoint.json'
    if endpoint.exists():
        try:metadata['runningEndpoint']={k:v for k,v in json.loads(endpoint.read_text()).items() if k in ['protocol','pid','version','buildId']}
        except (OSError,ValueError):metadata['runningEndpoint']={'unreadable':True}
    metadata['fixtureSourceHashes']={str(p.relative_to(ROOT)):digest(p) for d in ['tests/baseline','macos/Acceptance'] for p in (ROOT/d).rglob('*') if p.is_file() and '__pycache__' not in str(p)}
    metadata['fixtureSourceHashes']['tests/reliability_baseline.rs']=digest(ROOT/'tests/reliability_baseline.rs')
    write('environment.json',metadata)
    cargo=shutil.which('cargo') or str(Path.home()/'.cargo/bin/cargo')
    run('rust-tests',[cargo,'test'],timeout=600)
    run('clippy',[cargo,'clippy','--all-targets','--','-D','warnings'],timeout=600)
    run('format',[cargo,'fmt','--check'])
    run('jobapps',[sys.executable,'-B','-m','unittest','discover','-s','jobapps/tests','-v'])
    run('restart',[sys.executable,'-B','-m','unittest','discover','-s','macos/Tests/Restart','-v'])
    run('rust-release',[cargo,'build','--release'],timeout=600)
    sdk=Path('/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk')
    sdk_env={'SDKROOT':str(sdk)} if sdk.exists() else {}
    run('swift-tests',['xcrun','swift','test'],ROOT/'macos',extra_env=sdk_env)
    run('swift-release',['xcrun','swift','build','-c','release'],ROOT/'macos',extra_env=sdk_env)
    run('mechanics',[cargo,'test','--test','reliability_baseline','--','--ignored','--nocapture'],extra_env={'LORCA_BASELINE_OUTPUT':str(out/'mechanics.json')})
    if args.live:
        if show.get('unavailable'):
            results.append({'name':'live','exit':'unavailable','reason':show['unavailable']});write('checks.json',results)
        else:
            run('qwen-live',[cargo,'test','--test','agent_live','--','--ignored','--nocapture','--test-threads=1'],timeout=600,extra_env={'LORCA_LIVE_MODEL':args.model})
            run('thinking',[cargo,'test','--test','agent_thinking_live','--','--ignored','--nocapture','--test-threads=1'],timeout=600,extra_env={'LORCA_LIVE_MODEL':args.model})
            import live
            live.run(out/'live',args.model)
    if args.native:
        if show.get('unavailable'):
            results.append({'name':'native','exit':'unavailable','reason':show['unavailable']});write('checks.json',results)
        else:
            run('native',[sys.executable,'-B','macos/Acceptance/run.py','--skip-build','--model',args.model,'--effort','nominal'],timeout=1800)
    after={str(p.relative_to(ROOT)):digest(p) for p in source_files}
    write('production-sources-after.json',after)
    write('summary.json',{'kind':'baseline recording, not acceptance certification','productionSourceUnchanged':before==after,'checks':results,'builds':{str(p.relative_to(ROOT)):digest(p) for p in [ROOT/'target/release/lorca',ROOT/'macos/.build/release/lorca-macos']},'loadedModelsAfter':ollama('ps'),'notRun':['full long-context native acceptance','live Google/Gmail/Sheets','production app replacement','live interrupted-generation stress test'],'liveClassification':'See inventory and independent evidence; Completed does not imply success.'})
    print('Evidence:',out,flush=True)
    if before!=after:raise SystemExit('Production source changed during baseline; investigate before comparison.')

if __name__=='__main__':main()
