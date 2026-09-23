#!/usr/bin/env python3
"""After a funding refusal, restoring the account alone does not renew the
authorization; re-authorizing the profile (a new authorization revision)
does, and the profile launches again (0.4.1 S6b). Never invokes a model
provider; the provider executables are fixtures."""
import json
import sys

sys.dont_write_bytecode = True
from provider_fixture import Fixture
from funding_safety import launched, reauthorize, refused, refusals, run


def scenario(binary, provider):
    f = Fixture(binary)
    try:
        if provider == 'codex':
            switch = f.root / 'codex-account.json'
            switch.write_text(json.dumps({'type': 'chatgpt', 'planType': 'plus',
                                          'email': 'other@example.invalid'}))
            refused(f, run(f), 'the Codex account changed')
            switch.unlink()
        else:
            auth = f.root / 'auth.json'
            original = auth.read_text()
            value = json.loads(original)
            value['email'] = 'other@example.invalid'
            auth.write_text(json.dumps(value))
            refused(f, run(f), 'account changed')
            auth.write_text(original)
        refused(f, run(f), 'was refused')
        reauthorize(f)
        launched(f, run(f))
        assert [r for (r, _) in refusals(f)] == [1], refusals(f)
    finally:
        f.cleanup()


if __name__ == '__main__':
    scenario(sys.argv[1], sys.argv[2])
