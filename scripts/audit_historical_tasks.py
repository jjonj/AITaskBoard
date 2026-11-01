import os,json,yaml,re,pathlib,collections
from datetime import datetime,timezone
root=pathlib.Path(os.environ['APPDATA'])/'TaskBoard'/'projects.json'
projects=json.loads(root.read_text(encoding='utf-8'))['projects'];report={'generated':datetime.now(timezone.utc).isoformat(),'boards':[]}
for project in projects:
 p=pathlib.Path(project['path']); tasks=[]
 for f in (p/'Active').glob('*.md'):
  try:
   raw=f.read_text(encoding='utf-8-sig'); parts=raw.split('---',2)
   meta=yaml.safe_load(parts[1]) if len(parts)>2 else None
   if isinstance(meta,dict): tasks.append((f,meta))
  except Exception as e: continue
 byid={t.get('id'):t for _,t in tasks};flags=[]
 for f,t in tasks:
  id=t.get('id');col=t.get('column');att=t.get('attention') or [];deps=t.get('depends_on') or []
  waiting=[d for d in deps if d not in byid or not(byid[d].get('completed') or byid[d].get('column')=='done')]
  if col=='blocked' and waiting: flags.append({'id':id,'type':'blocked_with_waiting_dependencies','waiting':waiting,'reason':t.get('blocked_reason'),'action':'inspect_not_automatic'})
  if col=='blocked' and not t.get('blocked_reason') and 'blocked' not in att: flags.append({'id':id,'type':'blocked_without_reason','action':'inspect'})
  if col=='in-progress' and not t.get('claimed_by'):flags.append({'id':id,'type':'unclaimed_in_progress','action':'inspect'})
  if col not in ('in-progress','done') and not t.get('completed') and t.get('claimed_by'): flags.append({'id':id,'type':'claim_outside_in_progress','action':'inspect'})
  if t.get('blocked_reason') and col!='blocked' and 'blocked' not in att:flags.append({'id':id,'type':'stale_reason','action':'inspect'})
 report['boards'].append({'name':project['name'],'path':str(p),'tasks':len(tasks),'flags':flags})
out=pathlib.Path(__file__).resolve().parent.parent/'artifacts'/'misc0101-audit-dry-run.json';out.parent.mkdir(exist_ok=True);out.write_text(json.dumps(report,indent=2),encoding='utf-8');print('Report',out)
for b in report['boards']:
 print(b['name'],'tasks',b['tasks'],'flags',len(b['flags']), 'types',dict(collections.Counter(x['type'] for x in b['flags'])))