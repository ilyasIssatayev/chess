#!/usr/bin/env python3
"""Check Rust crops/tensors against JavaScript and CPU graphs against ONNX's reference evaluator.

Development check only. Requires native assets, built Rust tools, JavaScriptCore,
and a Python environment with onnx==1.19.1 and numpy==2.0.2. Images are local.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import zlib

ROOT = Path(__file__).resolve().parent.parent
JSC = '/System/Library/Frameworks/JavaScriptCore.framework/Versions/A/Helpers/jsc'
ATOL = 1e-4
RTOL = 1e-4
PROBABILITY_ATOL = 1e-5


def write_png(path, width=320, height=240):
    def chunk(name, body):
        return struct.pack('>I', len(body)) + name + body + struct.pack('>I', zlib.crc32(name + body))
    pixels = b''.join(b'\0' + bytes(channel for x in range(width) for channel in [(x * 13 + y * 7) % 256, (x * 3 + y * 19) % 256, (x * 29 + y) % 256]) for y in range(height))
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(pixels)) + chunk(b'IEND', b''))


def corners(points, shift=0):
    return {name: {'x': points[(i + shift) % 4][0], 'y': points[(i + shift) % 4][1]} for i, name in enumerate(['a8', 'h8', 'h1', 'a1'])}


def run(command, stdout=None):
    return subprocess.run([str(c) for c in command], cwd=ROOT, check=True, capture_output=stdout is None, stdout=stdout, timeout=180)


def js(script, path):
    return json.loads(run([JSC, ROOT / 'apps/recorder-ui-prototype/vision-core.js', ROOT / script, '--', path]).stdout)


def softmax(a, np):
    a = a.astype(np.float64)
    values = np.exp(a - a.max(axis=1, keepdims=True))
    return values / values.sum(axis=1, keepdims=True)


def check(output, photo=None):
    import numpy as np
    import onnx
    from onnx.reference import ReferenceEvaluator
    if onnx.__version__ != '1.19.1' or np.__version__ != '2.0.2':
        raise ValueError('Use the pinned reference environment: onnx==1.19.1, numpy==2.0.2.')
    if output.exists():
        raise ValueError('Output already exists. Choose a new check directory.')
    tools = ROOT / 'target/release'
    for name in ['vision-preprocess', 'vision-infer']:
        if not (tools / name).is_file():
            raise ValueError('Build first: cargo build --offline --release -p vision-inference')
    if not Path(JSC).is_file():
        raise ValueError('JavaScriptCore is required for the existing JavaScript reference.')
    output.mkdir(parents=True)
    image = output / 'synthetic.png'
    write_png(image)
    model_manifest = json.loads((ROOT / 'models/vision-manifest.json').read_text())
    evaluators = {}
    for entry in model_manifest['models']:
        path = ROOT / 'local-data/vision/models' / entry['file']
        if path.stat().st_size != entry['bytes'] or hashlib.sha256(path.read_bytes()).hexdigest() != entry['sha256']:
            raise ValueError('Reference model hash failed: ' + entry['file'])
        evaluators[entry['file'].split('.')[0]] = ReferenceEvaluator(str(path))
    cases = [(f'rotation-{i}', image, corners([(0.2, 0.2), (0.8, 0.2), (0.8, 0.8), (0.2, 0.8)], i)) for i in range(4)]
    cases += [('skew', image, corners([(0.08, 0.07), (0.88, 0.23), (0.96, 0.91), (0.21, 0.82)])),
              ('precise-calibration', image, corners([(0.12345678901234566, 0.13457482593840247), (0.8473456728945139, 0.1335432198765421), (0.8546231785432189, 0.8492167842543721), (0.11567932456874326, 0.8254927683129454)])),
              ('edge', image, corners([(0.001, 0.002), (0.995, 0.01), (0.99, 0.998), (0.004, 0.99)])),
              ('shallow', image, corners([(0.02, 0.48), (0.98, 0.49), (0.97, 0.57), (0.03, 0.56)]))]
    if photo:
        cases.append(('public-photo', photo, corners([(325 / 1200, 230 / 800), (790 / 1200, 232 / 800), (785 / 1200, 694 / 800), (321 / 1200, 690 / 800)])))
    report = {'schema_version': 1, 'report_kind': 'native_cpu_parity_development', 'model_version': model_manifest['version'],
              'runtime_version': '1.23.2', 'reference': {'onnx': onnx.__version__, 'numpy': np.__version__, 'preprocessing': 'vision-core.js'},
              'tolerances': {'logits_features_atol': ATOL, 'logits_features_rtol': RTOL, 'probabilities_atol': PROBABILITY_ATOL}, 'passed': False, 'cases': [],
              'limitations': ['No physical-board accuracy or phase/release qualification claim.', 'Pixel parity begins at decoded RGBA; JPEG/ICC/Canvas scaling parity is outside this check.', 'Base models only; personal heads and CoreML are not qualified.']}
    cache = {}
    try:
        for name, image_path, labels in cases:
            folder = output / name
            folder.mkdir()
            calibration = folder / 'corners.json'
            calibration.write_text(json.dumps(labels))
            stage_dir = folder / 'stages'
            run([tools / 'vision-preprocess', image_path, calibration, stage_dir])
            native_stages = json.loads((stage_dir / 'stages.json').read_text())
            reference_stages = js('scripts/tests/vision-reference.js', stage_dir / 'input.json')
            (folder / 'javascript-stages.json').write_text(json.dumps(reference_stages))
            if reference_stages != native_stages:
                raise ValueError(name + ': crop/tensor/coverage/orientation parity failed')
            with (folder / 'native.json').open('wb') as stream:
                run([tools / 'vision-infer', image_path, calibration, '--all-pieces'], stream)
            native = json.loads((folder / 'native.json').read_text())
            if native['quality'] != reference_stages['quality']:
                raise ValueError(name + ': side-view quality diagnostics differ')
            result = {'name': name, 'preprocessing_exact': True, 'latency_ms': native['latency_ms'], 'graphs': {}}
            references = {}
            for graph, height in [('occupancy', 100), ('pieces', 200)]:
                key = (graph, native_stages[graph]['tensor_sha256'])
                if key not in cache:
                    inputs = np.fromfile(stage_dir / (graph + '.f32'), dtype='<f4').reshape(64, 3, height, 100)
                    outputs = [evaluators[graph].run(['logits', model_manifest['features']['tensor']], {'input': batch}) for batch in np.array_split(inputs, 8)]
                    cache[key] = (np.concatenate([b[0] for b in outputs]), np.concatenate([b[1] for b in outputs]))
                logits, features = cache[key]
                actual_logits = np.asarray(native[graph]['logits'], dtype=np.float32)
                actual_features = np.asarray(native[graph]['features'], dtype=np.float32)
                for label, a, b in [('logits', actual_logits, logits), ('features', actual_features, features)]:
                    if a.shape != b.shape or not np.isfinite(a).all() or not np.isfinite(b).all() or not np.allclose(a, b, atol=ATOL, rtol=RTOL):
                        raise ValueError(name + ': ' + graph + ' ' + label + ' parity failed')
                probabilities = softmax(logits, np)
                actual_probabilities = np.asarray(native[graph]['probabilities'])
                if not np.allclose(actual_probabilities, probabilities, atol=PROBABILITY_ATOL, rtol=0) or not np.array_equal(actual_logits.argmax(axis=1), logits.argmax(axis=1)):
                    raise ValueError(name + ': ' + graph + ' probability/class parity failed')
                result['graphs'][graph] = {'max_logit_error': float(np.abs(actual_logits - logits).max()), 'max_feature_error': float(np.abs(actual_features - features).max()), 'max_probability_error': float(np.abs(actual_probabilities - probabilities).max()), 'top_classes_match': True}
                references[graph] = probabilities
            evidence_input = {'squares': reference_stages['squares'], 'occupied': references['occupancy'][:, 1].tolist(), 'pieces': references['pieces'].tolist(), 'coverage': native['coverage']}
            # Coverage is independently derived from both reference crop stages.
            expected_coverage = np.minimum(reference_stages['occupancy']['coverage'], reference_stages['pieces']['coverage']).tolist()
            if native['coverage'] != expected_coverage:
                raise ValueError(name + ': evidence crop coverage differs')
            evidence_input['coverage'] = expected_coverage
            evidence_path = folder / 'reference-evidence-input.json'
            evidence_path.write_text(json.dumps(evidence_input))
            evidence = js('scripts/tests/evidence-reference.js', evidence_path)
            for a, b in zip(native['squares'], evidence):
                if a['square'] != b['square'] or a['visible_probability'] != b['visible_probability'] or abs(a['empty_probability'] - b['empty_probability']) > PROBABILITY_ATOL or not np.allclose(a['piece_probabilities'], b['piece_probabilities'], atol=PROBABILITY_ATOL, rtol=0):
                    raise ValueError(name + ': normalized square evidence parity failed')
            result['square_evidence_matches'] = True
            # Also check the production occupancy>=0.10 selection and mixed batch sizes.
            with (folder / 'native-selected.json').open('wb') as stream:
                run([tools / 'vision-infer', image_path, calibration], stream)
            selected = json.loads((folder / 'native-selected.json').read_text())
            indices = np.flatnonzero(references['occupancy'][:, 1] >= 0.10).tolist()
            if selected['piece_indices'] != indices:
                raise ValueError(name + ': occupancy selection differs')
            for offset, index in enumerate(indices):
                if not np.allclose(selected['pieces']['probabilities'][offset], references['pieces'][index], atol=PROBABILITY_ATOL, rtol=0):
                    raise ValueError(name + ': selected-piece inference differs')
            selected_coverage = reference_stages['occupancy']['coverage'].copy()
            conditional = [None] * 64
            for index in indices:
                selected_coverage[index] = min(selected_coverage[index], reference_stages['pieces']['coverage'][index])
                conditional[index] = references['pieces'][index].tolist()
            evidence_input.update(pieces=conditional, coverage=selected_coverage)
            evidence_path.write_text(json.dumps(evidence_input))
            selected_evidence = js('scripts/tests/evidence-reference.js', evidence_path)
            for a, b in zip(selected['squares'], selected_evidence):
                if a['square'] != b['square'] or a['visible_probability'] != b['visible_probability'] or abs(a['empty_probability'] - b['empty_probability']) > PROBABILITY_ATOL or not np.allclose(a['piece_probabilities'], b['piece_probabilities'], atol=PROBABILITY_ATOL, rtol=0):
                    raise ValueError(name + ': selected-square evidence parity failed')
            if name == 'public-photo':
                expected = '....k....R...............p............P........q.PP......K.R....'
                lookup = {s['square']: s for s in selected['squares']}
                actual = ''
                for rank in range(8, 0, -1):
                    for file in 'abcdefgh':
                        square = lookup[file + str(rank)]
                        values = [square['empty_probability']] + square['piece_probabilities']
                        best = values.index(max(values))
                        actual += '.' if best == 0 else 'PNBRQKpnbrqk'[best - 1]
                result['labelled_photo_matches'] = sum(a == b for a, b in zip(actual, expected))
                if result['labelled_photo_matches'] != 64:
                    raise ValueError('Public-photo regression no longer matches its labelled position')
            result['selected_piece_count'] = len(indices)
            result['selected_latency_ms'] = selected['latency_ms']
            report['cases'].append(result)
            print(name + ': preprocessing, logits/features, probabilities and evidence pass', flush=True)
        report['passed'] = True
    except Exception as error:
        report['error'] = str(error)
        raise
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print('Native parity report: ' + str(output / 'report.json'))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--photo', type=Path, help='Optional existing 1200×800 public regression fixture with its known calibration')
    args = parser.parse_args()
    try:
        check(args.output.resolve(), args.photo.resolve() if args.photo else None)
    except Exception as error:
        parser.exit(1, 'Native parity check failed: ' + str(error) + '\n')


if __name__ == '__main__':
    main()
