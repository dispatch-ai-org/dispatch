#!/usr/bin/env python3
"""Standard-library duplex Dispatch client. A human must provision the grant.

python3 examples/control_client.py --binary target/release/dispatch \
    --state-dir /private/path/state --grant /private/path/state/control-grants/ID.key \
    --task 'Add the requested test'

For a deterministic, no-model demonstration see tests/fixtures/phase5_control.py.
"""
import argparse
import json
import os
import queue
import subprocess
import threading
import uuid


class Client:
    def __init__(self, binary, state, grant, read_only=False, initialize=True):
        with open(grant, 'rb') as handle:
            args = [str(binary), '--state-dir', str(state), 'control', '--stdio',
                    '--grant-fd', str(handle.fileno())]
            if read_only:
                args.append('--read-only')
            self.process = subprocess.Popen(args, pass_fds=(handle.fileno(),),
                                            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                            stderr=subprocess.PIPE)
        self.incoming = queue.Queue()
        self.replies = {}
        self.events = []
        self.serial = 0
        self.prefix = uuid.uuid4().hex
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()
        if initialize:
            self.hello = self.call('initialize')

    def _read(self):
        try:
            for line in self.process.stdout:
                self.incoming.put(json.loads(line))
        except Exception as error:
            self.incoming.put(error)
        finally:
            self.incoming.put(EOFError('control output closed'))

    def send(self, op, request_id=None, **fields):
        self.serial += 1
        request_id = request_id or self.prefix + '-' + str(self.serial)
        message = dict(protocol_version=1, request_id=request_id, op=op, **fields)
        self.process.stdin.write((json.dumps(message) + '\n').encode())
        self.process.stdin.flush()
        return request_id

    def receive(self, request_id, timeout=20):
        while request_id not in self.replies:
            message = self.incoming.get(timeout=timeout)
            if isinstance(message, BaseException):
                raise message
            if message['type'] == 'event':
                self.events.append(message)
            else:
                self.replies[message['request_id']] = message
        return self.replies.pop(request_id)

    def call(self, op, request_id=None, **fields):
        reply = self.receive(self.send(op, request_id=request_id, **fields))
        if not reply['ok']:
            raise RuntimeError(reply['error'])
        return reply['result']

    def close(self):
        if self.process.stdin and not self.process.stdin.closed:
            self.process.stdin.close()  # Foreground EOF requests controlled cleanup.
        try:
            self.process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
            raise
        diagnostics = self.process.stderr.read().decode(errors='replace')
        self.reader.join(timeout=2)
        self.process.stdout.close()
        self.process.stderr.close()
        return self.process.returncode, diagnostics


def workflow(client, task, factual_answer=None):
    accepted = client.call('submit', task=task)
    run_id = accepted['run_id']
    subscription = client.send('subscribe', run_id=run_id, after=0, timeout_ms=1000)
    attention = client.call('await', run_id=run_id, after=accepted['cursor'],
                            predicate='attention_required', timeout_ms=10000)
    status = client.call('status', run_id=run_id)
    if status['outcome']['waiting_on'] == 'human':
        question = status['phase3']['questions'][-1]
        if factual_answer is None:
            raise RuntimeError('A delegated factual answer is required: ' + question['report']['question'])
        # Demonstrate an outstanding wait that does not block the answer.
        finished = client.send('await', run_id=run_id, after=status['cursor'],
                               predicate='execution_finished', timeout_ms=10000)
        client.call('answer', run_id=run_id, question_id=question['id'],
                    revision=question['revision'], generation=question['generation'],
                    answer=factual_answer)
        client.receive(finished)
    elif not attention['reached']:
        raise RuntimeError('Goal did not reach an actionable state within the example timeout')
    result = client.call('result', run_id=run_id)
    client.receive(subscription)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', default='dispatch')
    parser.add_argument('--state-dir', required=True)
    parser.add_argument('--grant', required=True)
    parser.add_argument('--task', required=True)
    parser.add_argument('--factual-answer')
    args = parser.parse_args()
    client = Client(args.binary, args.state_dir, args.grant)
    try:
        print(json.dumps(workflow(client, args.task, args.factual_answer), indent=2))
    finally:
        code, diagnostics = client.close()
        if diagnostics:
            print(diagnostics, file=__import__('sys').stderr)
        if code:
            raise SystemExit(code)


if __name__ == '__main__':
    main()
