# The README recording. Needs a linked Agent reachable over the tailnet, with its own
# slingshot start stopped. Run from the repository root: vhs docs/media/demo.tape

Output docs/media/demo.gif

Set Shell zsh
# Twice the size the README shows it at, so text stays sharp on high density screens.
Set FontSize 24
Set Width 1660
Set Height 1100
Set Padding 30
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms

Hide
Type "tmux -f docs/media/demo.tmux.conf new-session -s demo"
Enter
Sleep 1s
Type `tmux select-pane -T 'archbox · Agent' && ssh -i ~/.ssh/slingshot_ed25519 -o IdentitiesOnly=yes -o "UserKnownHostsFile=\"$HOME/Library/Application Support/slingshot/known_hosts\"" ado@100.67.90.119`
Enter
Sleep 3s
Type "clear"
Enter
Ctrl+A
Type '"'
Sleep 1s
Type "tmux select-pane -T 'laptop · Client' && clear"
Enter
Ctrl+A
Up
Sleep 500ms
Show

Type "slingshot start"
Sleep 400ms
Enter
Wait+Screen@30s /Press Enter for a new code/
Sleep 3s

Ctrl+A
Down
Hide
Type `code=$(tmux capture-pane -p -t :.0 | grep tailnet | grep -oE '[0-9.]+:7433:[A-Z0-9]+'); clear; print -z "slingshot link $code"`
Enter
Sleep 500ms
Show
Sleep 1.5s
Enter
Wait+Screen@30s /Try it/
Sleep 4s

Type "clear && slingshot attach"
Sleep 400ms
Enter
Wait+Screen@30s /archbox · slingshot/
Sleep 1.5s
Type "nvidia-smi -L && cargo build --release"
Sleep 400ms
Enter
Wait+Screen@120s /Finished/
Sleep 3s
Ctrl+B
Type "d"
Wait+Screen@15s /detached/
Sleep 3s

Hide
Type "tmux kill-session -t demo"
Enter
