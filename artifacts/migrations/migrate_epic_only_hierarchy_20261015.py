from pathlib import Path
import re,zipfile,datetime,hashlib,tempfile,os,json
root=Path(r'B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks')
out=Path(r'D:\SSDProjects\Tools\TaskBoard\artifacts\migrations')
out.mkdir(parents=True,exist_ok=True)
stamp=datetime.datetime.now().strftime('%Y%m%dT%H%M%S')
paths=sorted([x for x in root.rglob('*.md') if re.fullmatch(r'(WT|WTE)-\d+\.md',x.name)])
def frontmatter(raw):
    decoded=raw.decode('utf-8-sig')
    newline='\r\n' if '\r\n' in decoded else '\n'
    norm=decoded.replace('\r\n','\n')
    match=re.match(r'^---\n(.*?)\n---\n',norm,re.S)
    if not match:raise RuntimeError('Task does not start with expected YAML frontmatter')
    return match.group(1),norm[match.end():],newline
items={}
for path in paths:
    raw=path.read_bytes()
    front,body,nl=frontmatter(raw)
    meta=dict(re.findall(r'(?m)^(id|kind|parent|icon|claimed_by):\s*(.*)$',front))
    id=meta.get('id')
    if not id or id!=path.stem:raise RuntimeError(f'Invalid ID: {path}')
    items[id]={'path':path,'raw':raw,'front':front,'body':body,'newline':nl,**meta}
def linked(front,original_parent):
    m=re.search(r'(?m)^related_to: *\[(.*?)\] *$',front)
    if m:
        old=[x.strip().strip("'\"") for x in m.group(1).split(',') if x.strip()]
        if original_parent not in old:old.append(original_parent)
        return front[:m.start()]+f"related_to: [{', '.join(old)}]"+front[m.end():]
    # serde_yaml renders nonempty lists as indented-by-zero YAML blocks.
    block=re.search(r'(?m)^related_to:\s*\n((?:- [^\n]*\n?)*)',front)
    if block:
        existing=re.findall(r'(?m)^- (\S[^\n]*)',block.group(1))
        if original_parent in existing:return front
        replacement='related_to:\n'+''.join('- '+item+'\n' for item in [*existing,original_parent])
        return front[:block.start()]+replacement+front[block.end():]
    # No related_to field yet; add one conservatively.
    return front+'\nrelated_to: ['+original_parent+']'
def stripfield(front,key):
    # Safety: fail if a complex YAML block exists; there should be none for
    # these historical epics, but never silently corrupt human-authored YAML.
    lines=front.split('\n')
    found=[i for i,line in enumerate(lines) if re.match(r'^'+re.escape(key)+r':(?: |$)',line)]
    if not found:return front
    if len(found)>1:raise RuntimeError('Duplicate metadata field '+key)
    i=found[0]
    if i+1<len(lines) and (lines[i+1].startswith('  ') or lines[i+1].startswith('- ')):
        raise RuntimeError('Nested YAML field '+key+' requires manual conversion')
    lines.pop(i)
    return '\n'.join(lines)
def extract_section(body,title):
    pat=re.compile(r'(?m)^## '+re.escape(title)+r'\n(.*?)(?=^## |\Z)',re.S|re.M)
    m=pat.search(body)
    if not m:return body,''
    return body[:m.start()]+body[m.end():],m.group(1).strip()
changes=[]
for id,item in items.items():
    old_parent=item.get('parent') or None
    kind=item.get('kind','task')
    front=item['front'];body=item['body'];changes_for=[]
    if kind=='epic':
        if old_parent:
            front=stripfield(front,'parent')
            front=linked(front,old_parent)
            changes_for.append('epic_deparented')
        for k in ('icon','attention','question'):
            if re.search(r'(?m)^'+k+r':',front):
                front=stripfield(front,k)
                changes_for.append(f'cleared_epic_{k}')
        if re.search(r'(?m)^depends_on:\s*\[\s*[^]\s]',front):
            raise RuntimeError(f'Epic {id} unexpectedly depends on tasks')
        body,acceptance=extract_section(body,'Acceptance Criteria')
        body,ai_notes=extract_section(body,'AI Notes')
        if acceptance or ai_notes:
            additions=[]
            if acceptance:additions.append('### Prior acceptance criteria (historical planning reference)\n\n'+acceptance)
            if ai_notes:additions.append('### Prior AI notes (historical coordination reference)\n\n'+ai_notes)
            if re.search(r'(?m)^## Notes\s*$',body):
                m=re.search(r'(?m)^## Notes\s*$',body)
                # This board's epics have no pre-existing Notes section; fail
                # rather than append under an unrelated following section.
                raise RuntimeError(f'Unexpected existing Notes section in {id}; migrate manually')
            body=body.rstrip('\n')+'\n\n## Notes\n\n'+'\n\n'.join(additions)+'\n'
            changes_for.append('preserved_acceptance_ai_notes_in_notes')
        elif '## Acceptance Criteria' in item['body'] or '## AI Notes' in item['body']:
            changes_for.append('removed_empty_epic_sections')
    elif old_parent:
        parent=items.get(old_parent)
        if not parent or parent.get('kind')!='epic':
            ancestor_id=old_parent
            visited=set()
            nearest=None
            while ancestor_id and ancestor_id not in visited:
                visited.add(ancestor_id)
                ancestor=items.get(ancestor_id)
                if not ancestor:break
                if ancestor.get('kind')=='epic':
                    nearest=ancestor_id;break
                ancestor_id=ancestor.get('parent') or None
            front=stripfield(front,'parent')
            if nearest:
                front += '\nparent: '+nearest
                changes_for.append(f'reparented_to_epic:{nearest}')
            else:
                changes_for.append('detached_no_epic_ancestor')
            front=linked(front,old_parent)
            changes_for.append('original_parent_preserved_as_related')
    if not changes_for:continue
    new='---\n'+front+'\n---\n'+body
    nl=item['newline']
    if nl=='\r\n':new=new.replace('\n','\r\n')
    changed=new.encode('utf-8')
    if changed!=item['raw']:
        changes.append((item['path'],item['raw'],changed,id,changes_for))
zip_path=out/f'{stamp}_epic_only_hierarchy_before.zip'
with zipfile.ZipFile(zip_path,'w',zipfile.ZIP_DEFLATED) as z:
    for f,old,_,_,_ in changes:z.writestr(str(f.relative_to(root)).replace('\\','/'),old)
print('Inspected',len(items),'task files, modifications:',len(changes),'backup:',zip_path,flush=True)
applied=[];skipped=[]
for f,old,new,id,reasons in changes:
    if f.read_bytes()!=old:
        skipped.append({'id':id,'reasons':reasons});continue
    fd,tmp=tempfile.mkstemp(prefix='.tb-epic-rule-',suffix='.tmp',dir=f.parent)
    try:
        with os.fdopen(fd,'wb') as handle:
            handle.write(new);handle.flush();os.fsync(handle.fileno())
        if f.read_bytes()!=old:
            skipped.append({'id':id,'reasons':reasons});continue
        os.replace(tmp,f)
        applied.append({'id':id,'reasons':reasons})
    finally:
        if os.path.exists(tmp):os.unlink(tmp)
report=out/f'{stamp}_epic_only_hierarchy_report.json'
report.write_text(json.dumps({'backup':str(zip_path),'total':len(items),'changed':applied,'skipped_concurrent':skipped},indent=2),encoding='utf-8')
print('Migrated:',len(applied),'skipped concurrent:',len(skipped),'report:',report)
for m in applied:print(m['id'],':',', '.join(m['reasons']))
