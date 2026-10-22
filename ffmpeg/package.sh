#!/usr/bin/env bash
#
# Puts the release together.
#
# The program is put in a folder with the conversion program beside it and
# the licences of everything that is in there, and the whole thing is
# written to a zip. The notices are made from the manifest, so that they
# say what is actually in the package.
#
# The conversion program is never put in the repository. It is built by
# the build script, and only the build script puts it in the package.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
MANIFEST="$HERE/components.txt"

VERSION="${FFMPEG_VERSION:-9.0.2}"
PREFIX="${FFMPEG_PREFIX:-/c/Users/thorfinn/AppData/Local/Temp/opencode/ffmpeg-build/prefix}"
SOURCE="${FFMPEG_SOURCE:-/c/Users/thorfinn/AppData/Local/Temp/opencode/ffmpeg-build/ffmpeg-$VERSION}"
DIST="${DIST:-$ROOT/dist}"
NAME="OpenTangerine"

# The program of the application, built by cargo.
APP="${APP:-$ROOT/target/release/open-tangerine.exe}"

echo "=== what goes in ==="
[ -f "$PREFIX/bin/ffmpeg.exe" ] || { echo "the conversion program was not built"; exit 1; }
[ -f "$PREFIX/bin/ffprobe.exe" ] || { echo "the program that inspects was not built"; exit 1; }

# Git is not always on the path of the shell that runs this script, and
# neither is cargo. Both are looked for in the places they are usually
# found, so that the script works from a plain shell.
GIT="git"
if ! command -v "$GIT" >/dev/null 2>&1; then
    for candidate in "/c/Program Files/Git/cmd/git.exe" "/c/Program Files/Git/bin/git.exe"; do
        if [ -x "$candidate" ]; then
            GIT="$candidate"
            break
        fi
    done
fi
CARGO="cargo"
if ! command -v "$CARGO" >/dev/null 2>&1; then
    for candidate in "$HOME/.cargo/bin/cargo.exe" "/c/Users/$USERNAME/.cargo/bin/cargo.exe"; do
        if [ -x "$candidate" ]; then
            CARGO="$candidate"
            break
        fi
    done
fi

# The program of this tree is built here, and not taken from whatever
# happens to be in the build folder. A package that is made from an older
# build is the reason a copy of the program can be found that does not do
# what the source says.
#
# The revision of the tree is given to the build, so that the program says
# which revision it is of. The build is made again when the answer changes,
# so the program and the tree cannot drift apart.
TREE_REVISION="unknown"
TREE_DIRTY=""
if command -v "$GIT" >/dev/null 2>&1; then
    TREE_REVISION="$("$GIT" -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
    if [ -n "$("$GIT" -C "$ROOT" status --porcelain 2>/dev/null)" ]; then
        TREE_DIRTY="-dirty"
    fi
fi
if [ "${SKIP_BUILD:-}" = "" ] && command -v "$CARGO" >/dev/null 2>&1; then
    echo "building the program of revision '$TREE_REVISION$TREE_DIRTY'"
    (cd "$ROOT" && OPEN_TANGERINE_REVISION="$TREE_REVISION$TREE_DIRTY" "$CARGO" build --release)
else
    echo "note: the program was not built here"
fi

[ -f "$APP" ] || { echo "the application was not built: $APP is not there"; exit 1; }
ls -l "$APP" "$PREFIX/bin/ffmpeg.exe" "$PREFIX/bin/ffprobe.exe"

# The program says which revision it was built from, and the tree says which
# revision it is at. A program that is not from this tree is not put in the
# package.
#
# The tree may have changes that are not committed, and the program says so
# with a "-dirty" ending. The two are compared on the revision itself.
APP_REVISION="$("$APP" --version 2>&1 | sed -n 's/.*revision //p')"
APP_COMMIT="${APP_REVISION%%-dirty}"
if [ "$TREE_REVISION" != "unknown" ]; then
    TREE_COMMIT="${TREE_REVISION%%-dirty}"
    if [ "$APP_COMMIT" != "$TREE_COMMIT" ]; then
        echo "the program is from revision '$APP_REVISION', but the tree is at '$TREE_REVISION'"
        echo "build it again: cargo build --release"
        exit 1
    fi
    echo "the program is from revision '$APP_REVISION', and the tree is at '$TREE_REVISION'"
else
    echo "the program is from revision '$APP_REVISION'"
    echo "note: git was not found, so the tree could not be compared with the program"
fi

STAGE="$DIST/$NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE/ffmpeg" "$STAGE/licenses/ffmpeg"

cp "$APP" "$STAGE/"
cp "$PREFIX/bin/ffmpeg.exe" "$STAGE/ffmpeg/"
cp "$PREFIX/bin/ffprobe.exe" "$STAGE/ffmpeg/"

# The licence of the conversion program, and the licence of everything
# that is compiled into it.
cp "$SOURCE/COPYING.LGPLv2.1" "$STAGE/licenses/ffmpeg/LGPL-2.1.txt"
for library in $(sed -n 's/^library[[:space:]]*=[[:space:]]*\([^[:space:]]*\).*/\1/p' "$MANIFEST"); do
    for file in /c/msys64/ucrt64/share/licenses/"$library"/*; do
        [ -f "$file" ] || continue
        cp "$file" "$STAGE/licenses/ffmpeg/$library-$(basename "$file")"
    done
done

# A notice of what is in the package and under what terms, made from the
# manifest, so that it says what is actually here.
{
    echo "What is in this package"
    echo
    echo "open-tangerine.exe"
    echo "  The application. Licensed under the MIT licence or the Apache"
    echo "  Licence, Version 2.0, at your choice. The texts of both are in"
    echo "  the repository of the project, in the files LICENSE-MIT and"
    echo "  LICENSE-APACHE."
    echo
    echo "ffmpeg/ffmpeg.exe, ffmpeg/ffprobe.exe"
    echo "  The FFmpeg program, version $VERSION, built from the sources of"
    echo "  the FFmpeg project. Licensed under the GNU Lesser General Public"
    echo "  Licence, version 2.1 or later; the licence text is in this folder."
    echo "  The sources it was built from, and how it was built, are named in"
    echo "  SOURCE-OFFER.txt and in licenses/ffmpeg/HOW-IT-WAS-BUILT.txt."
    echo
    echo "The libraries compiled into the conversion program:"
    echo
    sed -n 's/^library[[:space:]]*=[[:space:]]*\([^[:space:]]*\).*/\1/p' "$MANIFEST" |
    while read -r library; do
        licence="$(sed -n "s/^library[[:space:]]*=[[:space:]]*$library[[:space:]].*licence=\([^[:space:]]*\).*/\1/p" "$MANIFEST" | head -n 1)"
        version="$(sed -n "s/^library[[:space:]]*=[[:space:]]*$library[[:space:]].*version=\([^[:space:]]*\).*/\1/p" "$MANIFEST" | head -n 1)"
        echo "  $library $version - $licence"
    done
    echo
    echo "The licence of each of them that is not part of Windows is in this"
    echo "folder."
} > "$STAGE/licenses/ffmpeg/NOTICE.txt"

# How it was built, and what was changed. The licence asks for both.
{
    echo "The conversion program was built from the FFmpeg $VERSION sources with"
    echo "these options:"
    echo
    "$PREFIX/bin/ffmpeg.exe" -hide_banner -version | sed -n 's/^configuration: //p'
    echo
    echo "The list of parts the build was made of is in components.txt, next to"
    echo "this file in the repository."
    if [ -f "$SOURCE/ffbuild/config.log" ]; then
        echo
        echo "The whole of the configure output is in the build log, which is kept"
        echo "with the build."
    fi
} > "$STAGE/licenses/ffmpeg/HOW-IT-WAS-BUILT.txt"

# The offer of the source of the conversion program, which the licence of
# it asks for.
cat > "$STAGE/SOURCE-OFFER.txt" <<OFFER
The source of the conversion program

This package contains a program built from the FFmpeg project. The FFmpeg
project is licensed under the GNU Lesser General Public License, version
2.1 or later, and that licence covers the program in the ffmpeg folder of
this package as well.

The complete corresponding source code of that program is the FFmpeg
$VERSION source release, together with the options it was built with,
which are in licenses/ffmpeg/HOW-IT-WAS-BUILT.txt. It can be obtained
from:

    https://ffmpeg.org/releases/ffmpeg-$VERSION.tar.xz

The source of the libraries that were compiled into that program, with
their licences, is in licenses/ffmpeg. They are the libraries named in
the components.txt file of this project.

The application itself, open-tangerine.exe, is not part of the FFmpeg
project and is not covered by the LGPL. It is licensed under the MIT
licence or the Apache Licence, Version 2.0, at your choice.
OFFER

# The zip.
cd "$DIST"
rm -f "$NAME.zip"
if command -v 7z >/dev/null 2>&1; then
    7z a -tzip -mx=9 "$NAME.zip" "$NAME" >/dev/null
elif command -v zip >/dev/null 2>&1; then
    zip -qr9 "$NAME.zip" "$NAME"
else
    echo "there is no zip program; the folder $STAGE is ready"
    exit 1
fi

echo "=== what is in the package ==="
find "$STAGE" -type f | sed "s|$DIST/||" | sort
echo
ls -l "$DIST/$NAME.zip" 2>/dev/null || true
du -sh "$STAGE" 2>/dev/null || true
echo "=== DONE ==="
