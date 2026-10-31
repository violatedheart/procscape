#!/usr/bin/env python3
"""Fetch the pinned packaging tool into a build directory, verifying its checksum."""
import hashlib
import io
from pathlib import Path
import sys
import tarfile
import urllib.request

VERSION = '2.48.0'
ASSET = f'nfpm_{VERSION}_Linux_x86_64.tar.gz'
BASE = f'https://github.com/goreleaser/nfpm/releases/download/v{VERSION}'


def main():
    output = Path(sys.argv[1] if len(sys.argv) > 1 else '/tmp/procscape-tools')
    output.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(f'{BASE}/checksums.txt', timeout=60) as response:
        entries = dict(line.split()[::-1] for line in response.read().decode().splitlines() if len(line.split()) == 2)
    with urllib.request.urlopen(f'{BASE}/{ASSET}', timeout=60) as response:
        archive = response.read()
    if hashlib.sha256(archive).hexdigest() != entries.get(ASSET):
        raise RuntimeError('nFPM archive checksum does not match the upstream checksum file')
    # Extract only the expected executable, without honoring archive paths.
    with tarfile.open(fileobj=io.BytesIO(archive), mode='r:gz') as tar:
        source = tar.extractfile('nfpm')
        if source is None:
            raise RuntimeError('nFPM executable not found')
        binary = output / 'nfpm'
        binary.write_bytes(source.read())
        binary.chmod(0o755)
    print(binary)


if __name__ == '__main__':
    main()
