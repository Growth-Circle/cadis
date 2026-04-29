#!/usr/bin/env bash
# screenshot-parity.sh — Manual HUD screenshot parity procedure for C.A.D.I.S.
# Run the HUD, then follow the steps below to capture and verify screenshots.
set -euo pipefail

OUTPUT_DIR="output/screenshots"
RESOLUTIONS=("1600x1000" "1920x1080")

echo "=== C.A.D.I.S. Screenshot Parity Test ==="
echo ""
echo "Output directory: $OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

echo ""
echo "--- Manual Procedure ---"
echo "1. Start cadisd and the HUD (pnpm tauri:dev from apps/cadis-hud)."
echo "2. For each resolution below, resize the HUD window and capture a screenshot."
echo "3. Save screenshots to $OUTPUT_DIR/<resolution>.png"
echo ""

for res in "${RESOLUTIONS[@]}"; do
  echo "  Resolution: $res"
  echo "    File: $OUTPUT_DIR/${res}.png"
done

echo ""
echo "--- Verification Checklist ---"
echo "For each screenshot, verify:"
echo "  [ ] No overlapping agent cards on the orbital HUD"
echo "  [ ] Status bar text is fully visible (daemon state, model, agent counts)"
echo "  [ ] Chat panel renders messages and composer without clipping"
echo "  [ ] Approval stack cards do not overlap or overflow"
echo "  [ ] Central orb text (state label) is readable and not truncated"
echo ""
echo "Record pass/fail per resolution in $OUTPUT_DIR/results.txt"
