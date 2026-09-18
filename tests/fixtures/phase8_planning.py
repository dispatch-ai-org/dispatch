#!/usr/bin/env python3
"""Disposable C project, synthetic provider protocols, actual spawn/check/DB evidence."""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import select
import signal
import shutil
import subprocess
import sys
sys.dont_write_bytecode = True
from phase5_control import Fixture, assert_error
from datetime import datetime, timezone, timedelta


def finish(client, run_id):
    # Semantic waits may time out before a multi-call goal. Follow committed state
    # only until the goal's original deadline, rather than treating a poll as completion.
    cursor=0
    while True:
        reply=client.call('await',run_id=run_id,after=cursor,predicate='execution_finished',timeout_ms=1000)
        if reply['reached']:return client.call('result',run_id=run_id)
        cursor=reply['cursor']
        status=client.call('status',run_id=run_id)
        deadline=datetime.fromisoformat(status['phase3']['deadline_at'].replace('Z','+00:00'))
        assert datetime.now(timezone.utc)<deadline,reply


def check_id(command):
    return 'check-' + hashlib.sha256(json.dumps(command, separators=(',', ':')).encode()).hexdigest()[:16]


def fixture(binary, mode='success'):
    f = Fixture(binary)
    f.mode.write_text(mode)
    for file in ['a.c', 'b.c', 'c.c', 'd.c']:
        (f.source / file).write_text('int '+file[0]+'(void) { return 1; }\n')
    (f.source/'tests').mkdir()
    (f.source/'tests/check.c').write_text('int a(void); int b(void); int c(void); int d(void);\nint main(void) { return a()+b()+c()+d() > 0 ? 0 : 1; }\n')
    (f.source/'verify.sh').write_text('''#!/bin/sh
set -eu
build=$(mktemp -d)
trap 'rm -rf "$build"' EXIT
cc -Wall -Wextra -Werror a.c b.c c.c d.c tests/check.c -o "$build/check"
"$build/check"
if [ "${1:-}" = root ] && [ -f root-failure ]; then exit 1; fi
''')
    commands = ['sh ./verify.sh', 'sh ./verify.sh root']
    task = lambda id, deps: dict(id=id, objective='Update '+id+'.c within its stated contract', read=[id+'.c','a.c','b.c'], write=[id+'.c'], acceptance=['The component returns a positive updated value'], checks=[check_id(commands[0])], prerequisites=deps, inputs=['Existing C component and checked prerequisites'], outputs=['Updated C component'])
    tasks = [task('a', []), task('b', ['a'])]
    if mode in ('dependency', 'assistance', 'bad_dependency', 'cycle_report', 'stale_report', 'self_report', 'cross_report', 'already_ready'):
        tasks[1]['prerequisites'] = []
    if mode in ('four', 'six', 'second_repair'):
        tasks += [task('c',['b']), task('d',['c'])]
    if mode=='fidelity':
        (f.source/'data.bin').write_bytes(bytes([0,255,1]))
        (f.source/'old.txt').write_text('rename this\n')
        (f.source/'run.sh').write_text('#!/bin/sh\nexit 0\n');(f.source/'run.sh').chmod(0o644)
        tasks[0]['write'] += ['data.bin','old.txt','renamed.txt','run.sh']
    if mode == 'duplicate': tasks[1]['id'] = 'a'
    if mode == 'cycle': tasks[0]['prerequisites'] = ['b']
    if mode == 'missing_check': tasks[0]['checks'] = ['unapproved']
    if mode == 'traversal': tasks[0]['write'] = ['../outside']
    if mode == 'symlink':
        (f.source/'link').symlink_to('tests')
        tasks[0]['write'] = ['link/new.c']
    if mode == 'conflict': tasks[1]['write'] = ['a.c']; tasks[1]['prerequisites'] = []
    if mode == 'oversized': tasks = tasks * 3
    (f.root/'plan.json').write_text(json.dumps({'dispatch_plan':dict(version=1,tasks=tasks)}))
    agent = f.root/f.provider
    agent.write_text('''#!/usr/bin/env python3
import json, pathlib, sys, os, re
root=pathlib.Path('''+repr(str(f.root))+''')
provider='''+repr(f.provider)+'''
if '--version' in sys.argv:
    print('claude phase6 fixture' if provider=='claude' else 'codex phase8 fixture');sys.exit()
if 'auth' in sys.argv and 'status' in sys.argv:
    print((root/'auth.json').read_text());sys.exit()
prompt=sys.argv[-1]
model=sys.argv[sys.argv.index('--model')+1] if '--model' in sys.argv else sys.argv[sys.argv.index('-m')+1]
mode=(root/'mode').read_text()
identity=re.search(r'Report identity: plan_revision=([^,]+), task_id=([^,]+), attempt_id=([^,]+), generation=1',prompt)
task=identity[2] if identity else 'direct'
active=root/'active'
fd=os.open(active,os.O_CREAT|os.O_EXCL|os.O_WRONLY);os.close(fd)
with (root/'invocations').open('a') as log: log.write(json.dumps(dict(task=task,model=model,attempt=identity[3] if identity else None,pid=os.getpid()))+'\\n')
calls=[json.loads(l) for l in (root/'invocations').read_text().splitlines()]
n=sum(c['task']==task for c in calls)
def emit(text):
    if provider=='claude':
        print(json.dumps({'type':'result','subtype':'success','is_error':False,'result':text,'modelUsage':{model:{'inputTokens':10,'outputTokens':4}},'usage':{'input_tokens':10,'output_tokens':4}}))
    else:
        print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':text}}))
        print(json.dumps({'type':'turn.completed','usage':{'input_tokens':10,'output_tokens':4}}))
try:
    if mode in ('hold_planner','hold_child','deadline') and task==('a' if mode=='hold_child' else 'planner'):
        with (root/'ready').open('wb',buffering=0) as ready:ready.write(b'R')
        with (root/'barrier').open('rb',buffering=0) as gate:gate.read(1)
    if mode=='fault_manifest' and task=='a':(pathlib.Path.cwd().parent/'artifact.json').mkdir()
    if task=='planner':
        if provider=='codex': assert sys.argv[sys.argv.index('--sandbox')+1]=='read-only'
        else: assert sys.argv[sys.argv.index('--tools')+1]=='Read,Glob,Grep'
        text=(root/'plan.json').read_text()
        if mode=='planner_edit': pathlib.Path('a.c').write_text('planner violation')
        if mode=='failed': emit(text);sys.exit(1)
        if mode in ('malformed','truncated'): text=text[:-7]
        if mode=='nonfinal': emit(text);text='not a plan'
        emit(text)
    elif task=='a' and n==1 and mode=='question':
        emit(json.dumps({'dispatch_checkpoint':{'version':1,'question':'Which positive value?','choices':['2','3'],'category':'factual'}}))
    elif n==1 and ((task=='a' and mode in ('dependency','assistance','bad_dependency','cycle_report','stale_report','self_report','cross_report')) or (task=='b' and mode=='already_ready')):
        target='a' if task=='b' else 'b'
        if mode=='bad_dependency': target='outside-goal'
        if mode=='self_report': target=task
        report=dict(version=1,plan_revision=identity[1],task_id=task,attempt_id=identity[3],generation=1,prerequisite=target,assistance=None)
        if mode=='stale_report':report['generation']=2
        if mode=='cross_report':report['plan_revision']='different-goal-revision'
        if mode=='assistance':report.update(prerequisite=None,assistance='Review the arithmetic boundary before implementing')
        emit(json.dumps({'dispatch_dependency':report}))
    elif task=='integration':
        if mode=='root_repair': pathlib.Path('root-failure').unlink()
        emit('Integration repaired')
    elif mode=='raylib_copy':
        if task=='a':
            file=pathlib.Path('main.c');body=file.read_text();assert 'const int ballRadiusX = 30;' in body;file.write_text(body.replace('const int ballRadiusX = 30;','const int ballRadiusX = 32;'))
        else:
            assert task=='b' and 'const int ballRadiusX = 32;' in pathlib.Path('main.c').read_text()
            pathlib.Path('PLANNING-FIXTURE.md').write_text('Disposable Phase 8 fixture: horizontal ball radius is 32 pixels. Build and collision/layout checks pass; visual behavior is not assessed.\\n')
        emit('Done')
    elif task=='direct':
        pathlib.Path('a.c').write_text('int a(void) { return 2; }\\n');emit('Done')
    else:
        if task=='b' and mode not in ('dependency','assistance','bad_dependency','cycle_report','stale_report','self_report','cross_report','already_ready'):assert 'return 2' in pathlib.Path('a.c').read_text()
        value=2 if task=='a' else 3
        if n==1 and (mode in ('repair','six','second_repair') and task=='a' or mode=='second_repair' and task=='b'):value=-100
        pathlib.Path(task+'.c').write_text('int '+task+'(void) { return '+str(value)+'; }\\n')
        if mode=='fidelity' and task=='a':
            pathlib.Path('data.bin').write_bytes(bytes([0,255,2,0]));pathlib.Path('old.txt').rename('renamed.txt');pathlib.Path('run.sh').chmod(0o755)
        if mode=='out_of_scope': pathlib.Path('outside.txt').write_text('violation')
        if mode=='weaken':pathlib.Path('tests/check.c').write_text('int main(void) { return 0; }\\n');pathlib.Path('a.c').write_text('int a(void) { return -100; }\\n')
        if mode in ('root_fail','root_repair') and task=='b':pathlib.Path('root-failure').write_text('root-only failure')
        emit('Done')
finally: active.unlink()
''')
    agent.chmod(0o755)
    if mode in ('root_fail','root_repair'): tasks[1]['write'].append('root-failure')
    if mode == 'weaken': tasks[0]['write'].append('tests/check.c')
    if mode == 'cycle_report': tasks[1]['prerequisites'] = ['a']
    (f.root/'plan.json').write_text(json.dumps({'dispatch_plan':dict(version=1,tasks=tasks)}))
    config = dict(execution=dict(timeout_secs=60),checks=dict(verify=commands),harnesses={f.provider:dict(executable=str(agent))},planning=dict(verification_paths=['verify.sh','tests'],routine=[dict(paths=['a.c'],checks=[check_id(commands[0])])]))
    (f.source/'dispatch.yml').write_text(json.dumps(config))
    # Existing provider proof stays synthetic; update executable identity after fixture editing.
    original=(f.state/'resources.yml').read_text()
    profile=original.split('profiles:\n')[1]
    if f.provider=='claude':
        prefix,raw=profile.split('    claude_subscription: ')
        evidence=json.loads(raw)
        evidence['executable_sha256']=hashlib.sha256(agent.read_bytes()).hexdigest()
        profile=prefix+'    claude_subscription: '+json.dumps(evidence)+'\n'
    profiles=''.join(profile.replace('fixture-model','fixture-'+tier).replace('tier: light','tier: '+tier) for tier in ['light','standard','strong'])
    (f.state/'resources.yml').write_text('version: 1\nallocation_enabled: true\ncapacity:\n  codex_probe: false\nprofiles:\n'+profiles)
    f.key=Path(f.command('control-grant',f.source,'--allow-unsafe-local','--delegate-factual','--allow-plan','--max-invocations','6').stdout.decode().strip())
    return f


def run(f, *extra):
    out=f.command('run',f.source,'--task','Update the independent C components and integrate their checked behavior','--plan','--allow-unsafe-local','--json',*extra,check=False)
    assert out.stdout, out.stderr.decode()
    result=json.loads(out.stdout)
    stored=f.stored(result['run_id'])
    assert (f.source/'a.c').read_text()=='int a(void) { return 1; }\n'
    with sqlite3.connect(f.state/'dispatch.db') as db:
        leases=db.execute('SELECT COUNT(*) FROM pool_leases').fetchone()[0]
        assert leases==(1 if f.mode.read_text()=='uncertain_cleanup' else 0)
        launched=db.execute("SELECT COUNT(*) FROM planned_invocations WHERE knowledge IN ('cleanup_confirmed','child_recorded','spawn_may_have_occurred','cleanup_uncertain')").fetchone()[0]
        assert launched==f.count(),(launched,f.count())
        assert db.execute('SELECT COUNT(*) FROM sync_outbox').fetchone()[0]==0
    return stored


def scenario(binary, name):
    mode={'budget':'success','no_retry':'repair','control':'success','direct':'success','grant':'success','tight':'success','override':'success','cancel':'hold_child','disconnect':'hold_planner','crash':'hold_child','pty_plain':'success','pty_narrow':'success','pty_wide':'success','drift':'success','tamper':'success','fault_plan':'success','fault_integration':'success','fault_delivery':'success','fault_completion':'success','fault_fence':'success','uncertain_cleanup':'uncertain_cleanup','mixed':'success','pinned':'success','planning_false':'success','no_checks':'success','no_planner':'success','tight_repair':'repair','private_attribution':'success','manifest_tamper':'success','question_deadline':'question','pty_question_deadline':'question','question_no_retry':'question'}.get(name,name)
    f=fixture(binary,mode)
    try:
        if name=='raylib_copy':
            live=Path(os.environ['DISPATCH_RAYLIB_SOURCE']).resolve()
            def tree_digest(root):
                h=hashlib.sha256()
                for file in sorted(root.rglob('*')):
                    h.update(str(file.relative_to(root)).encode());h.update(str(file.lstat().st_mode).encode())
                    if file.is_symlink():h.update(os.readlink(file).encode())
                    elif file.is_file():h.update(file.read_bytes())
                return h.hexdigest()
            before=tree_digest(live)
            config=json.loads((f.source/'dispatch.yml').read_text())
            shutil.rmtree(f.source);shutil.copytree(live,f.source,symlinks=True)
            command='sh ./verify.sh';ref=check_id(command)
            config['checks']['verify']=[command]
            config['planning']=dict(verification_paths=['verify.sh','tests'],routine=[dict(paths=['main.c'],checks=[ref])])
            (f.source/'dispatch.yml').write_text(json.dumps(config))
            tasks=[dict(id='a',objective='Set the horizontal ball radius to 32 pixels',read=['main.c'],write=['main.c'],acceptance=['Horizontal radius is 32; original build and collision/layout checks pass'],checks=[ref],prerequisites=[],inputs=['Original C app'],outputs=['Updated main.c']),dict(id='b',objective='Document the exact checked radius change in PLANNING-FIXTURE.md',read=['main.c'],write=['PLANNING-FIXTURE.md'],acceptance=['Documentation matches the checked prerequisite'],checks=[ref],prerequisites=['a'],inputs=['Checked main.c from a'],outputs=['PLANNING-FIXTURE.md'])]
            (f.root/'plan.json').write_text(json.dumps(dict(dispatch_plan=dict(version=1,tasks=tasks))))
            goal='Set horizontal ball radius to 32 pixels, then document the exact checked change in PLANNING-FIXTURE.md.'
            out=f.command('run',f.source,'--task',goal,'--plan','--allow-unsafe-local','--json','--max-invocations','4','--timeout','60')
            result=json.loads(out.stdout);stored=f.stored(result['run_id'])
            assert stored['outcome']['work_result']=='ready' and stored['outcome']['verification']=='passed',stored
            assert stored['outcome']['review']=='pending' and stored['outcome']['application']=='not_applied'
            assert f.count()==3 and [a['resolved_model'] for a in stored['attempts']]==['fixture-strong','fixture-light','fixture-standard']
            assert 'const int ballRadiusX = 30;' in (f.source/'main.c').read_text()
            assert tree_digest(live)==before
            for check in stored['phase3']['planning']['root_checks']:
                print(Path(check['stdout_path']).read_text().strip())
            print('raylib_copy: PASS; original source SHA256='+before+'; supervised invocations=3; final review pending; original checks unchanged')
            return
        if name=='mixed':
            claude=f.root/'claude'
            claude.write_text((f.root/'codex').read_text().replace("provider='codex'", "provider='claude'"))
            claude.chmod(0o755)
            (f.root/'auth.json').write_text(json.dumps(dict(loggedIn=True,authMethod='claude.ai',apiProvider='firstParty',email='fixture@example.invalid',orgId='fixture-org')))
            resources=f.state/'resources.yml'
            prefix,strong=resources.read_text().rsplit('  - provider: openai',1)
            evidence=dict(contract_version=1,cli_version='claude phase6 fixture',executable_sha256=hashlib.sha256(claude.read_bytes()).hexdigest(),account_sha256=hashlib.sha256(json.dumps(['fixture@example.invalid','fixture-org'],separators=(',',':')).encode()).hexdigest(),checked_at=datetime.now(timezone.utc).isoformat(),valid_until=(datetime.now(timezone.utc)+timedelta(hours=1)).isoformat(),print_mode_included=True,usage_credits_disabled=True,unmanaged_account=True)
            resources.write_text(prefix+'  - provider: anthropic'+strong.replace('funding_source: chatgpt-plus','funding_source: claude-fixture').replace('harness: codex','harness: claude').replace('provider_buckets: [codex]','provider_buckets: [claude]').replace('pool: shared','pool: claude-fixture')+'    claude_subscription: '+json.dumps(evidence)+'\n')
            config=json.loads((f.source/'dispatch.yml').read_text());config['harnesses']['claude']=dict(executable=str(claude));(f.source/'dispatch.yml').write_text(json.dumps(config))
        if name in ('no_checks','no_planner'):
            if name=='no_checks':
                config=json.loads((f.source/'dispatch.yml').read_text());config['checks']['verify']=[];(f.source/'dispatch.yml').write_text(json.dumps(config))
            else:
                resources=f.state/'resources.yml';resources.write_text(resources.read_text().rsplit('  - provider: openai',1)[0])
            out=f.command('run',f.source,'--task','Goal','--plan','--allow-unsafe-local','--json',check=False)
            assert out.returncode!=0 and f.count()==0,out
            return
        if name=='planning_false':
            f.key=Path(f.command('control-grant',f.source,'--allow-unsafe-local').stdout.decode().strip())
            c=f.client();submitted=c.call('submit',request_id='direct-once',task='Update a.c',plan=False)
            assert c.call('submit',request_id='direct-once',task='Update a.c')==submitted
            finish(c,submitted['run_id']);assert 'planning' not in f.stored(submitted['run_id'])['phase3'] and f.count()==1
            return
        if name=='question_no_retry':
            stored=run(f,'--no-retry');q=stored['phase3']['questions'][-1]
            out=f.command('answer',stored['id'],q['id'],'--revision',q['revision'],'--generation',q['generation'],'--answer','2','--json')
            result=json.loads(out.stdout);final=f.stored(result['run_id'])
            assert f.count()==4 and final['outcome']['work_result']=='ready'
            assert final['phase3']['deadline_at']==stored['phase3']['deadline_at'] and final['phase3']['no_retry']
            return
        if name=='question_deadline':
            f.key=Path(f.command('control-grant',f.source,'--allow-unsafe-local','--allow-plan','--delegate-factual','--max-invocations','4','--timeout','5').stdout.decode().strip())
            c=f.client();submitted=c.call('submit',task='Goal',plan=True)
            attention=c.call('await',run_id=submitted['run_id'],after=0,predicate='attention_required',timeout_ms=5000)
            assert attention['reached'],attention
            s=c.call('status',run_id=submitted['run_id']);q=s['phase3']['questions'][-1]
            done=c.call('await',run_id=submitted['run_id'],after=s['cursor'],predicate='execution_finished',timeout_ms=10000)
            assert done['reached'],done
            s=f.stored(submitted['run_id']);assert s['phase3']['failure']=='deadline' and f.count()==2,s
            assert not c.receive(c.send('answer',run_id=s['id'],question_id=q['id'],revision=q['revision'],generation=q['generation'],answer='2'))['ok']
            return
        if name.startswith('fault_') and name != 'fault_manifest' or name=='uncertain_cleanup':
            targets={'fault_plan':"NEW.event_type='plan.validated'",'fault_integration':"NEW.event_type='task.integrated'",'fault_delivery':"NEW.event_type='run.finished'",'fault_completion':"NEW.event_type='attempt.finished'"}
            with sqlite3.connect(f.state/'dispatch.db') as db:
                if name=='fault_fence':db.execute("CREATE TRIGGER fault BEFORE INSERT ON planned_invocations BEGIN SELECT RAISE(ABORT,'fixture launch-fence write failure'); END")
                elif name=='uncertain_cleanup':db.execute("CREATE TRIGGER fault BEFORE UPDATE OF launch_lifecycle ON pool_leases WHEN NEW.launch_lifecycle='cleanup_confirmed' BEGIN SELECT RAISE(ABORT,'fixture cleanup publication failure'); END")
                else:db.execute("CREATE TRIGGER fault BEFORE INSERT ON events WHEN "+targets[name]+" BEGIN SELECT RAISE(ABORT,'fixture event publication failure'); END")
        if name in ('cancel','disconnect','crash','deadline'):
            ready=os.open(f.ready,os.O_RDONLY|os.O_NONBLOCK)
            if name=='deadline':
                cmd=[f.binary,'--state-dir',str(f.state),'run',str(f.source),'--task','Goal','--plan','--allow-unsafe-local','--json','--timeout','5']
                worker=subprocess.Popen(cmd,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
                assert select.select([ready],[],[],10)[0]
                os.read(ready,1);os.close(ready)
                out,err=worker.communicate(timeout=10)
                result=json.loads(out)
                assert result['phase3']['failure']=='deadline',(result,err)
                assert f.count()==1
                return
            c=f.client()
            submitted=c.call('submit',task='Goal',plan=True)
            assert select.select([ready],[],[],10)[0]
            os.read(ready,1);os.close(ready)
            if name=='cancel':
                current=c.call('status',run_id=submitted['run_id'])
                waiting=c.send('await',run_id=submitted['run_id'],after=current['cursor'],predicate='execution_finished',timeout_ms=10000)
                c.call('cancel',run_id=submitted['run_id'],revision=current['state_revision'])
                assert c.receive(waiting)['ok']
            elif name=='disconnect':c.close()
            else:
                # Kill only this fixture's foreground owner. Reconciliation must not replay.
                c.process.kill();c.process.wait(timeout=5)
                with f.barrier.open('wb',buffering=0) as gate:gate.write(b'R')
            stored=f.stored(submitted['run_id'])
            assert f.count()<=2
            before=f.count()
            f.command('status',submitted['run_id'],'--json',check=False)
            if name!='disconnect':c.close()
            observer=f.client()
            if name=='crash':
                reply=observer.receive(observer.send('recover',run_id=submitted['run_id'],revision=stored['state_revision']))
                assert not reply['ok'],reply
            else:assert stored['outcome']['work_result'] in ('cancelled','failed'),stored['outcome']
            assert f.count()==before
            return
        if name.startswith('pty_'):
            from review_refinement import Session,has_color
            if name=='pty_question_deadline':
                config=json.loads((f.source/'dispatch.yml').read_text());config['execution']['timeout_secs']=5;(f.source/'dispatch.yml').write_text(json.dumps(config))
            args=[f.binary,'--state-dir',str(f.state),'--no-color','--ascii']
            if name=='pty_plain':args+=['--plain']
            captures=os.environ.get('DISPATCH_REVIEW_CAPTURES',str(f.root/'captures'))
            with Session(args,f.source,captures,name,width=38 if name=='pty_narrow' else 100,height=32) as ui:
                ui.wait('accomplish?');ui.mark('startup')
                ui.send('/plan Update the C components and their dependency\r')
                ui.wait('[y/N]');ui.send('y\r')
                if name=='pty_question_deadline':
                    ui.wait('Your answer');ui.mark('question')
                    ui.wait('[n] next goal');ui.mark('deadline');ui.send('n\r');ui.wait('accomplish?');ui.send(b'\x04');ui.finish()
                    with sqlite3.connect(f.state/'dispatch.db') as db:stored=json.loads(db.execute('SELECT run_projection_json FROM runs').fetchone()[0])
                    assert stored['phase3']['failure']=='deadline' and f.count()==2,stored
                    return
                ui.wait('Review changes');ui.mark('review')
                assert 'sequential' in ui.clean and f.count()==3
                if name=='pty_plain':assert ui.clean.count('Ready for review')==1,ui.clean
                ui.send('i\r');ui.wait('[Enter] Back to review' if name=='pty_plain' else 'Esc back');ui.mark('details');ui.send('\r' if name=='pty_plain' else 'q')
                ui.wait('Review changes');ui.send('d\r');ui.wait('[q] back' if name=='pty_plain' else 'q back');ui.mark('diff');ui.send('q\r' if name=='pty_plain' else 'q')
                ui.wait('Review changes');ui.send('n\r');ui.wait('accomplish?');ui.send(b'\x04');ui.finish()
                assert not has_color(bytes(ui.output))
            return
        if name in ('control','question','grant'):
            if name=='grant':
                f.key=Path(f.command('control-grant',f.source,'--allow-unsafe-local').stdout.decode().strip())
                c=f.client()
                assert_error(c,'submit','authorization_required',task='Goal',plan=True)
                assert f.count()==0
                return
            c=f.client()
            submit=c.call('submit',request_id='planned-once',task='Update C components',plan=True)
            assert c.call('submit',request_id='planned-once',task='Update C components',plan=True)==submit
            if name=='question':
                attention=c.call('await',run_id=submit['run_id'],after=0,predicate='attention_required',timeout_ms=10000)
                assert attention['reached'], attention
                s=c.call('status',run_id=submit['run_id'])
                q=s['phase3']['questions'][-1]
                with sqlite3.connect(f.state/'dispatch.db') as db:assert db.execute('SELECT COUNT(*) FROM pool_leases').fetchone()[0]==0
                wait=c.send('await',run_id=submit['run_id'],after=s['cursor'],predicate='execution_finished',timeout_ms=10000)
                fields=dict(run_id=submit['run_id'],question_id=q['id'],revision=q['revision'],generation=q['generation'],answer='2')
                answer=c.call('answer',request_id='answer-once',**fields)
                assert c.call('answer',request_id='answer-once',**fields)==answer
                assert c.receive(wait)['ok']
            finish(c,submit['run_id'])
            stored=f.stored(submit['run_id'])
            assert stored['outcome']['work_result']=='ready',stored['phase3']['planning']['error']
            assert f.count()==(4 if name=='question' else 3)
            final=c.call('result',run_id=submit['run_id'])
            ref=next(a for a in final['artifacts'] if a['kind']=='final_diff')
            patch=c.call('artifact',**ref)['text']
            assert 'a.c' in patch and 'b.c' in patch
            c.close()
            c=f.client();assert c.call('submit',request_id='planned-once',task='Update C components',plan=True)==submit
        elif name=='direct':
            out=f.command('run',f.source,'--task','Update a.c','--allow-unsafe-local','--json')
            result=json.loads(out.stdout)
            assert 'planning' not in result['phase3'] and f.count()==1
        elif name=='override':
            out=f.command('run',f.source,'--task','Goal','--plan','--allow-unsafe-local','--model','fixture-light','--json',check=False)
            assert out.returncode!=0 and f.count()==0
        else:
            extra=['--max-invocations','2'] if name=='tight' else ['--max-invocations','3'] if name in ('budget','tight_repair') else ['--no-retry'] if name=='no_retry' else ['--model','fixture-strong'] if name=='pinned' else []
            stored=run(f,*extra)
            p=stored['phase3']['planning']
            success=name in ('success','budget','four','six','repair','dependency','assistance','already_ready','root_repair','drift','tamper','mixed','pinned','private_attribution','manifest_tamper','fidelity')
            assert (stored['outcome']['work_result']=='ready')==success,(name,p['error'],stored['outcome'])
            if success:
                assert stored['outcome']['verification']=='passed' and stored['outcome']['review']=='pending'
                assert len(stored['candidates'])==1
                assert all(t['state']=='integrated' for t in p['tasks'])
                assert len(p['snapshots'])==len(p['tasks'])+1+(name=='root_repair')
                assert len(p['artifacts'])==len(p['tasks'])+(name=='root_repair')
                calls=[json.loads(l) for l in f.invocations.read_text().splitlines()]
                assert calls[0]['model']=='fixture-strong'
                if name=='pinned':assert all(c['model']=='fixture-strong' for c in calls)
                elif name!='fidelity':
                    assert any(c['model']=='fixture-light' for c in calls[1:])
                    assert any(c['model']=='fixture-standard' for c in calls[1:])
                if name=='mixed':assert {a['harness_id'] for a in stored['attempts']}=={'codex','claude'}
                assert len(calls)<=len(p['tasks'])+2
                patch=Path(stored['candidates'][0]['diff_path']).read_text()
                assert 'a.c' in patch and 'b.c' in patch
                if name=='drift':
                    (f.source/'a.c').write_text('owner changed source\n')
                    assert f.command('accept',stored['id'],check=False).returncode!=0
                    assert (f.source/'a.c').read_text()=='owner changed source\n'
                    return
                if name in ('tamper','manifest_tamper'):
                    Path(p['artifacts'][0]['manifest'] if name=='manifest_tamper' else stored['candidates'][0]['diff_path']).write_text('tampered')
                    assert f.command('accept',stored['id'],check=False).returncode!=0
                    return
                f.command('accept',stored['id'])
                assert 'return 2' in (f.source/'a.c').read_text()
                if name=='fidelity':
                    assert (f.source/'data.bin').read_bytes()==bytes([0,255,2,0])
                    assert not (f.source/'old.txt').exists() and (f.source/'renamed.txt').read_text()=='rename this\n'
                    assert (f.source/'run.sh').stat().st_mode & 0o777 == 0o755
                if name=='private_attribution':
                    from phase7_evidence import owner
                    code,transcript=owner(f,'evidence','annotate',stored['id'],'--origin','scripted_smoke','--review','scripted');assert code==0,transcript
                    report=json.loads(f.command('evidence','private',stored['id']).stdout)['current']
                    assert report['counts']['policy_chains']==1 and report['counts']['reviewed']==0 and report['counts']['eligible_executions']==0,report
                    assert report['counts']['continuations']==0 and report['counts']['launched']==3,report
                    assert report['economics']['scripted_smoke']['harness_ms']['count']==3,report
                    assert report['economics']['scripted_smoke']['verification_ms']['count']==6,report
                    with sqlite3.connect(f.state/'dispatch.db') as db:
                        assert db.execute('SELECT COUNT(*) FROM goal_feedback_revisions').fetchone()[0]==1
                        assert db.execute('SELECT COUNT(*) FROM routing_feedback_events').fetchone()[0]==0
                        assert db.execute('SELECT COUNT(*) FROM sync_outbox').fetchone()[0]==0
                        assert db.execute('SELECT COUNT(*) FROM private_policy_transitions').fetchone()[0]==0
            else:
                assert stored['candidates']==[] and stored['outcome']['review']=='not_requested'
                assert f.count()<=6
                if name in ('malformed','truncated','nonfinal','failed','planner_edit','duplicate','cycle','missing_check','traversal','conflict','oversized','tight','symlink'):assert f.count()==1,(name,f.count())
                if name in ('no_retry','tight_repair'):assert f.count()==2
                if name=='second_repair':assert f.count()==4
        print(name+': PASS; actual supervised invocations='+str(f.count()))
    finally:
        if sys.exc_info()[0]:
            for m in (f.state/'runs').glob('*/metadata.json'):
                r=json.loads(m.read_text());print('planning diagnostic:',r.get('phase3',{}).get('planning',{}).get('error'),file=sys.stderr)
        f.cleanup()


if __name__=='__main__':
    for name in sys.argv[2:] or ['success','second_repair']:
        scenario(sys.argv[1],name)
