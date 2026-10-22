from pathlib import Path
import datetime,zipfile,re,hashlib,os,tempfile,json
root=Path(r'B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks')
out=Path(r'D:\SSDProjects\Tools\TaskBoard\artifacts\migrations')
out.mkdir(parents=True,exist_ok=True)
stamp=datetime.datetime.now().strftime('%Y%m%dT%H%M%S')
epics={'WT-0075','WT-0052','WT-0020','WT-0163','WT-0132','WT-0145','WT-0086','WT-0096','WT-0102'}
# Only link explicitly confirmed associations. No speculative hard prerequisites.
soft_links={'WT-0150':['WT-0147'],'WT-0186':['WT-0070','WT-0177'],'WT-0087':['WT-0179']}
files=sorted([f for f in root.rglob('*.md') if re.fullmatch(r'(?:WT|WTE)-\d+\.md',f.name)])
state={}
for f in files:
    contents=f.read_bytes()
    if not re.search(rb'(?m)^---\r?$',contents) or b'\n---' not in contents:
        raise ValueError(f'Invalid frontmatter: {f}')
    matched=re.search(rb'(?m)^id:\s*((?:WT|WTE)-\d+)\s*\r?$',contents)
    if not matched: raise ValueError(f'Missing id: {f}')
    id=matched.group(1).decode('ascii')
    if id+'.md' != f.name: raise ValueError(f'ID and filename differ: {f}')
    state[f]=(contents,id)
existing={id for data,id in state.values()}
for id in epics:
    if id not in existing: raise ValueError(f'Epic classification references absent id {id}')
backup=out/f'{stamp}_before_epic_migration.zip'
with zipfile.ZipFile(backup,'w',compression=zipfile.ZIP_DEFLATED) as z:
    for path,(content,_) in state.items():
        z.writestr(str(path.relative_to(root)).replace('\\','/'),content)
edits=[]
for f,(contents,id) in state.items():
    if b'\r\n' in contents: newline=b'\r\n'
    else: newline=b'\n'
    end=contents.find(newline+b'---',4)
    if end<0: raise ValueError(f'Malformed frontmatter: {f}')
    front=contents[:end+len(newline)]
    tail=contents[end+len(newline):]
    kind=b'epic' if id in epics else b'task'
    def field(front,key,value):
        pattern=rb'(?m)^'+key+rb':.*(?:\r?\n|$)'
        line=key+b': '+value+newline
        if re.search(pattern,front): return re.sub(pattern,lambda m:line,front,count=1)
        if key==b'kind':
            return re.sub(rb'(?m)^id:\s*(?:WT|WTE)-\d+\r?\n',lambda m:m.group(0)+line,front,count=1)
        return front+line
    front=field(front,b'kind',kind)
    desired_deps=[]
    desired_related=[v for v in soft_links.get(id,[]) if v in existing and v!=id]
    front=field(front,b'depends_on',b'[]')
    rel=b'['+b', '.join(i.encode('ascii') for i in desired_related)+b']'
    # Existing nonempty metadata must never be overwritten by migration.
    if re.search(rb'(?m)^related_to:\s*\[.*\]',front):
        current=re.search(rb'(?m)^related_to:\s*\[(.*)\]',front).group(1).strip()
        if current: rel=b'['+current+(b', ' if desired_related else b'')+b', '.join(i.encode('ascii') for i in desired_related)+b']'
    front=field(front,b'related_to',rel)
    if id in epics:
        front=re.sub(rb'(?m)^claimed_by:.*(?:\r?\n|$)',b'',front)
    modified=front+tail
    if modified!=contents: edits.append((f,contents,modified,id))
print('Files:',len(files),'Epic IDs:',sorted(epics),'To update:',len(edits),'Backup:',backup)
migrated=[]
try:
    for f,old,new,id in edits:
        if f.read_bytes()!=old:
            print('Skipped concurrent edit',id)
            continue
        fd,temp=tempfile.mkstemp(prefix='.taskboard-epic-',suffix='.tmp',dir=f.parent)
        try:
            with os.fdopen(fd,'wb') as stream:
                stream.write(new)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temp,f)
        finally:
            if os.path.exists(temp):os.remove(temp)
        migrated.append(id)
except Exception as exc:
    # The complete original content of every modified file is in the backup.
    raise RuntimeError(f'Interrupted after {len(migrated)} files: {exc}')
report=out/f'{stamp}_report.json'
report.write_text(json.dumps({'backup':str(backup),'total':len(files),'migrated':migrated,'epics':sorted(epics),'related_seeded':soft_links},indent=2),encoding='utf-8')
print('Migrated:',len(migrated),'epics:',len(epics),'report:',report)
