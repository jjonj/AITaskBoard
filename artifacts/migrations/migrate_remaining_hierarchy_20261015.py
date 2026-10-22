from pathlib import Path
import re,os,tempfile,datetime,json,zipfile
root=Path(r'B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks')
out=Path(r'D:\SSDProjects\Tools\TaskBoard\artifacts\migrations')
out.mkdir(exist_ok=True,parents=True)
# Legacy IDs deliberately remain stable.
new_epics={'WT-0009','WT-0059','WT-0072','WT-0074','WT-0162','WT-0187'}
# Accidental cross-project hierarchy and historical design artifacts are links,
# not "whole implementation is an epic" relationships.
detach={'WT-0016':'WT-0006','WT-0015':'WT-0016','WT-0041':'WT-0009','WT-0179':'WT-0087'}
files={f.stem:f for f in root.rglob('WT-*.md')}
assert all(id in files for id in new_epics|set(detach))
stamp=datetime.datetime.now().strftime('%Y%m%dT%H%M%S')
changes=[]
for id in sorted(new_epics|set(detach)):
    file=files[id]; original=file.read_bytes()
    newline=b'\r\n' if b'\r\n' in original else b'\n'
    head=original.split(newline+b'---',1)
    assert len(head)==2,id
    front=head[0]
    if id in new_epics:
        assert re.search(rb'(?m)^kind: task\s*$',front),id
        if re.search(rb'(?m)^claimed_by:\s*\S+',front):
            raise RuntimeError(f'{id} is actively claimed; must be released before converting')
        updated=re.sub(rb'(?m)^kind: task\s*$',b'kind: epic',original,count=1)
    else:
        assert re.search(rb'(?m)^parent: '+detach[id].encode()+rb'\s*$',front), id
        updated=re.sub(rb'(?m)^parent:\s*WT-\d+\s*\r?\n',b'',original,count=1)
    if id=='WT-0179':
        updated=updated.replace(b'related_to: []',b'related_to: [WT-0087]',1)
    changes.append((file,original,updated,id))
backup=out/f'{stamp}_hierarchy_before.zip'
with zipfile.ZipFile(backup,'w',zipfile.ZIP_DEFLATED) as z:
    for path,old,_,_ in changes:z.writestr(str(path.relative_to(root)).replace('\\','/'),old)
migrated=[];skipped=[]
for path,old,new,id in changes:
    if path.read_bytes()!=old:
        skipped.append(id);continue
    fd,temp=tempfile.mkstemp(prefix='.taskboard-hierarchy-',suffix='.tmp',dir=path.parent)
    try:
        with os.fdopen(fd,'wb') as stream:
            stream.write(new); stream.flush();os.fsync(stream.fileno())
        os.replace(temp,path)
    finally:
        if os.path.exists(temp):os.remove(temp)
    migrated.append(id)
manifest={'backup':str(backup),'converted_to_epics':sorted(new_epics),'removed_accidental_or_historical_parents':detach,'applied':migrated,'skipped_concurrent_writes':skipped,'deferred_actively_claimed_parents':['WT-0043','WT-0051']}
(out/f'{stamp}_hierarchy_report.json').write_text(json.dumps(manifest,indent=2),encoding='utf-8')
print(json.dumps(manifest,indent=2))
