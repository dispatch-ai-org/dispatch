#!/usr/bin/env python3
"""Deterministic real-pipe Phase 5 fixture. Never invokes a model provider."""
import json
import hashlib
from datetime import datetime, timedelta, timezone
import os
from pathlib import Path
import select
import signal
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'examples'))
from control_client import Client, workflow


class Fixture:
    def __init__(self, binary, mode='success'):
        self.provider = os.environ.get('DISPATCH_FIXTURE_PROVIDER', 'codex')
        self.binary = str(Path(binary).resolve())
        self.temp = tempfile.TemporaryDirectory(prefix='dispatch-phase5-')
        self.root = Path(self.temp.name)
        self.source = self.root / 'source'
        self.source.mkdir()
        self.state = self.root / 'state'
        self.state.mkdir(mode=0o700)
        (self.source / 'result.txt').write_text('ok\n')
        (self.source / 'src').mkdir()
        (self.source / 'src/lib.rs').write_text('// baseline\n')
        self.mode = self.root / 'mode'
        self.mode.write_text(mode)
        self.invocations = self.root / 'invocations'
        self.barrier = self.root / 'barrier'
        os.mkfifo(self.barrier)
        self.ready = self.root / "ready"
        os.mkfifo(self.ready)
        agent = self.root / self.provider
        agent.write_text('''#!/usr/bin/env python3
import json, pathlib, sys, os
root=pathlib.Path(''' + repr(str(self.root)) + ''')
if '--version' in sys.argv:
    print('codex phase5 fixture');sys.exit()
prompt=sys.stdin.read()
with (root/'invocations').open('a') as f: f.write('invocation\\n')
count=len((root/'invocations').read_text().splitlines())
mode=(root/'mode').read_text()
if mode=='wait':
    try:
        fd=os.open(root/'ready',os.O_WRONLY|os.O_NONBLOCK);os.write(fd,b'R');os.close(fd)
    except OSError: pass
    with (root/'barrier').open('rb',buffering=0) as gate: gate.read(1)
if mode=='noisy':
    for i in range(30000): print('x'*1024)
    sys.stdout.flush()
    try:
        fd=os.open(root/'ready',os.O_WRONLY|os.O_NONBLOCK);os.write(fd,b'R');os.close(fd)
    except OSError: pass
    with (root/'barrier').open('rb',buffering=0) as gate: gate.read(1)
if mode in ('clarify','unclassified') and count==1:
    checkpoint={'version':1,'question':'Which label?','choices':['blue','green']}
    if mode=='clarify': checkpoint['category']='factual'
    print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':json.dumps({'dispatch_checkpoint':checkpoint})}}))
else:
    pathlib.Path('src/lib.rs').write_text('// delivered\\n')
    pathlib.Path('result.txt').write_text('ok\\n')
    print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':'Done'}}))
''')
        if self.provider == 'claude':
            os.environ.setdefault('USER', 'dispatch-fixture-user')
            script = agent.read_text().replace("print('codex phase5 fixture');sys.exit()", "print('claude phase6 fixture');sys.exit()")
            script = script.replace('prompt=sys.stdin.read()', """if 'auth' in sys.argv and 'status' in sys.argv:
    assert sys.argv[-4:]==['--no-chrome','auth','status','--json']
    assert os.environ.get('USER'), 'Keychain lookup needs username metadata'
    count_path=root/'auth-count'
    n=int(count_path.read_text())+1 if count_path.exists() else 1
    count_path.write_text(str(n))
    boundary=root/'auth-boundary'
    if boundary.exists() and n==int(boundary.read_text()):
        auth=json.loads((root/'auth.json').read_text());auth['email']='changed@example.invalid'
        (root/'auth.json').write_text(json.dumps(auth))
    print((root/'auth.json').read_text());sys.exit()
assert '--dangerously-skip-permissions' not in sys.argv and '--bare' not in sys.argv
assert os.environ.get('USER'), 'model invocation must use the same Keychain environment'
assert sys.argv[sys.argv.index('--setting-sources')+1]==''
assert sys.argv[sys.argv.index('--permission-mode')+1]=='dontAsk'
assert sys.argv[sys.argv.index('--tools')+1]=='Bash,Read,Edit,Write,Glob,Grep'
assert not any(k in os.environ for k in ['ANTHROPIC_API_KEY','ANTHROPIC_AUTH_TOKEN','CLAUDE_CODE_USE_BEDROCK','DISPATCH_CONTROL_GRANT_FD'])
prompt=sys.argv[-1]
def emit(text):
    model=sys.argv[sys.argv.index('--model')+1]
    model_usage={model:{'inputTokens':10,'outputTokens':4}}
    settings=json.loads(sys.argv[sys.argv.index('--settings')+1])
    # Reproduce the real CLI's extra title-generation model unless disabled.
    if settings.get('env',{}).get('CLAUDE_CODE_DISABLE_TERMINAL_TITLE')!='1':
        model_usage['claude-haiku-4-5-20251001']={'inputTokens':1095,'outputTokens':13}
    print(json.dumps({'type':'system','subtype':'init','model':model}))
    print(json.dumps({'type':'result','subtype':'success','is_error':False,'result':text,
        'usage':{'input_tokens':10,'output_tokens':4,'cache_read_input_tokens':6,'cache_creation_input_tokens':2},
        'total_cost_usd':0.42,'modelUsage':model_usage}))
""")
            script = script.replace("print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':json.dumps({'dispatch_checkpoint':checkpoint})}}))", "emit(json.dumps({'dispatch_checkpoint':checkpoint}))")
            script = script.replace("print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':'Done'}}))", "emit('Done')")
            agent.write_text(script)
            (self.root/'auth.json').write_text(json.dumps({'loggedIn':True,'authMethod':'claude.ai','apiProvider':'firstParty','email':'fixture@example.invalid','orgId':'fixture-org'}))
            home = self.root/'home'
            home.mkdir()
            os.environ['HOME'] = str(home)
        agent.chmod(0o755)
        (self.source / 'dispatch.yml').write_text(
            f"execution:\n  timeout_secs: 30\nchecks:\n  verify: ['test -f result.txt']\nharnesses:\n  {self.provider}:\n    executable: '{agent}'\n")
        (self.state / 'resources.yml').write_text('''version: 1
allocation_enabled: true
capacity:
  codex_probe: false
profiles:
  - provider: openai
    funding_source: chatgpt-plus
    harness: codex
    model: fixture-model
    effort: low
    runtime: local
    service_mode: standard
    pool: shared
    provider_buckets: [codex]
    tier: light
    included: true
    no_overage_verified: true
    authorization_revision: 1
''')
        if self.provider == 'claude':
            path = self.state/'resources.yml'
            text = path.read_text().replace('provider: openai','provider: anthropic').replace('funding_source: chatgpt-plus','funding_source: claude-fixture').replace('harness: codex','harness: claude')
            now = datetime.now(timezone.utc)
            evidence = {'contract_version':1,'cli_version':'claude phase6 fixture',
                'executable_sha256':hashlib.sha256(agent.read_bytes()).hexdigest(),
                'account_sha256':hashlib.sha256(json.dumps(['fixture@example.invalid','fixture-org'],separators=(',',':')).encode()).hexdigest(),
                'checked_at':now.isoformat(),'valid_until':(now+timedelta(hours=1)).isoformat(),
                'print_mode_included':True,'usage_credits_disabled':True,'unmanaged_account':True}
            text += '    claude_subscription: '+json.dumps(evidence)+'\n'
            path.write_text(text)
        self.key = Path(self.command('control-grant', self.source, '--allow-unsafe-local',
                                     '--delegate-factual').stdout.decode().strip())
        self.clients = []

    def command(self, *args, check=True):
        return subprocess.run([self.binary, '--state-dir', str(self.state), *map(str,args)],
                              capture_output=True, check=check)

    def client(self, **kwargs):
        c = Client(self.binary, self.state, self.key, **kwargs)
        self.clients.append(c)
        return c

    def count(self):
        return len(self.invocations.read_text().splitlines()) if self.invocations.exists() else 0

    def stored(self, run_id):
        with sqlite3.connect(self.state / 'dispatch.db') as db:
            return json.loads(db.execute('SELECT run_projection_json FROM runs WHERE id=?', (run_id,)).fetchone()[0])

    def cleanup(self):
        if sys.exc_info()[0] is not None:
            for metadata in (self.state/'runs').glob('*/metadata.json'):
                run=json.loads(metadata.read_text())
                print('fixture failure:', run.get('outcome'),
                      [c.get('error') for c in run.get('candidates',[])], file=sys.stderr)
        for c in self.clients:
            if c.process.poll() is None:
                c.close()
        self.temp.cleanup()


def assert_error(c, op, code, **fields):
    reply=c.receive(c.send(op, **fields))
    assert not reply['ok'], reply
    assert reply['error']['code']==code, reply
    return reply


def finish(c, run_id, after=0):
    reply=c.call('await', run_id=run_id, after=after, predicate='execution_finished', timeout_ms=10000)
    assert reply['reached'], reply
    return c.call('result', run_id=run_id)


def wait_question(c, run_id):
    c.call('await', run_id=run_id, after=0, predicate='attention_required', timeout_ms=10000)
    return c.call('status', run_id=run_id)


def scenario(binary, name):
    f=Fixture(binary, 'clarify' if name in ('clarify','authority','recovery','question_eof','lost_answer','funding','recovery_limits','recovery_drift','unclassified','answer_cancel') else 'wait' if name in ('cancel','disconnect','observer') else 'success')
    try:
        c=f.client(initialize=name!='framing')
        if name=='smoke':
            result=workflow(c,'Add tests in src/lib.rs')
            assert result['result']['outcome']['verification']=='passed', result
            assert result['result']['outcome']['review']=='pending', result
            assert result['result']['outcome']['application']=='not_applied', result
            assert f.count()==1
            run_id=result['result']['run_id']
            ref=result['artifacts'][0]
            patch=c.call('artifact', **ref)
            assert 'delivered' in patch['text'], patch
            assert_error(c,'artifact','unauthorized',run_id=run_id,attempt_id='../other',kind='diff')
            second=workflow(c,'Add tests in src/lib.rs')
            assert second['result']['run_id']!=run_id
            assert f.stored(run_id)['outcome']['review']=='pending'
            print('standalone verified delivery; human review pending; second goal; scoped diff')
        elif name=='idempotency':
            first=c.call('submit',request_id='stable',task='Add tests in src/lib.rs')
            again=c.call('submit',request_id='stable',task='Add tests in src/lib.rs')
            assert first==again
            assert_error(c,'submit','request_conflict',request_id='stable',task='Different task')
            finish(c,first['run_id'])
            assert f.count()==1
            c.close()
            c=f.client()
            assert c.call('submit',request_id='stable',task='Add tests in src/lib.rs')==first
            assert f.count()==1
            batch=[dict(protocol_version=1,request_id='burst',op='submit',task=task) for task in ('Add tests in src/lib.rs','Different task')]
            c.process.stdin.write((''.join(json.dumps(r)+'\n' for r in batch)).encode());c.process.stdin.flush()
            replies=[c.receive('burst'),c.receive('burst')]
            assert sorted(r['ok'] for r in replies)==[False,True],replies
            assert next(r for r in replies if not r['ok'])['error']['code']=='request_conflict',replies
            run=next(r for r in replies if r['ok'])['result']['run_id']
            finish(c,run);assert f.count()==2
            print('durable submit retry and conflict, including new session')
        elif name in ('clarify','authority','recovery','question_eof'):
            accepted=c.call('submit',task='Add tests in src/lib.rs')
            run_id=accepted['run_id']
            status=wait_question(c,run_id)
            q=status['phase3']['questions'][-1]
            args=dict(run_id=run_id,question_id=q['id'],revision=q['revision'],generation=q['generation'],answer='blue')
            if name=='authority':
                denied=f.command('answer',run_id,q['id'],'--revision',q['revision'],'--answer','blue','--json',check=False)
                assert denied.returncode!=0 and b'foreground control owner' in denied.stderr
                assert f.count()==1
                assert_error(c,'status','unauthorized',run_id='01OTHER')
                assert_error(c,'answer','invalid_request',**args,human=True)
                assert_error(c,'answer','invalid_request',**args,actor='human')
                assert_error(c,'submit','invalid_request',task='escape',source='/tmp')
                assert_error(c,'answer','stale_revision',**dict(args,question_id='wrong'))
                assert_error(c,'answer','stale_revision',**dict(args,revision=99))
                observer=f.client(read_only=True)
                assert_error(observer,'answer','unauthorized',**args)
                observer.close()
                cancelled=c.call('cancel',run_id=run_id,revision=status['state_revision'])
                assert cancelled['accepted']
                assert_error(c,'answer','recovery_required',**args)
                assert f.count()==1
            elif name=='question_eof':
                c.close()
                assert f.stored(run_id)['outcome']['work_result']=='cancelled'
                assert f.count()==1
            else:
                if name=='recovery':
                    original_deadline=status['phase3']['deadline_at']
                    c.process.kill();c.process.wait()
                    c=f.client()
                    assert f.count()==1
                    assert_error(c,'answer','recovery_required',**args)
                    c.call('recover',run_id=run_id,revision=status['state_revision'])
                waiting=c.send('await',run_id=run_id,after=status['cursor'],predicate='execution_finished',timeout_ms=10000)
                answer=c.call('answer',request_id='answer-once',**args)
                assert c.call('answer',request_id='answer-once',**args)==answer
                assert c.receive(waiting)['result']['reached']
                result=c.call('result',run_id=run_id)
                assert len(result['result']['attempts'])==2 and f.count()==2, result
                assert result['result']['phase3']['questions'][-1]['actor'].startswith('machine:')
                assert result['result']['outcome']['review']=='pending'
                if name=='recovery': assert result['result']['phase3']['deadline_at']==original_deadline
                # Historical attention cannot disappear when the answer races a reader.
                transient=c.call('await',run_id=run_id,after=accepted['cursor'],predicate='attention_required',timeout_ms=1)
                assert transient['outcome']['waiting_on']=='human', transient
                assert c.call('answer',request_id='answer-once',**args)==answer
            print(name+' passed: real-pipe question/authority/ownership boundary')
        elif name in ('cancel','disconnect','observer'):
            accepted=c.call('submit',task='Add tests in src/lib.rs')
            run_id=accepted['run_id']
            assert_error(c,'submit','busy',task='Other work')
            assert_error(c,'result','not_ready',run_id=run_id)
            # FIFO open is a deterministic barrier: the fixture has reached execution.
            if name=='observer':
                observer=f.client(read_only=True)
                observer.call('status',run_id=run_id)
                observer.close()
                assert c.call('status',run_id=run_id)['outcome']['lifecycle']!='finished'
            waiting=c.send('await',run_id=run_id,after=accepted['cursor'],predicate='execution_finished',timeout_ms=10000)
            if name=='disconnect':
                c.close()
                assert f.stored(run_id)['outcome']['work_result']=='cancelled'
            else:
                for _ in range(50):
                    status=c.call('status',run_id=run_id)
                    reply=c.receive(c.send('cancel',run_id=run_id,revision=status['state_revision']))
                    if reply['ok']:break
                    assert reply['error']['code']=='stale_revision',reply
                else: raise AssertionError('cancel did not commit')
                assert c.receive(waiting)['result']['reached']
                assert c.call('result',run_id=run_id)['result']['outcome']['work_result']=='cancelled'
            print(name+' passed while semantic await was pending')
        elif name in ('lost_answer','funding','recovery_limits','recovery_drift','unclassified','answer_cancel'):
            if name=='unclassified': f.mode.write_text('unclassified')
            accepted=c.call('submit',task='Add tests in src/lib.rs')
            run_id=accepted['run_id']
            status=wait_question(c,run_id)
            q=status['phase3']['questions'][-1]
            args=dict(run_id=run_id,question_id=q['id'],revision=q['revision'],generation=q['generation'],answer='blue')
            if name=='unclassified':
                assert_error(c,'answer','authorization_required',**args)
                assert f.count()==1
            elif name=='funding':
                resources=f.state/'resources.yml'
                resources.write_text(resources.read_text().replace('no_overage_verified: true','no_overage_verified: false'))
                # The answer itself commits; fresh continuation authorization then refuses spending.
                c.call('answer',**args)
                final=finish(c,run_id,status['cursor'])
                assert final['result']['outcome']['work_result']!='ready',final
                assert f.count()==1
            elif name=='answer_cancel':
                ready=os.open(f.ready,os.O_RDONLY|os.O_NONBLOCK);f.mode.write_text('wait')
                answer=c.call('answer',request_id='cancelled-answer',**args)
                assert select.select([ready],[],[],10)[0]
                assert os.read(ready,1)==b'R'
                status=c.call('status',run_id=run_id)
                c.call('cancel',run_id=run_id,revision=status['state_revision'])
                finish(c,run_id,status['cursor'])
                assert c.call('answer',request_id='cancelled-answer',**args)==answer
                assert_error(c,'answer','recovery_required',**args)
                assert f.count()==2
                os.close(ready)
            elif name=='recovery_drift':
                c.process.kill();c.process.wait()
                (f.source/'src/lib.rs').write_text('// unrelated user edit\n')
                c=f.client()
                assert_error(c,'recover','command_rejected',run_id=run_id,revision=status['state_revision'])
                assert f.count()==1
            elif name=='lost_answer':
                config=f.state/'runs'/run_id/'config.snapshot.yml'
                saved=config.read_bytes();config.unlink();os.mkfifo(config)
                c.send('answer',request_id='lost-reply',**args)
                # Opening the FIFO proves answer commit completed and config loading began.
                fd=os.open(config,os.O_WRONLY)
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    receipt=json.loads(db.execute("SELECT reply_json FROM command_receipts WHERE request_id='lost-reply'").fetchone()[0])
                c.process.kill();c.process.wait();os.close(fd)
                config.unlink();config.write_bytes(saved)
                before=f.stored(run_id)
                c=f.client()
                assert c.receive(c.send('answer',request_id='lost-reply',**args))==receipt
                assert f.count()==1  # Retrying the receipt never resumes an abandoned invocation.
                c.call('recover',run_id=run_id,revision=before['state_revision'])
                final=finish(c,run_id,status['cursor'])
                assert f.count()==2 and len(final['result']['attempts'])==2
                assert final['result']['phase3']['deadline_at']==status['phase3']['deadline_at']
                assert c.receive(c.send('answer',request_id='lost-reply',**args))==receipt
            else:
                # A live grant owner cannot be displaced, and an expired original deadline cannot recover.
                with f.key.open('rb') as key:
                    other=subprocess.run([f.binary,'--state-dir',str(f.state),'control','--stdio','--grant-fd',str(key.fileno())],pass_fds=(key.fileno(),),input=b'',capture_output=True)
                assert other.returncode!=0 and b'foreground owner' in other.stderr
                c.process.kill();c.process.wait()
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    run=json.loads(db.execute('SELECT run_projection_json FROM runs WHERE id=?',(run_id,)).fetchone()[0])
                    run['phase3']['deadline_at']='2000-01-01T00:00:00Z'
                    db.execute('UPDATE runs SET run_projection_json=? WHERE id=?',(json.dumps(run),run_id))
                c=f.client()
                assert_error(c,'recover','recovery_ineligible',run_id=run_id,revision=status['state_revision'])
                assert f.count()==1
            print(name+' passed with original budget and no duplicate spending')
        elif name in ('slow','broken_pipe','broken_question','broken_admission','hangup','queued_eof','overlap','tui_overlap','mapped_overlap','atomic','paths','migration','limits'):
            if name=='limits':
                f.mode.write_text('wait')
                run_id=c.call('submit',task='Add tests in src/lib.rs')['run_id']
                for _ in range(50):
                    status=c.call('status',run_id=run_id)
                    if status['outcome']['phase']=='executing' and status['admission'] and status['admission']['state']=='admitted':break
                ids=[c.send('await',run_id=run_id,after=status['cursor'],predicate='execution_finished',timeout_ms=60000) for _ in range(8)]
                assert_error(c,'await','too_many_requests',run_id=run_id,after=status['cursor'],predicate='execution_finished',timeout_ms=60000)
                # Cancel is processed even with every observer slot occupied.
                for _ in range(50):
                    latest=f.stored(run_id)
                    reply=c.receive(c.send('cancel',run_id=run_id,revision=latest['state_revision']))
                    if reply['ok']:break
                    assert reply['error']['code']=='stale_revision',reply
                assert reply['ok']
                for id in ids: assert c.receive(id)['result']['reached']
            elif name=='atomic':
                assert_error(c,'recover','unauthorized',run_id='../../escape',revision=0)
                assert not (f.root/'escape').exists()
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    db.execute("CREATE TRIGGER receipt_fault BEFORE INSERT ON command_receipts BEGIN SELECT RAISE(ABORT,'receipt fault'); END")
                reply=c.receive(c.send('submit',task='Add tests in src/lib.rs'))
                assert not reply['ok'],reply
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    assert db.execute('SELECT count(*) FROM runs').fetchone()[0]==0
                    assert db.execute('SELECT count(*) FROM events').fetchone()[0]==0
                    assert db.execute('SELECT count(*) FROM control_runs').fetchone()[0]==0
                assert f.count()==0
            elif name=='migration':
                c.close();f.mode.write_text('clarify')
                before=f.command('run',f.source,'--task','Add tests in src/lib.rs','--allow-unsafe-local','--json',check=False)
                assert before.returncode==4,before.stderr
                run_id=json.loads(before.stdout)['run_id']
                tables=('attempts','clarifications','capacity_authorizations','events','admission_requests')
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    rows={table:db.execute('SELECT * FROM '+table).fetchall() for table in tables}
                    db.execute('DROP TABLE command_receipts');db.execute('DROP TABLE control_runs');db.execute('DROP TABLE control_grants')
                    db.execute('DELETE FROM schema_migrations WHERE version=18')
                after=f.command('status',run_id,'--json',check=False)
                assert after.returncode in (0,4),after.stderr
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    assert db.execute('SELECT max(version) FROM schema_migrations').fetchone()[0]==21
                    assert not db.execute('PRAGMA foreign_key_check').fetchall()
                    for table in tables: assert db.execute('SELECT * FROM '+table).fetchall()==rows[table],table
                assert json.loads(after.stdout)['phase3']['questions'][0]['state']=='pending'
            elif name=='paths':
                result=workflow(c,'Add tests in src/lib.rs')
                ref=result['artifacts'][0]
                run=f.stored(ref['run_id'])
                patch=Path(run['attempts'][0]['detail']['result']['diff_path'])
                patch.unlink();patch.symlink_to(f.key)
                assert_error(c,'artifact','unauthorized',**ref)
                assert_error(c,'submit','invalid_request',task='escape',grant={'human':True})
                assert_error(c,'submit','command_rejected',task='Add tests in src/lib.rs',model='ungranted')
                assert f.count()==1
            elif name in ('overlap','queued_eof','tui_overlap','mapped_overlap'):
                f.mode.write_text('wait')
                ready=os.open(f.ready,os.O_RDONLY|os.O_NONBLOCK)
                # Existing CLI uses the same allocation/admission execution path as the TUI.
                if name=='tui_overlap':
                    human_source=f.root/'human-source'
                    shutil.copytree(f.source,human_source)
                    human=subprocess.Popen([sys.executable,str(ROOT/'tests/fixtures/phase4_session.py'),f.binary,str(human_source),str(f.state),'concurrent'],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
                else:
                    human=subprocess.Popen([f.binary,'--state-dir',str(f.state),'run',str(f.source),'--task','Add tests in src/lib.rs','--allow-unsafe-local','--json'],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
                assert select.select([ready],[],[],10)[0], 'human fixture did not start'
                assert os.read(ready,1)==b'R'
                if name=='mapped_overlap':
                    resources=f.state/'resources.yml'
                    resources.write_text(resources.read_text().replace('pool: shared','pool: expanded').replace('[codex]','[codex, extra]'))
                    c.close()
                    f.key=Path(f.command('control-grant',f.source,'--allow-unsafe-local','--delegate-factual').stdout.decode().strip())
                    c=f.client()
                accepted=c.call('submit',task='Add tests in src/lib.rs')
                run_id=accepted['run_id']
                cursor=accepted['cursor']
                for _ in range(50):
                    changed=c.call('await',run_id=run_id,after=cursor,predicate='state_changed',timeout_ms=1000)
                    cursor=changed['cursor']
                    status=c.call('status',run_id=run_id)
                    if status['outcome']['waiting_on'] in ('capacity','admission'): break
                assert status['outcome']['waiting_on'] in ('capacity','admission'),status
                assert f.count()==1
                if name=='queued_eof':
                    c.close();assert f.stored(run_id)['outcome']['work_result']=='cancelled'
                else:
                    f.mode.write_text('success')
                with f.barrier.open('wb',buffering=0) as gate: gate.write(b'G')
                human_out,human_err=human.communicate(timeout=15)
                assert human.returncode==0,(human_out,human_err)
                if name!='queued_eof':
                    finish(c,run_id,cursor);assert f.count()==2
                else: assert f.count()==1
                os.close(ready)
            elif name=='hangup':
                f.mode.write_text('clarify')
                accepted=c.call('submit',task='Add tests in src/lib.rs')
                status=wait_question(c,accepted['run_id'])
                c.process.send_signal(signal.SIGHUP);c.process.wait(timeout=10)
                assert f.stored(accepted['run_id'])['outcome']['work_result']=='cancelled'
            else:
                c.close()
                f.mode.write_text('noisy' if name=='slow' else 'clarify' if name=='broken_question' else 'wait')
                ready=os.open(f.ready,os.O_RDONLY|os.O_NONBLOCK)
                holder=None
                if name=='broken_admission':
                    holder=subprocess.Popen([f.binary,'--state-dir',str(f.state),'run',str(f.source),'--task','Add tests in src/lib.rs','--allow-unsafe-local','--json'],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
                    assert select.select([ready],[],[],10)[0];assert os.read(ready,1)==b'R'
                with f.key.open('rb') as key:
                    proc=subprocess.Popen([f.binary,'--state-dir',str(f.state),'control','--stdio','--grant-fd',str(key.fileno())],pass_fds=(key.fileno(),),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
                def send(op,id,**fields):
                    proc.stdin.write((json.dumps(dict(protocol_version=1,request_id=id,op=op,**fields))+'\n').encode());proc.stdin.flush()
                send('initialize','hello');assert json.loads(proc.stdout.readline())['ok']
                send('submit','start',task='Add tests in src/lib.rs');accepted=json.loads(proc.stdout.readline())['result'];run_id=accepted['run_id']
                if name=='broken_question':
                    send('await','question',run_id=run_id,after=accepted['cursor'],predicate='attention_required',timeout_ms=10000)
                    assert json.loads(proc.stdout.readline())['result']['outcome']['waiting_on']=='human'
                    proc.stdout.close();send('status','broken',run_id=run_id)
                elif name=='broken_admission':
                    cursor=accepted['cursor']
                    for i in range(40):
                        send('await','change-'+str(i),run_id=run_id,after=cursor,predicate='state_changed',timeout_ms=1000)
                        change=json.loads(proc.stdout.readline())['result'];cursor=change['cursor']
                        if change['outcome']['waiting_on']=='admission':break
                    assert change['outcome']['waiting_on']=='admission'
                    proc.stdout.close();send('status','broken',run_id=run_id)
                elif name=='broken_pipe':
                    assert select.select([ready],[],[],10)[0]
                    proc.stdout.close();send('status','broken',run_id=run_id)
                else:
                    # Stop draining control output while the child emits 30 MiB.
                    for i in range(6): send('x'*60000,'flood-'+str(i))
                    assert select.select([ready],[],[],10)[0], 'child pipe was not drained'
                    assert os.read(ready,1)==b'R'
                proc.wait(timeout=15)
                assert proc.returncode!=0
                assert f.stored(run_id)['outcome']['work_result']=='cancelled',f.stored(run_id)['outcome']
                proc.stdin.close();proc.stderr.close()
                if not proc.stdout.closed:proc.stdout.close()
                os.close(ready)
                if holder:
                    f.mode.write_text('success')
                    with f.barrier.open('wb',buffering=0) as gate:gate.write(b'G')
                    holder.communicate(timeout=15)
                    assert holder.returncode==0 and f.count()==1
                c=f.client()
                cursor=c.call('events',run_id=run_id,after=0)['cursor']
                assert c.events[-1]['event']['sequence']==cursor
                assert c.call('result',run_id=run_id)['result']['outcome']['work_result']=='cancelled'
            print(name+' passed at real process and persistence boundary')
        elif name=='waits':
            accepted=c.call('submit',task='Add tests in src/lib.rs')
            run_id=accepted['run_id']
            result=finish(c,run_id)
            current=c.call('status',run_id=run_id)
            assert c.call('await',run_id=run_id,after=current['cursor'],predicate='execution_finished',timeout_ms=0)['reached']
            assert c.call('await',run_id=run_id,after=current['cursor'],predicate='state_changed',timeout_ms=1)['timed_out']
            assert_error(c,'await','invalid_cursor',run_id=run_id,after=current['cursor']+1,predicate='state_changed',timeout_ms=0)
            c.call('events',run_id=run_id,after=0)
            seq=[e['event']['sequence'] for e in c.events]
            assert seq==list(range(1,current['cursor']+1)),seq
            assert len({e['event_id'] for e in c.events})==len(seq)
            print('already satisfied, timeout, invalid cursor, ordered journal replay')
        elif name=='framing':
            assert_error(c,'submit','initialize_required',task='uninitialized')
            c.process.stdin.write(b'{bad}\n\xff\n');c.process.stdin.flush()
            for _ in range(2): assert not c.incoming.get(timeout=5)['ok']
            assert_error(c,'initialize','invalid_request',human=True)
            frame=json.dumps(dict(protocol_version=1,request_id='fragment',op='initialize')).encode()+b'\n'
            for piece in (frame[:3],frame[3:15],frame[15:]):c.process.stdin.write(piece);c.process.stdin.flush()
            assert c.receive('fragment')['ok']
            c.process.stdin.write(b'{"protocol_version":9,"request_id":"version","op":"submit","task":"no"}\n');c.process.stdin.flush()
            assert c.receive('version')['error']['code']=='unsupported_version'
            assert_error(c,'set_state','invalid_request',state='verified')
            assert_error(c,'submit','invalid_request')
            c.process.stdin.write(b'{"op":"submit"}');c.process.stdin.flush()
            code,_=c.close();assert code!=0
            assert f.count()==0
            c=f.client()
            c.process.stdin.write(b'x'*65537);c.process.stdin.flush()
            c.process.wait(timeout=10)
            assert f.count()==0
            print('pre-init, malformed/UTF-8, unknown/forged fields, version, fragments, partial EOF, frame bound')
        else: raise AssertionError(name)
    finally:
        f.cleanup()


if __name__=='__main__':
    scenario(sys.argv[1],sys.argv[2] if len(sys.argv)>2 else 'smoke')
