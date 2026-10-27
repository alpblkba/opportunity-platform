// Local end-to-end checks. All application submissions and email delivery use loopback fixtures.
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
const require=createRequire(resolve('apps/browser-worker/package.json'));
const {chromium}=require('playwright');
const name='opportunity_journey_'+Date.now();
// TEST_OFFLINE=1 disables every seed source: no network, so CI is deterministic. Without it, live discovery is a smoke test.
const offline=process.env.TEST_OFFLINE==='1';
const dbUser=process.env.TEST_DB_USER||'opportunity',dbPassword=process.env.TEST_DB_PASSWORD||'opportunity_local_dev',dbPort=process.env.TEST_DB_PORT||'5432';
const dbURL=`postgres://${dbUser}:${dbPassword}@127.0.0.1:${dbPort}/${name}`;
const dbContainer=process.env.TEST_DB_CONTAINER||'opportunity-db';
const out=resolve(process.env.TEST_OUTPUT_DIR||'private/product-test');mkdirSync(out,{recursive:true,mode:0o700});
const sql=(query,db=name)=>execFileSync('docker',['exec','-i',dbContainer,'psql','-U',dbUser,'-d',db,'-At','-v','ON_ERROR_STOP=1'],{input:query,encoding:'utf8'}).trim();
const children=[];let browser;let server;let mail;let submitted=0;let accepted=0;const messages=new Map();
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function until(fn,timeout=120000){const end=Date.now()+timeout;while(Date.now()<end){const v=await fn();if(v)return v;await sleep(1000);}throw Error('Timed out waiting for product state');}
const launch=(args,env={})=>{const p=spawn(resolve('target/debug/opportunity'),args,{env:{...process.env,DATABASE_URL:dbURL,ACCESS_MODE:'native',APPLICATION_ENV:'development',DOCUMENT_ROOT:resolve(out,'documents'),BROWSER_WORKER_ARTEFACTS:resolve(out,'browser'),MAIL_API_URL:'http://127.0.0.1:8909',MAIL_API_KEY:'synthetic-test-key',MAIL_FROM:'test@example.test',MAIL_TO:'owner@example.test',...env},stdio:['pipe','pipe','pipe']});children.push(p);p.stdout.on('data',d=>writeFileSync(resolve(out,'server.log'),d,{flag:'a'}));p.stderr.on('data',d=>writeFileSync(resolve(out,'server.log'),d,{flag:'a'}));return p;};
const run=async(args,input='')=>{const p=launch(args);p.stdin.end(input);await new Promise((ok,bad)=>p.on('exit',code=>code===0?ok():bad(Error(`CLI failed: ${args.join(' ')}`))));};
const appOrigin='http://127.0.0.1:8897';
try{
 sql(`create database "${name}"`,'postgres');
 await run(['migrate']);
 await run(['user','create','--email','owner@example.test','--name','Test Owner'],'Synthetic-CV-journey-2026!\n');
 if(offline){await run(['source','import','config/sources.seed.toml']);sql('update sources set enabled=false');}
 // A synthetic ranking corpus: equal technical evidence at three career stages, plus an unrelated internship.
 sql(`insert into organizations(id,normalized_name,display_name) values('00000000-0000-4000-8000-000000000011','synthetic ranking employer','Synthetic ranking employer');
 insert into sources(id,key,display_name,kind,organization_id,endpoint,enabled) values('00000000-0000-4000-8000-000000000012','test-ranking','Synthetic ranking employer','lever','00000000-0000-4000-8000-000000000011','{"kind":"lever","site_id":"ranking"}',false);
 insert into opportunities(id,organization_id,dedupe_key,canonical_title,kind,advertised_state,search_text) values
  ('00000000-0000-4000-8000-000000000021','00000000-0000-4000-8000-000000000011','rank-intern','Memory systems intern','internship','advertised','Memory systems intern DRAM CXL Linux kernel C++'),
  ('00000000-0000-4000-8000-000000000022','00000000-0000-4000-8000-000000000011','rank-junior','Junior memory engineer','employment','advertised','Junior memory engineer DRAM CXL Linux kernel C++'),
  ('00000000-0000-4000-8000-000000000023','00000000-0000-4000-8000-000000000011','rank-senior','Senior memory engineer','employment','advertised','Senior memory engineer DRAM CXL Linux kernel C++'),
  ('00000000-0000-4000-8000-000000000024','00000000-0000-4000-8000-000000000011','rank-marketing','Marketing intern','internship','advertised','Marketing intern social media brand');
 insert into postings(source_id,native_id,opportunity_id,url,title,description_text,content_hash) values
  ('00000000-0000-4000-8000-000000000012','r1','00000000-0000-4000-8000-000000000021','http://127.0.0.1:8908/apply/single','Memory systems intern','DRAM CXL Linux kernel C++ memory performance','r1'),
  ('00000000-0000-4000-8000-000000000012','r2','00000000-0000-4000-8000-000000000022','http://127.0.0.1:8908/apply/single','Junior memory engineer','DRAM CXL Linux kernel C++ memory performance','r2'),
  ('00000000-0000-4000-8000-000000000012','r3','00000000-0000-4000-8000-000000000023','http://127.0.0.1:8908/apply/single','Senior memory engineer','DRAM CXL Linux kernel C++ memory performance','r3'),
  ('00000000-0000-4000-8000-000000000012','r4','00000000-0000-4000-8000-000000000024','http://127.0.0.1:8908/apply/single','Marketing intern','Social media and brand strategy','r4');`);
 const mock=await import(resolve('apps/browser-worker/dist/mock-server.js'));
 server=await mock.startMockAts(8908);
 mail=createServer(async(req,res)=>{
  const path=new URL(req.url,'http://localhost').pathname;
  if(req.method==='POST'&&path==='/emails'){
   let raw='';for await(const c of req)raw+=c;
   const body=JSON.parse(raw);assert.deepEqual(body.to,['owner@example.test']);assert.equal(body.from,'test@example.test');
   const key=req.headers['idempotency-key'];assert.ok(key);
   if(!messages.has(key)){accepted++;messages.set(key,{id:String(accepted),body});}
   res.writeHead(200,{'content-type':'application/json'}).end(JSON.stringify({id:messages.get(key).id}));
  }else if(path.startsWith('/emails/'))res.writeHead(200,{'content-type':'application/json'}).end(JSON.stringify({last_event:'delivered'}));
  else res.writeHead(404).end();
 });await new Promise(r=>mail.listen(8909,'127.0.0.1',r));
 let app=launch(['serve','--port','8897']);
 await until(async()=>{try{return(await fetch(appOrigin+'/ready')).ok;}catch{return false;}});
 browser=await chromium.launch();const page=await browser.newPage({viewport:{width:1150,height:950}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto(appOrigin);await page.getByText('bro casually replacing Linkedin').waitFor();await page.getByRole('link',{name:'Log in as Alp (alpblkba)'}).click();await page.getByLabel('Email',{exact:true}).fill('owner@example.test');await page.getByLabel('Password',{exact:true}).fill('Synthetic-CV-journey-2026!');await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.getByLabel('Searching for').waitFor();
 const api=async(path,body)=>page.evaluate(async([path,body])=>{const r=await fetch('/api'+path,body===undefined?{}:{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});if(!r.ok)throw Error(await r.text());return r.status===204?null:r.json();},[path,body]);
 assert.equal((await api('/home')).cv,null);
 const cv='Ada Example\nada@example.test\nPhone: +49 123 4567890\nEducation\nBSc Computer Science, completed 2022\nExperience\nSystems Engineer, 2022 - 2025\n3 years of experience\nLinux kernel, C++, memory performance, firmware and device driver development\nProjects\nCXL memory tiering, DRAM controller, RISC-V FPGA and SystemVerilog\nSkills\nC, C++, Python, Linux, DRAM, CXL, kernel, firmware, RTL, FPGA, RISC-V, SystemVerilog\n';
 await page.getByLabel('Upload CV',{exact:true}).setInputFiles({name:'synthetic-test-cv.txt',mimeType:'text/plain',buffer:Buffer.from(cv)});
 await page.getByText('CV updated. Your matches now use this version.').waitFor();
 let home=await api('/home');assert.equal(home.profiles.length,5);const memory=home.profiles.find(p=>p.name.startsWith('Memory'));await page.getByLabel('Searching for').selectOption(memory.id);
 await until(async()=>{home=await api('/home');return (offline||home.discovered>300&&home.jobs.length>5)&&home.selected_profile===memory.id&&home.jobs.some(j=>j.source_key==='test-ranking');},240000);
 assert.equal(new URL(page.url()).pathname,'/workspace');
 const rank=title=>home.jobs.findIndex(j=>j.canonical_title===title);
 assert.ok(rank('Memory systems intern')>=0&&rank('Memory systems intern')<rank('Junior memory engineer')&&rank('Junior memory engineer')<rank('Senior memory engineer'),JSON.stringify(home.jobs.map(j=>[j.canonical_title,j.score])));
 assert.equal(rank('Marketing intern'),-1);
 assert.equal(home.ranking_policy,'career-priority-v3');
 const cvId=home.cv.id;
 await page.reload();await page.getByText('synthetic-test-cv.txt').waitFor();assert.equal((await api('/home')).cv.id,cvId);
 app.kill('SIGTERM');await new Promise(r=>app.on('exit',r));app=launch(['serve','--port','8897']);await until(async()=>{try{return(await fetch(appOrigin+'/ready')).ok;}catch{return false;}});
 await page.reload();await page.getByText('synthetic-test-cv.txt').waitFor();home=await api('/home');assert.equal(home.cv.id,cvId);assert.equal(home.selected_profile,memory.id);
 // A replacement changes the candidate record and rankings, without asking for manual facts.
 await page.getByLabel('Upload CV',{exact:true}).setInputFiles({name:'synthetic-test-cv-v2.txt',mimeType:'text/plain',buffer:Buffer.from(cv.replace('3 years','4 years'))});
 await page.getByText('CV updated. Your matches now use this version.').waitFor();home=await api('/home');assert.notEqual(home.cv.id,cvId);
 console.log(`PASS landing, login, persistent CV, profile persistence, restart, CV replacement, ${offline?'synthetic':'real'} discovery and career-stage ranking`);
 await page.screenshot({path:resolve(out,'ranked-desktop.png'),fullPage:false});
 await page.setViewportSize({width:390,height:844});await page.screenshot({path:resolve(out,'ranked-mobile.png'),fullPage:false});
 assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);await page.setViewportSize({width:1150,height:950});
 const examples=home.jobs.slice(0,5).map(j=>({employer:j.organization,title:j.canonical_title,score:j.score,explanation:j.explanation,gaps:j.gaps,url:j.apply_url}));
 // Synthetic ATS records exist only in this isolated test database.
 sql(`insert into organizations(id,normalized_name,display_name) values('00000000-0000-4000-8000-000000000001','synthetic test employer','Synthetic test employer');
 insert into sources(id,key,display_name,kind,organization_id,endpoint,enabled) values('00000000-0000-4000-8000-000000000002','test-lever','Synthetic test employer','lever','00000000-0000-4000-8000-000000000001','{"kind":"lever","site_id":"test"}',false);
 insert into opportunities(id,organization_id,dedupe_key,canonical_title,kind,advertised_state,search_text) values('00000000-0000-4000-8000-000000000003','00000000-0000-4000-8000-000000000001','synthetic-journey','Firmware and memory engineer','employment','advertised','firmware memory Linux');
 insert into postings(source_id,native_id,opportunity_id,url,title,description_text,content_hash) values('00000000-0000-4000-8000-000000000002','test-job','00000000-0000-4000-8000-000000000003','http://127.0.0.1:8908/apply/open','Firmware and memory engineer','C++ Linux firmware memory DRAM CXL kernel','synthetic');`);
 const job='00000000-0000-4000-8000-000000000003';await api('/home/apply/'+job,{});
 await until(async()=>{home=await api('/home');return home.questions.length===2;},120000);
 assert.equal(mock.receipts.size,0);console.log('PASS employer questions extracted, no submission without authorization');
 await api('/home/authorize',{profile_id:memory.id,source_key:'test-lever'});
 await page.reload();await page.getByLabel('Do you require visa sponsorship?').selectOption('No');
 const visaForm=page.getByLabel('Do you require visa sponsorship?').locator('..').locator('..');await visaForm.getByRole('button',{name:'Save answer'}).click();
 await page.getByLabel('Notice period').fill('One month');const noticeForm=page.getByLabel('Notice period').locator('..').locator('..');await noticeForm.getByRole('button',{name:'Save answer'}).click();
 await until(async()=>{home=await api('/home');return home.applications.some(a=>a.opportunity_id===job&&a.status==='submitted');},120000);
 assert.equal(mock.receipts.size,1);submitted=mock.receipts.size;
 const a=home.applications.find(a=>a.opportunity_id===job);assert.equal(sql(`select cv_document_id from applications where id='${a.id}'`),home.cv.id,'the submission records the CV version it used');await api('/home/apply/'+job,{});await sleep(18000);assert.equal(mock.receipts.size,1);
 await until(async()=>{home=await api('/home');return home.mail.some(m=>m.application_id===a.id&&m.state==='delivered');},120000);
 const submittedEmails=[...messages.values()].filter(m=>m.body.subject.startsWith('Confirmed submitted'));assert.equal(submittedEmails.length,1);assert.match(submittedEmails[0].body.text,/Synthetic test employer/);assert.match(submittedEmails[0].body.text,/http:\/\/127.0.0.1:8908\/apply\/open/);
 console.log('PASS question answers resume, CV attached, confirmed submission, duplicate prevention, one remotely confirmed synthetic email');
 assert.equal(errors.length,0,errors.join('\n'));
 const counts=sql("select s.kind,count(*) from postings p join sources s on s.id=p.source_id where s.key<>'test-lever' and p.closed_at is null group by s.kind order by s.kind;");
 const result={database:name,checks:['login','CV upload/extraction','stored CV across reload/restart','five profiles','profile persistence','CV replacement','live discovery','rankings','responsive UI','exact employer questions','no unauthorized submit','answer/resume','confirmed receipt','deduplication','email accepted then remotely confirmed'],counts:counts,examples,submissions:submitted,confirmedSubmissionEmails:submittedEmails.length,browserErrors:errors};
 writeFileSync(resolve(out,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result,null,2));
}finally{
 if(browser)await browser.close();for(const p of children)if(p.exitCode===null)p.kill('SIGTERM');
 if(server)await server.close();if(mail){mail.closeAllConnections();mail.close();}
 // Keep the isolated database only when diagnosing a failure; it never becomes the owner's database.
 console.log('Test database:',name);
}
