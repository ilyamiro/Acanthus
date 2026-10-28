#!/usr/bin/env bash
session="${1:-acanthus}"
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if ! tmux has-session -t "$session" 2>/dev/null; then
  tmux new-session -d -s "$session" -c "$dir" claude
fi
exec tmux attach -t "$session"
