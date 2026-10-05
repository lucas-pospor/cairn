#!/usr/bin/env python3
"""Drop the page cache for every file under a folder (no root needed), to
measure cold-start times:  python3 scripts/evict-cache.py <dir>"""
import os, sys

n = 0
for root, _, files in os.walk(sys.argv[1]):
    for f in files:
        try:
            fd = os.open(os.path.join(root, f), os.O_RDONLY)
            os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
            os.close(fd)
            n += 1
        except OSError:
            pass
print(f"evicted {n} files")
