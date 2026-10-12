#!/usr/bin/env python3
"""Generates the Phase 5 glTF import fixture: tests/fixtures/walker.gltf.

A self-contained glTF 2.0 document (single buffer, base64 data URI):
- 2-joint skin ("root" -> "tip"),
- one 4-vertex skinned quad mesh (JOINTS_0/WEIGHTS_0),
- one "walk" animation with LINEAR translation, STEP rotation and
  LINEAR scale channels,
- inverse-bind matrices (identity) as the spec default wants.

The generated file is committed to the repo; this script exists so the
fixture's provenance is auditable and regenerable byte-for-byte
(float formatting is fixed-width, dict ordering is insertion-ordered).

Run from the repo root:  python3 scripts/gen_gltf_fixture.py
"""
import base64
import json
import struct
from pathlib import Path

FIXTURE = Path(__file__).resolve().parent.parent / (
    "crates/gens-assets/tests/fixtures/walker.gltf"
)


def f32(v: float) -> bytes:
    return struct.pack("<f", v)


def u8(v: int) -> bytes:
    return struct.pack("<B", v)


def u16(v: int) -> bytes:
    return struct.pack("<H", v)


def build_buffer() -> bytes:
    # Mesh: 4-vertex quad in the XY plane, +Z normals.
    positions = b"".join(
        f32(x) + f32(y) + f32(z)
        for x, y, z in [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]
    )
    normals = b"".join(f32(0) + f32(0) + f32(1) for _ in range(4))
    # JOINTS_0: v0,v2 -> joint 0; v1,v3 -> joint 1 (vec4<u8>).
    joints = b"".join(u8(j) for j in (0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1))
    # WEIGHTS_0: rigid binding (w = 1 on the assigned joint).
    weights = b"".join(f32(w) for w in (1, 0, 0, 0) * 4)
    indices = b"".join(u16(i) for i in (0, 1, 2, 0, 2, 3))
    # Animation times: two keys at t = 0.0 and t = 1.0 per channel.
    times = b"".join(f32(t) for t in (0.0, 1.0)) * 3
    # ch1: root translation LINEAR (0,0,0) -> (0,0.5,0).
    trans = f32(0) + f32(0) + f32(0) + f32(0) + f32(0.5) + f32(0)
    # ch2: root rotation STEP identity -> 90 deg around Y (x,y,z,w).
    s = 0.70710677
    rot = f32(0) + f32(0) + f32(0) + f32(1) + f32(0) + f32(s) + f32(0) + f32(s)
    # ch3: tip scale LINEAR (1,1,1) -> (2,2,2).
    scale = f32(1) + f32(1) + f32(1) + f32(2) + f32(2) + f32(2)
    # Inverse bind matrices (identity, column-major) for the 2 joints.
    ibm = b"".join(f32(v) for row in [(1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1)] for v in row) * 2
    return positions + normals + joints + weights + indices + times + trans + rot + scale + ibm


def main() -> None:
    buf = build_buffer()
    # (byteOffset, byteLength) per bufferView, in build_buffer order.
    views = [
        (0, 48),    # 0 positions
        (48, 48),   # 1 normals
        (96, 16),   # 2 joints
        (112, 64),  # 3 weights
        (176, 12),  # 4 indices
        (188, 8),   # 5 times ch1
        (212, 24),  # 6 trans out
        (196, 8),   # 7 times ch2 (times are one contiguous 24 B run)
        (236, 32),  # 8 rot out
        (204, 8),   # 9 times ch3
        (268, 24),  # 10 scale out
        (292, 128), # 11 inverse bind matrices (2 x mat4 f32)
    ]
    assert views[-1][0] + views[-1][1] == len(buf), "view layout must cover the buffer"
    data_uri = "data:application/octet-stream;base64," + base64.b64encode(buf).decode()

    doc = {
        "asset": {"version": "2.0", "generator": "scripts/gen_gltf_fixture.py"},
        "scene": 0,
        "scenes": [{"name": "scene", "nodes": [2]}],
        "nodes": [
            {"name": "root", "children": [1]},
            {"name": "tip"},
            {"name": "body", "mesh": 0, "skin": 0},
        ],
        "skins": [
            {
                "name": "armature",
                "joints": [0, 1],
                "inverseBindMatrices": 11,
            }
        ],
        "meshes": [
            {
                "name": "quad",
                "primitives": [
                    {
                        "attributes": {
                            "POSITION": 0,
                            "NORMAL": 1,
                            "JOINTS_0": 2,
                            "WEIGHTS_0": 3,
                        },
                        "indices": 4,
                        "mode": 4,
                    }
                ],
            }
        ],
        "animations": [
            {
                "name": "walk",
                "samplers": [
                    {"input": 5, "interpolation": "LINEAR", "output": 6},
                    {"input": 7, "interpolation": "STEP", "output": 8},
                    {"input": 9, "interpolation": "LINEAR", "output": 10},
                ],
                "channels": [
                    {"sampler": 0, "target": {"node": 0, "path": "translation"}},
                    {"sampler": 1, "target": {"node": 0, "path": "rotation"}},
                    {"sampler": 2, "target": {"node": 1, "path": "scale"}},
                ],
            }
        ],
        "buffers": [{"byteLength": len(buf), "uri": data_uri}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": off, "byteLength": ln}
            for off, ln in views
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3",
             "min": [0, 0, 0], "max": [1, 1, 0]},                       # 0 positions
            {"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3"},  # 1 normals
            {"bufferView": 2, "componentType": 5121, "count": 4, "type": "VEC4"},  # 2 joints u8
            {"bufferView": 3, "componentType": 5126, "count": 4, "type": "VEC4"},  # 3 weights f32
            {"bufferView": 4, "componentType": 5123, "count": 6, "type": "SCALAR"},  # 4 indices u16
            {"bufferView": 5, "componentType": 5126, "count": 2, "type": "SCALAR",
             "min": [0], "max": [1]},                                    # 5 times ch1
            {"bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC3"},  # 6 trans
            {"bufferView": 7, "componentType": 5126, "count": 2, "type": "SCALAR",
             "min": [0], "max": [1]},                                    # 7 times ch2
            {"bufferView": 8, "componentType": 5126, "count": 2, "type": "VEC4"},  # 8 rot
            {"bufferView": 9, "componentType": 5126, "count": 2, "type": "SCALAR",
             "min": [0], "max": [1]},                                    # 9 times ch3
            {"bufferView": 10, "componentType": 5126, "count": 2, "type": "VEC3"},  # 10 scale
            {"bufferView": 11, "componentType": 5126, "count": 2, "type": "MAT4"},  # 11 IBMs
        ],
    }
    FIXTURE.parent.mkdir(parents=True, exist_ok=True)
    FIXTURE.write_text(json.dumps(doc, indent=1) + "\n", encoding="utf-8")
    print(f"wrote {FIXTURE} ({FIXTURE.stat().st_size} bytes, buffer {len(buf)} B)")


if __name__ == "__main__":
    main()
