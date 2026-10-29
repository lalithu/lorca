"""Serial disposable live Core diagnostics. Known failures are retained, not fixed."""
import os,pathlib,tempfile,subprocess,time,json,urllib.request,uuid,hashlib,re,html.parser,sys
ROOT=pathlib.Path(__file__).resolve().parents[2];OUT=None;MODEL=None;RESULTS=[]
HTML='''<!doctype html><html><head><meta charset="utf-8"><title>Gradient playground</title></head><body style="background:linear-gradient(0deg,#667eea,#764ba2)"><main><h1>Gradient playground</h1><label>Gradient <input id="gradient" type="range" min="0" max="100" value="0"></label><button id="dark">Dark mode</button></main><script>
const slider=document.getElementById('gradient');
slider.addEventListener('input',e=>{document.body.style.background=`linear-gradient(${e.target.value}deg,#667eea,#764ba2)`;});
document.getElementById('dark').addEventListener('click',()=>document.body.classList.toggle('dark'));
</script></body></html>
const duplicate = 1;
'''
class HTMLScan(html.parser.HTMLParser):
 def __init__(self):super().__init__();self.scripts=[];self.inscript=False;self.visible=[];self.starts={};self.ends={};self.attrs={}
 def handle_starttag(self,t,a):
  self.starts[t]=self.starts.get(t,0)+1
  if t=='script':self.inscript=True;self.scripts.append('')
  d=dict(a)
  if d.get('id'):self.attrs[d['id']]=d
 def handle_endtag(self,t):
  self.ends[t]=self.ends.get(t,0)+1
  if t=='script':self.inscript=False
 def handle_data(self,s):
  if self.inscript:self.scripts[-1]+=s
  else:self.visible.append(s)
def post(url,data,headers=None,timeout=30):
 req=urllib.request.Request(url,data=json.dumps(data).encode(),headers={'Content-Type':'application/json',**(headers or {})})
 try:
  with urllib.request.urlopen(req,timeout=timeout) as r:return json.load(r)
 except urllib.error.HTTPError as e:raise RuntimeError(e.read().decode())
class Backend:
 def __init__(self,name,files):
  self.root=OUT/name;self.root.mkdir();self.w=self.root/'project';self.w.mkdir();self.log=(self.root/'backend.log').open('w')
  for n,s in files.items():p=self.w/n;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(s)
  self.proc=subprocess.Popen([str(ROOT/'target/release/lorca'),'serve'],env=dict(os.environ,LORCA_DATA_DIR=str(self.root/'data')),stdout=self.log,stderr=self.log)
  p=self.root/'data/runtime/endpoint.json'
  for _ in range(100):
   if p.exists():break
   time.sleep(.1)
  ep=json.loads(p.read_text());self.url=ep['url']+'/command';self.headers={'Authorization':'Bearer '+ep['token']}
  r=self.cmd({'op':'connect','host':'http://127.0.0.1:11434','model':MODEL});self.session=r['session'];self.chat=next(s for s in self.cmd({'op':'snapshot'})['sessions'] if s['id']==self.session)['selectedId'];self.results=[]
 def cmd(self,v):return post(self.url,{'session':getattr(self,'session',None),**v},self.headers)
 def task(self,prompt,allow,effort='nominal',limit=240):
  start=time.monotonic();req=str(uuid.uuid4());r=self.cmd({'op':'agent_start','model':MODEL,'task':prompt,'workspace':str(self.w),'conversationId':self.chat,'requestId':req,'effort':effort,'limits':{'iterations':10,'modelSeconds':60,'commandSeconds':30}});tid=r['taskId'];decisions=[];seen=set();v={}
  while time.monotonic()-start<limit:
   v=self.cmd({'op':'agent_snapshot'})
   approval=v.get('approval')
   if approval and approval['id'] not in seen:
    seen.add(approval['id']);events=v.get('events',[]);requested=next((e['data'] for e in events if e['kind']=='tool_requested' and e['data'].get('callId')==approval['callId']),{});args=requested.get('arguments',{});ok=allow(approval['tool'],args);decision='allow_once' if ok else 'deny';decisions.append({'tool':approval['tool'],'arguments':args,'decision':decision});self.cmd({'op':'agent_decide','taskId':tid,'approvalId':approval['id'],'decision':decision})
   if v.get('status') not in ['running','awaiting_approval']:break
   time.sleep(.2)
  else:
   self.cmd({'op':'agent_cancel','taskId':tid});time.sleep(.5);v=self.cmd({'op':'agent_snapshot'});v['auditTimedOut']=True
  v['auditPrompt']=prompt;v['auditSeconds']=round(time.monotonic()-start,2);v['auditApprovals']=decisions
  self.results.append(v);(self.root/'audit-runs.json').write_text(json.dumps(self.results,indent=2));print('RUN',self.root.name,v.get('status'),v['auditSeconds'],flush=True);return v
 def close(self):
  try:self.cmd({'op':'close','unload':False})
  except Exception:pass
  self.proc.terminate()
  try:self.proc.wait(timeout=12)
  except subprocess.TimeoutExpired:self.proc.kill();self.proc.wait()
  self.log.close()

def save(name,b,checks):
 d={'name':name,'model':MODEL,'root':str(b.root),'runs':b.results,'checks':checks,'classification':'unclassified; inspect independent evidence'};RESULTS.append(d);(OUT/'core-live-matrix.json').write_text(json.dumps(RESULTS,indent=2))
def file_approval(tool,a,paths):return tool=='apply_patch' and isinstance(a.get('path'),str) and pathlib.PurePosixPath(a['path']).as_posix() in paths

def clarify(topic_shift=False):
 name='topic-shift' if topic_shift else 'clarification-resume';b=Backend(name,{})
 try:
  first=b.task('Create a file.',lambda t,a:False,limit=100)
  checks={'firstStatus':first.get('status'),'noFilesBeforeAnswer':not list(b.w.iterdir())}
  if first.get('status')=='waiting_for_input':
   prompt='Forget the file request. Just tell me what 2 + 2 is. Do not edit files or run commands.' if topic_shift else 'Create an empty CHOSEN.md, with no content. Do nothing else.'
   second=b.task(prompt,lambda t,a:False if topic_shift else file_approval(t,a,['CHOSEN.md']),limit=160)
   checks.update({'secondStatus':second.get('status'),'effectiveObjective':second.get('task'),'files':sorted(p.name for p in b.w.iterdir()),'chosenEmpty':(b.w/'CHOSEN.md').exists() and (b.w/'CHOSEN.md').stat().st_size==0})
   if topic_shift:checks['outcomeSatisfied']=second.get('status')=='completed' and not list(b.w.iterdir()) and '4' in (second.get('result') or '')
   else:checks['outcomeSatisfied']=checks['chosenEmpty'] and second.get('status')=='completed'
  save(name,b,checks)
 finally:b.close()

def frontend(name):
 b=Backend(name,{'index.html':HTML})
 try:
  v=b.task('Fix index.html: moving the Gradient slider must change the background colors across the full rainbow, not just rotate two fixed colors. Remove the stray visible JavaScript text. Keep the existing page layout and dark-mode button. Do not add dependencies or change any other files. Ask me if a meaningful decision is needed.',lambda t,a:file_approval(t,a,['index.html']),limit=240)
  s=(b.w/'index.html').read_text();parser=HTMLScan();parser.feed(s);checks={'status':v.get('status'),'bytes':len(s.encode()),'htmlOpen':parser.starts.get('html',0),'htmlClose':parser.ends.get('html',0),'scriptOpen':parser.starts.get('script',0),'scriptClose':parser.ends.get('script',0),'strayVisibleJS':bool(re.search(r'const\s+duplicate|document\.(body|getElementById)|=>',' '.join(parser.visible))),'syntax':[]}
  for i,script in enumerate(parser.scripts):
   p=b.root/f'script-{i}.js';p.write_text(script);r=subprocess.run([__import__('shutil').which('node') or 'node','--check',str(p)],capture_output=True,text=True);checks['syntax'].append({'exit':r.returncode,'error':r.stderr[:1600]})
  (b.root/'html-scan.json').write_text(json.dumps({'scripts':parser.scripts,'attrs':parser.attrs},indent=2))
  oracle=subprocess.run([__import__('shutil').which('node') or 'node',str(ROOT/'tests/baseline/check_frontend.js'),str(b.w/'index.html')],capture_output=True,text=True,timeout=10)
  (b.root/'behavior.json').write_text(oracle.stdout if oracle.returncode==0 else json.dumps({'unavailable':oracle.stderr}))
  checks['structureAndSyntaxPass']=checks['htmlOpen']==checks['htmlClose']==1 and checks['scriptOpen']==checks['scriptClose'] and not checks['strayVisibleJS'] and all(x['exit']==0 for x in checks['syntax'])
  save(name,b,checks)
 finally:b.close()

def rust_fix():
 b=Backend('rust-fix',{'Cargo.toml':'[package]\nname="audit_add"\nversion="0.1.0"\nedition="2021"\n','src/lib.rs':'pub fn add(a:i32,b:i32)->i32 { a-b }\n#[cfg(test)] mod tests { #[test] fn sum() { assert_eq!(super::add(2,3),5); } }\n'})
 try:
  def allow(t,a):return file_approval(t,a,['src/lib.rs']) or (t in ['exec_command','run_command'] and a.get('executable')=='cargo' and a.get('directory','.')=='.' and a.get('arguments') in [['test'],['test','--offline'],['test','--quiet']])
  v=b.task('Fix the subtraction bug in src/lib.rs so add(2,3) returns 5. Run cargo test after editing and report the actual exit code. Do not edit other source files.',allow,limit=240)
  r=subprocess.run([str(pathlib.Path.home()/'.cargo/bin/cargo'),'test','--offline'],cwd=b.w,capture_output=True,text=True,timeout=60)
  (b.root/'independent-cargo-test.log').write_text(r.stdout+r.stderr);save('rust-fix',b,{'status':v.get('status'),'independentExit':r.returncode,'source':(b.w/'src/lib.rs').read_text(),'outcomeSatisfied':r.returncode==0 and v.get('status')=='completed'})
 finally:b.close()


def question_flow(name,shift):
 b=Backend(name,{})
 try:
  first=b.task('I need to create a file. Before editing, use request_user_input to ask me which filename and contents to use. Wait for my answer.',lambda t,a:False,limit=120)
  checks={'firstStatus':first.get('status'),'firstResult':first.get('result'),'noFilesBeforeAnswer':not list(b.w.iterdir())}
  if first.get('status')=='waiting_for_input':
   prompt='Cancel the previous file-creation task. This is a new request: tell me what 2 + 2 is. Do not edit files or run commands.' if shift else 'Use filename CHOSEN.md with empty content. Create only that file.'
   second=b.task(prompt,lambda t,a:False if shift else file_approval(t,a,['CHOSEN.md']),limit=150)
   checks.update({'secondStatus':second.get('status'),'secondResult':second.get('result'),'effectiveObjective':second.get('task'),'files':sorted(p.name for p in b.w.iterdir())})
   checks['outcomeSatisfied']=(second.get('status')=='completed' and not list(b.w.iterdir()) and '4' in (second.get('result') or '')) if shift else ((b.w/'CHOSEN.md').exists() and (b.w/'CHOSEN.md').stat().st_size==0 and second.get('status')=='completed')
  save(name,b,checks)
 finally:b.close()

def rust_positive():
 b=Backend('rust-positive',{'Cargo.toml':'[package]\nname="audit_positive"\nversion="0.1.0"\nedition="2021"\n','src/lib.rs':'pub fn add(a:i32,b:i32)->i32 { a-b }\n#[cfg(test)] mod tests { #[test] fn sum() { assert_eq!(super::add(2,3),5); } }\n'})
 try:
  def allow(t,a):return file_approval(t,a,['src/lib.rs']) or (t in ['exec_command','run_command'] and a.get('executable')=='cargo' and a.get('directory','.')=='.' and a.get('arguments') in [['test'],['test','--offline'],['test','--quiet']])
  r=b.task('Fix the subtraction bug in src/lib.rs so add(2,3) returns 5. Run cargo test after applying the fix and report the actual exit code. Keep changes limited to src/lib.rs.',allow,limit=180)
  run=subprocess.run([str(pathlib.Path.home()/'.cargo/bin/cargo'),'test','--offline'],cwd=b.w,capture_output=True,text=True,timeout=60);(b.root/'independent-cargo-test.log').write_text(run.stdout+run.stderr);save('rust-positive',b,{'status':r.get('status'),'independentExit':run.returncode,'outcomeSatisfied':r.get('status')=='completed' and run.returncode==0,'source':(b.w/'src/lib.rs').read_text()})
 finally:b.close()

def specific_frontend():
 b=Backend('frontend-specific',{'index.html':HTML})
 try:
  prompt='Edit index.html only. Make the slider a hue control: near its minimum the background should be red, one-third through green, and two-thirds through blue. Change the hue/color values as the slider moves, not just the gradient angle. Remove the stray const duplicate text outside the document. Keep the page controls and the dark-mode button. Use small targeted changes.'
  r=b.task(prompt,lambda t,a:file_approval(t,a,['index.html']),limit=180);s=(b.w/'index.html').read_text();parser=HTMLScan();parser.feed(s)
  (b.root/'html-scan.json').write_text(json.dumps({'scripts':parser.scripts,'attrs':parser.attrs},indent=2))
  oracle=subprocess.run([__import__('shutil').which('node') or 'node',str(ROOT/'tests/baseline/check_frontend.js'),str(b.w/'index.html')],capture_output=True,text=True,timeout=10)
  (b.root/'behavior.json').write_text(oracle.stdout if oracle.returncode==0 else json.dumps({'unavailable':oracle.stderr}));syntax=[]
  for i,script in enumerate(parser.scripts):
   p=b.root/f'script-{i}.js';p.write_text(script);run=subprocess.run([__import__('shutil').which('node') or 'node','--check',str(p)],capture_output=True,text=True);syntax.append({'exit':run.returncode,'error':run.stderr[:1000]})
  oracle=subprocess.run([__import__('shutil').which('node') or 'node',str(ROOT/'tests/baseline/check_frontend.js'),str(b.w/'index.html')],capture_output=True,text=True,timeout=10)
  (b.root/'behavior.json').write_text(oracle.stdout if oracle.returncode==0 else json.dumps({'unavailable':oracle.stderr}))
  save('frontend-specific',b,{'status':r.get('status'),'syntax':syntax,'strayVisibleJS':bool(re.search(r'const\s+duplicate|document\.(body|getElementById)|=>',' '.join(parser.visible))),'htmlOpen':parser.starts.get('html',0),'htmlClose':parser.ends.get('html',0)})
 finally:b.close()

def run(output, model):
 global OUT,MODEL,RESULTS
 OUT=pathlib.Path(output);OUT.mkdir(exist_ok=True);MODEL=model;RESULTS=[]
 cases=[('clarification-resume',lambda:clarify(False)),('frontend-a',lambda:frontend('frontend-a')),('rust-fix',rust_fix),('native-question-resume',lambda:question_flow('native-question-resume',False)),('native-question-topic-shift',lambda:question_flow('native-question-topic-shift',True)),('rust-positive',rust_positive),('frontend-specific',specific_frontend)]
 for name,fn in cases:
  try:fn()
  except Exception as e:
   RESULTS.append({'name':name,'harnessError':repr(e),'classification':'environment or diagnostic error; inspect logs'})
   (OUT/'core-live-matrix.json').write_text(json.dumps(RESULTS,indent=2));print('ERROR',name,repr(e),flush=True)
 return RESULTS
