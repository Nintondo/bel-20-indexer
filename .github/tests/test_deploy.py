"""Failure-injection tests for the real deployment script; no SSH or secrets."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = json.loads((Path(__file__).with_name('deployment-fixture.json')).read_text())
FIXTURE = FIXTURES[int(os.environ.get('DEPLOY_FIXTURE_INDEX','0'))]

FAKE_DOCKER = r'''#!/usr/bin/env python3
import json, os, re, sys
from pathlib import Path
a = sys.argv[1:]
p = Path(os.environ['FAKE_STATE'])
s = json.loads(p.read_text())
with Path(os.environ['FAKE_LOG']).open('a') as f:
    f.write(json.dumps(a) + '\n')
def image():
    text = Path(os.environ['FAKE_TARGET']).read_text()
    return re.search(r'^\s*image:\s*(\S+)', text, re.M)[1]
def save():
    p.write_text(json.dumps(s))
if a[0] == 'compose':
    expected = os.environ['FAKE_ROOT_COMPOSE']
    assert '-f' in a and a[a.index('-f')+1] == expected, a
    assert Path(expected).is_file(), expected
    if 'config' in a:
        if os.environ.get('FAIL_CONFIG') == 'true' and image().endswith('b'*40): sys.exit(8)
        if '--format' in a: print(json.dumps({'services':{os.environ['FAKE_SERVICE']:{'image':image()}}}))
    elif 'up' in a:
        assert '--no-deps' in a and '--force-recreate' in a, a
        if image().endswith('a'*40) and os.environ.get('FAIL_ROLLBACK') == 'true': sys.exit(9)
        s['image'] = image(); s['generation'] += 1; save()
    elif 'run' in a:
        assert '--no-deps' in a and '--rm' in a and '--entrypoint' in a, a
        if os.environ.get('FAIL_MIGRATION') == 'true': sys.exit(10)
elif a[0] == 'inspect':
    fmt = a[a.index('-f')+1]
    if '.Config.Image' in fmt: print(s['image'])
    else:
        old = s['image'].endswith('a'*40)
        health = ('missing' if os.environ.get('LEGACY') == 'true' else 'healthy') if old else os.environ.get('NEW_HEALTH','healthy')
        print('running ' + health)
elif a[:2] == ['image','inspect']: print('b'*40)
elif a[0] == 'run': print('Bellscoin version -g' + 'b'*40)
elif a[0] == 'exec':
    if os.environ.get('FAIL_LEGACY_PROBE') == 'true': sys.exit(11)
elif a[0] != 'pull': raise AssertionError(a)
'''

class DeployTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.bin = self.root / 'bin'; self.bin.mkdir()
        self.env = dict(os.environ, **FIXTURE['env'])
        self.env.update(DEPLOYMENT_ID='test_run', DEPLOY_PHASE='deploy', HEALTH_TIMEOUT_SECONDS='3', RELOAD_NGINX='false')
        self.env['PATH'] = str(self.bin) + os.pathsep + os.environ['PATH']
        self.env['MAINTENANCE_NETWORKS'] = 'bells-testnet'
        self.env['RUN_MIGRATIONS'] = 'true'
        self.service = self.root / FIXTURE['service_path'].lstrip('/')
        self.service.mkdir(parents=True)
        self.base = self.root / FIXTURE['base_path'].lstrip('/')
        self.base.mkdir(parents=True, exist_ok=True)
        self.target = self.service / FIXTURE['compose_file']
        self.old_image = 'registry.example/project/' + self.env['IMAGE_NAME_OVERRIDE'] + ':' + 'a'*40
        self.new_image = 'registry.example/project/' + self.env['IMAGE_NAME_OVERRIDE'] + ':' + 'b'*40
        self.target.write_text(f'services:\n  {FIXTURE["compose_service"]}:\n    image: {self.old_image}\n')
        root_compose = self.root / FIXTURE['root_compose'].lstrip('/')
        if root_compose != self.target:
            root_compose.write_text('include:\n  - services/service/compose.yml\n')
        self.originals = {self.target:self.target.read_bytes()}
        for relative in FIXTURE.get('files', []):
            target = self.service / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text('ORIGINAL\n')
            self.originals[target] = target.read_bytes()
        for relative in FIXTURE.get('staged_files', []):
            target = self.service / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            content = f'services:\n  {FIXTURE["compose_service"]}:\n    image: {self.new_image}\n' if relative.endswith(('.yml.updated','.yaml.updated')) else 'UPDATED\n'
            target.write_text(content)
        if self.env.get('SERVICE_PATH_INPUT'):
            self.env['SERVICE_PATH_INPUT'] = str(self.service)
        self.state = self.root / 'state.json'
        self.state.write_text(json.dumps(dict(image=self.old_image, generation=0)))
        self.log = self.root / 'docker.jsonl'
        self.env.update(FAKE_STATE=str(self.state), FAKE_LOG=str(self.log), FAKE_TARGET=str(self.target), FAKE_SERVICE=FIXTURE['compose_service'], FAKE_ROOT_COMPOSE=str(root_compose))
        self.write_executable('docker', FAKE_DOCKER)
        self.write_executable('sleep', '#!/bin/sh\nexit 0\n')
        self.write_executable('timeout', '#!/bin/sh\nshift\nexec "$@"\n')
        # Linux CI uses real flock; macOS has no flock and tests the persistent lock.
        if not shutil.which('flock'):
            self.write_executable('flock', '#!/bin/sh\nexit 0\n')
        if os.uname().sysname == 'Darwin':
            self.write_executable('sed', r'''#!/usr/bin/env python3
import re,sys
from pathlib import Path
a=sys.argv[1:]; assert a[0]=='-i'
_, pattern, replacement, _ = a[1].split('|')
p=Path(a[2]); p.write_text(re.sub(pattern,replacement,p.read_text()))
''')
        self.script = self.root / 'deploy.sh'
        source = (ROOT / '.github/actions/deploy-over-ssh/deploy.sh').read_text()
        self.script.write_text(source.replace('/opt/', str(self.root / 'opt')+'/'))
        self.transaction = self.service / '.deploy-transaction'

    def tearDown(self):
        self.temp.cleanup()

    def write_executable(self, name, text):
        path = self.bin / name; path.write_text(text); path.chmod(0o755)

    def run_phase(self, phase='deploy', **env):
        result = subprocess.run([os.environ.get('DEPLOY_TEST_BASH','bash'), str(self.script)], env=dict(self.env, DEPLOY_PHASE=phase, **env), text=True, capture_output=True, timeout=15)
        return result

    def assert_success(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def assert_originals_restored(self):
        self.assertEqual(json.loads(self.state.read_text())['image'], self.old_image)
        for path, contents in self.originals.items():
            self.assertEqual(path.read_bytes(), contents, str(path))
        self.assertFalse((self.base / '.env').exists() and self.base != self.service, 'Environment restored into wrong directory')

    def test_success_keeps_backups_until_commit(self):
        self.assert_success(self.run_phase())
        self.assertTrue(self.transaction.is_dir())
        self.assertEqual(self.transaction.stat().st_mode & 0o777, 0o700)
        self.assert_success(self.run_phase('commit'))
        self.assertFalse(self.transaction.exists())
        self.assertEqual(json.loads(self.state.read_text())['image'], self.new_image)

    def test_unhealthy_image_restores_all_service_files(self):
        result = self.run_phase(NEW_HEALTH='unhealthy')
        self.assertNotEqual(result.returncode, 0)
        self.assert_originals_restored()
        self.assertTrue((self.transaction / 'restored').exists())

    def test_public_probe_failure_can_restore_pending_deployment(self):
        self.assert_success(self.run_phase())
        # Public curl runs in CI after SSH has returned, then triggers rollback.
        self.assert_success(self.run_phase('rollback'))
        self.assert_originals_restored()
        self.assertTrue(self.transaction.is_dir())

    def test_failed_restore_retains_backups_and_can_be_retried(self):
        self.assert_success(self.run_phase())
        result = self.run_phase('rollback', FAIL_ROLLBACK='true')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('ROLLBACK FAILED', result.stderr)
        self.assertTrue(self.transaction.is_dir())
        self.assertFalse((self.transaction / 'restored').exists())
        self.assert_success(self.run_phase('rollback'))
        self.assert_originals_restored()

    def test_pending_transaction_blocks_another_run(self):
        self.assert_success(self.run_phase())
        before = self.log.read_text()
        result = self.run_phase(DEPLOYMENT_ID='another_run')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.log.read_text(), before)
        self.assert_success(self.run_phase('rollback', DEPLOYMENT_ID='another_run'))
        self.assertEqual(self.log.read_text(), before)

    def test_legacy_image_recovery_uses_existing_probe(self):
        self.assert_success(self.run_phase(LEGACY='true'))
        self.assert_success(self.run_phase('rollback', LEGACY='true'))
        self.assert_originals_restored()
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertGreaterEqual(sum(call[0]=='exec' for call in calls), 3)

    def test_failed_legacy_probe_never_reports_recovery_success(self):
        self.assert_success(self.run_phase(LEGACY='true'))
        result = self.run_phase('rollback', LEGACY='true', FAIL_LEGACY_PROBE='true')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.transaction / 'restored').exists())

    def test_new_image_without_healthcheck_is_rejected(self):
        result = self.run_phase(NEW_HEALTH='missing')
        self.assertNotEqual(result.returncode, 0)
        self.assert_originals_restored()

    def test_configuration_failure_restores_before_recreation(self):
        result = self.run_phase(FAIL_CONFIG='true')
        self.assertNotEqual(result.returncode, 0)
        self.assert_originals_restored()

    def test_new_environment_is_removed_on_rollback(self):
        path = self.service / '.env'
        if '.env' not in FIXTURE.get('files', []): self.skipTest('No staged environment for this service')
        path.unlink(); self.originals.pop(path)
        self.assert_success(self.run_phase())
        self.assertTrue(path.exists())
        self.assert_success(self.run_phase('rollback'))
        self.assertFalse(path.exists())
        self.assert_originals_restored()

    def test_migration_failure_does_not_install_new_container(self):
        if FIXTURE['env'].get('RUN_MIGRATIONS') is None: self.skipTest('No database migration in this service')
        result = self.run_phase(FAIL_MIGRATION='true')
        self.assertNotEqual(result.returncode, 0)
        self.assert_originals_restored()

    def test_commit_rechecks_health_and_image(self):
        self.assert_success(self.run_phase())
        result = self.run_phase('commit', NEW_HEALTH='unhealthy')
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(self.transaction.is_dir())
        self.assert_success(self.run_phase('rollback'))

    def test_invalid_image_tag_is_rejected_before_mutation(self):
        result = self.run_phase(SERVICE_TAG='latest')
        self.assertNotEqual(result.returncode, 0)
        self.assert_originals_restored()
        self.assertFalse(self.log.exists())
        self.assertFalse(self.transaction.exists())

    def test_paths_cannot_escape_the_service(self):
        result = self.run_phase(DOCKER_COMPOSE_FILE='../other-service.yml')
        self.assertNotEqual(result.returncode, 0)
        self.assert_originals_restored()
        self.assertFalse(self.log.exists())

    def test_failed_snapshot_does_not_restart_service_or_leave_pending_deploy(self):
        self.write_executable('cp', '#!/bin/sh\nexit 13\n')
        result = self.run_phase()
        self.assertNotEqual(result.returncode, 0)
        self.assert_originals_restored()
        self.assertFalse(self.transaction.exists())
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertFalse(any(call[0]=='compose' and 'up' in call for call in calls))

    def test_restored_deployment_cannot_be_committed(self):
        self.assert_success(self.run_phase())
        self.assert_success(self.run_phase('rollback'))
        self.assertNotEqual(self.run_phase('commit').returncode, 0)
        self.assertTrue(self.transaction.is_dir())
        self.assert_originals_restored()

    def test_maintenance_only_works_with_existing_frontend_image(self):
        if FIXTURE['env'].get('MAINTENANCE_ONLY') is None:
            self.skipTest('Frontend maintenance only')
        self.assert_success(self.run_phase(MAINTENANCE_ONLY='true', SERVICE_TAG='', LEGACY='true'))
        self.assertEqual(json.loads(self.state.read_text())['image'], self.old_image)
        self.assert_success(self.run_phase('commit', MAINTENANCE_ONLY='true', SERVICE_TAG='', LEGACY='true'))

if __name__ == '__main__':
    if len(FIXTURES) > 1 and 'DEPLOY_FIXTURE_INDEX' not in os.environ:
        for index in range(len(FIXTURES)):
            print('Testing layout:', FIXTURES[index]['service_path'], flush=True)
            result = subprocess.run([__import__('sys').executable, __file__], env=dict(os.environ, DEPLOY_FIXTURE_INDEX=str(index)))
            if result.returncode: raise SystemExit(result.returncode)
    else:
        unittest.main()
