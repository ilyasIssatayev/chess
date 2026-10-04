#!/usr/bin/env python3
"""Install the pinned official ONNX Runtime CPU library for macOS arm64."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import platform
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = json.loads((ROOT / 'models/native-runtime.json').read_text())
DEST = ROOT / 'local-data/vision/native'


def verified(body, sha, size):
    if len(body) != size or hashlib.sha256(body).hexdigest() != sha:
        raise ValueError('Native runtime integrity check failed.')
    return body


def installed(entry):
    path = DEST / entry['file']
    return path.is_file() and path.stat().st_size == entry['bytes'] and hashlib.sha256(path.read_bytes()).hexdigest() == entry['sha256']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if platform.system() != 'Darwin' or platform.machine() != 'arm64':
        parser.exit(1, 'This pinned runtime is for macOS arm64 only. No other platform has been qualified.\n')
    if all(installed(e) for e in MANIFEST['files']):
        print('Native CPU runtime verified: ' + MANIFEST['version'])
        return 0
    if args.check:
        parser.exit(1, 'Native runtime missing or invalid. Run python3 scripts/setup-native-vision.py.\n')
    archive_path = DEST / 'runtime.tgz'
    body = archive_path.read_bytes() if archive_path.is_file() else None
    if body is None or len(body) != MANIFEST['archive_bytes'] or hashlib.sha256(body).hexdigest() != MANIFEST['archive_sha256']:
        print('Downloading ' + MANIFEST['url'], flush=True)
        with urllib.request.urlopen(MANIFEST['url'], timeout=60) as response:
            body = response.read(MANIFEST['archive_bytes'] + 1)
    verified(body, MANIFEST['archive_sha256'], MANIFEST['archive_bytes'])
    with tarfile.open(fileobj=io.BytesIO(body)) as archive:
        # Validate all explicitly allowed members before replacing any asset.
        files = [(e, verified(archive.extractfile(e['member']).read(), e['sha256'], e['bytes'])) for e in MANIFEST['files']]
    DEST.mkdir(parents=True, exist_ok=True)
    for entry, data in files:
        path = DEST / entry['file']
        temporary = path.with_suffix(path.suffix + '.part')
        temporary.write_bytes(data)
        temporary.replace(path)
    print('Native CPU runtime installed: ' + MANIFEST['version'])
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
