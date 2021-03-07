#!/bin/sh

# This file resets all transcripts and logs

# TODO: move transcripts into logs folder by changing cwd of processes

# coordinator transcript
rm -rf aleo-setup-coordinator/transcript

# coordinator log
rm -f aleo-setup.log

rm -rf logs
rm -rf keys

# contributor transcript
rm -rf transcript

