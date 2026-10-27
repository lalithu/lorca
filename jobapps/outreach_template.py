"""Fill saved outreach wording from saved facts, without loading a model."""
import re


EXPERIENCE='ADD ONE SENTENCE ABOUT MY PAST EXP AND MENTION HOW THAT ROLE ALIGNS WITH THE ROLE YOU ARE REACHING OUT FOR'
MARKER=re.compile(r'\[([^\[\]\r\n]+)\](?!\()')


def has_markers(text):
    return bool(re.search(r'\[[A-Z][^\]\n]*\](?!\()|_{3,}',text))


def section(profile,title):
    found=re.search(r'^## '+re.escape(title)+r'\s*\n(.*?)(?=^## |\Z)',profile,re.M|re.S)
    return found[1] if found else ''


def skills(profile,job,data):
    listed=[]
    for line in section(profile,'Technical Skills').splitlines():
        match=re.match(r'\s*-\s*\*\*[^*]+:\*\*\s*(.+)',line)
        if match:listed.extend(item.strip().rstrip('.') for item in match[1].split(',') if item.strip())
    if not listed:raise ValueError('Add a Technical Skills section with your verified skills to Candidate profile before preparing outreach.')
    posting=' '.join([job['title'],job.get('description',''),data.get('role',{}).get('tech_stack','')])
    matches=[skill for skill in listed if re.search(r'(?<!\w)'+re.escape(skill)+r'(?!\w)',posting,re.I)]
    chosen=list(dict.fromkeys(matches+listed))[:2]
    return ' and '.join(chosen)


def experience(profile,job):
    match=re.search(r'^- (.+)',section(profile,'Professional Experience'),re.M)
    if not match:raise ValueError('Add a factual bullet under Professional Experience in Candidate profile before preparing outreach.')
    fact=re.sub(r'\*\*|__','',match[1]).strip().rstrip('.')
    if not fact.startswith('I '):fact='I '+fact[0].lower()+fact[1:]
    return fact+', experience I would bring to the '+job['title']+' role at '+job['company']+'.'


def render(template,job,who,data,profile,kind):
    # Unconfirmed PDF-only additions must never become outreach claims.
    profile=profile.split('## PDF cross-check:')[0]
    fields={'first_name':who.get('name','').split(' ')[0],'contact_name':who.get('name',''),
            'company':job['company'],'role':job['title'],'job_url':job['job_url'],
            'job_specific':data.get('role',{}).get('fit',''),'your_name':'Lalith Uriti'}
    unknown=set(re.findall(r'\{\{\s*(\w+)\s*\}\}',template))-set(fields)
    if unknown:raise ValueError('Unknown template placeholders: '+', '.join(sorted(unknown)))
    text=re.sub(r'\{\{\s*(\w+)\s*\}\}',lambda m:fields[m[1]],template)
    if '{{' in text or '}}' in text:raise ValueError('Unknown template placeholder. Check the fields in Job Apps Email settings.')
    aliases={'NAME':fields['your_name'],'FIRST NAME':fields['first_name'],'CONTACT NAME':fields['contact_name'],
             'COMPANY':fields['company'],"COMPANY'S":fields['company']+'’s','ROLE':fields['role'],
             'TYPE OF ROLL':fields['role'],'TYPE OF ROLE':fields['role'],'ATTACH RESUME':''}
    def replace(match):
        key=' '.join(match[1].upper().replace('’',"'").split())
        if key=='LIST A COUPLE SKILLS':return skills(profile,job,data)
        if key==EXPERIENCE:return experience(profile,job)
        if key not in aliases:raise ValueError('Unknown template placeholder: ['+match[1][:140]+']. Edit it in Job Apps Email settings.')
        return aliases[key]
    text=MARKER.sub(replace,text)
    text=re.sub(r'^(Hi|Hello|Dear)\s+_{2,}',lambda m:m[1]+' '+(fields['first_name'] or 'there'),text,flags=re.M)
    first,_,body=text.partition('\n')
    automatic=('Follow-up: ' if kind=='followup' else '')+job['title']+' @ '+job['company']
    if first.lower().startswith('subject:'):subject=first[8:].strip() or automatic
    else:subject=automatic;body=text
    subject=' '.join(subject.split());body=body.strip()
    if not subject or not body or len(subject)>240 or len(body)>20000:raise ValueError('Template subject/body is empty or too long')
    return subject,body
