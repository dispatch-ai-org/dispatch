#!/usr/bin/env python3
"""Isolated CLI/TUI setup journeys. Only synthetic provider auth is invoked."""
import json, os, sys, tempfile, subprocess, hashlib, re
from pathlib import Path
sys.dont_write_bytecode=True
from review_refinement import Session
binary=str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='dispatch-setup-') as tmp:
    root=Path(tmp); source=root/'project'; source.mkdir(); home=root/'home';home.mkdir();bins=root/'bin';bins.mkdir();state=root/'state'
    (source/'main.c').write_text('int main(void) { return 0; }\n')
    (root/'auth.json').write_text(json.dumps({'loggedIn':True,'authMethod':'claude.ai','apiProvider':'firstParty','email':'fixture@example.invalid','orgId':'fixture'}))
    for provider in ('codex','claude'):
        script='''#!/usr/bin/env python3
import sys,json,pathlib,os
root=pathlib.Path(__file__).resolve().parent.parent
with (root/'probe-args').open('a') as log:log.write(json.dumps(sys.argv[1:])+'\\n')
if '--version' in sys.argv: print('setup fixture 1');sys.exit()
if 'login' in sys.argv:
    assert not any(k in os.environ for k in ('OPENAI_API_KEY','ANTHROPIC_API_KEY','DISPATCH_CONTROL_GRANT_FD'))
    print('Fixture device login - press Enter',flush=True);input();sys.exit()
if 'auth' in sys.argv and 'status' in sys.argv: print((root/'auth.json').read_text());sys.exit()
assert 'app-server' in sys.argv, 'MODEL INVOCATION FORBIDDEN'
for line in sys.stdin:
    q=json.loads(line)
    if 'id' not in q:continue
    if q['method']=='initialize':r={'userAgent':'setup fixture'}
    elif q['method']=='account/read':r={'account':{'type':'chatgpt','planType':'plus','email':'fixture@example.invalid'}}
    else:r={}
    print(json.dumps({'id':q['id'],'result':r}),flush=True)
'''
        (bins/provider).write_text(script);(bins/provider).chmod(0o755)
    env={'HOME':str(home),'PATH':str(bins)+os.pathsep+os.environ['PATH'],'USER':'dispatch-fixture','DISPATCH_COLOR':'truecolor','COLORTERM':'truecolor','NO_COLOR':None,'OPENAI_API_KEY':'not-a-real-key','ANTHROPIC_API_KEY':'not-a-real-key'}
    captures=os.environ.get('DISPATCH_SETUP_CAPTURES',str(root/'captures'))
    def session(name,args=None):return Session([binary,'--state-dir',str(state)]+(args or []),source,captures,name,env=env,width=100,height=36)
    with session('fresh') as ui:
        ui.wait('accomplish?');ui.send('Update main.c safely\r');ui.wait('Add Codex');ui.mark('missing')
        ui.send('c\r');ui.wait('Exact included model ID');ui.send('fixed-codex-model\r');ui.wait('Effort');ui.send('\r');ui.wait('light / standard / strong');ui.send('\r')
        ui.wait('Type confirm');ui.mark('funding');ui.send('cancel\r');ui.wait('Add Codex');assert not state.exists()
        ui.send('b\r');ui.wait('accomplish?');ui.mark('preserved');ui.pump(.1)
        assert 'Update main.c safely' in ui.clean
        ui.send(b'\x03');ui.finish()
    for provider in ('codex','claude'):
        with session('cli-'+provider,['setup',provider]) as ui:
            ui.wait('Exact included model ID');ui.send(('fixed-codex-model' if provider=='codex' else 'claude-sonnet-5')+'\r')
            ui.wait('Effort');ui.send('\r');ui.wait('light / standard / strong');ui.send('\r');ui.wait('Type confirm');ui.mark('confirm');ui.send('confirm\r');ui.wait('Add Codex');ui.send('b\r');ui.finish()
    original=(state/'resources.yml').read_bytes();config=original.decode()
    assert config.count('provider: ')==2
    assert 'print_mode_included: true' in config and re.search(r'account_sha256: [a-f0-9]{64}',config)
    # Expired evidence can be refreshed; cancellation never renews it.
    config=re.sub(r'valid_until: [^\n]+','valid_until: 2020-01-01T00:00:00Z',config)
    (state/'resources.yml').write_text(config);expired=(state/'resources.yml').read_bytes()
    with session('revalidate',['setup']) as ui:
        ui.wait('Add Codex');ui.mark('expired');ui.send('2\r');ui.wait('Type confirm');ui.send('cancel\r');ui.wait('Add Codex');assert (state/'resources.yml').read_bytes()==expired
        ui.send('2\r');ui.wait('Type confirm');ui.mark('refresh');ui.send('confirm\r');ui.wait('Add Codex');ui.send('b\r');ui.finish()
    refreshed=(state/'resources.yml').read_bytes();assert max(map(int,re.findall(rb'authorization_revision: (\d+)',refreshed)))>2
    auth=json.loads((root/'auth.json').read_text());auth['email']='different@example.invalid';(root/'auth.json').write_text(json.dumps(auth))
    with session('account-change',['setup']) as ui:
        ui.wait('Add Codex');ui.send('2\r');ui.wait('account changed');ui.mark('blocked');ui.send('b\r');ui.finish()
    assert (state/'resources.yml').read_bytes()==refreshed
    auth['authMethod']='api';(root/'auth.json').write_text(json.dumps(auth))
    with session('unsupported',['setup','claude']) as ui:
        ui.wait('Resource unchanged');ui.send('b\r');ui.finish()
    with session('login',['setup']) as ui:
        ui.wait('Add Codex');ui.send('l\r');ui.wait('Choose codex');ui.send('codex\r');ui.wait('Type login');ui.send('login\r');ui.wait('Fixture device login');ui.send('\r');ui.wait('Add Codex');ui.mark('return');ui.send('b\r');ui.finish()
    assert (state/'resources.yml').read_bytes()==refreshed
    (source/'verify.sh').write_text('exit 0\n')
    with session('checks',['setup','--checks']) as ui:
        ui.wait('Choose checks');ui.mark('choices');ui.send('1\r');ui.finish()
    assert '- sh ./verify.sh' in (source/'dispatch.yml').read_text()
    calls=[json.loads(l) for l in (root/'probe-args').read_text().splitlines()]
    assert all('--version' in c or 'app-server' in c or 'status' in c or 'login' in c for c in calls)
    assert not (state/'dispatch.db').exists(), 'setup acquired execution authority'
    # A state that has already been used has a database; setup must still save.
    used=root/'used-state'
    subprocess.run([binary,'--state-dir',str(used),'history'],check=True,capture_output=True,env={k:v for k,v in env.items() if v is not None})
    before=(used/'dispatch.db').read_bytes()
    with Session([binary,'--state-dir',str(used),'setup','codex'],source,captures,'with-database',env=env,width=100,height=36) as ui:
        ui.wait('Exact included model ID');ui.send('claude-sonnet-5\r');ui.wait('Effort');ui.send('\r');ui.wait('light / standard / strong');ui.send('\r')
        ui.wait('Type confirm');ui.send('confirm\r');ui.wait('Resource saved');ui.wait('Add Codex');ui.send('b\r');ui.finish()
    assert 'provider: openai' in (used/'resources.yml').read_text()
    assert (used/'dispatch.db').read_bytes()==before, 'setup wrote to the database'
    print('CLI/TUI setup journeys passed; zero model calls; no database or grants created')
