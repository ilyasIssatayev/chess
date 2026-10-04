"""FFmpeg integration checks with generated test patterns, not camera accuracy."""
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('prepare_evaluation', ROOT / 'scripts/prepare-evaluation.py')
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


@unittest.skipUnless(shutil.which('ffmpeg') and shutil.which('ffprobe'), 'FFmpeg tools not installed')
class PreparationTests(unittest.TestCase):
    def setUp(self):
        self.folder = Path(tempfile.mkdtemp(prefix='chess-preparation-test-'))
        self.addCleanup(shutil.rmtree, self.folder)
        self.video = self.folder / 'pattern.mkv'
        subprocess.run(['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc=size=320x240:rate=10', '-frames:v', '10', '-c:v', 'ffv1', '-pix_fmt', 'bgr0', str(self.video)], check=True)
        self.manifest = json.loads((ROOT / 'crates/evaluation/examples/manifest.example.json').read_text())
        s = self.manifest['sessions'][0]
        s['media'] = {'uri': 'pattern.mkv', 'sha256': prepare.digest(self.video), 'width_px': 320, 'height_px': 240}
        s['frames'] = [{'frame_index': i, 'capture_us': i * 100000} for i in [0, 3, 7]]
        s['board']['corners'] = {name: {'x_px': x, 'y_px': y} for name, x, y in [('near_left', 20, 220), ('near_right', 300, 220), ('far_right', 300, 20), ('far_left', 20, 20)]}
        s['reference_game']['moves'] = []
        self.path = self.folder / 'manifest.json'
        self.write()

    def write(self):
        self.path.write_text(json.dumps(self.manifest))

    def test_selected_frames_match_an_independent_full_decode_and_keep_capture_time(self):
        full = self.folder / 'full'
        full.mkdir()
        subprocess.run(['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-noautorotate', '-i', str(self.video), '-fps_mode', 'passthrough', '-pix_fmt', 'rgb24', '-start_number', '0', str(full / '%06d.png')], check=True)
        output = self.folder / 'selected'
        prepare.prepare(self.path, None, output)
        bundle = json.loads((output / 'bundle.json').read_text())
        sealed = json.loads((output / 'manifest.json').read_text())
        media = sealed['sessions'][0]['media']
        self.assertEqual((output / media['uri']).resolve(), self.video.resolve())
        self.assertEqual(media['sha256'], prepare.digest(self.video))
        self.assertEqual(bundle['session']['media'], media)
        for f in bundle['frames']:
            self.assertEqual(prepare.digest(output / f['file']), prepare.digest(full / f"{f['frame_index']:06d}.png"))
            self.assertEqual(f['capture_us'], f['frame_index'] * 100000)
            self.assertEqual(f['source_presentation_us'], f['capture_us'])
        self.assertEqual(len({f['sha256'] for f in bundle['frames']}), 3)

    def test_changed_media_and_invented_media_times_are_not_published(self):
        output = self.folder / 'invalid'
        self.manifest['sessions'][0]['media']['sha256'] = 'a' * 64
        self.write()
        with self.assertRaisesRegex(ValueError, 'SHA-256'):
            prepare.prepare(self.path, None, output)
        self.assertFalse(output.exists())
        self.manifest['sessions'][0]['media']['sha256'] = prepare.digest(self.video)
        self.manifest['sessions'][0]['frames'][1]['capture_us'] += 1000
        self.write()
        with self.assertRaisesRegex(ValueError, 'timeline'):
            prepare.prepare(self.path, None, output)
        self.assertFalse(output.exists())

    def test_existing_output_is_preserved(self):
        output = self.folder / 'existing'
        output.mkdir()
        sentinel = output / 'retained.txt'
        sentinel.write_text('keep')
        with self.assertRaisesRegex(ValueError, 'already exists'):
            prepare.prepare(self.path, None, output)
        self.assertEqual(sentinel.read_text(), 'keep')


if __name__ == '__main__':
    unittest.main()
