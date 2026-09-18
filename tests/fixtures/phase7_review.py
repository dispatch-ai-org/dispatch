"""Scripted simulations of owner review in disposable state; never real human labels."""
import hashlib
import json
import sqlite3
import sys
sys.dont_write_bytecode = True
from phase7_evidence import fixture, run, owner, stamp, summary
from review_refinement import Session, has_color


def exercise(binary, name):
    f, light, strong = fixture(binary, domain=False)
    try:
        args = [f.binary, '--state-dir', str(f.state), '--no-color']
        if name == 'plain': args += ['--plain']
        with Session(args, f.source, f.root/'captures', name) as session:
            session.wait('accomplish?')
            session.send('Add tests in src/lib.rs\r')
            session.wait('[y/N]'); session.send('y\r')
            session.wait('Review changes')
            with sqlite3.connect(f.state/'dispatch.db') as db:
                ident = db.execute('SELECT id FROM runs ORDER BY created_at DESC LIMIT 1').fetchone()[0]
            initial = f.stored(ident)
            def annotations():
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    return db.execute('SELECT feedback_id,payload_json FROM private_annotations WHERE run_id=?', (ident,)).fetchall()
            def annotate_provenance(origin):
                with sqlite3.connect(f.state/'dispatch.db') as db:
                    feedback = db.execute('SELECT id FROM goal_feedback_revisions WHERE run_id=? ORDER BY revision DESC LIMIT 1', (ident,)).fetchone()
                    db.execute('INSERT INTO private_annotations(run_id,feedback_id,payload_json,created_at) VALUES (?,?,?,?)',
                        (ident, feedback[0] if feedback else None, json.dumps({'origin':origin,'review':'scripted','delivery':'synthetic-test-marker','actor':'synthetic-test','repair_minutes':None}),stamp()))
            if name in ('synthetic', 'scripted_smoke'):
                annotate_provenance(name)
            if name == 'pending':
                code, _ = owner(f, 'evidence', 'annotate', ident, '--origin','ordinary','--review','human')
                assert code != 0 and not annotations()
                session.send('n\r'); session.wait('accomplish?')
                assert f.stored(ident)['outcome']['review'] == 'pending'
            else:
                if name == 'blocked':
                    (f.source/'src/lib.rs').write_text('// owner changed source\n')
                reject = name in ('reject', 'reject_attest')
                session.send('r\r' if reject else 'a\r')
                session.wait('Use this review for local routing')
                reviewed = f.stored(ident)
                expected = 'rejected' if reject else 'accepted'
                assert reviewed['outcome']['review'] == expected
                if name == 'blocked': assert reviewed['outcome']['application'] == 'blocked_by_source_drift'
                elif not reject: assert reviewed['outcome']['application'] == 'applied'
                if name not in ('accept', 'reject'):
                    # Pinning must survive a different session becoming the latest run.
                    if name == 'other_session':
                        other = run(f)
                        assert other['run_id'] != ident
                    session.send('f\r')
                    if name in ('synthetic', 'scripted_smoke'):
                        session.wait('experiment provenance cannot become ordinary work')
                    else:
                        session.wait('Back without recording')
                        assert 'reflects my own review' in session.clean
                        assert 'Review: '+expected.capitalize() in session.clean
                        with sqlite3.connect(f.state/'dispatch.db') as db:
                            feedback_id = db.execute('SELECT id FROM goal_feedback_revisions WHERE run_id=? ORDER BY revision DESC LIMIT 1',(ident,)).fetchone()[0]
                            if name == 'revision':
                                db.execute('INSERT INTO goal_feedback_revisions VALUES (?,?,?,?,?,?,?)', ('synthetic-correction',ident,2,expected,'[]',None,stamp()))
                            if name == 'delivery':
                                changed = json.loads(db.execute('SELECT run_projection_json FROM runs WHERE id=?',(ident,)).fetchone()[0])
                                changed['candidates'][0]['diff_stats']['lines_added'] += 1
                                db.execute('UPDATE runs SET run_projection_json=? WHERE id=?',(json.dumps(changed),ident))
                            if name == 'failure':
                                db.execute("CREATE TRIGGER synthetic_annotation_failure BEFORE INSERT ON private_annotations BEGIN SELECT RAISE(ABORT,'synthetic feedback failure'); END")
                        if name == 'provenance': annotate_provenance('scripted_smoke')
                        if name == 'cancel':
                            session.position = len(session.clean)
                            session.resize(65,24)
                            session.wait('Back without recording')
                            session.send(b'\x03')
                            session.wait('No local feedback recorded')
                        elif name == 'back':
                            session.send('b\r'); session.wait('No local feedback recorded')
                        else:
                            session.send('c\r')
                            failed = name in ('revision','delivery','failure','provenance')
                            session.wait('Local feedback not recorded:' if failed else 'Local routing evidence recorded.')
                            if not failed:
                                rows = annotations()
                                assert len(rows) == 1 and rows[0][0] == feedback_id
                                a = json.loads(rows[0][1])
                                payload = {'final':reviewed['phase3']['final_attempt_id'],'contributors':reviewed['phase3']['contributing_attempts'],
                                    'baseline':reviewed['baseline_commit'],'candidates':[[c['id'],c['diff_stats'],c['checks']] for c in reviewed['candidates']]}
                                digest = hashlib.sha256(json.dumps(payload,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
                                assert a['delivery'] == digest and a['origin'] == 'ordinary' and a['review'] == 'human'
                                if name == 'duplicate':
                                    session.wait('Use this review for local routing')
                                    session.send('f\r'); session.wait('Back without recording'); session.send('c\r')
                                    session.wait('Local routing evidence recorded.')
                                    assert len(annotations()) == 1
                                    code, log = owner(f, 'evidence', 'annotate', ident, '--origin','ordinary','--review','human')
                                    assert code == 0, log
                                    assert len(annotations()) == 1, 'CLI and presenter share idempotent persistence'
                                assert summary(f,ident)['counts']['reviewed'] == 1
                            elif name != 'provenance':
                                assert not annotations()
                                if name in ('failure', 'revision'):
                                    if name == 'failure':
                                        with sqlite3.connect(f.state/'dispatch.db') as db:
                                            db.execute('DROP TRIGGER synthetic_annotation_failure')
                                    session.wait('Use this review for local routing')
                                    session.send('f\r'); session.wait('Back without recording'); session.send('c\r')
                                    session.wait('Local routing evidence recorded.')
                                    assert len(annotations()) == 1
                                    if name == 'revision':
                                        assert annotations()[0][0] == 'synthetic-correction', 'retry needs a fresh explicit confirmation'
                if name not in ('delivery',):
                    assert f.stored(ident) == reviewed, 'feedback rewrote review/application/verification'
                if name in ('accept','reject','cancel','back'): assert not annotations()
            if name not in ('accept', 'reject', 'pending'):
                session.wait('accomplish?')
            session.send(b'\x04'); session.finish()
            assert not has_color(session.output)
            assert session.output.count(b'\x1b[?1049h') == session.output.count(b'\x1b[?1049l')
            assert f.count() == (4 if name == 'other_session' else 3)
            with sqlite3.connect(f.state/'dispatch.db') as db:
                assert db.execute('SELECT count(*) FROM pool_leases').fetchone()[0] == 0
                assert db.execute('SELECT count(*) FROM private_policy_transitions').fetchone()[0] == 0
                assert db.execute('SELECT count(*) FROM goal_feedback_revisions WHERE run_id=?',(ident,)).fetchone()[0] == (0 if name == 'pending' else 2 if name == 'revision' else 1)
            if name == 'other_session':
                assert f.stored(other['run_id'])['outcome']['review'] == 'pending'
        print('PASS synthetic owner-review simulation:', name)
    finally:
        f.cleanup()


if __name__ == '__main__':
    for name in sys.argv[2:]: exercise(sys.argv[1], name)
