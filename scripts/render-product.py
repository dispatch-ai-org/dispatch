#!/usr/bin/env python3
"""Render real captured VT byte prefixes. Optional local Pillow, no image generation."""
import sys,json,subprocess,hashlib
from pathlib import Path
root=Path(__file__).resolve().parents[1]
source,target=map(lambda p:Path(p).resolve(),sys.argv[1:3]);target.mkdir(parents=True,exist_ok=True)
chosen={'direct':['startup','working','review'],'attention':['question'],'unverified':['review'],'failed':['failed'],'large':['file-index','file'],'narrow':['review'],'light':['review'],'mono':['review'],'missing':['setup'], 'fresh':['missing','funding','preserved'],'cli-claude':['confirm'],'revalidate':['expired','refresh'],'account-change':['blocked'],'login':['return'],'checks':['choices']}
index=[]
for name,labels in chosen.items():
    path=source/(name+'.markers.json')
    if not path.exists():continue
    markers=json.loads(path.read_text())
    for label in labels:
        if label not in markers:continue
        info=markers[label];raw=source/(name+'.'+label+'.ansi');output=target/(name+'-'+label+'.png')
        args=[sys.executable,str(root/'docs/captures/phase4-refinement/render-capture.py'),str(raw),str(output),'--width',str(info['width']),'--height',str(info['height'])]
        if name=='light':args+=['--background','#ffffff','--foreground','#17212d']
        subprocess.run(args,check=True)
        metadata=json.loads(output.with_suffix('.render.json').read_text());metadata.update(raw_sha256=hashlib.sha256(raw.read_bytes()).hexdigest(),milestone_seconds=info['seconds'])
        output.with_suffix('.render.json').write_text(json.dumps(metadata,indent=2)+'\n');index.append(dict(image=output.name,**metadata))
(target/'index.json').write_text(json.dumps(index,indent=2)+'\n')
print('Rendered',len(index),'actual PTY frames')
