# The engine's own source, as it was when a binary was built.
#
# Sourced by tools/dist.sh and tools/build-android.sh, which both ship one --
# a desktop archive beside the binary, an APK as an asset inside it. One copy
# here rather than one each, because what goes in it is a decision and not a
# convenience: get it wrong in one place and that platform's archive quietly
# stops being a record of what built it.
#
# The commit in an archive's name says which source it is, but only while the
# commit is reachable, and says nothing at all for a `-dirty` build -- which is
# exactly the build whose source is hardest to reconstruct later. So the source
# travels with the binary rather than being pointed at.
#
# Tracked files plus untracked ones git is not ignoring, taken from the working
# tree rather than from HEAD: a dirty build then ships what actually built it.
# `target/` is excluded because .gitignore excludes it.
#
# The third_party submodules are gitlinks, so their contents are not in here.
# That is deliberate and not a compliance gap: FFMPEG-SOURCE.txt, shipped
# alongside, names ffmpeg's exact upstream commit, which is what LGPL 2.1 asks
# for, and .gitmodules in the zip names the repository it came from.

# days_source_zip <root> <version> <what was built> <zip to write>
days_source_zip() {
    local root=$1 version=$2 built_for=$3 zip_path=$4
    local list note
    list=$(mktemp)
    note=$(mktemp -d)/SOURCE.txt
    git -C "$root" ls-files --cached --others --exclude-standard |
        grep -vx -e third_party/ffmpeg -e third_party/sdl >"$list"

    {
        echo "DaysEngine source, as built"
        echo
        echo "Commit:  $version"
        echo "Built:   $(date -u '+%Y-%m-%d %H:%M:%S UTC') for $built_for"
        echo
        if [ "${version%-dirty}" != "$version" ]; then
            echo "This build was made from a WORKING TREE, not from a commit. What"
            echo "is in this zip is what went into the binary; the commit named"
            echo "above is only where those changes started."
            echo
        fi
        echo "The vendored SDL3 and ffmpeg sources are not in here -- they are"
        echo "git submodules, and FFMPEG-SOURCE.txt and SDL-SOURCE.txt beside"
        echo "this zip name the exact upstream commit of each. Restore them with"
        echo "'git submodule update --init --depth 1'."
        echo
        echo "Build it with 'cargo build --locked --release'; see docs/WINDOWS.md"
        echo "for a release archive and docs/ANDROID.md for an APK."
    } >"$note"

    rm -f "$zip_path"
    (cd "$root" && zip -q -X "$zip_path" -@ <"$list")
    (cd "$(dirname "$note")" && zip -qj "$zip_path" SOURCE.txt)
    rm -rf "$list" "$(dirname "$note")"
}
