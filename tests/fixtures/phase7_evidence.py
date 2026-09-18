#!/usr/bin/env python3
"""Explicitly synthetic, disposable-state Phase 7 smoke. No real models or human labels.
Seeds retained projections/feedback for analytics tests; actual fixture calls are counted.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import errno
import sqlite3
import sys
import time
from datetime import datetime, timedelta, timezone
sys.dont_write_bytecode = True
from phase5_control import Fixture, finish, assert_error


def stamp():
    return datetime.now(timezone.utc).isoformat()


def command(f, *args):
    p=f.command(*args)
    return json.loads(p.stdout)


def run(f, *args):
    return command(f,'run',f.source,'--task','Add tests in src/lib.rs','--allow-unsafe-local','--json','--no-retry',*args)


def owner_process(argv, timeout=15):
    """Bounded PTY command; only the forked child's acknowledged group is signalled."""
    ready_read, ready_write = os.pipe()
    try:
        pid, fd = pty.fork()
    except BaseException:
        os.close(ready_read)
        os.close(ready_write)
        raise
    if pid == 0:
        try:
            os.close(ready_read)
            # forkpty establishes a new session; prove ownership before group cleanup.
            assert os.getpgrp() == os.getpid() == os.getsid(0)
            os.write(ready_write, str(os.getpgrp()).encode())
            os.close(ready_write)
            os.execv(argv[0], argv)
        finally:
            os._exit(127)
    os.close(ready_write)
    data = b''
    status = None
    group_owned = False
    deadline = time.monotonic() + timeout

    def poll():
        nonlocal status
        if status is None:
            done, value = os.waitpid(pid, os.WNOHANG)
            if done:
                status = value
        return status

    def cleanup():
        # Keep our child unreaped until the last group signal: its PID cannot be reused.
        if status is not None:
            return
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                if group_owned:
                    os.killpg(pid, sig)
                else:
                    os.kill(pid, sig)
            except ProcessLookupError:
                pass
            if sig == signal.SIGTERM:
                time.sleep(0.2)
        until = time.monotonic() + 2
        while poll() is None and time.monotonic() < until:
            time.sleep(0.02)
        if status is None:
            raise RuntimeError(f'cleanup could not reap owned child {pid}')

    try:
        if not select.select([ready_read], [], [], max(0, deadline-time.monotonic()))[0]:
            raise RuntimeError('timeout establishing owned process group')
        group_owned = os.read(ready_read, 64) == str(pid).encode()
        if not group_owned:
            raise RuntimeError('child did not establish owned process group')
        answered = False
        while time.monotonic() < deadline:
            if select.select([fd], [], [], min(0.1, max(0, deadline-time.monotonic())))[0]:
                try:
                    chunk = os.read(fd, 65536)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    chunk = b''
                if not chunk:
                    # EOF can precede waitpid readiness by a scheduler turn.
                    until = min(deadline, time.monotonic()+0.3)
                    while poll() is None and time.monotonic() < until:
                        time.sleep(0.01)
                    if status is None:
                        raise RuntimeError('unexpected terminal closure while child is running')
                    return os.waitstatus_to_exitcode(status), data
                data = (data + chunk)[-65536:]
                challenge = re.search(rb'Type ([0-9A-Z]{26}) to attest', data)
                if challenge and not answered:
                    os.write(fd, challenge[1]+b'\n')
                    answered = True
            if poll() is not None:
                return os.waitstatus_to_exitcode(status), data
        raise RuntimeError('timeout waiting for owner command')
    except Exception as error:
        raise RuntimeError(f'{error}; owned child {pid}; terminal tail: {data[-4096:]!r}') from error
    finally:
        try:
            try:
                cleanup()
            except Exception as error:
                raise RuntimeError(f'owner cleanup failed for {pid}: {error}; terminal tail: {data[-4096:]!r}') from error
        finally:
            try:
                os.close(ready_read)
            finally:
                os.close(fd)


def owner(f, *args):
    # Scripted approval of explicitly synthetic state, never evidence of human judgment.
    return owner_process([f.binary, '--state-dir', str(f.state), *map(str, args)])


def fixture(binary,domain=True):
    os.environ['DISPATCH_FIXTURE_PROVIDER']='codex'
    f=Fixture(binary)
    (f.source/"Cargo.toml").write_text('[package]\nname="synthetic"\nversion="0.0.0"\n')
    (f.source/"src/other.rs").write_text("// explicit synthetic language fixture\n")
    if domain:
        with sqlite3.connect(f.state/'dispatch.db') as db:
            db.execute('INSERT INTO private_fixture_domain VALUES (1)')
    path=f.source/'dispatch.yml'
    path.write_text(path.read_text()+'''private_evidence:
  shadow: false
  routine_mappings:
    - id: rust-tests
      features: {language: rust, task_kind: tests, scope: localized}
      verify: ['test -f result.txt']
''')
    path=f.state/'resources.yml'
    profile=path.read_text().split('profiles:\n')[1]
    path.write_text(path.read_text()+profile.replace('fixture-model','fixture-standard').replace('tier: light','tier: standard'))
    agent=f.root/'codex'
    agent.write_text(agent.read_text().replace("prompt=sys.stdin.read()", "prompt=sys.stdin.read()\nwith (root/'argv.jsonl').open('a') as log: log.write(json.dumps(sys.argv[1:])+'\\n')"))
    light=run(f);strong=run(f,'--model','fixture-standard')
    assert light['allocation']['selected']['tier']=='light'
    assert strong['allocation']['selected']['tier']=='standard'
    return f,light,strong


def seed(f,template,index,outcome='accepted',origin='synthetic',review='synthetic',reasons=None,mutate=None):
    original=f.stored(template['run_id']);run=copy.deepcopy(original)
    ident=f'SYNTHETIC-{index:06d}'
    run['id']=ident
    run['created_at']=stamp();run['completed_at']=stamp()
    # Every clone is explicitly synthetic; copies are measurements in fixture state only.
    run['task']='SYNTHETIC Phase 7 evidence fixture'
    if outcome: run['outcome']['review']=outcome
    if mutate: mutate(run)
    with sqlite3.connect(f.state/'dispatch.db') as db:
        cols=[r[1] for r in db.execute('PRAGMA table_info(runs)')]
        original_row=list(db.execute('SELECT * FROM runs WHERE id=?',(template['run_id'],)).fetchone())
        for key,value in {'id':ident,'task':run['task'],'created_at':run['created_at'],'completed_at':run['completed_at'],'run_projection_json':json.dumps(run)}.items():
            original_row[cols.index(key)]=value
        db.execute(f"INSERT INTO runs ({','.join(cols)}) VALUES ({','.join('?' for _ in cols)})",original_row)
        feedback=None
        if outcome:
            feedback=f'goal-feedback-{ident}-1'
            db.execute('INSERT INTO goal_feedback_revisions VALUES (?,?,?,?,?,?,?)',(feedback,ident,1,outcome,json.dumps(reasons or []),None,stamp()))
    # Use the same canonical digest algorithm as Rust, including exact delivery fields.
    payload={'final':run['phase3']['final_attempt_id'],'contributors':run['phase3']['contributing_attempts'],
        'baseline':run['baseline_commit'],'candidates':[[c['id'],c['diff_stats'],c['checks']] for c in run['candidates']]}
    delivery=hashlib.sha256(json.dumps(payload,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
    annotation={'origin':origin,'review':review,'delivery':delivery,'actor':'synthetic-fixture','repair_minutes':None}
    with sqlite3.connect(f.state/'dispatch.db') as db:
        db.execute('INSERT INTO private_annotations(run_id,feedback_id,payload_json,created_at) VALUES (?,?,?,?)',(ident,feedback,json.dumps(annotation),stamp()))
    return ident


def reviewed(f,light,strong):
    ids=[seed(f,light,i,'rejected' if i<5 else 'accepted',reasons=['correctness']) for i in range(20)]
    ids += [seed(f,light,i,None) for i in range(20,25)]
    seed(f,strong,25,None)
    return ids


def summary(f,run_id):
    return command(f,'evidence','private',run_id)['current']


def proposal(f,run_id):
    return command(f,'evidence','propose',run_id)


def shadow(f,enabled=True):
    path=f.source/'dispatch.yml'
    path.write_text(path.read_text().replace('shadow: false','shadow: true') if enabled else path.read_text().replace('shadow: true','shadow: false'))


def c_cohort(binary):
    os.environ['DISPATCH_FIXTURE_PROVIDER']='codex'
    f=Fixture(binary)
    try:
        with sqlite3.connect(f.state/'dispatch.db') as db:
            db.execute('INSERT INTO private_fixture_domain VALUES (1)')
        (f.source/'src/lib.rs').rename(f.source/'main.c')
        (f.source/'src').rmdir()
        agent=f.root/'codex'
        agent.write_text(agent.read_text().replace('src/lib.rs','main.c'))
        resources=f.state/'resources.yml'
        resources.write_text(resources.read_text().replace('tier: light','tier: standard'))
        def feature(task):
            return command(f,'run',f.source,'--task',task,'--model','fixture-model',
                           '--allow-unsafe-local','--json','--no-retry')
        old=feature('Add support for a platform in main.c')
        frozen=copy.deepcopy(old['allocation']['private_evidence'])
        assert frozen['context']['features']['language'] is None,frozen
        (f.source/'tests').mkdir()
        (f.source/'tests/collision_test.c').write_text('// explicitly synthetic C fixture\n')
        (f.source/'verify.sh').write_text('#!/bin/sh\nset -eu\ntest -f result.txt\n')
        config=f.source/'dispatch.yml'
        config.write_text(config.read_text().replace("['test -f result.txt']","['sh ./verify.sh']")+'''private_evidence:
  shadow: true
  routine_mappings:
    - id: c-platform-features
      features: {language: c, task_kind: feature, scope: multi_file}
      verify: ['sh ./verify.sh']
''')
        new=feature('Add support for a second platform in main.c and tests/collision_test.c')
        decision=new['allocation']['private_evidence']
        assert decision['context']['features']=={'language':'c','task_kind':'feature','scope':'multi_file'},decision
        mapping={'id':'c-platform-features','features':{'language':'c','task_kind':'feature','scope':'multi_file'},'verify':['sh ./verify.sh']}
        expected=hashlib.sha256(json.dumps(mapping,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        assert decision['context']['mapping']==expected,decision
        assert decision['reason']=='task_scope_outside_trial_rule',decision
        assert new['outcome']['verification']=='passed',new
        seed(f,new,9000)
        current=summary(f,new['run_id'])
        assert current['counts']['eligible_executions']==1,current
        cohort=next(iter(current['resources'].values()))
        assert (cohort['reviewed'],cohort['accepted_verified'])==(1,1),cohort
        assert proposal(f,new['run_id'])['reason']=='task_scope_outside_trial_rule'
        historical=command(f,'evidence','private',old['run_id'])
        assert historical['at_decision_time']==frozen,historical
        assert historical['current']['context']['features']['language'] is None,historical
        assert historical['current']['context']['mapping'] is None,historical
        with sqlite3.connect(f.state/'dispatch.db') as db:
            assert db.execute('SELECT count(*) FROM private_policy_transitions').fetchone()[0]==0
        assert f.count()==2
        print('PASS synthetic C feature cohort; frozen unknown history; localized-only trials; two fixture invocations, no activation')
    finally:
        f.cleanup()


def scenario(binary,name):
    if name=='c_cohort':
        return c_cohort(binary)
    f,light,strong=fixture(binary,domain=name!='ordinary')
    try:
        if name=='smoke':
            reviewed(f,light,strong)
            s=summary(f,light['run_id'])
            cs=[v for v in s['resources'].values() if v['goals']==25][0]
            assert (cs['reviewed'],cs['accepted'],cs['rejected'],cs['missing_review'])==(20,15,5,5),s
            assert cs['launched']==cs['attempts']==25 and s['counts']['quality_rejected']==5
            off=run(f);shadow(f);on=run(f)
            assert off['allocation']['selected']==on['allocation']['selected']
            assert off['outcome']==on['outcome'] and len(off['attempts'])==len(on['attempts'])==1
            d=on['allocation']['private_evidence']
            assert d['mode']=='shadow' and d['proposed_choice']['resolved_model']=='fixture-standard',d
            assert f.count()==4
            argv=[json.loads(line) for line in (f.root/'argv.jsonl').read_text().splitlines()]
            assert argv[2]==argv[3],(argv[2],argv[3])
            for result in (off,on):
                stored=f.stored(result['run_id'])
                assert stored['environment']['timeout_secs']==30 and stored['phase3']['max_invocations']==2
                assert stored['admission']['state']=='released'
                assert stored['phase3']['no_retry']
            pid=d['proposal_id']
            p=command(f,'evidence','policy',pid)
            assert p['status']=='proposed' and p['currently_valid']
            frozen=copy.deepcopy(d)
            # A grant made before activation must not expand, but old receipts/results survive.
            f.key=Path(f.command('control-grant',f.source,'--allow-unsafe-local').stdout.decode().strip())
            code,log=owner(f,'evidence','activate',f.source,pid,'--expected-revision','0')
            assert code==0,log.decode()
            future=run(f)
            assert future['allocation']['selected']['resolved_model']=='fixture-standard',future
            assert future['allocation']['private_evidence']['mode']=='promoted'
            assert command(f,'evidence','private',on['run_id'])['at_decision_time']==frozen
            c=f.client();assert_error(c,'submit','authorization_required',task='Add tests in src/lib.rs');c.close()
            assert f.count()==5
            code,log=owner(f,'evidence','rollback',f.source,'--expected-revision','1');assert code==0,log.decode()
            restored=run(f)
            assert restored['allocation']['selected']==off['allocation']['selected'] and f.count()==6
            code,log=owner(f,'evidence','activate',f.source,pid,'--expected-revision','2');assert code!=0
            assert command(f,'evidence','policy',pid)['status']=='revoked'
            with sqlite3.connect(f.state/'dispatch.db') as db:
                assert db.execute('SELECT COUNT(*) FROM goal_feedback_revisions').fetchone()[0]==20
                assert db.execute('SELECT COUNT(*) FROM pool_leases').fetchone()[0]==0
                assert db.execute('SELECT COUNT(*) FROM sync_outbox').fetchone()[0]==0
                assert db.execute('SELECT COUNT(*) FROM private_policy_transitions').fetchone()[0]==2
            print('PASS synthetic fixture smoke: 20 reviewed / 25 comparable light goals; 6 actual fixture invocations; shadow unchanged; explicit activation; rollback; no human labels')
        elif name=='outcomes':
            seed(f,light,0,None)
            seed(f,light,1,'accepted',mutate=lambda r:r['outcome'].update(verification='not_configured'))
            seed(f,light,2,'accepted',mutate=lambda r:r['outcome'].update(application='blocked_by_source_drift'))
            seed(f,light,3,'rejected',reasons=['changed-requirements'])
            seed(f,light,4,'rejected',reasons=['other'])
            def failed(r):
                r['outcome'].update(verification='failed',review='pending')
                r['attempts'][0]['detail']['failure']='target_verification'
                for candidate in (r['candidates'][0],r['attempts'][0]['detail']['result']):
                    candidate['checks'][0].update(status='failed',exit_code=1)
            seed(f,light,5,None,mutate=failed)
            def chain(r):
                a=copy.deepcopy(r['attempts'][0]);a['id']='synthetic-recovery';a['detail']['parent_attempt_id']=r['attempts'][0]['id'];r['attempts'].append(a)
                r['phase3']['contributing_attempts'].append(a['id'])
            seed(f,light,6,'accepted',mutate=chain)
            def op(r):
                r['outcome'].update(verification='not_run',review='not_requested',work_result='failed')
                r['attempts'][0]['detail']['failure']='authorization';r['attempts'][0]['observed_model']=None
                r['attempts'][0]['detail']['result']['checks']=[]
                r['candidates'][0]['checks']=[]
            seed(f,light,7,None,origin='scripted_smoke',review='scripted',mutate=op)
            seed(f,light,8,None,origin='scripted_smoke',review='scripted')
            seed(f,light,9,None,mutate=lambda r:r['outcome'].update(review='deferred'))
            s=summary(f,light['run_id']);c=next(iter(s['resources'].values()))
            assert c['accepted_unverified']==1 and c['accepted_apply_blocked']==1,c
            assert c['quality_rejected']==0 and c['verification']['failed']==1,c
            assert s['counts']['first_attempt_checks']['failed']==1
            assert s['counts']['deferred']==1 and c['missing_review']==3
            assert s['counts']['policy_chains']==1 and s['counts']['failures']['authorization']==1,s
            assert s['origins']['scripted_smoke']['launched']==2 and s['origins']['scripted_smoke']['accepted']==0
            assert s['economics']['scripted_smoke']['harness_ms']['count']==2
            assert s['economics']['scripted_smoke']['harness_ms_per_accepted'] is None
            assert proposal(f,light['run_id'])['proposal_id'] is None
            def uncertain(r):
                r['attempts'][0]['id']='synthetic-unknown-launch'
                r['attempts'][0]['detail']['result']=None
            seed(f,light,10,None,mutate=uncertain)
            current=summary(f,light['run_id'])
            assert current['origins']['synthetic']['launch_unknown']==1
            assert current['economics']['synthetic']['harness_ms_per_accepted'] is None
            assert current['counts']['exclusions']['unfinished_execution']==1
            print('PASS independent outcome dimensions, failed-attempt burden, live-smoke shapes, no fabricated acceptance rate')
        elif name=='revisions':
            ids=reviewed(f,light,strong);p=proposal(f,light['run_id']);assert p['proposal_id'],p
            before=summary(f,light['run_id'])['counts']
            with sqlite3.connect(f.state/'dispatch.db') as db:
                for sequence in (900,901):
                    db.execute('INSERT INTO events(run_id,event_type,timestamp,payload_json,protocol_version,sequence,generation,actor) VALUES (?,?,?,?,1,?,1,?)',(ids[0],'review.accepted',stamp(),'{}',sequence,'synthetic-replay'))
            assert summary(f,light['run_id'])['counts']==before
            shadow(f);on=run(f);frozen=on['allocation']['private_evidence']
            with sqlite3.connect(f.state/'dispatch.db') as db:
                db.execute('INSERT INTO goal_feedback_revisions VALUES (?,?,?,?,?,?,?)',(f'{ids[0]}-revision-2',ids[0],2,'accepted','[]',None,stamp()))
            s=summary(f,light['run_id']);c=[c for c in s['resources'].values() if c['goals']==25][0]
            assert c['reviewed']==19 and c['missing_review']==6 and c['quality_rejected']==4,c
            assert not command(f,'evidence','policy',p['proposal_id'])['currently_valid']
            code,log=owner(f,'evidence','activate',f.source,p['proposal_id'],'--expected-revision','0');assert code!=0
            assert command(f,'evidence','private',on['run_id'])['at_decision_time']==frozen
            assert proposal(f,light['run_id'])['proposal_id'] is None
            print('PASS latest revision counted once, missing re-attestation excluded, exact historical shadow retained, stale activation refused')
        elif name=='compatibility':
            reviewed(f,light,strong);assert proposal(f,light['run_id'])['proposal_id']
            resources=f.state/'resources.yml';old=resources.read_text()
            resources.write_text(old.replace('authorization_revision: 1','authorization_revision: 2').replace('chatgpt-plus','renamed-display'))
            assert proposal(f,light['run_id'])['proposal_id']
            resources.write_text(old)
            config=f.source/'dispatch.yml';text=config.read_text();config.write_text(text.replace('test -f result.txt','test -e result.txt'))
            assert f.command('evidence','propose',light['run_id'],check=False).returncode!=0
            config.write_text(text)
            with sqlite3.connect(f.state/'dispatch.db') as db:
                raw=json.loads(db.execute('SELECT run_projection_json FROM runs WHERE id=?',('SYNTHETIC-000000',)).fetchone()[0])
                raw['attempts'][0]['harness_version']='another-version'
                db.execute('UPDATE runs SET run_projection_json=? WHERE id=?',(json.dumps(raw),'SYNTHETIC-000000'))
            assert proposal(f,light['run_id'])['proposal_id'] is None
            print('PASS proof/name renewal compatible; checks and harness version incompatible')
        elif name=='ordinary':
            seed(f,light,0,'accepted',origin='ordinary',review='human')
            seed(f,light,1,'accepted',origin='ordinary',review='machine')
            seed(f,light,2,'accepted',origin='scripted_smoke',review='scripted')
            seed(f,light,3,'accepted',origin='synthetic',review='synthetic')
            seed(f,light,4,'accepted',origin='unknown',review='human')
            with sqlite3.connect(f.state/'dispatch.db') as db:
                principal=db.execute('SELECT id FROM control_grants LIMIT 1').fetchone()[0]
                db.execute('INSERT INTO control_runs(run_id,principal,session_id) VALUES (?,?,?)',('SYNTHETIC-000000',principal,'synthetic-submitter'))
            s=summary(f,light['run_id'])
            assert s['submissions']['scoped_machine']==1
            assert s['counts']['reviewed']==1 and s['counts']['accepted']==1,s
            assert s['counts']['eligible_executions']==2,s
            # A terminal/accept operation alone is not an attestation.
            f.command('reject',light['run_id'],'--reason','correctness')
            assert summary(f,light['run_id'])['counts']['reviewed']==1
            assert f.command('evidence','annotate',light['run_id'],'--origin','ordinary','--review','human',check=False).returncode!=0
            print('PASS isolated synthetic test of ordinary eligibility: explicit review provenance required; no TTY/accept/machine-text inference')
        elif name=='inflight':
            reviewed(f,light,strong);pid=proposal(f,light['run_id'])['proposal_id'];assert pid
            f.key=Path(f.command('control-grant',f.source,'--allow-unsafe-local').stdout.decode().strip())
            c1=f.client();f.mode.write_text('wait')
            ready=os.open(f.ready,os.O_RDONLY|os.O_NONBLOCK)
            first=c1.call('submit',task='Add tests in src/lib.rs')
            assert select.select([ready],[],[],10)[0] and os.read(ready,1)==b'R'
            f.key=Path(f.command('control-grant',f.source,'--allow-unsafe-local').stdout.decode().strip())
            c2=f.client();second=c2.call('submit',task='Add tests in src/lib.rs')
            deadline=time.monotonic()+10
            while time.monotonic()<deadline:
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    queued=db.execute("SELECT COUNT(*) FROM admission_requests WHERE run_id=? AND status='queued'",(second['run_id'],)).fetchone()[0]
                if queued:break
                time.sleep(0.01)
            assert queued and f.count()==3
            before=[f.stored(x['run_id'])['allocation'] for x in (first,second)]
            code,log=owner(f,'evidence','activate',f.source,pid,'--expected-revision','0');assert code==0,log.decode()
            f.mode.write_text('success')
            with f.barrier.open('wb',buffering=0) as gate:gate.write(b'R')
            for client,accepted,decision in zip((c1,c2),(first,second),before):
                result=finish(client,accepted['run_id'],accepted['cursor'])
                assert result['result']['allocation']['selected']['tier']=='light'
                assert f.stored(accepted['run_id'])['allocation']==decision
                assert len(result['result']['attempts'])==1
                client.close()
            os.close(ready)
            assert f.count()==4
            assert run(f)['allocation']['selected']['tier']=='standard' and f.count()==5
            print('PASS activation leaves running/queued goal choices, deadlines and invocation bounds pinned')
        elif name in ('safety','stale_active','atomic'):
            ids=reviewed(f,light,strong);p=proposal(f,light['run_id']);pid=p['proposal_id'];assert pid,p
            if name=='atomic':
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    db.execute("CREATE TRIGGER synthetic_transition_fault BEFORE INSERT ON private_policy_transitions BEGIN SELECT RAISE(ABORT,'synthetic crash before policy commit'); END")
                code,log=owner(f,'evidence','activate',f.source,pid,'--expected-revision','0');assert code!=0
                assert command(f,'evidence','policy',pid)['status']=='proposed'
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    assert db.execute('SELECT COUNT(*) FROM private_policy_transitions').fetchone()[0]==0
                    db.execute('DROP TRIGGER synthetic_transition_fault')
            code,log=owner(f,'evidence','activate',f.source,pid,'--expected-revision','0');assert code==0,log.decode()
            if name=='stale_active':
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    db.execute('INSERT INTO goal_feedback_revisions VALUES (?,?,?,?,?,?,?)',(f'{ids[0]}-corrected',ids[0],2,'accepted','[]',None,stamp()))
                result=run(f)
                assert result['allocation']['selected']['tier']=='light'
                assert result['allocation']['private_evidence']['reason']=='active_evidence_stale_base_fallback'
                assert command(f,'evidence','policy',pid)['status']=='stale'
                again=run(f);assert again['allocation']['selected']['tier']=='light'
                code,log=owner(f,'evidence','activate',f.source,pid,'--expected-revision','1');assert code!=0
            elif name=='atomic':
                code,log=owner(f,'evidence','rollback',f.source,'--expected-revision','0');assert code!=0
                assert run(f)['allocation']['selected']['tier']=='standard'
                other=f.root/'other-project';other.mkdir()
                code,log=owner(f,'evidence','activate',other,pid,'--expected-revision','0');assert code!=0
                # Equal/stale projection writes cannot alter the separate authoritative policy history.
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    db.execute('UPDATE runs SET run_projection_json=run_projection_json WHERE id=?',(light['run_id'],))
                    changed=f.stored(light['run_id']);changed['allocation']['private_evidence']['mode']='forged'
                    try:db.execute('UPDATE runs SET run_projection_json=? WHERE id=?',(json.dumps(changed),light['run_id']))
                    except sqlite3.IntegrityError:pass
                    else:raise AssertionError('historical decision mutation was accepted')
                assert command(f,'evidence','policy',pid)['status']=='active'
            else:
                assert run(f,'--model','fixture-model')['allocation']['selected']['tier']=='light'
                path=f.state/'resources.yml';old=path.read_text()
                path.write_text(old.replace('model: fixture-standard','enabled: false\n    model: fixture-standard'))
                assert run(f)['allocation']['selected']['tier']=='light'
                # Grant created with only light visible never acquires the restored target.
                f.key=Path(f.command('control-grant',f.source,'--allow-unsafe-local').stdout.decode().strip())
                path.write_text(old)
                c=f.client();accepted=c.call('submit',task='Add tests in src/lib.rs')
                result=finish(c,accepted['run_id'],accepted['cursor'])
                assert result['result']['allocation']['selected']['tier']=='light',result
                assert 'private_evidence' not in json.dumps(result) and 'SYNTHETIC' not in json.dumps(result)
                reply=c.receive(c.send('activate',proposal_id=pid,human=True));assert not reply['ok']
                c.close()
                path.write_text(old.replace('model: fixture-standard','model: changed-standard'))
                assert run(f)['allocation']['selected']['tier']=='light'
                path.write_text(old.replace('model: fixture-standard','no_overage_verified: false\n    model: fixture-standard').replace('    no_overage_verified: true\n    authorization_revision: 1\n','    authorization_revision: 1\n'))
                # All profiles now lack a valid funding assertion: no invocation is permitted.
                before=f.count();blocked=f.command('run',f.source,'--task','Add tests in src/lib.rs','--allow-unsafe-local','--json',check=False)
                assert blocked.returncode!=0 and f.count()==before
                path.write_text(old)
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    pool=light['capacity']['pool_id']
                    obs=json.loads(db.execute('SELECT payload_json FROM capacity_observations WHERE pool_id=? ORDER BY sampled_at DESC LIMIT 1',(pool,)).fetchone()[0])
                    obs['id']='synthetic-phase7-exhausted';obs['sampled_at']=stamp();obs['scarcity']='exhausted';obs['mapping']='mapped'
                    obs['constraints'][0].update(provider_bucket_id='codex',window_id='weekly',scope={'knowledge':'reported','value':'included_subscription'},remaining={'knowledge':'reported','value':0.0})
                    db.execute('INSERT INTO capacity_observations(id,pool_id,source,source_version,sampled_at,valid_until,payload_json) VALUES(?,?,?,?,?,?,?)',(obs['id'],pool,obs['source'],obs['source_version'],obs['sampled_at'],obs['valid_until'],json.dumps(obs)))
                    db.execute('INSERT INTO capacity_observation_constraints VALUES(?,?,?,?,?)',(obs['id'],0,'codex','weekly',json.dumps(obs['constraints'][0])))
                blocked=f.command('run',f.source,'--task','Add tests in src/lib.rs','--allow-unsafe-local','--json',check=False)
                assert blocked.returncode!=0 and f.count()==before
                value=json.loads(blocked.stdout)
                assert value['allocation']['selected']['tier']=='light'
                assert not any(a['eligible'] for a in value['allocation']['alternatives'])

            print('PASS '+name+': isolated promotion guard/fallback, no permission expansion, durable policy history')
        elif name=='analytics_failure':
            reviewed(f,light,strong);pid=proposal(f,light['run_id'])['proposal_id'];assert pid
            code,log=owner(f,'evidence','activate',f.source,pid,'--expected-revision','0');assert code==0,log.decode()
            broken=seed(f,light,90,None)
            with sqlite3.connect(f.state/'dispatch.db') as db:
                # Keep immutable decision intact while corrupting an unrelated optional projection.
                raw=f.stored(broken);raw['attempts']='invalid synthetic projection'
                db.execute('UPDATE runs SET run_projection_json=? WHERE id=?',(json.dumps(raw),broken))
            result=run(f)
            meta=result['allocation']['private_evidence']
            assert result['allocation']['selected']['tier']=='light'
            assert meta['reason']=='analytics_unavailable_base_fallback' and meta['counts'] is None
            before=f.count()
            with sqlite3.connect(f.state/'dispatch.db') as db:
                raw=json.loads(db.execute('SELECT payload_json FROM private_proposals WHERE id=?',(pid,)).fetchone()[0]);raw['version']=999
                db.execute('UPDATE private_proposals SET payload_json=? WHERE id=?',(json.dumps(raw),pid))
            failed=f.command('run',f.source,'--task','Add tests in src/lib.rs','--allow-unsafe-local','--json',check=False)
            assert failed.returncode!=0 and f.count()==before
            print('PASS optional analytics failure yields base with unknown counts; authoritative policy corruption refuses launch')
        elif name=='bounds':
            for i in range(1050):seed(f,light,i,None)
            start=time.monotonic();s=summary(f,light['run_id']);elapsed=time.monotonic()-start
            assert s['truncated'] and s['counts']['goals']==1000,s['counts']
            decision_start=time.monotonic()
            assert proposal(f,light['run_id'])['proposal_id'] is None
            decision_elapsed=time.monotonic()-decision_start
            assert f.count()==2
            print(f'PASS bounded history: 1052 synthetic stored goals, 1000-row cap, inspect {elapsed:.3f}s, proposal {decision_elapsed:.3f}s (CLI/DB/rule time; no provider calls; not a universal latency guarantee)')
        else: raise AssertionError(name)
    finally:f.cleanup()

if __name__=='__main__': scenario(sys.argv[1],sys.argv[2])
