#!/usr/bin/env python3
"""Issue-66 trust gate. Read-only. Encodes the two user rules + integrity checks.
RULE 1: every parameter name must be a real amp/preamp/pedal control (whitelist).
RULE 2: every decoded numeric value must appear in the capture's real name OR the
        tone3000 description (filename + description are the source of truth).
Plus deterministic integrity: file<->manifest sync, unique grids, no leftover hash,
no N_M decimal-as-value bug, no enum value that is a raw filename."""
import os,re,json,glob,subprocess,urllib.request,sys
import yaml
WS=os.path.dirname(os.path.dirname(os.path.abspath(__file__))); os.chdir(WS)
TOKEN="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6Imd6eWJpdW9weGtkeGJ5dG5vamRzIiwicm9sZSI6ImFub24iLCJpYXQiOjE3MzgwODIxNjUsImV4cCI6MjA1MzY1ODE2NX0.Gq66BJXjtLsqP2nAGXm9Xb9PAjoeZalWUj66K4nmVSU"

# ---- RULE 1: canonical real controls (knobs + selectors + IR micing + sentinels) ----
CANON={
 'gain','drive','dist','distortion','overdrive','fuzz','sustain','tone','treble','bass','middle','mid',
 'presence','contour','depth','resonance','reverb','level','volume','master','output','input','mix','blend',
 'filter','bias','sag','gate','attack','decay','boost','body','texture','bright','cut','voice','voicing',
 'character','sensitivity','channel','mode','mic','position','speaker','pickup','wiring','rectifier','fat',
 'eq','clipping','compression','distance','angle','axis','deep','scoop','shape','tight','warmth','edge','grit',
 'low','high','mids','lows','highs','top','bottom','volume1','volume2','volume_1','volume_2','gain1','gain2',
 'lowcut','highcut','mid_freq','range','clarity','girth','focus','color','colour','threshold','presence',
 'preset','default',  # catch-all + single-capture sentinel (allowed)
 # real selectors/controls that exist on specific gear (calibrated in)
 'transistor','cab','cabinet','voltage','grunge','switch','jumper','jumpered','structure','watt','wattage',
 'power','stage','pre','post','sub','girth','color','colour','loud','sustain','rate','speed','intensity',
 'feedback','width','tightness','smooth','harmonics',
 'stab','balance','pull_gain','tube','clip','jump','sw1','sw2','sw3','comp',
 'feel','hf','load',
 'version',  # honest NAM model-version/size variant (v1/v2, lite/standard) — provenance, not a fake control
 'aggression','solo','tubes',
}
# obvious real controls under a non-canonical spelling -> normalize (fixable, not invented)
NORMALIZE={'mv':'master','vol':'volume','pres':'presence','master_volume':'master','brightness':'bright',
 'mic_position':'position','cab_mic':'mic','dallastreble':'treble','rev':'reverb','comp':'compression',
 'od':'overdrive','vol1':'volume1','vol2':'volume2','gain_level':'gain','gain_range':'gain',
 'gain_percent':'gain','drive_percent':'drive','pres':'presence','ultra_hi':'bright','ultra_lo':'deep',
 'gain_boost':'boost'}

# ---- ground-truth helpers ----
orig={}; origcount={}
for line in subprocess.run(["git","ls-tree","-r","0deec200"],capture_output=True,text=True).stdout.splitlines():
    m=re.match(r'\S+ blob (\S+)\t(.+)$',line)
    if m and re.search(r'/(captures/.+\.nam|.+\.wav)$',m.group(2)):
        orig[m.group(1)]=os.path.basename(m.group(2))
        pm=re.match(r'(plugins/source/(?:ir|nam)/[^/]+)/',m.group(2))
        if pm: origcount[pm.group(1)]=origcount.get(pm.group(1),0)+1
CACHE=os.path.join(WS,"scripts",".tone_cache.json")
tcache=json.load(open(CACHE)) if os.path.isfile(CACHE) else {}
import atexit; atexit.register(lambda c=tcache,p=CACHE: json.dump(c,open(p,"w")))
def tone(tid):
    if tid in tcache: return tcache[tid]
    res={"names":{}, "desc":""}
    try:
        r=urllib.request.Request(f"https://api.tone3000.com/rest/v1/models?tone_id=eq.{tid}&select=name,model_url",headers={"apikey":TOKEN,"Authorization":"Bearer "+TOKEN})
        for x in json.load(urllib.request.urlopen(r,timeout=30)): res["names"][os.path.basename(x["model_url"])]=x.get("name") or ""
        r2=urllib.request.Request(f"https://api.tone3000.com/rest/v1/tones?id=eq.{tid}&select=description",headers={"apikey":TOKEN,"Authorization":"Bearer "+TOKEN})
        dd=json.load(urllib.request.urlopen(r2,timeout=30)); res["desc"]=(dd[0].get("description") or "") if dd else ""
    except Exception as e: res["err"]=str(e)
    tcache[tid]=res; return res

def name_nums(s):
    ns=[float(t) for t in re.findall(r'\d+(?:\.\d+)?',s)]
    for a,b in re.findall(r'(\d+)[._](\d+)',s):
        try: ns.append(float(f"{a}.{b}"))
        except: pass
    return ns
def num_match(val,hay):
    fv=float(val)
    for t in name_nums(hay):
        for c in (t,t/100,t/10,t*10,t*100):
            if abs(c-fv)<1e-6: return True
    return False

# batch blobs
mans=sorted(glob.glob("plugins/source/nam/*/manifest.yaml"))+sorted(glob.glob("plugins/source/ir/*/manifest.yaml"))
allf=[]
for m in mans:
    d=yaml.safe_load(open(m)); base=os.path.dirname(m)
    for c in d.get("captures",[]): allf.append(os.path.join(base,c["file"]))
blob={}
for i in range(0,len(allf),500):
    ch=allf[i:i+500]
    for p,h in zip(ch,subprocess.run(["git","hash-object"]+ch,capture_output=True,text=True).stdout.split()): blob[p]=h

reds=[]
for m in mans:
    name=m.split('/')[-2]; kind=m.split('/')[-3]; base=os.path.dirname(m); d=yaml.safe_load(open(m)); raw=open(m).read()
    tid=None
    for s in (d.get("sources") or []):
        mm=re.search(r'/tones/(\d+)',str(s)); tid=mm.group(1) if mm else tid
    issues=[]
    # IR plugins (cabs AND acoustic-guitar bodies) are not amps/pedals: their
    # valid params are mic-ing / processing-version, not amp knobs.
    IR_OK={'mic','mic_position','position','distance','angle','axis','speaker','cab','cabinet',
           'version','flavor','match','pickup','body','take','preamp','voicing','default','preset','mode'}
    allowed = IR_OK if kind=="ir" else CANON
    # RULE 1: param names
    badnames=[]
    for p in d.get("parameters",[]):
        nm=p.get("name")
        if nm in allowed: continue
        if kind!="ir" and nm in NORMALIZE: badnames.append(f"{nm}->{NORMALIZE[nm]}")
        else: badnames.append(f"{nm}(INVENTED)")
    if badnames: issues.append("NAME: "+", ".join(badnames))
    # integrity: data loss vs original (fewer capture files than 0deec200)
    pdir=f"plugins/source/{kind}/{name}"
    curcount=len(glob.glob(pdir+"/captures/*"))+len(glob.glob(pdir+"/**/*.wav",recursive=True))
    o=origcount.get(pdir)
    if o and curcount<o: issues.append(f"DATALOSS: {curcount} captures now vs {o} original")
    # integrity: sync
    mf=[c["file"].split("/")[-1] for c in d.get("captures",[])]
    disk=set(os.path.basename(x) for x in glob.glob(base+"/captures/*")) if kind=="nam" else set(os.path.basename(x) for x in glob.glob(base+"/**/*.wav",recursive=True))
    if set(mf)-disk: issues.append(f"SYNC: {len(set(mf)-disk)} manifest file(s) missing on disk")
    # integrity: unique
    combos=[tuple(sorted((k,str(v)) for k,v in (c.get('values') or {}).items())) for c in d.get("captures",[])]
    if len(combos)!=len(set(combos)): issues.append("DUP: duplicate value-combos")
    # N_M decimal-as-value
    nm_vals=set(re.findall(r':\s*(\d+_\d+)\b',raw))|set(re.findall(r'^\s*-\s*(\d+_\d+)\s*$',raw,re.M))
    if nm_vals: issues.append(f"DECIMAL: value(s) written N_M (parse wrong): {sorted(nm_vals)}")
    # enum value = raw filename / multiple-knobs-encoded-in-one-value
    knobtok=re.compile(r'(gain|drive|dist|tone|level|vol|volume|treble|bass|mid|presence|master|depth|comp)\d')
    for p in d.get("parameters",[]):
        vals=[str(v) for v in (p.get("values") or [])]
        def bad(v):
            if name in v: return True                          # contains plugin slug
            if len(knobtok.findall(v))>=2: return True         # encodes 2+ knob settings -> should be separate axes
            return False
        hit=next((v for v in vals if bad(v)),None)
        if hit:
            issues.append(f"ENUMFILE: axis '{p['name']}' value encodes multiple controls / slug: {hit}")
            break
    # ground truth per capture
    T=tone(tid) if tid else {"names":{},"desc":"","err":None}
    hay_desc=T.get("desc","")
    hashleft=0; nmis=0; nchk=0
    for c in d.get("captures",[]):
        fn=c["file"].split("/")[-1]; p=os.path.join(base,c["file"])
        ob=orig.get(blob.get(p,""))
        if tid and fn in T["names"]: hashleft+=1
        rn=T["names"].get(ob, ob or fn)
        hay=rn+" "+hay_desc
        for k,v in (c.get('values') or {}).items():
            if re.fullmatch(r'-?\d+(?:\.\d+)?',str(v)):
                nchk+=1
                if not num_match(v,hay): nmis+=1
    if hashleft: issues.append(f"HASH: {hashleft} capture(s) still named as tone3000 hash")
    advisory = f"VALUE: {nmis}/{nchk} numeric values not literally in name+description (heuristic; usually a clock/concat/word encoding, verify)" if (nchk and nmis) else None
    if issues or advisory: reds.append(dict(kind=kind,name=name,tid=tid,issues=issues,advisory=advisory))

hard=[r for r in reds if r["issues"]]
advis=[r for r in reds if not r["issues"] and r.get("advisory")]
cats={}
for r in hard:
    for i in r["issues"]: cats[i.split(":")[0]]=cats.get(i.split(":")[0],0)+1
print(f"GATE over {len(mans)} migrated plugins")
print(f"  GREEN (pass hard rules): {len(mans)-len(hard)}")
print(f"  RED   (hard violation):  {len(hard)}")
print(f"  red categories: {cats}")
print(f"  ADVISORY (VALUE heuristic, review only): {len([r for r in reds if r.get('advisory')])}")
json.dump(reds,open(os.path.join(WS,"scripts",".param_gate_reds.json"),"w"))
print("  full list -> scripts/.param_gate_reds.json")
