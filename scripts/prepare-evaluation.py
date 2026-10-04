#!/usr/bin/env python3
"""Prepare hash-checked lossless frames for the offline browser evaluator.

Requires installed FFmpeg/ffprobe and the project's Rust manifest validator.
Only local media is accepted. Acquisition timestamps come from the annotations,
not extraction time. Output is staged and published only after verification.
"""
import argparse
from decimal import Decimal
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
MAX_FRAMES = 2000
MAX_BYTES = 512 * 1024 * 1024


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def validate_manifest(path):
    env = dict(os.environ)
    cargo = shutil.which('cargo')
    local = ROOT / '.toolchain/cargo/bin/cargo'
    if local.is_file():
        cargo = str(local)
        env.update(CARGO_HOME=str(ROOT / '.toolchain/cargo'), RUSTUP_HOME=str(ROOT / '.toolchain/rustup'), RUSTUP_TOOLCHAIN='stable-aarch64-apple-darwin')
    if not cargo:
        raise ValueError('Rust toolchain missing. Build manifest-check before preparing footage.')
    subprocess.run([cargo, 'run', '--offline', '-p', 'chess-evaluation', '--bin', 'manifest-check', '--', str(path)], cwd=ROOT, env=env, check=True)


def select_filter(indices):
    # Commas are escaped for FFmpeg's filter parser, never a shell.
    ranges = []
    start = previous = indices[0]
    for i in indices[1:] + [None]:
        if i is not None and i == previous + 1:
            previous = i
            continue
        ranges.append(f'eq(n\\,{start})' if start == previous else f'between(n\\,{start}\\,{previous})')
        start = previous = i
    return 'select=' + '+'.join(ranges)


def png_size(path):
    with path.open('rb') as stream:
        header = stream.read(24)
    if len(header) != 24 or header[:8] != b'\x89PNG\r\n\x1a\n' or header[12:16] != b'IHDR':
        raise ValueError('Invalid PNG output: ' + path.name)
    return struct.unpack('>II', header[16:24])


def prepare(manifest_path, session_id, output):
    manifest_path = manifest_path.resolve()
    output = output.resolve()
    if output.exists():
        raise ValueError('Output already exists; choose a new directory.')
    validate_manifest(manifest_path)
    manifest = json.loads(manifest_path.read_text())
    matches = [s for s in manifest['sessions'] if s['session_id'] == session_id] if session_id else manifest['sessions']
    if len(matches) != 1:
        raise ValueError('Choose exactly one manifest session using --session ID.')
    session = matches[0]
    if session['split'] == 'qualification':
        raise ValueError('This development workflow does not support qualification data. Use a frozen candidate/evaluator for qualification.')
    frames = session['frames']
    if len(frames) > MAX_FRAMES:
        raise ValueError(f'Use a development clip with at most {MAX_FRAMES} annotated frames; preserve the original partition_key when splitting clips.')
    uri = session['media']['uri']
    if '://' in uri or uri.startswith('file:'):
        raise ValueError('Only local media paths are supported.')
    media = (manifest_path.parent / uri).resolve()
    if not media.is_file() or media.stat().st_size > 2 * 1024 ** 3:
        raise ValueError('Media must be an existing local file no larger than 2 GiB.')
    media_hash = digest(media)
    expected = session['media'].get('sha256')
    if expected and expected.lower() != media_hash:
        raise ValueError('Source media SHA-256 does not match the manifest.')
    if not shutil.which('ffmpeg') or not shutil.which('ffprobe'):
        raise ValueError('Install FFmpeg (including ffprobe) before preparing media.')
    probe = subprocess.run(['ffprobe', '-v', 'error', '-protocol_whitelist', 'file,pipe', '-select_streams', 'v:0', '-show_frames', '-show_entries', 'frame=best_effort_timestamp_time,width,height', '-of', 'json', str(media)], check=True, capture_output=True, timeout=60)
    source = json.loads(probe.stdout)['frames']
    if frames[-1]['frame_index'] >= len(source):
        raise ValueError('The manifest refers to a frame that the media does not contain.')
    selected = [source[f['frame_index']] for f in frames]
    dimensions = (session['media']['width_px'], session['media']['height_px'])
    if any((f['width'], f['height']) != dimensions for f in selected):
        raise ValueError('Decoded media dimensions differ from the calibrated manifest.')
    pts = [int(Decimal(f['best_effort_timestamp_time']) * 1000000) if 'best_effort_timestamp_time' in f else None for f in selected]
    if session['timestamp_source'] == 'media_presentation':
        if any(t is None for t in pts):
            raise ValueError('Media presentation timestamps are unavailable.')
        if any(abs((t - pts[0]) - (f['capture_us'] - frames[0]['capture_us'])) > 2 for t, f in zip(pts, frames)):
            raise ValueError('Annotated media-presentation timestamps disagree with the decoded media timeline.')
    output.parent.mkdir(parents=True, exist_ok=True)
    staged = Path(tempfile.mkdtemp(prefix='.chess-eval-', dir=output.parent))
    try:
        folder = staged / 'frames'
        folder.mkdir()
        expression = select_filter([f['frame_index'] for f in frames])
        command = ['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-protocol_whitelist', 'file,pipe', '-noautorotate', '-i', str(media), '-map', '0:v:0', '-vf', expression, '-fps_mode', 'passthrough', '-frames:v', str(len(frames)), '-pix_fmt', 'rgb24', '-start_number', '0', str(folder / '%06d.png')]
        with (staged / 'extraction.log').open('wb') as log:
            process = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=log)
            try:
                deadline = time.monotonic() + 300
                while process.poll() is None:
                    if time.monotonic() > deadline or sum(p.stat().st_size for p in folder.glob('*.png')) > MAX_BYTES:
                        raise ValueError('Extraction exceeded the five-minute/512 MiB development limit.')
                    time.sleep(.2)
                if process.returncode:
                    raise ValueError('FFmpeg extraction failed: ' + (staged / 'extraction.log').read_text()[-2000:])
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
        images = sorted(folder.glob('*.png'))
        if len(images) != len(frames) or sum(p.stat().st_size for p in images) > MAX_BYTES:
            raise ValueError('Extraction frame count or size limit failed.')
        records = []
        for i, (frame, image, timestamp) in enumerate(zip(frames, images, pts)):
            if png_size(image) != dimensions:
                raise ValueError('PNG dimensions changed; camera calibration would be invalid.')
            records.append({**frame, 'file': f'frames/{i:06d}.png', 'sha256': digest(image), 'source_presentation_us': timestamp})
        version = subprocess.run(['ffmpeg', '-version'], check=True, capture_output=True, text=True).stdout.splitlines()[0]
        # Seal the selected media identity in the exported annotations, too.
        # Keep its path meaningful relative to the new bundle directory.
        session['media']['sha256'] = media_hash
        for entry in manifest['sessions']:
            original_uri = entry['media']['uri']
            if '://' not in original_uri and not original_uri.startswith('file:'):
                entry['media']['uri'] = os.path.relpath((manifest_path.parent / original_uri).resolve(), output)
        bundle = {'schema_version': 1, 'kind': 'chess-frame-bundle', 'dataset_id': manifest['dataset_id'], 'session': session, 'source_sha256': media_hash, 'ffmpeg_version': version, 'frames': records}
        (staged / 'bundle.json').write_text(json.dumps(bundle, indent=2) + '\n')
        (staged / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        (staged / 'extraction.log').unlink()
        if output.exists():
            raise ValueError('Output appeared during extraction; refusing to replace it.')
        staged.rename(output)
        print(f'Prepared {len(records)} timestamped frames: {output}\nOpen http://localhost:8770/evaluation.html and select this folder. Frames remain local.')
    finally:
        if staged.exists():
            shutil.rmtree(staged)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--session')
    args = parser.parse_args()
    try:
        prepare(args.manifest, args.session, args.output)
    except (ValueError, OSError, KeyError, ArithmeticError, subprocess.SubprocessError) as error:
        parser.exit(1, f'Cannot prepare evaluation: {error}\n')


if __name__ == '__main__':
    main()
