"""lab.parity_fixtures() builds the parity fixtures' binary and runs only it, never another Cargo artifact."""
import json
import subprocess
import unittest
from unittest import mock

from tools import lab


def artifact(name, executable, kind='bin'):
    return json.dumps(dict(reason='compiler-artifact', executable=executable, target=dict(name=name, kind=[kind])))


class ParityFixturesBuildTests(unittest.TestCase):
    def setUp(self):
        patcher = mock.patch.dict(lab._PARITY_FIXTURES, clear=True)
        patcher.start()
        self.addCleanup(patcher.stop)

    def build(self, *rows):
        return subprocess.CompletedProcess([], 0, stdout='\n'.join(rows))

    def test_only_the_fixtures_executable_is_taken(self):
        rows = ['', artifact('sunrise-client', '/t/sunrise-client'),
                artifact('build-script-build', '/t/build-script'),
                json.dumps(dict(reason='compiler-artifact', executable=None, target=dict(name='sunrise_client'))),
                artifact('sunrise-parity-fixtures', '/t/sunrise-parity-fixtures'),
                json.dumps(dict(reason='build-finished', success=True))]
        with mock.patch.object(lab.subprocess, 'run', return_value=self.build(*rows)):
            self.assertEqual(lab.parity_fixtures(), lab.Path('/t/sunrise-parity-fixtures'))

    def test_each_variant_builds_once_with_its_features(self):
        built = {'fixtures,client-defaults': '/t/default', 'fixtures': '/t/vanilla'}

        def run(command, **kwargs):
            return self.build(artifact('sunrise-parity-fixtures', built[command[command.index('--features') + 1]]))

        with mock.patch.object(lab.subprocess, 'run', side_effect=run) as cargo:
            for _ in range(2):
                self.assertEqual(lab.parity_fixtures(), lab.Path('/t/default'))
                self.assertEqual(lab.parity_fixtures(default_features=False), lab.Path('/t/vanilla'))
        self.assertEqual(cargo.call_count, 2)
        for call in cargo.call_args_list:
            command = call.args[0]
            self.assertEqual(command[command.index('-p') + 1], 'sunrise-parity-fixtures')
            self.assertEqual(command[command.index('--bin') + 1], 'sunrise-parity-fixtures')
            self.assertNotIn('test', command[:2])

    def test_no_executable_or_several_is_refused(self):
        for rows in ([artifact('sunrise-client', '/t/sunrise-client')],
                     [artifact('sunrise-parity-fixtures', '/t/a'), artifact('sunrise-parity-fixtures', '/t/b')]):
            with self.subTest(rows=rows), mock.patch.object(lab.subprocess, 'run', return_value=self.build(*rows)):
                with self.assertRaisesRegex(RuntimeError, 'sunrise-parity-fixtures executables'):
                    lab.parity_fixtures()
            self.assertEqual(lab._PARITY_FIXTURES, {})


class ParityFixturesManifestTests(unittest.TestCase):
    def test_the_package_follows_the_clients_manifest(self):
        # Its dependencies and features are sunrise-client's: a client change that misses it breaks only the daily.
        from tools.ci import parity_fixtures_manifest
        self.assertEqual(parity_fixtures_manifest.main(['--check']), 0)

    def test_the_package_has_every_client_dependency_and_feature_behind_fixtures(self):
        import tomllib
        from tools.ci import parity_fixtures_manifest as manifest
        client = tomllib.loads(manifest.CLIENT.read_text(encoding='utf-8'))
        fixtures = tomllib.loads(manifest.render())
        names = set(client['dependencies']) | set(client['dev-dependencies'])
        self.assertEqual(set(fixtures['dependencies']), names)
        self.assertTrue(all(entry['optional'] for entry in fixtures['dependencies'].values()))
        enabled = set(fixtures['features']['fixtures'])
        self.assertTrue({f'dep:{name}' for name, entry in client['dependencies'].items()
                         if not (isinstance(entry, dict) and entry.get('optional'))} <= enabled)
        self.assertTrue({f'dep:{name}' for name in client['dev-dependencies']} <= enabled)
        self.assertEqual(fixtures['features']['client-defaults'], client['features']['default'])
        self.assertNotIn('default', fixtures['features'])
        self.assertEqual(fixtures['dependencies']['sunrise-client-lib']['features'],
                         client['dev-dependencies']['sunrise-client-lib']['features'])


if __name__ == '__main__':
    unittest.main()
