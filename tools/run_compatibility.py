"""Run isolated boot observations for problem titles from an extracted workbook.

No user settings or save files are written. Results are observations, not
automatic claims of playable/completed compatibility. Native probe must be built.
"""
import argparse,collections,concurrent.futures,json,os,re,subprocess,time,unicodedata
from pathlib import Path

def key(name):
    name=re.sub(r'\.(chd|cue|iso|bin)$','',name,flags=re.I)
    stage=''
    for word in ('demo','beta','proto','prototype','testing'):
        if re.search(r'\b'+word+r'\b',name,re.I):stage+=word
    name=re.split(r'[\[(]',name,maxsplit=1)[0].replace('’',"'").replace('™','').replace('®','')
    name=unicodedata.normalize('NFKD',name).encode('ascii','ignore').decode().lower().replace('&','and')
    name=re.sub(r'[^a-z0-9 ]',' ',name);words=name.split()
    articles={'the','a','an','le','la','les','het','de','der','die','das','el','los','las'}
    if len(words)>1 and words[0]in articles:words=words[1:]
    if len(words)>1 and words[-1]in articles:words=words[:-1]
    return ''.join(words)+'|'+stage

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--results',type=Path,required=True);p.add_argument('--discs',type=Path,required=True)
    p.add_argument('--portable',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    p.add_argument('--frames',type=int,default=1800);p.add_argument('--workers',type=int,default=2)
    p.add_argument('--limit',type=int);p.add_argument('--resume',action='store_true')
    p.add_argument('--binary',type=Path)
    p.add_argument('--scripted-input',action='store_true',help='A repeatable centre-button and pointer sweep, not a gameplay verdict')
    a=p.parse_args();repo=Path(__file__).resolve().parents[1]
    a.output.mkdir(parents=True,exist_ok=True)
    aliases=json.loads((repo/'assets/title-aliases.json').read_text(encoding='utf8'))['titles']
    index=collections.defaultdict(set)
    for t in aliases:
        for alias in [t['name'],*t['aliases']]:index[key(alias)].add(t['slug'])
    def identity(title):
        ids=index.get(key(title),set());return next(iter(ids))if len(ids)==1 else None
    paths=[f for f in a.discs.rglob('*')if f.is_file()and f.suffix.lower()in ('.chd','.cue','.iso','.bin')]
    cues={str(f.with_suffix('')).lower()for f in paths if f.suffix.lower()=='.cue'}
    paths=[f for f in paths if not(f.suffix.lower()=='.bin'and str(f.with_suffix('')).lower()in cues)]
    by_id=collections.defaultdict(list);by_key=collections.defaultdict(list)
    for f in paths:
        d=re.search(r'\((?:Disc|Disk)\s+(\d+)',f.stem,re.I)
        if d and int(d[1])!=1:continue
        if (i:=identity(f.name)):by_id[i].append(f)
        by_key[key(f.name)].append(f)
    rows=json.loads(a.results.read_text(encoding='utf8'))
    problems={r['title']for r in rows if r['notes']or r['controls']=='Issues'or r['sound']=='Issues'or r['compat']not in ('Playable','Completed','Untested')}
    priorities=['Burn Cycle','Alice in Wonderland','Arcade Classics','Archeon CD-i Quiz','Who Shot Johnny Rock?™','Tetris®','Battleship®']
    rank=lambda t:priorities.index(t)if t in priorities else len(priorities)
    titles=sorted(problems,key=lambda t:(rank(t),t))
    cases=[];missing=[]
    def preference(f):
        n=f.name.lower();return (0 if '(usa' in n else 1 if '(uk' in n else 2 if '(europe' in n else 3,n)
    for title in titles:
        matches=by_id.get(identity(title),[])or by_key.get(key(title),[])
        if not matches:missing.append(title);continue
        disc=sorted(matches,key=preference)[0]
        stem=re.sub(r'\s*\((?:Disc|Disk)\s+\d+(?:\s+of\s+\d+)?\).*$', '',disc.stem,flags=re.I)
        slug=identity(title)or re.sub('[^a-z0-9]+','-',key(title)).strip('-')
        for bios in ('490','220'):
            case_dir=a.output/(slug+'-'+bios);case_dir.mkdir(parents=True,exist_ok=True)
            c={'title':title,'bios':str(a.portable/'BIOS'/('cdi490a.rom'if bios=='490'else'cdi220b.rom')),
               'dvc':str(a.portable/'BIOS'/('impega.rom'if bios=='490'else'vmpega.rom')),
               'disc':str(disc),'output':str(case_dir),'frames':a.frames,'snapshots':[min(900,a.frames),a.frames],
               'reported':[r for r in rows if r['title']==title and r['bios']==bios]}
            if a.scripted_input:
                relative=key(title)in (key('Tetris'),key('Battleship'))
                events=[]
                for frame in (1900,2200,2500):
                    events.extend([dict(frame=frame,x=384,y=280,buttons=1,relative=relative),dict(frame=frame+10,x=384,y=280,buttons=0,relative=relative)])
                for frame,x,y in [(2700,300,220),(2730,450,220),(2760,450,340),(2790,300,340),(2820,384,280)]:
                    events.append(dict(frame=frame,x=x,y=y,buttons=0,relative=relative))
                c.update(events=events,repeat_input=True,require_handover=True,snapshots=[1800,a.frames])
            for n in (stem,disc.stem):
                nv=a.portable/'NVRAM'/bios/(n+'.bin')
                if nv.is_file()and nv.stat().st_size==(32768 if bios=='490'else 8192):c['nvram']=str(nv);break
            case=case_dir/'case.json';case.write_text(json.dumps(c,ensure_ascii=False,indent=2),encoding='utf8');cases.append(case)
    if a.limit:cases=cases[:a.limit]
    scope='one available edition per listed title, both BIOSes; '+('scripted centre presses and pointer sweep after boot handover'if a.scripted_input else'no injected gameplay inputs in boot pass')
    (a.output/'manifest.json').write_text(json.dumps({'cases':[str(f)for f in cases],'missing_images':missing,'frames':a.frames,'scope':scope},ensure_ascii=False,indent=2),encoding='utf8')
    binary=a.binary or repo/'target/release/compatibility_probe.exe'
    def run(case):
        c=json.loads(case.read_text(encoding='utf8'));out=Path(c['output']);existing=out/'result.json'
        if a.resume and existing.exists():return json.loads(existing.read_text(encoding='utf8'))
        env=os.environ.copy()
        for k in list(env):
            if k.startswith('CDI_'):env.pop(k,None)
        try:
            result=subprocess.run([str(binary),str(case)],capture_output=True,encoding='utf8',errors='replace',timeout=150,env=env)
            (out/'stderr.log').write_text(result.stderr,encoding='utf8')
            if result.returncode!=0:raise RuntimeError('probe process exited '+str(result.returncode))
            return json.loads(existing.read_text(encoding='utf8'))
        except Exception as error:
            r={'title':c['title'],'bios':Path(c['bios']).stem,'error':str(error),'classification':'probe failure; investigate before judging game compatibility'}
            (out/'failure.json').write_text(json.dumps(r,indent=2),encoding='utf8');return r
    results=[];start=time.monotonic()
    print(f'Manifest: {len(cases)} runs, {len(missing)} titles without matched local images',flush=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1,min(a.workers,4)))as pool:
        for future in concurrent.futures.as_completed({pool.submit(run,c):c for c in cases}):
            r=future.result();results.append(r)
            (a.output/'results.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf8')
            print(f"{len(results)}/{len(cases)} {r['title']} {r['bios']}: "+('probe error'if'error'in r else 'CPU halted'if r['halted']else 'handover detected'if r['launch_detected_frame']is not None else 'handover NOT detected'),flush=True)
    lines=['# Automated compatibility observations','',scope,'',
           'These observations do not replace the user’s gameplay results or certify playability. A changing picture may be an intro; a static picture may be a valid menu.',
           f'{len(results)} runs completed. {len(missing)} titles had no unambiguous local image match.',
           '', '| Title | BIOS | Boot handover | CPU halted | Audio detected |','|---|---|---|---|---|']
    for r in sorted(results,key=lambda r:(r['title'],str(r['bios']))):
        lines.append('| '+ ' | '.join(str(x).replace('|','/')for x in [r['title'],r['bios'],'probe error'if'error'in r else r['launch_detected_frame'],r.get('halted','unknown'),r.get('audio_peak',0)>0.0001])+' |')
    if missing:lines+=['','Unmatched local images: '+', '.join(missing)]
    lines+=['',f'Elapsed minutes: {(time.monotonic()-start)/60:.1f}']
    (a.output/'report.md').write_text('\n'.join(lines),encoding='utf8')

if __name__=='__main__':main()
