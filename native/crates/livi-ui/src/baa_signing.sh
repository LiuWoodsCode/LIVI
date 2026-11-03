# SPDX-FileCopyrightText: 2026 Lasse Heitgres
# SPDX-License-Identifier: GPL-3.0-or-later
# Additional terms: see NOTICE

# Embedded in LIVI and run by /bin/sh in its own process group. Paths and the
# original launch arguments arrive as positional parameters, never shell code.
set -eu
trap '' HUP
parent=$1 helper=$2 exe=$3 work=$4 app=$5
shift 5
staged="${helper}.baa-$$"
stopped=0
changed=0

sign_app() {
    if [ -n "$app" ]; then
        /usr/bin/codesign --force --sign - \
            --preserve-metadata=identifier,entitlements,flags,runtime "$app"
        /usr/bin/codesign --verify --deep --strict "$app"
    fi
}

finish() {
    status=$?
    trap - EXIT
    keep_backup=0
    if [ "$status" -ne 0 ]; then
        echo "[baa-signing] signing failed ($status)"
        if [ "$changed" -eq 1 ]; then
            if ! { /bin/cp -p "$work/original-helper" "$staged" &&
                /bin/mv -f "$staged" "$helper" && sign_app; }; then
                keep_backup=1
                echo "[baa-signing] rollback incomplete; original helper retained at $work/original-helper"
            fi
        fi
    fi
    /bin/rm -f "$staged"
    if [ "$stopped" -eq 1 ]; then
        echo '[baa-signing] relaunching LIVI'
        # The one-launch guard also lets LIVI run after a failed bundle repair.
        LIVI_BAA_REPAIR_ATTEMPTED=1 "$exe" "$@" &
    fi
    if [ "$keep_backup" -eq 0 ]; then
        /bin/rm -f "$work/original-helper"
    fi
    /bin/rm -f "$work/entitlements.plist"
    exit "$status"
}
trap 'finish "$@"' EXIT

/bin/cp -p "$helper" "$work/original-helper"
/bin/cp -p "$helper" "$staged"
/usr/bin/codesign --force --sign - --identifier dev.f-io.livi.helperd \
    --entitlements "$work/entitlements.plist" "$staged"
/usr/bin/codesign --verify --strict "$staged"

echo "[baa-signing] staged helper verified; stopping LIVI ($parent)"
kill -TERM "$parent"
attempt=0
while kill -0 "$parent" 2>/dev/null; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ]; then
        echo '[baa-signing] LIVI did not stop; leaving its helper unchanged'
        exit 1
    fi
    /bin/sleep 0.1
done
stopped=1
/bin/mv -f "$staged" "$helper"
changed=1
sign_app
echo '[baa-signing] BAA signature installed and verified'
