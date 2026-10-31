#!/usr/bin/env python3
"""Create a fresh, contrived tree for the PwrDisk accessibility audit."""

import argparse
import os
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--entries", type=int, default=128)
    args = parser.parse_args()
    if not 0 <= args.entries <= 100_000:
        parser.error("--entries must be between 0 and 100000")
    # Refuse an existing destination so reruns cannot overwrite user data.
    args.root.mkdir(parents=True, exist_ok=False)
    files = {
        "keep/notes.txt": 4096,
        "cache/build.bin": 2 * 1024 * 1024,
        "cache/nested/older.bin": 1024 * 1024,
        ".hidden/sample.bin": 512 * 1024,
        "documents/Résumé 日本語.txt": 128 * 1024,
        "documents/" + "long-name-" * 18 + ".txt": 64 * 1024,
        "empty-file.txt": 0,
    }
    for name, count in files.items():
        path = args.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"x" * count)
    (args.root / "empty-directory").mkdir()
    many = args.root / "many-small-files"
    many.mkdir()
    for index in range(args.entries):
        (many / f"item-{index:06d}.txt").write_bytes(b"x" * 1024)
    # Fixed dates make the age legend reproducible across reruns.
    for path in args.root.rglob("*"):
        if path.is_file():
            timestamp = 1_700_000_000 if "older" in path.name else 1_790_000_000
            os.utime(path, (timestamp, timestamp))
    print(args.root.resolve())
    print(f"{len(files) + args.entries} files; use --apparent-size")


if __name__ == "__main__":
    main()
