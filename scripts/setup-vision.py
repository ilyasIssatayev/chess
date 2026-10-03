#!/usr/bin/env python3
"""Install immutable, hash-checked local models and ONNX Runtime assets."""
import argparse
import base64
import hashlib
import io
import json
from pathlib import Path
import tarfile
import urllib.request
import importlib.util

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = json.loads((ROOT / 'models/vision-manifest.json').read_text())
DEST = ROOT / 'local-data/vision'
spec = importlib.util.spec_from_file_location('vision_features', ROOT / 'scripts/vision-features.py')
features = importlib.util.module_from_spec(spec)
spec.loader.exec_module(features)


def valid(path, entry):
    return path.is_file() and path.stat().st_size == entry['bytes'] and hashlib.sha256(path.read_bytes()).hexdigest() == entry['sha256']


def save(body, path, entry):
    if len(body) != entry['bytes'] or hashlib.sha256(body).hexdigest() != entry['sha256']:
        raise RuntimeError('Asset hash mismatch: ' + entry['file'])
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + '.part')
    temporary.write_bytes(body)
    temporary.replace(path)


def download(url):
    print('Downloading', url, flush=True)
    with urllib.request.urlopen(url, timeout=120) as response:
        return response.read()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Verify installed files without network access')
    args = parser.parse_args()
    entries = [(DEST / 'models' / e['file'], e) for e in MANIFEST['models']]
    entries += [(DEST / 'runtime' / e['file'], e) for e in MANIFEST['runtime']['files']]
    missing = [(p, e) for p, e in entries if not valid(p, e)]
    if args.check:
        if missing:
            print('Missing or invalid vision assets: ' + ', '.join(e['file'] for _, e in missing))
            return 1
        print('Local neural recognition assets verified: ' + MANIFEST['version'])
        return 0
    for entry in MANIFEST['models']:
        path = DEST / 'models' / entry['file']
        if not valid(path, entry):
            source = {'file': entry['file'], 'bytes': entry['source_bytes'], 'sha256': entry['source_sha256']}
            body = path.read_bytes() if valid(path, source) else download(entry['url'])
            if len(body) != source['bytes'] or hashlib.sha256(body).hexdigest() != source['sha256']:
                raise RuntimeError('Source model integrity mismatch: ' + entry['file'])
            save(features.expose_features(body, MANIFEST['features']['tensor'], MANIFEST['features']['dimension']), path, entry)
    runtime = MANIFEST['runtime']
    if any(not valid(DEST / 'runtime' / e['file'], e) for e in runtime['files']):
        archive = download(runtime['url'])
        integrity = 'sha512-' + base64.b64encode(hashlib.sha512(archive).digest()).decode()
        if integrity != runtime['integrity']:
            raise RuntimeError('ONNX Runtime archive integrity mismatch')
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            for entry in runtime['files']:
                # Explicit files only; never extract arbitrary archive paths.
                save(tar.extractfile(entry['member']).read(), DEST / 'runtime' / entry['file'], entry)
    print('Local neural recognition assets ready: ' + MANIFEST['version'])
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
