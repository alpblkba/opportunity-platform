import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import App from "../App";
const home = { cv:{id:"cv1",label:"stored-cv.pdf",created_at:"2026-10-25T08:00:00Z"},cv_ready:true,profiles:[{id:"firmware",name:"Firmware and platform systems"},{id:"memory",name:"Memory and operating systems"}],selected_profile:"memory",jobs:[],matches:0,discovered:10,applications:[],source_health:{enabled:3,failing:0,never_succeeded:0,stale:0,last_success_at:null},application_counts:{confirmed:0,reported:0,uncertain:0,failed:0,needs_review:0,queued:0,manual:0},questions:[],policies:[],mail:[],email_configured:false,last_updated:null };
afterEach(()=>{cleanup();vi.unstubAllGlobals();window.history.replaceState({},"","/");});
it("opens login immediately and shows the stored CV after authentication",async()=>{
 window.history.replaceState({},"","/login");
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
it("labels the blended priority honestly and flags postings that need review",async()=>{
 const job={id:'j2',canonical_title:'PhD position, memory systems',organization:'Example Lab',provider:'academic',source_key:'lab',apply_url:'https://example.test',last_seen_at:'2026-10-25T08:00:00Z',score:81.4,technical:70.2,profile_match:40,career_stage:'PhD position',career_priority:60,eligibility:'needs_review',explanation:'PhD position · Your CV: DRAM',gaps:['No funded vacancy confirmed']};
 vi.stubGlobal('fetch',vi.fn(async(path:string)=>Response.json(path.endsWith('/home')?{...home,jobs:[job],matches:1}:{})));render(<App/>);
 expect(await screen.findByText('Needs review')).toBeDefined();
 expect(screen.getByText('Priority 81 · technical fit 70 · career stage 60 · profile 40')).toBeDefined();
 expect(screen.getByText('No funded vacancy confirmed')).toBeDefined();
 expect(screen.queryByText('Strong fit')).toBeNull();
});
it("pauses and resumes every submission from one control",async()=>{
 const fetch=vi.fn(async(path:string)=>Response.json(path.endsWith('/home')?{...home,submissions_paused:true,max_automatic_per_day:10}:{}));vi.stubGlobal('fetch',fetch);render(<App/>);
 expect(await screen.findByText('All submissions paused')).toBeDefined();
 fireEvent.click(screen.getByRole('button',{name:'Resume'}));
 await waitFor(()=>expect(fetch.mock.calls.some(c=>c[0]==='/api/home/pause'&&JSON.parse(String((c as unknown as [string,RequestInit])[1]?.body)).paused===false)).toBe(true));
});
it("counts applied from every application, never from the recent page or uncertain outcomes",async()=>{
 const recent=[{id:'a1',opportunity_id:'j1',status:'submitted',title:'Engineer',employer:'Example',url:'https://example.test'}];
 vi.stubGlobal('fetch',vi.fn(async(path:string)=>Response.json(path.endsWith('/home')?{...home,applications:recent,application_counts:{confirmed:140,reported:12,uncertain:3,failed:9,needs_review:0,queued:0,manual:0}}:{})));render(<App/>);
 const overview=await screen.findByLabelText('Application overview');
 expect(overview.textContent).toContain('152Applied');
 expect(overview.textContent).toContain('3Outcome unknown');
});
it("never implies every source is healthy while one keeps failing",async()=>{
 vi.stubGlobal('fetch',vi.fn(async(path:string)=>Response.json(path.endsWith('/home')?{...home,last_updated:'2026-10-27T08:00:00Z',source_health:{enabled:20,failing:2,never_succeeded:0,stale:1,last_success_at:'2026-10-27T08:00:00Z'}}:{})));render(<App/>);
 expect(await screen.findByText(/2 of 20 employer sources keep failing to refresh/)).toBeDefined();
});
