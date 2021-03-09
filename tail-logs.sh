#!/bin/sh
tmux new-session -s test -n Coordinator -d 'tail -f out/coordinator/coordinator.log'
tmux new-window -t test:1 -n "Coordinator Proxy" 'tail -f out/coordinator_proxy/coordinator_proxy.log'
tmux new-window -t test:2 -n Contributor 'tail -f out/contributor/contributor.log'
tmux new-window -t test:3 -n Verifier 'tail -f out/verifier/verifier.log'
tmux select-window -t test:0
tmux -2 attach-session -t test
