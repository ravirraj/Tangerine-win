#!/usr/bin/env bash
#
# Builds the FFmpeg that the program uses.
#
# The list of what goes into the build is not written here: it is read from
# components.txt, the same file the program reads. The configure flags,
# the assertions that follow the build, and the notices that are packaged
# all come from that one file, so what is built and what the program
# expects cannot drift apart.
#
# The build is meant to run in the pinned image, where the toolchain and
# the libraries are the versions the manifest names. Set FFMPEG_VERSION to
# build another release.

set -euo pipefail

VERSION="${FFMPEG_VERSION:-9.0.2}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
MANIFEST="$HERE/components.txt"
WORK="${FFMPEG_WORK:-/c/Users/thorfinn/AppData/Local/Temp/opencode/ffmpeg-build}"
PREFIX="${FFMPEG_PREFIX:-$WORK/prefix}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1700000000}"

# The toolchain of the pinned image.
export PATH="/c/msys64/ucrt64/bin:/c/msys64/usr/bin:$PATH"
export MSYSTEM=UCRT64

echo "=== the manifest ==="
manifest_value() {
    # The value of a key in the manifest.
    sed -n "s/^$1[[:space:]]*=[[:space:]]*//p" "$MANIFEST" | head -n 1
}

manifest_words() {
    # Every value of a key in the manifest, one word per line.
    sed -n "s/^$1[[:space:]]*=[[:space:]]*//p" "$MANIFEST" | tr ' ' '\n' | grep -v '^-$' | grep -v '^$' || true
}

manifest_field() {
    # Every value of a field of a key, one word per line.
    sed -n "s/^$1[[:space:]]*=[[:space:]]*//p" "$MANIFEST" \
        | tr ' ' '\n' | grep "^$2=" | cut -d= -f2 | grep -v '^-$' || true
}

PROTOCOLS="$(manifest_value protocols)"
FILTERS="$(manifest_value filters)"

DEMUXERS="$( { manifest_field read demuxer; manifest_field animation demuxer; manifest_field confirm demuxer; manifest_words test-demuxers; } | sort -u | tr '\n' ' ')"
DECODERS="$( { manifest_field read decoder; manifest_field animation decoder; manifest_field confirm decoder; manifest_words test-decoders; } | sort -u | tr '\n' ' ')"
ENCODERS="$( { manifest_field write encoder; manifest_words test-encoders; } | sort -u | tr '\n' ' ')"
MUXERS="$( { manifest_field write muxer; manifest_words test-muxers; manifest_words reserved-muxers; } | sort -u | tr '\n' ' ')"

# The lists of names are given to the configure script separated by commas.
comma() { tr ' ' ',' <<<"$1" | sed 's/,$//'; }

# The name the build system knows a demuxer by.
#
# Most parts are called the same in the build and on the command line, but
# the demuxers that read a picture from a stream are not: the build calls
# the PNG one "image_png_pipe" and the program calls it "png_pipe". The
# assertions below look for the names on the command line, because those
# are the names the program uses.
configure_name() {
    case "$1" in
        *_pipe) echo "image_$1" ;;
        *) echo "$1" ;;
    esac
}

configure_names() {
    # The names the build system knows, separated by spaces.
    local out="" name
    for name in $1; do out="$out $(configure_name "$name")"; done
    echo "${out# }"
}

# The external libraries. A library whose name begins with "lib" is one
# FFmpeg asks for by name; the rest are system libraries it finds.
EXTERNALS=""
while read -r name; do
    case "$name" in
        lib*) EXTERNALS="$EXTERNALS --enable-$name" ;;
    esac
done < <(sed -n 's/^library[[:space:]]*=[[:space:]]*\([^[:space:]]*\).*/\1/p' "$MANIFEST")

echo "protocols:  $PROTOCOLS"
echo "demuxers:   $DEMUXERS"
echo "decoders:   $DECODERS"
echo "encoders:   $ENCODERS"
echo "muxers:     $MUXERS"
echo "filters:    $FILTERS"
echo "externals:  $EXTERNALS"
echo "in the build system, the demuxers are called:"
echo "            $(configure_names "$DEMUXERS")"

CONFIGURE_FLAGS=(
    --disable-everything
    --enable-protocol="$(comma "$PROTOCOLS")"
    --enable-demuxer="$(comma "$(configure_names "$DEMUXERS")")"
    --enable-decoder="$(comma "$DECODERS")"
    --enable-encoder="$(comma "$ENCODERS")"
    --enable-muxer="$(comma "$MUXERS")"
    --enable-filter="$(comma "$FILTERS")"
    $EXTERNALS
    # The program is put in the package on its own, so everything it needs
    # has to be in it: the libraries it was built against, and the run-time
    # of the compiler. The only things left are the ones Windows itself
    # has.
    --extra-ldflags="-static"
    # The network protocols are not wanted. The pipe is not one of them, and
    # it is named above as well.
    --disable-network
    # The system libraries that are detected and are not wanted. zlib is
    # wanted, and is pinned in the manifest.
    --disable-bzlib
    --disable-iconv
    --disable-lzma
    --disable-avdevice
    --disable-ffplay
    --disable-doc
    --disable-debug
)

echo "=== the source ==="
mkdir -p "$WORK"
cd "$WORK"
TARBALL="ffmpeg-$VERSION.tar.xz"
if [ ! -f "$TARBALL" ]; then
    curl -fsSL -o "$TARBALL" "https://ffmpeg.org/releases/$TARBALL"
fi
if [ ! -f "$TARBALL.asc" ]; then
    curl -fsSL -o "$TARBALL.asc" "https://ffmpeg.org/releases/$TARBALL.asc"
fi
if [ ! -d "ffmpeg-$VERSION" ]; then
    tar -xf "$TARBALL"
fi
cd "ffmpeg-$VERSION"

# The build is made the same way every time.
export SOURCE_DATE_EPOCH

echo "=== configure ==="
# The summary of what was enabled and what was linked in is only printed
# here, so it is kept.
./configure \
    --prefix="$PREFIX" \
    --pkg-config-flags=--static \
    "${CONFIGURE_FLAGS[@]}" 2>&1 | tee "$WORK/configure.txt"

echo "=== make ==="
make -j"$(nproc 2>/dev/null || echo 4)"

echo "=== install ==="
make install

echo "=== the assertions ==="
FFMPEG="$PREFIX/bin/ffmpeg.exe"
FFPROBE="$PREFIX/bin/ffprobe.exe"
[ -x "$FFMPEG" ] || { echo "ffmpeg.exe was not built"; exit 1; }
[ -x "$FFPROBE" ] || { echo "ffprobe.exe was not built"; exit 1; }

assert_listing() {
    # The names a listing must hold.
    local what="$1" listing="$2"
    shift 2
    local found
    found="$("$FFMPEG" -hide_banner "$listing" 2>/dev/null || true)"
    for name in "$@"; do
        if ! grep -qE "(^|[ .])${name}([ .]|$)" <<<"$found"; then
            echo "MISSING: $name is not in $listing"
            MISSING=1
        fi
    done
}

assert_formats() {
    # The demuxers and the muxers the manifest asks for, told apart by the
    # flags the format listing carries.
    local found
    found="$("$FFMPEG" -hide_banner -formats 2>/dev/null || true)"
    local demuxers muxers
    demuxers="$( { manifest_field read demuxer; manifest_field animation demuxer; manifest_field confirm demuxer; manifest_words test-demuxers; } | sort -u )"
    muxers="$( { manifest_field write muxer; manifest_words test-muxers; manifest_words reserved-muxers; } | sort -u )"
    for name in $demuxers; do
        if ! grep -qE "^ *[.A-Z]*D[^ ]*[ ]+$name([ ]|$)" <<<"$found"; then
            echo "MISSING: $name is not a demuxer"
            MISSING=1
        fi
    done
    for name in $muxers; do
        if ! grep -qE "^ *[.A-Z]*E[^ ]*[ ]+$name([ ]|$)" <<<"$found"; then
            echo "MISSING: $name is not a muxer"
            MISSING=1
        fi
    done
}

MISSING=0
assert_listing encoders "-encoders" $ENCODERS
assert_listing decoders "-decoders" $DECODERS
assert_listing filters  "-filters"  $FILTERS
assert_formats

for protocol in $PROTOCOLS; do
    if ! "$FFMPEG" -hide_banner -protocols 2>/dev/null | grep -qE "(^|[ .])${protocol}([ .]|$)"; then
        echo "MISSING: $protocol is not a protocol"
        MISSING=1
    fi
done

# The network protocols are not wanted, and the pipe is wanted.
for unwanted in http https tcp udp rtp ftp tls; do
    if "$FFMPEG" -hide_banner -protocols 2>/dev/null | grep -qE "(^|[ .])${unwanted}([ .]|$)"; then
        echo "UNWANTED: $unwanted is a protocol"
        MISSING=1
    fi
done

echo "=== the libraries that were linked in ==="
# The build must have found exactly the libraries the manifest names, and
# the versions of the ones that are pinned must be the ones that were
# asked for. A build that quietly gained a library cannot be released.
ACTUAL_LIBRARIES="$(sed -n '/^External libraries:/,/^$/p' "$WORK/configure.txt" \
    | tail -n +2 | tr -s ' ' '\n' | grep -v '^$' | sort -u)"
EXPECTED_LIBRARIES="$(sed -n 's/^library[[:space:]]*=[[:space:]]*\([^[:space:]]*\).*/\1/p' "$MANIFEST" | sort -u)"
if [ "$ACTUAL_LIBRARIES" != "$EXPECTED_LIBRARIES" ]; then
    echo "MISSING: the libraries in the build are not the ones in the manifest:"
    diff <(echo "$EXPECTED_LIBRARIES") <(echo "$ACTUAL_LIBRARIES") || true
    MISSING=1
else
    echo "the libraries are the ones the manifest names"
fi

for name in $EXPECTED_LIBRARIES; do
    version="$(sed -n "s/^library[[:space:]]*=[[:space:]]*$name[[:space:]].*version=\([^[:space:]]*\).*/\1/p" "$MANIFEST" | head -n 1)"
    case "$version" in
        ""|system) continue ;;
    esac
    found="$(pkgconf --modversion "$name" 2>/dev/null || echo "not found")"
    if [ "$found" != "$version" ]; then
        echo "MISSING: $name is $found, not $version"
        MISSING=1
    else
        echo "$name is $found"
    fi
done

if [ "$MISSING" != "0" ]; then
    echo "=== the build does not have what the manifest asks for ==="
    exit 1
fi

echo "=== the libraries that were linked in ==="
# The external libraries the build actually found.
grep -E "^(zlib|libwebp) " "$WORK/ffmpeg-$VERSION/ffbuild/config.log" 2>/dev/null || true
"$FFMPEG" -hide_banner -version 2>/dev/null | grep -E "configuration:" || true

echo "=== DONE ==="
"$FFMPEG" -hide_banner -version | head -n 3
ls -l "$PREFIX/bin"