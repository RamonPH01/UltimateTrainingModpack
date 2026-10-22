#!/bin/sh
# Downloads the real-match recordings the tests compare against.
# They come from the ReFramed project (https://github.com/reframedultimate) and are not
# stored in this repository. Pinned to a commit and checked by hash so tests stay reproducible.
set -eu
cd "$(dirname "$0")"

COMMIT=dcbafcaf8721765d3dfaf880b773cf30f782c8c2
URL="https://raw.githubusercontent.com/reframedultimate/ReFramed-server-emulator/$COMMIT/data/optimal-bridges-3rep-3xuair-finish.raw"
FILE=reframed_pikachu_joker.raw
SHA256=538e3c4eec894f9d86b83b43faf64b041519637300e20a83cd78f1f7c8d2effd

if [ ! -f "$FILE" ]; then
    curl -fsSL --retry 3 -o "$FILE.part" "$URL"
    mv "$FILE.part" "$FILE"
fi

if command -v sha256sum >/dev/null; then
    actual=$(sha256sum "$FILE" | cut -d' ' -f1)
else
    actual=$(shasum -a 256 "$FILE" | cut -d' ' -f1)
fi
if [ "$actual" != "$SHA256" ]; then
    echo "$FILE: checksum mismatch ($actual)" >&2
    rm -f "$FILE"
    exit 1
fi
echo "$FILE ok"
