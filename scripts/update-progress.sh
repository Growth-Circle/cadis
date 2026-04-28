#!/usr/bin/env bash
# Calculates milestone progress from docs/07_MASTER_CHECKLIST.md
# and writes docs/progress.json for dynamic README badges.
set -euo pipefail

CHECKLIST="docs/07_MASTER_CHECKLIST.md"
OUTPUT="docs/progress.json"

done=$(grep -c '^\- \[x\]' "$CHECKLIST" || true)
todo=$(grep -c '^\- \[ \]' "$CHECKLIST" || true)
total=$((done + todo))
pct=$((done * 100 / total))

# Determine current milestone and next target
if [ "$pct" -lt 80 ]; then
  milestone="pre-alpha"
  next="alpha"
  # Alpha target: 80% checklist completion
  target=80
elif [ "$pct" -lt 92 ]; then
  milestone="alpha"
  next="beta"
  target=92
elif [ "$pct" -lt 98 ]; then
  milestone="beta"
  next="rc"
  target=98
else
  milestone="rc"
  next="stable"
  target=100
fi

cat > "$OUTPUT" <<EOF
{
  "checklist": {
    "done": $done,
    "total": $total,
    "percent": $pct
  },
  "milestone": "$milestone",
  "next_milestone": "$next",
  "next_target_percent": $target,
  "updated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF

echo "Progress: $done/$total ($pct%) — $milestone → $next at ${target}%"
