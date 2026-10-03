# Local photo recognition model and preprocessing notices

Models: [cstr/chess-board-photo-onnx](https://huggingface.co/cstr/chess-board-photo-onnx),
revision `4690fd5418ff7c405a76d1c156077ecd865ac053`, declared MIT by the author.
These are MobileNetV3-Small occupancy and 12-class piece classifiers for physical
board photographs, not screenshot classifiers. Weights stay in ignored local
storage and are installed from immutable URLs with SHA-256 checks.
The installer verifies the original files, then adds a graph output exposing
the existing 1024-dimensional penultimate activation. No network weights or
original logits are changed. Derived files have separate pinned hashes. This
protobuf-only transform is in `scripts/vision-features.py`; ONNX's checker was
used during development, but no Python neural framework is needed at startup.

Crop/normalization implementation adapted to JavaScript from
[CrispChess](https://github.com/CrispStrobe/CrispChess/tree/f029a319e35ce018936fc620dbf35a656a409b84/lib/vision/photo)
and its chesscog-compatible reference. Copyright (c) 2024–2026 CrispStrobe.
The project [chesscog](https://github.com/georg-wolflein/chesscog) by Georg Wölflein
and Ognjen Arandjelović is MIT. Cite their paper: *Determining Chess Game State
From an Image*, Journal of Imaging 7(6), 2021, DOI 10.3390/jimaging7060094.

The model card attributes training inputs to chesscog synthetic renders (OSF
xf3ka, CC BY 4.0), samryan18/chess-dataset (MIT), and Roboflow 100
chess-pieces-mjzgj (CC BY 4.0), and ImageNet-initialized torchvision weights
(BSD-3-Clause). The downloaded original model card is retained under
`local-data/model-research/cstr-README.md` in this development checkout; the
linked card is the canonical provenance record.

Runtime: Microsoft ONNX Runtime Web 1.23.2, MIT. Its official npm archive's
SHA-512 integrity and individual extracted asset SHA-256 hashes are pinned in
`vision-manifest.json`. No arbitrary archive paths are extracted.

Browser smoke-photo fixture: samryan18/chess-dataset, © 2019 Samuel Ryan,
Mukund Venkateswaran, Kurt Convey, Michael Deng (MIT), via the pinned CrispChess
fixture. The source licence is retained beside the downloaded test photo.
This photo is a development regression, not an independent accuracy evaluation.

## CrispChess MIT licence

MIT License

Copyright (c) 2024-2026 CrispStrobe

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.


## ONNX Runtime licence

MIT License

Copyright (c) Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
