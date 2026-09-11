# Roadmap

Priority-ordered backlog. Commit subjects reference these item numbers.
Completed items are removed here and recorded in docs/CHANGELOG.md.

## P1

1. Region source via slurp.
2. Audio source picker (enumerate PipeWire nodes via wpctl / pw-dump).
3. Start/stop a wf-recorder capture (spawn, track PID, SIGINT to stop).
4. Status panel: elapsed timer + growing file size.
5. Live audio level meter.
6. Identify overlay: flash each output's picker number on its physical screen
   (Wayland layer-shell; wlroots compositors only).

## P2

7. Window source (compositor IPC geometry captured as a region).
8. Audio-only mode (flac) for lean transcripts.
9. Whisper handoff (configured transcription command after stop).
10. wl-screenrec backend option (hardware encode).
11. Config file (default output dir, container, default audio source).
