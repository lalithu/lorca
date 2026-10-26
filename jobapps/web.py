"""Exa free transport adapted from Hermes (MIT, vendor/HERMES-LICENSE). Stdlib only."""
import ipaddress
import json
import re
import sys
import urllib.request
import urllib.parse

ENDPOINT = 'https://mcp.exa.ai/mcp'

def public_url(url):
    p = urllib.parse.urlsplit(url)
    if p.scheme != 'https' or not p.hostname or p.username or p.password:
        raise ValueError('Expected a public HTTPS source URL')
    host=p.hostname.lower()
    if host=='localhost' or host.endswith(('.local','.localhost','.internal')): raise ValueError('Local sources are not allowed')
    try: address=ipaddress.ip_address(host)
    except ValueError: address=None
    if address is not None and not address.is_global:raise ValueError('Private sources are not allowed')
    return url

def mcp(tool, args):
    payload={'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':tool,'arguments':args}}
    request=urllib.request.Request(ENDPOINT,json.dumps(payload).encode(),{'Content-Type':'application/json','Accept':'application/json, text/event-stream','User-Agent':'lorca-job-apps'})
    with urllib.request.urlopen(request,timeout=35) as response: body=response.read(2_000_001).decode('utf-8')
    if len(body)>2_000_000:raise ValueError('Web response exceeded limit')
    candidates=[body] if body.lstrip().startswith('{') else [s[6:] for s in re.split(r'\r\n|\r|\n',body) if s.startswith('data: ')]
    for candidate in candidates:
        try:data=json.loads(candidate)
        except ValueError:continue
        if data.get('error'):raise ValueError(str(data['error'].get('message','Search provider error')))
        result=data.get('result',{})
        texts=[c.get('text','') for c in result.get('content',[]) if isinstance(c,dict)]
        if result.get('isError'):raise ValueError(' '.join(texts)[:500])
        if texts:return '\n'.join(texts)
    raise ValueError('Search provider returned no text')

def execute(name,args):
    if name=='web_search':
        query=args.get('query','')
        if not isinstance(query,str) or not 1<=len(query)<=1500:raise ValueError('Invalid search query')
        limit=max(1,min(5,int(args.get('limit',5))))
        text=mcp('web_search_exa',{'query':query,'numResults':limit})
        results=[]
        for block in text.split('\n---\n'):
            title=url='';summary=[];highlight=False
            for line in map(str.strip,block.splitlines()):
                if line.startswith('Title:'):title=line[6:].strip()
                elif line.startswith('URL:'):url=line[4:].strip()
                elif highlight:summary.append(line)
                if line.startswith(('Title:','URL:','Highlights:','Published:','Author:')):highlight=line.startswith('Highlights:')
            if url:results.append({'url':url,'title':title,'description':' '.join(summary)})
        return {'ok':True,'results':results[:limit],'source':'Exa free'}
    if name!='web_extract':raise ValueError('Research only allows web_search and web_extract')
    urls=args.get('urls',[])
    if isinstance(urls,str):urls=[urls]
    if not isinstance(urls,list) or len(urls)!=1:raise ValueError('Extract exactly one URL per call')
    url=public_url(urls[0]);limit=max(1000,min(12000,int(args.get('char_limit',3500))))
    text=mcp('web_fetch_exa',{'urls':[url]})
    return {'ok':True,'results':[{'url':url,'content':text[:limit],'truncated':len(text)>limit}]}

if __name__=='__main__':
    try:
        data=json.load(sys.stdin);result=execute(data['name'],data['arguments'])
    except Exception as exc:result={'ok':False,'error':str(exc)[:500]}
    print(json.dumps(result))
