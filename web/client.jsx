import React from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
export const apiBase=new URL('./api/',document.baseURI);
export async function call(endpoint,params={},body){
 const url=new URL(endpoint,apiBase);Object.entries(params).forEach(([k,v])=>url.searchParams.set(k,v));
 const res=await fetch(url,body?{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)}:{cache:'no-store'});
 if(!res.headers.get('content-type')?.includes('application/json'))throw Error('Session expired or TaskBoard is offline. Reload the page.');
 const data=await res.json();if(!res.ok)throw Error(data.message||'Request failed');return data;
}
export const action=(method,params)=>call('action',{}, {method,params});
export const op=()=>crypto.randomUUID();
export const fmt=s=>String(s||'').replaceAll('_',' ').replaceAll('-',' ');
export const matches=(t,q)=>!q||[t.id||t.taskId,t.title,t.description,t.category,t.claimed_by,t.user_notes].some(v=>String(v||'').toLowerCase().includes(q.toLowerCase()));
export function Markdown({value}){return <div className="markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} components={{a:({children,href})=><a href={/^https?:\/\//i.test(href||'')?href:undefined} target="_blank" rel="noreferrer">{children}</a>,img:({alt})=><span className="muted">[Image: {alt||'attachment'}]</span>}}>{value||''}</ReactMarkdown></div>}
