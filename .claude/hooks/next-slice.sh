#!/usr/bin/env bash
default_skill="acanthus"
marker="${CLAUDE_PROJECT_DIR:-$PWD}/.agents/next-slice"
[ -e "$marker" ] || exit 0
skill="$(tr -cd 'A-Za-z0-9_:-' < "$marker")"
skill="${skill:-$default_skill}"
rm -f "$marker"
[ -n "$TMUX" ] && [ -n "$TMUX_PANE" ] || exit 0
pane="$TMUX_PANE"
setsid nohup bash -c "
sleep 5
tmux send-keys -t '$pane' -l '/clear'
sleep 0.5
tmux send-keys -t '$pane' Enter
sleep 4
tmux send-keys -t '$pane' -l '/$skill'
sleep 0.5
tmux send-keys -t '$pane' Enter
" >/dev/null 2>&1 &
exit 0
