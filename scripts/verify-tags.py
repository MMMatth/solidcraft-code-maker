#!/usr/bin/env python3
"""Decodes the markers the plugin draws, with OpenCV's ArUco detector and the AprilTag library.

  cd code-maker-src && DUMP_GRIDS=/tmp/grids.txt cargo test --lib dump_grids -- --ignored
  pip install opencv-contrib-python-headless pupil-apriltags numpy
  scripts/verify-tags.py        # reads grids.txt from the current directory

Every dictionary is tried with four ids; each must decode back to its own id.
"""
import cv2, numpy as np, sys
from pupil_apriltags import Detector
lines = open('grids.txt').read().split('\n')
i = 0; ok = bad = 0; fails = []
det = {}
while i < len(lines) and lines[i]:
    kind, name, id_, n, margin = lines[i].split(); id_ = int(id_); n = int(n)
    rows = lines[i+1:i+1+n]; i += 1 + n
    g = np.array([[c == '#' for c in r] for r in rows])
    img = np.where(g, 0, 255).astype(np.uint8)
    img = np.pad(img, 3, constant_values=255)
    img = cv2.resize(img, None, fx=24, fy=24, interpolation=cv2.INTER_NEAREST)
    if kind == 'aruco':
        const = {'original': 'DICT_ARUCO_ORIGINAL', 'mip_36h12': 'DICT_ARUCO_MIP_36h12'}.get(name, 'DICT_' + name.upper())
        d = cv2.aruco.getPredefinedDictionary(getattr(cv2.aruco, const))
        ids = cv2.aruco.ArucoDetector(d, cv2.aruco.DetectorParameters()).detectMarkers(img)[1]
        got = [] if ids is None else [int(v) for v in ids.flatten()]
    else:
        fam = {'standard_41h12': 'tagStandard41h12'}.get(name, 'tag' + name)
        det.setdefault(fam, Detector(families=fam))
        got = [r.tag_id for r in det[fam].detect(img)]
    if got == [id_]: ok += 1
    else: bad += 1; fails.append((kind, name, id_, got))
print('ok', ok, 'bad', bad); print(fails[:20])
