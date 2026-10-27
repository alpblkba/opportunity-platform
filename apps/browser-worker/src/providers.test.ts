import { test } from "node:test";
import { strict as assert } from "node:assert";
import { chromium } from "playwright";
import { applicationUrl, assertSupported, providerFor, receiptOn } from "./providers.js";
test("hosted forms are recognized, unsafe destinations and unauthorised submission are refused",()=>{
 assert.equal(applicationUrl('https://jobs.lever.co/company/123'),'https://jobs.lever.co/company/123/apply');
 assert.equal(applicationUrl('https://jobs.ashbyhq.com/company/123'),'https://jobs.ashbyhq.com/company/123/application');
 assert.equal(providerFor('https://job-boards.greenhouse.io/company/jobs/123'),'greenhouse');
 assert.equal(providerFor('https://jobs.lever.co.evil.test/company/123'),null);
 assert.throws(()=>assertSupported('https://jobs.lever.co/company/123',true,false));
 process.env.APPLICATION_ENV='development';
 assert.throws(()=>assertSupported('https://jobs.lever.co/company/123',true,true));
 delete process.env.APPLICATION_ENV;
});
test("all three providers require visible employer confirmation",async()=>{
 const browser=await chromium.launch();try{
  const page=await browser.newPage();
  for(const provider of ['greenhouse','lever','ashby'] as const){
   await page.setContent('<form><button>Submit application</button></form><p hidden>Your application has been submitted</p>');
   assert.equal(await receiptOn(page,provider),null);
   await page.setContent('<h1>Thank you for applying</h1><p>Your application has been received.</p>');
   assert.ok(await receiptOn(page,provider));
  }
 }finally{await browser.close();}
});
