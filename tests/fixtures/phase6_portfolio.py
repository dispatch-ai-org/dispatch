#!/usr/bin/env python3
"""Synthetic Claude/Codex protocols; real binary, pipes, SQLite and spawn markers.
No account discovery outside these fixture executables and no provider traffic.
"""
import hashlib
import json
import os
from pathlib import Path
import select
import sqlite3
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone
sys.dont_write_bytecode = True
from provider_fixture import Fixture


def proof(f):
    path=f.state/'resources.yml'
    text=path.read_text()
    prefix, encoded=text.split('    claude_subscription: ',1)
    line, *rest=encoded.splitlines()
    evidence=json.loads(line)
    evidence['executable_sha256']=hashlib.sha256((f.root/'claude').read_bytes()).hexdigest()
    path.write_text(prefix+'    claude_subscription: '+json.dumps(evidence)+'\n'+'\n'.join(rest)+'\n')


def run(f, *args):
    output=f.command('run', f.source, '--task','Add tests in src/lib.rs','--allow-unsafe-local','--json',*args,check=False)
    try: result=json.loads(output.stdout)
    except ValueError: result=None
    return output,result


def no_live_launch(f):
    with sqlite3.connect(f.state/'dispatch.db') as db:
        assert db.execute("SELECT COUNT(*) FROM attempt_launches WHERE state IN ('intent','spawned','uncertain')").fetchone()[0]==0


def add_peer(f, binary, first='claude'):
    os.environ['DISPATCH_FIXTURE_PROVIDER']='codex'
    peer=Fixture(binary)
    os.environ['DISPATCH_FIXTURE_PROVIDER']='claude'
    # Keep one shared state directory; independent provider fixture executables.
    config=f.source/'dispatch.yml'
    config.write_text(config.read_text()+f"  codex:\n    executable: '{peer.root/'codex'}'\n")
    path=f.state/'resources.yml'
    claude=path.read_text().split('profiles:\n')[1].replace('pool: shared','pool: claude-pool')
    codex=(peer.state/'resources.yml').read_text().split('profiles:\n')[1].replace('pool: shared','pool: codex-pool')
    codex=codex.replace('model: fixture-model','model: codex-fixed')
    path.write_text('version: 1\nallocation_enabled: true\ncapacity:\n  codex_probe: false\nprofiles:\n'+(claude+codex if first=='claude' else codex+claude))
    return peer


def checked(output, result):
    assert output.returncode==0,(output.stderr.decode(),result)
    assert result['outcome']['verification']=='passed',result
    assert result['outcome']['review']=='pending',result
    assert result['outcome']['application']=='not_applied',result


def scenario(binary, name):
    f=Fixture(binary)
    peer=None
    try:
        if name=='direct':
            refused=f.command('run',f.source,'--task','Add tests in src/lib.rs','--json',check=False)
            assert refused.returncode!=0 and f.count()==0
            assert not (f.root/'auth-count').exists(), 'consent must precede configured probes'
            os.environ['ANTHROPIC_API_KEY']='fixture-secret-not-forwarded'
            os.environ['CLAUDE_CODE_USE_BEDROCK']='1'
            output,result=run(f,'--agent','claude','--model','fixture-model','--effort','low')
            checked(output,result)
            a=result['attempts'][0]
            assert a['harness_id']=='claude' and a['observed_model']=='fixture-model'
            assert a['requested_effort']=='low' and a['observed_effort'] is None
            stored=f.stored(result['run_id'])
            assert stored['candidates'][0]['tokens']==22 and stored['candidates'][0]['cost_usd'] is None
            assert (f.source/'src/lib.rs').read_text()=='// baseline\n'
            assert f.count()==1
            no_live_launch(f)
        elif name in ('selection','missing'):
            peer=add_peer(f,binary,'codex')
            output,result=run(f)
            checked(output,result)
            assert result['allocation']['selected']['harness']=='codex'
            if name=='selection':
                output,result=run(f,'--agent','claude')
                checked(output,result)
                assert result['allocation']['selected']['harness']=='claude'
                output,_=run(f,'--agent','codex','--model','fixture-model')
                assert output.returncode!=0
                assert f.count()==1 and peer.count()==1
            else:
                (peer.root/'codex').unlink()
                output,result=run(f)
                checked(output,result)
                assert result['allocation']['selected']['harness']=='claude'
                output,_=run(f,'--agent','codex')
                assert output.returncode!=0 and f.count()==1
                # Missing optional Claude must not contaminate Codex-only selection.
                path=f.state/'resources.yml'
                path.write_text(path.read_text().replace('harness: claude','harness: claude\n    enabled: false'))
                output,_=run(f,'--agent','claude')
                assert output.returncode!=0 and f.count()==1
        elif name=='funding':
            for index,delta in enumerate([{'authMethod':'api_key'},{'authMethod':'oauth_token'},{'apiProvider':'bedrock'},
                          {'loggedIn':False},{'email':None},{'email':'different@example.invalid'},{'extraUsageEnabled':True}]):
                if index: f.cleanup();f=Fixture(binary)
                auth=f.root/'auth.json'
                value=json.loads(auth.read_text());value.update(delta);auth.write_text(json.dumps(value))
                output,_=run(f)
                assert output.returncode!=0 and f.count()==0,(delta,output.stdout,output.stderr)
            for field,value in [('usage_credits_disabled',False),('print_mode_included',False),('unmanaged_account',False)]:
                f.cleanup();f=Fixture(binary)
                path=f.state/'resources.yml';original_profile=path.read_text()
                prefix,encoded=original_profile.split('    claude_subscription: ')
                evidence=json.loads(encoded);evidence[field]=value
                path.write_text(prefix+'    claude_subscription: '+json.dumps(evidence)+'\n')
                output,_=run(f)
                assert output.returncode!=0 and f.count()==0
            f.cleanup();f=Fixture(binary)
            home=Path(os.environ['HOME'])
            (home/'.claude.json').write_text('{"apiKeyHelper":"must-not-run"}')
            output,_=run(f);assert output.returncode!=0 and f.count()==0
            f.cleanup();f=Fixture(binary)
            config=f.source/'dispatch.yml';config.write_text(config.read_text()+'    extra_args: [--bare]\n')
            output,_=run(f);assert output.returncode!=0 and f.count()==0
        elif name=='configuration':
            peer=add_peer(f,binary,'claude')
            path=f.state/'resources.yml';original=path.read_text()
            for old,new in [('model: fixture-model','model: sonnet'),('model: fixture-model','model: best'),('model: fixture-model','model: fixed[1m]'),
                            ('effort: low','effort: minimal'),('service_mode: standard','service_mode: fast'),
                            ('"contract_version": 1','"contract_version": 99'),
                            ('"account_sha256": "','"account_sha256": "invalid')]:
                path.write_text(original.replace(old,new,1))
                output,_=run(f)
                assert output.returncode!=0 and f.count()==0 and peer.count()==0,(old,output.stderr)
            path.write_text(original)
            # Missing/expired authorization is optional unavailability, not permission.
            prefix,tail=original.split('    claude_subscription: ',1);line,*rest=tail.splitlines()
            evidence=json.loads(line);evidence['valid_until']=(datetime.now(timezone.utc)-timedelta(seconds=1)).isoformat()
            path.write_text(prefix+'    claude_subscription: '+json.dumps(evidence)+'\n'+'\n'.join(rest)+'\n')
            output,result=run(f);checked(output,result);assert result['allocation']['selected']['harness']=='codex'
            output,_=run(f,'--agent','claude');assert output.returncode!=0 and f.count()==0
        elif name=='deadline':
            f.mode.write_text('wait')
            output,result=run(f,'--timeout','2')
            assert output.returncode!=0 and f.count()<=1,(output.stdout,output.stderr)
            assert result is not None,(output.stdout,output.stderr)
            assert len(result['attempts'])<=1
            assert f.stored(result['run_id'])['execution']['failure']=='deadline'
            no_live_launch(f)
        elif name=='protocol':
            agent=f.root/'claude';original=agent.read_text()
            cases=["print('{broken')", "print(json.dumps({'type':'assistant','message':{'model':'fixture-model'}}))",
                   "print(json.dumps({'type':'result','subtype':'error_during_execution','is_error':True,'errors':['unsupported model']}))",
                   "emit(json.dumps({'dispatch_checkpoint':{'version':1,'question':'Which?','category':'factual'}}));print('{broken')", "print(json.dumps({'type':'result','subtype':'error_during_execution','is_error':True,'result':'rate limit','errors':['rate limit']}))",
                   "emit('Done');emit('Done')", "emit('Done');print('{broken')",
                   "print(json.dumps({'type':'assistant','message':{'model':'other-model'}}));emit('Done')",
                   "emit('{\\\"dispatch_checkpoint\\\":42}')",
                   "emit(json.dumps({'dispatch_checkpoint':{'version':1,'question':'Approve?','category':'spending'}}))",
                   "print(json.dumps({'type':'result','subtype':'success','is_error':False,'result':'Done','permission_denials':[{'tool_name':'Bash'}]}))"]
            for index,replacement in enumerate(cases):
                if index:
                    f.cleanup();f=Fixture(binary);agent=f.root/'claude';original=agent.read_text()
                agent.write_text(original.replace("emit('Done')",replacement));proof(f)
                before=f.count();output,result=run(f)
                assert output.returncode!=0,(replacement,result,output.stderr)
                assert f.count()==before+1
                assert len(result['attempts'])==1 and result['outcome']['work_result']=='failed'
                assert not f.stored(result['run_id'])['execution']['questions']
                no_live_launch(f)
        elif name=='launch_change':
            # Account checks: selection, then the preflight at the spawn boundary.
            (f.root/'auth-boundary').write_text('2')
            output,result=run(f)
            assert output.returncode!=0 and f.count()==0,(output.stdout,output.stderr)
            assert len(result['attempts'])==1
            with sqlite3.connect(f.state/'dispatch.db') as db:
                assert db.execute("SELECT COUNT(*) FROM funding_refusals").fetchone()[0]>0
            # Restoring credentials alone must not renew the invalidated epoch.
            auth=f.root/'auth.json';value=json.loads(auth.read_text());value['email']='fixture@example.invalid';auth.write_text(json.dumps(value))
            output,_=run(f);assert output.returncode!=0 and f.count()==0
            no_live_launch(f)
        elif name=='settings':
            directory=f.source/'.claude';directory.mkdir()
            (directory/'settings.json').write_text(json.dumps({'hooks':{'SessionStart':[{'command':'must-not-run'}]},'apiKeyHelper':'must-not-run','fastMode':True,'fallbackModel':['credit-model']}))
            (f.source/'CLAUDE.md').write_text('Retain project instructions.')
            original=(directory/'settings.json').read_bytes()
            output,result=run(f);checked(output,result)
            assert (directory/'settings.json').read_bytes()==original
            assert Path(f.stored(result['run_id'])['candidates'][0]['workspace_path'],'CLAUDE.md').read_text()=='Retain project instructions.'
            (Path(os.environ['HOME'])/'.claude').mkdir()
            (Path(os.environ['HOME'])/'.claude/managed-settings.json').write_text('{}')
            output,_=run(f);assert output.returncode!=0 and f.count()==1
        elif name=='lifecycle':
            output,result=run(f);checked(output,result)
            path=f.state/'resources.yml';original=path.read_text()
            disabled=original.replace('harness: claude','harness: claude\n    enabled: false')
            path.write_text(disabled)
            output,_=run(f);assert output.returncode!=0 and f.count()==1
            # A disabled old definition cannot shadow an enabled replacement.
            path.write_text(disabled.replace('"contract_version": 1','"contract_version": 99')+original.split('profiles:\n')[1])
            output,replacement=run(f);checked(output,replacement);assert f.count()==2
            path.write_text(original.split('profiles:\n')[0]+'profiles: []\n')
            # Public historical status is readable after removal.
            status=f.command('status',result['run_id'],'--json');assert json.loads(status.stdout)['outcome']['review']=='pending'
        else: raise AssertionError(name)
        print('PASS phase6 '+name)
    finally:
        f.cleanup()
        if peer: peer.cleanup()

if __name__=='__main__': scenario(sys.argv[1],sys.argv[2])
