#!/bin/sh
tmux new-session -s test -n Coordinator -d 'tail -f logs/coordinator.log'
tmux new-window -t test:1 -n "Coordinator Proxy" 'tail -f logs/coordinator_proxy.log'
tmux new-window -t test:2 -n Contributor 'tail -f logs/contributor.log'
tmux new-window -t test:3 -n Verifier 'tail -f logs/verifier.log'
tmux select-window -t test:0
tmux -2 attach-session -t test
