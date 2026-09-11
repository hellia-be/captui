# Roadmap

Priority-ordered backlog. Commit subjects reference these item numbers.
Completed items are removed here and recorded in docs/CHANGELOG.md.

## P1

1. Audio source picker (enumerate PipeWire nodes via wpctl / pw-dump).
2. Start/stop a wf-recorder capture (spawn, track PID, SIGINT to stop).
3. Status panel: elapsed timer + growing file size.
4. Live audio level meter.

## P2

5. Window source (compositor IPC geometry captured as a region).
6. Audio-only mode (flac) for lean transcripts.
7. Whisper handoff (configured transcription command after stop).
8. wl-screenrec backend option (hardware encode).
9. Config file (default output dir, container, default audio source).
