#!/usr/bin/env python3
"""Repeatable real PTY core loop. Uses disposable provider fixtures, never accounts."""
import sys, os, json, hashlib, subprocess, time
from pathlib import Path
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'tests/fixtures'))
from phase8_planning import fixture
from review_refinement import Session

binary, out = str(Path(sys.argv[1]).resolve()), Path(sys.argv[2]).resolve()
out.mkdir(parents=True, exist_ok=True)
finish_build='0.1.2' not in subprocess.check_output([binary,'version'],text=True)
scenarios = sys.argv[3:] or ['direct', 'planned', 'attention', 'unverified', 'failed', 'large', 'narrow', 'light', 'mono', 'missing']
for name in scenarios:
    f=fixture(binary, 'question' if name=='attention' else 'success')
    try:
        agent=f.root/f.provider
        script=agent.read_text().replace("prompt=sys.argv[-1]", "import time\ntime.sleep(0.6)\nprompt=sys.argv[-1]")
        if name=='large':
            for i in range(48): (f.source/f'item-{i:02}.txt').write_text('before\n')
            (f.source/'image.bin').write_bytes(bytes([0,255,1]))
            (f.source/'old.txt').write_text('rename\n')
            (f.source/'mode.sh').write_text('#!/bin/sh\nexit 0\n')
            script=script.replace("elif task=='direct':", """elif task=='direct':
        for i in range(48): pathlib.Path(f'item-{i:02}.txt').write_text('after\\n')
        pathlib.Path('image.bin').write_bytes(bytes([0,255,2]))
        pathlib.Path('old.txt').rename('new.txt')
        pathlib.Path('mode.sh').chmod(0o755)
        pathlib.Path('generated.lock').write_text('generated\\n'+'x'*40000+'\\n')""")
        agent.write_text(script)
        cfg=json.loads((f.source/'dispatch.yml').read_text())
        if name=='unverified': cfg['checks']={'verify':[]}
        if name=='failed': cfg['checks']={'verify':['false']}
        (f.source/'dispatch.yml').write_text(json.dumps(cfg))
        if name=='missing': (f.state/'resources.yml').unlink()
        width,height=(38,24) if name=='narrow' else (80,24) if name in ('mono','light') else (120,40)
        env={'COLORTERM':'truecolor','DISPATCH_COLOR':'truecolor','DISPATCH_THEME':'light' if name=='light' else 'native','NO_COLOR':None}
        args=[binary,'--state-dir',str(f.state),'--no-retry']
        if name=='mono':args+=['--no-color','--ascii']
        with Session(args, f.source, out, name, env=env,width=width,height=height) as ui:
            ui.wait('accomplish?');ui.mark('startup')
            goal=('/plan ' if name in ('planned','attention','narrow') else '')+'Update a.c to return a positive value'
            ui.send('\x1b[200~'+goal+'\x1b[201~');ui.pump(.08)
            sent=time.monotonic();ui.send('\r')
            ui.wait('Goal')
            ui.markers['local_ack']={'milliseconds':round((time.monotonic()-sent)*1000,2),'method':'Staged bracketed paste then Enter; PTY reader polls every 50 ms; upper bound, not provider latency'}
            if name=='missing':
                ui.wait('Add Codex' if finish_build else 'allocation is not configured');ui.mark('setup');ui.send(b'\x03');ui.pump(.3)
                if ui.process.poll() is None: ui.send(b'\x03')
                ui.finish();continue
            ui.wait('[y/N]');ui.send('y\r')
            if name=='unverified':
                ui.pump(.15)
                if 'Choose checks' in ui.clean:ui.send('n\r')
            ui.wait('elapsed')
            if name in ('planned','attention','narrow'): ui.wait('requires checked output')
            elif finish_build: ui.wait('tool activity is in private logs')
            else: ui.wait('Working')
            ui.mark('working')
            if name=='attention':
                ui.wait('Your answer');ui.mark('question');ui.send('2\r')
            if name=='failed':
                ui.wait('Work stopped.');ui.mark('failed');ui.send('n\r')
            else:
                ui.wait('Review changes');ui.mark('review')
                if name=='large':
                    ui.send('d\r');ui.wait('q back');ui.mark('file-index');ui.send('j\r');ui.pump(.2);ui.mark('file');ui.send('q');ui.wait('Review changes')
                ui.send('r\r')
            ui.wait('accomplish?');ui.send(b'\x04');ui.finish()
        print(name, 'passed', flush=True)
    finally:f.temp.cleanup()
(out/'identity.json').write_text(json.dumps({'binary':binary,'binary_sha256':hashlib.sha256(Path(binary).read_bytes()).hexdigest(),'git_head':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'fixture_only':True,'platform':sys.platform},indent=2)+'\n')
