"""Regression checks for path remapping and the package publication gate."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('package_builder', Path(__file__).resolve().parents[1] / 'packaging/build-packages.py')
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


class PackagePrivacyTests(unittest.TestCase):
    def test_remaps_home_checkout_and_custom_caches_without_losing_flags(self):
        env, paths = BUILDER.release_environment(
            {'RUSTFLAGS': '-C opt-level=2', 'CARGO_HOME': '/cache/private cargo', 'RUSTUP_HOME': '/toolchains/private'},
            root=Path('/home/builder/my project'), home=Path('/home/builder'))
        flags = env['CARGO_ENCODED_RUSTFLAGS'].split('\x1f')
        self.assertEqual(flags[:2], ['-C', 'opt-level=2'])
        self.assertIn('--remap-path-prefix=/home/builder=/build/user', flags)
        self.assertIn('--remap-path-prefix=/cache/private cargo=/build/cargo', flags)
        self.assertIn('--remap-path-prefix=/toolchains/private=/build/rustup', flags)
        self.assertIn('--remap-path-prefix=/home/builder/my project=/build/procscape', flags)
        self.assertLess(flags.index('--remap-path-prefix=/home/builder=/build/user'), flags.index('--remap-path-prefix=/home/builder/my project=/build/procscape'))
        self.assertNotIn('RUSTFLAGS', env)
        self.assertIn('/cache/private cargo', paths)

    def test_encoded_flags_keep_arguments_containing_spaces(self):
        env, _ = BUILDER.release_environment({'CARGO_ENCODED_RUSTFLAGS': '-L\x1fnative=/build/native libs', 'RUSTFLAGS': 'ignored'}, home=Path('/home/builder'))
        self.assertEqual(env['CARGO_ENCODED_RUSTFLAGS'].split('\x1f')[:2], ['-L', 'native=/build/native libs'])
        empty, _ = BUILDER.release_environment({'CARGO_ENCODED_RUSTFLAGS': '', 'RUSTFLAGS': 'ignored'}, home=Path('/home/builder'))
        self.assertNotIn('ignored', empty['CARGO_ENCODED_RUSTFLAGS'].split('\x1f'))

    def test_clean_binaries_pass_but_either_personal_binary_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name in ('procscape', 'procscape-gui'):
                (root / name).write_bytes(b'ELF\0/build/cargo/registry/source.rs\0/usr/lib\0')
            BUILDER.check_binary_privacy(root, ('/home/builder',))
            for name in ('procscape', 'procscape-gui'):
                with self.subTest(binary=name):
                    path = root / name
                    previous = path.read_bytes()
                    path.write_bytes(previous + b'/home/another-user/.cargo/registry/source.rs\0')
                    with self.assertRaisesRegex(RuntimeError, name + ': embedded build-machine path'):
                        BUILDER.check_binary_privacy(root, ('/home/builder',))
                    path.write_bytes(previous)

    def test_custom_non_home_path_is_also_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'procscape').write_bytes(b'/private-cache/account/source.rs')
            (root / 'procscape-gui').write_bytes(b'clean')
            with self.assertRaisesRegex(RuntimeError, 'embedded build-machine path'):
                BUILDER.check_binary_privacy(root, ('/private-cache/account',))

    def test_skip_build_cannot_package_missing_binaries(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaisesRegex(RuntimeError, 'rerun without --skip-build'):
                BUILDER.check_binary_privacy(temp, ())


if __name__ == '__main__':
    unittest.main()
