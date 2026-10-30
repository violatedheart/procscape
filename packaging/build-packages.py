#!/usr/bin/env python3
"""Build a distro-native x86_64 package; never relabel a foreign binary."""
import argparse
import hashlib
import os
from pathlib import Path
import platform
import re
import shlex
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent


PACKAGE_TARGET = ROOT / 'target' / 'packages'


def release_environment(environ=None, root=ROOT, home=None):
    """Apply path remapping to every crate, including cached dependencies."""
    env = dict(os.environ if environ is None else environ)
    home = Path.home() if home is None else Path(home)
    paths = [
        (home, '/build/user'),
        (Path(env.get('CARGO_HOME', home / '.cargo')).expanduser(), '/build/cargo'),
        (Path(env.get('RUSTUP_HOME', home / '.rustup')).expanduser(), '/build/rustup'),
        (Path(root), '/build/procscape'),
    ]
    mappings = {}
    for source, replacement in paths:
        for path in (source.absolute(), source.resolve()):
            if path != Path('/'):
                mappings[str(path)] = replacement
    # rustc uses the last matching remap, so put specific prefixes last.
    if 'CARGO_ENCODED_RUSTFLAGS' in env:
        flags = env['CARGO_ENCODED_RUSTFLAGS'].split('\x1f') if env['CARGO_ENCODED_RUSTFLAGS'] else []
    else:
        flags = shlex.split(env.get('RUSTFLAGS', ''))
    flags += [f'--remap-path-prefix={source}={replacement}'
              for source, replacement in sorted(mappings.items(), key=lambda pair: len(pair[0]))]
    env['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(flags)
    env.pop('RUSTFLAGS', None)
    return env, tuple(mappings)


def check_binary_privacy(directory, forbidden_paths):
    """Reject host paths even when --skip-build is requested."""
    personal_path = re.compile(rb'/(?:home|Users)/[^/\x00\s]+')
    for name in ('procscape', 'procscape-gui'):
        path = Path(directory) / name
        if not path.is_file():
            raise RuntimeError(f'{name}: no package build found; rerun without --skip-build')
        data = path.read_bytes()
        if personal_path.search(data) or any(os.fsencode(prefix) in data for prefix in forbidden_paths):
            raise RuntimeError(f'{name}: embedded build-machine path detected; rebuild with this package builder before distributing')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--format', choices=['rpm', 'deb'], required=True)
    parser.add_argument('--output', type=Path, default=ROOT / 'dist')
    parser.add_argument('--skip-build', action='store_true', help='Reuse target/packages/release binaries; privacy checks still run')
    args = parser.parse_args()
    distro = platform.freedesktop_os_release()
    supported = {'rpm': ('fedora', '44'), 'deb': ('debian', '13')}
    expected = supported[args.format]
    if (distro.get('ID'), distro.get('VERSION_ID')) != expected:
        parser.error(f"{args.format} packages must be built on {expected[0]} {expected[1]}; use the release workflow's matching container")
    if platform.machine() != 'x86_64':
        parser.error('Release packages currently support x86_64 only')
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['package']['version']
    if os.environ.get('GITHUB_REF_TYPE') == 'tag' and os.environ.get('GITHUB_REF_NAME') != f'v{version}':
        parser.error('The release tag must match the version in Cargo.toml')
    env, forbidden_paths = release_environment()
    binary_dir = PACKAGE_TARGET / 'release'
    if not args.skip_build:
        subprocess.run(['cargo', 'build', '--release', '--locked', '--features', 'gui', '--bins',
                        '--target-dir', str(PACKAGE_TARGET)], cwd=ROOT, env=env, check=True)
    check_binary_privacy(binary_dir, forbidden_paths)
    args.output.mkdir(parents=True, exist_ok=True)
    output = args.output.resolve() / f'procscape_{version}_{expected[0]}{expected[1]}_x86_64.{args.format}'
    env.update(PROC_VERSION=version)
    env.setdefault('PACKAGE_MAINTAINER', 'ProcScape contributors')
    subprocess.run([env.get('NFPM', 'nfpm'), 'package', '--config', 'packaging/nfpm.yaml', '--packager', args.format, '--target', str(output)], cwd=ROOT, env=env, check=True)
    checksum = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_suffix(output.suffix + '.sha256').write_text(f'{checksum}  {output.name}\n')
    print(output)


if __name__ == '__main__':
    main()
