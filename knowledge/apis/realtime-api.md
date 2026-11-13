---
type: Specification
title: "OpenAI Realtime API Specification"
description: "Voice sessions over WebSocket: PCM16 audio in, audio plus transcript out, server VAD, barge-in, GA and beta event names."
tags:
  - llmsim
  - api
  - realtime
  - audio
---
# OpenAI Realtime API Specification

## Abstract

Voice agents hold a WebSocket open, stream microphone audio in and play the
answer audio while it arrives. What breaks them is timing and turn-taking:
when the server decides the user stopped talking, how fast audio comes back,
what happens when the user talks over the answer. llmsim serves
`GET /openai/v1/realtime?model=...` with those behaviors and audio of
realistic size. Code: [`src/cli/realtime.rs`](../../src/cli/realtime.rs).

## Requirements

- R1: Events and session objects follow the GA API. `OpenAI-Beta:
  realtime=v1` switches to the beta event names and session shape;
  `session.update` accepts both shapes.
- R2: Client events: `session.update`, `input_audio_buffer.append` /
  `commit` / `clear`, `conversation.item.create`, `response.create`,
  `response.cancel`. Anything else, bad base64 or bad JSON is an `error`
  event carrying the client's `event_id`; the connection stays open.
- R3: Server VAD (default on) measures 20 ms PCM16 frames: RMS at or above
  `threshold` x 0.05 is speech. Speech start emits `speech_started` (start
  minus `prefix_padding_ms`); `silence_duration_ms` of quiet emits
  `speech_stopped`, commits the buffer as a user item and, with
  `create_response`, starts a response. Speech during a response cancels it
  when `interrupt_response` is on. Without speech only the padding window
  stays buffered.
- R4: Manual commit needs at least 100 ms of audio
  (`input_audio_buffer_commit_empty`); a second `response.create` during a
  response is `conversation_already_has_active_response`; cancelling with no
  response is `response_cancel_not_active`.
- R5: Responses stream the full event sequence (`response.created`, output
  item, content part, transcript or text deltas, audio deltas, done events,
  `response.done`, then `rate_limits.updated`). Audio is a quiet 220 Hz tone,
  400 ms of PCM16 per word, paced by the model's latency profile (realtime:
  about 450 ms to first audio). Cancelled responses end with status
  `cancelled` and an `incomplete` item.
- R6: Text mode (`output_modalities: ["text"]`) sends text deltas only.
  `tool_choice: "required"` or a named function produces a `function_call`
  item with schema-valid arguments.
- R7: Input transcription, when configured, emits
  `conversation.item.input_audio_transcription.completed` with a placeholder
  transcript.
- R8: Usage: input audio at 10 tokens/s, output audio at 20 tokens/s, text
  through the tokenizer, split in `input_token_details` /
  `output_token_details`.

## Non-goals

- Speech recognition or synthesis; audio formats other than PCM16; WebRTC
  and SIP transports; scenarios.
