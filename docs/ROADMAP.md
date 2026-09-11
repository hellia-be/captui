# Roadmap

Priority-ordered backlog. Commit subjects reference these item numbers.

## P1

1. Source picker: enumerate outputs (wlr-randr) and select a display.
2. Region source via slurp.
3. Audio source picker (enumerate PipeWire nodes via wpctl / pw-dump).
4. Start/stop a wf-recorder capture (spawn, track PID, SIGINT to stop).
5. Status panel: elapsed timer + growing file size.
6. Live audio level meter.

## P2

7. Window source (compositor IPC geometry captured as a region).
8. Audio-only mode (flac) for lean transcripts.
9. Whisper handoff (configured transcription command after stop).
10. wl-screenrec backend option (hardware encode).
11. Config file (default output dir, container, default audio source).
