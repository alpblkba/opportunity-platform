// PDF upload boundary against the same isolated test database used by test-product.mjs.
import { spawn } from 'node:child_process';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const db=process.env.TEST_DATABASE_URL;if(!db)throw Error('TEST_DATABASE_URL must point to an isolated test database');
const p=spawn(resolve('target/debug/opportunity'),['serve','--port','8898','--no-scheduler'],{env:{...process.env,DATABASE_URL:db,ACCESS_MODE:'native',APPLICATION_ENV:'development',DOCUMENT_ROOT:resolve('private/pdf-test')},stdio:'ignore'});
const base='http://127.0.0.1:8898';const wait=ms=>new Promise(r=>setTimeout(r,ms));
try{
 for(let n=0;n<50;n++){try{if((await fetch(base+'/ready')).ok)break;}catch{}await wait(100);}
 const auth=await fetch(base+'/api/auth/login',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({email:'owner@example.test',password:'Synthetic-CV-journey-2026!'})});assert.equal(auth.status,200);const cookie=auth.headers.get('set-cookie').split(';')[0];
 const req=async(path,body)=>fetch(base+'/api'+path,{headers:{Cookie:cookie,...(body?{'Content-Type':'application/octet-stream'}:{})},...(body?{method:'POST',body}:{})});
 const lines=['Ada Example','ada@example.test','Education','BSc Computer Science, completed 2022','Experience','3 years of experience developing Linux firmware','Projects','DRAM memory project in C++','Skills','C++, Linux, DRAM, CXL'];
 const content='BT /F1 12 Tf 50 750 Td 16 TL '+lines.map((line,i)=>(i?'T* ':'')+'('+line+') Tj').join('\n')+' ET';
 const objects=['<< /Type /Catalog /Pages 2 0 R >>','<< /Type /Pages /Kids [3 0 R] /Count 1 >>','<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>','<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',`<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`];
 let pdf='%PDF-1.4\n', offsets=[0];for(let i=0;i<objects.length;i++){offsets.push(Buffer.byteLength(pdf));pdf+=`${i+1} 0 obj\n${objects[i]}\nendobj\n`;}
 const xref=Buffer.byteLength(pdf);pdf+=`xref\n0 6\n0000000000 65535 f \n`+offsets.slice(1).map(n=>String(n).padStart(10,'0')+' 00000 n \n').join('')+`trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
 const upload=await req('/documents/cv?filename=synthetic-pdf-cv.pdf',Buffer.from(pdf));assert.equal(upload.status,201,await upload.clone().text());
 const doc=await upload.json();const home=await(await req('/home')).json();assert.equal(home.cv.id,doc.id);assert.equal(home.cv_ready,true);assert.ok(home.jobs.length>0);
 const bad=await req('/documents/cv?filename=invalid.pdf',Buffer.from('%PDF-1.4\nnot a PDF'));assert.equal(bad.status,400);assert.equal((await(await req('/home')).json()).cv.id,doc.id);
 const bytes=Buffer.from(await(await req('/documents/'+doc.id+'/download')).arrayBuffer());assert.deepEqual(bytes,Buffer.from(pdf));
 const anonymous=await fetch(base+'/api/documents/'+doc.id+'/download');assert.equal(anonymous.status,401);
 console.log('PASS PDF extraction, candidate/ranking update, private download, rejected invalid PDF preserves current CV');
}finally{p.kill('SIGTERM');}
