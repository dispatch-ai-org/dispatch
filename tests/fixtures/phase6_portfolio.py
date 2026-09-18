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
from phase5_control import Fixture, finish, assert_error


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


def no_lease(f):
    with sqlite3.connect(f.state/'dispatch.db') as db:
        assert db.execute('SELECT COUNT(*) FROM pool_leases').fetchone()[0]==0


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


def grant(f):
    f.key=Path(f.command('control-grant',f.source,'--allow-unsafe-local','--delegate-factual').stdout.decode().strip())


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
            assert stored['capacity']['scarcity']=='unknown'
            assert (f.source/'src/lib.rs').read_text()=='// baseline\n'
            assert f.count()==1
            no_lease(f)
            with sqlite3.connect(f.state/'dispatch.db') as db:
                assert db.execute('SELECT COUNT(*) FROM sync_outbox').fetchone()[0]==0
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
            assert len(result['attempts'])<=1
            assert f.stored(result['run_id'])['phase3']['failure']=='deadline'
            no_lease(f)
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
                assert not f.stored(result['run_id'])['phase3']['questions']
                no_lease(f)
        elif name=='recovery':
            peer=add_peer(f,binary)
            path=f.state/'resources.yml';original_profiles=path.read_text()
            config=f.source/'dispatch.yml';config.write_text(config.read_text().replace('test -f result.txt', 'test "$(cat result.txt)" = ok'))
            claude=f.root/'claude';codex=peer.root/'codex'
            originals={claude:claude.read_text(),codex:codex.read_text()}
            for first in ('claude','codex'):
                # The first lane fails a check known to pass on the original baseline.
                for agent,text in originals.items():
                    if agent.name==first:
                        text=text.replace("write_text('ok\\n')", "write_text('bad\\n')")
                    agent.write_text(text)
                text=original_profiles
                if first=='claude':
                    text=text.replace('model: codex-fixed\n    effort: low','model: codex-fixed\n    effort: low').replace('pool: codex-pool\n    provider_buckets: [codex]\n    tier: light','pool: codex-pool\n    provider_buckets: [codex]\n    tier: strong')
                else:
                    text=text.replace('pool: claude-pool\n    provider_buckets: [codex]\n    tier: light','pool: claude-pool\n    provider_buckets: [codex]\n    tier: strong')
                path.write_text(text);proof(f)
                output,result=run(f)
                checked(output,result)
                assert [a['harness_id'] for a in result['attempts']]==[first,'codex' if first=='claude' else 'claude'],result
                a,b=result['attempts'];assert b['detail']['parent_attempt_id']==a['id']
                assert a['detail']['input_baseline']==b['detail']['input_baseline']
                assert Path(a['detail']['result']['workspace_path'],'result.txt').read_text()=='bad\n'
                assert (f.source/'result.txt').read_text()=='ok\n'
                no_lease(f)
                before=f.count()+peer.count()
                out,limited=run(f,'--no-retry')
                assert out.returncode!=0 and len(limited['attempts'])==1 and f.count()+peer.count()==before+1
                before=f.count()+peer.count()
                out,limited=run(f,'--agent',first)
                assert out.returncode!=0 and len(limited['attempts'])==1 and f.count()+peer.count()==before+1
                for agent,script in originals.items():
                    agent.write_text(script.replace("write_text('ok\\n')", "write_text('bad\\n')"))
                proof(f);before=f.count()+peer.count()
                out,failed=run(f)
                assert out.returncode!=0 and len(failed['attempts'])==2 and f.count()+peer.count()==before+2
                no_lease(f)
        elif name=='launch_change':
            (f.root/'auth-boundary').write_text('3')
            output,result=run(f)
            assert output.returncode!=0 and f.count()==0,(output.stdout,output.stderr)
            assert len(result['attempts'])==1
            with sqlite3.connect(f.state/'dispatch.db') as db:
                assert db.execute("SELECT COUNT(*) FROM capacity_authorizations WHERE status='rejected'").fetchone()[0]>0
            # Restoring credentials alone must not renew the invalidated epoch.
            auth=f.root/'auth.json';value=json.loads(auth.read_text());value['email']='fixture@example.invalid';auth.write_text(json.dumps(value))
            output,_=run(f);assert output.returncode!=0 and f.count()==0
            no_lease(f)
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
        elif name=='grants':
            peer=add_peer(f,binary)
            path=f.state/'resources.yml';both=path.read_text()
            path.write_text(both.replace('harness: claude','harness: claude\n    enabled: false'))
            grant(f)
            path.write_text(both)
            c=f.client()
            accepted=c.call('submit',request_id='one',task='Add tests in src/lib.rs')
            result=finish(c,accepted['run_id'])['result'];assert result['allocation']['selected']['harness']=='codex'
            count=peer.count()+f.count()
            path.write_text(both.replace('model: codex-fixed','model: changed'))
            assert c.call('submit',request_id='one',task='Add tests in src/lib.rs')==accepted
            assert peer.count()+f.count()==count
            path.write_text(both);grant(f);broader=f.client()
            accepted=broader.call('submit',task='Add tests in src/lib.rs')
            result=finish(broader,accepted['run_id'])['result'];assert result['allocation']['selected']['harness']=='claude'
        elif name=='pools':
            peer=add_peer(f,binary)
            grant(f)
            f.mode.write_text('wait')
            ready=os.open(f.ready,os.O_RDONLY|os.O_NONBLOCK)
            c=f.client();a=c.call('submit',task='Add tests in src/lib.rs',model='fixture-model')
            assert select.select([ready],[],[],10)[0];assert os.read(ready,1)==b'R'
            grant(f);c2=f.client();queued=c2.call('submit',task='Add tests in src/lib.rs',model='fixture-model')
            # Durable admission state is the barrier, not elapsed sleep.
            deadline=time.monotonic()+10
            while True:
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    waiting=db.execute("SELECT COUNT(*) FROM admission_requests WHERE status='queued'").fetchone()[0]
                if waiting: break
                assert time.monotonic()<deadline
                time.sleep(.01)
            output,result=run(f,'--agent','codex');checked(output,result)
            assert f.count()==1 and peer.count()==1
            status=c2.call('status',run_id=queued['run_id'])
            c2.call('cancel',run_id=queued['run_id'],revision=status['state_revision'])
            with f.barrier.open('wb',buffering=0) as gate: gate.write(b'R')
            finish(c,a['run_id']);finish(c2,queued['run_id']);os.close(ready)
            no_lease(f)
        elif name=='capacity':
            peer=add_peer(f,binary)
            output,result=run(f);checked(output,result)
            pool=result['capacity']['pool_id']
            with sqlite3.connect(f.state/'dispatch.db') as db:
                obs=json.loads(db.execute('SELECT payload_json FROM capacity_observations WHERE pool_id=? ORDER BY sampled_at DESC LIMIT 1',(pool,)).fetchone()[0])
                obs['id']='phase6-exhausted';obs['sampled_at']=datetime.now(timezone.utc).isoformat();obs['scarcity']='exhausted'
                obs['constraints'][0].update(provider_bucket_id='codex',window_id='weekly',scope={'knowledge':'reported','value':'included_subscription'},remaining={'knowledge':'reported','value':0.0})
                obs['mapping']='mapped'
                db.execute('INSERT INTO capacity_observations(id,pool_id,source,source_version,sampled_at,valid_until,payload_json) VALUES(?,?,?,?,?,?,?)',
                    (obs['id'],pool,obs['source'],obs['source_version'],obs['sampled_at'],obs['valid_until'],json.dumps(obs)))
                constraint=obs['constraints'][0]
                db.execute('INSERT INTO capacity_observation_constraints VALUES(?,?,?,?,?)',(obs['id'],0,'codex','weekly',json.dumps(constraint)))
            output,result=run(f);checked(output,result)
            assert result['allocation']['selected']['harness']=='codex'
            assert f.count()==1 and peer.count()==1
            path=f.state/'resources.yml'
            path.write_text(path.read_text().replace('pool: claude-pool','pool: renamed-claude').replace('funding_source: claude-fixture','funding_source: renamed-funding').replace('provider_buckets: [codex]','provider_buckets: [codex, additional]',1))
            output,result=run(f);checked(output,result)
            assert result['allocation']['selected']['harness']=='codex' and f.count()==1
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
