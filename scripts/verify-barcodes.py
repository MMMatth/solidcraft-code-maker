#!/usr/bin/env python3
"""Reads back the barcodes the plugin draws, with the ZXing C++ reader (pip install zxing-cpp opencv-python-headless numpy).

  cd code-maker-src && DUMP_BARCODES=barcodes.txt cargo test --lib dump_barcodes -- --ignored
  scripts/verify-barcodes.py    # reads barcodes.txt from the current directory

Each barcode is drawn with a white margin, bars 40 pixels tall, and must say the text it was made from.
"""
import cv2, numpy as np, zxingcpp
lines = open('barcodes.txt').read().split('\n'); i = ok = 0; bad = []
while i < len(lines) and lines[i]:
    name, w, h, text = lines[i].split(' ', 3); w, h = int(w), int(h)
    g = np.array([[c == '#' for c in r] for r in lines[i+1:i+1+h]]); i += 1 + h
    img = np.where(g, 0, 255).astype(np.uint8)
    img = cv2.resize(img, None, fx=8, fy=8 if h > 1 else 1, interpolation=cv2.INTER_NEAREST)
    if h == 1: img = np.repeat(img, 80, axis=0)
    img = np.pad(img, 60, constant_values=255)
    res = zxingcpp.read_barcodes(img)
    got = [r.text for r in res]
    # A reader gives a UPC-A as an EAN-13 with a 0 in front, and a UPC-E as the UPC-A it stands for (here 01234565).
    want = {'upca': ['0' + text], 'upce': ['0012345000065']}.get(name, [text])
    if got == want and (name != 'upce' or res[0].format.name == 'UPCE'): ok += 1
    else: bad.append((name, text, got))
print('ok', ok, 'bad', len(bad)); print(bad)
