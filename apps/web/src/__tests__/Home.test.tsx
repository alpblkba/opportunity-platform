import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import App from "../App";
const home = { cv:{id:"cv1",label:"stored-cv.pdf",created_at:"2026-10-25T08:00:00Z"},cv_ready:true,profiles:[{id:"firmware",name:"Firmware and platform systems"},{id:"memory",name:"Memory and operating systems"}],selected_profile:"memory",jobs:[],matches:0,discovered:10,applications:[],questions:[],policies:[],mail:[],email_configured:false,last_updated:null };
afterEach(()=>{cleanup();vi.unstubAllGlobals();});
it("opens login immediately and shows the stored CV after authentication",async()=>{
 let signed=false;
 const fetch=vi.fn(async(path:string)=>{
  if(path.endsWith('/login')){signed=true;return Response.json({});}
  if(path.endsWith('/session'))return Response.json(signed?{}:{detail:'Sign in',needs:'native'},{status:signed?200:401});
  if(path.endsWith('/home'))return Response.json(home);
  return Response.json({});
 });vi.stubGlobal('fetch',fetch);render(<App/>);
 fireEvent.change(await screen.findByLabelText('Email'),{target:{value:'owner@example.test'}});
 fireEvent.change(screen.getByLabelText('Password'),{target:{value:'synthetic-password'}});
 fireEvent.click(screen.getByRole('button',{name:'Sign in'}));
 await screen.findByText('stored-cv.pdf');
 expect(screen.getByLabelText('Searching for').getAttribute('id')).toBe('profile');
 expect(screen.queryByRole('navigation')).toBeNull();
 expect(screen.queryByText('Cloudflare')).toBeNull();
 expect(screen.queryByText('Scoring weights')).toBeNull();
 expect(fetch.mock.calls.some(c=>c[0].includes('password'))).toBe(false);
});
it("persists profile changes through the API without reuploading the CV",async()=>{
 const fetch=vi.fn(async(path:string)=>Response.json(path.endsWith('/home')?home:{}));vi.stubGlobal('fetch',fetch);render(<App/>);
 fireEvent.change(await screen.findByLabelText('Searching for'),{target:{value:'firmware'}});
 await waitFor(()=>expect(fetch.mock.calls.some(c=>c[0]==='/api/home/profile')).toBe(true));
 expect(fetch.mock.calls.some(c=>c[0].includes('/documents/cv'))).toBe(false);
});
it("requires a deliberate employer authorization and displays exact questions",async()=>{
 const data={...home,questions:[{id:'q1',application_id:'a1',question_text:'Do you require visa sponsorship?',field_type:'select',options:['Yes','No']}],applications:[{id:'a1',opportunity_id:'j1',status:'needs_review',title:'Engineer',employer:'Example',url:'https://example.test'}],jobs:[{id:'j1',canonical_title:'Firmware engineer',organization:'Example',provider:'lever',source_key:'example',apply_url:'https://example.test',last_seen_at:'2026-10-25T08:00:00Z',score:70,explanation:'Your CV: C, Linux',gaps:[]}]};
 const fetch=vi.fn(async(path:string)=>Response.json(path.endsWith('/home')?data:{}));vi.stubGlobal('fetch',fetch);render(<App/>);
 const question=await screen.findByLabelText('Do you require visa sponsorship?');
 fireEvent.change(question,{target:{value:'No'}});fireEvent.click(screen.getByRole('button',{name:'Save answer'}));
 await waitFor(()=>expect(fetch.mock.calls.some(c=>c[0]==='/api/home/answer/q1')).toBe(true));
 expect(fetch.mock.calls.some(c=>c[0]==='/api/home/authorize')).toBe(false);
});
