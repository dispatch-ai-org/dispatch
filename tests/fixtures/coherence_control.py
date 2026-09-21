#!/usr/bin/env python3
"""Deterministic real-pipe fixture for `result.coherence` over the control
protocol. Reuses the phase 5 fixture (fake codex agent, grant, duplex client)
and never invokes a model provider.

Each scenario submits one goal, waits for the finished, ready result, mutates
the source underneath it, and reads `status` and `result` again.
"""
import json
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
from phase5_control import Fixture, finish  # noqa: E402


def local_path_keys(value, found=None):
    """Every object key that the result projection promises to strip."""
    found = [] if found is None else found
    if isinstance(value, dict):
        for key, child in value.items():
            if key.endswith('_path') or key in ('path', 'patch', 'manifest'):
                found.append(key)
            local_path_keys(child, found)
    elif isinstance(value, list):
        for child in value:
            local_path_keys(child, found)
    return found


def git(root, *args):
    subprocess.run(['git', '-C', str(root), '-c', 'user.name=Test', '-c', 'user.email=t@example.invalid', *args],
                   check=True, capture_output=True)


def scenario(binary, name):
    f = Fixture(binary, 'success')
    try:
        if name == 'ignored':
            # A git source: ignored build output is not part of the world.
            (f.source / '.gitignore').write_text('target/\n')
            git(f.source, 'init', '--quiet')
            git(f.source, 'add', '-A')
            git(f.source, 'commit', '--quiet', '-m', 'initial')
        c = f.client()
        run_id = c.call('submit', task='Add tests in src/lib.rs')['run_id']
        before = finish(c, run_id)
        assert before['result']['outcome']['work_result'] == 'ready', before
        assert before['result']['exit_code'] == 0, before
        status_before = c.call('status', run_id=run_id)
        cursor = status_before['cursor']
        # A source that has not moved carries no coherence object at all.
        assert 'coherence' not in before['result'], before['result']

        if name == 'unchanged':
            again = c.call('result', run_id=run_id)
            assert 'coherence' not in again['result'], again
        elif name == 'ignored':
            (f.source / 'target').mkdir()
            (f.source / 'target/out.bin').write_bytes(b'build output')
            after = c.call('result', run_id=run_id)
            assert 'coherence' not in after['result'], after['result']
        elif name == 'unrelated':
            (f.source / 'notes.md').write_text('someone else\n')
            after = c.call('result', run_id=run_id)
            summary = after['result']['coherence']
            # The world moved, but nothing the patch depends on did.
            assert summary['decision'] == 'continue', summary
            assert summary['changed_files'] == 1, summary
            assert summary['reasons'] == [], summary
            assert summary['analysis'] in ('symbols', 'files_only', 'integration'), summary
        elif name == 'conflict':
            (f.source / 'src/lib.rs').write_text('// someone else got here first\n')
            after = c.call('result', run_id=run_id)
            summary = after['result']['coherence']
            assert summary['decision'] == 'refresh', summary
            assert summary['reasons'][0]['code'] == 'patch_conflict', summary
            assert summary['changed_files'] == 1, summary
            assert len(summary['reasons']) <= 5, summary
            # Local paths are stripped from the whole coherence object.
            assert local_path_keys(summary) == [], summary
            assert str(f.root) not in json.dumps(summary), summary
        else:
            raise AssertionError(name)

        if name in ('unrelated', 'conflict'):
            # A client that ignores unknown fields sees the same result as before.
            without = json.loads(json.dumps(after))
            del without['result']['coherence']
            assert set(without) == set(before), (set(without), set(before))
            assert set(without['result']) == set(before['result'])
            for key in ('run_id', 'schema_version', 'mode', 'exit_code', 'state_revision', 'outcome'):
                assert without['result'][key] == before['result'][key], key
            assert without['candidates'] and [x['label'] for x in without['candidates']] == \
                [x['label'] for x in before['candidates']]
            assert without['artifacts'] == before['artifacts']
        # Reading is display-only: status is unchanged and nothing was journaled.
        status_after = c.call('status', run_id=run_id)
        assert set(status_after) == set(status_before), status_after
        assert 'coherence' not in status_after, status_after
        assert status_after['cursor'] == cursor, (status_after['cursor'], cursor)
        assert status_after['state_revision'] == status_before['state_revision']
        assert status_after['outcome'] == status_before['outcome']
        stored = f.stored(run_id)
        assert stored.get('coherence') is None, stored.get('coherence')
        assert f.count() == 1
        print(name + ' passed: result.coherence is additive, path-free and never stored by reading')
    finally:
        f.cleanup()


if __name__ == '__main__':
    scenario(sys.argv[1], sys.argv[2])
