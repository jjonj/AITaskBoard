import http from 'node:http';
import fs from 'node:fs/promises';
import {createReadStream} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {createHash} from 'node:crypto';
const exec=promisify(execFile), here=path.dirname(fileURLToPath(import.meta.url));
const vault=path.resolve(process.env.OMNI_TASKBOARD_VAULT_ROOT || 'B:/GDrive/SharedWithPhone/Obsidian/jjj');
const cli=path.join(here,'..','TaskBoard.exe'), dist=path.join(here,'dist');
const catalog=path.join(process.env.APPDATA,'TaskBoard','projects.json');
const mediaCache=path.join(process.env.LOCALAPPDATA,'TaskBoard','review-media');
const hubConfig=process.env.OMNIHUB_AUTH_CONFIG || path.join(process.env.LOCALAPPDATA,'OmniHub','config','appsettings.json');
const port=Number(process.env.TASKBOARD_WEB_PORT || 8795);
const allowedMethods=new Set(['update','create','archive','restore','review.resolve','queue.list','queue.set','session.open','inbox.list']);
const mime={'.html':'text/html; charset=utf-8','.js':'text/javascript; charset=utf-8','.css':'text/css; charset=utf-8','.png':'image/png','.jpg':'image/jpeg','.jpeg':'image/jpeg','.webp':'image/webp','.gif':'image/gif','.mp4':'video/mp4','.webm':'video/webm','.ico':'image/x-icon'};
function fail(message,status=400){throw Object.assign(new Error(message),{status});}
function inside(base,target){const rel=path.relative(base,target);return !!rel && rel!=='..' && !rel.startsWith('..'+path.sep) && !path.isAbsolute(rel);}
async function projects(){
 const config=JSON.parse((await fs.readFile(catalog,'utf8')).replace(/^\uFEFF/,''));
 const base=await fs.realpath(vault), result=[];
 for(const p of config.projects || []){
  try{
   const root=await fs.realpath(p.path);
   if(!inside(base,root))continue;
   const board=path.relative(base,root).split(path.sep).join('/');
   if(!/^[a-zA-Z0-9_. /-]+$/.test(board))continue;
   await fs.access(path.join(root,'_board.md'));
   result.push({id:p.id,name:p.name,board,root,priority:p.priority});
  }catch{}
 }
 return result;
}
async function boardFor(board){const p=(await projects()).find(p=>p.board===board);if(!p)fail('Board is not in the active desktop catalog',404);return p;}
async function command(p,...args){
 try{const {stdout}=await exec(cli,['--root',p.root,'--json',...args],{windowsHide:true,timeout:45000,maxBuffer:64*1024*1024,encoding:'utf8'});return JSON.parse(stdout);}
 catch(e){fail('TaskBoard could not read this board: '+String(e.stderr||e.message).slice(0,500),503);}
}
async function snapshot(p,archived=false){
 const boardPath=path.join(p.root,'_board.md');
 const before=await fs.readFile(boardPath);
 const [columns,tasks,queue]=await Promise.all([command(p,'column','list'),command(p,'task','list',...(archived?['--include-archived']:[])),command(p,'task','queue','list').catch(()=>({items:[],unavailable:true}))]);
 const after=await fs.readFile(boardPath);
 if(!before.equals(after))fail('Board configuration changed; refresh',409);
 return {config:{name:p.name,columns:columns.columns,revision:createHash('sha256').update(after).digest('hex')},tasks:tasks.tasks,warnings:tasks.advisories||[],queue:queue.items,queueUnavailable:!!queue.unavailable};
}
async function hub(method,params){
 // Read the existing local service credential at request time. It is never returned to the browser.
 const key=process.env.OMNIHUB_AUTH_API_KEY || JSON.parse((await fs.readFile(hubConfig,'utf8')).match(/"AuthApiKey"\s*:\s*("(?:\\.|[^"\\])*")/)?.[1]||'null');
 if(!key)fail('Local OmniHub authentication is not configured',503);
 const response=await fetch('http://127.0.0.1:5000/api/agents/call',{method:'POST',headers:{'content-type':'application/json','x-omni-api-key':key},body:JSON.stringify({method:'taskboard.phone.'+method,params}),signal:AbortSignal.timeout(45000)});
 const result=await response.json();
 if(!response.ok)fail(result.message||'OmniHub rejected the request',response.status);
 return result;
}
async function body(req){
 if(!String(req.headers['content-type']).startsWith('application/json'))fail('JSON required',415);
 if(req.headers['sec-fetch-site']==='cross-site')fail('Cross-site request rejected',403);
 let chunks=[],length=0;
 for await(const c of req){length+=c.length;if(length>1_100_000)fail('Request too large',413);chunks.push(c);}
 try{return JSON.parse(Buffer.concat(chunks).toString('utf8'));}catch{fail('Invalid JSON');}
}
function json(res,data,status=200){res.writeHead(status,{'content-type':'application/json; charset=utf-8','cache-control':'no-store','x-content-type-options':'nosniff'});res.end(JSON.stringify(data));}
async function stream(req,res,file,base){
 const [real,root]=await Promise.all([fs.realpath(file),fs.realpath(base)]);
 if(!inside(root,real))fail('Invalid asset',403);
 const ext=path.extname(real).toLowerCase(), type=mime[ext];if(!type)fail('Unsupported file type',415);
 const stat=await fs.stat(real);if(!stat.isFile())fail('Not found',404);
 const headers={'content-type':type,'x-content-type-options':'nosniff','cache-control':'private, no-store','accept-ranges':'bytes'};
 const range=/^bytes=(\d+)-(\d*)$/.exec(req.headers.range||'');
 let start=0,end=stat.size-1;
 if(range){start=Number(range[1]);end=range[2]?Math.min(Number(range[2]),end):end;if(start>end||start>=stat.size){res.writeHead(416,{'content-range':'bytes */'+stat.size});return res.end();}headers['content-range']='bytes '+start+'-'+end+'/'+stat.size;}
 headers['content-length']=Math.max(0,end-start+1);
 res.writeHead(range?206:200,headers);if(req.method==='HEAD'||!stat.size)return res.end();
 createReadStream(real,{start,end}).on('error',()=>res.destroy()).pipe(res);
}
async function reviewMedia(req,res,p,url){
 const id=url.searchParams.get('taskId'), artifact=url.searchParams.get('artifact');
 if(!/^[A-Za-z][A-Za-z0-9-]{1,127}$/.test(id||''))fail('Invalid task ID');
 const manifest=await command(p,'task','review','show',id);
 const m=manifest.review||manifest.manifest||manifest;
 const r=m.rounds?.find(r=>r.number===Number(url.searchParams.get('round')));
 const a=[...(r?.artifacts||[]),...(r?.references||[]),...(r?.previous?[r.previous]:[])].find(a=>a.id===artifact);
 if(!a||a.pruned)fail('Review asset unavailable',404);
 const file=url.searchParams.get('poster')==='1'?a.poster:a.path;
 if(!file||a.storage==='url'&&!a.poster)fail('Asset is an external link',404);
 // Only inert image/video data is served from a review manifest. Never run arbitrary review HTML on the authenticated origin.
 if(!/\.(png|jpe?g|webp|gif|mp4|webm)$/i.test(file))fail('This review media type is desktop-only',415);
 for(const base of [path.join(p.root,'Reviews'),mediaCache]){
  try{const [rr,rb]=await Promise.all([fs.realpath(file),fs.realpath(base)]);if(inside(rb,rr))return stream(req,res,rr,rb);}catch{}
 }
 fail('Review asset is unavailable',404);
}
const server=http.createServer(async(req,res)=>{
 try{
  if(!['127.0.0.1:'+port,'localhost:'+port].includes(String(req.headers.host).toLowerCase()))fail('Invalid host',403);
  const url=new URL(req.url,'http://localhost'), route=url.pathname;
  if(route==='/api/health'&&req.method==='GET')return json(res,{service:'taskboard-web',ok:true});
  if(route==='/api/projects'&&req.method==='GET')return json(res,{projects:(await projects()).map(({root,...p})=>p)});
  if(route==='/api/board'&&req.method==='GET')return json(res,await snapshot(await boardFor(url.searchParams.get('board')),url.searchParams.get('archived')==='1'));
  if(route==='/api/review-media'&&['GET','HEAD'].includes(req.method))return await reviewMedia(req,res,await boardFor(url.searchParams.get('board')),url);
  if(route==='/api/action'&&req.method==='POST'){
   const {method,params}=await body(req);
   if(!allowedMethods.has(method)||!params||typeof params!=='object')fail('Unknown TaskBoard action');
   if(method!=='inbox.list')await boardFor(params.board);
   return json(res,await hub(method,params));
  }
  if(!['GET','HEAD'].includes(req.method))fail('Method not allowed',405);
  if(route==='/'||route==='/index.html')return await stream(req,res,path.join(dist,'index.html'),dist);
  if(/^\/assets\/[a-zA-Z0-9_.-]+$/.test(route))return await stream(req,res,path.join(dist,route.slice(1)),dist);
  fail('Not found',404);
 }catch(e){if(!res.headersSent)json(res,{message:e.status?e.message:'TaskBoard service unavailable: '+String(e.message).slice(0,300)},e.status||503);else res.destroy();}
});
server.listen(port,'127.0.0.1',()=>console.log('TaskBoard web listening on 127.0.0.1:'+port));

