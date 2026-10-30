#!/usr/bin/env python3
"""Build a distro-native x86_64 package; never relabel a foreign binary."""
import argparse
import hashlib
import os
from pathlib import Path
import platform
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--format', choices=['rpm', 'deb'], required=True)
    parser.add_argument('--output', type=Path, default=ROOT / 'dist')
    parser.add_argument('--skip-build', action='store_true', help='Package existing release binaries built on this distribution')
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
    if not args.skip_build:
        subprocess.run(['cargo', 'build', '--release', '--locked', '--features', 'gui', '--bins'], cwd=ROOT, check=True)
    args.output.mkdir(parents=True, exist_ok=True)
    output = args.output.resolve() / f'procscape_{version}_{expected[0]}{expected[1]}_x86_64.{args.format}'
    env = dict(os.environ, PROC_VERSION=version)
    env.setdefault('PACKAGE_MAINTAINER', 'ProcScape contributors')
    subprocess.run([env.get('NFPM', 'nfpm'), 'package', '--config', 'packaging/nfpm.yaml', '--packager', args.format, '--target', str(output)], cwd=ROOT, env=env, check=True)
    checksum = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_suffix(output.suffix + '.sha256').write_text(f'{checksum}  {output.name}\n')
    print(output)


if __name__ == '__main__':
    main()
