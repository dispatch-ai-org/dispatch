"""Bounded child/PTY regression, no Dispatch state or providers."""
import errno
import os
import sys
import time
import unittest
from unittest.mock import patch
sys.dont_write_bytecode = True
from phase7_evidence import owner_process
import pty


class OwnerTests(unittest.TestCase):
    def exercise(self, script, expected=None, error=None):
        fork = pty.fork
        owned = []
        waitpid = os.waitpid
        def bounded_wait(pid, flags):
            self.assertEqual(flags, os.WNOHANG, "helper used a blocking reap")
            return waitpid(pid, flags)
        def capture():
            pid, fd = fork()
            if pid:
                owned.append((pid, fd))
            return pid, fd
        start = time.monotonic()
        with patch('phase7_evidence.pty.fork', capture), patch('phase7_evidence.os.waitpid', bounded_wait):
            if error:
                with self.assertRaisesRegex(RuntimeError, error):
                    owner_process([sys.executable, '-c', script], timeout=1)
            else:
                code, output = owner_process([sys.executable, '-c', script], timeout=5)
                self.assertEqual(code, expected, output)
        self.assertLess(time.monotonic()-start, 8, 'helper exceeded finite cleanup bound')
        self.assertEqual(len(owned), 1)
        pid, fd = owned[0]
        with self.assertRaises(ChildProcessError):
            os.waitpid(pid, os.WNOHANG)
        with self.assertRaises(OSError) as closed:
            os.fstat(fd)
        self.assertEqual(closed.exception.errno, errno.EBADF)

    def test_timeout_without_expected_prompt_kills_and_reaps(self):
        self.exercise("import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); print('unexpected prompt',flush=True); time.sleep(60)", error='timeout waiting')

    def test_premature_terminal_close_cleans_live_child(self):
        self.exercise("import os,signal,time; signal.signal(signal.SIGHUP,signal.SIG_IGN); signal.signal(signal.SIGTERM,signal.SIG_IGN); [os.close(fd) for fd in (0,1,2)]; time.sleep(60)", error='unexpected terminal closure')

    def test_successful_attestation_and_normal_exit(self):
        self.exercise("print('Type 0123456789ABCDEFGHIJKLMNOP to attest',flush=True); assert input() == '0123456789ABCDEFGHIJKLMNOP'", expected=0)

    def test_nonzero_exit_is_preserved(self):
        self.exercise("print('deliberate failure',flush=True); raise SystemExit(7)", expected=7)


if __name__ == '__main__':
    unittest.main()
